//! `--headless`: the no-window route's flag, route name and receipt line.
//! Split out of `arena_capture` so that file stays under its production-line
//! cap; the session itself still runs through the capture helpers.

/// `--headless`: play the session with no window and no GPU presenter. A net
/// seat keeps its own `--headless` meaning, so the flag is left for the net
/// split then.
pub(crate) const HEADLESS_FLAG: &str = "--headless";

/// Why a session ran with no window: `--headless` asked for it, or the
/// machine has no display. The token rides the smoke receipt, so a shipped
/// check can name the route that ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HeadlessWhy {
    /// `--headless`: no window was ever requested.
    Forced,
    /// The event loop or the window could not open.
    NoDisplay,
}

impl HeadlessWhy {
    /// The `headless=` token on the smoke receipt.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Forced => "forced",
            Self::NoDisplay => "no-display",
        }
    }
}

/// The `arena headless smoke:` receipt: ticks, presented frames, the last
/// checkpoint hash, the HUD counts, the audio and presenter notices, and the
/// route that ran. `hash=-` when no tick ran, so a zero-tick receipt never
/// looks like a hash. Pure, so the shipped checker and the tests read one
/// shape.
pub(crate) fn headless_smoke_line(
    report: &engine_game::windowed::WinReport,
    why: HeadlessWhy,
    display: &str,
) -> String {
    let hash = report
        .last_checkpoint
        .map_or_else(|| "-".to_string(), |point| format!("{:016x}", point.hash));
    format!(
        "arena headless smoke: ticks={} frames={} hash={hash} hud_cells={} hud_quads={} mixed_frames={} headless={} audio=\"{}\" presenter=\"{}\" display=\"{display}\"",
        report.ticks_run,
        report.frames_presented,
        report.hud_cells,
        report.hud_quads,
        report.mixed_frames,
        why.token(),
        report.audio_notice,
        report.presenter_notice,
    )
}

/// Strip a plain `--headless` before the net parser sees it. With a net
/// seat the flag stays: there it asks the net session to run windowless.
pub(crate) fn split_plain_headless(raw: &[String]) -> (bool, Vec<String>) {
    let net_seat = raw.iter().any(|arg| arg == "--host" || arg == "--join");
    if net_seat {
        (false, raw.to_vec())
    } else {
        crate::arena_args::split_flag(raw, HEADLESS_FLAG)
    }
}
