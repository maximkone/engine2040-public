//! The `e2040-mcp` bin hosts the real look judge: it hands [`host_look_judge`]
//! to `packs::view::register_look_judge` once at start, and that one
//! registration (process-wide, one-shot) then answers every `look.judge` call.

use std::path::Path;

use engine_playtest::look::judge::{
    judge, load_capture_dir, load_png_dir, JudgeConfig, JudgeError, NamedPng, References,
};

use crate::mcp::McpToolError;
use crate::packs::view::LookJudgeArgs;

/// Fighter-preview directory used when the request names none.
const DEFAULT_PREVIEWS: &str = "content/fighters/previews";

/// Refusal code for a capture set the judge could not read or score.
const CODE_CAPTURE: &str = "look_capture";

/// Refusal code for a manifest the judge refused to bind.
const CODE_MANIFEST: &str = "look_manifest";

/// One [`JudgeError`] as the door's typed refusal: a refused manifest keeps
/// its own code, every other read or score fault rides `look_capture`.
fn bridge(error: JudgeError) -> McpToolError {
    let code = if matches!(error, JudgeError::BadManifest(_)) {
        CODE_MANIFEST
    } else {
        CODE_CAPTURE
    };
    McpToolError::new(code, error.to_string())
}

/// One reference pool in name order: a directory the request names must be
/// readable, while the shipped default is used only when it is on disk.
fn pool(dir: Option<&str>, default: Option<&str>) -> Result<Vec<NamedPng>, McpToolError> {
    let path = match (dir, default) {
        (Some(named), _) => Path::new(named),
        (None, Some(fallback)) if Path::new(fallback).is_dir() => Path::new(fallback),
        (None, _) => return Ok(Vec::new()),
    };
    load_png_dir(path).map_err(bridge)
}

/// Judge one `look.judge` request with the real engine-playtest judge: load
/// the capture directory, load the optional reference pools, score the set
/// and return the report as its JSON text.
///
/// # Errors
///
/// `bad_args` never reaches here (the door checks the paths first). A capture
/// directory the judge cannot read or score crosses as `look_capture`; a
/// manifest it refuses crosses as `look_manifest`.
pub fn host_look_judge(args: &LookJudgeArgs) -> Result<String, McpToolError> {
    let set = load_capture_dir(Path::new(&args.capture_dir)).map_err(bridge)?;
    let refs = References {
        fighters: pool(args.preview_dir.as_deref(), Some(DEFAULT_PREVIEWS))?,
        q1: pool(args.q1_dir.as_deref(), None)?,
    };
    let judgement = judge(&set, &refs, &JudgeConfig::default()).map_err(bridge)?;
    Ok(judgement.to_json())
}
