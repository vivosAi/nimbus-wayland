//! Nimbus for Wayland.
//!
//! The layers that depend on Wayland and GL are gated to Linux. Everything
//! else, which is where the interesting bugs live, compiles and tests on any
//! platform.

pub mod focus;
pub mod render;
pub mod style;
