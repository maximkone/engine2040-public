//! `play` pack: `play.step_until` until one hp falls, plus the rival-door
//! verbs `input.inject` (hold one named action) and `state.digest` (hash one
//! bout prefix), and `play.sweep` (seeded bots play N bouts, one envelope);
//! see `play_ext.rs`, `rival.rs` and `sweep.rs`.
//! `seat.open` / `seat.step` / `seat.close` drive the live `arena.exe` seat:
//! the door spawns the window with `--agent-seat 0`, pushes one validated
//! `FighterInput` per fixed tick and returns that tick's match hash, so an
//! agent plays the picture it can inspect without a raw socket.
//! Consumer: MCP door after `api.load`. Owner: `engine-arena` sim; DTOs here.
//! Units: ticks, permille, meters, `u64` digests (decimal strings). Same args
//! twice seal same bytes; no clock. Errors: typed with recovery. DTO docs off.
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::{dec_u64, BoutSpec, PlaySlot};
use crate::play::{build_tape, PlayInputDto, MAX_PLAY_INPUTS, MAX_PLAY_TICKS};
use crate::tool_capabilities::CapabilityCard;
use engine_ai::decision_log::{Decision, DecisionLog};
use engine_arena::MAX_PARTICIPANTS;
use engine_input::fighter::WISH_RANGE;
use engine_input::{Buttons, ParticipantError, ParticipantInput, PARTICIPANT_ZERO};
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};
/// Pack name.
pub const PACK: &str = "play";
/// Verb: run until one hp falls.
pub const TOOL_PLAY_STEP_UNTIL: &str = "play.step_until";
/// Verb: open the live arena seat of this session.
pub const TOOL_SEAT_OPEN: &str = "seat.open";
/// Verb: author one tick of the live seat and read its hash.
pub const TOOL_SEAT_STEP: &str = "seat.step";
/// Verb: close the live seat and kill its window.
pub const TOOL_SEAT_CLOSE: &str = "seat.close";
/// Verb: show one AI decision tape, or diff two and name the first divergent tick.
pub const TOOL_AI_DECISIONS: &str = "ai.decisions";

#[path = "play_ext.rs"]
mod play_ext;
#[path = "play_scene.rs"]
mod play_scene;
#[path = "rival.rs"]
mod rival;
#[path = "sweep.rs"]
mod sweep;

pub use play_scene::{seat_ask, seat_is_open};
pub use play_ext::{
    InputInjectArgs, InputInjectOutput, InputInjectTool, StateDigestArgs, StateDigestOutput,
    StateDigestTool, TOOL_INPUT_INJECT, TOOL_STATE_DIGEST,
};
pub use rival::{rival_for, verb_for_rival, RIVAL_PARITY};
pub use sweep::{PlaySweepArgs, PlaySweepOutput, PlaySweepTool, TOOL_PLAY_SWEEP};

/// The play pack, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "run a bout prefix until one hp falls or a cap, hold one named action, hash one prefix",
        false,
    );
    pack.add(VerbSpec::of::<PlayStepUntilTool>(
        TOOL_PLAY_STEP_UNTIL,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SeatOpenTool>(
        TOOL_SEAT_OPEN,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SeatStepTool>(
        TOOL_SEAT_STEP,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SeatCloseTool>(
        TOOL_SEAT_CLOSE,
        PACK,
        CapabilityCard::Observe,
    )?);
    play_ext::add_verbs(&mut pack)?;
    sweep::add_verb(&mut pack)?;
    pack.add(VerbSpec::of::<AiDecisionsTool>(
        TOOL_AI_DECISIONS,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}
/// Request: seed, bout, tape, cap and watched hp threshold.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepUntilArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
    #[serde(default)]
    pub tick_cap: Option<u32>,
    #[serde(default)]
    pub fighter: Option<u8>,
    #[serde(default)]
    pub hp_below: Option<u16>,
}
/// Answer: first tripping tick, digest and hp.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepUntilOutput {
    pub tick: u32,
    pub stopped: bool,
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    pub participants: u8,
    pub fighter_hp: u16,
}
/// Run until watched hp falls or cap; same slot path as `sim.step`.
pub fn step_until_report(args: &PlayStepUntilArgs) -> Result<PlayStepUntilOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    let half = args.half_extent_m.unwrap_or(20.0);
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!(
                "participants {participants} is outside 2..={MAX_PARTICIPANTS}; re-run in range"
            ),
        ));
    }
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0; re-run in range"),
        ));
    }
    let cap = args.tick_cap.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("tick_cap missing; submit 1..={MAX_PLAY_TICKS}; re-run with a cap"),
        )
    })?;
    if cap == 0 || cap > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("tick_cap {cap} is outside 1..={MAX_PLAY_TICKS}; re-run in range"),
        ));
    }
    let slot_id = args.fighter.ok_or_else(|| {
        McpToolError::new(
            "bad_fighters",
            format!("fighter missing; submit 0..{participants}; re-run with a slot"),
        )
    })?;
    if slot_id >= participants {
        return Err(McpToolError::new(
            "bad_fighters",
            format!("fighter {slot_id} is outside 0..{participants}; re-run with a live slot"),
        ));
    }
    let below = args
        .hp_below
        .ok_or_else(|| bad_args("hp_below missing; submit 0..=1000; re-run".to_owned()))?;
    if below > 1000 {
        return Err(bad_args(format!(
            "hp_below {below} is outside 0..=1000; re-run in range"
        )));
    }
    let inputs = args.inputs.clone().unwrap_or_default();
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS}; re-run shorter",
                inputs.len()
            ),
        ));
    }
    let spec = BoutSpec::new(seed, participants, half);
    let mut last = (cap, 0u64, 1000u16);
    for now in 1..=cap {
        let prefix: Vec<PlayInputDto> = inputs
            .iter()
            .filter(|dto| dto.tick < now)
            .cloned()
            .collect();
        let tape = build_tape(&prefix, now, participants)?;
        let mut slot = PlaySlot::default();
        let view = slot.advance_to(spec, &tape, now)?.observe(0);
        let hp = view
            .fighters
            .iter()
            .find(|f| f.id == slot_id)
            .map_or(1000u16, |f| f.hp);
        last = (now, view.digest, hp);
        if hp < below {
            return Ok(PlayStepUntilOutput {
                tick: now,
                stopped: true,
                digest: view.digest,
                participants,
                fighter_hp: hp,
            });
        }
    }
    Ok(PlayStepUntilOutput {
        tick: last.0,
        stopped: false,
        digest: last.1,
        participants,
        fighter_hp: last.2,
    })
}
fn play_step_until(
    service: &McpServer,
    args: &PlayStepUntilArgs,
) -> Result<PlayStepUntilOutput, McpToolError> {
    service.dispatch(TOOL_PLAY_STEP_UNTIL, |_state| step_until_report(args))
}
crate::e2_tool!(
    /// `play.step_until` tool: run until one hp falls. Read-only.
    PlayStepUntilTool,
    name = TOOL_PLAY_STEP_UNTIL,
    description = "Run a bout prefix until one hp falls below hp_below or tick_cap: seed, bout, tape, cap and slot in; first tick, digest and hp out. Same inputs replay same tick. Read-only.",
    readonly = true,
    args = PlayStepUntilArgs,
    output = PlayStepUntilOutput,
    invoke = play_step_until
);

// ---------------------------------------------------------------------------
// seat.open / seat.step / seat.close: the door drives the live AIN1 seat.
// ---------------------------------------------------------------------------

/// Boot line the arena prints once its seat listens (`agent_seat`).
const SEAT_BOOT_MARK: &str = "arena seat: listening on 127.0.0.1:";
/// Longest wait for the arena's boot line: a cold content load plus the window.
const SEAT_BOOT_TIMEOUT: Duration = Duration::from_millis(30_000);
/// Poll step while the arena boots.
const SEAT_BOOT_POLL: Duration = Duration::from_millis(20);
/// Longest wait for one tick line. A cold window stages its content for
/// about 20 s before the first tick and then runs about 60 ticks a second,
/// so the same bound covers the first tick and a stalled window.
const SEAT_TICK_TIMEOUT: Duration = Duration::from_secs(60);
/// Most raw lines one `seat.step` reads before it gives up.
const SEAT_MAX_LINES: usize = 4096;
/// Frames one seat run may ask for (matches the game's tick cap).
const MAX_SEAT_FRAMES: u32 = 100_000;
/// Kept stdout tail beside the close reply, lines.
const SEAT_LOG_KEEP: usize = 64;

/// `seat.open` request: fighters and frames of the spawned window.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SeatOpenArgs {
    #[serde(default)]
    pub fighters: Option<u8>,
    #[serde(default)]
    pub frames: Option<u32>,
}

/// `seat.open` answer: the bound port, the bout and the first tick to author.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SeatOpenOutput {
    pub port: u16,
    pub fighters: u8,
    pub frames: u32,
    pub tick: u32,
    pub exe: String,
    pub boot: String,
}

/// `seat.step` request: one validated fighter input for one fixed tick.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SeatStepArgs {
    pub tick: u32,
    /// Wish x in percent of top speed, `-100..=100` (the range the handler takes).
    #[schemars(range(min = -100, max = 100))]
    pub mx: i8,
    /// Wish z in percent of top speed, `-100..=100` (the range the handler takes).
    #[schemars(range(min = -100, max = 100))]
    pub mz: i8,
    pub facing: i16,
    #[serde(default)]
    pub buttons: u8,
}

/// `seat.step` answer: that tick's match hash, the last view line and the
/// door's invalid-action count.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SeatStepOutput {
    pub tick: u32,
    pub hash: String,
    pub view: Option<String>,
    pub invalid: u32,
    pub steps: u32,
}

/// `seat.close` request: no fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct SeatCloseArgs {}

/// `seat.close` answer: the run's counters and the arena's own seat receipt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SeatCloseOutput {
    pub closed: bool,
    pub port: u16,
    pub tick: u32,
    pub steps: u32,
    pub invalid: u32,
    pub receipt: Option<String>,
}

/// One refused line from the seat wire.
#[derive(Debug)]
enum WireError {
    /// The arena closed the socket or a write failed.
    Closed,
    /// No line arrived inside [`SEAT_TICK_TIMEOUT`].
    Timeout,
    /// A tick line outside `tick <n> <hash:016x>`.
    BadLine,
    /// The window ran past the tick before its input arrived.
    Missed(u32),
}

/// The socket half of a seat: `input` lines out, `tick` and `view` lines in.
struct SeatWire {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    /// Latest `view <arena view: ...>` line, kept for the next replies.
    view: Option<String>,
}

impl SeatWire {
    fn connect(port: u16) -> std::io::Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(SEAT_TICK_TIMEOUT))?;
        stream.set_write_timeout(Some(SEAT_TICK_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            view: None,
        })
    }

    /// One client line: `input <tick> <mx> <mz> <facing> <buttons>`.
    fn author(&mut self, input: ParticipantInput) -> Result<(), WireError> {
        let line = format!(
            "input {} {} {} {} {}\n",
            input.tick, input.mx, input.mz, input.facing, input.buttons.0
        );
        self.stream
            .write_all(line.as_bytes())
            .and_then(|()| self.stream.flush())
            .map_err(|_| WireError::Closed)
    }

    /// Read until `tick <n> <hash>` for `tick`. View lines are kept; a later
    /// tick means the window ran past the input.
    fn wait_tick(&mut self, tick: u32) -> Result<u64, WireError> {
        let mut line = String::new();
        for _ in 0..SEAT_MAX_LINES {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return Err(WireError::Closed),
                Ok(_) => {}
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(WireError::Timeout);
                }
                Err(_) => return Err(WireError::Closed),
            }
            if let Some(view) = line.strip_prefix("view ") {
                self.view = Some(view.trim_end().to_owned());
                continue;
            }
            let Some(rest) = line.strip_prefix("tick ") else {
                continue;
            };
            let (seen, hash) = parse_tick_line(rest).ok_or(WireError::BadLine)?;
            if seen == tick {
                return Ok(hash);
            }
            if seen > tick {
                return Err(WireError::Missed(seen));
            }
        }
        Err(WireError::Timeout)
    }

    /// Let the seat's reader end: `quit` then close the socket.
    fn quit(&mut self) {
        let _ = self.stream.write_all(b"quit\n");
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// One live seat: the spawned arena, its stdout tail and the socket.
struct SeatHost {
    child: Option<Child>,
    wire: SeatWire,
    port: u16,
    fighters: u8,
    frames: u32,
    steps: u32,
    invalid: u32,
    /// Tick the window runs next: the last published tick plus one.
    next_tick: u32,
    log: Arc<Mutex<Vec<String>>>,
}

impl SeatHost {
    /// Close the socket and kill the window. Idempotent; `Drop` calls it.
    fn kill(&mut self) {
        self.wire.quit();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// The arena's own run receipt, once its run ended. The boot line also
    /// starts `arena seat:`, so only the `port=` form counts: a killed window
    /// prints no receipt, and the boot line is not one.
    fn receipt(&self) -> Option<String> {
        const RECEIPT_HEAD: &str = "arena seat: port=";
        lock(&self.log)
            .iter()
            .rev()
            .find(|line| line.starts_with(RECEIPT_HEAD))
            .cloned()
    }
}

impl Drop for SeatHost {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The session's one live seat. The door is one process per session, so this
/// process-wide slot is that session's seat; a second `seat.open` refuses
/// until `seat.close` takes it. A door that exits without `seat.close` leaves
/// the window to its own `--exit` frame bound.
static SEAT: Mutex<Option<SeatHost>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn seat_already_open() -> McpToolError {
    McpToolError::new(
        "seat_already_open",
        "a seat is already open in this session; call seat.close first, then re-run seat.open",
    )
}

fn no_seat() -> McpToolError {
    McpToolError::new(
        "no_seat",
        "no live seat in this session; call seat.open first, then re-run",
    )
}

/// The port in the arena's boot line, once it is listening.
fn boot_port(line: &str) -> Option<u16> {
    let rest = line.strip_prefix(SEAT_BOOT_MARK)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `tick <n> <hash:016x>` fields (the `tick ` head already stripped).
fn parse_tick_line(rest: &str) -> Option<(u32, u64)> {
    let mut parts = rest.split_whitespace();
    let tick: u32 = parts.next()?.parse().ok()?;
    let hash = parts.next()?;
    if parts.next().is_some() || hash.len() != 16 {
        return None;
    }
    let hash = u64::from_str_radix(hash, 16).ok()?;
    Some((tick, hash))
}

/// The last non-empty lines of the arena's log, for a loud refusal.
fn log_tail(log: &Mutex<Vec<String>>, lines: usize) -> String {
    let log = lock(log);
    let tail: Vec<&str> = log
        .iter()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(lines)
        .map(String::as_str)
        .collect();
    if tail.is_empty() {
        "no output".to_owned()
    } else {
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    }
}

/// Read one child pipe line by line into the bounded log, off the door's
/// thread, so a chatty window never blocks on a full pipe.
fn pump_log<R: std::io::Read + Send + 'static>(pipe: R, log: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let mut log = lock(&log);
                    if log.len() >= SEAT_LOG_KEEP {
                        log.remove(0);
                    }
                    log.push(line.trim_end().to_owned());
                }
            }
        }
    });
}

/// Spawn one arena window and connect to its seat. The boot line names the
/// port the OS bound; a window that never reaches it fails loud with its log.
fn seat_spawn(exe: &Path, fighters: u8, frames: u32) -> Result<SeatHost, McpToolError> {
    if !exe.is_file() {
        return Err(McpToolError::new(
            "arena_exe_missing",
            format!(
                "no arena binary at `{}`; build one with \
                 `cargo build -p engine-game --release --features app --bin arena` or set \
                 {} to a built arena.exe, then re-run",
                exe.display(),
                super::view::ARENA_EXE_ENV
            ),
        ));
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut child = Command::new(exe)
        .args(["--scene", "arena", "--fighters", &fighters.to_string()])
        .args([
            "--frames",
            &frames.to_string(),
            "--exit",
            "--agent-seat",
            "0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            McpToolError::new(
                "seat_start_failed",
                format!(
                    "`{}` did not start ({err}); re-run with a built arena.exe",
                    exe.display()
                ),
            )
        })?;
    if let Some(stdout) = child.stdout.take() {
        pump_log(stdout, Arc::clone(&log));
    }
    if let Some(stderr) = child.stderr.take() {
        pump_log(stderr, Arc::clone(&log));
    }
    let port = wait_boot(&mut child, &log)?;
    let wire = SeatWire::connect(port).map_err(|err| {
        let _ = child.kill();
        let _ = child.wait();
        McpToolError::new(
            "seat_start_failed",
            format!(
                "the seat booted on port {port} but the door could not connect ({err}); \
                 re-run, and check that nothing else holds the port"
            ),
        )
    })?;
    Ok(SeatHost {
        child: Some(child),
        wire,
        port,
        fighters,
        frames,
        steps: 0,
        invalid: 0,
        next_tick: 0,
        log,
    })
}

/// Poll the arena's log for its boot line while the child lives.
fn wait_boot(child: &mut Child, log: &Mutex<Vec<String>>) -> Result<u16, McpToolError> {
    let started = Instant::now();
    loop {
        if let Some(port) = lock(log).iter().find_map(|line| boot_port(line)) {
            return Ok(port);
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(McpToolError::new(
                "seat_start_failed",
                format!(
                    "the arena exited {status} before its seat listened: {}; re-run with a \
                     working window (the seat needs a display)",
                    log_tail(log, 3)
                ),
            ));
        }
        if started.elapsed() >= SEAT_BOOT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(McpToolError::new(
                "seat_start_failed",
                format!(
                    "the arena printed no seat line within {} s: {}; re-run with a built \
                     arena.exe",
                    SEAT_BOOT_TIMEOUT.as_secs(),
                    log_tail(log, 3)
                ),
            ));
        }
        std::thread::sleep(SEAT_BOOT_POLL);
    }
}

/// One `seat.open` on an explicit binary: the test seam and the one owner of
/// the argument ranges, the spawn and the boot wait.
fn seat_open_core(
    exe: &Path,
    args: &SeatOpenArgs,
) -> Result<(SeatHost, SeatOpenOutput), McpToolError> {
    let fighters = args.fighters.unwrap_or(2);
    let frames = args.frames.unwrap_or(600);
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(bad_args(format!(
            "fighters {fighters} is outside 2..={MAX_PARTICIPANTS}; re-run in range"
        )));
    }
    if frames == 0 || frames > MAX_SEAT_FRAMES {
        return Err(bad_args(format!(
            "frames {frames} is outside 1..={MAX_SEAT_FRAMES}; re-run in range"
        )));
    }
    let host = seat_spawn(exe, fighters, frames)?;
    let boot = lock(&host.log)
        .iter()
        .rev()
        .find(|line| boot_port(line).is_some())
        .cloned()
        .unwrap_or_default();
    let out = SeatOpenOutput {
        port: host.port,
        fighters: host.fighters,
        frames: host.frames,
        tick: host.next_tick,
        exe: exe.display().to_string(),
        boot,
    };
    Ok((host, out))
}

/// One validated input from the request, or the field that failed.
fn seat_action(args: &SeatStepArgs) -> Result<ParticipantInput, ParticipantError> {
    let input = ParticipantInput {
        tick: args.tick,
        fighter: PARTICIPANT_ZERO,
        mx: args.mx,
        mz: args.mz,
        facing: args.facing,
        buttons: Buttons(args.buttons),
    };
    input.validate()?;
    Ok(input)
}

/// Names the refused field and the range the handler takes for a wish
/// outside it; any other contract breach keeps its raw refusal text.
fn refusal_detail(args: &SeatStepArgs, why: &ParticipantError) -> String {
    match why {
        ParticipantError::WishOutOfRange(_) => {
            let (axis, value) = if (-WISH_RANGE..=WISH_RANGE).contains(&args.mx) {
                ("mz", args.mz)
            } else {
                ("mx", args.mx)
            };
            format!("{axis} {value} is outside -{WISH_RANGE}..={WISH_RANGE}")
        }
        other => format!("{other:?}"),
    }
}

/// One `seat.step` on a live host: validate, author, read the tick's hash.
fn seat_step_core(
    host: &mut SeatHost,
    args: &SeatStepArgs,
) -> Result<SeatStepOutput, McpToolError> {
    let input = match seat_action(args) {
        Ok(input) => input,
        Err(why) => {
            host.invalid = host.invalid.saturating_add(1);
            return Err(McpToolError::new(
                "invalid_action",
                format!(
                    "input refused ({}); the seat counted it invalid and sent nothing; \
                     re-run in range",
                    refusal_detail(args, &why)
                ),
            ));
        }
    };
    if input.tick < host.next_tick {
        host.invalid = host.invalid.saturating_add(1);
        return Err(McpToolError::new(
            "seat_late_input",
            format!(
                "tick {} is behind the window, which runs tick {} next; re-run with tick {}",
                input.tick, host.next_tick, host.next_tick
            ),
        ));
    }
    if host.wire.author(input).is_err() {
        host.kill();
        return Err(McpToolError::new(
            "seat_closed",
            "the arena seat closed before the input crossed; re-run seat.open",
        ));
    }
    match host.wire.wait_tick(input.tick) {
        Ok(hash) => {
            host.next_tick = input.tick.saturating_add(1);
            host.steps = host.steps.saturating_add(1);
            Ok(SeatStepOutput {
                tick: input.tick,
                hash: format!("{hash:016x}"),
                view: host.wire.view.clone(),
                invalid: host.invalid,
                steps: host.steps,
            })
        }
        Err(WireError::Missed(seen)) => {
            host.invalid = host.invalid.saturating_add(1);
            host.next_tick = seen.saturating_add(1);
            Err(McpToolError::new(
                "seat_missed_tick",
                format!(
                    "the window reached tick {seen} before tick {} was fed; re-run with tick {}",
                    input.tick, host.next_tick
                ),
            ))
        }
        Err(WireError::Timeout) => Err(McpToolError::new(
            "seat_timeout",
            format!(
                "no tick line for tick {} within {} s; re-run against a running window",
                input.tick,
                SEAT_TICK_TIMEOUT.as_secs()
            ),
        )),
        Err(WireError::BadLine) => Err(McpToolError::new(
            "seat_protocol",
            "the arena sent a line outside `tick <n> <hash:016x>`; restart the door and re-run",
        )),
        Err(WireError::Closed) => {
            host.kill();
            Err(McpToolError::new(
                "seat_closed",
                "the arena closed the seat before the tick line arrived; re-run seat.open",
            ))
        }
    }
}

/// One `seat.close`: take the counters, close the socket and kill the window.
fn seat_close_core(mut host: SeatHost) -> SeatCloseOutput {
    let receipt = host.receipt();
    let out = SeatCloseOutput {
        closed: true,
        port: host.port,
        tick: host.next_tick,
        steps: host.steps,
        invalid: host.invalid,
        receipt,
    };
    host.kill();
    out
}

fn seat_open(service: &McpServer, args: &SeatOpenArgs) -> Result<SeatOpenOutput, McpToolError> {
    service.dispatch(TOOL_SEAT_OPEN, |_state| {
        let mut slot = lock(&SEAT);
        if slot.is_some() {
            return Err(seat_already_open());
        }
        let (host, out) = seat_open_core(&super::view::arena_exe_path(), args)?;
        *slot = Some(host);
        Ok(out)
    })
}

fn seat_step(service: &McpServer, args: &SeatStepArgs) -> Result<SeatStepOutput, McpToolError> {
    service.dispatch(TOOL_SEAT_STEP, |_state| {
        let mut slot = lock(&SEAT);
        let Some(host) = slot.as_mut() else {
            return Err(no_seat());
        };
        seat_step_core(host, args)
    })
}

fn seat_close(service: &McpServer, _args: &SeatCloseArgs) -> Result<SeatCloseOutput, McpToolError> {
    service.dispatch(TOOL_SEAT_CLOSE, |_state| {
        let mut slot = lock(&SEAT);
        let Some(host) = slot.take() else {
            return Err(no_seat());
        };
        Ok(seat_close_core(host))
    })
}

crate::e2_tool!(
    /// `seat.open` tool: spawn the arena window and open its seat. Writes.
    SeatOpenTool,
    name = TOOL_SEAT_OPEN,
    description = "Open the live arena seat: spawns arena.exe --scene arena --fighters N --frames M --exit --agent-seat 0, reads its seat port and connects. E2040_ARENA_EXE or target/release/arena.exe. One seat per session; close it before opening another.",
    readonly = false,
    args = SeatOpenArgs,
    output = SeatOpenOutput,
    invoke = seat_open
);

crate::e2_tool!(
    /// `seat.step` tool: author one tick and read its hash. Writes.
    SeatStepTool,
    name = TOOL_SEAT_STEP,
    description = "Author slot 0's input for one fixed tick of the live arena seat: tick, mx, mz, facing, buttons in (FighterInput::validate); that tick's match hash, the latest arena view line, the step count and the invalid-action count out. Keep up with the window: a tick the window already ran is refused and counted.",
    readonly = false,
    args = SeatStepArgs,
    output = SeatStepOutput,
    invoke = seat_step
);

crate::e2_tool!(
    /// `seat.close` tool: close the seat and kill its window. Writes.
    SeatCloseTool,
    name = TOOL_SEAT_CLOSE,
    description = "Close the live arena seat: closes the socket, kills the window, and returns the tick, step and invalid-action counters plus the arena's own arena seat receipt when the run printed one.",
    readonly = false,
    args = SeatCloseArgs,
    output = SeatCloseOutput,
    invoke = seat_close
);

/// Longest hex a tape can carry: the header plus [`MAX_ROWS`](engine_ai::decision_log::MAX_ROWS) rows.
const MAX_TAPE_HEX: usize = 2 * engine_ai::bt_trace::MAX_TAPE_BYTES;

/// Most trace rows one `trace` answer prints before it says how many it cut.
const MAX_TRACE_ROWS_OUT: usize = 400;

/// Request: one decision tape as hex, optionally a second tape and a mode.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AiDecisionsArgs {
    /// Left tape: canonical decision-log bytes as hex.
    pub left: String,
    /// Right tape for `diff`; omit for `show`.
    #[serde(default)]
    pub right: Option<String>,
    /// `show` (default without a right tape), `diff` (needs a right tape) or
    /// `trace` (`left` is a `BtTrace` tape, not a decision tape).
    #[serde(default)]
    pub mode: Option<String>,
    /// `trace` only: fighter slot to show; omit for every slot.
    #[serde(default)]
    pub slot: Option<u8>,
    /// `trace` only: first tick (default 0).
    #[serde(default)]
    pub from: Option<u32>,
    /// `trace` only: last tick (default the end).
    #[serde(default)]
    pub to: Option<u32>,
    /// `trace` only: `tree` (default, indented) or `lines` (one line per node).
    #[serde(default)]
    pub format: Option<String>,
}

/// One decision row on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AiDecisionsRow {
    /// Evaluation tick.
    pub tick: u32,
    /// Fighter slot.
    pub fighter: u8,
    /// Chosen lane name (`attack`, `block`, `dodge`, `advance`, `retreat`, `reposition`).
    pub chosen: String,
    /// Utility score per lane, in action order.
    pub scores: [f32; 5],
}

/// First divergent row of two tapes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AiDecisionsDiff {
    /// First tick the tapes disagree.
    pub tick: u32,
    /// Fighter of the differing row.
    pub fighter: u8,
    /// Left row, when the left tape has one at this tick.
    pub left: Option<AiDecisionsRow>,
    /// Right row, when the right tape has one at this tick.
    pub right: Option<AiDecisionsRow>,
    /// Absolute chosen-score gap; 0 when one side has no row.
    pub gap: f32,
    /// One printable line naming the tick, both rows and the gap.
    pub line: String,
}

/// Answer: the mode served, row count and either the show text or the diff.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AiDecisionsOutput {
    /// Mode served: `show`, `diff` or `trace`.
    pub mode: String,
    /// Rows in the left tape.
    pub rows: u32,
    /// Show text, one line per row (`show` only).
    pub show: Option<String>,
    /// Trace text for the slot and ticks asked (`trace` only).
    pub trace: Option<String>,
    /// First divergence (`diff` only).
    pub diff: Option<AiDecisionsDiff>,
    /// True when `diff` found a divergence.
    pub divergent: bool,
}

fn tape_hex(side: &str, text: &str) -> Result<Vec<u8>, McpToolError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(bad_args(format!("{side} tape is empty")));
    }
    if text.len() > MAX_TAPE_HEX {
        return Err(bad_args(format!(
            "{side} tape is {} hex chars, cap is {MAX_TAPE_HEX}",
            text.len()
        )));
    }
    if !text.len().is_multiple_of(2) {
        return Err(bad_args(format!(
            "{side} tape has an odd hex length {}",
            text.len()
        )));
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut at = 0;
    while at < bytes.len() {
        let hi = hex_nibble(side, bytes[at], at)?;
        let lo = hex_nibble(side, bytes[at + 1], at + 1)?;
        out.push((hi << 4) | lo);
        at += 2;
    }
    Ok(out)
}

fn hex_nibble(side: &str, byte: u8, at: usize) -> Result<u8, McpToolError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(bad_args(format!(
            "{side} tape has a non-hex byte at char {at}"
        ))),
    }
}

fn tape(side: &str, text: &str) -> Result<DecisionLog, McpToolError> {
    let bytes = tape_hex(side, text)?;
    DecisionLog::from_bytes(&bytes).map_err(|error| {
        McpToolError::new("bad_tape", format!("{side} tape refused: {error}"))
            .with_data(rmcp::serde_json::json!({ "side": side, "cause": error.to_string() }))
    })
}

fn decision_row(decision: &Decision) -> AiDecisionsRow {
    AiDecisionsRow {
        tick: decision.tick,
        fighter: decision.fighter,
        chosen: decision.chosen_name().to_owned(),
        scores: decision.scores,
    }
}

/// `trace` mode: decode a `BtTrace` tape and print the slot and tick window.
fn ai_trace_core(args: &AiDecisionsArgs) -> Result<AiDecisionsOutput, McpToolError> {
    let bytes = tape_hex("left", &args.left)?;
    let trace = engine_ai::bt_trace::BtTrace::from_bytes(&bytes).map_err(|error| {
        McpToolError::new("bad_tape", format!("left tape refused: {error}"))
            .with_data(rmcp::serde_json::json!({ "side": "left", "cause": error.to_string() }))
    })?;
    let from = args.from.unwrap_or(0);
    let to = args.to.unwrap_or(u32::MAX);
    if from > to {
        return Err(bad_args(format!("from {from} is after to {to}")));
    }
    let text = match args.format.as_deref().unwrap_or("tree") {
        "tree" => trace.tree_text(args.slot, from, to, MAX_TRACE_ROWS_OUT),
        "lines" => trace.lines_text(args.slot, from, to, MAX_TRACE_ROWS_OUT),
        other => {
            return Err(bad_args(format!(
                "format `{other}` is not `tree` or `lines`"
            )))
        }
    };
    Ok(AiDecisionsOutput {
        mode: "trace".to_owned(),
        rows: u32::try_from(trace.len()).unwrap_or(u32::MAX),
        show: None,
        trace: Some(text),
        diff: None,
        divergent: false,
    })
}

fn ai_decisions_core(args: &AiDecisionsArgs) -> Result<AiDecisionsOutput, McpToolError> {
    if args.mode.as_deref() == Some("trace") {
        return ai_trace_core(args);
    }
    let left = tape("left", &args.left)?;
    let rows = u32::try_from(left.len()).unwrap_or(u32::MAX);
    let mode =
        args.mode
            .as_deref()
            .unwrap_or_else(|| if args.right.is_some() { "diff" } else { "show" });
    match mode {
        "show" => Ok(AiDecisionsOutput {
            mode: "show".to_owned(),
            rows,
            show: Some(left.show_text()),
            trace: None,
            diff: None,
            divergent: false,
        }),
        "diff" => {
            let right_text = args
                .right
                .as_deref()
                .ok_or_else(|| bad_args("mode `diff` needs a right tape".to_owned()))?;
            let right = tape("right", right_text)?;
            let found = engine_ai::decision_log::diff(&left, &right);
            Ok(AiDecisionsOutput {
                mode: "diff".to_owned(),
                rows,
                show: None,
                trace: None,
                diff: found.map(|found| AiDecisionsDiff {
                    tick: found.tick,
                    fighter: found.fighter,
                    left: found.left.as_ref().map(decision_row),
                    right: found.right.as_ref().map(decision_row),
                    gap: found.gap,
                    line: found.line(),
                }),
                divergent: found.is_some(),
            })
        }
        other => Err(bad_args(format!(
            "mode `{other}` is not `show`, `diff` or `trace`"
        ))),
    }
}

fn ai_decisions(
    _service: &McpServer,
    args: &AiDecisionsArgs,
) -> Result<AiDecisionsOutput, McpToolError> {
    ai_decisions_core(args)
}

crate::e2_tool!(
    /// `ai.decisions` tool: show one decision tape or diff two. Read-only.
    AiDecisionsTool,
    name = TOOL_AI_DECISIONS,
    description = "Show one AI decision tape, diff two, or trace a behavior tree: feed canonical bytes as hex. show prints one line per decision row (tick, fighter, chosen lane, five utility scores); diff returns the first divergent tick and the chosen-score gap; trace (left is a BtTrace tape) prints which node ran, its status, the state or blackboard value it read and the utility lanes, for a slot and tick range (format tree or lines). Off by default; enable with Brain::record_decisions or BtTree::tick_with.",
    readonly = true,
    args = AiDecisionsArgs,
    output = AiDecisionsOutput,
    invoke = ai_decisions
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }
    fn script() -> Vec<PlayInputDto> {
        vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
            PlayInputDto {
                tick: 30,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
        ]
    }
    fn until_args(cap: u32) -> PlayStepUntilArgs {
        PlayStepUntilArgs {
            seed: Some(0),
            participants: Some(2),
            half_extent_m: Some(20.0),
            inputs: Some(script()),
            tick_cap: Some(cap),
            fighter: Some(1),
            hp_below: Some(900),
        }
    }
    #[test]
    fn play_step_until_hp_below_900_stops_on_the_first_such_tick() {
        let server = server();
        let first = PlayStepUntilTool::invoke(&server, until_args(60)).expect("serves");
        assert!(first.stopped, "seed 0 hurts below 900: {first:?}");
        assert!((1..=60).contains(&first.tick));
        assert!(first.fighter_hp < 900);
        assert_ne!(first.digest, 0);
        assert_eq!(
            first,
            PlayStepUntilTool::invoke(&server, until_args(60)).expect("replays")
        );
        let early = step_until_report(&until_args(first.tick - 1)).expect("early serves");
        assert!(!early.stopped, "before still holds: {early:?}");
    }
    #[test]
    fn play_step_until_is_listed_read_only_and_refuses_bad_input() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_PLAY_STEP_UNTIL)
            .expect("registers");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_PLAY_STEP_UNTIL.to_owned(),
            },
        )
        .expect("describe lists it");
        assert_eq!(described.path, TOOL_PLAY_STEP_UNTIL);
        assert!(described.docs.contains("hp"));
        for bad in [
            PlayStepUntilArgs {
                tick_cap: Some(0),
                ..until_args(60)
            },
            PlayStepUntilArgs {
                fighter: Some(5),
                ..until_args(60)
            },
            PlayStepUntilArgs {
                hp_below: Some(1001),
                ..until_args(60)
            },
        ] {
            assert!(step_until_report(&bad)
                .expect_err("refuses")
                .message
                .contains("re-run"));
        }
        assert_eq!(
            step_until_report(&PlayStepUntilArgs {
                tick_cap: None,
                ..until_args(60)
            })
            .expect_err("missing")
            .code,
            "bad_ticks"
        );
    }

    // ---- seat.open / seat.step / seat.close ------------------------------

    use std::net::TcpListener;

    /// A fake AIN1 seat: reads `input` lines and answers in the arena's order
    /// (the tick line, then the view line every second tick). Returns the bound
    /// port and the lines it saw.
    fn fake_seat() -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fake seat binds");
        let port = listener.local_addr().expect("fake seat addr").port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("door connects");
            let mut writer = stream.try_clone().expect("fake seat writes");
            let mut reader = BufReader::new(stream);
            let mut seen = Vec::new();
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Some(tick) = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|tick| tick.parse::<u32>().ok())
                else {
                    continue;
                };
                seen.push(line.trim_end().to_owned());
                // The client reads the tick line and may hang up before the
                // view line lands; a closed connection ends the fake seat's
                // session, it is not a failure of the seat.
                if writeln!(writer, "tick {tick} {:016x}", 0xABCD_0000 + u64::from(tick)).is_err() {
                    break;
                }
                if tick % 2 == 0
                    && writeln!(writer, "view arena view: frames={tick} bodies=2").is_err()
                {
                    break;
                }
            }
            seen
        });
        (port, handle)
    }

    fn fake_host(port: u16) -> SeatHost {
        SeatHost {
            child: None,
            wire: SeatWire::connect(port).expect("wire connects"),
            port,
            fighters: 2,
            frames: 600,
            steps: 0,
            invalid: 0,
            next_tick: 0,
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn step_args(tick: u32, mx: i8) -> SeatStepArgs {
        SeatStepArgs {
            tick,
            mx,
            mz: 0,
            facing: 0,
            buttons: 1,
        }
    }

    #[test]
    fn seat_boot_port_and_tick_lines_parse() {
        assert_eq!(
            boot_port("arena seat: listening on 127.0.0.1:52341; the client owns slot 0"),
            Some(52341)
        );
        assert_eq!(boot_port("arena seat: port=1 clients=0"), None);
        assert_eq!(boot_port("arena smoke: ticks=0"), None);
        assert_eq!(
            parse_tick_line("20 dcc10f28eaabb591"),
            Some((20, 0xdcc1_0f28_eaab_b591))
        );
        for bad in [
            "20",
            "20 zz",
            "20 0123456789abcdef extra",
            "x 0123456789abcdef",
        ] {
            assert_eq!(parse_tick_line(bad), None, "{bad} is not a tick line");
        }
    }

    #[test]
    fn seat_receipt_names_only_the_run_line() {
        let (port, server) = fake_seat();
        let mut host = fake_host(port);
        lock(&host.log).push(
            "arena seat: listening on 127.0.0.1:1; the client owns slot 0 and the keyboard is ignored"
                .to_owned(),
        );
        assert!(
            host.receipt().is_none(),
            "the boot line is not the run receipt"
        );
        lock(&host.log).push(
            "arena seat: port=1 clients=1 inputs=0 late=0 superseded=0 held=0 invalid=0 dropped=0"
                .to_owned(),
        );
        assert_eq!(
            host.receipt().as_deref(),
            Some(
                "arena seat: port=1 clients=1 inputs=0 late=0 superseded=0 held=0 invalid=0 dropped=0"
            )
        );
        host.kill();
        server.join().expect("fake seat ends");
    }

    #[test]
    fn seat_wire_authors_and_reads_hashes_against_a_fake_seat() {
        let (port, server) = fake_seat();
        let mut wire = SeatWire::connect(port).expect("wire connects");
        wire.author(seat_action(&step_args(0, 50)).expect("valid"))
            .expect("tick 0 crosses");
        let zero = wire.wait_tick(0).expect("tick 0 answers");
        assert_eq!(zero, 0xABCD_0000);
        assert!(wire.view.is_none(), "the view follows its tick line");
        wire.author(seat_action(&step_args(1, 0)).expect("valid"))
            .expect("tick 1 crosses");
        assert_eq!(wire.wait_tick(1).expect("tick 1 answers"), 0xABCD_0001);
        assert_eq!(
            wire.view.as_deref(),
            Some("arena view: frames=0 bodies=2"),
            "the tick-0 view crossed before tick 1"
        );
        // A tick the window ran past is named, never mistaken for the answer.
        wire.author(seat_action(&step_args(5, 0)).expect("valid"))
            .expect("tick 5 crosses");
        assert!(matches!(wire.wait_tick(4), Err(WireError::Missed(5))));
        let seen = {
            wire.quit();
            server.join().expect("fake seat ends")
        };
        assert_eq!(
            seen,
            ["input 0 50 0 0 1", "input 1 0 0 0 1", "input 5 0 0 0 1"],
            "the wire speaks the documented input line"
        );
    }

    #[test]
    fn seat_step_schema_states_the_wish_range_the_handler_takes() {
        use rmcp::handler::server::router::tool::ToolBase;
        let schema = SeatStepTool::input_schema().expect("seat.step carries an input schema");
        let text = rmcp::serde_json::to_string(&*schema).expect("schema serializes");
        let value: rmcp::serde_json::Value =
            rmcp::serde_json::from_str(&text).expect("schema is JSON");
        for axis in ["mx", "mz"] {
            let field = &value["properties"][axis];
            assert_eq!(field["minimum"], -100, "{axis} minimum: {text}");
            assert_eq!(field["maximum"], 100, "{axis} maximum: {text}");
        }
    }

    #[test]
    fn seat_step_counts_invalid_and_late_actions_on_a_live_host() {
        let (port, server) = fake_seat();
        let mut host = fake_host(port);
        let refused = seat_step_core(&mut host, &step_args(0, 127)).expect_err("mx 127 refuses");
        assert_eq!(refused.code, "invalid_action");
        assert!(
            refused.message.contains("counted it invalid"),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("mx 127 is outside -100..=100"),
            "the refusal names the field and the range: {}",
            refused.message
        );
        assert_eq!(host.invalid, 1, "the refused action is counted");
        let ok = seat_step_core(&mut host, &step_args(0, 50)).expect("tick 0 serves");
        assert_eq!(ok.tick, 0);
        assert_eq!(ok.hash, "00000000abcd0000");
        assert_eq!((ok.invalid, ok.steps), (1, 1));
        let late = seat_step_core(&mut host, &step_args(0, 0)).expect_err("tick 0 is behind");
        assert_eq!(late.code, "seat_late_input");
        assert_eq!(host.invalid, 2);
        let closed = seat_close_core(host);
        assert!(closed.closed);
        assert_eq!((closed.steps, closed.invalid, closed.tick), (1, 2, 1));
        assert!(
            closed.receipt.is_none(),
            "a killed window prints no receipt"
        );
        let seen = server.join().expect("fake seat ends");
        assert_eq!(
            seen,
            ["input 0 50 0 0 1"],
            "the refused inputs never crossed"
        );
    }

    #[test]
    fn seat_verbs_register_and_refuse_without_a_seat() {
        let server = server();
        for (name, needle) in [
            (TOOL_SEAT_OPEN, "agent-seat 0"),
            (TOOL_SEAT_STEP, "invalid-action count"),
            (TOOL_SEAT_CLOSE, "arena seat receipt"),
        ] {
            let verb = PackRegistry::shipped()
                .verb(name)
                .expect("the seat verb registers");
            assert_eq!(verb.pack(), PACK);
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(!verb.readonly(), "{name} spawns or drives a window");
            let described = ApiDescribeTool::invoke(
                &server,
                ApiDescribeArgs {
                    path: name.to_owned(),
                },
            )
            .expect("api.describe lists the seat verb");
            assert_eq!(described.path, name);
            assert!(described.docs.contains(needle), "{}", described.docs);
        }
        assert_eq!(
            SeatStepTool::invoke(&server, step_args(0, 0))
                .expect_err("nothing open")
                .code,
            "no_seat"
        );
        assert_eq!(
            SeatCloseTool::invoke(&server, SeatCloseArgs {})
                .expect_err("nothing open")
                .code,
            "no_seat"
        );
        // The missing-binary path is loud and names the path it tried.
        let exe = Path::new("target/no-such-build/arena.exe");
        let Err(refused) = seat_open_core(
            exe,
            &SeatOpenArgs {
                fighters: Some(2),
                frames: Some(60),
            },
        ) else {
            panic!("no binary there, so the open must refuse");
        };
        assert_eq!(refused.code, "arena_exe_missing");
        assert!(
            refused.message.contains(&exe.display().to_string()),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("re-run"), "{}", refused.message);
        // Ranges refuse before any process is spawned.
        for bad in [
            SeatOpenArgs {
                fighters: Some(1),
                frames: Some(60),
            },
            SeatOpenArgs {
                fighters: Some(2),
                frames: Some(0),
            },
        ] {
            let Err(refused) = seat_open_core(exe, &bad) else {
                panic!("bad range {bad:?} must refuse");
            };
            assert_eq!(refused.code, "bad_args");
        }
    }

    fn decision_hex(rows: &[(u32, u8, [f32; 5])]) -> String {
        let mut log = DecisionLog::new(8);
        for (tick, chosen, scores) in rows {
            log.push(Decision {
                tick: *tick,
                fighter: 1,
                scores: *scores,
                chosen: *chosen,
            });
        }
        let mut out = String::with_capacity(log.to_bytes().len() * 2);
        for byte in log.to_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    #[test]
    fn ai_decisions_show_lists_every_row() {
        let left = decision_hex(&[
            (10, 0, [9.0, 1.0, 0.0, 0.0, 0.0]),
            (20, 3, [0.0, 0.0, 0.0, 8.0, 1.0]),
        ]);
        let served = ai_decisions_core(&AiDecisionsArgs {
            left,
            right: None,
            mode: None,
            ..AiDecisionsArgs::default()
        })
        .expect("show serves");
        assert_eq!(served.mode, "show");
        assert_eq!(served.rows, 2);
        assert!(!served.divergent);
        let text = served.show.expect("show text");
        assert!(text.contains("tick=10"), "{text}");
        assert!(text.contains("chose=advance"), "{text}");
        assert!(text.contains("tick=20"), "{text}");
    }

    #[test]
    fn ai_decisions_diff_names_the_first_divergent_tick() {
        let left = decision_hex(&[
            (10, 0, [9.0, 1.0, 0.0, 0.0, 0.0]),
            (30, 1, [0.0, 7.0, 0.0, 0.0, 0.0]),
        ]);
        let right = decision_hex(&[
            (10, 0, [9.0, 1.0, 0.0, 0.0, 0.0]),
            (30, 1, [0.0, 4.0, 0.0, 0.0, 0.0]),
        ]);
        let served = ai_decisions_core(&AiDecisionsArgs {
            left,
            right: Some(right),
            mode: Some("diff".to_owned()),
            ..AiDecisionsArgs::default()
        })
        .expect("diff serves");
        assert!(served.divergent);
        let found = served.diff.expect("a divergence");
        assert_eq!(found.tick, 30);
        assert_eq!(found.fighter, 1);
        assert_eq!(found.left.expect("left row").scores[1], 7.0);
        assert_eq!(found.right.expect("right row").scores[1], 4.0);
        assert_eq!(found.gap, 3.0);
        assert!(found.line.contains("tick=30"), "{}", found.line);
    }

    #[test]
    fn ai_decisions_refuses_bad_hex_bad_tape_and_unknown_mode() {
        let odd = ai_decisions_core(&AiDecisionsArgs {
            left: "abc".to_owned(),
            right: None,
            mode: None,
            ..AiDecisionsArgs::default()
        })
        .expect_err("odd hex refuses");
        assert_eq!(odd.code(), "bad_args");
        assert!(odd.to_string().contains("odd"), "{odd}");
        let non_hex = ai_decisions_core(&AiDecisionsArgs {
            left: "zz".to_owned(),
            right: None,
            mode: None,
            ..AiDecisionsArgs::default()
        })
        .expect_err("non-hex refuses");
        assert_eq!(non_hex.code(), "bad_args");
        assert!(non_hex.to_string().contains("non-hex"), "{non_hex}");
        let short = ai_decisions_core(&AiDecisionsArgs {
            left: "0100".to_owned(),
            right: None,
            mode: None,
            ..AiDecisionsArgs::default()
        })
        .expect_err("short tape refuses");
        assert_eq!(short.code(), "bad_tape");
        let unknown = ai_decisions_core(&AiDecisionsArgs {
            left: decision_hex(&[(1, 0, [1.0, 0.0, 0.0, 0.0, 0.0])]),
            right: None,
            mode: Some("replay".to_owned()),
            ..AiDecisionsArgs::default()
        })
        .expect_err("unknown mode refuses");
        assert_eq!(unknown.code(), "bad_args");
        let missing = ai_decisions_core(&AiDecisionsArgs {
            left: decision_hex(&[(1, 0, [1.0, 0.0, 0.0, 0.0, 0.0])]),
            right: None,
            mode: Some("diff".to_owned()),
            ..AiDecisionsArgs::default()
        })
        .expect_err("diff needs a right tape");
        assert_eq!(missing.code(), "bad_args");
    }

    fn trace_hex() -> String {
        use engine_ai::bt::{guard_tree, pressure_tree};
        let mut trace = engine_ai::bt_trace::BtTrace::new(2048);
        let _ =
            engine_ai::bout::run_bout_traced(&pressure_tree(), &guard_tree(), 1, 30, 9, &mut trace);
        trace
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn trace_args(hex: String) -> AiDecisionsArgs {
        AiDecisionsArgs {
            left: hex,
            mode: Some("trace".to_owned()),
            ..AiDecisionsArgs::default()
        }
    }

    #[test]
    fn ai_decisions_trace_prints_the_tree_for_one_slot_and_tick_range() {
        let served = ai_decisions_core(&AiDecisionsArgs {
            slot: Some(1),
            from: Some(4),
            to: Some(5),
            ..trace_args(trace_hex())
        })
        .expect("trace serves");
        assert_eq!(served.mode, "trace");
        assert!(
            served.rows > 30,
            "the tape rows are counted: {}",
            served.rows
        );
        let text = served.trace.expect("trace text");
        assert!(
            text.starts_with(
                "tick 4 slot 1
"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "tick 5 slot 1
"
            ),
            "{text}"
        );
        assert!(!text.contains("slot 0"), "{text}");
        assert!(!text.contains("tick 6"), "{text}");
        assert!(text.contains("selector -> "), "{text}");
        assert!(text.contains("[dist_mm="), "{text}");
    }

    #[test]
    fn ai_decisions_trace_lines_format_and_refusals() {
        let lines = ai_decisions_core(&AiDecisionsArgs {
            format: Some("lines".to_owned()),
            slot: Some(0),
            from: Some(2),
            to: Some(2),
            ..trace_args(trace_hex())
        })
        .expect("lines serve")
        .trace
        .expect("text");
        assert!(lines.starts_with("t2 s0 #"), "{lines}");
        assert!(!lines.contains("tick 2 slot"), "{lines}");
        let backwards = ai_decisions_core(&AiDecisionsArgs {
            from: Some(9),
            to: Some(3),
            ..trace_args(trace_hex())
        })
        .expect_err("from after to refuses");
        assert_eq!(backwards.code(), "bad_args");
        let format = ai_decisions_core(&AiDecisionsArgs {
            format: Some("svg".to_owned()),
            ..trace_args(trace_hex())
        })
        .expect_err("unknown format refuses");
        assert_eq!(format.code(), "bad_args");
        let decision_tape_as_trace = ai_decisions_core(&trace_args(decision_hex(&[(
            1,
            0,
            [1.0, 0.0, 0.0, 0.0, 0.0],
        )])))
        .expect_err("a decision tape is not a trace tape");
        assert_eq!(decision_tape_as_trace.code(), "bad_tape");
    }

    #[test]
    fn ai_decisions_verb_is_listed_read_only() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_AI_DECISIONS)
            .expect("registers");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 12,
    name: PACK,
    default_loaded: false,
    tools: &[
        "play.step_until",
        "seat.open",
        "seat.step",
        "seat.close",
        "input.inject",
        "state.digest",
        "play.sweep",
        "tune.sweep",
        "ai.decisions",
    ],
    build: pack,
};
