//! MCP `play.step`: a live duel held across steps, one receipt per step.
//!
//! Consumer: the vision playtester over `e2040-mcp` (via [`PlayStepTool`] and
//! `ServerState::play_step`); a GPU host wires its capture callback once,
//! while it builds the session, with
//! [`ServerState::with_step_capture_hook`](crate::mcp::ServerState::with_step_capture_hook)
//! (or [`PlaySession::new_with_hook`]). Data owner for simulation is
//! `engine-arena`; this module owns only the step cursor, the accumulated
//! tape, the PNG receipt and the session's play slot ([`PlaySlot`]: the live
//! bout, the capture hook and the re-query cache, shared with the gym verbs
//! `sim.step` and `sim.observe`). Nothing here is process-wide: two sessions
//! never share a hook, a bout or a cache. Units follow arena
//! (meters, radians, permille, fixed ticks); receipts are sRGB PNG.
//! Determinism: same seed plus same tape twice gives the same digest, the
//! same numbers and the same PNG bytes; no threads. Wall time is opt-in
//! (`timing`) and receipt-only: it never enters a digest, a hash or the tick,
//! and a missing sample answers `unknown`, never zero. Errors
//! are typed `McpToolError` codes naming the legal values; a wired GPU hook
//! is validated like `capture_with`, and the software rasterizer never
//! answers `unsupported_capture`. Budget: one step simulates at most
//! `MAX_PLAY_TICKS` ticks and software-encodes at most 512x512 pixels;
//! allocation per step is fine (service path, never the fixed tick).
//!
//! ## Live bout
//!
//! A step names its whole tape prefix, so its answer is a pure function of
//! its arguments. The session holds one [`GymLive`] bout: when a step names
//! the same seed, headcount and extent and its tape agrees with what the bout
//! already consumed, only the new ticks are simulated; anything else rebuilds
//! from tick 0 and gives the same bytes. Stepping 20 ticks at a time over a
//! 600-tick bout costs 600 sim steps, not 6,300.
//!
//! ## Wire numbers
//!
//! Every `u64` that crosses the door (`digest`, `seed`) goes out as a
//! decimal JSON string, so a client that parses JSON numbers as f64 keeps it
//! exact (see [`dec_u64`]); input takes the string or a plain JSON number.
//!
//! ## Tick-label legend (the 0/1-index map)
//!
//! `upto_tick: N` simulates the `0..N` prefix: N ticks with 0-indexed tick
//! ids `0..=N-1`. The receipt reports the 1-indexed count `tick: N`, which
//! equals the `play.run` poll field `tick_count: N` and sits exactly one past
//! the last checkpoint tick id (`N - 1`). The digest is the last spine
//! checkpoint hash ([`spine_digest`]), so `play.step(upto_tick = N).digest`
//! equals
//! `play.run(ticks = N).final_hash` on the same seed, extent, participants and
//! tape. Inputs always carry 0-indexed tick ids inside the covered prefix;
//! [`PlaySession::cursor`] is the next uncovered tick id (0 before the first
//! step, N after covering `0..N`). A model that reads receipt `tick: 8` next
//! to checkpoint `tick: 7` is looking at the same state, not a skew.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt;

use engine_arena::gym::{GymEvent, GymLive};
use engine_arena::spine_step::SpineMatch;
use engine_arena::world::MatchWorld;
use engine_arena::{ArenaConfig, BoutTape, MAX_PARTICIPANTS};
use engine_core::{adler::adler32, crc32::crc32, hash::GOLDEN_GAMMA, splitmix64};
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
// `self` puts `serde` itself in scope: the schemars derive names it when a
// field has both `default` and a `serde(with)` module (see `dec_u64`).
use rmcp::serde::{self, Deserialize, Serialize};

use super::requery::{requery_key, RequeryCache, RequeryParams};
use super::{
    build_tape, capture_with, CaptureArgs, CaptureManifestDto, CaptureOutput, NoSample,
    PlayInputDto, PlayTimingDto, StepMillis, CAPTURE_DEFAULT_HEIGHT, CAPTURE_DEFAULT_WIDTH,
    CAPTURE_MAX_DIM, MAX_CAPTURE_BYTES, MAX_PLAY_INPUTS, MAX_PLAY_TICKS, PNG_MAGIC,
    TIMING_LABEL_SOFTWARE, TIMING_RING_CAP,
};
use crate::mcp::{McpServer, McpToolError};

/// Tool: live duel step (state summary plus a real capture PNG per step).
pub const TOOL_PLAY_STEP: &str = "play.step";
/// Software rasterizer pixel bound (512x512): stored-deflate receipts stay
/// under [`MAX_CAPTURE_BYTES`]; a wired GPU callback covers the full 2048.
const MAX_SOFTWARE_PIXELS: u64 = 512 * 512;

/// `#[serde(with = "dec_u64")]` for a `u64` hash, digest or seed: the wire
/// carries the exact decimal text (JS and LLM clients parse JSON numbers as
/// f64 and round anything above 2^53); input takes that text or a plain JSON
/// integer. Pair it with `#[schemars(with = "String")]` so the advertised
/// schema says string. `opt` is the `Option<u64>` form for optional fields
/// (with `#[serde(default)]`).
pub(crate) mod dec_u64 {
    use rmcp::serde::de::Error;
    use rmcp::serde::{Deserialize, Deserializer, Serializer};
    use rmcp::serde_json::Value;

    /// Write the value as decimal text.
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    /// Read a decimal string or a non-negative JSON integer.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        decode(&Value::deserialize(deserializer)?)
    }

    /// The one parse behind both forms; refusals name the legal shapes.
    fn decode<E: Error>(value: &Value) -> Result<u64, E> {
        let parsed = match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => text.parse::<u64>().ok(),
            _ => None,
        };
        parsed.ok_or_else(|| {
            E::custom(format!(
                "expected a u64 as a decimal string or a non-negative JSON integer, got {value}"
            ))
        })
    }

    /// `Option<u64>` form: `null` and a missing field are `None`.
    pub mod opt {
        use rmcp::serde::{Deserialize, Deserializer, Serializer};
        use rmcp::serde_json::Value;

        /// Write `Some` as decimal text and `None` as null.
        pub fn serialize<S: Serializer>(
            value: &Option<u64>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(number) => serializer.collect_str(number),
                None => serializer.serialize_none(),
            }
        }

        /// Read null, a decimal string or a non-negative JSON integer.
        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<u64>, D::Error> {
            match Option::<Value>::deserialize(deserializer)? {
                None | Some(Value::Null) => Ok(None),
                Some(value) => super::decode(&value).map(Some),
            }
        }
    }
}

/// Step request: seed, participants, extent, tick count, tape and receipt size.
/// Units are meters, ticks and pixels. Errors are typed `bad_ticks`,
/// `bad_extent` and tape refusals. Budget is at most 3600 ticks a step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepArgs {
    /// Match seed (a decimal string or a number); same seed plus same tape
    /// replays the same digest.
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    /// Participant count, 2..=`MAX_PARTICIPANTS`.
    #[serde(alias = "fighters")]
    pub participants: Option<u8>,
    /// Arena half-extent in meters, 5.0..=100.0, explicit (never defaulted
    /// across tools when comparing receipts).
    pub half_extent_m: Option<f32>,
    /// Ticks to simulate as the 1-indexed count N of the 0-indexed prefix
    /// `0..N` (1..=`MAX_PLAY_TICKS`); the digest is the checkpoint `N - 1`
    /// hash. See the module legend.
    pub upto_tick: Option<u32>,
    /// Tape inputs with 0-indexed tick ids inside `0..upto_tick`.
    pub inputs: Option<Vec<PlayInputDto>>,
    /// Receipt width in pixels, 1..=512 software.
    pub width: Option<u32>,
    /// Receipt height in pixels, 1..=512 software.
    pub height: Option<u32>,
    // Opt-in wall-time receipt: `true` adds a `timing` object (p50/p99 step
    // wall ms labelled `software`); absent or `false` reads no clock and
    // keeps the output byte-identical.
    pub timing: Option<bool>,
}

/// Step answer: tick count, state digest, the numbers and the PNG receipt.
/// Units are ticks, digest hashes, meters, radians, permille and PNG bytes.
/// No errors once simulated. Budget is one receipt of at most 512x512
/// software pixels.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepOutput {
    /// Ticks simulated: the 1-indexed count N. Equals the `play.run` poll
    /// field `tick_count` and sits one past the last checkpoint tick id
    /// (`N - 1`); see the module legend.
    pub tick: u32,
    /// State digest: the spine checkpoint `tick - 1` hash, equal to the
    /// `play.run` `final_hash` on the same seed, extent, participants and
    /// tape. A decimal string on the wire.
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    pub participants: u8,
    pub inputs_applied: usize,
    pub summary: String,
    pub manifest: CaptureManifestDto,
    pub png_bytes: Vec<u8>,
    /// Every fighter after tick `N - 1`, in slot order: hp, stamina, phase,
    /// position and facing.
    #[serde(default)]
    pub fighters: Vec<ActorNumbers>,
    /// The last [`EVENT_DEFAULT`] combat events, oldest first, each with the
    /// tick it resolved on (always below `tick`).
    #[serde(default)]
    pub events: Vec<BoutEvent>,
    /// Opt-in wall-time receipt: present only when the caller asked, so a
    /// step without the flag stays byte-identical. Wall time never enters a
    /// digest or a hash; a zero-sample ring answers `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<PlayTimingDto>,
}

/// One fighter's numbers after the last simulated tick: what a model reads
/// to play or debug. Units are permille for `hp`, stamina points, meters,
/// radians, reward `0..=1` and meters per second for velocity. Mirrors the
/// arena snapshot one to one; read-only, never hashed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ActorNumbers {
    /// Fighter slot, `0..participants`.
    pub id: u8,
    /// Health in permille, `0..=1000`; `0` means out of the fight.
    pub hp: u16,
    /// Stamina points, `0..=100`.
    pub stamina: u16,
    /// What the fighter is doing: `idle`, `attacking`, `blocking`,
    /// `dodging`, `staggered`, `knocked_down` or `out`.
    pub phase: String,
    /// Feet position `[x, y, z]` in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
    /// Immediate survival reward: remaining health fraction `0..=1`.
    #[serde(default)]
    pub reward: f32,
    /// Legal discrete actions `[attack, block, dodge]`; `false` while
    /// staggered, down, out, or without the stamina to start the move.
    #[serde(default)]
    pub legal_mask: [bool; engine_arena::gym::GYM_NUM_ACTIONS],
    /// Planar velocity `[vx, vy, vz]` in meters per second (`vy` is `0`).
    #[serde(default)]
    pub velocity: [f32; 3],
}

/// One combat event with the tick it resolved on. Units are fixed ticks and
/// permille.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BoutEvent {
    /// 0-indexed tick the event resolved on; the hp it changed shows in any
    /// observation whose `tick` is above this value.
    pub tick: u32,
    /// `hit`, `block`, `parry`, `dodge`, `stamina_out` or `kill`.
    pub kind: String,
    /// Attacker slot; absent for `stamina_out`.
    pub attacker: Option<u8>,
    /// Victim slot (the fighter itself for `stamina_out`).
    pub victim: u8,
    /// Damage permille for a `hit`, chip damage for a `block`, else `0`.
    pub amount: u16,
}

/// Combat events a bout remembers (the ring behind `sim.observe`).
pub(crate) const EVENT_LOG_CAP: usize = 64;
/// Events a step or an observation returns when the caller names no count.
pub(crate) const EVENT_DEFAULT: usize = 16;

/// Bounded ring of step wall times for the opt-in `timing` receipt. Samples
/// are milliseconds from the CPU path; they never enter a digest, a hash or
/// the fixed tick, and a missing sample answers `unknown`, never zero. One
/// ring per [`PlaySlot`] (play.step) and one per `PlayJobs` (play.run starts),
/// so a session's samples never mix with another's.
#[derive(Debug, Default)]
pub(crate) struct TimingRing {
    samples_ms: VecDeque<f64>,
}

impl TimingRing {
    /// Record one step's wall time in milliseconds, dropping the oldest
    /// sample once the ring is full.
    pub(crate) fn record_ms(&mut self, ms: f64) {
        if self.samples_ms.len() == TIMING_RING_CAP {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(ms);
    }

    /// The receipt over a sorted copy of the ring: nearest-rank p50 and p99,
    /// labelled `software`. An empty ring reports every figure `unknown`.
    #[must_use]
    pub(crate) fn report(&self) -> PlayTimingDto {
        PlayTimingDto {
            label: TIMING_LABEL_SOFTWARE.to_owned(),
            p50_ms: self.percentile(50),
            p99_ms: self.percentile(99),
            samples: self.samples_ms.len(),
        }
    }

    /// Nearest-rank percentile: rank `ceil(n * percent / 100)`, 1-based, on
    /// the ascending sorted copy. Integer rank math only, so the same ring
    /// always answers the same figure.
    fn percentile(&self, percent: usize) -> StepMillis {
        let mut sorted: Vec<f64> = self.samples_ms.iter().copied().collect();
        sorted.sort_unstable_by(f64::total_cmp);
        let Some(last) = sorted.len().checked_sub(1) else {
            return StepMillis::Missing(NoSample::Unknown);
        };
        let rank = sorted.len().saturating_mul(percent).div_ceil(100).max(1);
        StepMillis::Measured(sorted[rank.saturating_sub(1).min(last)])
    }
}

/// What identifies one bout besides its tape: the match seed, the headcount
/// and the arena half-extent bits. Two requests with equal specs and a
/// shared tape prefix are the same match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoutSpec {
    pub(crate) seed: u64,
    pub(crate) participants: u8,
    pub(crate) half_bits: u32,
}

impl BoutSpec {
    /// Spec for a validated request.
    pub(crate) fn new(seed: u64, participants: u8, half_extent_m: f32) -> Self {
        Self {
            seed,
            participants,
            half_bits: half_extent_m.to_bits(),
        }
    }

    /// Arena half-extent in meters.
    pub(crate) fn half_extent_m(self) -> f32 {
        f32::from_bits(self.half_bits)
    }
}

/// One observation of a bout: the numbers a step or `sim.observe` returns.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoutView {
    /// Ticks simulated (1-indexed count).
    pub(crate) tick: u32,
    /// Checkpoint `tick - 1` hash, `0` before the first tick.
    pub(crate) digest: u64,
    pub(crate) fighters: Vec<ActorNumbers>,
    pub(crate) events: Vec<BoutEvent>,
}

fn event_of(event: &GymEvent) -> BoutEvent {
    BoutEvent {
        tick: event.tick,
        kind: event.kind.name().to_owned(),
        attacker: event.attacker,
        victim: event.victim,
        amount: event.amount,
    }
}

/// One live match: the arena env, every input it consumed and the bounded
/// event ring. Opened at tick 0 and only ever stepped forward.
pub(crate) struct LiveBout {
    spec: BoutSpec,
    env: GymLive,
    /// Every input the env consumed, in tape order.
    consumed: BoutTape,
    log: VecDeque<GymEvent>,
    /// State digest of the reached tick: the last spine checkpoint hash
    /// (see [`spine_digest`]), `0` before the first tick.
    digest: u64,
}

impl LiveBout {
    /// Fresh bout at tick 0; `None` when the arena refuses the config.
    fn open(spec: BoutSpec) -> Option<Self> {
        // The cap only bounds how far the env may step; it never enters the
        // hash, so one long cap serves every prefix a step can ask for.
        let config = ArenaConfig {
            seed: spec.seed,
            ticks: MAX_PLAY_TICKS,
            fighters: spec.participants,
            half_extent_m: spec.half_extent_m(),
            table_version: 1,
        };
        Some(Self {
            spec,
            env: GymLive::new(&config)?,
            consumed: BoutTape::default(),
            log: VecDeque::with_capacity(EVENT_LOG_CAP),
            digest: 0,
        })
    }

    /// True when a request for `ticks` over `tape` extends this bout: same
    /// spec, nothing to rewind, and the tape agrees with every input already
    /// consumed.
    fn continues(&self, spec: BoutSpec, tape: &BoutTape, ticks: u32) -> bool {
        let held = self.env.tick();
        let split = tape.0.partition_point(|input| input.tick < held);
        self.spec == spec && held <= ticks && self.consumed.0[..] == tape.0[..split]
    }

    /// Step forward to `ticks`, one env step per tick, feeding the event
    /// ring. `tape` is the full validated request tape.
    fn run_to(&mut self, tape: &BoutTape, ticks: u32) -> Result<(), McpToolError> {
        let mut at = tape.0.partition_point(|input| input.tick < self.env.tick());
        while self.env.tick() < ticks {
            let tick = self.env.tick();
            let end = at + tape.0[at..].partition_point(|input| input.tick <= tick);
            let events = self.env.step(&tape.0[at..end]).ok_or_else(|| {
                McpToolError::new(
                    "invalid_tape",
                    format!(
                        "arena refused the inputs of tick {tick}; they passed MCP range checks, \
                         so report the tape verbatim"
                    ),
                )
            })?;
            for event in events {
                if self.log.len() == EVENT_LOG_CAP {
                    self.log.pop_front();
                }
                self.log.push_back(*event);
            }
            at = end;
        }
        self.consumed.0.clear();
        self.consumed.0.extend_from_slice(&tape.0);
        Ok(())
    }

    /// The bout's identity: seed, headcount and extent.
    pub(crate) fn spec(&self) -> BoutSpec {
        self.spec
    }

    /// The numbers now: every fighter plus the last `events` events, oldest
    /// first.
    pub(crate) fn observe(&self, events: usize) -> BoutView {
        let snapshot = self.env.snapshot();
        let skip = self.log.len().saturating_sub(events);
        BoutView {
            tick: snapshot.tick,
            digest: self.digest,
            fighters: snapshot
                .fighters
                .iter()
                .map(|state| ActorNumbers {
                    id: state.id.0,
                    hp: state.hp,
                    stamina: state.stamina,
                    phase: state.phase.name().to_owned(),
                    pos: state.pos,
                    yaw: state.yaw,
                    reward: state.reward,
                    legal_mask: state.legal_mask,
                    velocity: state.velocity,
                })
                .collect(),
            events: self.log.iter().skip(skip).map(event_of).collect(),
        }
    }
}

/// Read the spine checkpoint `ticks - 1` over `tape`: the state digest the
/// door reports. The spine step is the authority `play.step` reads; the
/// legacy bout beside it supplies the numbers and the event ring, and its
/// checkpoints equal these bytes (ARC4 parity). `None` when the spine refuses
/// a tape the legacy step accepted; `0` before the first tick.
fn spine_digest(spec: BoutSpec, tape: &BoutTape, ticks: u32) -> Option<u64> {
    if ticks == 0 {
        return Some(0);
    }
    let config = ArenaConfig {
        seed: spec.seed,
        // The cap only bounds the walk; it never enters the hash, and the
        // request tape stops below `ticks` (the `build_tape` contract).
        ticks,
        fighters: spec.participants,
        half_extent_m: spec.half_extent_m(),
        table_version: 1,
    };
    let mut sim = SpineMatch::build(&config, tape, MatchWorld::empty())?;
    let mut digest = 0;
    while sim.tick() < ticks {
        digest = sim.step()?.hash;
    }
    Some(digest)
}

/// Everything `play.step`, `sim.step` and `sim.observe` hold for one
/// session: the live bout (at most one, replaced when a request cannot extend
/// it), the GPU capture hook fixed at construction and the re-query cache.
/// Lives in `ServerState` (or a `PlaySession`), so every part is per session,
/// never shared across sessions or processes.
#[derive(Default)]
pub(crate) struct PlaySlot {
    bout: Option<LiveBout>,
    /// GPU capture callback; `None` means the software rasterizer.
    hook: Option<StepCaptureHook>,
    /// This session's bounded receipt cache.
    requery: RequeryCache,
    /// Bounded ring of step wall times (read only by the `timing` receipt).
    timing: TimingRing,
    /// Sim steps performed through this slot's live bout: the cost proof
    /// tests read.
    #[cfg(test)]
    pub(crate) steps: u64,
    /// Spine steps spent reading the digest (always the full prefix, from
    /// tick 0): the ARC16 receipt reads it.
    #[cfg(test)]
    pub(crate) spine_steps: u64,
}

impl fmt::Debug for PlaySlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tick = self.bout.as_ref().map(|bout| bout.env.tick());
        write!(f, "PlaySlot(tick {tick:?}, hook {})", self.hook.is_some())
    }
}

impl PlaySlot {
    /// Empty slot with its capture hook fixed for the slot's whole life.
    fn with_hook(hook: Option<StepCaptureHook>) -> Self {
        Self {
            hook,
            ..Self::default()
        }
    }

    /// Bring the bout to `ticks` over `tape` and hand it back. Extends the
    /// held bout when it can, else opens a fresh one and steps from tick 0;
    /// both give the bytes a from-zero run would. The reported digest is the
    /// spine checkpoint `ticks - 1` read from `tape` (see [`spine_digest`]).
    /// A failure leaves the slot empty, so the next call rebuilds.
    pub(crate) fn advance_to(
        &mut self,
        spec: BoutSpec,
        tape: &BoutTape,
        ticks: u32,
    ) -> Result<&LiveBout, McpToolError> {
        let mut bout = match self.bout.take() {
            Some(held) if held.continues(spec, tape, ticks) => held,
            _ => Self::open(spec)?,
        };
        #[cfg(test)]
        let from = bout.env.tick();
        bout.run_to(tape, ticks)?;
        bout.digest = spine_digest(spec, tape, ticks).ok_or_else(|| {
            McpToolError::new(
                "invalid_tape",
                format!(
                    "the spine step refused the tape through tick {ticks}; the legacy step \
                     accepted it, so report the tape verbatim"
                ),
            )
        })?;
        #[cfg(test)]
        {
            self.steps += u64::from(ticks - from);
            self.spine_steps += u64::from(ticks);
        }
        Ok(self.bout.insert(bout))
    }

    /// Replace the bout with a fresh one at tick 0 (`sim.reset`).
    pub(crate) fn reset(&mut self, spec: BoutSpec) -> Result<&LiveBout, McpToolError> {
        let bout = Self::open(spec)?;
        Ok(self.bout.insert(bout))
    }

    /// The held bout, if any step or reset opened one.
    pub(crate) fn current(&self) -> Option<&LiveBout> {
        self.bout.as_ref()
    }

    fn open(spec: BoutSpec) -> Result<LiveBout, McpToolError> {
        LiveBout::open(spec).ok_or_else(|| {
            McpToolError::new(
                "invalid_config",
                format!(
                    "arena refused seed={} participants={} half={}; checked ranges already \
                     passed, so report this request verbatim",
                    spec.seed,
                    spec.participants,
                    spec.half_extent_m()
                ),
            )
        })
    }
}

/// One-shot GPU capture for a step: asked size, state digest and tick in,
/// validated receipt out. The window/GPU host hands this to a session when it
/// builds it; the endpoint never imports render, the callback owns the pixels.
pub type StepCaptureHook =
    fn(&CaptureArgs, digest: u64, tick: u32) -> Result<CaptureOutput, McpToolError>;

/// Stateless step over a full tape prefix: validate, simulate the
/// `0..upto_tick` prefix, read its spine checkpoint, capture. Same seed
/// plus same tape twice gives the same digest, numbers and PNG bytes
/// (software re-queries replay the call's own re-query cache). Holds nothing
/// between calls: the MCP tool path keeps a bout, a hook and a cache per
/// session through [`PlaySlot`].
pub fn play_step_with(
    args: &PlayStepArgs,
    hook: Option<StepCaptureHook>,
) -> Result<PlayStepOutput, McpToolError> {
    play_step_in(&mut PlaySlot::with_hook(hook), args)
}

/// Step over a full tape prefix inside `slot`: extends the slot's live bout
/// when the request continues it, else rebuilds from tick 0; the slot's hook
/// and cache decide the receipt. With `timing` on, the call's own wall time is
/// recorded in the slot's ring and returned; nothing else changes.
fn play_step_in(slot: &mut PlaySlot, args: &PlayStepArgs) -> Result<PlayStepOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    let half = args.half_extent_m.unwrap_or(20.0);
    let inputs = args.inputs.clone().unwrap_or_default();
    let upto = args.upto_tick.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("play.step needs upto_tick; submit 1..={MAX_PLAY_TICKS}"),
        )
    })?;
    let wants_timing = args.timing.unwrap_or(false);
    // Receipt-only clock: read only when the caller opted in, and the value
    // never enters the tick, a digest or a hash.
    let clock = wants_timing.then(std::time::Instant::now);
    let mut out = step_core(
        slot,
        &StepRequest {
            seed,
            participants,
            half,
            upto,
            width: args.width,
            height: args.height,
        },
        &inputs,
    )?;
    if let Some(clock) = clock {
        slot.timing
            .record_ms(clock.elapsed().as_secs_f64() * 1000.0);
        out.timing = Some(slot.timing.report());
    }
    Ok(out)
}

/// One validated step: match config, tape prefix, digest and receipt.
struct StepRequest {
    seed: u64,
    participants: u8,
    half: f32,
    upto: u32,
    width: Option<u32>,
    height: Option<u32>,
}

fn step_core(
    slot: &mut PlaySlot,
    request: &StepRequest,
    inputs: &[PlayInputDto],
) -> Result<PlayStepOutput, McpToolError> {
    let participants = request.participants;
    let half = request.half;
    let upto = request.upto;
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_participants",
            format!("participants {participants} is outside 2..={MAX_PARTICIPANTS}"),
        ));
    }
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0 meters"),
        ));
    }
    if upto == 0 || upto > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("upto_tick {upto} is outside 1..={MAX_PLAY_TICKS} (MCP play.step bound)"),
        ));
    }
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS} per match",
                inputs.len()
            ),
        ));
    }
    let tape = build_tape(inputs, upto, participants)?;
    let spec = BoutSpec::new(request.seed, participants, half);
    let view = slot.advance_to(spec, &tape, upto)?.observe(EVENT_DEFAULT);
    match slot.hook {
        None => step_cached(&slot.requery, request, inputs, view),
        Some(hook) => step_compute(Some(hook), request, inputs.len(), view),
    }
}

/// Cached software step: the same validated composition twice replays the
/// filed receipt (identical digest, numbers and PNG bytes) without
/// re-rendering. The hooked GPU path never enters here: the device owns those
/// pixels.
fn step_cached(
    cache: &RequeryCache,
    request: &StepRequest,
    inputs: &[PlayInputDto],
    view: BoutView,
) -> Result<PlayStepOutput, McpToolError> {
    let key = requery_key(&RequeryParams {
        seed: request.seed,
        participants: request.participants,
        half_bits: request.half.to_bits(),
        upto: request.upto,
        width: request.width.unwrap_or(CAPTURE_DEFAULT_WIDTH),
        height: request.height.unwrap_or(CAPTURE_DEFAULT_HEIGHT),
        inputs,
    });
    if let Some(hit) = cache.requery_lookup(&key, inputs)? {
        return Ok(hit);
    }
    let out = step_compute(None, request, inputs.len(), view)?;
    cache.requery_store(&key, inputs, &out)?;
    Ok(out)
}

/// Capture the receipt of one simulated prefix: the hooked GPU receipt or the
/// software rasterizer, around the digest and numbers the live bout read.
fn step_compute(
    hook: Option<StepCaptureHook>,
    request: &StepRequest,
    inputs_len: usize,
    view: BoutView,
) -> Result<PlayStepOutput, McpToolError> {
    let seed = request.seed;
    let participants = request.participants;
    let upto = request.upto;
    let digest = view.digest;
    let asked = CaptureArgs {
        width: request.width,
        height: request.height,
        max_size: None,
    };
    let capture = match hook {
        Some(render) => capture_with(&asked, Box::new(|asked| render(asked, digest, upto)))?,
        None => software_capture(&asked, digest, upto, seed, &view)?,
    };
    Ok(PlayStepOutput {
        tick: upto,
        digest,
        participants,
        inputs_applied: inputs_len,
        summary: format!(
            "tick {upto} duel {participants} participants digest {digest:016x} inputs {inputs_len}",
        ),
        manifest: capture.manifest,
        png_bytes: capture.png_bytes,
        fighters: view.fighters,
        events: view.events,
        timing: None,
    })
}

/// Live duel held across steps: the accumulated tape plus the cursor.
/// `advance` appends one step's inputs, simulates the new prefix and returns
/// its digest plus PNG; the cursor only moves on success.
#[derive(Debug)]
pub struct PlaySession {
    seed: u64,
    participants: u8,
    half: f32,
    cursor: u32,
    tape: Vec<PlayInputDto>,
    slot: PlaySlot,
}

impl PlaySession {
    /// Open a duel; invalid config files nothing and names the legal values.
    pub fn new(seed: u64, participants: u8, half_extent_m: f32) -> Result<Self, McpToolError> {
        Self::new_with_hook(seed, participants, half_extent_m, None)
    }

    /// Open a duel with a wired GPU capture callback (the device path).
    pub fn new_with_hook(
        seed: u64,
        participants: u8,
        half_extent_m: f32,
        hook: Option<StepCaptureHook>,
    ) -> Result<Self, McpToolError> {
        if !(2..=MAX_PARTICIPANTS).contains(&participants) {
            return Err(McpToolError::new(
                "bad_participants",
                format!("participants {participants} is outside 2..={MAX_PARTICIPANTS}"),
            ));
        }
        if !half_extent_m.is_finite() || !(5.0..=100.0).contains(&half_extent_m) {
            return Err(McpToolError::new(
                "bad_extent",
                format!("half_extent_m {half_extent_m} is outside 5.0..=100.0 meters"),
            ));
        }
        Ok(Self {
            seed,
            participants,
            half: half_extent_m,
            cursor: 0,
            tape: Vec::new(),
            slot: PlaySlot::with_hook(hook),
        })
    }

    /// Ticks simulated so far as the next uncovered 0-indexed tick id: 0 on
    /// open, N after covering `0..N`. The next step covers
    /// `cursor..cursor + ticks`; see the module legend.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Take inputs for the next `ticks` ticks and return the step receipt.
    /// New inputs must land in `cursor..cursor + ticks`, sorted, with no
    /// duplicates; a refusal moves neither the cursor nor the tape. The
    /// receipt `tick` is the 1-indexed count `cursor + ticks` (one past the
    /// last checkpoint id); its digest equals the `play.run` `final_hash`
    /// for the same prefix.
    pub fn advance(
        &mut self,
        inputs: &[PlayInputDto],
        ticks: u32,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<PlayStepOutput, McpToolError> {
        if ticks == 0 || ticks > MAX_PLAY_TICKS {
            return Err(McpToolError::new(
                "bad_ticks",
                format!("ticks {ticks} is outside 1..={MAX_PLAY_TICKS} (MCP play.step bound)"),
            ));
        }
        let upto = self.cursor.checked_add(ticks).ok_or_else(|| {
            McpToolError::new(
                "bad_ticks",
                format!(
                    "cursor {} plus {ticks} ticks overflows; at most {MAX_PLAY_TICKS}",
                    self.cursor
                ),
            )
        })?;
        if upto > MAX_PLAY_TICKS {
            return Err(McpToolError::new(
                "bad_ticks",
                format!("upto_tick {upto} exceeds {MAX_PLAY_TICKS} (MCP play.step bound)"),
            ));
        }
        for (index, dto) in inputs.iter().enumerate() {
            if dto.tick < self.cursor {
                return Err(McpToolError::new(
                    "stale_tick",
                    format!(
                        "inputs[{index}].tick {} is behind the live cursor {}; \
                         submit ticks in {}..{upto}",
                        dto.tick, self.cursor, self.cursor
                    ),
                ));
            }
            if dto.tick >= upto {
                return Err(McpToolError::new(
                    "invalid_tape",
                    format!(
                        "inputs[{index}].tick {} is past this step end {upto}; \
                         this step covers {}..{upto}",
                        dto.tick, self.cursor
                    ),
                ));
            }
        }
        let mut combined = self.tape.clone();
        combined.extend_from_slice(inputs);
        // Global order/duplicate/range gate over the whole live tape.
        let _ = build_tape(&combined, upto, self.participants)?;
        let out = step_core(
            &mut self.slot,
            &StepRequest {
                seed: self.seed,
                participants: self.participants,
                half: self.half,
                upto,
                width,
                height,
            },
            &combined,
        )?;
        self.tape = combined;
        self.cursor = upto;
        Ok(out)
    }
}

impl crate::mcp::ServerState {
    /// Live duel step through the MCP session: the answer is a function of
    /// the caller-held tape prefix, and the session's live bout only makes a
    /// continuing prefix cheap. Uses this session's GPU hook when one was
    /// wired at construction, else the software rasterizer.
    pub fn play_step(&mut self, args: &PlayStepArgs) -> Result<PlayStepOutput, McpToolError> {
        play_step_in(&mut self.play, args)
    }

    /// Wire the GPU capture callback for this session's `play.step` (the
    /// device exists, so captures succeed through it, never
    /// `unsupported_capture`). Set once while the state is built, before the
    /// server wraps it; without a hook the software rasterizer answers.
    #[must_use]
    pub fn with_step_capture_hook(mut self, hook: StepCaptureHook) -> Self {
        self.play.hook = Some(hook);
        self
    }
}

/// `play.step` tool: state summary plus a real capture PNG per step.
pub struct PlayStepTool;

impl ToolBase for PlayStepTool {
    type Parameter = PlayStepArgs;
    type Output = PlayStepOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_PLAY_STEP.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Live duel step: take inputs for N ticks and return the state digest, \
             per-fighter numbers (hp, stamina, phase, pos, yaw), the last combat \
             events and a real capture PNG. Holds the match across calls (a \
             longer prefix of the same tape only simulates the new ticks); same \
             inputs twice give the same digests. upto_tick N simulates ticks 0..N \
             and reports tick N with the checkpoint N-1 digest (the play.run \
             tick_count/final_hash for the same prefix)."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::write_annotations(false)
    }
}

impl SyncTool<McpServer> for PlayStepTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PLAY_STEP, |state| state.play_step(&param))
    }
}

// ---------------------------------------------------------------------------
// Software receipt: deterministic raster of the live duel state plus a
// std-only PNG codec (stored deflate, filter 0). Same inputs give the same
// bytes; the GPU hook replaces this when a device exists.
// ---------------------------------------------------------------------------

/// Render one receipt through the software rasterizer and validate it like a
/// wired callback (dimensions, PNG magic, byte bound, non-black variance).
fn software_capture(
    args: &CaptureArgs,
    digest: u64,
    tick: u32,
    seed: u64,
    view: &BoutView,
) -> Result<CaptureOutput, McpToolError> {
    let width = args.width.unwrap_or(CAPTURE_DEFAULT_WIDTH);
    let height = args.height.unwrap_or(CAPTURE_DEFAULT_HEIGHT);
    if width == 0 || height == 0 {
        return Err(McpToolError::new(
            "bad_size",
            format!("capture size {width}x{height} is zero; submit 1..={CAPTURE_MAX_DIM} each"),
        ));
    }
    if width > CAPTURE_MAX_DIM || height > CAPTURE_MAX_DIM {
        return Err(McpToolError::new(
            "bad_size",
            format!("capture size {width}x{height} exceeds {CAPTURE_MAX_DIM}"),
        ));
    }
    if u64::from(width) * u64::from(height) > MAX_SOFTWARE_PIXELS {
        return Err(McpToolError::new(
            "bad_size",
            format!(
                "capture size {width}x{height} exceeds the software {MAX_SOFTWARE_PIXELS} pixels \
                 (512x512); a wired GPU callback covers up to {CAPTURE_MAX_DIM}"
            ),
        ));
    }
    let rgb = render_step_pixels(
        width,
        height,
        tick,
        digest,
        seed,
        &view.fighters,
        &view.events,
    );
    require_non_black(&rgb)?;
    let png_bytes = encode_png_rgb(width, height, &rgb);
    if png_bytes.len() > MAX_CAPTURE_BYTES {
        return Err(McpToolError::new(
            "bad_receipt",
            format!(
                "receipt holds {} png bytes; at most {MAX_CAPTURE_BYTES}",
                png_bytes.len()
            ),
        ));
    }
    Ok(CaptureOutput {
        manifest: CaptureManifestDto {
            adapter: "endpoint-software".to_owned(),
            driver: "play.step software rasterizer (no device)".to_owned(),
            width,
            height,
            validation_errors: 0,
        },
        png_bytes,
    })
}

/// Deterministic duel raster: slate gradient plus checker, one bright disc
/// per participant placed by the state digest, and the tick/digest bits along the
/// top row so consecutive steps never render identical. Integer math only.
fn render_step_pixels(
    width: u32,
    height: u32,
    tick: u32,
    digest: u64,
    seed: u64,
    fighters: &[ActorNumbers],
    events: &[BoutEvent],
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut rgb = vec![0u8; w * h * 3];
    paint_backdrop(&mut rgb, w, h);
    if w >= 16 && h >= 16 {
        paint_fighter_discs(&mut rgb, (w, h), tick, digest, seed, fighters, events);
    }
    paint_hud(&mut rgb, w, h, tick, digest, seed, fighters);
    rgb
}

/// Slate gradient with a checker and a vignette darkening toward the corners.
fn paint_backdrop(rgb: &mut [u8], w: usize, h: usize) {
    let cx = w as i32 / 2;
    let cy = h as i32 / 2;
    let denom = (cx * cx + cy * cy + 1) as u32;
    for y in 0..h {
        for x in 0..w {
            let base = 16 + (y as u32 * 36 / h.max(1) as u32);
            let checker = (((x / 8) + (y / 8)) % 2) as u32 * 8;
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            let vig = ((dx * dx + dy * dy) as u32 * 24) / denom;
            let luma = (base + checker).saturating_sub(vig);
            let at = (y * w + x) * 3;
            rgb[at] = luma.min(255) as u8;
            rgb[at + 1] = luma.saturating_add(4).min(255) as u8;
            rgb[at + 2] = luma.saturating_add(16).min(255) as u8;
        }
    }
}

/// One disc per fighter, placed by the seeded digest stream. A recent hit lifts
/// the disc and a fallen fighter dims it.
fn paint_fighter_discs(
    rgb: &mut [u8],
    (w, h): (usize, usize),
    tick: u32,
    digest: u64,
    seed: u64,
    fighters: &[ActorNumbers],
    events: &[BoutEvent],
) {
    const HIT_FLASH_TICKS: u32 = 8;
    const PALETTE: [[u32; 3]; 4] = [
        [206, 74, 62],
        [74, 144, 226],
        [96, 200, 120],
        [220, 180, 70],
    ];
    let mut rng = seed ^ digest ^ (u64::from(tick).wrapping_mul(GOLDEN_GAMMA));
    let rad = (w.min(h) / 7).max(3) as i32;
    for (index, fighter) in fighters.iter().enumerate() {
        let draw_px = splitmix64(rng);
        rng = rng.wrapping_add(GOLDEN_GAMMA);
        let draw_py = splitmix64(rng);
        rng = rng.wrapping_add(GOLDEN_GAMMA);
        let px = 6 + (draw_px % (w as u64 - 12)) as i32;
        let py = 6 + (draw_py % (h as u64 - 12)) as i32;
        let color = PALETTE[index % PALETTE.len()];
        // A recent hit lifts the victim; a fallen fighter (`out`) dims.
        let hit = events.iter().any(|event| {
            event.kind == "hit"
                && usize::from(event.victim) == index
                && tick.saturating_sub(event.tick) <= HIT_FLASH_TICKS
        });
        let lift = if hit { 90 } else { 40 };
        let fallen = fighter.phase == "out";
        for dy in -rad..=rad {
            for dx in -rad..=rad {
                let d2 = dx * dx + dy * dy;
                if d2 > rad * rad {
                    continue;
                }
                let (x, y) = (px + dx, py + dy);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let core = d2 * 4 <= rad * rad;
                let at = ((y as usize) * w + x as usize) * 3;
                for lane in 0..3 {
                    let base = if fallen { color[lane] / 3 } else { color[lane] };
                    let value = if core {
                        base.saturating_add(lift).min(255)
                    } else {
                        base
                    };
                    rgb[at + lane] = value as u8;
                }
            }
        }
    }
}

/// Fighter HP bars from the top-left, then the tick and digest bits along the
/// top row. The bars need a frame of at least 16 by 16.
fn paint_hud(
    rgb: &mut [u8],
    w: usize,
    h: usize,
    tick: u32,
    digest: u64,
    seed: u64,
    fighters: &[ActorNumbers],
) {
    if w >= 16 && h >= 16 {
        // One bar per fighter from the top-left: lit width follows permille hp.
        let bar_h = (h / 8).clamp(1, 3);
        let span = w - 12;
        for (index, fighter) in fighters.iter().enumerate() {
            let top = 6 + index * (bar_h + 2);
            let lit = span * usize::from(fighter.hp.min(1000)) / 1000;
            for y in top..(top + bar_h).min(h) {
                for x in 6..6 + span {
                    let at = (y * w + x) * 3;
                    let shade = if x < 6 + lit { 235 } else { 40 };
                    rgb[at] = shade;
                    rgb[at + 1] = shade;
                    rgb[at + 2] = shade;
                }
            }
        }
    }
    let bits = u64::from(tick) ^ digest ^ seed;
    for b in 0..(32.min(w)) {
        let at = b * 3;
        if (bits >> b) & 1 == 1 {
            rgb[at] = 255;
            rgb[at + 1] = 255;
            rgb[at + 2] = 255;
        } else {
            rgb[at] = 8;
            rgb[at + 1] = 8;
            rgb[at + 2] = 10;
        }
    }
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut key = Vec::with_capacity(4 + body.len());
    key.extend_from_slice(tag);
    key.extend_from_slice(body);
    out.extend_from_slice(&crc32(&key).to_be_bytes());
}

/// Encode RGB8 rows as a PNG (filter 0, stored deflate, one IDAT).
fn encode_png_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut raw = Vec::with_capacity(h * (1 + w * 3));
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * w * 3..(y + 1) * w * 3]);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).len().max(1);
    for (index, piece) in raw.chunks(65_535).enumerate() {
        zlib.push(u8::from(index + 1 == blocks));
        let len = piece.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(piece);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = Vec::with_capacity(PNG_MAGIC.len() + 64 + zlib.len());
    out.extend_from_slice(&PNG_MAGIC);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &zlib);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

/// Decode our own receipts (RGB8, filter 0, stored deflate) for the
/// pixel-variance gate. Anything else fails as `bad_receipt`, never success.
/// Test-only: production validates receipts through `require_non_black` on
/// the bytes it just rendered, plus `capture_with` for hooked receipts.
#[cfg(test)]
fn decode_png_rgb(png: &[u8]) -> Result<(u32, u32, Vec<u8>), McpToolError> {
    let bad = |detail: &str| McpToolError::new("bad_receipt", detail.to_owned());
    if png.len() < PNG_MAGIC.len() || png[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err(bad(
            "receipt bytes are not a PNG (missing the 8-byte magic)",
        ));
    }
    let mut pos = PNG_MAGIC.len();
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut idat = Vec::new();
    let mut ended = false;
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let tag = [png[pos + 4], png[pos + 5], png[pos + 6], png[pos + 7]];
        pos += 8;
        if pos + len + 4 > png.len() {
            return Err(bad("receipt PNG chunk runs past the buffer end"));
        }
        let body = &png[pos..pos + len];
        let want = crc32(&png[pos - 4..pos + len]);
        let got = u32::from_be_bytes([
            png[pos + len],
            png[pos + len + 1],
            png[pos + len + 2],
            png[pos + len + 3],
        ]);
        if want != got {
            return Err(bad("receipt PNG chunk fails its CRC"));
        }
        match &tag {
            b"IHDR" => {
                if len != 13 {
                    return Err(bad("receipt IHDR has the wrong length"));
                }
                width = Some(u32::from_be_bytes([body[0], body[1], body[2], body[3]]));
                height = Some(u32::from_be_bytes([body[4], body[5], body[6], body[7]]));
                if body[8] != 8 || body[9] != 2 {
                    return Err(bad("receipt PNG is not RGB8"));
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => {
                ended = true;
                break;
            }
            _ => {}
        }
        pos += len + 4;
    }
    if !ended {
        return Err(bad("receipt PNG never ends (missing IEND)"));
    }
    let (width, height) = (
        width.ok_or_else(|| bad("receipt PNG has no IHDR"))?,
        height.ok_or_else(|| bad("receipt PNG has no IHDR"))?,
    );
    if width == 0 || height == 0 || width > CAPTURE_MAX_DIM || height > CAPTURE_MAX_DIM {
        return Err(bad("receipt PNG dimensions are outside 1..=2048"));
    }
    if u64::from(width) * u64::from(height) > MAX_SOFTWARE_PIXELS * 16 {
        return Err(bad("receipt PNG dimensions overflow the decode bound"));
    }
    if idat.len() < 6 {
        return Err(bad("receipt PNG IDAT is too short for zlib"));
    }
    if idat[0] != 0x78 || (u16::from(idat[0]) * 256 + u16::from(idat[1])) % 31 != 0 {
        return Err(bad("receipt PNG IDAT is not a zlib stream"));
    }
    let mut raw = Vec::new();
    let mut cursor = 2;
    loop {
        if cursor + 5 > idat.len() {
            return Err(bad("receipt PNG deflate block is truncated"));
        }
        let header = idat[cursor];
        let last = header & 1 == 1;
        if header & 0x06 != 0 {
            return Err(bad("receipt PNG uses a compressed deflate block"));
        }
        cursor += 1;
        let len = u16::from_le_bytes([idat[cursor], idat[cursor + 1]]);
        let nlen = u16::from_le_bytes([idat[cursor + 2], idat[cursor + 3]]);
        if nlen != !len {
            return Err(bad("receipt PNG stored block fails its length check"));
        }
        cursor += 4;
        let len = len as usize;
        if cursor + len + 4 > idat.len() {
            return Err(bad("receipt PNG stored block runs past the stream"));
        }
        raw.extend_from_slice(&idat[cursor..cursor + len]);
        cursor += len;
        if last {
            let want = adler32(&raw);
            let got = u32::from_be_bytes([
                idat[cursor],
                idat[cursor + 1],
                idat[cursor + 2],
                idat[cursor + 3],
            ]);
            if want != got {
                return Err(bad("receipt PNG fails its Adler check"));
            }
            break;
        }
    }
    let w = width as usize;
    let h = height as usize;
    if raw.len() != h * (1 + w * 3) {
        return Err(bad("receipt PNG scanlines do not match IHDR"));
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &raw[y * (1 + w * 3)..(y + 1) * (1 + w * 3)];
        if row[0] != 0 {
            return Err(bad("receipt PNG uses a non-zero filter"));
        }
        rgb.extend_from_slice(&row[1..]);
    }
    Ok((width, height, rgb))
}

/// Pixel-variance gate: mean luma plus variance over per-pixel luma. Black
/// frames and flat fills fail as `black_capture`, never as success.
fn require_non_black(rgb: &[u8]) -> Result<(), McpToolError> {
    if !rgb.len().is_multiple_of(3) || rgb.is_empty() {
        return Err(McpToolError::new(
            "black_capture",
            "capture holds no pixels; a step receipt must be non-black",
        ));
    }
    let n = (rgb.len() / 3) as u64;
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    for pixel in rgb.chunks_exact(3) {
        let luma = (u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2])) / 3;
        sum += luma;
        sum_sq += luma * luma;
    }
    let mean = sum / n;
    let variance = sum_sq / n - mean * mean;
    if mean < 8 || variance < 100 {
        return Err(McpToolError::new(
            "black_capture",
            format!("capture is black or flat (mean {mean}, variance {variance}); a step receipt must vary"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::frame_call_result;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use rmcp::model::ContentBlock;

    const SEED: u64 = 2040;
    const W: u32 = 96;
    const H: u32 = 64;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// A session at revision 1 whose scene plays as the empty arena (the one
    /// node hangs above head height), so `play.run` has a revision to pin.
    fn pinned_server() -> McpServer {
        use engine_editor::commands::{self, Batch, Document, Journal, Op};
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        let marker = Batch {
            ops: vec![Op::Insert {
                id: engine_scene::ids::ObjectId::new(),
                parent: engine_scene::ids::ObjectId::NIL,
                local: engine_scene::transform::Transform::new(
                    [0.0, 10.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                ),
            }],
            base_revision: 0,
            idempotency_key: 1,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &marker).expect("marker applies");
        McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
    }

    /// One duel step's inputs: fighter 0 pushes +x, fighter 1 pushes -x, and
    /// fighter 0 throws an attack every fourth step.
    fn step_inputs(base: u32, step: u32) -> Vec<PlayInputDto> {
        let mut out = Vec::new();
        for tick in base..base + 2 {
            out.push(PlayInputDto {
                tick,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: u8::from(step.is_multiple_of(4)),
            });
            out.push(PlayInputDto {
                tick,
                fighter: 1,
                mx: -60,
                mz: 0,
                facing: 0,
                buttons: 0,
            });
        }
        out
    }

    /// Play 20 steps on a session, checking the receipt gate per step.
    fn play_twenty(session: &mut PlaySession) -> (Vec<u64>, Vec<Vec<u8>>) {
        let mut digests = Vec::with_capacity(20);
        let mut frames = Vec::with_capacity(20);
        for step in 0..20 {
            let base = session.cursor();
            let out = session
                .advance(&step_inputs(base, step as u32), 2, Some(W), Some(H))
                .expect("duel step advances");
            assert_eq!(out.tick, base + 2);
            assert_eq!(out.participants, 2);
            assert_eq!((out.manifest.width, out.manifest.height), (W, H));
            assert_eq!(out.manifest.validation_errors, 0);
            assert_eq!(&out.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
            let (w, h, rgb) = decode_png_rgb(&out.png_bytes).expect("receipt decodes");
            assert_eq!((w, h), (W, H));
            require_non_black(&rgb).expect("every step is non-black with variance");
            assert!(out.summary.contains(&format!("tick {}", out.tick)));
            assert_eq!(out.fighters.len(), 2, "every step carries both fighters");
            assert!(out.fighters.iter().all(|fighter| fighter.hp <= 1000));
            digests.push(out.digest);
            frames.push(out.png_bytes);
        }
        (digests, frames)
    }

    #[test]
    fn scripted_duel_twenty_steps_repeat_identical() {
        let mut first = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
        let (digests, frames) = play_twenty(&mut first);
        assert_eq!(first.cursor(), 40);
        assert_ne!(digests[0], digests[19], "the duel moves, so digests move");

        // Same inputs twice give the same digests and the same PNG bytes.
        let mut second = PlaySession::new(SEED, 2, 20.0).expect("duel reopens");
        let (repeat_digests, repeat_frames) = play_twenty(&mut second);
        assert_eq!(digests, repeat_digests, "digests repeat across runs");
        assert_eq!(frames, repeat_frames, "receipt bytes repeat across runs");
    }

    /// Run one spine match over `tape` to `ticks`: its last checkpoint hash
    /// and its spine snapshot bytes.
    fn spine_run(tape: &BoutTape, ticks: u32) -> (u64, Vec<u8>) {
        let config = ArenaConfig {
            seed: SEED,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let mut sim = SpineMatch::build(&config, tape, MatchWorld::empty()).expect("spine builds");
        let mut hash = 0;
        while sim.tick() < ticks {
            hash = sim.step().expect("spine steps").hash;
        }
        (hash, sim.spine().snapshot().bytes)
    }

    /// ARC16: `play.step` reads its digest from the spine checkpoint, not the
    /// legacy fold. Twenty two-tick duel steps repeat equal digests and PNG
    /// bytes x2 (every PNG non-black, checked by [`play_twenty`]), every step
    /// digest equals the checkpoint a spine run over the same tape folds, and
    /// two spine runs seal equal snapshot bytes.
    #[test]
    fn arc16_twenty_step_duel_digests_read_spine_checkpoints_x2() {
        let mut first = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
        let (digests, frames) = play_twenty(&mut first);
        assert_eq!(digests.len(), 20);
        assert_eq!(first.slot.steps, 40, "the live bout stays incremental");
        assert_eq!(
            first.slot.spine_steps, 420,
            "the digest reads the spine prefix: 2+4+...+40 ticks"
        );

        let mut second = PlaySession::new(SEED, 2, 20.0).expect("duel reopens");
        let (repeat_digests, repeat_frames) = play_twenty(&mut second);
        assert_eq!(digests, repeat_digests, "digests repeat across runs x2");
        assert_eq!(frames, repeat_frames, "receipt bytes repeat across runs x2");

        // The same tape on the spine step, two ticks at a time: the step
        // digest is the checkpoint that run folded at that tick.
        let mut tape: Vec<PlayInputDto> = Vec::new();
        for step in 0..20u32 {
            tape.extend_from_slice(&step_inputs(step * 2, step));
            let ticks = (step + 1) * 2;
            let arena_tape = build_tape(&tape, ticks, 2).expect("tape validates");
            let (hash, _) = spine_run(&arena_tape, ticks);
            assert_eq!(
                digests[usize::try_from(step).expect("step fits")],
                hash,
                "step {step}: the digest is spine checkpoint {}",
                ticks - 1
            );
        }

        // Same spine bytes twice: the digest and the snapshot agree x2.
        let arena_tape = build_tape(&tape, 40, 2).expect("tape validates");
        let once = spine_run(&arena_tape, 40);
        let twice = spine_run(&arena_tape, 40);
        assert_eq!(once.0, digests[19], "the last digest is checkpoint 39");
        assert_eq!(once.0, twice.0, "spine digests repeat x2");
        assert_eq!(once.1, twice.1, "spine snapshot bytes repeat x2");
    }

    #[test]
    fn session_steps_cost_one_sim_step_per_tick() {
        let mut session = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
        play_twenty(&mut session);
        assert_eq!(session.cursor(), 40);
        assert_eq!(
            session.slot.steps, 40,
            "twenty two-tick steps cost forty sim steps, not the 420 a re-run per step costs"
        );
    }

    #[test]
    fn tool_path_numbers_match_sim_step_and_observe() {
        use crate::packs::gym::{SimObserveArgs, SimObserveTool, SimStepArgs, SimStepTool};

        let server = server();
        let inputs = step_inputs(0, 0);
        let stepped = PlayStepTool::invoke(
            &server,
            PlayStepArgs {
                seed: Some(SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
                upto_tick: Some(60),
                inputs: Some(inputs.clone()),
                width: Some(W),
                height: Some(H),
                timing: None,
            },
        )
        .expect("play.step serves");
        assert_eq!(stepped.tick, 60);
        assert_eq!(stepped.fighters.len(), 2);
        assert!(
            stepped
                .fighters
                .iter()
                .all(|f| f.stamina <= 100 && f.hp <= 1000),
            "numbers stay in their ranges"
        );
        // sim.step over the same composition reads the same bout bytes.
        let gym = SimStepTool::invoke(
            &server,
            SimStepArgs {
                seed: Some(SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
                ticks: Some(60),
                inputs: Some(inputs),
                events: None,
            },
        )
        .expect("sim.step serves");
        assert_eq!(gym.digest, stepped.digest);
        assert_eq!(gym.fighters, stepped.fighters);
        assert_eq!(gym.events, stepped.events);
        // sim.observe reads that bout without stepping it.
        let seen =
            SimObserveTool::invoke(&server, SimObserveArgs::default()).expect("sim.observe serves");
        assert_eq!(seen.tick, 60);
        assert_eq!(seen.digest, stepped.digest.to_string());
        assert_eq!(seen.fighters, stepped.fighters);
        assert_eq!(
            server.lock().expect("test lock holds").play.steps,
            60,
            "both verbs shared one bout: sixty ticks simulated once"
        );
    }

    #[test]
    fn tool_path_matches_session_prefix_digest() {
        let mut session = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
        let mut tape = Vec::new();
        for step in 0..2 {
            let base = session.cursor();
            let inputs = step_inputs(base, step);
            tape.extend_from_slice(&inputs);
            session
                .advance(&inputs, 2, Some(W), Some(H))
                .expect("session advances");
        }
        let args = PlayStepArgs {
            seed: Some(SEED),
            participants: Some(2),
            half_extent_m: Some(20.0),
            upto_tick: Some(4),
            inputs: Some(tape),
            width: Some(W),
            height: Some(H),
            timing: None,
        };
        let via_tool = PlayStepTool::invoke(&server(), args.clone()).expect("tool serves one step");
        let via_fn = play_step_with(&args, None).expect("stateless step serves");
        assert_eq!(via_tool.digest, via_fn.digest);
        assert_eq!(via_tool.png_bytes, via_fn.png_bytes);
        // The tool path is live on the server: same prefix, same digest.
        let direct = play_step_with(&args, None).expect("repeat serves");
        assert_eq!(via_tool.digest, direct.digest);
        assert_eq!(via_tool.tick, 4);
        let (_, _, rgb) = decode_png_rgb(&via_tool.png_bytes).expect("tool PNG decodes");
        require_non_black(&rgb).expect("tool PNG is non-black");
    }

    #[test]
    fn playtest57_requery_upto120_reproduces_digest_pixels_and_sim_bytes() {
        // Board repro shape: seed 95, 2 fighters, 20 m half-extent, inputs at
        // t0/t30/t60/t88/t100, re-queried at upto_tick 120 twice.
        let inputs = vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: 1,
            },
            PlayInputDto {
                tick: 30,
                fighter: 0,
                mx: 60,
                mz: 10,
                facing: 100,
                buttons: 0,
            },
            PlayInputDto {
                tick: 60,
                fighter: 1,
                mx: -60,
                mz: 0,
                facing: 0,
                buttons: 1,
            },
            PlayInputDto {
                tick: 88,
                fighter: 0,
                mx: 0,
                mz: -60,
                facing: -500,
                buttons: 4,
            },
            PlayInputDto {
                tick: 100,
                fighter: 1,
                mx: 20,
                mz: 20,
                facing: 9000,
                buttons: 2,
            },
        ];
        let args = PlayStepArgs {
            seed: Some(95),
            participants: Some(2),
            half_extent_m: Some(20.0),
            upto_tick: Some(120),
            inputs: Some(inputs.clone()),
            width: None,
            height: None,
            timing: None,
        };
        let mut slot = PlaySlot::default();
        let first = play_step_in(&mut slot, &args).expect("seed95 upto120 serves");
        assert_eq!(slot.requery.len(), 1, "the first query files its receipt");
        let second = play_step_in(&mut slot, &args).expect("seed95 re-query serves");
        assert_eq!(slot.requery.len(), 1, "the re-query replays, never refiles");
        assert_eq!(slot.steps, 120, "the re-query simulated nothing");
        assert_eq!(
            first.digest, second.digest,
            "a re-queried composition must reproduce its digest"
        );
        assert_eq!(
            first.png_bytes, second.png_bytes,
            "a re-queried composition must reproduce its pixels"
        );
        assert_ne!(first.digest, 0, "the duel moves, so the digest is non-zero");
        assert_eq!(first.inputs_applied, inputs.len());
        eprintln!(
            "playtest57 repro digest {} png {} bytes (re-queried x2 identical)",
            first.digest,
            first.png_bytes.len()
        );
        let (_, _, rgb) = decode_png_rgb(&first.png_bytes).expect("repro PNG decodes");
        require_non_black(&rgb).expect("repro PNG is non-black with variance");

        // The same composition through the live session prefix agrees.
        let mut session = PlaySession::new(95, 2, 20.0).expect("duel opens");
        let via_session = session
            .advance(&inputs, 120, None, None)
            .expect("session prefix serves");
        assert_eq!(via_session.digest, first.digest);
        assert_eq!(via_session.png_bytes, first.png_bytes);

        // Sim bytes reproduce over the same composition.
        let tape = build_tape(&inputs, 120, 2).expect("repro tape validates");
        let config = ArenaConfig {
            seed: 95,
            ticks: 120,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let bytes_once = engine_arena::run_match_bytes(&config, &tape);
        let bytes_twice = engine_arena::run_match_bytes(&config, &tape);
        assert!(!bytes_once.is_empty(), "the repro tape simulates");
        assert_eq!(bytes_once, bytes_twice, "sim bytes reproduce");
    }

    #[test]
    fn rejects_stale_past_duplicate_and_bad_config() {
        let mut session = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
        session
            .advance(&step_inputs(0, 0), 2, Some(W), Some(H))
            .expect("first step advances");

        let stale = PlayInputDto {
            tick: 0,
            fighter: 0,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: 0,
        };
        let refused = session
            .advance(std::slice::from_ref(&stale), 2, Some(W), Some(H))
            .expect_err("stale tick must fail");
        assert_eq!(refused.code, "stale_tick");
        assert_eq!(session.cursor(), 2, "a refusal moves no cursor");

        let past_end = PlayInputDto { tick: 9, ..stale };
        let refused = session
            .advance(std::slice::from_ref(&past_end), 2, Some(W), Some(H))
            .expect_err("tick past the step end must fail");
        assert_eq!(refused.code, "invalid_tape");

        let dupe = [
            PlayInputDto {
                tick: 2,
                fighter: 0,
                mx: 10,
                mz: 0,
                facing: 0,
                buttons: 0,
            },
            PlayInputDto {
                tick: 2,
                fighter: 0,
                mx: 20,
                mz: 0,
                facing: 0,
                buttons: 0,
            },
        ];
        let refused = session
            .advance(&dupe, 2, Some(W), Some(H))
            .expect_err("duplicate input must fail");
        assert_eq!(refused.code, "duplicate_input");

        assert_eq!(
            PlaySession::new(SEED, 1, 20.0)
                .expect_err("solo duel must fail")
                .code,
            "bad_participants"
        );
        assert_eq!(
            PlaySession::new(SEED, 2, f32::NAN)
                .expect_err("NaN extent must fail")
                .code,
            "bad_extent"
        );
        let zero = session
            .advance(&step_inputs(2, 1), 2, Some(0), Some(H))
            .expect_err("zero width must fail");
        assert_eq!(zero.code, "bad_size");
    }

    fn tagged_receipt(
        adapter: &str,
        asked: &CaptureArgs,
        digest: u64,
        tick: u32,
    ) -> Result<CaptureOutput, McpToolError> {
        let width = asked.width.unwrap_or(0);
        let height = asked.height.unwrap_or(0);
        let fighters = [
            ActorNumbers {
                id: 0,
                hp: 1000,
                stamina: 100,
                phase: "idle".to_owned(),
                pos: [-5.0, 0.0, 0.0],
                yaw: 0.0,
                reward: 1.0,
                legal_mask: [true, true, true],
                velocity: [0.0, 0.0, 0.0],
            },
            ActorNumbers {
                id: 1,
                hp: 1000,
                stamina: 100,
                phase: "idle".to_owned(),
                pos: [5.0, 0.0, 0.0],
                yaw: 0.0,
                reward: 1.0,
                legal_mask: [true, true, true],
                velocity: [0.0, 0.0, 0.0],
            },
        ];
        let rgb = render_step_pixels(width, height, tick, digest, 99, &fighters, &[]);
        Ok(CaptureOutput {
            manifest: CaptureManifestDto {
                adapter: adapter.to_owned(),
                driver: "test-driver".to_owned(),
                width,
                height,
                validation_errors: 0,
            },
            png_bytes: encode_png_rgb(width, height, &rgb),
        })
    }

    fn gpu_hook(
        asked: &CaptureArgs,
        digest: u64,
        tick: u32,
    ) -> Result<CaptureOutput, McpToolError> {
        tagged_receipt("test-gpu", asked, digest, tick)
    }

    fn hook_alpha(
        asked: &CaptureArgs,
        digest: u64,
        tick: u32,
    ) -> Result<CaptureOutput, McpToolError> {
        tagged_receipt("hook-alpha", asked, digest, tick)
    }

    fn hook_beta(
        asked: &CaptureArgs,
        digest: u64,
        tick: u32,
    ) -> Result<CaptureOutput, McpToolError> {
        tagged_receipt("hook-beta", asked, digest, tick)
    }

    #[test]
    fn play_step_receipt_carries_image_block() {
        let out = play_step_with(&tool_args(2), None).expect("software step answers");
        let result =
            frame_call_result(rmcp::serde_json::to_value(&out).expect("step receipt serializes"));
        assert!(
            result.content.iter().any(
                |item| matches!(item, ContentBlock::Image(image) if image.mime_type == "image/png")
            ),
            "the step answers with one image/png block"
        );
        let structured = result.structured_content.expect("structured content kept");
        assert!(
            structured.get("png_bytes").is_none() && structured.get("manifest").is_some(),
            "the step keeps its manifest and drops the raw array"
        );
    }

    #[test]
    fn gpu_hook_wires_and_validates_receipts() {
        // Wired device path: the hook's receipt wins, never unsupported.
        let mut session =
            PlaySession::new_with_hook(SEED, 2, 20.0, Some(gpu_hook)).expect("duel opens");
        let out = session
            .advance(&step_inputs(0, 0), 2, Some(W), Some(H))
            .expect("hooked step advances");
        assert_eq!(out.manifest.adapter, "test-gpu");
        assert_eq!(&out.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
        let (_, _, rgb) = decode_png_rgb(&out.png_bytes).expect("hooked PNG decodes");
        require_non_black(&rgb).expect("hooked PNG is non-black");

        // Non-PNG bytes from the callback fail loudly, never as success.
        let bad = play_step_with(
            &PlayStepArgs {
                seed: Some(SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
                upto_tick: Some(2),
                inputs: Some(step_inputs(0, 0)),
                width: Some(W),
                height: Some(H),
                timing: None,
            },
            Some(|_, _, _| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-gpu".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: W,
                        height: H,
                        validation_errors: 0,
                    },
                    png_bytes: vec![0, 1, 2, 3],
                })
            }),
        )
        .expect_err("non-PNG hook receipt must fail");
        assert_eq!(bad.code, "bad_receipt");
        assert_ne!(bad.code, "unsupported_capture");

        // Dimension mismatch between receipt and request fails loudly too.
        let skewed = play_step_with(
            &PlayStepArgs {
                seed: Some(SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
                upto_tick: Some(2),
                inputs: Some(step_inputs(0, 0)),
                width: Some(W),
                height: Some(H),
                timing: None,
            },
            Some(|_, _, _| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-gpu".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: 32,
                        height: 32,
                        validation_errors: 0,
                    },
                    png_bytes: PNG_MAGIC.to_vec(),
                })
            }),
        )
        .expect_err("skewed hook receipt must fail");
        assert_eq!(skewed.code, "bad_receipt");
    }

    /// A session whose state was built with `hook` (or none).
    fn session_server(hook: Option<StepCaptureHook>) -> McpServer {
        let state = crate::mcp::ServerState::new(7, Capability::unrestricted(7));
        McpServer::with_state(match hook {
            Some(hook) => state.with_step_capture_hook(hook),
            None => state,
        })
    }

    fn tool_args(upto: u32) -> PlayStepArgs {
        PlayStepArgs {
            seed: Some(SEED),
            participants: Some(2),
            half_extent_m: Some(20.0),
            upto_tick: Some(upto),
            inputs: Some(step_inputs(0, 0)),
            width: Some(W),
            height: Some(H),
            timing: None,
        }
    }

    #[test]
    fn session_hook_serves_its_own_tool_path_only() {
        let args = tool_args(2);
        let hooked = PlayStepTool::invoke(&session_server(Some(gpu_hook)), args.clone())
            .expect("hooked tool serves");
        assert_eq!(hooked.manifest.adapter, "test-gpu");
        // A session built without a hook never sees another session's.
        let soft = PlayStepTool::invoke(&session_server(None), args).expect("software tool serves");
        assert_eq!(soft.manifest.adapter, "endpoint-software");
        assert_eq!(&soft.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
    }

    #[test]
    fn two_sessions_in_parallel_keep_their_own_hooks_caches_and_bouts() {
        let alpha = session_server(Some(hook_alpha));
        let beta = session_server(Some(hook_beta));
        let plain_x = session_server(None);
        let plain_y = session_server(None);
        let (args_x, args_y) = (tool_args(2), tool_args(4));
        let adapters = |server: &McpServer, args: &PlayStepArgs| -> Vec<String> {
            (0..6)
                .map(|_| {
                    PlayStepTool::invoke(server, args.clone())
                        .expect("tool serves")
                        .manifest
                        .adapter
                })
                .collect()
        };
        let results = std::thread::scope(|scope| {
            let jobs = [
                scope.spawn(|| adapters(&alpha, &args_x)),
                scope.spawn(|| adapters(&beta, &args_x)),
                scope.spawn(|| adapters(&plain_x, &args_x)),
                scope.spawn(|| adapters(&plain_y, &args_y)),
            ];
            jobs.map(|job| job.join().expect("session thread finishes"))
        });
        let uniform = |names: &[String], want: &str| names.iter().all(|name| name == want);
        assert!(uniform(&results[0], "hook-alpha"), "{:?}", results[0]);
        assert!(uniform(&results[1], "hook-beta"), "{:?}", results[1]);
        assert!(uniform(&results[2], "endpoint-software"));
        assert!(uniform(&results[3], "endpoint-software"));

        // Each software session filed only its own composition.
        let key = |args: &PlayStepArgs| {
            requery_key(&RequeryParams {
                seed: SEED,
                participants: 2,
                half_bits: 20.0_f32.to_bits(),
                upto: args.upto_tick.expect("tool args name a tick"),
                width: W,
                height: H,
                inputs: &step_inputs(0, 0),
            })
        };
        let (key_x, key_y) = (key(&args_x), key(&args_y));
        {
            let state = plain_x.lock().expect("test lock holds");
            assert_eq!(state.play.requery.len(), 1);
            assert!(state.play.requery.contains(&key_x));
            assert!(!state.play.requery.contains(&key_y));
            assert_eq!(state.play.steps, 2, "its own bout stepped two ticks once");
        }
        {
            let state = plain_y.lock().expect("test lock holds");
            assert_eq!(state.play.requery.len(), 1);
            assert!(state.play.requery.contains(&key_y));
            assert!(!state.play.requery.contains(&key_x));
            assert_eq!(state.play.steps, 4, "its own bout stepped four ticks once");
        }
        // The hooked path bypasses the store: the device owns those pixels.
        for server in [&alpha, &beta] {
            assert_eq!(
                server.lock().expect("test lock holds").play.requery.len(),
                0
            );
        }
    }

    #[test]
    fn digest_is_a_decimal_string_and_seed_takes_both_forms() {
        let out = play_step_with(&tool_args(60), None).expect("step serves");
        assert!(
            out.digest > 1 << 53,
            "the proof needs a digest an f64 cannot hold"
        );
        let json = rmcp::serde_json::to_value(&out).expect("output serializes");
        let text = json["digest"].as_str().expect("digest is a JSON string");
        assert_eq!(text.parse::<u64>().expect("decimal"), out.digest);
        // The number form would have rounded through an f64 client.
        assert_ne!(out.digest as f64 as u64, out.digest);
        let back: PlayStepOutput = rmcp::serde_json::from_value(json).expect("round trips");
        assert_eq!(back, out);
        // Output and input schemas both advertise strings.
        let schema = rmcp::serde_json::to_value(rmcp::schemars::schema_for!(PlayStepOutput))
            .expect("schema serializes");
        assert_eq!(schema["properties"]["digest"]["type"], "string");
        let schema = rmcp::serde_json::to_value(rmcp::schemars::schema_for!(PlayStepArgs))
            .expect("schema serializes");
        let seed_type = schema["properties"]["seed"]["type"].to_string();
        assert!(
            seed_type.contains("string"),
            "seed advertises a string: {seed_type}"
        );
        assert!(
            !seed_type.contains("integer"),
            "not an integer: {seed_type}"
        );

        // Input: a decimal string, a number or nothing.
        let parse = |seed: &str| -> Result<PlayStepArgs, _> {
            rmcp::serde_json::from_str(&format!(r#"{{"upto_tick":2{seed}}}"#))
        };
        assert_eq!(parse("").expect("no seed").seed, None);
        assert_eq!(parse(r#","seed":null"#).expect("null seed").seed, None);
        assert_eq!(
            parse(r#","seed":2040"#).expect("number seed").seed,
            Some(2040)
        );
        assert_eq!(
            parse(r#","seed":"18446744073709551615""#)
                .expect("string seed")
                .seed,
            Some(u64::MAX),
            "a string carries a seed no f64 can"
        );
        for bad in [
            r#","seed":"abc""#,
            r#","seed":-1"#,
            r#","seed":1.5"#,
            r#","seed":true"#,
        ] {
            let refused = parse(bad).expect_err("a bad seed refuses");
            assert!(
                refused.to_string().contains("decimal string"),
                "{bad}: {refused}"
            );
        }
    }

    #[test]
    fn black_or_flat_frames_fail_the_gate() {
        let black = encode_png_rgb(8, 8, &[0u8; 8 * 8 * 3]);
        let (_, _, rgb) = decode_png_rgb(&black).expect("black PNG still decodes");
        let refused = require_non_black(&rgb).expect_err("black must fail");
        assert_eq!(refused.code, "black_capture");

        let flat = encode_png_rgb(8, 8, &[128u8; 8 * 8 * 3]);
        let (_, _, rgb) = decode_png_rgb(&flat).expect("flat PNG still decodes");
        assert_eq!(
            require_non_black(&rgb).expect_err("flat must fail").code,
            "black_capture"
        );

        let garbage = decode_png_rgb(b"not a png").expect_err("garbage must fail");
        assert_eq!(garbage.code, "bad_receipt");
    }

    /// Tick-label legend, pinned on seeds 6700/6800: the `play.step`
    /// 1-indexed receipt (`tick: N`, digest of checkpoint `N - 1`) matches
    /// the `play.run` poll (`tick_count: N`, 0-indexed checkpoints
    /// `0..N`, `final_hash`), and the live session cursor lands on N with
    /// the same digest. Seed 6800 must break seed 6700 equality.
    #[test]
    fn tick_labels_align_with_play_run_on_seeds_6700_6800() {
        use super::super::{PlayRunArgs, PlayRunTool};

        const TICKS: u32 = 8;
        const HALF: f32 = 10.0;
        fn start_args(seed: u64) -> PlayRunArgs {
            PlayRunArgs {
                action: "start".to_owned(),
                seed: Some(seed),
                ticks: Some(TICKS),
                fighters: Some(2),
                half_extent_m: Some(HALF),
                inputs: Some(Vec::new()),
                ttl_polls: None,
                job_id: None,
                scene_rev: Some(1),
                timing: None,
            }
        }
        fn poll_args(job_id: u64) -> PlayRunArgs {
            PlayRunArgs {
                action: "poll".to_owned(),
                seed: None,
                ticks: None,
                fighters: None,
                half_extent_m: None,
                inputs: None,
                ttl_polls: None,
                job_id: Some(job_id),
                scene_rev: None,
                timing: None,
            }
        }

        let mut digests = Vec::with_capacity(2);
        for seed in [6700u64, 6800] {
            // Step side: 1-indexed count N over the 0-indexed 0..N prefix.
            let step_args = PlayStepArgs {
                seed: Some(seed),
                participants: Some(2),
                half_extent_m: Some(HALF),
                upto_tick: Some(TICKS),
                inputs: None,
                width: None,
                height: None,
                timing: None,
            };
            let step = play_step_with(&step_args, None).expect("step serves");
            assert_eq!(step.tick, TICKS, "seed {seed}: receipt carries the count");

            // Run side: start, halfway poll, completing poll.
            let server = pinned_server();
            let started = PlayRunTool::invoke(&server, start_args(seed)).expect("run starts");
            assert_eq!(started.tick_count, TICKS);
            let id = started.job_id;
            let mid = PlayRunTool::invoke(&server, poll_args(id)).expect("halfway poll serves");
            assert_eq!(mid.status, "running");
            let done = PlayRunTool::invoke(&server, poll_args(id)).expect("completing poll serves");
            assert_eq!(done.status, "completed");
            assert_eq!(done.tick_count, TICKS);
            assert_eq!(done.checkpoints.len(), TICKS as usize);
            for (index, point) in done.checkpoints.iter().enumerate() {
                assert_eq!(point.tick, index as u32, "checkpoints stay 0-indexed");
            }
            let last = done.checkpoints.last().expect("8 ticks give checkpoints");
            assert_eq!(last.tick, TICKS - 1);
            assert_eq!(done.final_hash, Some(last.hash));

            // The cross-tool contract: same prefix, same digest, count = id + 1.
            assert_eq!(
                step.digest, last.hash,
                "seed {seed}: step digest is checkpoint N-1"
            );
            assert_eq!(
                format!("{:016x}", step.digest),
                format!("{:016x}", done.final_hash.expect("run has a final hash")),
                "seed {seed}: hex receipts agree across tools"
            );

            // Live session prefix agrees and the cursor lands on N.
            let mut session = PlaySession::new(seed, 2, HALF).expect("duel opens");
            let via_session = session
                .advance(&[], TICKS, None, None)
                .expect("session prefix serves");
            assert_eq!(session.cursor(), TICKS);
            assert_eq!(via_session.tick, TICKS);
            assert_eq!(via_session.digest, step.digest);
            digests.push(step.digest);
        }
        assert_ne!(
            digests[0], digests[1],
            "seed 6800 breaks seed 6700 equality"
        );
    }

    fn death_cause_fighter(id: u8, hp: u16, phase: &str, x: f32) -> ActorNumbers {
        ActorNumbers {
            id,
            hp,
            stamina: 100,
            phase: phase.to_owned(),
            pos: [x, 0.0, 0.0],
            yaw: 0.0,
            reward: f32::from(hp) / 1000.0,
            legal_mask: [true, true, true],
            velocity: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn software_capture_shows_hp_loss() {
        let full = [
            death_cause_fighter(0, 1000, "idle", -5.0),
            death_cause_fighter(1, 1000, "idle", 5.0),
        ];
        let hurt = [
            death_cause_fighter(0, 200, "idle", -5.0),
            death_cause_fighter(1, 1000, "idle", 5.0),
        ];
        let whole = render_step_pixels(W, H, 600, 7, SEED, &full, &[]);
        let damaged = render_step_pixels(W, H, 600, 7, SEED, &hurt, &[]);
        assert_ne!(
            whole, damaged,
            "hp 1000 vs 200 must change the receipt (hp bar)"
        );
    }

    #[test]
    fn software_capture_flashes_recent_hit() {
        let fighters = [
            death_cause_fighter(0, 800, "idle", -5.0),
            death_cause_fighter(1, 1000, "idle", 5.0),
        ];
        let quiet = render_step_pixels(W, H, 600, 7, SEED, &fighters, &[]);
        let hit = BoutEvent {
            tick: 595,
            kind: "hit".to_owned(),
            attacker: Some(1),
            victim: 0,
            amount: 200,
        };
        let flashed = render_step_pixels(W, H, 600, 7, SEED, &fighters, &[hit]);
        assert_ne!(quiet, flashed, "a recent hit must flash the victim");
    }

    #[test]
    fn software_capture_marks_death_pose() {
        let alive = [
            death_cause_fighter(0, 800, "idle", -5.0),
            death_cause_fighter(1, 1000, "idle", 5.0),
        ];
        let dead = [
            death_cause_fighter(0, 0, "out", -5.0),
            death_cause_fighter(1, 1000, "idle", 5.0),
        ];
        let kill = BoutEvent {
            tick: 599,
            kind: "kill".to_owned(),
            attacker: Some(1),
            victim: 0,
            amount: 0,
        };
        let living = render_step_pixels(W, H, 600, 7, SEED, &alive, &[]);
        let fallen = render_step_pixels(W, H, 600, 7, SEED, &dead, &[kill]);
        assert_ne!(living, fallen, "out phase plus kill must mark death");
    }
}
