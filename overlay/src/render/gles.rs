//! The GPU: an EGL context on a Wayland surface, and the ring program.
//!
//! Linux-only, and the only part of the crate that is. Everything it needs to
//! decide has already been decided in [`super::uniforms`], which is testable
//! anywhere — that split is deliberate, because the macOS version's real bugs
//! all lived above this layer rather than in it.
//!
//! SPEC.md §4 left the GL path open. This is raw EGL with GLES3 rather than
//! `wgpu`: Hyprland itself has required GLES 3.0 since 0.50, so every machine
//! that can run the compositor can run this, and `wgpu` would add a shader
//! translator and four backends for one fragment shader.

use glow::HasContext;
use khronos_egl as egl;
use std::ffi::c_void;

use super::uniforms::{Uniforms, UNIFORMS_SIZE};

type Egl = egl::DynamicInstance<egl::EGL1_4>;

pub const VERTEX_SRC: &str = include_str!("../../../shaders/ring.vert");
pub const FRAGMENT_SRC: &str = include_str!("../../../shaders/ring.frag");

/// Four quads, two triangles each. The vertex stage builds them from the
/// uniform block, so there is no vertex buffer and no attributes.
const RING_VERTEX_COUNT: i32 = 24;

#[derive(Debug)]
pub enum Error {
    Egl(String),
    Gl(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Egl(m) => write!(f, "EGL: {m}"),
            Error::Gl(m) => write!(f, "GL: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// The EGL display and context, shared by every output's surface.
///
/// One context for all outputs: the surfaces must agree on phase anyway
/// (SPEC.md §6), and a shared context means the program and uniform buffer are
/// compiled and allocated once.
pub struct GlContext {
    egl: Egl,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    pub gl: glow::Context,
    program: glow::Program,
    ubo: glow::Buffer,
    vao: glow::VertexArray,
}

impl GlContext {
    /// Bring up EGL against the compositor's `wl_display`.
    ///
    /// # Safety
    /// `wl_display` must be a live `*mut wl_display` for the whole lifetime of
    /// the returned context.
    pub unsafe fn new(wl_display: *mut c_void) -> Result<Self, Error> {
        let egl = unsafe { Egl::load_required() }
            .map_err(|e| Error::Egl(format!("libEGL is not loadable: {e}")))?;

        let display = unsafe { egl.get_display(wl_display) }
            .ok_or_else(|| Error::Egl("no EGL display for this wl_display".into()))?;
        egl.initialize(display)
            .map_err(|e| Error::Egl(format!("eglInitialize failed: {e}")))?;

        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| Error::Egl(format!("eglBindAPI(ES) failed: {e}")))?;

        // ALPHA_SIZE 8 is the one that matters. Without a destination alpha
        // channel the surface is opaque and the premultiplied output has
        // nothing to blend into — the failure mode SPEC.md §13 describes, where
        // every frame renders correctly and composites to nothing.
        const OPENGL_ES3_BIT: egl::Int = 0x0040;
        let attribs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            OPENGL_ES3_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attribs)
            .map_err(|e| Error::Egl(format!("eglChooseConfig failed: {e}")))?
            .ok_or_else(|| {
                Error::Egl("no EGL config with an alpha channel and GLES3".into())
            })?;

        let ctx_attribs = [egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE];
        let context = egl
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| Error::Egl(format!("eglCreateContext failed: {e}")))?;

        // A context must be current before any GL entry point is resolved.
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| Error::Egl(format!("eglMakeCurrent(surfaceless) failed: {e}")))?;

        let gl = unsafe {
            glow::Context::from_loader_function(|sym| {
                egl.get_proc_address(sym)
                    .map_or(std::ptr::null(), |p| p as *const c_void)
            })
        };

        let program = unsafe { link_program(&gl)? };
        let (ubo, vao) = unsafe { create_buffers(&gl, program)? };

        unsafe { configure_blending(&gl) };

        Ok(GlContext { egl, display, config, context, gl, program, ubo, vao })
    }

    pub fn egl(&self) -> &Egl {
        &self.egl
    }
    pub fn display(&self) -> egl::Display {
        self.display
    }
    pub fn config(&self) -> egl::Config {
        self.config
    }

    /// Make a surface current and size the viewport to it, both in physical
    /// pixels.
    pub fn make_current(&self, surface: egl::Surface, width: i32, height: i32) -> Result<(), Error> {
        self.egl
            .make_current(self.display, Some(surface), Some(surface), Some(self.context))
            .map_err(|e| Error::Egl(format!("eglMakeCurrent failed: {e}")))?;
        unsafe { self.gl.viewport(0, 0, width, height) };
        Ok(())
    }

    /// Clear to fully transparent and draw the ring.
    ///
    /// The clear is not optional: the surface is a transparent sheet over the
    /// desktop, so every pixel the ring does not cover must be zero, not
    /// whatever the previous frame left in the buffer.
    pub fn draw(&self, uniforms: &Uniforms) {
        unsafe {
            self.gl.clear_color(0.0, 0.0, 0.0, 0.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);

            self.gl.use_program(Some(self.program));
            self.gl.bind_buffer(glow::UNIFORM_BUFFER, Some(self.ubo));
            self.gl
                .buffer_sub_data_u8_slice(glow::UNIFORM_BUFFER, 0, uniforms.as_bytes());
            self.gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, Some(self.ubo));

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLES, 0, RING_VERTEX_COUNT);
        }
    }

    /// Clear a surface to nothing. Used when an output's surface should show no
    /// ring — SPEC.md §6: only the output holding the focused window draws.
    pub fn clear(&self) {
        unsafe {
            self.gl.clear_color(0.0, 0.0, 0.0, 0.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    pub fn swap_buffers(&self, surface: egl::Surface) -> Result<(), Error> {
        self.egl
            .swap_buffers(self.display, surface)
            .map_err(|e| Error::Egl(format!("eglSwapBuffers failed: {e}")))
    }

    /// Ask the compositor not to hold back the swap. The frame loop is driven
    /// by frame callbacks, which already pace to the refresh rate; letting EGL
    /// block as well would pace it twice.
    pub fn set_swap_interval(&self, interval: i32) {
        let _ = self.egl.swap_interval(self.display, interval);
    }
}

/// Premultiplied alpha, matching `shaders/ring.frag` and the macOS pipeline.
/// Getting this wrong puts a grey film over whatever is behind the ring instead
/// of adding light to it.
unsafe fn configure_blending(gl: &glow::Context) {
    unsafe {
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.enable(glow::BLEND);
        gl.blend_equation_separate(glow::FUNC_ADD, glow::FUNC_ADD);
        gl.blend_func_separate(
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
        );
    }
}

unsafe fn link_program(gl: &glow::Context) -> Result<glow::Program, Error> {
    unsafe {
        let program = gl
            .create_program()
            .map_err(|e| Error::Gl(format!("glCreateProgram: {e}")))?;

        let mut shaders = Vec::new();
        for (kind, src, label) in [
            (glow::VERTEX_SHADER, VERTEX_SRC, "ring.vert"),
            (glow::FRAGMENT_SHADER, FRAGMENT_SRC, "ring.frag"),
        ] {
            let shader = gl
                .create_shader(kind)
                .map_err(|e| Error::Gl(format!("glCreateShader({label}): {e}")))?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(Error::Gl(format!(
                    "{label} failed to compile:\n{}",
                    gl.get_shader_info_log(shader)
                )));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }

        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(Error::Gl(format!(
                "link failed:\n{}",
                gl.get_program_info_log(program)
            )));
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        Ok(program)
    }
}

unsafe fn create_buffers(
    gl: &glow::Context,
    program: glow::Program,
) -> Result<(glow::Buffer, glow::VertexArray), Error> {
    unsafe {
        // The block is shared by both stages, so it is bound once to index 0.
        let index = gl
            .get_uniform_block_index(program, "Uniforms")
            .ok_or_else(|| Error::Gl("the shaders declare no Uniforms block".into()))?;
        gl.uniform_block_binding(program, index, 0);

        let ubo = gl
            .create_buffer()
            .map_err(|e| Error::Gl(format!("glGenBuffers: {e}")))?;
        gl.bind_buffer(glow::UNIFORM_BUFFER, Some(ubo));
        gl.buffer_data_size(
            glow::UNIFORM_BUFFER,
            UNIFORMS_SIZE as i32,
            glow::DYNAMIC_DRAW,
        );

        // GLES3 requires a bound vertex array object even when drawing without
        // attributes, which is what the four-quad vertex stage does.
        let vao = gl
            .create_vertex_array()
            .map_err(|e| Error::Gl(format!("glGenVertexArrays: {e}")))?;

        Ok((ubo, vao))
    }
}
