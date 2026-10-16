//! window proof: N procedural sprites via wgpu, fps printed, auto-exit.
//! `cargo run --release --example window --features window -- [sprites] [seconds]`

#[cfg(feature = "window")]
fn main() {
    let mut args = std::env::args().skip(1);
    let sprites: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    if let Err(e) = engine_render::gpu::run_window(sprites, seconds) {
        eprintln!("window example failed: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "window"))]
fn main() {
    eprintln!("headless build: rerun with --features window");
    std::process::exit(2);
}
