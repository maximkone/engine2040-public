//! scale: 8-16 fighter hardening (bandwidth caps, priority deltas, soak).
//! Owner: net lane. Units: bytes/s and bits/s for bandwidth, whole ticks
//! for time. Determinism: integer math and stable slot order only, so same
//! inputs give same bytes, same selection and same soak hash. Errors:
//! [`ScaleError`], never panics. Cap: match slots come from
//! `engine_input::MAX_PARTICIPANTS` (the canonical cap); this module
//! declares no private fighter cap. Crate edge `engine-net -> engine-input`
//! is in `tools/workspace-check.mjs`.

use std::collections::BTreeMap;

use engine_input::MAX_PARTICIPANTS;

use crate::session::{SNAPSHOT_HEADER_LEN, SNAPSHOT_SLOT_LEN, STATE_DELTA_HEADER_LEN};

/// The canonical slot cap as an array length (the one widening of it).
const SLOT_CAP: usize = MAX_PARTICIPANTS as usize;
/// Fixed tick rate in Hz (the arena tick [`engine_arena::TICK_HZ`], widened to u64).
pub const TICK_HZ: u64 = engine_arena::TICK_HZ as u64;
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
/// Headless 16-bot soak length in ticks (the proof bar).
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
    /// Fighter count outside `2..=16`.
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
                write!(f, "fighter count {got} outside 2..=16")
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
    /// True when every field matches (an idle slot the delta would omit).
    #[must_use]
    pub const fn same_as(self, other: Self) -> bool {
        self.x_mm == other.x_mm
            && self.y_mm == other.y_mm
            && self.z_mm == other.z_mm
            && self.yaw_ddeg == other.yaw_ddeg
            && self.hp == other.hp
            && self.stamina == other.stamina
    }

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
/// 64 kbps, up to 16 to 96 kbps (kbps * 1000 / 8).
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
    let mut prev: Option<u8> = None;
    for view in slots {
        if view.slot >= MAX_PARTICIPANTS {
            return Err(ScaleError::BadFighterCount {
                got: usize::from(view.slot),
            });
        }
        if prev.is_some_and(|p| view.slot <= p) {
            return Err(ScaleError::UnsortedSlots);
        }
        prev = Some(view.slot);
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
/// time over at most 16 slots, zero heap use (all working state is stack
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

    /// Step with an authored wish (human dwell): movement follows the held
    /// wish while yaw walk plus scripted attrition match [`SoakBot::step`],
    /// so human and AI slots fold through one bot owner.
    fn step_human(&mut self, wish_dx: i32, wish_dz: i32, slot: u8, tick: u32, seed: u64) {
        self.x_mm = self.x_mm.saturating_add(wish_dx);
        self.z_mm = self.z_mm.saturating_add(wish_dz);
        self.swing(slot, tick, seed);
    }

    /// Yaw walk plus scripted attrition shared by lattice and human steps.
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
/// Same seed plus same counts give the same hash on every run (the 16-bot
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
/// 8 at 20 Hz under 64 kbps, 16 at 15 Hz under 96 kbps.
pub fn bandwidth_chart() -> [ChartRow; 3] {
    let mut rows = [ChartRow {
        fighters: 0,
        hz: 0,
        snapshot_bytes: 0,
        bytes_per_sec: 0,
        kbps_target: 0,
    }; 3];
    let targets = [(2u8, 16u64), (8u8, 64u64), (16u8, 96u64)];
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

// Human-shaped bout fixture: reaction dwell, burst loss plus one mid-fight
// drop+rejoin, converging headless with full checkpoint equality. Owner:
// net lane. Units: ticks at 60 Hz, whole slots. Determinism: integer draws
// through the shared [`engine_core::mix64`] owner in stable slot order, so
// the same seed authors the same wishes and the same delivery ledger on
// every run. Errors: [`ScaleError`], never panics. Budget: the burst holds
// exactly [`RESIM_CAP_TICKS`] ticks (repeat-last prediction, then live
// resume); the mid-fight snapshot adopt re-baselines every lane, so burst
// and jitter residue heal with the drop victim and the resim window never
// grows past the cap. Distributions (confirm-or-correct handoff for the
// follow-up tuning row): dwell 5..15 ticks per wish epoch, delivery jitter
// 0..2 ticks per wish change, burst ticks 120..128 on both human uplinks,
// drop ticks 240..360 on the victim slot with one snapshot adopt at 360,
// ledger scheduled 4800 dropped 136 delivered 4664 resyncs 1 rollback_max 8.
// Loopback only: no sockets, no
// two-machine play here; played sessions stay operator-side hardware.

/// Human bout fighters: slots 0-1 author dwell-held wishes, the rest lattice.
pub const HUMAN_BOUT_FIGHTERS: u8 = 8;
/// Human bout length in ticks (600 ticks at 60 Hz, the soak bar).
pub const HUMAN_BOUT_TICKS: u32 = 600;
/// Checkpoint cadence in ticks (every second, ten per bout).
pub const HUMAN_CHECKPOINT_EVERY: u32 = 60;
/// Checkpoint slots (covers `SOAK_TICKS_MAX` at the checkpoint cadence).
pub const HUMAN_CHECKPOINT_CAP: usize = 60;
/// Slots with human-shaped wishes (the first two; the rest AI lattice).
pub const HUMAN_SLOTS: u8 = 2;
/// Drop victim slot (a human uplink).
pub const HUMAN_DROP_SLOT: u8 = 1;
/// Mid-fight drop window: first frozen tick (4 s in, 2 s long at 60 Hz).
pub const HUMAN_DROP_AT: u32 = 240;
/// First tick the victim is back (exact snapshot adopt, converge at once).
pub const HUMAN_BACK_AT: u32 = 360;
/// Burst-loss window start in ticks (2 s in).
pub const HUMAN_BURST_AT: u32 = 120;
/// Burst-loss length in ticks (exactly the resim cap: prediction then live).
pub const HUMAN_BURST_LEN: u32 = 8;
/// Shortest reaction dwell in ticks (5 ticks are about 80 ms at 60 Hz).
pub const HUMAN_DWELL_MIN: u32 = 5;
/// Dwell span in ticks (5..15 ticks cover about 80..250 ms reactions).
pub const HUMAN_DWELL_SPAN: u64 = 11;
/// Canonical human-bout seed (its own lane seed, not the soak seed).
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

/// Delivery jitter for one wish change: 0..2 ticks (about 0..30 ms).
fn jitter_hold(seed: u64, slot: u8, tick: u32) -> u32 {
    let lane = (seed ^ 0x4A49_5454_4552_3031)
        .wrapping_add(u64::from(slot).wrapping_mul(0x1656_67B1_9E37_79F9))
        .wrapping_add(u64::from(tick).wrapping_mul(0x85EB_CA6B_C2B2_AE35));
    u32::try_from(engine_core::mix64(lane) % 3).unwrap_or(0)
}

/// True while the fault schedule freezes `slot` at `tick`: the burst window
/// holds human uplinks, the drop window holds the victim (repeat-last).
const fn fault_frozen(slot: u8, tick: u32) -> bool {
    (slot < HUMAN_SLOTS && tick >= HUMAN_BURST_AT && tick < HUMAN_BURST_AT + HUMAN_BURST_LEN)
        || (slot == HUMAN_DROP_SLOT && tick >= HUMAN_DROP_AT && tick < HUMAN_BACK_AT)
}

/// True on the first live tick after a freeze: live wish resumes at once.
const fn fault_rejoined(slot: u8, tick: u32) -> bool {
    (slot < HUMAN_SLOTS && tick == HUMAN_BURST_AT + HUMAN_BURST_LEN)
        || (slot == HUMAN_DROP_SLOT && tick == HUMAN_BACK_AT)
}

/// One bout lane: the bot plus authored truth, applied delivery and ledger.
#[derive(Clone, Copy)]
struct BoutLane {
    bot: SoakBot,
    wish_dx: i32,
    wish_dz: i32,
    dwell_left: u32,
    epoch: u32,
    adx: i32,
    adz: i32,
    hold: u32,
    pend_dx: i32,
    pend_dz: i32,
    frozen_run: u32,
}

/// Seed one lane: humans open on epoch zero, AI lanes stay lattice-driven.
fn seed_lane(slot: u8, seed: u64) -> BoutLane {
    let (wish_dx, wish_dz) = if slot < HUMAN_SLOTS {
        human_wish(seed, slot, 0)
    } else {
        (0, 0)
    };
    BoutLane {
        bot: SoakBot::seed_bot(slot, seed),
        wish_dx,
        wish_dz,
        dwell_left: if slot < HUMAN_SLOTS {
            human_dwell(seed, slot, 0)
        } else {
            0
        },
        epoch: 0,
        adx: wish_dx,
        adz: wish_dz,
        hold: 0,
        pend_dx: wish_dx,
        pend_dz: wish_dz,
        frozen_run: 0,
    }
}

/// Advance one lane's authored wish; true when a new dwell epoch opens.
/// AI slots never dwell (their lattice steps every tick instead).
fn author_tick(lane: &mut BoutLane, seed: u64, slot: u8) -> bool {
    if slot >= HUMAN_SLOTS {
        return false;
    }
    let mut changed = false;
    if lane.dwell_left == 0 {
        lane.epoch += 1;
        let (dx, dz) = human_wish(seed, slot, lane.epoch);
        lane.wish_dx = dx;
        lane.wish_dz = dz;
        lane.dwell_left = human_dwell(seed, slot, lane.epoch);
        changed = true;
    }
    lane.dwell_left -= 1;
    changed
}

/// Headless bout report: the converged hash and checkpoints plus the delivery
/// ledger. `checkpoints` holds the adopted authoritative checkpoints (full
/// equality with the clean run); the ledger proves the faults really ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HumanBoutReport {
    /// Fighters in the bout.
    pub fighters: u8,
    /// Ticks stepped.
    pub ticks: u32,
    /// Converged hash (clean and faulted runs must agree).
    pub hash: u64,
    /// Adopted authoritative checkpoints (prefix of length `checkpoint_count`).
    pub checkpoints: [u64; HUMAN_CHECKPOINT_CAP],
    /// Checkpoints taken.
    pub checkpoint_count: usize,
    /// Slot-ticks the schedule offered.
    pub scheduled: u32,
    /// Slot-ticks delivered live.
    pub delivered: u32,
    /// Slot-ticks held by burst loss or the drop (repeat-last, then adopt).
    pub dropped: u32,
    /// Snapshot adopts taken (one per mid-fight rejoin).
    pub resyncs: u32,
    /// Deepest live resume in ticks (never past the resim cap).
    pub rollback_max: u32,
    /// Ticks after the last fault before every lane held truth (adopt exact).
    pub converge_ticks: u32,
}

impl HumanBoutReport {
    /// Checkpoints taken, oldest first.
    #[must_use]
    pub fn checkpoints(&self) -> &[u64] {
        &self.checkpoints[..self.checkpoint_count]
    }
}

impl std::fmt::Display for HumanBoutReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HUMAN bots={} ticks={} hash={:016x} cps={} sched={} delivered={} dropped={} resyncs={} rollback_max={} converge={}",
            self.fighters,
            self.ticks,
            self.hash,
            self.checkpoint_count,
            self.scheduled,
            self.delivered,
            self.dropped,
            self.resyncs,
            self.rollback_max,
            self.converge_ticks,
        )
    }
}

/// Run the clean human bout: dwell-held wishes, no delivery faults.
pub fn run_human_bout(seed: u64, fighters: u8, ticks: u32) -> Result<HumanBoutReport, ScaleError> {
    Ok(run_inner(seed, fighters, ticks, false, false)?.0)
}

/// Run the faulted human bout: the same wishes through jitter, one burst
/// window and one mid-fight drop+rejoin, converging to the clean checkpoints.
pub fn run_human_bout_with_faults(
    seed: u64,
    fighters: u8,
    ticks: u32,
) -> Result<HumanBoutReport, ScaleError> {
    Ok(run_inner(seed, fighters, ticks, true, true)?.0)
}

/// Step clean and faulted lanes side by side; returns the report plus the
/// fault-side checkpoint trail (the negative control reads the trail).
/// Wish-advance, fault-apply, adopt-heal and ledger-commit each own one step.
fn run_inner(
    seed: u64,
    fighters: u8,
    ticks: u32,
    faults: bool,
    adopt: bool,
) -> Result<(HumanBoutReport, [u64; HUMAN_CHECKPOINT_CAP]), ScaleError> {
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
    let mut state = bout_state(seed, usize::from(fighters));
    for tick in 0..ticks {
        adopt_heal(&mut state, tick, faults, adopt);
        wish_advance(&mut state, seed, tick);
        fault_apply(&mut state, seed, tick, faults);
        ledger_commit_tick(&mut state, tick);
    }
    Ok(ledger_commit(state, fighters, ticks, faults))
}

/// Live bout state stepped side by side: clean truth plus the faulted lane,
/// with the running hashes and the delivery ledger.
struct BoutState {
    clean: [BoutLane; SLOT_CAP],
    fault: [BoutLane; SLOT_CAP],
    clean_hash: u64,
    fault_hash: u64,
    checkpoints: [u64; HUMAN_CHECKPOINT_CAP],
    trail: [u64; HUMAN_CHECKPOINT_CAP],
    checkpoint_count: usize,
    dropped: u32,
    resyncs: u32,
    rollback_max: u32,
    jitter_max: u32,
    width: usize,
}

/// Lane index as a slot id: `take(width)` keeps every index below the slot cap.
fn slot_index(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(0)
}

/// Seed both sides of one bout: clean truth plus the faulted delivery lane.
fn bout_state(seed: u64, width: usize) -> BoutState {
    let mut clean = [seed_lane(0, seed); SLOT_CAP];
    let mut fault = [seed_lane(0, seed); SLOT_CAP];
    for (slot, lane) in clean.iter_mut().enumerate().take(width) {
        *lane = seed_lane(slot_index(slot), seed);
    }
    for (slot, lane) in fault.iter_mut().enumerate().take(width) {
        *lane = seed_lane(slot_index(slot), seed);
    }
    BoutState {
        clean,
        fault,
        clean_hash: 1_464_717_047_381_303_168 ^ seed,
        fault_hash: 1_464_717_047_381_303_168 ^ seed,
        checkpoints: [0u64; HUMAN_CHECKPOINT_CAP],
        trail: [0u64; HUMAN_CHECKPOINT_CAP],
        checkpoint_count: 0,
        dropped: 0,
        resyncs: 0,
        rollback_max: 0,
        jitter_max: 0,
        width,
    }
}

/// Adopt-heal: re-baseline every fault lane (authored wishes, delivery ledger
/// and bots) plus the running hash from the clean side, so burst residue and
/// jitter residue heal together with the drop victim and the tail steps live
/// from exact equality.
fn adopt_heal(state: &mut BoutState, tick: u32, faults: bool, adopt: bool) {
    if !(faults && adopt && tick == HUMAN_BACK_AT) {
        return;
    }
    if state.width <= usize::from(HUMAN_DROP_SLOT) {
        return;
    }
    for (slot, lane) in state.fault.iter_mut().enumerate().take(state.width) {
        *lane = state.clean[slot];
    }
    state.fault_hash = state.clean_hash;
    state.resyncs += 1;
}

/// Wish-advance: author one tick of wishes on the clean side and step its bots.
fn wish_advance(state: &mut BoutState, seed: u64, tick: u32) {
    for (slot, lane) in state.clean.iter_mut().enumerate().take(state.width) {
        let slot_u8 = slot_index(slot);
        author_tick(lane, seed, slot_u8);
        if slot_u8 < HUMAN_SLOTS {
            lane.adx = lane.wish_dx;
            lane.adz = lane.wish_dz;
            lane.bot.step_human(lane.adx, lane.adz, slot_u8, tick, seed);
        } else {
            lane.bot.step(slot_u8, tick, seed);
        }
        state.clean_hash = fold_bot(state.clean_hash, &lane.bot);
    }
}

/// Fault-apply: author one tick of wishes through the fault schedule and step
/// the faulted bots.
fn fault_apply(state: &mut BoutState, seed: u64, tick: u32, faults: bool) {
    for (slot, lane) in state.fault.iter_mut().enumerate().take(state.width) {
        let slot_u8 = slot_index(slot);
        if slot_u8 < HUMAN_SLOTS {
            let changed = author_tick(lane, seed, slot_u8);
            step_delivery(
                lane,
                seed,
                slot_u8,
                tick,
                changed,
                faults,
                &mut state.dropped,
                &mut state.rollback_max,
                &mut state.jitter_max,
            );
            lane.bot.step_human(lane.adx, lane.adz, slot_u8, tick, seed);
        } else {
            lane.bot.step(slot_u8, tick, seed);
        }
        state.fault_hash = fold_bot(state.fault_hash, &lane.bot);
    }
}

/// Ledger-commit: fold the tick into both running hashes and take the
/// checkpoint when the cadence lands.
fn ledger_commit_tick(state: &mut BoutState, tick: u32) {
    state.clean_hash ^= u64::from(tick);
    state.clean_hash = state.clean_hash.wrapping_mul(1_099_511_628_211);
    state.fault_hash ^= u64::from(tick);
    state.fault_hash = state.fault_hash.wrapping_mul(1_099_511_628_211);
    if (tick + 1).is_multiple_of(HUMAN_CHECKPOINT_EVERY)
        && state.checkpoint_count < HUMAN_CHECKPOINT_CAP
    {
        state.checkpoints[state.checkpoint_count] = state.clean_hash;
        state.trail[state.checkpoint_count] = state.fault_hash;
        state.checkpoint_count += 1;
    }
}

/// Ledger-commit: close the delivery ledger into the headless bout report.
fn ledger_commit(
    state: BoutState,
    fighters: u8,
    ticks: u32,
    faults: bool,
) -> (HumanBoutReport, [u64; HUMAN_CHECKPOINT_CAP]) {
    let rollback_max = state.rollback_max.max(state.jitter_max);
    let scheduled = u32::from(fighters).saturating_mul(ticks);
    (
        HumanBoutReport {
            fighters,
            ticks,
            hash: if faults {
                state.fault_hash
            } else {
                state.clean_hash
            },
            checkpoints: state.checkpoints,
            checkpoint_count: state.checkpoint_count,
            scheduled,
            delivered: scheduled.saturating_sub(state.dropped),
            dropped: state.dropped,
            resyncs: state.resyncs,
            rollback_max,
            converge_ticks: 0,
        },
        state.trail,
    )
}

/// Deliver one human lane's authored wish through the fault schedule: frozen
/// ticks repeat the last applied wish, a rejoin resumes the live wish, and a
/// fresh epoch before the mid-fight adopt applies at once or after its jitter
/// hold. Past the adopt the session is re-baselined, so fresh epochs apply
/// at once and the tail holds exact equality with the clean run.
#[expect(
    clippy::too_many_arguments,
    reason = "one delivery step keeps its ledger beside it"
)]
fn step_delivery(
    lane: &mut BoutLane,
    seed: u64,
    slot: u8,
    tick: u32,
    changed: bool,
    faults: bool,
    dropped: &mut u32,
    rollback_max: &mut u32,
    jitter_max: &mut u32,
) {
    if !faults {
        lane.adx = lane.wish_dx;
        lane.adz = lane.wish_dz;
        return;
    }
    if fault_frozen(slot, tick) {
        *dropped += 1;
        lane.frozen_run += 1;
        lane.hold = 0;
        return;
    }
    if lane.frozen_run > 0 {
        if !(slot == HUMAN_DROP_SLOT && tick == HUMAN_BACK_AT) {
            *rollback_max = (*rollback_max).max(lane.frozen_run);
        }
        lane.frozen_run = 0;
    }
    if fault_rejoined(slot, tick) {
        lane.adx = lane.wish_dx;
        lane.adz = lane.wish_dz;
        lane.hold = 0;
        return;
    }
    if lane.hold > 0 {
        lane.hold -= 1;
        if lane.hold == 0 {
            lane.adx = lane.pend_dx;
            lane.adz = lane.pend_dz;
        }
        return;
    }
    if changed && tick < HUMAN_BACK_AT {
        let wait = jitter_hold(seed, slot, tick);
        *jitter_max = (*jitter_max).max(wait);
        if wait == 0 {
            lane.adx = lane.wish_dx;
            lane.adz = lane.wish_dz;
        } else {
            lane.pend_dx = lane.wish_dx;
            lane.pend_dz = lane.wish_dz;
            lane.hold = wait;
        }
    } else if changed {
        lane.adx = lane.wish_dx;
        lane.adz = lane.wish_dz;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn full_16_snapshot_fits_a_datagram() {
        assert_eq!(full_wire_len(16), 313);
        assert!(full_wire_len(16) <= MAX_DATAGRAM_BYTES);
        assert_eq!(cap_bytes_per_snapshot(16).expect("cap"), 800);
        assert!(full_wire_len(16) <= cap_bytes_per_snapshot(16).expect("cap"));
    }

    #[test]
    fn combat_inputs_never_starve_under_snapshot_flood() {
        // 16-fighter shape stays within 1200 bytes, never fragments.
        let full = full_wire_len(16);
        assert_eq!(full, 313);
        assert!(full <= MAX_DATAGRAM_BYTES);
        within_cap(16, full).expect("16 must fit");
        assert!(full <= crate::session::IOQ3_FRAGMENT_SIZE);
        // Flood: every non-viewer slot combat-changed; budget is the cap.
        let base = lineup(16);
        let mut now = base.clone();
        for view in now.iter_mut().skip(1) {
            view.hp = view.hp.saturating_sub(120);
        }
        let mut first = [0u8; 16];
        let mut second = [0u8; 16];
        let a =
            select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut first).expect("select");
        let b =
            select_priority(&now, &base, 0, 9, 8, MAX_DATAGRAM_BYTES, &mut second).expect("select");
        assert_eq!((a, &first[..a]), (b, &second[..b]), "twice equal");
        assert_eq!(a, 16, "zero combat starvation");
        assert_eq!(
            &first[..a],
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
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
            within_cap(17, 47),
            Err(ScaleError::BadFighterCount { got: 17 })
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
    fn soak_16_bot_x600_converges_with_matching_hashes() {
        let a = run_soak(2040, 16, SOAK_TICKS).expect("soak");
        let b = run_soak(2040, 16, SOAK_TICKS).expect("soak");
        assert_eq!(a, b, "same seed must converge to the same report");
        assert_ne!(a.hash, 0, "soak must actually simulate");
        assert_eq!((a.fighters, a.ticks), (16, SOAK_TICKS));
        assert_eq!(a.snapshot_bytes, 313);
        assert_eq!(a.snapshot_hz, 15);
        assert_eq!(a.bytes_per_sec, 4_695);
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
        let a = soak_hash(1, 16, 600).expect("soak");
        let b = soak_hash(2, 16, 600).expect("soak");
        assert_ne!(a, b, "different seeds must diverge");
    }

    #[test]
    fn soak_rejects_bad_counts_and_absurd_ticks() {
        assert_eq!(
            soak_hash(1, 1, 10),
            Err(ScaleError::BadFighterCount { got: 1 })
        );
        assert_eq!(
            soak_hash(1, 17, 10),
            Err(ScaleError::BadFighterCount { got: 17 })
        );
        assert!(soak_hash(1, 16, SOAK_TICKS_MAX + 1).is_err());
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
    fn human_bout_converges_x2_with_full_checkpoint_equality() {
        // Reaction dwell plus burst loss plus one mid-fight drop+rejoin still
        // lands on the clean checkpoints: the headless convergence proof.
        let clean_a =
            run_human_bout(HUMAN_BOUT_SEED, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS).expect("bout");
        let clean_b =
            run_human_bout(HUMAN_BOUT_SEED, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS).expect("bout");
        assert_eq!(clean_a, clean_b, "same seed must author the same bout");
        let fault_a =
            run_human_bout_with_faults(HUMAN_BOUT_SEED, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS)
                .expect("faulted bout");
        let fault_b =
            run_human_bout_with_faults(HUMAN_BOUT_SEED, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS)
                .expect("faulted bout");
        assert_eq!(
            fault_a, fault_b,
            "the delivery ledger must be deterministic"
        );
        assert_eq!(clean_a.hash, fault_a.hash, "faults must converge to clean");
        assert_eq!(
            clean_a.checkpoints(),
            fault_a.checkpoints(),
            "adopted checkpoints must equal clean"
        );
        assert_eq!(clean_a.checkpoint_count, 10, "one per second over the bout");
        assert_ne!(clean_a.hash, 0, "the bout must actually simulate");
        // Ledger: burst freezes two uplinks for 8 ticks, the drop holds one
        // slot for 120 ticks, the rejoin adopts exactly once, and the deepest
        // live resume sits on the resim cap (never past it).
        assert_eq!(fault_a.scheduled, 4_800);
        assert_eq!(fault_a.dropped, 16 + 120);
        assert_eq!(fault_a.delivered, fault_a.scheduled - fault_a.dropped);
        assert_eq!(fault_a.resyncs, 1);
        assert_eq!(fault_a.rollback_max, RESIM_CAP_TICKS);
        assert_eq!(fault_a.converge_ticks, 0);
        assert_eq!(HUMAN_BURST_LEN, RESIM_CAP_TICKS);
        // Post-adopt trail matches live (not just by copy): the suffix after
        // the rejoin steps identically, while the pre-adopt prefix diverged.
        let (_, trail) = run_inner(
            HUMAN_BOUT_SEED,
            HUMAN_BOUT_FIGHTERS,
            HUMAN_BOUT_TICKS,
            true,
            true,
        )
        .expect("trail");
        assert_eq!(&trail[6..10], &clean_a.checkpoints()[6..10]);
        assert_ne!(&trail[..6], &clean_a.checkpoints()[..6]);
        println!("{fault_a}");
    }

    #[test]
    fn human_bout_without_resync_diverges() {
        // Negative control: the same faults without the mid-fight adopt must
        // move the hash, so the convergence above comes from the resync.
        let (clean, _) = run_inner(
            HUMAN_BOUT_SEED,
            HUMAN_BOUT_FIGHTERS,
            HUMAN_BOUT_TICKS,
            false,
            false,
        )
        .expect("clean");
        let (predicted, predicted_trail) = run_inner(
            HUMAN_BOUT_SEED,
            HUMAN_BOUT_FIGHTERS,
            HUMAN_BOUT_TICKS,
            true,
            false,
        )
        .expect("predicted");
        assert_eq!(predicted.resyncs, 0);
        assert!(predicted.dropped > 0, "the control must actually drop");
        assert_ne!(clean.hash, predicted.hash, "unhealed faults must diverge");
        assert_ne!(
            clean.checkpoints(),
            &predicted_trail[..predicted.checkpoint_count],
            "unhealed checkpoints must diverge"
        );
    }

    #[test]
    fn human_bout_rejects_bad_shapes() {
        assert_eq!(
            run_human_bout(11, 1, HUMAN_BOUT_TICKS),
            Err(ScaleError::BadFighterCount { got: 1 })
        );
        assert_eq!(
            run_human_bout_with_faults(11, 17, HUMAN_BOUT_TICKS),
            Err(ScaleError::BadFighterCount { got: 17 })
        );
        assert!(run_human_bout(11, HUMAN_BOUT_FIGHTERS, SOAK_TICKS_MAX + 1).is_err());
        assert!(run_human_bout_with_faults(11, HUMAN_BOUT_FIGHTERS, SOAK_TICKS_MAX + 1).is_err());
    }

    #[test]
    fn human_bout_converges_second_jitter_seed() {
        // Second jitter seed: same burst/drop windows, different wishes, so the
        // adopt must heal any residue, not just the canonical seed 11 tape.
        let seed = 23;
        let clean_a = run_human_bout(seed, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS).expect("bout");
        let clean_b = run_human_bout(seed, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS).expect("bout");
        assert_eq!(clean_a, clean_b, "seed 23 must author the same bout");
        let fault_a = run_human_bout_with_faults(seed, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS)
            .expect("faulted bout");
        let fault_b = run_human_bout_with_faults(seed, HUMAN_BOUT_FIGHTERS, HUMAN_BOUT_TICKS)
            .expect("faulted bout");
        assert_eq!(fault_a, fault_b, "seed 23 ledger must be deterministic");
        assert_eq!(clean_a.hash, fault_a.hash, "seed 23 must converge to clean");
        assert_eq!(
            clean_a.checkpoints(),
            fault_a.checkpoints(),
            "seed 23 checkpoints must equal clean"
        );
        assert_eq!(fault_a.dropped, 16 + 120);
        assert_eq!(fault_a.resyncs, 1);
        assert_ne!(clean_a.hash, 0, "the bout must actually simulate");
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
        for tick in 0..HUMAN_BOUT_TICKS {
            assert!(
                jitter_hold(HUMAN_BOUT_SEED, 0, tick) <= 2,
                "jitter must stay inside two ticks"
            );
        }
        assert!(fault_frozen(0, HUMAN_BURST_AT));
        assert!(fault_frozen(1, HUMAN_DROP_AT + 119));
        assert!(!fault_frozen(1, HUMAN_BACK_AT));
        assert!(!fault_frozen(4, HUMAN_BURST_AT));
    }

    #[test]
    fn human_bout_step_allocates_nothing() {
        let (report, heap) = engine_core::alloc_probe::measure(|| {
            run_inner(
                HUMAN_BOUT_SEED,
                HUMAN_BOUT_FIGHTERS,
                HUMAN_BOUT_TICKS,
                true,
                true,
            )
            .expect("bout")
            .0
        });
        assert_eq!(heap.calls, 0, "the bout path must not allocate");
        assert_ne!(report.hash, 0);
    }

    /// Encode one bout's authored wishes as fixed bytes: two human slots by
    /// 600 ticks by (dx, dz) as offset `u16` pairs, so both ends of a
    /// loopback compare tapes byte for byte. Steps the real `author_tick`
    /// path, never a copy of its dwell logic.
    fn bout_tape_bytes(seed: u64) -> [u8; 4800] {
        let mut lanes = [seed_lane(0, seed), seed_lane(1, seed)];
        let mut out = [0u8; 4800];
        let mut at = 0usize;
        for _ in 0..HUMAN_BOUT_TICKS {
            for (index, lane) in lanes.iter_mut().enumerate() {
                author_tick(lane, seed, slot_index(index));
                for wish in [lane.wish_dx, lane.wish_dz] {
                    let shifted = u16::try_from(wish + 1_024).unwrap_or(0);
                    let bytes = shifted.to_le_bytes();
                    out[at] = bytes[0];
                    out[at + 1] = bytes[1];
                    at += 2;
                }
            }
        }
        assert_eq!(at, 4800, "every wish must land in the tape");
        out
    }

    #[test]
    fn human_bout_tape_bytes_match_on_both_ends() {
        // Both ends author the same tape from the same seed: byte-identical
        // inputs, so delivery and resim start from the same bytes on every PC.
        let a = bout_tape_bytes(HUMAN_BOUT_SEED);
        let b = bout_tape_bytes(HUMAN_BOUT_SEED);
        assert_eq!(a, b, "same seed must author the same tape bytes");
        assert!(
            a.iter().any(|byte| *byte != 0),
            "the tape must actually drive"
        );
        // Negative control: a second seed authors a different tape.
        let other = bout_tape_bytes(23);
        assert_ne!(a, other, "different seeds must author different tapes");
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
        assert!(
            MP6_POSTED_WAN_ROLLBACK_MAX > RESIM_CAP_TICKS - 1,
            "window minus one would not cover the posted WAN max"
        );
        assert_eq!(HUMAN_BURST_LEN, RESIM_CAP_TICKS);
    }
}
