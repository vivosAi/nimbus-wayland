//! Rendering. The GL layer is Linux-only; the uniform maths is not.

pub mod uniforms;

// The GPU is the only Linux-gated part of the crate.
#[cfg(target_os = "linux")]
pub mod gles;
