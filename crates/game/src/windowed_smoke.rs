//! windowed smoke helpers: `--exit` deadline, surface fate, CLI parse pieces
//! and the timed-smoke line.
//!
//! Consumer: `windowed` (re-exported there so `arena` keeps its `windowed::`
//! paths) plus the headless smoke legs. Data owner: this module owns only pure
//! helpers; session state stays in `windowed`. Units: wall elapsed in
//! milliseconds, fps as presented frames per wall second. Determinism scope:
//! pure functions only; same inputs give same outputs and same error strings.
//! Errors: typed `WinError` from the parent; invalid input fails loudly,
//! never panics.

use super::{smoke_fps, WinError};
use engine_render::present::PresentError;

/// Check the `--exit` wall-clock bound against an explicit limit: `Ok` while
/// `elapsed_ms` is finite nonnegative and below `limit_ms`. A hung run fails
/// itself loudly with the elapsed plus the limit named; a zero limit or a
/// non-finite/negative elapsed reading is refused the same way. Pure, so
/// tests force the deadline without waiting for it. Kept beside the landed
/// single-arg check so both call shapes stay green.
pub fn check_exit_deadline_with_limit(elapsed_ms: f64, limit_ms: u32) -> Result<(), WinError> {
    if limit_ms == 0 {
        return Err(WinError::BadArgs("--deadline-ms 0".to_string()));
    }
    if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
        return Err(WinError::BadArgs(format!(
            "exit wall elapsed_ms {elapsed_ms} is not finite nonnegative"
        )));
    }
    if elapsed_ms >= f64::from(limit_ms) {
        return Err(WinError::Render(format!(
            "exit wall-clock deadline exceeded: elapsed_ms={elapsed_ms:.0} limit_ms={limit_ms}"
        )));
    }
    Ok(())
}

/// Decide a surface-present error: transient acquire states (timeout,
/// occluded, outdated, lost) return false so the window loop skips the frame
/// and retries next frame; every other failure returns true so an unpainted
/// window fails loudly instead of reporting green. Pure over the typed
/// error, so headless tests pin the mapping without a GPU.
#[must_use]
pub fn surface_error_is_loud(err: &PresentError) -> bool {
    match err {
        PresentError::SurfaceTimeout
        | PresentError::SurfaceOccluded
        | PresentError::SurfaceLost { .. }
        | PresentError::SurfaceOutdated { .. } => false,
        PresentError::NoAdapter
        | PresentError::Device(_)
        | PresentError::DeviceLost(_)
        | PresentError::OutOfMemory { .. }
        | PresentError::Validation(_)
        | PresentError::Readback(_)
        | PresentError::InvalidSize { .. }
        | PresentError::InvalidDraw { .. }
        | PresentError::InvalidMesh(_)
        | PresentError::InvalidTexture(_)
        | PresentError::MissingTexture { .. }
        | PresentError::NoSurface
        | PresentError::BadDigest(_) => true,
    }
}

/// Parse one `--deadline-ms` value: unparsable or zero fails loudly naming
/// the flag. Split out of the CLI parse (fn-length bar).
pub(crate) fn parse_deadline_value(raw: &str) -> Result<u32, WinError> {
    let value: u32 = raw
        .parse()
        .map_err(|_| WinError::BadArgs(format!("--deadline-ms {raw}")))?;
    if value == 0 {
        return Err(WinError::BadArgs(format!("--deadline-ms {raw}")));
    }
    Ok(value)
}

/// Parse one `--fighters` value: unparsable or outside `2..=MAX` fails loudly
/// naming the flag; the documented range always fits `u8`, so the typed
/// bound replaces the cast. Split out of the CLI parse (fn-length bar).
pub(crate) fn parse_fighter_count(raw: &str) -> Result<u8, WinError> {
    let value: u32 = raw
        .parse()
        .map_err(|_| WinError::BadArgs(format!("--fighters {raw}")))?;
    if value < 2 || value > u32::from(engine_input::MAX_PARTICIPANTS) {
        return Err(WinError::BadArgs(format!("--fighters {raw}")));
    }
    u8::try_from(value).map_err(|_| WinError::BadArgs(format!("--fighters {raw}")))
}

/// Parse one `--fixed-dt` value: the frame period in milliseconds.
/// Unparsable, zero or over [`FIXED_DT_MAX_MS`] fails loudly naming the
/// flag; a bad period never silently becomes the default pace.
/// Split out of the CLI parse (fn-length bar).
pub(crate) fn parse_fixed_dt_value(raw: &str) -> Result<u32, WinError> {
    let value: u32 = raw
        .parse()
        .map_err(|_| WinError::BadArgs(format!("--fixed-dt {raw}")))?;
    if value == 0 || value > super::FIXED_DT_MAX_MS {
        return Err(WinError::BadArgs(format!("--fixed-dt {raw}")));
    }
    Ok(value)
}

/// Format one timed-smoke line: presented frames plus wall elapsed plus
/// measured fps. Non-finite or non-positive elapsed fails loudly.
pub fn format_timed_smoke(frames: u32, ticks: u32, elapsed_ms: f64) -> Result<String, WinError> {
    let fps = smoke_fps(frames.max(1), elapsed_ms)?;
    Ok(format!(
        "arena timed-smoke: frames={frames} ticks={ticks} elapsed_ms={elapsed_ms:.3} fps={fps:.1}"
    ))
}

/// CSV header for the timed-smoke frame-times file: one row per presented
/// frame with wall time plus input-to-present time. Units: milliseconds.
/// Determinism: pure constant, same bytes on every machine.
pub const FRAME_TIMES_CSV_HEADER: &str = "frame,wall_ms,input_to_present_ms";

/// Format one frame-times CSV row: `frame,wall_ms,input_to_present_ms`
/// with three decimals. Non-finite or negative wall or present times fail
/// loudly naming the leg, so a bad sample never files a fake row. Pure over
/// its inputs, so tests force bad rows without a run. Presentation only,
/// never hashed.
/// Caller: `crates/game/src/bin/arena.rs` files one row per presented frame.
pub fn frame_times_csv_row(
    frame: u32,
    wall_ms: f64,
    input_to_present_ms: f64,
) -> Result<String, WinError> {
    if !wall_ms.is_finite() || wall_ms < 0.0 {
        return Err(WinError::BadArgs(format!(
            "frame-times wall_ms {wall_ms} is not finite nonnegative"
        )));
    }
    if !input_to_present_ms.is_finite() || input_to_present_ms < 0.0 {
        return Err(WinError::BadArgs(format!(
            "frame-times input_to_present_ms {input_to_present_ms} is not finite nonnegative"
        )));
    }
    Ok(format!("{frame},{wall_ms:.3},{input_to_present_ms:.3}"))
}
