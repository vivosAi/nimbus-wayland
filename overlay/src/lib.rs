//! Nimbus for Wayland.
//!
//! The layers that depend on Wayland and GL are gated to Linux. Everything
//! else, which is where the interesting bugs live, compiles and tests on any
//! platform.

pub mod config;
pub mod control;
pub mod focus;

#[cfg(target_os = "linux")]
pub mod install;
pub mod render;
pub mod style;

#[cfg(target_os = "linux")]
pub mod app;
