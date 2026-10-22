//! scale: 8-32 fighter hardening (bandwidth caps, priority deltas, soak).
//! Owner: net lane. Units: bytes/s and bits/s for bandwidth, whole ticks
//! for time. Determinism: integer math and stable slot order only, so same
//! inputs give same bytes, same selection and same soak hash. Errors:
//! [`ScaleError`], never panics. Cap: match slots come from
//! `engine_input::MAX_PARTICIPANTS` (the canonical cap); this module
//! declares no private fighter cap. Crate edge `engine-net -> engine-input`
//! is in `tools/workspace-check.mjs`.

use std::collections::BTreeMap;

use engine_arena::Checkpoint;
use engine_core::Fnv64;
use engine_input::fighter::{Buttons, ATTACK, BLOCK};
use engine_input::{FighterId, FighterInput, MAX_PARTICIPANTS};

use crate::delta::{encode_delta, FighterSnap, FullSnap};
use crate::netcode::{Channel, KIND_CHANNEL};
use crate::session::{
    encode_input, encode_input_compressed, encode_snapshot, encode_state_delta, NetError,
    KIND_INPUT, KIND_SNAPSHOT, KIND_STATE, SNAPSHOT_HEADER_LEN, SNAPSHOT_SLOT_LEN,
    STATE_DELTA_HEADER_LEN,
};
use crate::validate::SortedUnique;

/// The canonical slot cap as an array length (the one widening of it).
const SLOT_CAP: usize = MAX_PARTICIPANTS as usize;
/// Fixed tick rate in Hz (the engine-core tick [`engine_core::TICK_HZ`], widened to u64).
pub const TICK_HZ: u64 = engine_core::TICK_HZ;
/// Smallest fighter count (matches, like beacons, need at least a pair).
pub const SCALE_MIN_FIGHTERS: u8 = 2;
/// Full-snapshot header bytes on the wire (the session codec [`SNAPSHOT_HEADER_LEN`]).
pub const FULL_HEADER_LEN: usize = SNAPSHOT_HEADER_LEN;
/// Full-snapshot bytes per fighter slot on the wire (the session codec [`SNAPSHOT_SLOT_LEN`]).
pub const FULL_SLOT_LEN: usize = SNAPSHOT_SLOT_LEN;
/// Delta header bytes on the wire (the session codec [`STATE_DELTA_HEADER_LEN`]).
pub const DELTA_HEADER_LEN: usize = STATE_DELTA_HEADER_LEN;
/// Largest single snapshot datagram in bytes (loopback UDP, no fragmentation).
pub const MAX_DATAGRAM_BYTES: usize = 1200;
/// Resimulation cap in ticks per frame (authoritative rule, owned copy,
/// measured by the posted MP6 distributions, see `MP6_POSTED_*` below).
pub const RESIM_CAP_TICKS: u32 = 8;
/// Adaptive input delay for a bout hold (MP24): the round-trip worst minus
/// the resim window, saturating at zero.
///
/// `hold` is `worst_delay + 1` (see `BoutConfig::canonical`): the round-trip
/// worst is `2 * (hold - 1)`, plus one tick of authoring slack, minus the
/// 8-tick window. LAN (hold 3) stays 0, WAN (hold 10) delays 11 ticks, so
/// every live resimulation lands inside [`RESIM_CAP_TICKS`]. Pure integer
/// math over the config, no clock, no hash input: the same config draws the
/// same delay on every run. Caller: `crates/net/src/played.rs` `BoutPart`.
#[must_use]
pub const fn adaptive_input_delay_for_hold(hold: u32) -> u32 {
    let worst = hold.saturating_sub(1);
    worst.saturating_add(hold).saturating_sub(RESIM_CAP_TICKS)
}

/// Posted MP6 pacing measurement the MP26 accept bar reads (seed 2040
/// `net-bout mp6`, `target/mp6/distributions.json`): at the 2-tick default
/// delay over 1000 ticks of the clean one-tick link, the client never
/// mispredicted, so the rollbacks taken per 1000 ticks stayed under the bar
/// and the deepest sat inside the resim window. The same link with the
/// delay off mispredicts on every frame, so the posted rate has teeth.
/// Logged, never hashed.
pub const MP6_POSTED_ROLLBACKS_PER_1000: u32 = 0;

/// Posted MP6 live distributions sizing the resim window and the input
/// delay (seed 2040 `net-bout mp6`, `target/mp6/distributions.json`):
/// LAN live rollback max 6, WAN live rollback max 8, WAN one-way delay
/// max 9, live states deeper than the window 0 on both seats, LAN hold 3,
/// WAN hold 10. Digest is FNV-1a over the six posted `u32` words in order
/// `[lan_max, wan_max, wan_delay_max, deeper, lan_hold, wan_hold]`
/// (`engine_core::Fnv64::write_u32`); the sizing test below recomputes it,
/// so fresh hardware data moves the digest and fails the fixture. Units:
/// whole ticks. Determinism: integer max only, no clock.
#[cfg(test)]
const MP6_POSTED_LAN_ROLLBACK_MAX: u32 = 6;
#[cfg(test)]
const MP6_POSTED_WAN_ROLLBACK_MAX: u32 = 8;
#[cfg(test)]
const MP6_POSTED_WAN_DELAY_MAX: u32 = 9;
#[cfg(test)]
const MP6_POSTED_DEEPER_THAN_WINDOW: u32 = 0;
#[cfg(test)]
const MP6_POSTED_LAN_HOLD: u32 = 3;
#[cfg(test)]
const MP6_POSTED_WAN_HOLD: u32 = 10;
#[cfg(test)]
const MP6_POSTED_DISTRIBUTIONS_DIGEST: u64 = 0x9fc9_f9b2_6e90_4e0b;

/// Measured resim window in ticks: the deeper of the posted LAN/WAN live
/// rollback maxima. Integer max only, so the same postings size the same
/// window on every run.
#[cfg(test)]
const fn measured_resim_window(lan_max: u32, wan_max: u32) -> u32 {
    if lan_max > wan_max {
        lan_max
    } else {
        wan_max
    }
}
/// Headless 32-bot soak length in ticks (the proof bar).
pub const SOAK_TICKS: u32 = 600;
/// Upper bound on soak length so a stray caller cannot hang a gate.
pub const SOAK_TICKS_MAX: u32 = 3600;

/// Priority classes, lower first: local inputs, remote combat state, pose
/// deltas, idle crowd. Director hints never enter the snapshot at all.
pub const PRIORITY_LOCAL: u8 = 0;
/// A fighter whose hp or stamina changed since the baseline.
pub const PRIORITY_COMBAT: u8 = 1;
/// A fighter whose pose changed since the baseline.
pub const PRIORITY_POSE: u8 = 2;
/// A fighter identical to the baseline (crowd/ambient, dropped first).
pub const PRIORITY_IDLE: u8 = 3;

/// Scale failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScaleError {
    /// Fighter count outside `2..=MAX_PARTICIPANTS`.
    BadFighterCount {
        /// Count the caller offered.
        got: usize,
    },
    /// Snapshot with no fighters is not a snapshot.
    EmptySnapshot,
    /// Slots must arrive sorted and unique.
    UnsortedSlots,
    /// Current and baseline snapshots name different slot populations.
    SlotMismatch,
    /// Target tick older than the baseline it diffs against.
    TickRegression {
        /// Target tick offered.
        tick: u32,
        /// Baseline tick offered.
        base_tick: u32,
    },
    /// Viewer slot is not in the snapshot.
    UnknownViewer {
        /// Viewer slot offered.
        viewer: u8,
    },
    /// Even the cheapest legal choice exceeds the byte budget.
    OverBudget {
        /// Bytes the cheapest choice needs.
        need: usize,
        /// Bytes the caller allowed.
        budget: usize,
    },
    /// Caller output buffer shorter than the fighter count.
    OutTooSmall {
        /// Slots that must fit.
        need: usize,
        /// Slots the buffer fits.
        have: usize,
    },
}

impl std::fmt::Display for ScaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadFighterCount { got } => {
                write!(f, "fighter count {got} outside 2..={MAX_PARTICIPANTS}")
            }
            Self::EmptySnapshot => write!(f, "snapshot has no fighters"),
            Self::UnsortedSlots => write!(f, "slots must be sorted and unique"),
            Self::SlotMismatch => write!(f, "current and baseline name different slots"),
            Self::TickRegression { tick, base_tick } => {
                write!(f, "target tick {tick} older than baseline {base_tick}")
            }
            Self::UnknownViewer { viewer } => {
                write!(f, "viewer slot {viewer} not in snapshot")
            }
            Self::OverBudget { need, budget } => {
                write!(f, "cheapest choice needs {need} bytes over budget {budget}")
            }
            Self::OutTooSmall { need, have } => {
                write!(f, "output fits {have} slots but {need} must fit")
            }
        }
    }
}

impl std::error::Error for ScaleError {}

/// One fighter's quantized pose plus combat state. Plain snapshot shape owned
/// by this module: same field layout as the delta codec by wire parity, read
/// here only through these fields, never by importing another module's type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FighterView {
    /// Fighter slot, `0..MAX_PARTICIPANTS`.
    pub slot: u8,
    /// Feet x in millimeters.
    pub x_mm: i32,
    /// Feet y in millimeters.
    pub y_mm: i32,
    /// Feet z in millimeters.
    pub z_mm: i32,
    /// Facing yaw in decidegrees.
    pub yaw_ddeg: i16,
    /// Health in permille.
    pub hp: u16,
    /// Stamina points.
    pub stamina: u16,
}

impl FighterView {
    /// True when combat state (hp or stamina) changed since the baseline.
    #[must_use]
    pub const fn combat_changed(self, base: Self) -> bool {
        self.hp != base.hp || self.stamina != base.stamina
    }

    /// True when pose (position or yaw) changed since the baseline.
    #[must_use]
    pub const fn pose_changed(self, base: Self) -> bool {
        self.x_mm != base.x_mm
            || self.y_mm != base.y_mm
            || self.z_mm != base.z_mm
            || self.yaw_ddeg != base.yaw_ddeg
    }
}

/// Priority class of one slot against its baseline: viewer first, then combat
/// changes, then pose changes, then idle slots the budget drops first.
#[must_use]
pub const fn priority_of(now: FighterView, base: FighterView, viewer: u8) -> u8 {
    if now.slot == viewer {
        PRIORITY_LOCAL
    } else if now.combat_changed(base) {
        PRIORITY_COMBAT
    } else if now.pose_changed(base) {
        PRIORITY_POSE
    } else {
        PRIORITY_IDLE
    }
}

/// Wire cost in bytes of sending `now` diffed against `base`: slot plus mask
/// bytes plus only the differing fields (4 per changed axis, 2 for yaw/hp/
/// stamina). An unchanged slot still costs its 2 header bytes when selected.
#[must_use]
pub const fn delta_cost_bytes(now: FighterView, base: FighterView) -> usize {
    let mut n = 2usize;
    if now.x_mm != base.x_mm {
        n += 4;
    }
    if now.y_mm != base.y_mm {
        n += 4;
    }
    if now.z_mm != base.z_mm {
        n += 4;
    }
    if now.yaw_ddeg != base.yaw_ddeg {
        n += 2;
    }
    if now.hp != base.hp {
        n += 2;
    }
    if now.stamina != base.stamina {
        n += 2;
    }
    n
}

/// Full-snapshot wire bytes for `fighters` slots (header plus every slot).
#[must_use]
pub const fn full_wire_len(fighters: usize) -> usize {
    FULL_HEADER_LEN + fighters * FULL_SLOT_LEN
}

/// Snapshot rate in Hz: 20 up to 8 fighters, 15 above (the chart rows).
pub const fn snapshot_hz(fighters: u8) -> Result<u64, ScaleError> {
    // Explicit bounds: `RangeInclusive::contains` is not const.
    if fighters < SCALE_MIN_FIGHTERS || fighters > MAX_PARTICIPANTS {
        return Err(ScaleError::BadFighterCount {
            got: fighters as usize,
        });
    }
    if fighters <= 8 {
        Ok(20)
    } else {
        Ok(15)
    }
}

/// Per-client bandwidth cap in bytes/s: 2 fighters map to 16 kbps, up to 8 to
/// 64 kbps, up to 32 to 96 kbps (kbps * 1000 / 8).
pub const fn cap_bytes_per_sec(fighters: u8) -> Result<u64, ScaleError> {
    // Explicit bounds: `RangeInclusive::contains` is not const.
    if fighters < SCALE_MIN_FIGHTERS || fighters > MAX_PARTICIPANTS {
        return Err(ScaleError::BadFighterCount {
            got: fighters as usize,
        });
    }
    if fighters <= 2 {
        Ok(2_000)
    } else if fighters <= 8 {
        Ok(8_000)
    } else {
        Ok(12_000)
    }
}

/// Bytes/s carried when every snapshot costs `bytes_per_snapshot` at `hz`.
/// Saturating: absurd inputs pin at `u64::MAX` instead of wrapping.
#[must_use]
pub const fn bytes_per_sec(bytes_per_snapshot: usize, hz: u64) -> u64 {
    (bytes_per_snapshot as u64).saturating_mul(hz)
}

/// Bits/s carried (bytes/s times 8, saturating).
#[must_use]
pub const fn bits_per_sec(bytes_per_snapshot: usize, hz: u64) -> u64 {
    bytes_per_sec(bytes_per_snapshot, hz).saturating_mul(8)
}

/// Kilobits/s carried (presentation helper; gates compare integer bytes/s).
#[must_use]
pub fn kbps(bytes_per_snapshot: usize, hz: u64) -> f64 {
    bits_per_sec(bytes_per_snapshot, hz) as f64 / 1_000.0
}

/// Per-snapshot byte budget for `fighters` at the chart rate: cap divided by
/// rate, so a snapshot that fits always streams under the kbps cap.
pub fn cap_bytes_per_snapshot(fighters: u8) -> Result<usize, ScaleError> {
    let cap = cap_bytes_per_sec(fighters)?;
    let hz = snapshot_hz(fighters)?;
    Ok((cap / hz) as usize)
}

/// Gate one snapshot size: it must fit a datagram and stream under the
/// per-client cap at the chart rate. Anything else is [`ScaleError`].
pub fn within_cap(fighters: u8, bytes_per_snapshot: usize) -> Result<(), ScaleError> {
    if !(SCALE_MIN_FIGHTERS..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(ScaleError::BadFighterCount {
            got: usize::from(fighters),
        });
    }
    let hz = snapshot_hz(fighters)?;
    let cap = cap_bytes_per_sec(fighters)?;
    if bytes_per_snapshot > MAX_DATAGRAM_BYTES {
        return Err(ScaleError::OverBudget {
            need: bytes_per_snapshot,
            budget: MAX_DATAGRAM_BYTES,
        });
    }
    let carried = bytes_per_sec(bytes_per_snapshot, hz);
    if carried > cap {
        return Err(ScaleError::OverBudget {
            need: bytes_per_snapshot,
            budget: (cap / hz) as usize,
        });
    }
    Ok(())
}

/// Check that slots are sorted, unique and in range; empty is its own error.
fn check_order(slots: &[FighterView]) -> Result<(), ScaleError> {
    if slots.is_empty() {
        return Err(ScaleError::EmptySnapshot);
    }
    if slots.len() > SLOT_CAP {
        return Err(ScaleError::BadFighterCount { got: slots.len() });
    }
    let mut order = SortedUnique::new();
    for view in slots {
        if view.slot >= MAX_PARTICIPANTS {
            return Err(ScaleError::BadFighterCount {
                got: usize::from(view.slot),
            });
        }
        if !order.push(view.slot) {
            return Err(ScaleError::UnsortedSlots);
        }
    }
    Ok(())
}

/// Priority-delta selection as a pure function over plain snapshot shapes.
///
/// Ranks every slot by ([`priority_of`], slot) with a stable insertion order,
/// keeps the viewer always, then takes the longest rank-ordered prefix whose
/// running delta (`DELTA_HEADER_LEN` plus taken [`delta_cost_bytes`]) fits
/// `budget_bytes`. Prefix (not skip-fit) keeps classes absolute: an idle slot
/// never ships while a pose delta is dropped. Taken slots are written sorted
/// ascending into `out`; the return is how many were written. Budget: O(n)
/// time over at most 32 slots, zero heap use (all working state is stack
/// arrays plus the caller buffer), so this is safe to call on the snapshot
/// path without steady-state allocation.
pub fn select_priority(
    current: &[FighterView],
    baseline: &[FighterView],
    viewer: u8,
    tick: u32,
    base_tick: u32,
    budget_bytes: usize,
    out: &mut [u8],
) -> Result<usize, ScaleError> {
    if tick < base_tick {
        return Err(ScaleError::TickRegression { tick, base_tick });
    }
    check_order(current)?;
    check_order(baseline)?;
    if current.len() < usize::from(SCALE_MIN_FIGHTERS) {
        return Err(ScaleError::BadFighterCount { got: current.len() });
    }
    if current.len() != baseline.len()
        || current
            .iter()
            .zip(baseline.iter())
            .any(|(a, b)| a.slot != b.slot)
    {
        return Err(ScaleError::SlotMismatch);
    }
    if out.len() < current.len() {
        return Err(ScaleError::OutTooSmall {
            need: current.len(),
            have: out.len(),
        });
    }
    let viewer_index = current
        .iter()
        .position(|view| view.slot == viewer)
        .ok_or(ScaleError::UnknownViewer { viewer })?;

    // Rank (priority, slot) with a stable insertion pass over a stack array.
    let mut rank: [(u8, u8); SLOT_CAP] = [(u8::MAX, u8::MAX); SLOT_CAP];
    let mut len = 0usize;
    for (index, view) in current.iter().enumerate() {
        let key = (priority_of(*view, baseline[index], viewer), view.slot);
        let mut at = len;
        while at > 0 && rank[at - 1] > key {
            rank[at] = rank[at - 1];
            at -= 1;
        }
        rank[at] = key;
        len += 1;
    }

    // The viewer always ships: budget below header plus viewer cost refuses.
    let viewer_cost = delta_cost_bytes(current[viewer_index], baseline[viewer_index]);
    if DELTA_HEADER_LEN.saturating_add(viewer_cost) > budget_bytes {
        return Err(ScaleError::OverBudget {
            need: DELTA_HEADER_LEN.saturating_add(viewer_cost),
            budget: budget_bytes,
        });
    }

    // Longest rank-ordered prefix that fits; the first item that overflows
    // holds the line so classes stay absolute. The running total decides,
    // never the clock.
    let mut taken = [false; SLOT_CAP];
    let mut used = DELTA_HEADER_LEN;
    taken[viewer_index] = true;
    used += viewer_cost;
    for (_, slot) in rank.iter().take(len) {
        let index = current
            .iter()
            .position(|view| view.slot == *slot)
            .expect("ranked slot is in the snapshot");
        if taken[index] {
            continue;
        }
        let cost = delta_cost_bytes(current[index], baseline[index]);
        if used.saturating_add(cost) > budget_bytes {
            break;
        }
        taken[index] = true;
        used += cost;
    }

    let mut count = 0usize;
    for (index, view) in current.iter().enumerate() {
        if taken[index] {
            out[count] = view.slot;
            count += 1;
        }
    }
    Ok(count)
}

/// Deterministic headless bot: integer lattice position plus combat attrition,
/// stepped with wrapping integer ops only (no float, no wall clock).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SoakBot {
    x_mm: i32,
    z_mm: i32,
    yaw_ddeg: i16,
    hp: u16,
}

impl SoakBot {
    /// Seed one bot: spread across the arena by slot, hp full.
    const fn seed_bot(slot: u8, seed: u64) -> Self {
        let spread = (slot as u64)
            .wrapping_mul(1_234_567)
            .wrapping_add(seed % 1_000_000);
        Self {
            x_mm: (spread % 20_000) as i32 - 10_000,
            z_mm: ((spread / 20_000) % 20_000) as i32 - 10_000,
            yaw_ddeg: (spread % 3_600) as i16 - 1_800,
            hp: 1_000,
        }
    }

    /// Step one bot one tick: lattice walk plus scripted attrition.
    fn step(&mut self, slot: u8, tick: u32, seed: u64) {
        let mix = u64::from(tick)
            .wrapping_mul(2_654_435_761)
            .wrapping_add(u64::from(slot).wrapping_mul(97_531))
            .wrapping_add(seed);
        let dx = i32::try_from(mix % 2_049).unwrap_or(0) - 1_024;
        let dz = i32::try_from((mix >> 11) % 2_049).unwrap_or(0) - 1_024;
        self.x_mm = self.x_mm.saturating_add(dx);
        self.z_mm = self.z_mm.saturating_add(dz);
        self.swing(slot, tick, seed);
    }

    /// Yaw walk plus scripted attrition folded into every lattice step.
    fn swing(&mut self, slot: u8, tick: u32, seed: u64) {
        let mix = u64::from(tick)
            .wrapping_mul(2_654_435_761)
            .wrapping_add(u64::from(slot).wrapping_mul(97_531))
            .wrapping_add(seed);
        let yaw = i32::from(self.yaw_ddeg) + i32::try_from((mix >> 22) % 361).unwrap_or(0) - 180;
        let mut wrapped = yaw % 3_600;
        if wrapped < -1_800 {
            wrapped += 3_600;
        } else if wrapped >= 1_800 {
            wrapped -= 3_600;
        }
        self.yaw_ddeg = wrapped as i16;
        if u64::from(tick)
            .wrapping_add(u64::from(slot))
            .is_multiple_of(97)
        {
            self.hp = self.hp.saturating_sub(7);
        }
    }
}

/// Fold one bot into an FNV-1a hash (integer ops only, stable across runs).
const fn fold_bot(hash: u64, bot: &SoakBot) -> u64 {
    let mut h = hash;
    h ^= bot.x_mm as u64;
    h = h.wrapping_mul(1_099_511_628_211);
    h ^= bot.z_mm as u64;
    h = h.wrapping_mul(1_099_511_628_211);
    h ^= bot.yaw_ddeg as u64;
    h = h.wrapping_mul(1_099_511_628_211);
    h ^= bot.hp as u64;
    h = h.wrapping_mul(1_099_511_628_211);
    h
}

/// Deterministic soak hash: `fighters` bots stepped `ticks` ticks from `seed`.
/// Same seed plus same counts give the same hash on every run (the 32-bot
/// x600 proof asserts two runs agree); different seeds diverge.
pub fn soak_hash(seed: u64, fighters: u8, ticks: u32) -> Result<u64, ScaleError> {
    if !(SCALE_MIN_FIGHTERS..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(ScaleError::BadFighterCount {
            got: usize::from(fighters),
        });
    }
    if ticks > SOAK_TICKS_MAX {
        return Err(ScaleError::TickRegression {
            tick: ticks,
            base_tick: SOAK_TICKS_MAX,
        });
    }
    let mut bots: [SoakBot; SLOT_CAP] = [SoakBot::seed_bot(0, seed); SLOT_CAP];
    for (slot, bot) in bots.iter_mut().enumerate().take(usize::from(fighters)) {
        *bot = SoakBot::seed_bot(slot as u8, seed);
    }
    let mut hash: u64 = 1_464_717_047_381_303_168 ^ seed;
    for tick in 0..ticks {
        for (slot, bot) in bots.iter_mut().enumerate().take(usize::from(fighters)) {
            bot.step(slot as u8, tick, seed);
            hash = fold_bot(hash, bot);
        }
        hash ^= u64::from(tick);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    Ok(hash)
}

/// Headless soak report: the converged hash plus the bandwidth the matching
/// full snapshots would carry per client at the chart rate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SoakReport {
    /// Fighters in the soak.
    pub fighters: u8,
    /// Ticks stepped.
    pub ticks: u32,
    /// Converged hash (two runs must agree).
    pub hash: u64,
    /// Full-snapshot bytes per snapshot.
    pub snapshot_bytes: usize,
    /// Snapshot rate in Hz.
    pub snapshot_hz: u64,
    /// Bytes/s per client at the chart rate.
    pub bytes_per_sec: u64,
}

impl std::fmt::Display for SoakReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SOAK bots={} ticks={} hash={:016x} snap_bytes={} hz={} Bps={} kbps={:.2}",
            self.fighters,
            self.ticks,
            self.hash,
            self.snapshot_bytes,
            self.snapshot_hz,
            self.bytes_per_sec,
            self.bytes_per_sec as f64 * 8.0 / 1_000.0,
        )
    }
}

/// Run the headless soak: step the bots, hash them, and attach the bandwidth
/// the matching full snapshots would carry. Loopback only: no sockets, no
/// two-machine play here; played sessions stay operator-side hardware.
pub fn run_soak(seed: u64, fighters: u8, ticks: u32) -> Result<SoakReport, ScaleError> {
    let hash = soak_hash(seed, fighters, ticks)?;
    let snapshot_bytes = full_wire_len(usize::from(fighters));
    within_cap(fighters, snapshot_bytes)?;
    let hz = snapshot_hz(fighters)?;
    Ok(SoakReport {
        fighters,
        ticks,
        hash,
        snapshot_bytes,
        snapshot_hz: hz,
        bytes_per_sec: bytes_per_sec(snapshot_bytes, hz),
    })
}

/// Bandwidth chart row the fixtures assert: fighters, rate, snapshot bytes,
/// bytes/s and the kbps target the row must stay under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChartRow {
    /// Fighters in the row.
    pub fighters: u8,
    /// Snapshot rate in Hz.
    pub hz: u64,
    /// Full-snapshot bytes.
    pub snapshot_bytes: usize,
    /// Bytes/s per client.
    pub bytes_per_sec: u64,
    /// kbps target the row must stay under (whole kbps).
    pub kbps_target: u64,
}

/// Posted per-client bandwidth chart: 2 fighters at 20 Hz under 16 kbps,
/// 8 at 20 Hz under 64 kbps, 16 and 32 at 15 Hz under 96 kbps.
pub fn bandwidth_chart() -> [ChartRow; 4] {
    let mut rows = [ChartRow {
        fighters: 0,
        hz: 0,
        snapshot_bytes: 0,
        bytes_per_sec: 0,
        kbps_target: 0,
    }; 4];
    let targets = [(2u8, 16u64), (8u8, 64u64), (16u8, 96u64), (32u8, 96u64)];
    let mut at = 0usize;
    while at < targets.len() {
        let (fighters, target) = targets[at];
        let hz = snapshot_hz(fighters).expect("chart rows are legal counts");
        let bytes = full_wire_len(usize::from(fighters));
        rows[at] = ChartRow {
            fighters,
            hz,
            snapshot_bytes: bytes,
            bytes_per_sec: bytes_per_sec(bytes, hz),
            kbps_target: target,
        };
        at += 1;
    }
    rows
}

/// Slots in the flood chart: the 16-fighter shape the MP18 bar names.
const FLOOD_SLOTS: u8 = 16;
/// The same count as an array length; the chart tests hold the pair equal.
const FLOOD_VIEWS: usize = 16;
/// Slots that fight on the flood (wounded, so combat outranks pose).
const FLOOD_COMBAT_SLOTS: u8 = 8;
/// Slots that moved on the flood (combat plus pose); the rest hold still.
const FLOOD_MOVING_SLOTS: u8 = 12;
/// Flood tick and the baseline tick it diffs against.
const FLOOD_TICK: u32 = 9;
/// Baseline tick of the flood chart's delta.
const FLOOD_BASE_TICK: u32 = 8;
/// Checkpoint hash stamped on the charted state packets. A fixture stamp, not
/// a sim hash: nothing charted here feeds the tick.
const FLOOD_CHECKPOINT_HASH: u64 = 0x2040_0000_0000_0009;

/// One row of the flood chart: a wire kind, the channel and priority the MP8
/// table gives it, and the bytes the MP9 codec puts on the wire. Units: bytes
/// per packet, bytes/s at the row rate through the helpers. Determinism: whole
/// integers read from the table and the codecs, no clock, no allocation here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FloodRow {
    /// Wire kind byte, read from [`KIND_CHANNEL`], never restated here.
    pub kind: u8,
    /// Transport channel the table maps the kind to.
    pub channel: Channel,
    /// Priority class the table gives the kind, lower first.
    pub priority: u8,
    /// Rate the row's packet is sent at, Hz (inputs ride every tick).
    pub hz: u64,
    /// Bytes per packet without the MP9 codec.
    pub raw_bytes: usize,
    /// Bytes per packet with the MP9 codec. Equal to raw where a kind has no
    /// codec yet, so its saving reads zero instead of guessing.
    pub packed_bytes: usize,
}

impl FloodRow {
    /// Bytes/s carried by the raw shape at the row rate.
    #[must_use]
    pub const fn raw_bytes_per_sec(self) -> u64 {
        bytes_per_sec(self.raw_bytes, self.hz)
    }

    /// Bytes/s carried by the packed shape at the row rate.
    #[must_use]
    pub const fn packed_bytes_per_sec(self) -> u64 {
        bytes_per_sec(self.packed_bytes, self.hz)
    }

    /// Whole permille of raw bytes the codec removes; zero when the row has
    /// no codec or the packed shape is not smaller.
    #[must_use]
    pub fn saving_permille(self) -> u64 {
        if self.raw_bytes == 0 || self.packed_bytes > self.raw_bytes {
            return 0;
        }
        u64::try_from((self.raw_bytes - self.packed_bytes) * 1_000 / self.raw_bytes)
            .unwrap_or(u64::MAX)
    }
}

/// The MP18 flood chart: the MP8 channel table and the MP9 input codec drawn
/// on one 16-fighter flood, with the MP16 shed and the quiet tape beside them,
/// so one chart decides both rows. Twelve slots move (eight fight), four hold
/// still as idle crowd; the quiet tape moves only the viewer by one
/// millimetre. Chart helper, not a wire path: no sockets, no clock, and the
/// same flood draws the same chart on every run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FloodChart {
    /// One row per [`KIND_CHANNEL`] entry, table order.
    pub rows: [FloodRow; KIND_CHANNEL.len()],
    /// Slots the MP16 shed keeps under the chart budget.
    pub kept_slots: usize,
    /// Slots the shed drops on this flood.
    pub dropped_slots: usize,
    /// Dropped slots whose class is combat (must stay zero).
    pub dropped_combat: usize,
    /// Dropped slots whose class is pose.
    pub dropped_pose: usize,
    /// Dropped slots whose class is idle (the crowd shed first).
    pub dropped_idle: usize,
    /// State-delta packet bytes on the flood tape.
    pub flood_delta_bytes: usize,
    /// State-delta packet bytes on the quiet tape.
    pub quiet_delta_bytes: usize,
    /// FNV-1a over every charted wire packet, so two draws compare a hash.
    pub digest: u64,
}

impl std::fmt::Display for FloodChart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in self.rows {
            writeln!(
                f,
                "FLOOD kind={} channel={:?} prio={} hz={} raw={}B packed={}B save={}permille {}B/s -> {}B/s",
                row.kind,
                row.channel,
                row.priority,
                row.hz,
                row.raw_bytes,
                row.packed_bytes,
                row.saving_permille(),
                row.raw_bytes_per_sec(),
                row.packed_bytes_per_sec(),
            )?;
        }
        writeln!(
            f,
            "FLOOD shed kept={} dropped={} combat={} pose={} idle={}",
            self.kept_slots,
            self.dropped_slots,
            self.dropped_combat,
            self.dropped_pose,
            self.dropped_idle,
        )?;
        write!(
            f,
            "FLOOD delta flood={}B quiet={}B digest={:016x}",
            self.flood_delta_bytes, self.quiet_delta_bytes, self.digest,
        )
    }
}

/// Baseline of the flood: 16 fighters at rest, spread by slot.
fn flood_baseline() -> [FighterView; FLOOD_VIEWS] {
    let mut base = [FighterView {
        slot: 0,
        x_mm: 0,
        y_mm: 0,
        z_mm: 0,
        yaw_ddeg: 0,
        hp: 0,
        stamina: 0,
    }; FLOOD_VIEWS];
    for (slot, view) in (0u8..).zip(base.iter_mut()) {
        *view = FighterView {
            slot,
            x_mm: 1_000 + i32::from(slot) * 500,
            y_mm: 0,
            z_mm: -2_000,
            yaw_ddeg: 100,
            hp: 1_000,
            stamina: 100,
        };
    }
    base
}

/// The flood tape beside its quiet twin: eight slots fight, four move, four
/// hold still; the quiet tape moves only the viewer, by one millimetre.
fn flood_tape(
    base: &[FighterView; FLOOD_VIEWS],
) -> ([FighterView; FLOOD_VIEWS], [FighterView; FLOOD_VIEWS]) {
    let mut flood = *base;
    let mut quiet = *base;
    for (slot, view) in (0u8..).zip(flood.iter_mut()) {
        if slot < FLOOD_COMBAT_SLOTS {
            view.x_mm += 500;
            view.z_mm -= 250;
            view.yaw_ddeg += 90;
            view.hp = view.hp.saturating_sub(120);
            view.stamina = view.stamina.saturating_sub(2);
        } else if slot < FLOOD_MOVING_SLOTS {
            view.x_mm += 250;
            view.z_mm -= 125;
        }
    }
    quiet[0].x_mm += 1;
    (flood, quiet)
}

/// The flood's held input tape: 16 fighters on one tick, a held human wish in
/// slot 0, a second human wish in slot 1 and fourteen held bot wishes, every
/// slot holding a combat button.
fn flood_inputs() -> [FighterInput; FLOOD_VIEWS] {
    let mut inputs = [FighterInput {
        tick: FLOOD_TICK,
        fighter: FighterId(0),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons::NONE,
    }; FLOOD_VIEWS];
    for (slot, input) in (0u8..).zip(inputs.iter_mut()) {
        *input = match slot {
            0 => FighterInput {
                tick: FLOOD_TICK,
                fighter: FighterId(0),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            },
            1 => FighterInput {
                tick: FLOOD_TICK,
                fighter: FighterId(1),
                mx: -60,
                mz: 40,
                facing: -900,
                buttons: Buttons(BLOCK),
            },
            _ => FighterInput {
                tick: FLOOD_TICK,
                fighter: FighterId(slot),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            },
        };
    }
    inputs
}

/// Quantized full snapshot of one charted tape.
fn flood_snap_of(tick: u32, views: &[FighterView; FLOOD_VIEWS]) -> FullSnap {
    FullSnap {
        tick,
        fighters: views
            .iter()
            .map(|view| FighterSnap {
                slot: view.slot,
                x_mm: view.x_mm,
                y_mm: view.y_mm,
                z_mm: view.z_mm,
                yaw_ddeg: view.yaw_ddeg,
                hp: view.hp,
                stamina: view.stamina,
            })
            .collect(),
    }
}

/// State-delta packet bytes for `now` against `base` through the session
/// codec, read here and never reimplemented.
fn state_delta_bytes(base: &FullSnap, now: &FullSnap) -> Result<Vec<u8>, NetError> {
    let delta = encode_delta(now, base)?;
    encode_state_delta(
        Checkpoint {
            tick: now.tick,
            hash: FLOOD_CHECKPOINT_HASH,
        },
        &delta,
    )
}

/// The MP16 shed on the chart flood under the chart's own budget: the viewer
/// plus every changed non-idle slot, so idle crowd is exactly what drops.
/// Returns `[kept, dropped, combat, pose, idle]`.
fn flood_shed(
    base: &[FighterView; FLOOD_VIEWS],
    now: &[FighterView; FLOOD_VIEWS],
) -> Result<[usize; 5], ScaleError> {
    let mut budget = DELTA_HEADER_LEN;
    for (index, view) in now.iter().enumerate() {
        if priority_of(*view, base[index], 0) != PRIORITY_IDLE {
            budget = budget.saturating_add(delta_cost_bytes(*view, base[index]));
        }
    }
    let mut out = [0u8; FLOOD_VIEWS];
    let kept = select_priority(now, base, 0, FLOOD_TICK, FLOOD_BASE_TICK, budget, &mut out)?;
    let mut tally = [kept, 0, 0, 0, 0];
    for (index, view) in now.iter().enumerate() {
        if out[..kept].contains(&view.slot) {
            continue;
        }
        tally[1] += 1;
        match priority_of(*view, base[index], 0) {
            PRIORITY_COMBAT => tally[2] += 1,
            PRIORITY_IDLE => tally[4] += 1,
            _ => tally[3] += 1,
        }
    }
    Ok(tally)
}

/// Draw the MP18 flood chart: the MP8 table plus the MP9 codec under one
/// 16-fighter flood, one chart deciding both rows.
///
/// # Errors
///
/// [`NetError`] only if the fixed fixture stops validating (a code change, not
/// an input): the chart never draws a partial row.
pub fn flood_chart() -> Result<FloodChart, NetError> {
    let base = flood_baseline();
    let (flood, quiet) = flood_tape(&base);
    let inputs = flood_inputs();
    let raw = encode_input(FLOOD_TICK, &inputs)?;
    let packed = encode_input_compressed(FLOOD_TICK, &inputs)?;
    let base_snap = flood_snap_of(FLOOD_BASE_TICK, &base);
    let now_snap = flood_snap_of(FLOOD_TICK, &flood);
    let quiet_snap = flood_snap_of(FLOOD_TICK, &quiet);
    let flood_delta = state_delta_bytes(&base_snap, &now_snap)?;
    let quiet_delta = state_delta_bytes(&base_snap, &quiet_snap)?;
    let full = encode_snapshot(&now_snap)?;
    let hz = snapshot_hz(FLOOD_SLOTS).map_err(|_| NetError::BadState)?;
    let shed = flood_shed(&base, &flood).map_err(|_| NetError::BadState)?;
    let mut rows = [FloodRow {
        kind: 0,
        channel: Channel::Unreliable,
        priority: u8::MAX,
        hz: 0,
        raw_bytes: 0,
        packed_bytes: 0,
    }; KIND_CHANNEL.len()];
    for (row, (kind, channel, priority)) in rows.iter_mut().zip(KIND_CHANNEL) {
        // Kinds this chart does not encode draw zeroes; the chart test
        // asserts every table row carries bytes, so table growth fails there.
        let (rate, raw_bytes, packed_bytes) = match kind {
            KIND_INPUT => (TICK_HZ, raw.len(), packed.len()),
            KIND_STATE => (hz, flood_delta.len(), flood_delta.len()),
            KIND_SNAPSHOT => (hz, full.len(), full.len()),
            _ => (0, 0, 0),
        };
        *row = FloodRow {
            kind,
            channel,
            priority,
            hz: rate,
            raw_bytes,
            packed_bytes,
        };
    }
    let mut digest = Fnv64::new();
    for bytes in [&raw, &packed, &flood_delta, &quiet_delta, &full] {
        digest.write(bytes);
    }
    Ok(FloodChart {
        rows,
        kept_slots: shed[0],
        dropped_slots: shed[1],
        dropped_combat: shed[2],
        dropped_pose: shed[3],
        dropped_idle: shed[4],
        flood_delta_bytes: flood_delta.len(),
        quiet_delta_bytes: quiet_delta.len(),
        digest: digest.finish(),
    })
}

/// Allocation-free proof hook: the hot selection path funnels through
/// [`select_priority`], which takes slices plus a caller buffer and owns no
/// `Vec`, `String`, `Box` or `BTreeMap`. This map fingerprints one scripted
/// selection deterministically without touching that path, so a counting
/// allocator watching selection sees zero heap traffic from it.
#[must_use]
pub fn selection_fingerprint(current: &[FighterView], baseline: &[FighterView], viewer: u8) -> u64 {
    let mut order: BTreeMap<(u8, u8), u8> = BTreeMap::new();
    for (index, view) in current.iter().enumerate() {
        order.insert(
            (priority_of(*view, baseline[index], viewer), view.slot),
            view.slot,
        );
    }
    let mut hash: u64 = 1_464_717_047_381_303_168;
    for ((prio, _), slot) in order {
        hash ^= u64::from(prio) << 32 | u64::from(slot);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

/// Slots with human-shaped wishes (the first two; the rest AI lattice).
pub const HUMAN_SLOTS: u8 = 2;
/// Shortest reaction dwell in ticks (5 ticks are about 80 ms at 60 Hz).
pub const HUMAN_DWELL_MIN: u32 = 5;
/// Dwell span in ticks (5..15 ticks cover about 80..250 ms reactions).
pub const HUMAN_DWELL_SPAN: u64 = 11;
/// Canonical human authoring seed (its own lane seed, not the soak seed).
pub const HUMAN_BOUT_SEED: u64 = 11;

/// Author one held wish for `(seed, slot, epoch)`: steady intent in
/// `[-1024, 1024]` per axis, changing only at epoch turns (a reaction dwell,
/// not a per-tick lattice jump). Plain `fn` so the residues convert through
/// `TryFrom` instead of `as`.
pub(crate) fn human_wish(seed: u64, slot: u8, epoch: u32) -> (i32, i32) {
    let lane = (seed ^ 0x4855_4D41_4E57_4953)
        .wrapping_add(u64::from(slot).wrapping_mul(0x9E37_79B1_7F4A_7C15))
        .wrapping_add(u64::from(epoch).wrapping_mul(0xC2B2_AE35_0D29_C127));
    let dx = i32::try_from(engine_core::mix64(lane) % 2_049).unwrap_or(0) - 1_024;
    let dz = i32::try_from(engine_core::mix64(lane ^ 0x4252_5354_4C4F_5353) % 2_049).unwrap_or(0)
        - 1_024;
    (dx, dz)
}

/// Reaction dwell for one wish epoch: 5..15 ticks (about 80..250 ms).
pub(crate) fn human_dwell(seed: u64, slot: u8, epoch: u32) -> u32 {
    let lane = (seed ^ 0x4457_454C_4C30_3038)
        .wrapping_add(u64::from(slot).wrapping_mul(0x94D0_49BB_1331_11EB))
        .wrapping_add(u64::from(epoch).wrapping_mul(0xC2B2_AE35_0D29_C127));
    HUMAN_DWELL_MIN
        .saturating_add(u32::try_from(engine_core::mix64(lane) % HUMAN_DWELL_SPAN).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::MASK_HP;
    use crate::interest::{
        entity_room, room_mask, ConnectionInterest, InterestCfg, InterestEntity, InterestError,
        MAX_INTEREST_ENTITIES, ON9_CAP_KBPS_X100, SCOPE_ROOMS, SIEGE_BUDGET_BYTES,
        SNAPSHOT_EVERY_TICKS, STARVE_TICKS,
    };
    use crate::netcode::kind_channel;
    use crate::replicate::{
        decode_snapshot_packet, encode_snapshot_packet, EntityId, ReplicationClient,
        ReplicationServer, Transform, Vitals, MASK_VITALS, REPLICATE_VERSION_OFFSET,
    };
    use crate::session::{decode_input, NetError};
    use crate::tick_budget::{check_p99, TickQuantiles, BUDGET_P99_MS};
    #[expect(
        clippy::disallowed_types,
        reason = "wall-clock perf sample, never hashed."
    )]
    use std::time::Instant;

    fn view(slot: u8) -> FighterView {
        FighterView {
            slot,
            x_mm: 1_000 + i32::from(slot) * 500,
            y_mm: 0,
            z_mm: -2_000,
            yaw_ddeg: 100,
            hp: 1_000,
            stamina: 100,
        }
    }

    fn lineup(n: u8) -> Vec<FighterView> {
        (0..n).map(view).collect()
    }

    #[test]
    fn kbps_chart_rows_stay_under_target() {
        // Fixture asserts the posted chart: full snapshots at the chart rate
        // must stream under 16 / 64 / 96 kbps per client.
        let chart = bandwidth_chart();
        assert_eq!(
            chart[0],
            ChartRow {
                fighters: 2,
                hz: 20,
                snapshot_bytes: 47,
                bytes_per_sec: 940,
                kbps_target: 16,
            }
        );
        assert_eq!(
            chart[1],
            ChartRow {
                fighters: 8,
                hz: 20,
                snapshot_bytes: 161,
                bytes_per_sec: 3_220,
                kbps_target: 64,
            }
        );
        assert_eq!(
            chart[2],
            ChartRow {
                fighters: 16,
                hz: 15,
                snapshot_bytes: 313,
                bytes_per_sec: 4_695,
                kbps_target: 96,
            }
        );
        assert_eq!(
            chart[3],
            ChartRow {
                fighters: 32,
                hz: 15,
                snapshot_bytes: 617,
                bytes_per_sec: 9_255,
                kbps_target: 96,
            }
        );
        for row in chart {
            within_cap(row.fighters, row.snapshot_bytes).expect("chart row must fit");
            assert!(
                kbps(row.snapshot_bytes, row.hz) < row.kbps_target as f64,
                "fighters={} carries {:.2} kbps over target {}",
                row.fighters,
                kbps(row.snapshot_bytes, row.hz),
                row.kbps_target
            );
        }
    }

    #[test]
    fn flood_chart_rows_follow_the_mp8_table_and_keep_combat_first() {
        let chart = flood_chart().expect("the fixed flood charts");
        println!("{chart}");
        assert_eq!(
            chart.rows.len(),
            KIND_CHANNEL.len(),
            "one row per table kind"
        );
        for (row, (kind, channel, priority)) in chart.rows.iter().zip(KIND_CHANNEL) {
            assert_eq!(row.kind, kind, "rows follow the table order");
            assert_eq!(row.channel, channel);
            assert_eq!(row.priority, priority);
            assert_eq!(
                kind_channel(kind),
                Some((channel, priority)),
                "kind {kind} must read the same table"
            );
            assert!(
                row.raw_bytes > 0 && row.packed_bytes > 0,
                "kind {kind} draws no bytes: {chart}"
            );
        }
        let priorities: Vec<u8> = chart.rows.iter().map(|row| row.priority).collect();
        assert!(
            priorities.windows(2).all(|pair| pair[0] < pair[1]),
            "combat inputs must outrank state and snapshot bulk: {priorities:?}"
        );
        assert_eq!(chart.dropped_combat, 0, "combat never starves: {chart}");
        // Combat state really carries: every wounded slot names hp in the
        // flood's state delta, and every slot's combat button survives both
        // input shapes byte-exact.
        let base = flood_baseline();
        let (flood, _) = flood_tape(&base);
        let delta = encode_delta(
            &flood_snap_of(FLOOD_TICK, &flood),
            &flood_snap_of(FLOOD_BASE_TICK, &base),
        )
        .expect("the flood deltas");
        for slot in 0..FLOOD_COMBAT_SLOTS {
            let change = delta
                .changed_slots
                .iter()
                .find(|(at, _)| *at == slot)
                .expect("wounded slot is named");
            assert_ne!(change.1.mask & MASK_HP, 0, "hp starved for slot {slot}");
        }
        let inputs = flood_inputs();
        let raw = encode_input(FLOOD_TICK, &inputs).expect("raw input packet");
        let packed = encode_input_compressed(FLOOD_TICK, &inputs).expect("packed input packet");
        assert_eq!(decode_input(&raw).expect("raw decodes").inputs, inputs);
        assert_eq!(
            decode_input(&packed).expect("packed decodes").inputs,
            inputs
        );
        for input in inputs {
            assert_ne!(
                input.buttons.0 & engine_input::fighter::KNOWN_BUTTONS,
                0,
                "slot {} holds no combat button",
                input.fighter.get()
            );
        }
    }

    #[test]
    fn flood_chart_sheds_idle_crowd_before_any_combat_slot() {
        let chart = flood_chart().expect("the fixed flood charts");
        let crowd = usize::from(FLOOD_SLOTS - FLOOD_MOVING_SLOTS);
        assert_eq!(chart.dropped_slots, crowd, "the idle crowd sheds: {chart}");
        assert_eq!(chart.dropped_idle, crowd);
        assert_eq!((chart.dropped_combat, chart.dropped_pose), (0, 0));
        assert_eq!(
            chart.kept_slots,
            usize::from(FLOOD_MOVING_SLOTS),
            "every changed slot ships: {chart}"
        );
    }

    #[test]
    fn flood_chart_deltas_are_smaller_on_quiet_tapes() {
        let chart = flood_chart().expect("the fixed flood charts");
        println!("{chart}");
        assert!(
            chart.quiet_delta_bytes < chart.flood_delta_bytes,
            "quiet tape must delta smaller: quiet {} vs flood {}",
            chart.quiet_delta_bytes,
            chart.flood_delta_bytes
        );
        assert!(
            chart.quiet_delta_bytes * 4 < chart.flood_delta_bytes,
            "quiet delta {} must stay under a quarter of the flood {}",
            chart.quiet_delta_bytes,
            chart.flood_delta_bytes
        );
        let full = chart
            .rows
            .iter()
            .find(|row| row.kind == KIND_SNAPSHOT)
            .expect("snapshot row")
            .raw_bytes;
        assert!(
            chart.flood_delta_bytes < full,
            "the flood delta {} must beat the full snapshot {full}",
            chart.flood_delta_bytes
        );
    }

    #[test]
    fn flood_chart_codec_saves_twenty_percent_at_sixteen_fighters() {
        let chart = flood_chart().expect("the fixed flood charts");
        let input = chart
            .rows
            .iter()
            .find(|row| row.kind == KIND_INPUT)
            .expect("input row");
        println!("{chart}");
        // The kill bar: >=20% saving at 16 fighters, or the row dies here
        // with the numbers on the failure line.
        assert!(
            input.packed_bytes * 5 <= input.raw_bytes * 4,
            "MP18 kill bar: need >=20% saving, raw={} packed={} ({} permille)",
            input.raw_bytes,
            input.packed_bytes,
            input.saving_permille()
        );
        assert!(input.saving_permille() >= 200);
        assert!(input.packed_bytes_per_sec() < input.raw_bytes_per_sec());
    }

    #[test]
    fn flood_chart_replays_identically_x2() {
        assert_eq!(
            FLOOD_VIEWS,
            usize::from(FLOOD_SLOTS),
            "the fixture count and its array length must agree"
        );
        let first = flood_chart().expect("the fixed flood charts");
        let second = flood_chart().expect("the fixed flood charts");
        assert_eq!(first.digest, second.digest, "same flood, same bytes");
        assert_ne!(first.digest, 0, "the chart must hash real bytes");
        assert_eq!(first, second, "same flood draws the same chart");
        println!("{first}");
    }

    #[test]
    fn full_snapshots_fit_both_budgets() {
        assert_eq!(full_wire_len(16), 313);
        assert_eq!(full_wire_len(32), 617);
        assert!(full_wire_len(32) <= MAX_DATAGRAM_BYTES);
        assert_eq!(cap_bytes_per_snapshot(16).expect("cap"), 800);
        assert_eq!(cap_bytes_per_snapshot(32).expect("cap"), 800);
        assert!(full_wire_len(16) <= cap_bytes_per_snapshot(16).expect("cap"));
        assert!(full_wire_len(32) <= cap_bytes_per_snapshot(32).expect("cap"));
    }

    #[test]
    fn thirty_two_slot_gates_follow_the_canonical_cap() {
        // One value, every gate: the chart rates and caps accept 32 slots.
        assert_eq!(snapshot_hz(32), Ok(15));
        assert_eq!(cap_bytes_per_sec(32), Ok(12_000));
        assert_eq!(cap_bytes_per_snapshot(32), Ok(800));
        within_cap(32, full_wire_len(32)).expect("32 slots fit the 1200-byte datagram");
        assert_eq!(
            within_cap(32, 801),
            Err(ScaleError::OverBudget {
                need: 801,
                budget: 800,
            })
        );
        assert_eq!(
            snapshot_hz(33),
            Err(ScaleError::BadFighterCount { got: 33 })
        );
        assert_eq!(
            cap_bytes_per_sec(33),
            Err(ScaleError::BadFighterCount { got: 33 })
        );
    }

    #[test]
    fn select_priority_orders_thirty_two_slots() {
        // A 32-length output is legal and every combat delta ships.
        let base = lineup(32);
        let mut now = base.clone();
        for view in now.iter_mut().skip(1) {
            view.hp = view.hp.saturating_sub(120);
        }
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];
        let a = select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut first)
            .expect("select 32");
        let b = select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut second)
            .expect("select 32");
        assert_eq!((a, &first[..a]), (b, &second[..b]), "twice equal");
        assert_eq!(a, 32, "every slot ships");
        let all: Vec<u8> = (0..32u8).collect();
        assert_eq!(&first[..a], all.as_slice());
    }

    #[test]
    fn combat_inputs_never_starve_under_snapshot_flood() {
        // 32-fighter shape stays within 1200 bytes, never fragments.
        let full = full_wire_len(32);
        assert_eq!(full, 617);
        assert!(full <= MAX_DATAGRAM_BYTES);
        within_cap(32, full).expect("32 must fit");
        assert!(full <= crate::session::IOQ3_FRAGMENT_SIZE);
        // Flood: every non-viewer slot combat-changed; budget is the cap.
        let base = lineup(32);
        let mut now = base.clone();
        for view in now.iter_mut().skip(1) {
            view.hp = view.hp.saturating_sub(120);
        }
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];
        let a =
            select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut first).expect("select");
        let b =
            select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut second).expect("select");
        assert_eq!((a, &first[..a]), (b, &second[..b]), "twice equal");
        assert_eq!(a, 32, "zero combat starvation");
        let all: Vec<u8> = (0..32u8).collect();
        assert_eq!(&first[..a], all.as_slice());
        // The same flood at the chart budget (the MP2 800-byte snapshot cap),
        // not only the 1200-byte datagram: every combat slot still ships.
        let chart_budget = cap_bytes_per_snapshot(32).expect("32 chart budget");
        assert_eq!(chart_budget, 800);
        let mut capped = [0u8; 32];
        let c = select_priority(&now, &base, 0, 9, 8, chart_budget, &mut capped)
            .expect("select 32 at the chart budget");
        assert_eq!(c, 32, "zero combat starvation at the 800-byte cap");
        // Worst case: all 32 slots change every field, so this is the largest
        // 32-slot delta the shed can ever offer; it must still fit the cap.
        let mut worst = base.clone();
        for view in worst.iter_mut() {
            view.x_mm += 1;
            view.y_mm += 1;
            view.z_mm += 1;
            view.yaw_ddeg += 1;
            view.hp = view.hp.saturating_add(1);
            view.stamina = view.stamina.saturating_add(1);
        }
        let mut worst_out = [0u8; 32];
        let w = select_priority(&worst, &base, 0, 9, 8, chart_budget, &mut worst_out)
            .expect("select worst-case 32");
        assert_eq!(w, 32, "worst-case 32-slot delta must fit the 800-byte cap");
        let worst_bytes = DELTA_HEADER_LEN
            + worst
                .iter()
                .zip(base.iter())
                .map(|(now, base)| delta_cost_bytes(*now, *base))
                .sum::<usize>();
        assert!(
            worst_bytes <= chart_budget,
            "worst-case delta {worst_bytes} B over the {chart_budget} B cap"
        );
        println!(
            "SCALE flood32 combat_delta={} B worst_delta={} B chart_budget={} B headroom={} B",
            DELTA_HEADER_LEN
                + now
                    .iter()
                    .zip(base.iter())
                    .map(|(now, base)| delta_cost_bytes(*now, *base))
                    .sum::<usize>(),
            worst_bytes,
            chart_budget,
            chart_budget - worst_bytes,
        );
        // Table order: combat inputs outrank state outranks snapshot bulk.
        let (_, ip) = crate::netcode::kind_channel(crate::session::KIND_INPUT).expect("input");
        let (_, sp) = crate::netcode::kind_channel(crate::session::KIND_STATE).expect("state");
        let (_, bp) = crate::netcode::kind_channel(crate::session::KIND_SNAPSHOT).expect("bulk");
        assert!(ip < sp && sp < bp, "inputs outrank state outranks bulk");
    }

    #[test]
    fn oversize_snapshot_refuses_both_gates() {
        assert_eq!(
            within_cap(2, MAX_DATAGRAM_BYTES + 1),
            Err(ScaleError::OverBudget {
                need: MAX_DATAGRAM_BYTES + 1,
                budget: MAX_DATAGRAM_BYTES,
            })
        );
        // 1200 bytes at 20 Hz is 24_000 B/s over the 2-fighter 2_000 cap.
        assert!(within_cap(2, MAX_DATAGRAM_BYTES).is_err());
        assert_eq!(
            within_cap(1, 47),
            Err(ScaleError::BadFighterCount { got: 1 })
        );
        assert_eq!(
            within_cap(33, 47),
            Err(ScaleError::BadFighterCount { got: 33 })
        );
    }

    #[test]
    fn priority_drops_idle_before_pose_before_combat() {
        let base = lineup(4);
        let mut now = base.clone();
        now[1].hp = 760;
        now[2].x_mm += 500;
        // Slot 3 is idle: identical to baseline.
        let mut out = [0xFFu8; 16];
        // Viewer (slot 0) plus the combat change plus the pose change fit;
        // the idle slot is the first dropped.
        let combat = delta_cost_bytes(now[1], base[1]);
        let pose = delta_cost_bytes(now[2], base[2]);
        let viewer = delta_cost_bytes(now[0], base[0]);
        let budget = DELTA_HEADER_LEN + viewer + combat + pose;
        let n = select_priority(&now, &base, 0, 7, 6, budget, &mut out).expect("select");
        assert_eq!(n, 3);
        assert_eq!(&out[..n], &[0, 1, 2]);
        // One byte less drops the pose slot, never the viewer or combat.
        let n = select_priority(&now, &base, 0, 7, 6, budget - 1, &mut out).expect("select");
        assert_eq!(&out[..n], &[0, 1]);
    }

    #[test]
    fn viewer_always_ships_or_budget_refuses() {
        let base = lineup(2);
        let now = base.clone();
        let mut out = [0xFFu8; 16];
        let viewer = delta_cost_bytes(now[1], base[1]);
        let exact = DELTA_HEADER_LEN + viewer;
        let n = select_priority(&now, &base, 1, 3, 3, exact, &mut out).expect("select");
        assert_eq!(&out[..n], &[1]);
        assert_eq!(
            select_priority(&now, &base, 1, 3, 3, exact - 1, &mut out),
            Err(ScaleError::OverBudget {
                need: exact,
                budget: exact - 1,
            })
        );
    }

    #[test]
    fn selection_is_deterministic_and_sorted() {
        let base = lineup(8);
        let mut now = base.clone();
        now[5].hp -= 120;
        now[6].z_mm += 1;
        now[7].yaw_ddeg -= 40;
        let mut first = [0u8; 16];
        let mut second = [0u8; 16];
        let a = select_priority(&now, &base, 0, 9, 8, 37, &mut first).expect("select");
        let b = select_priority(&now, &base, 0, 9, 8, 37, &mut second).expect("select");
        assert_eq!((a, &first[..a]), (b, &second[..b]));
        assert_eq!(&first[..a], &[0, 5, 6, 7]);
        assert_eq!(
            selection_fingerprint(&now, &base, 0),
            selection_fingerprint(&now, &base, 0)
        );
    }

    #[test]
    fn selection_rejects_bad_shapes_loudly() {
        let base = lineup(4);
        let now = base.clone();
        let mut out = [0u8; 16];
        assert_eq!(
            select_priority(&[], &[], 0, 1, 0, 1_000, &mut out),
            Err(ScaleError::EmptySnapshot)
        );
        let mut shuffled = base.clone();
        shuffled.swap(0, 1);
        assert_eq!(
            select_priority(&shuffled, &base, 0, 1, 0, 1_000, &mut out),
            Err(ScaleError::UnsortedSlots)
        );
        let mut dropped = base.clone();
        dropped.pop();
        assert_eq!(
            select_priority(&dropped, &base, 0, 1, 0, 1_000, &mut out),
            Err(ScaleError::SlotMismatch)
        );
        assert_eq!(
            select_priority(&now, &base, 0, 5, 6, 1_000, &mut out),
            Err(ScaleError::TickRegression {
                tick: 5,
                base_tick: 6,
            })
        );
        assert_eq!(
            select_priority(&now, &base, 9, 6, 6, 1_000, &mut out),
            Err(ScaleError::UnknownViewer { viewer: 9 })
        );
        let mut tiny = [0u8; 2];
        assert_eq!(
            select_priority(&now, &base, 0, 6, 6, 1_000, &mut tiny),
            Err(ScaleError::OutTooSmall { need: 4, have: 2 })
        );
        let solo = lineup(1);
        assert_eq!(
            select_priority(&solo, &solo, 0, 1, 0, 1_000, &mut out),
            Err(ScaleError::BadFighterCount { got: 1 })
        );
    }

    #[test]
    fn delta_of_one_slot_beats_full_snapshot() {
        let base = lineup(8);
        let mut now = base.clone();
        now[3].hp -= 120;
        let full = full_wire_len(8);
        let one = DELTA_HEADER_LEN + delta_cost_bytes(now[3], base[3]);
        assert!(one < full, "delta {one} must beat full {full}");
        let mut out = [0u8; 16];
        let n = select_priority(
            &now,
            &base,
            0,
            4,
            3,
            one + delta_cost_bytes(now[0], base[0]),
            &mut out,
        )
        .expect("select");
        assert!(out[..n].contains(&3));
    }

    #[test]
    fn soak_32_bot_x600_converges_with_matching_hashes() {
        let a = run_soak(2040, 32, SOAK_TICKS).expect("soak");
        let b = run_soak(2040, 32, SOAK_TICKS).expect("soak");
        assert_eq!(a, b, "same seed must converge to the same report");
        assert_ne!(a.hash, 0, "soak must actually simulate");
        assert_eq!((a.fighters, a.ticks), (32, SOAK_TICKS));
        assert_eq!(a.snapshot_bytes, 617);
        assert_eq!(a.snapshot_hz, 15);
        assert_eq!(a.bytes_per_sec, 9_255);
        within_cap(a.fighters, a.snapshot_bytes).expect("soak must fit the cap");
        println!("{a}");
    }

    #[test]
    fn soak_small_fights_converge_and_seeds_diverge() {
        for fighters in [2u8, 8u8] {
            let a = soak_hash(77, fighters, 600).expect("soak");
            let b = soak_hash(77, fighters, 600).expect("soak");
            assert_eq!(a, b, "fighters={fighters} must converge");
            assert_ne!(a, 0);
        }
        let a = soak_hash(1, 32, 600).expect("soak");
        let b = soak_hash(2, 32, 600).expect("soak");
        assert_ne!(a, b, "different seeds must diverge");
    }

    #[test]
    fn soak_rejects_bad_counts_and_absurd_ticks() {
        assert_eq!(
            soak_hash(1, 1, 10),
            Err(ScaleError::BadFighterCount { got: 1 })
        );
        assert_eq!(
            soak_hash(1, 33, 10),
            Err(ScaleError::BadFighterCount { got: 33 })
        );
        assert!(soak_hash(1, 32, SOAK_TICKS_MAX + 1).is_err());
    }

    #[test]
    fn error_strings_name_the_fault() {
        assert!(format!("{}", ScaleError::EmptySnapshot).contains("no fighters"));
        assert!(format!("{}", ScaleError::UnsortedSlots).contains("sorted"));
        assert!(format!("{}", ScaleError::SlotMismatch).contains("different slots"));
        assert!(format!(
            "{}",
            ScaleError::TickRegression {
                tick: 1,
                base_tick: 2
            }
        )
        .contains('1'));
    }

    #[test]
    fn human_authoring_holds_reaction_scale() {
        // Dwells sit on the reaction scale (about 80..250 ms) and wishes stay
        // inside the movement box; epochs actually turn wishes over.
        for slot in 0..HUMAN_SLOTS {
            for epoch in 0..32 {
                let dwell = human_dwell(HUMAN_BOUT_SEED, slot, epoch);
                assert!((5..=15).contains(&dwell), "epoch dwell {dwell} off scale");
                let (dx, dz) = human_wish(HUMAN_BOUT_SEED, slot, epoch);
                assert!((-1_024..=1_024).contains(&dx), "wish dx {dx} out of box");
                assert!((-1_024..=1_024).contains(&dz), "wish dz {dz} out of box");
            }
        }
        assert_ne!(
            human_wish(HUMAN_BOUT_SEED, 0, 0),
            human_wish(HUMAN_BOUT_SEED, 0, 1),
            "epochs must turn wishes over"
        );
    }

    #[test]
    fn adaptive_delay_covers_the_round_trip_minus_the_window() {
        // MP24: delay is round-trip worst plus one tick of authoring slack
        // minus the 8-tick window. LAN (hold 3) stays 0 so its digests never
        // move; WAN (hold 10) delays 11 so every live resim lands inside 8.
        assert_eq!(adaptive_input_delay_for_hold(3), 0);
        assert_eq!(adaptive_input_delay_for_hold(10), 11);
        assert_eq!(
            adaptive_input_delay_for_hold(10),
            9 + 10 - RESIM_CAP_TICKS,
            "worst up plus hold minus the window"
        );
        assert!(adaptive_input_delay_for_hold(3) <= RESIM_CAP_TICKS + 3);
    }

    #[test]
    fn mp14_window_and_delay_sized_by_posted_mp6() {
        // MP14: the 8-tick window and both delays are measured values sized
        // by the posted MP6 live distributions, referenced by digest. Fresh
        // hardware data moves the digest or the maxima and fails here.
        let mut posting = engine_core::Fnv64::new();
        posting.write_u32(MP6_POSTED_LAN_ROLLBACK_MAX);
        posting.write_u32(MP6_POSTED_WAN_ROLLBACK_MAX);
        posting.write_u32(MP6_POSTED_WAN_DELAY_MAX);
        posting.write_u32(MP6_POSTED_DEEPER_THAN_WINDOW);
        posting.write_u32(MP6_POSTED_LAN_HOLD);
        posting.write_u32(MP6_POSTED_WAN_HOLD);
        assert_eq!(
            posting.finish(),
            MP6_POSTED_DISTRIBUTIONS_DIGEST,
            "posted MP6 distributions digest must match target/mp6/distributions.json"
        );
        assert_eq!(
            measured_resim_window(MP6_POSTED_LAN_ROLLBACK_MAX, MP6_POSTED_WAN_ROLLBACK_MAX),
            RESIM_CAP_TICKS,
            "window is the deeper posted live rollback max"
        );
        assert_eq!(
            MP6_POSTED_DEEPER_THAN_WINDOW, 0,
            "no posted live state sat deeper than the window"
        );
        assert_eq!(adaptive_input_delay_for_hold(MP6_POSTED_LAN_HOLD), 0);
        assert_eq!(adaptive_input_delay_for_hold(MP6_POSTED_WAN_HOLD), 11);
        assert_eq!(
            adaptive_input_delay_for_hold(MP6_POSTED_WAN_HOLD),
            MP6_POSTED_WAN_DELAY_MAX + MP6_POSTED_WAN_HOLD - RESIM_CAP_TICKS,
            "WAN delay is posted worst plus hold minus the window"
        );
        const {
            assert!(
                MP6_POSTED_WAN_ROLLBACK_MAX > RESIM_CAP_TICKS - 1,
                "window minus one would not cover the posted WAN max"
            );
        };
    }

    /// The replicated entity id of one fighter slot.
    fn entity_of(view: &FighterView) -> EntityId {
        EntityId(u32::from(view.slot))
    }

    /// The replicated transform of one fighter: the ground plane, x and z.
    fn transform_of(view: &FighterView) -> Transform {
        Transform {
            x_mm: view.x_mm,
            y_mm: view.z_mm,
            yaw_ddeg: view.yaw_ddeg,
        }
    }

    /// The replicated vitals of one fighter.
    fn vitals_of(view: &FighterView) -> Vitals {
        Vitals {
            hp: view.hp,
            stamina: view.stamina,
        }
    }

    /// One 16-fighter flood run: the join message, the flood message and
    /// the world hash. Same fixture twice gives the same bytes and hash.
    fn replicate_flood_run() -> (Vec<u8>, Vec<u8>, u64) {
        let base = lineup(16);
        let mut now = base.clone();
        for (index, fighter) in now.iter_mut().enumerate() {
            // Everybody moves and everybody but the viewer fights.
            fighter.x_mm += 500 + i32::try_from(index).unwrap_or(0);
            fighter.z_mm -= 250;
            if index != 0 {
                fighter.hp = fighter.hp.saturating_sub(120);
            }
        }
        let mut world = ReplicationServer::new();
        for fighter in &base {
            world
                .spawn(
                    entity_of(fighter),
                    transform_of(fighter),
                    vitals_of(fighter),
                )
                .expect("spawn");
        }
        world.advance();
        // A late client joins with the whole world.
        let join = world.message_for(1).expect("join message");
        let join_bytes = encode_snapshot_packet(&join).expect("encode join");
        assert!(world.on_ack(1, join.tick).expect("ack"));
        // The flood lands on the authoritative world.
        for fighter in &now {
            world
                .set_transform(entity_of(fighter), transform_of(fighter))
                .expect("move");
            world
                .set_vitals(entity_of(fighter), vitals_of(fighter))
                .expect("vitals");
        }
        world.advance();
        let flood = world.message_for(1).expect("flood message");
        let flood_bytes = encode_snapshot_packet(&flood).expect("encode flood");
        (join_bytes, flood_bytes, world.world_hash())
    }

    #[test]
    fn replicate_deltas_ride_the_lan_snapshot_path_at_16_fighters() {
        let (join_bytes, flood_bytes, hash) = replicate_flood_run();
        // Hash x2: the same flood gives the same bytes and the same hash.
        let (join_again, flood_again, hash_again) = replicate_flood_run();
        assert_eq!(
            (join_bytes.clone(), flood_bytes.clone(), hash),
            (join_again, flood_again, hash_again),
            "the flood must replay exactly"
        );
        // The flood rides the LAN snapshot path inside the same budget.
        assert!(
            flood_bytes.len() <= MAX_DATAGRAM_BYTES,
            "replicate flood {} must fit the {} byte LAN budget",
            flood_bytes.len(),
            MAX_DATAGRAM_BYTES
        );
        assert!(
            join_bytes.len() <= MAX_DATAGRAM_BYTES,
            "replicate join {} must fit the {} byte LAN budget",
            join_bytes.len(),
            MAX_DATAGRAM_BYTES
        );
        // The version-refusing path decodes the flood back.
        let flood = decode_snapshot_packet(&flood_bytes).expect("decode flood");
        assert!(!flood.resync, "the flood rides the delta path");
        assert_eq!(flood.updates.len(), 16, "every fighter is named");
        // Zero combat starvation: every wounded fighter carries its vitals.
        for slot in 1..16u8 {
            let update = flood
                .updates
                .iter()
                .find(|update| update.entity == EntityId(u32::from(slot)))
                .expect("wounded fighter named");
            assert_ne!(
                update.mask & MASK_VITALS,
                0,
                "combat state starved for slot {slot}"
            );
        }
        // The mirror agrees with the authority after the flood.
        let mut client = ReplicationClient::new();
        let join = decode_snapshot_packet(&join_bytes).expect("decode join");
        client.apply(&join).expect("apply join");
        client.apply(&flood).expect("apply flood");
        assert_eq!(
            client.world_hash(),
            hash,
            "mirror agrees with the authority"
        );
        // A foreign replicate version refuses loudly and applies nothing.
        let mut foreign = flood_bytes.clone();
        foreign[REPLICATE_VERSION_OFFSET] = crate::replicate::REPLICATE_VERSION + 1;
        assert_eq!(
            decode_snapshot_packet(&foreign),
            Err(NetError::VersionMismatch {
                got: crate::replicate::REPLICATE_VERSION + 1
            })
        );
    }

    // ---- MP21: 64-player interest handoff spike -------------------------

    /// MP21 fighters: replicated entity ids, not fighter slots (cap 32).
    const MP21_FIGHTERS: u32 = 64;
    /// Legion stand-ins the row names beside the fighters.
    const MP21_LEGION: u32 = 1_000;
    /// Legion stand-ins that fit beside 64 fighters under the interest cap
    /// (1024 - 64). The row's literal 1_000 does not; the refusal test below
    /// records that, and widening the cap is an ON row, not a spike edit.
    const MP21_LEGION_FIT: u32 = MAX_INTEREST_ENTITIES as u32 - MP21_FIGHTERS;
    /// Entities in one spike tick: the interest cap, full.
    const MP21_ENTITIES: u32 = MP21_FIGHTERS + MP21_LEGION_FIT;
    /// Spike length: 600 ticks (10 s at 60 Hz), like the MP16b soak.
    const MP21_TICKS: u32 = 600;
    /// Half-side of a fighter's square orbit, in millimetres.
    const MP21_ORBIT_MM: i32 = 40_000;
    /// Fighter walk per tick along that orbit, in millimetres (2 m per tick,
    /// so one 320 m lap takes 160 ticks: rings hand off about every quarter).
    const MP21_ORBIT_STEP_MM: i32 = 2_000;
    /// Room cluster pitch: room `r` sits `r` x 800 m east of room 0.
    const MP21_ROOM_PITCH_MM: i32 = 800_000;
    /// World-step p99 bar for the spike: the P6 1,000-agent sim read.
    const MP21_STEP_P99_MS: f64 = 2.0;

    /// Where room `room` sits: its cluster centre on the x axis, in mm.
    fn mp21_room_x(room: u32) -> i32 {
        i32::try_from(room).expect("room fits") * MP21_ROOM_PITCH_MM
    }

    /// Fighter pose at `tick`: a square orbit around its room centre plus the
    /// facing of the side it walks along (yaw -1800 faces +x, -1125 +z,
    /// -225 -x, 675 -z). Integer only, so every tick replays bit-exactly.
    fn mp21_fighter_pose(id: u32, tick: u32) -> (i32, i32, i16) {
        let side = 2 * MP21_ORBIT_MM;
        let perimeter = u32::try_from(4 * side).expect("perimeter fits");
        let walk = u32::try_from(MP21_ORBIT_STEP_MM).expect("walk fits");
        let along = tick.wrapping_mul(walk).wrapping_add(id.wrapping_mul(9_000)) % perimeter;
        let side = u32::try_from(side).expect("side fits");
        let rem = i32::try_from(along % side).expect("rem fits");
        let o = MP21_ORBIT_MM;
        let (x, z, yaw) = match along / side {
            0 => (rem - o, -o, -1_800),
            1 => (o, rem - o, -1_125),
            2 => (o - rem, o, -225),
            _ => (-o, o - rem, 675),
        };
        (mp21_room_x(entity_room(id)) + x, z, yaw)
    }

    /// Legion position at `tick`: a 12 x 10 grid per room at 30 x 36 m pitch,
    /// drifting 4 m east or west every 7 ticks so ring membership really
    /// hands off during the run (the spike's handoff churn).
    fn mp21_legion_pos(id: u32, tick: u32) -> (i32, i32) {
        let local = (id - MP21_FIGHTERS) / SCOPE_ROOMS;
        let col = local % 12;
        let row = local / 12;
        let drift = if (tick / 7).wrapping_add(local).is_multiple_of(2) {
            4_000
        } else {
            -4_000
        };
        let x = i32::try_from(col).expect("col fits") * 30_000 - 165_000 + drift;
        let z = i32::try_from(row).expect("row fits") * 36_000 - 162_000;
        (mp21_room_x(entity_room(id)) + x, z)
    }

    /// MP21 world: every entity, the 64 scoped connections and one shared
    /// selection buffer. Positions and combat words are integers only.
    struct HandoffWorld {
        entities: Vec<InterestEntity>,
        conns: Vec<ConnectionInterest>,
        out: Vec<u32>,
    }

    impl HandoffWorld {
        /// Build the world at tick 0 and register that roster as every
        /// connection's delta baseline (match start). Each connection scopes
        /// one room, so about 128 of the 1024 entities are candidates and
        /// combat-dirty bodies still ship across rooms.
        fn new() -> Self {
            let entities: Vec<InterestEntity> = (0..MP21_ENTITIES)
                .map(|id| {
                    let (x_mm, z_mm) = if id < MP21_FIGHTERS {
                        let (x, z, _) = mp21_fighter_pose(id, 0);
                        (x, z)
                    } else {
                        mp21_legion_pos(id, 0)
                    };
                    InterestEntity {
                        id,
                        x_mm,
                        z_mm,
                        combat: 1_000,
                    }
                })
                .collect();
            let count = usize::try_from(MP21_ENTITIES).expect("entities fit");
            let mut conns = Vec::with_capacity(usize::try_from(MP21_FIGHTERS).expect("fits"));
            for viewer in 0..MP21_FIGHTERS {
                let mut conn =
                    ConnectionInterest::for_entities(viewer, count).expect("inside the cap");
                conn.set_rooms(room_mask(viewer % SCOPE_ROOMS));
                conn.register_baseline(&entities).expect("baseline");
                conns.push(conn);
            }
            Self {
                entities,
                conns,
                out: vec![0u32; count],
            }
        }

        /// Step to `tick`: the integer walk plus scripted fighter attrition
        /// (one hit every 23 ticks, staggered by id), so combat state is
        /// really dirty and never float.
        fn step(&mut self, tick: u32) {
            for entity in &mut self.entities {
                let id = entity.id;
                if id < MP21_FIGHTERS {
                    let (x_mm, z_mm, _) = mp21_fighter_pose(id, tick);
                    entity.x_mm = x_mm;
                    entity.z_mm = z_mm;
                    let hits = tick.wrapping_add(id) / 23;
                    entity.combat = 1_000u32.saturating_sub(hits * 3);
                } else {
                    let (x_mm, z_mm) = mp21_legion_pos(id, tick);
                    entity.x_mm = x_mm;
                    entity.z_mm = z_mm;
                }
            }
        }

        /// Fold one tick: every entity's state plus every selection receipt.
        fn digest(&self, tick: u32, receipts: &[(u32, u32, u32)]) -> u64 {
            let mut hash = Fnv64::new();
            hash.write_u32(tick);
            hash.write_len(self.entities.len());
            for entity in &self.entities {
                hash.write_u32(entity.id);
                hash.write_i32(entity.x_mm);
                hash.write_i32(entity.z_mm);
                hash.write_u32(entity.combat);
            }
            for (viewer, selected, bytes) in receipts {
                hash.write_u32(*viewer);
                hash.write_u32(*selected);
                hash.write_u32(*bytes);
            }
            hash.finish()
        }
    }

    /// One MP21 spike run: the evidence the row quotes.
    struct Mp21Report {
        /// Per-tick digests (two runs must match element by element).
        digests: Vec<u64>,
        /// Fold of every digest.
        hash: u64,
        /// Worst per-client rate over the run, kbps x100 (integer).
        kbps_x100: u64,
        /// Worst per-client bytes/s over the run.
        bytes_per_sec: u64,
        /// Worst snapshot bytes any client was handed.
        snapshot_bytes: usize,
        /// Worst ticks-since-sent on any in-scope entity.
        starve_max: u32,
        /// Worst ticks a changed combat body stayed unsent.
        combat_gap: u32,
        /// Snapshots one client received.
        snapshots_per_client: u64,
        /// Wall-clock p99 of a full essentials tick (step plus due selects).
        tick_p99_ms: f64,
        /// Wall-clock p99 of the world step alone.
        step_p99_ms: f64,
    }

    /// One MP21 spike arm: 600 ticks at 60 Hz over 1024 entities and 64
    /// scoped connections, each selecting every sixth tick on its own lane
    /// (`viewer % 6 == tick % 6`), so no tick pays all 64 selects.
    #[expect(
        clippy::disallowed_types,
        reason = "wall-clock perf sample, never hashed."
    )]
    #[expect(
        clippy::disallowed_methods,
        reason = "wall-clock perf sample, never hashed."
    )]
    fn mp21_run() -> Mp21Report {
        let cfg = InterestCfg::default_siege();
        let mut world = HandoffWorld::new();
        let ticks = usize::try_from(MP21_TICKS).expect("ticks fit");
        let mut digests = Vec::with_capacity(ticks);
        let mut tick_ms = Vec::with_capacity(ticks);
        let mut step_ms = Vec::with_capacity(ticks);
        let mut receipts: Vec<(u32, u32, u32)> =
            Vec::with_capacity(usize::try_from(MP21_FIGHTERS).expect("fits"));
        // Per connection: the combat word the receiver holds for its watched
        // fighter (viewer + 8, same room, never the viewer) and the tick it
        // fell behind on.
        let mut held = vec![1_000u32; usize::try_from(MP21_FIGHTERS).expect("fits")];
        let mut stale = vec![None::<u32>; usize::try_from(MP21_FIGHTERS).expect("fits")];
        let mut snapshot_bytes = 0usize;
        let mut combat_gap = 0u32;
        for tick in 1..=MP21_TICKS {
            let full = Instant::now();
            let started = Instant::now();
            world.step(tick);
            step_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            receipts.clear();
            let entities = &world.entities;
            let out = &mut world.out;
            for (index, conn) in world.conns.iter_mut().enumerate() {
                let viewer = u32::try_from(index).expect("viewer fits");
                if tick % SNAPSHOT_EVERY_TICKS != viewer % SNAPSHOT_EVERY_TICKS {
                    continue;
                }
                let me = &entities[usize::try_from(viewer).expect("viewer fits")];
                let (_, _, yaw) = mp21_fighter_pose(viewer, tick);
                let report = conn
                    .select(
                        &cfg,
                        entities,
                        viewer,
                        me.x_mm,
                        me.z_mm,
                        yaw,
                        tick,
                        SIEGE_BUDGET_BYTES,
                        out,
                    )
                    .expect("select inside the cap and budget");
                snapshot_bytes = snapshot_bytes.max(report.bytes);
                receipts.push((
                    viewer,
                    u32::try_from(report.selected).expect("selected fits"),
                    u32::try_from(report.bytes).expect("bytes fit"),
                ));
                let watcher = (viewer + SCOPE_ROOMS) % MP21_FIGHTERS;
                let watching = entities[usize::try_from(watcher).expect("watcher fits")].combat;
                if out[..report.selected].contains(&watcher) {
                    held[index] = watching;
                    stale[index] = None;
                } else if held[index] != watching {
                    let since = *stale[index].get_or_insert(tick);
                    combat_gap = combat_gap.max(tick - since);
                }
            }
            tick_ms.push(full.elapsed().as_secs_f64() * 1000.0);
            digests.push(world.digest(tick, &receipts));
        }
        let mut bytes_per_sec = 0u64;
        let mut starve_max = 0u32;
        let mut snapshots_per_client = 0u64;
        for conn in &world.conns {
            bytes_per_sec = bytes_per_sec.max(conn.bytes_per_sec(MP21_TICKS));
            starve_max = starve_max.max(conn.starve_max());
            snapshots_per_client = snapshots_per_client.max(conn.snapshots());
        }
        let hash = digests
            .iter()
            .fold(0u64, |acc, digest| acc.rotate_left(7) ^ digest);
        let tick_p99_ms = TickQuantiles::of("mp21 tick", &tick_ms)
            .expect("finite samples")
            .p99_ms;
        let step_p99_ms = TickQuantiles::of("mp21 step", &step_ms)
            .expect("finite samples")
            .p99_ms;
        Mp21Report {
            digests,
            hash,
            kbps_x100: bytes_per_sec * 8 * 100 / 1_000,
            bytes_per_sec,
            snapshot_bytes,
            starve_max,
            combat_gap,
            snapshots_per_client,
            tick_p99_ms,
            step_p99_ms,
        }
    }

    #[test]
    fn mp21_handoff_64_players_and_1024_entities_converge_x2() {
        let first = mp21_run();
        let second = mp21_run();
        // Hash x2, tick by tick: the first divergent tick names the drift.
        assert_eq!(
            first.digests.len(),
            usize::try_from(MP21_TICKS).expect("ticks fit")
        );
        for (index, (left, right)) in first.digests.iter().zip(&second.digests).enumerate() {
            assert_eq!(
                left,
                right,
                "MP21 drifted at tick {}: {left:016x} vs {right:016x}",
                index + 1
            );
        }
        assert_eq!(first.hash, second.hash, "MP21 hash x2 must agree");
        assert_ne!(first.hash, 0, "the spike must actually simulate");
        // Per client: inside the ON9 128 kbps bar, worst snapshot in budget.
        assert!(
            first.kbps_x100 <= ON9_CAP_KBPS_X100,
            "client {} kbps x100 over the {ON9_CAP_KBPS_X100} (128 kbps) bar",
            first.kbps_x100
        );
        assert!(
            first.snapshot_bytes <= SIEGE_BUDGET_BYTES,
            "worst snapshot {} B over the {SIEGE_BUDGET_BYTES} B budget",
            first.snapshot_bytes
        );
        // The two ledgers that desync a receiver: starve and combat handoff.
        assert!(
            first.starve_max <= STARVE_TICKS,
            "worst starve {} ticks over the {STARVE_TICKS}-tick bar",
            first.starve_max
        );
        assert!(
            first.combat_gap <= STARVE_TICKS,
            "combat body stale {} ticks over the {STARVE_TICKS}-tick bar",
            first.combat_gap
        );
        assert_eq!(
            first.snapshots_per_client,
            u64::from(MP21_TICKS) / u64::from(SNAPSHOT_EVERY_TICKS),
            "every client snapshots on its own lane"
        );
        // Timing sits in sight of the P6 bar; P6's own proof is release-only,
        // so a debug build prints the numbers instead of gating on them.
        if cfg!(debug_assertions) {
            println!(
                "MP21 timing (debug, not gated): tick_p99={:.3}ms step_p99={:.3}ms",
                first.tick_p99_ms, first.step_p99_ms
            );
        } else {
            check_p99(first.tick_p99_ms).expect("MP21 essentials p99 inside the P6 bar");
            assert!(
                first.step_p99_ms <= MP21_STEP_P99_MS,
                "world-step p99 {:.3} ms over {MP21_STEP_P99_MS} ms",
                first.step_p99_ms
            );
        }
        println!(
            "MP21 entities={} fighters={} legion={} ticks={} hash={:016x} \
             kbps_x100={} Bps={} snapshot_bytes={} starve_max={} combat_gap={} \
             snaps={} tick_p99={:.3}ms step_p99={:.3}ms budget_p99={BUDGET_P99_MS}ms",
            MP21_ENTITIES,
            MP21_FIGHTERS,
            MP21_LEGION_FIT,
            MP21_TICKS,
            first.hash,
            first.kbps_x100,
            first.bytes_per_sec,
            first.snapshot_bytes,
            first.starve_max,
            first.combat_gap,
            first.snapshots_per_client,
            first.tick_p99_ms,
            first.step_p99_ms,
        );
    }

    #[test]
    fn mp21_row_population_is_over_the_interest_cap() {
        // The row's literal population (64 fighters + 1000 legion = 1064)
        // does not fit the 1024-entity interest cap. The spike proves the
        // maximal legal shape; this test records the refusal by name so the
        // gap is a cap decision (owner / ON row), not a silent trim.
        assert_eq!(MP21_FIGHTERS + MP21_LEGION, 1_064);
        assert_eq!(MP21_LEGION_FIT, 960);
        assert_eq!(
            usize::try_from(MP21_ENTITIES).expect("entities fit"),
            MAX_INTEREST_ENTITIES
        );
        assert_eq!(
            ConnectionInterest::for_entities(0, 1_064).err(),
            Some(InterestError::TooManyEntities { got: 1_064 })
        );
        assert!(
            ConnectionInterest::for_entities(0, MAX_INTEREST_ENTITIES).is_ok(),
            "the cap itself is accepted"
        );
        let world = HandoffWorld::new();
        assert_eq!(world.entities.len(), MAX_INTEREST_ENTITIES);
        assert_eq!(
            world.conns.len(),
            usize::try_from(MP21_FIGHTERS).expect("fits")
        );
        println!(
            "MP21 cap: fighters={MP21_FIGHTERS} legion_row={MP21_LEGION} \
             total_row={} cap={MAX_INTEREST_ENTITIES} legion_fit={MP21_LEGION_FIT} \
             entities={MP21_ENTITIES} refused=1064",
            MP21_FIGHTERS + MP21_LEGION
        );
    }
}
