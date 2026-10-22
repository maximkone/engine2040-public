//! args: the arena window's own argument splits, one module.
//!
//! Owner: game lane; consumers are the arena binary's `run` and its tests.
//! Units: raw argument slices in, raw argument slices out.
//! Determinism: pure over the slice; the same arguments give the same split
//! every time.
//!
//! The window's own flags are pulled off the argument list here, before the
//! strict [`WinArgs`](engine_game::windowed::WinArgs) parser runs, because that
//! parser refuses an unknown flag. Each split takes the raw slice, finds its
//! own flag, and hands back the rest in order. A flag with a missing or
//! unparsable value fails loudly, never silently defaulted.

use std::path::PathBuf;

use engine_input::PhysicalButton;

/// `--view-brief <path>`: write the view truth of every presented frame as
/// one JSON line (camera, then each drawn body's act, clip, tint, bounds and
/// screen rect), so a script or a model reads the scene with no screenshot.
pub const VIEW_BRIEF_FLAG: &str = "--view-brief";
/// `--hold <KEYS>`: keys held down from the first tick, as if pressed by hand (`W`, `W,F`; the
/// names are W A S D F Q SPACE SHIFT). A dev flag for receipts and smoke runs: the keys take the
/// keyboard's own road (bindings, camera-relative wish, spear key), so a capture can show W
/// walking toward the rival. Presentation and input only.
pub const HOLD_FLAG: &str = "--hold";
/// `--agent-seat <PORT>`: take the agent seat on a port, so a tool can drive
/// the fight over the network. Port 0 asks the OS for a free port.
pub const AGENT_SEAT_FLAG: &str = "--agent-seat";
/// `--view-line <SEED> <TICKS> <FIGHTERS>`: run one scripted line.
pub const VIEW_LINE_FLAG: &str = "--view-line";
/// `--pause-at-tick <N>`: hold the fixed-tick accumulator once the accumulated
/// tick count reaches `N` (`0` holds before the first tick). Presentation only:
/// the sim's integer cursor stops, the frozen hash stands.
pub const PAUSE_AT_TICK_FLAG: &str = "--pause-at-tick";
/// `--step`: a bare flag that lets one Space press advance exactly one held
/// tick while `--pause-at-tick` holds. One press is one tick, never one per
/// presented frame. Presentation only.
pub const STEP_FLAG: &str = "--step";

/// Pull `--view-brief <path>` out of the argument list; a flag with no
/// path is an error.
pub fn split_view_brief(raw: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let Some(at) = raw.iter().position(|arg| arg == VIEW_BRIEF_FLAG) else {
        return Ok((None, raw.to_vec()));
    };
    let path = raw
        .get(at + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{VIEW_BRIEF_FLAG} needs a file path"))?;
    let mut rest = raw.to_vec();
    rest.drain(at..=at + 1);
    Ok((Some(PathBuf::from(path)), rest))
}

/// Pull `--hold <KEYS>` out of the argument list: the held buttons (a
/// comma list of W A S D F Q SPACE SHIFT, any case) and the rest in order. A
/// flag with no list, or a key it does not know, is an error.
pub fn split_hold(raw: &[String]) -> Result<(Vec<PhysicalButton>, Vec<String>), String> {
    use engine_input::map::Key;
    let Some(at) = raw.iter().position(|arg| arg == HOLD_FLAG) else {
        return Ok((Vec::new(), raw.to_vec()));
    };
    let list = raw
        .get(at + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{HOLD_FLAG} needs keys, for example W"))?;
    let mut held = Vec::new();
    for name in list.split(',') {
        let key = match name.trim().to_ascii_uppercase().as_str() {
            "W" => Key::W,
            "A" => Key::A,
            "S" => Key::S,
            "D" => Key::D,
            "F" => Key::F,
            "Q" => Key::Q,
            "SPACE" => Key::SPACE,
            "SHIFT" => Key::SHIFT,
            other => return Err(format!("{HOLD_FLAG} {other}: use W A S D F Q SPACE SHIFT")),
        };
        held.push(PhysicalButton::Key(key.0));
    }
    let mut rest = raw.to_vec();
    rest.drain(at..=at + 1);
    Ok((held, rest))
}

/// Pull one bare `flag` (`--look-probes`, `--require-framing`) out of the
/// argument list (the rest still goes to the strict `WinArgs` parser, which
/// refuses unknown flags).
pub fn split_flag(raw: &[String], flag: &str) -> (bool, Vec<String>) {
    let on = raw.iter().any(|arg| arg == flag);
    let rest = raw
        .iter()
        .filter(|arg| arg.as_str() != flag)
        .cloned()
        .collect();
    (on, rest)
}

/// Split the value flag `--pause-at-tick <N>` plus the bare `--step` flag off
/// the argument list before the strict `WinArgs` parser (it would refuse either
/// as unknown), same shape as [`split_flag`]. Returns the pause tick (if any),
/// whether stepping is armed, and the remainder in order. A `--pause-at-tick`
/// with a missing or unparsable tick fails loudly, never silently defaulted.
/// Pure over the argument slice.
pub fn split_pause_flags(raw: &[String]) -> Result<(Option<u64>, bool, Vec<String>), String> {
    let step_on = raw.iter().any(|arg| arg == STEP_FLAG);
    let mut pause_at = None;
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0usize;
    while index < raw.len() {
        let arg = &raw[index];
        if arg == PAUSE_AT_TICK_FLAG {
            let value = raw
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{PAUSE_AT_TICK_FLAG} needs a tick"))?;
            let tick: u64 = value
                .parse()
                .map_err(|_| format!("{PAUSE_AT_TICK_FLAG} {value}"))?;
            pause_at = Some(tick);
            index += 2;
            continue;
        }
        if arg == STEP_FLAG {
            index += 1;
            continue;
        }
        rest.push(arg.clone());
        index += 1;
    }
    Ok((pause_at, step_on, rest))
}

/// Split the value flag `--agent-seat <PORT>` off the argument list before
/// the strict parser (it would refuse an unknown flag). Port 0 asks the OS
/// for a free port; the bound one is printed at boot. A missing or
/// unparsable port fails loudly, never silently falls back to the keyboard.
pub fn split_agent_seat(raw: &[String]) -> Result<(Option<u16>, Vec<String>), String> {
    let mut port = None;
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0usize;
    while index < raw.len() {
        let arg = &raw[index];
        if arg == AGENT_SEAT_FLAG {
            let value = raw
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{AGENT_SEAT_FLAG} needs a port"))?;
            let parsed: u16 = value
                .parse()
                .map_err(|_| format!("{AGENT_SEAT_FLAG} {value}"))?;
            port = Some(parsed);
            index += 2;
            continue;
        }
        rest.push(arg.clone());
        index += 1;
    }
    Ok((port, rest))
}

/// A `--view-line` request (seed, ticks, fighters) and the arguments left for the strict parser.
pub type ViewLineSplit = (Option<(u64, u32, u8)>, Vec<String>);

/// Split `--view-line <SEED> <TICKS> <FIGHTERS>` off before the strict parser
/// (it would refuse an unknown flag) and check the three numbers against the
/// session config's own ranges, so a bad request fails loudly at boot instead
/// of building a bout the sim would refuse. Zero ticks is the empty line.
pub fn split_view_line(raw: &[String]) -> Result<ViewLineSplit, String> {
    let mut request = None;
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0usize;
    while index < raw.len() {
        let arg = &raw[index];
        if arg != VIEW_LINE_FLAG {
            rest.push(arg.clone());
            index += 1;
            continue;
        }
        let three = raw
            .get(index + 1..index + 4)
            .filter(|values| values.iter().all(|value| !value.starts_with("--")))
            .ok_or_else(|| format!("{VIEW_LINE_FLAG} needs SEED TICKS FIGHTERS"))?;
        let seed: u64 = three[0]
            .parse()
            .map_err(|_| format!("{VIEW_LINE_FLAG} seed {}", three[0]))?;
        let ticks: u32 = three[1]
            .parse()
            .map_err(|_| format!("{VIEW_LINE_FLAG} ticks {}", three[1]))?;
        let fighters: u8 = three[2]
            .parse()
            .map_err(|_| format!("{VIEW_LINE_FLAG} fighters {}", three[2]))?;
        if ticks > engine_game::MAX_TICKS {
            return Err(format!(
                "{VIEW_LINE_FLAG} ticks {ticks} is outside 0..={}",
                engine_game::MAX_TICKS
            ));
        }
        if !(2..=engine_arena::MAX_PARTICIPANTS).contains(&fighters) {
            return Err(format!(
                "{VIEW_LINE_FLAG} fighters {fighters} is outside 2..={}",
                engine_arena::MAX_PARTICIPANTS
            ));
        }
        request = Some((seed, ticks, fighters));
        index += 4;
    }
    Ok((request, rest))
}
