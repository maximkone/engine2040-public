//! Keypoint verifier: gameplay claims as versioned data specs.
//!
//! Consumer: the proof harness runs [`run_suite`]; future `sim.verify` maps
//! one spec to one tool call. Data owner: the caller owns spec files; this
//! module borrows tapes and owns reports. Units: fixed ticks, hp and damage
//! in permille, integer pixel bytes in captures. Determinism scope:
//! same-machine reproducibility — the same spec files twice give the
//! identical [`SuiteReport`]; digests hash integers only, never floats.
//! Errors: bad specs fail as [`KeypointError`]; gameplay mismatches are
//! [`KeypointFailure`] strings inside a passing report, never panics.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::{run_dataset, run_prefix, ArenaConfig, FighterInput, FighterTape};
use engine_core::hash::FNV64_OFFSET;

use crate::causal::Cause;
use crate::frame_check::digest_hex;
use crate::parse::{find_entry as field, Cursor, ParseError};

/// Spec format version this runner accepts.
pub const SPEC_VERSION: u16 = 1;
/// Gameplay keypoints shipped in `fixtures/keypoints/`.
pub const KEYPOINT_COUNT: usize = 42;
/// Seeded gameplay bugs paired with their keypoints: five on the fork specs,\n/// three on the ARC8 spine specs and three on the ARC11 spine rule specs.
pub const BUG_COUNT: usize = 11;
/// Largest accepted spec file in bytes.
pub const MAX_SPEC_BYTES: usize = 64 * 1024;
/// Largest accepted spec count in one directory.
pub const MAX_SPECS: usize = 64;
/// Longest setup prefix in fixed ticks.
pub const MAX_PREFIX_TICKS: u32 = 240;
/// Longest scripted window in fixed ticks.
pub const MAX_WINDOW_TICKS: u32 = 120;
/// Most obstacle boxes one spine spec may place.
pub const MAX_WALLS: usize = 8;
/// Longest claim string in chars.
pub const MAX_CLAIM_CHARS: usize = 280;
/// Capture edge in pixels; one deterministic raster per keypoint.
pub const CAPTURE_EDGE: u32 = 64;
/// Whole-suite simulated-tick budget (deterministic, no wall clock inside).
pub const SUITE_TICK_BUDGET: u64 = 20_000;
/// Report format version stamped on every suite report.
pub const REPORT_VERSION: u16 = 1;

mod json;
use json::*;
mod spec_parse;
use spec_parse::*;
mod spine;
pub use spine::{SpineRule, SPINE_RULES};
// Injected verification units (arXiv 2605.07442 port): one gameplay claim
// checked in isolation, the state it needs reached by playing to it.
pub mod inject;

// ---- spec types -----------------------------------------------------------

/// One scripted input: an explicit per-tick override. Unlisted
/// `(tick, fighter)` slots run idle, so specs stay short and explicit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScriptInput {
    /// Absolute match tick the input applies before.
    pub tick: u32,
    /// Fighter slot, `0..fighters`.
    pub fighter: u8,
    /// Forward wish, `-100..=100`.
    pub mx: i8,
    /// Right wish, `-100..=100`.
    pub mz: i8,
    /// Facing in centidegrees, `-18000..=18000`.
    pub facing: i16,
    /// Button byte; the fighter contract accepts the three low bits.
    pub buttons: u8,
}

/// Typed assertions over the scripted window. Every field is optional;
///
/// a missing field is unchecked, so each keypoint asserts only its claim.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeypointAsserts {
    /// Final health per slot in permille.
    pub hp: Vec<(u8, u16)>,
    /// Window damage dealt per slot in permille.
    pub damage: Vec<(u8, u16)>,
    /// Window stuck ticks per slot.
    pub stuck: Vec<(u8, u32)>,
    /// True when any fighter is knocked out inside the window.
    pub killed: Option<bool>,
    /// Window-relative ticks with fresh damage, exact.
    pub damage_ticks: Option<Vec<u32>>,
    /// Fork-relative kill tick (1-based), `None` when nobody drops.
    pub ticks_to_kill: Option<Option<u32>>,
    /// Spine track: vitals state bits per slot at the settle tick.
    pub state: Vec<(u8, u8)>,
    /// Spine track: stamina points per slot at the settle tick.
    pub stamina: Vec<(u8, u16)>,
    /// Spine track: planar position `[x, z]` per slot at the settle tick in
    /// whole millimeters.
    pub pos_mm: Vec<(u8, [i32; 2])>,
}

impl KeypointAsserts {
    /// True when any assert reads a spine column only the spine track has.
    pub fn reads_spine(&self) -> bool {
        !(self.state.is_empty() && self.stamina.is_empty() && self.pos_mm.is_empty())
    }
}

/// One gameplay claim as data: a setup prefix that injects mid-match state
/// into a fork, then a bounded scripted window with typed assertions.
#[derive(Clone, PartialEq, Debug)]
pub struct KeypointSpec {
    /// True when the claim runs over the spine columns (`"track":"spine"`):
    /// the whole script plays from tick 0 through the spine step and the
    /// asserts read the vitals and transform columns. False runs the legacy
    /// fork from an injected mid-match state.
    pub spine: bool,
    /// Spine track obstacles as axis-aligned boxes
    /// `[center_x, center_z, half_x, half_z]` in meters; empty is the open arena.
    pub walls: Vec<[f32; 4]>,
    /// File stem and registry id, e.g. `kp06-parry-riposte`.
    pub id: String,
    /// Human claim under test, for reasons only.
    pub claim: String,
    /// Match seed; spawn and prefix trajectories derive from it.
    pub seed: u64,
    /// Fighter count, `2..=MAX_PARTICIPANTS`.
    pub fighters: u8,
    /// Arena half-extent in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
    /// Setup prefix length in ticks; the fork starts here.
    pub prefix_ticks: u32,
    /// Setup inputs over `0..prefix_ticks`, absolute ticks.
    pub setup: Vec<ScriptInput>,
    /// Scripted window length in ticks.
    pub window_ticks: u32,
    /// Window inputs over `prefix..prefix+window`, absolute ticks.
    pub window: Vec<ScriptInput>,
    /// Slots frozen for the whole window via a fork proposal.
    pub freeze: Vec<u8>,
    /// Typed assertions over the window.
    pub asserts: KeypointAsserts,
}

/// Keypoint failures: bad specs fail typed, never partial and never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeypointError {
    /// The spec directory is missing or unreadable.
    DirUnreadable(String),
    /// Too many spec files; the bound carried.
    TooManySpecs(usize),
    /// One file failed to parse or validate; file stem plus reason.
    BadSpec {
        /// File stem of the offending spec.
        file: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The arena refused the built tapes; spec id carried.
    ArenaRefused(String),
}

impl core::fmt::Display for KeypointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DirUnreadable(reason) => write!(f, "keypoint dir unreadable: {reason}"),
            Self::TooManySpecs(count) => write!(f, "too many specs: {count}"),
            Self::BadSpec { file, reason } => write!(f, "bad spec `{file}`: {reason}"),
            Self::ArenaRefused(id) => write!(f, "arena refused keypoint `{id}`"),
        }
    }
}

impl std::error::Error for KeypointError {}

// ---- spec loading ---------------------------------------------------------

/// Directory holding the versioned keypoint specs, resolved from the crate
/// location so the suite runs from any working directory.
pub fn spec_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keypoints")
}

/// Load every `*.json` spec in [`spec_dir`], sorted by file stem. Strict:
/// unknown fields, duplicate keys and out-of-range values all fail.
pub fn load_specs() -> Result<Vec<KeypointSpec>, KeypointError> {
    let dir = spec_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|error| KeypointError::DirUnreadable(error.to_string()))?;
    let mut stems = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| KeypointError::DirUnreadable(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            stems.push(name);
        }
    }
    stems.sort();
    if stems.len() > MAX_SPECS {
        return Err(KeypointError::TooManySpecs(stems.len()));
    }
    let mut specs = Vec::with_capacity(stems.len());
    for name in &stems {
        let stem = name.trim_end_matches(".json").to_owned();
        let bytes = std::fs::read(dir.join(name)).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(KeypointError::BadSpec {
                file: stem.clone(),
                reason: format!("file over {MAX_SPEC_BYTES} bytes"),
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let parsed = Parser::parse(&text).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        let spec = parse_spec(&parsed, &stem).map_err(|error| KeypointError::BadSpec {
            file: stem.clone(),
            reason: error.to_string(),
        })?;
        specs.push(spec);
    }
    let mut ids: Vec<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != specs.len() {
        return Err(KeypointError::BadSpec {
            file: String::new(),
            reason: "duplicate keypoint id".to_owned(),
        });
    }
    Ok(specs)
}

// ---- seeded bugs ------------------------------------------------------------

/// One synthetic tape fault (sensitivity harness, not a paper-accuracy claim):
/// an input-level fault emulating a gameplay bug class inside a single
/// keypoint: the observable end state matches what the real defect would
/// produce (wrong outcome at the asserted tick), scoped so sibling
/// keypoints keep passing. Detection rates here measure sensitivity on
/// these 5 synthetic faults, not GameGen-Verifier accuracy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultOp {
    /// Shift one fighter's window inputs in `[from, to]` by `delta` ticks.
    MoveRange {
        /// Fighter slot to mutate.
        fighter: u8,
        /// First absolute tick to move, inclusive.
        from: u32,
        /// Last absolute tick to move, inclusive.
        to: u32,
        /// Signed shift in ticks.
        delta: i32,
    },
    /// Drop one window input (a swallowed press the claim needed).
    DropInput {
        /// Fighter slot to mutate.
        fighter: u8,
        /// Absolute tick to drop.
        tick: u32,
    },
}

/// One seeded bug: id, paired keypoint and the fault to inject.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BugSeed {
    /// Stable bug id for reports.
    pub id: &'static str,
    /// Bug class under test.
    pub class: &'static str,
    /// Paired keypoint id; exactly this keypoint must fail.
    pub target: &'static str,
    /// Fault to apply to the target's tapes.
    pub op: FaultOp,
}

/// The seeded bugs: wrong window, missing hit-stop, stuck state, a dead\n/// stamina regen and a dropped kill confirm on the fork specs, then the same\n/// kinds of fault on the spine specs (ARC8: parry edge, hit-stop press, arena\n/// clamp; ARC11: stamina regen, kill confirm, chip block).
pub const BUG_SEEDS: [BugSeed; BUG_COUNT] = [
    BugSeed {
        id: "bug-window-parry-late",
        class: "wrong window",
        target: "kp06-parry-riposte",
        op: FaultOp::DropInput {
            fighter: 1,
            tick: 8,
        },
    },
    BugSeed {
        id: "bug-hitstop-clock-runs",
        class: "missing hit-stop",
        target: "kp10-freeze-parry-clock",
        op: FaultOp::MoveRange {
            fighter: 2,
            from: 22,
            to: 22,
            delta: 1,
        },
    },
    BugSeed {
        id: "bug-react-stuck-early",
        class: "stuck state",
        target: "kp21-counter-after-react",
        op: FaultOp::MoveRange {
            fighter: 1,
            from: 18,
            to: 18,
            delta: -5,
        },
    },
    BugSeed {
        id: "bug-stamina-regen-dead",
        class: "missing stamina regen",
        target: "kp14-stamina-regen",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 54,
        },
    },
    BugSeed {
        id: "bug-kill-confirm-dropped",
        class: "dropped kill confirm",
        target: "kp15-kill-confirm",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 192,
        },
    },
    BugSeed {
        id: "bug-spine-parry-edge-missed",
        class: "wrong window",
        target: "spine_parry_first_edge",
        op: FaultOp::DropInput {
            fighter: 1,
            tick: 4,
        },
    },
    BugSeed {
        id: "bug-spine-hitstop-press-lost",
        class: "lost post-hit-stop press",
        target: "spine_hitstop_swallows_press",
        op: FaultOp::DropInput {
            fighter: 2,
            tick: 13,
        },
    },
    BugSeed {
        id: "bug-spine-clamp-short-walk",
        class: "bounds never reached",
        target: "spine_wall_clamp",
        op: FaultOp::MoveRange {
            fighter: 1,
            from: 0,
            to: 119,
            delta: 100,
        },
    },
    BugSeed {
        id: "bug-spine-stamina-regen-dead",
        class: "missing stamina regen",
        target: "spine_rules_stamina_regen",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 54,
        },
    },
    BugSeed {
        id: "bug-spine-kill-confirm-dropped",
        class: "dropped kill confirm",
        target: "spine_kill_confirm",
        op: FaultOp::DropInput {
            fighter: 0,
            tick: 192,
        },
    },
    BugSeed {
        id: "bug-spine-chip-block-released",
        class: "chip block released",
        target: "spine_late_block_chips",
        op: FaultOp::DropInput {
            fighter: 1,
            tick: 8,
        },
    },
];

fn apply_fault(
    window: &mut BTreeMap<(u32, u8), ScriptInput>,
    prefix: u32,
    window_end: u32,
    fault: &BugSeed,
) {
    match fault.op {
        FaultOp::MoveRange {
            fighter,
            from,
            to,
            delta,
        } => {
            let mut moved = Vec::new();
            window.retain(|(tick, slot), entry| {
                if *slot == fighter && *tick >= from && *tick <= to {
                    let shifted = i64::from(*tick) + i64::from(delta);
                    moved.push((*entry, shifted));
                    false
                } else {
                    true
                }
            });
            for (mut entry, shifted) in moved {
                if shifted >= i64::from(prefix) && shifted < i64::from(window_end) {
                    entry.tick = shifted as u32;
                    window.insert((entry.tick, entry.fighter), entry);
                }
            }
        }
        FaultOp::DropInput { fighter, tick } => {
            window.remove(&(tick, fighter));
        }
    }
}

// ---- captures and digests ---------------------------------------------------

/// Multiplier of the keypoint digests. Recorded keypoint evidence pins digests
/// made with it, so it stays distinct from the standard FNV-1a prime
/// (`engine_core::hash::FNV64_PRIME`); changing it re-records every digest.
const DIGEST_PRIME: u64 = 0x1000_0000_01b3;

/// Byte-fold digest builder; every digest in this module folds integers only.
struct Digester {
    hash: u64,
}

impl Digester {
    fn new() -> Self {
        Self { hash: FNV64_OFFSET }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(DIGEST_PRIME);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.hash
    }
}

/// One deterministic top-down capture: floor, walls, the crate prop and one
/// disc per fighter shaded by health with integer math only. Positions map
/// through integer millimeters; no raw float bits are hashed (placement
/// passes an f32-to-int cast first, same-machine scope per HASH-LAW).
fn capture_rgba(fighters: &[(u8, [f32; 3], u16)], half_extent_m: f32, edge: u32) -> Vec<u8> {
    let edge = edge.clamp(8, 256) as usize;
    let mut pixels = vec![0u8; edge * edge * 4];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let x = index % edge;
        let y = index / edge;
        let border = x == 0 || y == 0 || x + 1 == edge || y + 1 == edge;
        if border {
            pixel.copy_from_slice(&[90, 96, 110, 255]);
        } else {
            pixel.copy_from_slice(&[16, 18, 24, 255]);
        }
    }
    let cell = |pos_mm: i32, half_mm: i32| -> Option<usize> {
        if half_mm <= 0 {
            return None;
        }
        let scaled = ((pos_mm + half_mm) as i64 * edge as i64) / (2 * half_mm as i64);
        if scaled < 0 || scaled >= edge as i64 {
            None
        } else {
            Some(scaled as usize)
        }
    };
    let half_mm = (half_extent_m * 1000.0) as i32;
    let dot = |pixels: &mut [u8], cx: usize, cy: usize, color: [u8; 4]| {
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx * dx + dy * dy > 5 {
                    continue;
                }
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x < 0 || y < 0 || x >= edge as i32 || y >= edge as i32 {
                    continue;
                }
                let start = (y as usize * edge + x as usize) * 4;
                pixels[start..start + 4].copy_from_slice(&color);
            }
        }
    };
    const COLORS: [[u8; 3]; 4] = [
        [255, 170, 40],
        [80, 220, 230],
        [230, 110, 220],
        [150, 230, 120],
    ];
    for &(id, pos, hp) in fighters {
        let x_mm = (pos[0] * 1000.0) as i32;
        let z_mm = (pos[2] * 1000.0) as i32;
        let (Some(cx), Some(cy)) = (cell(x_mm, half_mm), cell(z_mm, half_mm)) else {
            continue;
        };
        let shade = 64 + 191 * u32::from(hp.min(1000)) / 1000;
        let base = COLORS[usize::from(id) % COLORS.len()];
        let color = [
            (u32::from(base[0]) * shade / 255) as u8,
            (u32::from(base[1]) * shade / 255) as u8,
            (u32::from(base[2]) * shade / 255) as u8,
            255,
        ];
        dot(&mut pixels, cx, cy, color);
    }
    if let (Some(cx), Some(cy)) = (cell(0, half_mm), cell(0, half_mm)) {
        dot(&mut pixels, cx, cy, [110, 110, 118, 255]);
    }
    pixels
}

// ---- runner -----------------------------------------------------------------

/// How a keypoint executes: injected mid-match state, the same script
/// without injection (ablation), or with one seeded fault applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunMode<'a> {
    Inject,
    Ablate,
    Fault(&'a BugSeed),
    FaultAblated(&'a BugSeed),
}

/// One keypoint outcome: pass plus the evidence that decided it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeypointResult {
    /// Registry id.
    pub id: String,
    /// True when every asserted field matched.
    pub pass: bool,
    /// One line per mismatched field, `expected … observed …`.
    pub failures: Vec<String>,
    /// Causal inputs behind a fault failure, `tick … fighter …`.
    pub causes: Vec<String>,
    /// FNV digest over the integer run evidence.
    pub digest: u64,
    /// Hex digest of the one capture taken.
    pub capture: String,
}

/// Whole-suite verdict: every keypoint outcome plus the suite digest.
/// `pass` is true exactly when every result passes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SuiteReport {
    /// Report format version, always [`REPORT_VERSION`].
    pub version: u16,
    /// One outcome per keypoint, sorted by id.
    pub results: Vec<KeypointResult>,
    /// FNV digest over the ordered per-keypoint digests.
    pub digest: u64,
    /// Total simulated ticks behind the report (the tick budget input).
    pub sim_ticks: u64,
}

impl SuiteReport {
    /// True when every keypoint passed.
    pub fn pass(&self) -> bool {
        self.results.iter().all(|result| result.pass)
    }

    /// Count of passing keypoints.
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|result| result.pass).count()
    }

    /// Spine rules with at least one passing spec, and the rule total
    /// ([`SPINE_RULES`]).
    pub fn rules_covered(&self) -> (usize, usize) {
        let passing = |id: &&str| self.results.iter().any(|r| r.pass && r.id == **id);
        let covered = SPINE_RULES
            .iter()
            .filter(|rule| rule.specs.iter().any(passing))
            .count();
        (covered, SPINE_RULES.len())
    }
}

/// Registry ids in run order, for the proof harness and `sim.verify`.
pub fn keypoint_ids() -> Result<Vec<String>, KeypointError> {
    Ok(load_specs()?.into_iter().map(|spec| spec.id).collect())
}

/// Run the full set with state injection: prefix each match, fork at the
/// prefix tick, run the bounded window, assert typed state, capture once.
pub fn run_suite() -> Result<SuiteReport, KeypointError> {
    let specs = load_specs()?;
    let mut results = Vec::with_capacity(specs.len());
    let mut sim_ticks: u64 = 0;
    for spec in &specs {
        let (result, ticks) = run_keypoint(spec, RunMode::Inject)?;
        sim_ticks += u64::from(ticks);
        results.push(result);
    }
    Ok(finish_suite(results, sim_ticks))
}

/// Run the full set with one seeded fault applied to its target.
pub fn run_suite_with_fault(fault: &BugSeed) -> Result<SuiteReport, KeypointError> {
    let specs = load_specs()?;
    let mut results = Vec::with_capacity(specs.len());
    let mut sim_ticks: u64 = 0;
    for spec in &specs {
        let mode = if spec.id == fault.target {
            RunMode::Fault(fault)
        } else {
            RunMode::Inject
        };
        let (result, ticks) = run_keypoint(spec, mode)?;
        sim_ticks += u64::from(ticks);
        results.push(result);
    }
    Ok(finish_suite(results, sim_ticks))
}

/// Ablation verdict for one seeded bug without state injection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AblationRow {
    /// Seeded bug id.
    pub bug: String,
    /// Paired keypoint id.
    pub target: String,
    /// The unmodified window passes blind from the default state.
    pub baseline_pass: bool,
    /// The faulted window still passes blind: the bug is missed.
    pub fault_pass: bool,
}

impl AblationRow {
    /// True when the bug is caught without injection (baseline green,
    /// fault red). Anything else misses the bug.
    pub fn caught(&self) -> bool {
        self.baseline_pass && !self.fault_pass
    }
}

/// Ablation report: every seeded bug rerun without injection.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AblationReport {
    /// One row per seeded bug, in [`BUG_SEEDS`] order.
    pub rows: Vec<AblationRow>,
}

impl AblationReport {
    /// Bugs missed without injection.
    pub fn missed(&self) -> usize {
        self.rows.iter().filter(|row| !row.caught()).count()
    }
}

/// Run every seeded bug with and without its fault, all without injection:
/// the window script runs from the default state, so injection-armed traps
/// cannot stage and the bugs must go undetected.
pub fn run_ablation() -> Result<AblationReport, KeypointError> {
    let specs = load_specs()?;
    let by_id: BTreeMap<&str, &KeypointSpec> =
        specs.iter().map(|spec| (spec.id.as_str(), spec)).collect();
    let mut rows = Vec::with_capacity(BUG_SEEDS.len());
    for fault in &BUG_SEEDS {
        let Some(spec) = by_id.get(fault.target) else {
            return Err(KeypointError::BadSpec {
                file: fault.target.to_owned(),
                reason: "fault target missing from suite".to_owned(),
            });
        };
        let (baseline, _) = run_keypoint(spec, RunMode::Ablate)?;
        let (faulted, _) = run_keypoint(spec, RunMode::FaultAblated(fault))?;
        rows.push(AblationRow {
            bug: fault.id.to_owned(),
            target: fault.target.to_owned(),
            baseline_pass: baseline.pass,
            fault_pass: faulted.pass,
        });
    }
    Ok(AblationReport { rows })
}

fn finish_suite(mut results: Vec<KeypointResult>, sim_ticks: u64) -> SuiteReport {
    results.sort_by(|a, b| a.id.cmp(&b.id));
    let mut digester = Digester::new();
    digester.u16(REPORT_VERSION);
    for result in &results {
        digester.text(&result.id);
        digester.u8(u8::from(result.pass));
        digester.u64(result.digest);
    }
    SuiteReport {
        version: REPORT_VERSION,
        results,
        digest: digester.finish(),
        sim_ticks,
    }
}

fn run_keypoint(
    spec: &KeypointSpec,
    mode: RunMode<'_>,
) -> Result<(KeypointResult, u32), KeypointError> {
    let ablate = matches!(mode, RunMode::Ablate | RunMode::FaultAblated(_));
    let fault = match mode {
        RunMode::Fault(fault) | RunMode::FaultAblated(fault) => Some(fault),
        _ => None,
    };
    let prefix = if ablate { 0 } else { spec.prefix_ticks };
    let window = spec.window_ticks;
    let total = prefix.saturating_add(window).saturating_add(1);
    let config = ArenaConfig {
        seed: spec.seed,
        ticks: total,
        fighters: spec.fighters,
        half_extent_m: spec.half_extent_m,
        table_version: 1,
    };
    let setup_map: BTreeMap<(u32, u8), ScriptInput> = if ablate {
        BTreeMap::new()
    } else {
        spec.setup
            .iter()
            .map(|entry| ((entry.tick, entry.fighter), *entry))
            .collect()
    };
    let mut window_map: BTreeMap<(u32, u8), ScriptInput> = spec
        .window
        .iter()
        .map(|entry| {
            let tick = if ablate {
                entry.tick.saturating_sub(spec.prefix_ticks)
            } else {
                entry.tick
            };
            let mut shifted = *entry;
            shifted.tick = tick;
            ((tick, shifted.fighter), shifted)
        })
        .collect();
    if let Some(fault) = fault {
        if ablate {
            let base = spec.prefix_ticks;
            let rebased = BugSeed {
                id: fault.id,
                class: fault.class,
                target: fault.target,
                op: rebase_op(fault.op, base),
            };
            apply_fault(&mut window_map, 0, window, &rebased);
        } else {
            apply_fault(
                &mut window_map,
                prefix,
                prefix.saturating_add(window),
                fault,
            );
        }
    }
    let setup_tape = materialize(&setup_map, 0, prefix, spec.fighters)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    // Full window coverage plus one idle settle tick: every `(tick, slot)`
    // is scripted, so brains are never consulted and the run is exact.
    let window_tape = materialize(&window_map, prefix, prefix + window + 1, spec.fighters)
        .ok_or_else(|| KeypointError::ArenaRefused(spec.id.clone()))?;
    let evidence = if spec.spine {
        let tape = FighterTape([setup_tape.0, window_tape.0].concat());
        spine::run(spec, &config, &tape, prefix, window)
    } else {
        run_fork(spec, &config, &setup_tape, &window_tape, prefix, window)
    }?;
    let pixels = evidence.pixels;
    let capture = digest_hex(&pixels);
    let mut failures = evidence.failures;
    let mut causes = Vec::new();
    if pixels.iter().min() == pixels.iter().max() {
        failures.push("capture is a flat field".to_owned());
    } else if fault.is_some() && !failures.is_empty() {
        causes = attribute_fault(spec, prefix, window, &setup_map, &window_map);
    }
    let mut digester = evidence.digester;
    digester.bytes(&pixels);
    Ok((
        KeypointResult {
            id: spec.id.clone(),
            pass: failures.is_empty(),
            failures,
            causes,
            digest: digester.finish(),
            capture,
        },
        total,
    ))
}

/// What one track hands the shared tail: the mismatched asserts, the raster
/// pixels of the settle tick and the digester already folded with the run's
/// integer evidence.
struct Evidence {
    failures: Vec<String>,
    pixels: Vec<u8>,
    digester: Digester,
}

/// Digester seeded with the spec header every track folds first.
fn spec_digester(spec: &KeypointSpec) -> Digester {
    let mut digester = Digester::new();
    digester.text(&spec.id);
    digester.u64(spec.seed);
    digester.u8(spec.fighters);
    digester.u32(spec.prefix_ticks);
    digester.u32(spec.window_ticks);
    digester
}

/// Fork track: prefix the setup tape, fork there, swap in the window tape
/// and run the bounded window with the recorder on.
fn run_fork(
    spec: &KeypointSpec,
    config: &ArenaConfig,
    setup_tape: &FighterTape,
    window_tape: &FighterTape,
    prefix: u32,
    window: u32,
) -> Result<Evidence, KeypointError> {
    let refused = || KeypointError::ArenaRefused(spec.id.clone());
    let mut window_full: BTreeMap<u8, Vec<FighterInput>> = BTreeMap::new();
    for input in &window_tape.0 {
        window_full.entry(input.fighter.0).or_default().push(*input);
    }
    let (snapshot, _) = run_prefix(config, setup_tape, prefix).ok_or_else(refused)?;
    let mut fork = Fork::from_snapshot(snapshot, prefix).ok_or_else(refused)?;
    for slot in &spec.freeze {
        fork.apply_proposal(ForkProposal::FreezeFighter { fighter: *slot })
            .map_err(|_| refused())?;
    }
    for slot in 0..spec.fighters {
        let inputs = window_full.remove(&slot).ok_or_else(refused)?;
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: slot,
            inputs,
        })
        .map_err(|_| refused())?;
    }
    let metrics = fork.run_n(window);
    let (_, dataset) = fork.run_recorded(window + 1);
    let failures = check_asserts(spec, prefix, window, &dataset, &metrics);
    let settle = dataset
        .rows
        .iter()
        .find(|row| row.observation.tick == prefix + window)
        .map_or_else(Vec::new, |row| {
            row.observation
                .fighters
                .iter()
                .map(|entry| (entry.id, entry.pos, entry.hp))
                .collect()
        });
    let pixels = capture_rgba(&settle, spec.half_extent_m, CAPTURE_EDGE);
    let mut digester = spec_digester(spec);
    for row in &dataset.rows {
        digester.u32(row.observation.tick);
        for entry in &row.observation.fighters {
            digester.u8(entry.id);
            digester.u16(entry.hp);
        }
        digester.u8(row.action.fighter);
        digester.u16(row.outcome.damage_before);
        digester.u16(row.outcome.damage_after);
        digester.u8(u8::from(row.outcome.killed));
    }
    for (slot, damage) in &metrics.damage_dealt {
        digester.u8(*slot);
        digester.u16(*damage);
    }
    for (slot, stuck) in &metrics.stuck_ticks {
        digester.u8(*slot);
        digester.u32(*stuck);
    }
    digester.u32(metrics.ticks_to_kill.unwrap_or(u32::MAX));
    Ok(Evidence {
        failures,
        pixels,
        digester,
    })
}
/// Rebase a fault's absolute ticks into ablation space.
fn rebase_op(op: FaultOp, base: u32) -> FaultOp {
    match op {
        FaultOp::MoveRange {
            fighter,
            from,
            to,
            delta,
        } => FaultOp::MoveRange {
            fighter,
            from: from.saturating_sub(base),
            to: to.saturating_sub(base),
            delta,
        },
        FaultOp::DropInput { fighter, tick } => FaultOp::DropInput {
            fighter,
            tick: tick.saturating_sub(base),
        },
    }
}

/// Attribute a fault failure to its input causes: the causal replay names
/// the 1-minimal set for single-input faults, otherwise the tape diff
/// names the moved inputs (a move leaves residue no single reset removes).
fn attribute_fault(
    spec: &KeypointSpec,
    prefix: u32,
    window: u32,
    setup_map: &BTreeMap<(u32, u8), ScriptInput>,
    window_map: &BTreeMap<(u32, u8), ScriptInput>,
) -> Vec<String> {
    let total = prefix + window + 1;
    let config = ArenaConfig {
        seed: spec.seed,
        ticks: total,
        fighters: spec.fighters,
        half_extent_m: spec.half_extent_m,
        table_version: 1,
    };
    let baseline_setup: BTreeMap<(u32, u8), ScriptInput> = spec
        .setup
        .iter()
        .map(|entry| ((entry.tick, entry.fighter), *entry))
        .collect();
    let baseline_window: BTreeMap<(u32, u8), ScriptInput> = spec
        .window
        .iter()
        .map(|entry| ((entry.tick, entry.fighter), *entry))
        .collect();
    let Some(baseline) = full_tape(
        &baseline_setup,
        &baseline_window,
        0,
        prefix + window,
        spec.fighters,
    ) else {
        return Vec::new();
    };
    let Some(faulted) = full_tape(setup_map, window_map, 0, prefix + window, spec.fighters) else {
        return Vec::new();
    };
    match crate::causal::attribute(&config, &baseline, &faulted) {
        Ok(report) if !report.causes.is_empty() => {
            report.causes.into_iter().take(4).map(cause_text).collect()
        }
        _ => diff_inputs(&baseline, &faulted),
    }
}

/// Tape diff fallback: every `(tick, fighter)` where the faulted tape
/// differs from baseline, capped. Deterministic in key order.
fn diff_inputs(baseline: &FighterTape, faulted: &FighterTape) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut keys = BTreeSet::new();
    for input in baseline.0.iter().chain(faulted.0.iter()) {
        keys.insert((input.tick, input.fighter.0));
    }
    let base_by_key: BTreeMap<(u32, u8), FighterInput> = baseline
        .0
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect();
    let fault_by_key: BTreeMap<(u32, u8), FighterInput> = faulted
        .0
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect();
    keys.into_iter()
        .filter(|key| base_by_key.get(key) != fault_by_key.get(key))
        .take(4)
        .map(|(tick, fighter)| format!("tick {tick} fighter {fighter}"))
        .collect()
}

fn cause_text(cause: Cause) -> String {
    format!("tick {} fighter {}", cause.tick, cause.fighter)
}

fn full_tape(
    setup: &BTreeMap<(u32, u8), ScriptInput>,
    window: &BTreeMap<(u32, u8), ScriptInput>,
    lower: u32,
    upper: u32,
    fighters: u8,
) -> Option<FighterTape> {
    let mut merged = setup.clone();
    for (key, entry) in window {
        merged.insert(*key, *entry);
    }
    materialize(&merged, lower, upper, fighters)
}

fn check_asserts(
    spec: &KeypointSpec,
    prefix: u32,
    window: u32,
    dataset: &engine_arena::recorder::Dataset,
    metrics: &engine_arena::fork::ForkMetrics,
) -> Vec<String> {
    let mut failures = Vec::new();
    let end = prefix + window;
    let settle_tick = prefix + window;
    let settle = dataset
        .rows
        .iter()
        .find(|row| row.observation.tick == settle_tick)
        .map(|row| &row.observation);
    let hp_at_settle = |slot: u8| -> Option<u16> {
        settle?
            .fighters
            .iter()
            .find(|entry| entry.id == slot)
            .map(|entry| entry.hp)
    };
    for (slot, expected) in &spec.asserts.hp {
        match hp_at_settle(*slot) {
            Some(observed) if observed == *expected => {}
            Some(observed) => failures.push(format!(
                "hp fighter {slot}: expected {expected} observed {observed}"
            )),
            None => failures.push(format!("hp fighter {slot}: no observation")),
        }
    }
    let mut damage_after: BTreeMap<u8, u16> = BTreeMap::new();
    let mut damage_ticks: Vec<u32> = Vec::new();
    let mut killed = false;
    for row in &dataset.rows {
        if row.observation.tick >= end {
            continue;
        }
        damage_after.insert(row.action.fighter, row.outcome.damage_after);
        if row.outcome.damage_after > row.outcome.damage_before
            && !damage_ticks.contains(&(row.observation.tick - prefix))
        {
            damage_ticks.push(row.observation.tick - prefix);
        }
        killed = killed || row.outcome.killed;
    }
    damage_ticks.sort_unstable();
    for (slot, expected) in &spec.asserts.damage {
        let observed = damage_after.get(slot).copied().unwrap_or(0);
        if observed != *expected {
            failures.push(format!(
                "damage fighter {slot}: expected {expected} observed {observed}"
            ));
        }
    }
    for (slot, expected) in &spec.asserts.stuck {
        let observed = metrics
            .stuck_ticks
            .iter()
            .find(|(known, _)| *known == *slot)
            .map(|(_, stuck)| *stuck);
        match observed {
            Some(value) if value == *expected => {}
            Some(value) => failures.push(format!(
                "stuck fighter {slot}: expected {expected} observed {value}"
            )),
            None => failures.push(format!("stuck fighter {slot}: no metric")),
        }
    }
    if let Some(expected) = spec.asserts.killed {
        if killed != expected {
            failures.push(format!("killed: expected {expected} observed {killed}"));
        }
    }
    if let Some(expected) = &spec.asserts.damage_ticks {
        if &damage_ticks != expected {
            failures.push(format!(
                "damage_ticks: expected {expected:?} observed {damage_ticks:?}"
            ));
        }
    }
    if let Some(expected) = spec.asserts.ticks_to_kill {
        if metrics.ticks_to_kill != expected {
            failures.push(format!(
                "ticks_to_kill: expected {expected:?} observed {:?}",
                metrics.ticks_to_kill
            ));
        }
    }
    let _ = window;
    failures.sort();
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypoint_suite_loads_every_spec() {
        let specs = load_specs().expect("specs load");
        assert_eq!(specs.len(), KEYPOINT_COUNT, "suite holds every keypoint");
        let mut ids: Vec<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KEYPOINT_COUNT, "ids are unique");
        for spec in &specs {
            assert!(
                spec.window_ticks > 0 && spec.window_ticks <= MAX_WINDOW_TICKS,
                "{} window bound",
                spec.id
            );
        }
    }

    #[test]
    fn keypoint_suite_passes_with_injection() {
        let report = run_suite().expect("suite runs");
        assert_eq!(report.results.len(), KEYPOINT_COUNT);
        assert_eq!(report.version, REPORT_VERSION);
        let red: Vec<_> = report
            .results
            .iter()
            .filter(|result| !result.pass)
            .map(|result| format!("{}: {:?}", result.id, result.failures))
            .collect();
        assert!(red.is_empty(), "keypoints must pass: {red:#?}");
        assert!(report.pass());
        assert_eq!(report.passed(), KEYPOINT_COUNT);
        println!(
            "suite digest {:016x} over {} ticks",
            report.digest, report.sim_ticks
        );
        for result in &report.results {
            println!(
                "pass {} digest {:016x} capture {}",
                result.id, result.digest, result.capture
            );
        }
    }

    #[test]
    fn keypoint_suite_reproducible_run1_eq_run2() {
        let first = run_suite().expect("first run");
        let second = run_suite().expect("second run");
        assert_eq!(first, second, "run1 must equal run2");
    }

    #[test]
    fn keypoint_faults_fail_exactly_paired() {
        assert_eq!(BUG_SEEDS.len(), BUG_COUNT);
        for fault in &BUG_SEEDS {
            let report = run_suite_with_fault(fault).expect("fault run");
            let mut failed = Vec::new();
            for result in &report.results {
                if result.pass {
                    continue;
                }
                failed.push(result.id.clone());
            }
            assert_eq!(
                failed,
                vec![fault.target.to_owned()],
                "bug {} must fail exactly its keypoint",
                fault.id
            );
            let target = report
                .results
                .iter()
                .find(|result| result.id == fault.target)
                .expect("target present");
            assert!(!target.failures.is_empty(), "failure must say what broke");
            println!(
                "bug {} fails {}: {:?}",
                fault.id, fault.target, target.failures
            );
        }
    }

    #[test]
    fn keypoint_ablation_without_injection_misses() {
        let ablation = run_ablation().expect("ablation runs");
        assert_eq!(ablation.rows.len(), BUG_COUNT);
        for row in &ablation.rows {
            println!(
                "ablation {} baseline_pass={} fault_pass={} caught={}",
                row.bug,
                row.baseline_pass,
                row.fault_pass,
                row.caught()
            );
        }
        assert!(
            ablation.missed() >= 3,
            "ablation must miss at least 3 of 5 without injection, missed {}",
            ablation.missed()
        );
    }

    #[test]
    fn keypoint_specs_reject_bad_data() {
        let bad_version = serde_like(r#"{"version": 0}"#, "kp-bad");
        assert!(bad_version.is_err(), "version 0 must fail");
        let bad_buttons = Parser::parse(
            r#"{"version":1,"id":"x","claim":"c","seed":1,"fighters":2,"half_extent_m":20.0,"prefix_ticks":0,"window_ticks":4,"setup":[],"window":[{"tick":0,"fighter":0,"mx":0,"mz":0,"facing":0,"buttons":8}],"assert":{}}"#,
        );
        assert!(bad_buttons.is_ok(), "parses as json");
        let spec = parse_spec(&bad_buttons.expect("json"), "x");
        assert!(spec.is_err(), "button byte 8 must fail validation");
        let unknown_field = Parser::parse(r#"{"version":1,"nope":true}"#).expect("json");
        assert!(
            parse_spec(&unknown_field, "x").is_err(),
            "unknown shape fails"
        );
    }

    /// The merged setup-plus-window tape of a spec, as the spine track plays it.
    fn scripted(spec: &KeypointSpec) -> (ArenaConfig, FighterTape) {
        let end = spec.prefix_ticks + spec.window_ticks;
        let config = ArenaConfig {
            seed: spec.seed,
            ticks: end + 1,
            fighters: spec.fighters,
            half_extent_m: spec.half_extent_m,
            table_version: 1,
        };
        let by_key = |inputs: &[ScriptInput]| -> BTreeMap<(u32, u8), ScriptInput> {
            inputs.iter().map(|i| ((i.tick, i.fighter), *i)).collect()
        };
        let tape = full_tape(
            &by_key(&spec.setup),
            &by_key(&spec.window),
            0,
            end + 1,
            spec.fighters,
        )
        .expect("spec tape builds");
        (config, tape)
    }

    #[test]
    fn keypoint_spine_specs_match_the_legacy_step_twice() {
        use engine_arena::parity::{parity, Divergence};
        use engine_arena::world::{MatchWorld, Obstacle};
        let specs = load_specs().expect("specs load");
        let mut open = 0;
        for spec in specs.iter().filter(|spec| spec.spine) {
            let (config, tape) = scripted(spec);
            if spec.walls.is_empty() {
                let world = MatchWorld::empty();
                let first = parity(&config, &tape, &world).expect("both paths build");
                assert!(first.exact(), "{} diverges: {first:?}", spec.id);
                assert_eq!(first.ticks, config.ticks);
                assert_eq!(
                    parity(&config, &tape, &world),
                    Some(first),
                    "{} x2",
                    spec.id
                );
                open += 1;
            } else {
                // A wall exists only on the spine side, so the legacy step must part ways.
                let walls = spec.walls.iter();
                let boxes = walls.map(|w| Obstacle::aligned([w[0], w[1]], [w[2], w[3]]));
                let world = MatchWorld::try_new(boxes.collect()).expect("walls valid");
                let report = parity(&config, &tape, &world).expect("both paths build");
                let (_, why) = report
                    .first_divergence
                    .expect("a wall in the path diverges");
                assert!(matches!(why, Divergence::Column(_)), "{} {why:?}", spec.id);
                assert_eq!(
                    parity(&config, &tape, &world),
                    Some(report),
                    "{} x2",
                    spec.id
                );
            }
        }
        assert_eq!(
            open, 11,
            "ten ARC8 specs plus the stamina rule spec play open"
        );
    }

    #[test]
    fn keypoint_spine_rules_map_is_complete() {
        let specs = load_specs().expect("specs load");
        let spine_ids: Vec<&str> = specs
            .iter()
            .filter(|spec| spec.spine)
            .map(|spec| spec.id.as_str())
            .collect();
        let mut rule_ids: Vec<&str> = SPINE_RULES.iter().map(|rule| rule.id).collect();
        rule_ids.sort_unstable();
        rule_ids.dedup();
        assert_eq!(rule_ids.len(), SPINE_RULES.len(), "rule ids are unique");
        for rule in &SPINE_RULES {
            assert!(matches!(rule.system, "Rules" | "Motion"), "{}", rule.id);
            assert!(!rule.specs.is_empty(), "{} names a spec", rule.id);
            for id in rule.specs {
                assert!(
                    spine_ids.contains(id),
                    "{} names unknown spec {id}",
                    rule.id
                );
            }
        }
        for id in &spine_ids {
            let named = SPINE_RULES.iter().any(|rule| rule.specs.contains(id));
            assert!(named, "spec {id} pins no rule");
        }
        let total = SPINE_RULES.len();
        assert_eq!(
            run_suite().expect("suite runs").rules_covered(),
            (total, total)
        );
        // A rule bug takes its rule off the map: stamina has only the one spec.
        let dead_regen = BUG_SEEDS
            .iter()
            .find(|seed| seed.id == "bug-spine-stamina-regen-dead")
            .expect("bug listed");
        let report = run_suite_with_fault(dead_regen).expect("fault run");
        assert_eq!(report.rules_covered(), (total - 1, total));
    }

    #[test]
    fn keypoint_spine_specs_reject_misuse() {
        let spec = |extra: &str, assert: &str| {
            let text = format!(
                r#"{{"version":1,"id":"x","claim":"c","seed":1,"fighters":2,"half_extent_m":20.0,"prefix_ticks":0,"window_ticks":4,"setup":[],"window":[],{extra}"assert":{{{assert}}}}}"#
            );
            serde_like(&text, "x")
        };
        assert!(spec(r#""track":"spine","#, r#""state":[[0,0]]"#).is_ok());
        assert!(
            spec("", r#""state":[[0,0]]"#).is_err(),
            "state needs the spine track"
        );
        assert!(
            spec(r#""walls":[[0,0,1,1]],"#, "").is_err(),
            "walls need the spine track"
        );
        assert!(
            spec(r#""track":"spine","#, r#""damage":[[0,0]]"#).is_err(),
            "no damage on spine"
        );
        assert!(
            spec(r#""track":"spine","freeze":[0],"#, "").is_err(),
            "no freeze on spine"
        );
        assert!(spec(r#""track":"cloud","#, "").is_err(), "unknown track");
        assert!(
            spec(r#""track":"spine","walls":[[0,0,0,1]],"#, "").is_err(),
            "flat wall"
        );
        assert!(
            spec(r#""track":"spine","#, r#""pos_mm":[[0,1]]"#).is_err(),
            "pos_mm is a triple"
        );
    }

    fn serde_like(text: &str, stem: &str) -> Result<KeypointSpec, KeypointError> {
        let parsed = Parser::parse(text).map_err(|error| KeypointError::BadSpec {
            file: stem.to_owned(),
            reason: error.to_string(),
        })?;
        parse_spec(&parsed, stem).map_err(|error| KeypointError::BadSpec {
            file: stem.to_owned(),
            reason: error.to_string(),
        })
    }

    #[test]
    fn keypoint_captures_cover_and_discern() {
        let report = run_suite().expect("suite runs");
        let mut digests = Vec::new();
        for result in &report.results {
            assert_eq!(result.capture.len(), 16, "capture digest is 16 hex chars");
            assert_ne!(result.capture, "0000000000000000", "capture digest set");
            digests.push(result.capture.clone());
        }
        digests.sort();
        digests.dedup();
        assert!(
            digests.len() >= report.results.len() / 2,
            "captures must discern scenarios, {} unique of {}",
            digests.len(),
            report.results.len()
        );
    }

    #[test]
    fn keypoint_failures_carry_causal_input() {
        let specs = load_specs().expect("specs load");
        let by_id: BTreeMap<&str, &KeypointSpec> =
            specs.iter().map(|spec| (spec.id.as_str(), spec)).collect();
        for fault in &BUG_SEEDS {
            let spec = by_id[fault.target];
            let (result, _) = run_keypoint(spec, RunMode::Fault(fault)).expect("fault runs");
            assert!(
                !result.pass,
                "{} must fail under {}",
                fault.target, fault.id
            );
            assert!(
                !result.causes.is_empty(),
                "{} must name a causal input",
                fault.id
            );
            println!("bug {} causes {:?}", fault.id, result.causes);
        }
        // Single-input drops attribute exactly to the dropped input.
        let drops = [
            ("bug-window-parry-late", "tick 8 fighter 1"),
            ("bug-stamina-regen-dead", "tick 54 fighter 0"),
            ("bug-kill-confirm-dropped", "tick 192 fighter 0"),
            ("bug-spine-parry-edge-missed", "tick 4 fighter 1"),
            ("bug-spine-hitstop-press-lost", "tick 13 fighter 2"),
            ("bug-spine-stamina-regen-dead", "tick 54 fighter 0"),
            ("bug-spine-kill-confirm-dropped", "tick 192 fighter 0"),
            ("bug-spine-chip-block-released", "tick 8 fighter 1"),
        ];
        for (bug, cause) in drops {
            let fault = BUG_SEEDS
                .iter()
                .find(|seed| seed.id == bug)
                .expect("bug listed");
            let spec = by_id[fault.target];
            let (result, _) = run_keypoint(spec, RunMode::Fault(fault)).expect("fault runs");
            assert_eq!(
                result.causes,
                vec![cause.to_owned()],
                "exact cause for {bug}"
            );
        }
    }

    #[test]
    fn keypoint_suite_within_tick_budget() {
        let report = run_suite().expect("suite runs");
        assert!(
            report.sim_ticks <= SUITE_TICK_BUDGET,
            "suite over tick budget: {}",
            report.sim_ticks
        );
        println!("suite ticks {} of {SUITE_TICK_BUDGET}", report.sim_ticks);
    }

    #[test]
    fn keypoint_registry_lists_every_spec() {
        let ids = keypoint_ids().expect("registry lists");
        assert_eq!(ids.len(), KEYPOINT_COUNT);
        let specs = load_specs().expect("specs load");
        for spec in &specs {
            assert!(ids.contains(&spec.id), "{} in registry", spec.id);
        }
    }
}
