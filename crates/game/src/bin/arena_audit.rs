//! arena_audit: the window's frame audit sidecar, `view-audit.jsonl` beside the capture PNGs.
//!
//! Owner: engine-game bin; consumer: `arena.rs` (`capture_offscreen`). One row per captured
//! frame: the posed parts, joints and spears `tools/frame-audit.mjs` judges against the pixels
//! (`engine_game::view_truth::audit`). View only: nothing here reaches the simulation or its hash.
//! Errors: a refused frame is the capture's own error; a refused audit row is a loud notice and
//! never a failed run. Helper module of the `arena` binary only (`mod arena_audit;` in `arena.rs`).

use std::io::Write as _;

use engine_game::capture::CaptureError;
use engine_game::view_truth::audit::{audit_line, AUDIT_FILE};

/// File one captured frame, then append its audit row. `Ok` with nothing written when no capture
/// is armed. The first frame of a run starts the file, so a re-run into the same dir never
/// doubles rows.
pub(crate) fn write_frame(
    app: &mut crate::ArenaApp,
    tick: u32,
    hash: u64,
    png: &[u8],
    camera: &engine_render::v1::Camera,
) -> Result<(), CaptureError> {
    let Some(slot) = app.capture.as_mut() else {
        return Ok(());
    };
    let (sink, tap) = (&mut slot.sink, &mut slot.look_tap);
    sink.write_frame_with_hook(tick, hash, png, Some(&mut |bytes| tap.note(bytes)))?;
    let (Some(look), Some((file, tick))) = (app.look.as_ref(), sink.last_frame()) else {
        return Ok(());
    };
    let scene = app.spear.scene.as_ref();
    let spears = scene
        .map(|s| s.audit_rows(app.last_alpha))
        .unwrap_or_default();
    let line = audit_line(file, tick, camera, look.view_truth(), &spears);
    let path = sink.dir().join(AUDIT_FILE);
    let first = sink.frames() == 1;
    let written = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(first)
        .append(!first)
        .open(&path)
        .and_then(|mut out| writeln!(out, "{line}"));
    if let Err(err) = written {
        let notice = format!("arena notice: {}: {err}", path.display());
        eprintln!("{notice}");
        engine_game::crash::mirror_line(&notice);
    }
    Ok(())
}
