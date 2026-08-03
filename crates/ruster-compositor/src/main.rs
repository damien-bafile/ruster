use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("ruster-compositor: Phase 0 scaffold");
        println!("Usage: ruster-compositor [--drm]");
        return Ok(());
    }
    if args.iter().any(|a| a == "--drm") {
        // TODO(Task 11): boot the udev/DRM backend on the primary GPU.
        anyhow::bail!("--drm backend is not implemented until Task 11");
    }
    run_winit()
}

fn run_winit() -> anyhow::Result<()> {
    use smithay::reexports::calloop::EventLoop;

    let _event_loop: EventLoop<'static, ()> = EventLoop::try_new()?;
    tracing::info!("Phase 0 winit scaffold ready (event loop created)");
    // TODO(Task 5): create the Display + CompositorState<WinitData>, seat,
    // keyboard, pointer, output global, and run the winit event pump.
    Ok(())
}
