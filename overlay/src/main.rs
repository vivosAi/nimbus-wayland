fn main() {
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("nimbus-wayland runs on Linux under a Wayland compositor.");
        eprintln!("The logic crate builds anywhere; only the surface does not.");
        std::process::exit(1);
    }
    #[cfg(target_os = "linux")]
    {
        eprintln!("not yet implemented: see SPEC.md milestones M0 to M5");
        std::process::exit(1);
    }
}
