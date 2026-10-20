//! delta: snapshot deltas and remote interpolation.
//!
//! Quantized fighter snapshots ([`FullSnap`]) are the unit the multiplayer
//! app renders for remote fighters: positions in integer
//! millimeters, yaw in integer decidegrees, hp/stamina as-is. [`encode_delta`]
//! diffs one full snapshot against an acked baseline into a [`Delta`] that
//! omits unchanged slots; [`decode_delta`] rebuilds the full snapshot
//! roundtrip-exact. [`Interpolator`] holds a bounded buffer of timestamped
//! snapshots and answers constant-delay (default 300 ms) interpolation queries
//! with integer-only math, clamping — never extrapolating — past the newest
//! sample.
//!
//! Wire: the version byte stays [`WIRE_VERSION`](super::session::WIRE_VERSION)
//! on every message. Delta state packets reuse [`KIND_STATE`](super::session::KIND_STATE)
//! with the delta bit set on the count byte; bit clear means the full
//! input-tape encoding byte-for-byte, so full-only peers keep decoding full
//! messages and loudly reject delta ones. Full fighter snapshots travel under
//! [`KIND_SNAPSHOT`](super::session::KIND_SNAPSHOT) as the resync primitive.
//!
//! Consumer: the multiplayer app (remote rendering + bandwidth budget).
//! Data owner: the server owns authoritative snapshots; each client owns its
//! interpolator buffer. Units: mm, decidegrees, permille hp, stamina points,
//! milliseconds, fixed ticks. Determinism scope: same full plus same baseline
//! gives the same [`Delta`]; interpolation uses integer math only, so the same
//! samples plus the same query give the same answer on every tier-3 machine.
//! Error type: [`NetError`](super::session::NetError); invalid snapshots or
//! deltas reject loudly and change nothing, never panic.

use super::session::NetError;
use crate::validate::SortedUnique;

/// Field mask bit: `x_mm` present in a [`FighterDelta`].
pub const MASK_X: u8 = 0x01;
/// Field mask bit: `y_mm` present in a [`FighterDelta`].
pub const MASK_Y: u8 = 0x02;
/// Field mask bit: `z_mm` present in a [`FighterDelta`].
pub const MASK_Z: u8 = 0x04;
/// Field mask bit: `yaw_ddeg` present in a [`FighterDelta`].
pub const MASK_YAW: u8 = 0x08;
/// Field mask bit: `hp` present in a [`FighterDelta`].
pub const MASK_HP: u8 = 0x10;
/// Field mask bit: `stamina` present in a [`FighterDelta`].
pub const MASK_STAMINA: u8 = 0x20;
/// All field mask bits; any other bit in a mask rejects the delta.
pub const MASK_ALL: u8 = MASK_X | MASK_Y | MASK_Z | MASK_YAW | MASK_HP | MASK_STAMINA;

/// Default remote-interpolation delay in milliseconds (300 ms is 18 ticks at 60 Hz).
///
/// The smallest of 100, 150, 200 and 300 ms with no render missing its
/// bracketing snapshots over a 60 s soak at 2% loss (10 snapshots per second,
/// 33 to 67 ms one way). Receipt: `interp.rs` test
/// `interp_delay_default_is_the_smallest_with_no_miss_at_2_percent_loss`.
/// A render needs the snapshot interval, the slowest transit and one lost
/// snapshot to fit inside the delay. At 5% loss the share of misses is 37%
/// at 100 ms, 4.9% at 150, 1.9% at 200 and 0.03% at 300. Two lost snapshots
/// in a row still miss.
pub const DEFAULT_INTERP_DELAY_MS: u64 = 300;
/// Upper bound on buffered interpolation samples per [`Interpolator`].
pub const MAX_INTERP_SAMPLES: usize = 32;

/// One fighter's quantized state: feet position in millimeters, facing yaw in
/// decidegrees wrapped to `[-1800, 1800)`, hp in permille (`0..=1000`) and
/// stamina in points (`0..=MAX_STAMINA`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FighterSnap {
    /// Fighter slot, below `engine_input::MAX_PARTICIPANTS`.
    pub slot: u8,
    /// Feet x in millimeters.
    pub x_mm: i32,
    /// Feet y in millimeters.
    pub y_mm: i32,
    /// Feet z in millimeters.
    pub z_mm: i32,
    /// Facing yaw in decidegrees, `[-1800, 1800)`.
    pub yaw_ddeg: i16,
    /// Health in permille.
    pub hp: u16,
    /// Stamina points.
    pub stamina: u16,
}

/// Neutral name for [`FighterSnap`], one body's quantized snapshot slot. New
/// engine code names the type through this alias so it stays genre-free
/// (the GE2 ledger in `tools/workspace-check.mjs` never grows).
pub type BodySnap = FighterSnap;

/// Full quantized snapshot for one tick: every active slot exactly once,
/// sorted by slot. This is the interpolation sample and the delta baseline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FullSnap {
    /// Fixed tick this snapshot covers.
    pub tick: u32,
    /// One entry per active slot, sorted and unique.
    pub fighters: Vec<FighterSnap>,
}

/// One slot's change set: `mask` names the fields that differ from the
/// baseline; named values carry the new quantized state, unnamed fields are
/// zeroed and ignored. `mask` is never zero and never carries bits outside
/// [`MASK_ALL`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FighterDelta {
    /// Field mask over `MASK_*`; nonzero, subset of [`MASK_ALL`].
    pub mask: u8,
    /// New `x_mm` when [`MASK_X`] is set, else zero.
    pub x_mm: i32,
    /// New `y_mm` when [`MASK_Y`] is set, else zero.
    pub y_mm: i32,
    /// New `z_mm` when [`MASK_Z`] is set, else zero.
    pub z_mm: i32,
    /// New `yaw_ddeg` when [`MASK_YAW`] is set, else zero.
    pub yaw_ddeg: i16,
    /// New `hp` when [`MASK_HP`] is set, else zero.
    pub hp: u16,
    /// New `stamina` when [`MASK_STAMINA`] is set, else zero.
    pub stamina: u16,
}

/// Snapshot delta against an acked baseline: `base_tick` names the baseline
/// tick the sender diffed against, `tick` names the target tick the receiver
/// rebuilds, and `changed_slots` carries only the slots that differ — sorted
/// by slot, unchanged slots omitted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Delta {
    /// Baseline tick: must equal the baseline snapshot's tick on decode.
    pub base_tick: u32,
    /// Target tick: the rebuilt full snapshot's tick.
    pub tick: u32,
    /// Changed `(slot, delta)` pairs, sorted by slot, one per changed slot.
    pub changed_slots: Vec<(u8, FighterDelta)>,
}

/// Quantize meters to integer millimeters: rounds half away from zero.
/// Non-finite input maps to zero; finite out-of-range input saturates via
/// float-to-int saturation, so this never panics.
pub fn quantize_pos_m(v: f32) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v * 1000.0).round() as i32
}

/// Quantize yaw radians to integer decidegrees wrapped to `[-1800, 1800)`.
/// Non-finite input maps to zero; only `+ - * /` plus `round` plus integer
/// wrapping, so this is tier-3 safe.
pub fn quantize_yaw_rad(yaw: f32) -> i16 {
    if !yaw.is_finite() {
        return 0;
    }
    let decideg = (yaw * 180.0 / core::f32::consts::PI * 10.0).round() as i32;
    wrap_decideg(decideg)
}

/// Wrap integer decidegrees to `[-1800, 1800)`.
pub(crate) fn wrap_decideg(d: i32) -> i16 {
    let mut w = d % 3600;
    if w < -1800 {
        w += 3600;
    } else if w >= 1800 {
        w -= 3600;
    }
    w as i16
}

impl FighterSnap {
    /// Quantize one fighter's pose plus combat state; rejects slots at or
    /// above `engine_input::MAX_PARTICIPANTS`.
    pub fn quantize(
        slot: u8,
        pos_m: [f32; 3],
        yaw_rad: f32,
        hp: u16,
        stamina: u16,
    ) -> Result<Self, NetError> {
        if slot >= engine_input::MAX_PARTICIPANTS {
            return Err(NetError::BadInput);
        }
        Ok(Self {
            slot,
            x_mm: quantize_pos_m(pos_m[0]),
            y_mm: quantize_pos_m(pos_m[1]),
            z_mm: quantize_pos_m(pos_m[2]),
            yaw_ddeg: quantize_yaw_rad(yaw_rad),
            hp,
            stamina,
        })
    }
}

impl FullSnap {
    /// Build a snapshot from already-quantized fighters: at least one slot,
    /// every slot below `MAX_PARTICIPANTS`, sorted and unique — otherwise
    /// [`NetError::BadState`].
    pub fn new(tick: u32, fighters: Vec<FighterSnap>) -> Result<Self, NetError> {
        let snap = Self { tick, fighters };
        snap.validate()?;
        Ok(snap)
    }

    /// Range/order check shared by constructors and decoders.
    pub fn validate(&self) -> Result<(), NetError> {
        if self.fighters.is_empty()
            || self.fighters.len() > usize::from(engine_input::MAX_PARTICIPANTS)
        {
            return Err(NetError::BadState);
        }
        let mut order = SortedUnique::new();
        for fighter in &self.fighters {
            if fighter.slot >= engine_input::MAX_PARTICIPANTS || !order.push(fighter.slot) {
                return Err(NetError::BadState);
            }
        }
        Ok(())
    }

    /// Canonical full wire size in bytes (header plus every slot unmasked):
    /// the size-assertion baseline a [`Delta`] must beat when few slots change.
    pub fn full_wire_len(&self) -> usize {
        super::session::SNAPSHOT_HEADER_LEN
            + self.fighters.len() * super::session::SNAPSHOT_SLOT_LEN
    }
}

/// Range/order check for one in-memory delta (slot population is checked
/// against the baseline on decode, not here).
pub(crate) fn validate_delta_shape(delta: &Delta) -> Result<(), NetError> {
    if delta.changed_slots.len() > usize::from(engine_input::MAX_PARTICIPANTS) {
        return Err(NetError::TooManyInputs(delta.changed_slots.len()));
    }
    let mut order = SortedUnique::new();
    for (slot, change) in &delta.changed_slots {
        if *slot >= engine_input::MAX_PARTICIPANTS || !order.push(*slot) {
            return Err(NetError::BadState);
        }
        if change.mask == 0 || change.mask & !MASK_ALL != 0 {
            return Err(NetError::BadState);
        }
        if change.mask & MASK_X == 0 && change.x_mm != 0
            || change.mask & MASK_Y == 0 && change.y_mm != 0
            || change.mask & MASK_Z == 0 && change.z_mm != 0
            || change.mask & MASK_YAW == 0 && change.yaw_ddeg != 0
            || change.mask & MASK_HP == 0 && change.hp != 0
            || change.mask & MASK_STAMINA == 0 && change.stamina != 0
        {
            return Err(NetError::BadState);
        }
    }
    Ok(())
}

/// Diff `full` against `baseline` into a [`Delta`]: unchanged slots are
/// omitted, changed slots carry a field mask plus only the differing fields.
/// Both snapshots must validate with identical slot populations and
/// `full.tick >= baseline.tick`; anything else is [`NetError::BadState`].
/// An identical pair yields a valid empty delta.
pub fn encode_delta(full: &FullSnap, baseline: &FullSnap) -> Result<Delta, NetError> {
    full.validate()?;
    baseline.validate()?;
    if full.tick < baseline.tick {
        return Err(NetError::BadState);
    }
    if full.fighters.len() != baseline.fighters.len()
        || full
            .fighters
            .iter()
            .zip(baseline.fighters.iter())
            .any(|(a, b)| a.slot != b.slot)
    {
        return Err(NetError::BadState);
    }
    let mut changed_slots = Vec::new();
    for (new, old) in full.fighters.iter().zip(baseline.fighters.iter()) {
        let mut mask = 0u8;
        if new.x_mm != old.x_mm {
            mask |= MASK_X;
        }
        if new.y_mm != old.y_mm {
            mask |= MASK_Y;
        }
        if new.z_mm != old.z_mm {
            mask |= MASK_Z;
        }
        if new.yaw_ddeg != old.yaw_ddeg {
            mask |= MASK_YAW;
        }
        if new.hp != old.hp {
            mask |= MASK_HP;
        }
        if new.stamina != old.stamina {
            mask |= MASK_STAMINA;
        }
        if mask != 0 {
            changed_slots.push((
                new.slot,
                FighterDelta {
                    mask,
                    x_mm: if mask & MASK_X != 0 { new.x_mm } else { 0 },
                    y_mm: if mask & MASK_Y != 0 { new.y_mm } else { 0 },
                    z_mm: if mask & MASK_Z != 0 { new.z_mm } else { 0 },
                    yaw_ddeg: if mask & MASK_YAW != 0 {
                        new.yaw_ddeg
                    } else {
                        0
                    },
                    hp: if mask & MASK_HP != 0 { new.hp } else { 0 },
                    stamina: if mask & MASK_STAMINA != 0 {
                        new.stamina
                    } else {
                        0
                    },
                },
            ));
        }
    }
    Ok(Delta {
        base_tick: baseline.tick,
        tick: full.tick,
        changed_slots,
    })
}

/// Rebuild the target full snapshot: `delta.base_tick` must equal
/// `baseline.tick`, every changed slot must exist in the baseline with a
/// valid mask, and the baseline must validate — otherwise
/// [`NetError::BadState`] and no partial snapshot. Roundtrip-exact: decoding
/// an [`encode_delta`] delta against the same baseline reproduces `full`.
pub fn decode_delta(delta: &Delta, baseline: &FullSnap) -> Result<FullSnap, NetError> {
    baseline.validate()?;
    validate_delta_shape(delta)?;
    if delta.base_tick != baseline.tick {
        return Err(NetError::BadState);
    }
    if delta.tick < delta.base_tick {
        return Err(NetError::BadState);
    }
    let mut fighters = Vec::with_capacity(baseline.fighters.len());
    let mut changes = delta.changed_slots.iter().peekable();
    for base in &baseline.fighters {
        let mut fighter = *base;
        if changes.peek().is_some_and(|(slot, _)| *slot == base.slot) {
            let (_, change) = changes.next().expect("peeked change");
            if change.mask & MASK_X != 0 {
                fighter.x_mm = change.x_mm;
            }
            if change.mask & MASK_Y != 0 {
                fighter.y_mm = change.y_mm;
            }
            if change.mask & MASK_Z != 0 {
                fighter.z_mm = change.z_mm;
            }
            if change.mask & MASK_YAW != 0 {
                fighter.yaw_ddeg = change.yaw_ddeg;
            }
            if change.mask & MASK_HP != 0 {
                fighter.hp = change.hp;
            }
            if change.mask & MASK_STAMINA != 0 {
                fighter.stamina = change.stamina;
            }
        }
        fighters.push(fighter);
    }
    if changes.next().is_some() {
        return Err(NetError::BadState);
    }
    Ok(FullSnap {
        tick: delta.tick,
        fighters,
    })
}

/// Canonical delta wire size in bytes (state-delta header plus masked fields):
/// compare against [`FullSnap::full_wire_len`] for the bandwidth budget.
pub fn delta_wire_len(delta: &Delta) -> usize {
    super::session::STATE_DELTA_HEADER_LEN
        + delta
            .changed_slots
            .iter()
            .map(|(_, change)| {
                2 + (if change.mask & MASK_X != 0 { 4 } else { 0 })
                    + (if change.mask & MASK_Y != 0 { 4 } else { 0 })
                    + (if change.mask & MASK_Z != 0 { 4 } else { 0 })
                    + (if change.mask & MASK_YAW != 0 { 2 } else { 0 })
                    + (if change.mask & MASK_HP != 0 { 2 } else { 0 })
                    + (if change.mask & MASK_STAMINA != 0 {
                        2
                    } else {
                        0
                    })
            })
            .sum::<usize>()
}

/// Bitstream with overflow flag (MSG discipline, integer only, byte appends).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Ioq3Msg {
    /// Bytes written so far.
    pub bytes: Vec<u8>,
    /// Sticky overflow once a write exceeds the cap.
    pub overflowed: bool,
}
impl Ioq3Msg {
    /// Append bytes unless over cap; overflow sticks and writes stop.
    pub fn push_bytes(&mut self, cap: usize, chunk: &[u8]) {
        if self.overflowed {
            return;
        }
        if self.bytes.len().saturating_add(chunk.len()) > cap {
            self.overflowed = true;
            return;
        }
        self.bytes.extend_from_slice(chunk);
    }
    /// Take bytes when no overflow happened, else refuse loud.
    pub fn take(self) -> Result<Vec<u8>, NetError> {
        if self.overflowed {
            return Err(NetError::TooLarge);
        }
        Ok(self.bytes)
    }
}
/// ioq3 per-client delta shape: acked lastframe plus server time, flags,
/// area bytes, then field-masked entity fields against the acked baseline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ioq3SnapFields {
    /// Acked baseline tick (must equal baseline tick and delta base).
    pub lastframe: u32,
    /// Authoritative tick being sent.
    pub server_time: u32,
    /// Snapshot flags (pattern parity, integer only).
    pub snap_flags: u8,
    /// Visible-area bytes for the viewer.
    pub area_bytes: Vec<u8>,
    /// Field-masked entity delta against the acked baseline.
    pub delta: Delta,
}
/// Encode ioq3 fields: baseline tick must equal lastframe, else BadState.
pub fn ioq3_encode_fields(
    lastframe: u32,
    server_time: u32,
    snap_flags: u8,
    area_bytes: Vec<u8>,
    full: &FullSnap,
    baseline: &FullSnap,
) -> Result<Ioq3SnapFields, NetError> {
    if baseline.tick != lastframe {
        return Err(NetError::BadState);
    }
    let delta = encode_delta(full, baseline)?;
    if delta.base_tick != lastframe || delta.tick != server_time {
        return Err(NetError::BadState);
    }
    Ok(Ioq3SnapFields {
        lastframe,
        server_time,
        snap_flags,
        area_bytes,
        delta,
    })
}
/// Decode ioq3 fields against the acked baseline; stale baseline is BadState.
pub fn ioq3_decode_fields(
    fields: &Ioq3SnapFields,
    baseline: &FullSnap,
) -> Result<FullSnap, NetError> {
    if fields.lastframe != baseline.tick || fields.delta.base_tick != baseline.tick {
        return Err(NetError::BadState);
    }
    decode_delta(&fields.delta, baseline)
}
/// Wire bytes for ioq3 fields (delta wire plus lastframe/time/flags/area).
pub fn ioq3_fields_wire_len(fields: &Ioq3SnapFields) -> usize {
    delta_wire_len(&fields.delta)
        .saturating_add(fields.area_bytes.len())
        .saturating_add(10)
}

/// Interpolation answer: the blended snapshot plus whether the query hit a
/// clamp boundary (no extrapolation was performed).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InterpResult {
    /// Interpolated (or clamped) snapshot.
    pub state: FullSnap,
    /// True when the query fell outside the buffered range, the buffer held
    /// fewer than two samples, or the bracketing samples disagreed on slots:
    /// `state` is a stored sample, not a blend.
    pub clamped: bool,
}

/// Bounded remote-interpolation buffer: `push` timestamped authoritative
/// snapshots, then query `at` a render time (usually `now - delay_ms` via
/// [`Interpolator::delayed`]). Queries between samples blend with integer
/// math; queries outside the buffered range clamp to the nearest stored
/// sample and flag it — this never extrapolates past the newest sample.
/// Samples arrive in any order and are kept sorted; same-time pushes replace.
pub struct Interpolator {
    delay_ms: u64,
    cap: usize,
    samples: Vec<(u64, FullSnap)>,
}

impl Interpolator {
    /// Open a buffer delaying `delay_ms` (see [`DEFAULT_INTERP_DELAY_MS`]) and
    /// holding [`MAX_INTERP_SAMPLES`].
    pub fn new(delay_ms: u64) -> Self {
        Self::with_cap(delay_ms, MAX_INTERP_SAMPLES)
    }

    /// Open a buffer with an explicit cap, clamped to at least 2 samples and
    /// at most [`MAX_INTERP_SAMPLES`] so the buffer stays bounded.
    pub fn with_cap(delay_ms: u64, cap: usize) -> Self {
        Self {
            delay_ms,
            cap: cap.clamp(2, MAX_INTERP_SAMPLES),
            samples: Vec::new(),
        }
    }

    /// Interpolation delay in milliseconds.
    pub fn delay_ms(&self) -> u64 {
        self.delay_ms
    }

    /// Buffered sample count.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when no sample arrived yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Store one timestamped snapshot; invalid snapshots are rejected and
    /// report `false`, leaving the buffer unchanged. Same-time pushes replace
    /// the stored sample; the oldest samples evict past the cap.
    pub fn push(&mut self, time_ms: u64, state: FullSnap) -> bool {
        if state.validate().is_err() {
            return false;
        }
        match self.samples.binary_search_by_key(&time_ms, |(t, _)| *t) {
            Ok(index) => self.samples[index] = (time_ms, state),
            Err(index) => self.samples.insert(index, (time_ms, state)),
        }
        while self.samples.len() > self.cap {
            self.samples.remove(0);
        }
        true
    }

    /// Answer the constant-delay query for render time `now_ms`.
    pub fn delayed(&self, now_ms: u64) -> Option<InterpResult> {
        self.at(now_ms.saturating_sub(self.delay_ms))
    }

    /// Answer the query at `time_ms`: blends the bracketing samples with
    /// integer math, or clamps to the nearest stored sample with
    /// `clamped: true` when the query leaves the buffered range. `None` only
    /// when the buffer is empty.
    pub fn at(&self, time_ms: u64) -> Option<InterpResult> {
        let (first_time, _) = self.samples.first()?;
        let (last_time, _) = self.samples.last()?;
        if self.samples.len() < 2 || time_ms <= *first_time || time_ms >= *last_time {
            let index = if time_ms <= *first_time {
                0
            } else {
                self.samples.len() - 1
            };
            let clamped = self.samples.len() < 2 || time_ms != self.samples[index].0;
            return Some(InterpResult {
                state: self.samples[index].1.clone(),
                clamped,
            });
        }
        let upper = self
            .samples
            .iter()
            .position(|(t, _)| *t > time_ms)
            .expect("query strictly inside a multi-sample buffer");
        let (ta, a) = &self.samples[upper - 1];
        let (tb, b) = &self.samples[upper];
        if a.fighters.len() != b.fighters.len()
            || a.fighters
                .iter()
                .zip(b.fighters.iter())
                .any(|(x, y)| x.slot != y.slot)
        {
            return Some(InterpResult {
                state: a.clone(),
                clamped: true,
            });
        }
        let num = i128::from(time_ms - ta);
        let den = i128::from(tb - ta);
        let mut fighters = Vec::with_capacity(a.fighters.len());
        for (x, y) in a.fighters.iter().zip(b.fighters.iter()) {
            fighters.push(FighterSnap {
                slot: x.slot,
                x_mm: lerp_i32(x.x_mm, y.x_mm, num, den),
                y_mm: lerp_i32(x.y_mm, y.y_mm, num, den),
                z_mm: lerp_i32(x.z_mm, y.z_mm, num, den),
                yaw_ddeg: lerp_yaw(x.yaw_ddeg, y.yaw_ddeg, num, den),
                hp: x.hp,
                stamina: x.stamina,
            });
        }
        Some(InterpResult {
            state: FullSnap {
                tick: a.tick,
                fighters,
            },
            clamped: false,
        })
    }
}

/// Integer lerp with half-away-from-zero rounding: `a + (b - a) * num / den`.
/// `den` must be positive; saturates on absurd ranges instead of panicking.
fn lerp_i32(a: i32, b: i32, num: i128, den: i128) -> i32 {
    if den <= 0 || num <= 0 {
        return a;
    }
    let diff = i128::from(b) - i128::from(a);
    let scaled = diff.saturating_mul(num);
    let rounded = if scaled >= 0 {
        (scaled + den / 2) / den
    } else {
        (scaled - den / 2) / den
    };
    let blended = i128::from(a) + rounded;
    i32::try_from(blended).unwrap_or(if blended < 0 { i32::MIN } else { i32::MAX })
}

/// Shortest-path yaw lerp in decidegrees: wraps the delta to
/// `[-1800, 1800)` first so a `1700 -> -1700` blend crosses 1800 instead of
/// spinning the long way around. Exactly-opposite endpoints deterministically
/// take the positive direction.
fn lerp_yaw(a: i16, b: i16, num: i128, den: i128) -> i16 {
    if den <= 0 || num <= 0 {
        return a;
    }
    let mut diff = (i32::from(b) - i32::from(a)) % 3600;
    if diff < -1800 {
        diff += 3600;
    } else if diff > 1800 {
        diff -= 3600;
    }
    if diff == -1800 {
        diff = 1800;
    }
    let scaled = i128::from(diff) * num;
    let rounded = if scaled >= 0 {
        (scaled + den / 2) / den
    } else {
        (scaled - den / 2) / den
    };
    wrap_decideg(i32::from(a) + rounded as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u32, raws: &[(u8, [f32; 3], f32, u16, u16)]) -> FullSnap {
        FullSnap::new(
            tick,
            raws.iter()
                .map(|(slot, pos, yaw, hp, stam)| {
                    FighterSnap::quantize(*slot, *pos, *yaw, *hp, *stam).expect("slot")
                })
                .collect(),
        )
        .expect("snapshot")
    }

    fn base_tick0() -> FullSnap {
        snap(
            10,
            &[
                (0, [1.0, 0.0, 2.0], 0.5, 1000, 100),
                (1, [-3.0, 0.0, 4.0], -1.0, 880, 60),
            ],
        )
    }

    #[test]
    fn delta_roundtrip_equals_full_snapshot() {
        let baseline = base_tick0();
        // Every field family changes somewhere: slot 0 moves and turns,
        // slot 1 takes damage and spends stamina.
        let full = snap(
            12,
            &[
                (0, [1.5, 0.0, 2.0], 0.6, 1000, 100),
                (1, [-3.0, 0.0, 4.0], -1.0, 760, 35),
            ],
        );
        let delta = encode_delta(&full, &baseline).expect("encode");
        assert_eq!((delta.base_tick, delta.tick), (10, 12));
        assert_eq!(delta.changed_slots.len(), 2);
        let rebuilt = decode_delta(&delta, &baseline).expect("decode");
        assert_eq!(rebuilt, full);
        // Identical pair: valid empty delta, still roundtrip-exact.
        let noop = encode_delta(&baseline, &baseline).expect("noop");
        assert!(noop.changed_slots.is_empty());
        assert_eq!(
            decode_delta(&noop, &baseline).expect("noop decode"),
            baseline
        );
    }

    #[test]
    fn ioq3_delta_beats_full_by_thirty_percent() {
        let raws: Vec<(u8, [f32; 3], f32, u16, u16)> = (0..8)
            .map(|slot| (slot, [f32::from(slot), 0.0, 0.0], 0.0, 1000, 100))
            .collect();
        let baseline = snap(100, &raws);
        let mut changed = raws.clone();
        changed[5].1 = [5.5, 0.0, 0.0];
        changed[5].4 = 90;
        let full = snap(106, &changed);
        let fields =
            ioq3_encode_fields(100, 106, 0, vec![0xA5, 0x5A], &full, &baseline).expect("fields");
        assert_eq!((fields.lastframe, fields.server_time), (100, 106));
        assert_eq!(
            ioq3_decode_fields(&fields, &baseline).expect("decode"),
            full,
            "decode against the acked baseline equals full"
        );
        let wire = ioq3_fields_wire_len(&fields);
        let budget = full.full_wire_len() * 7 / 10;
        assert!(
            wire <= budget,
            "delta wire {wire} must be 30%+ smaller than full {}",
            full.full_wire_len()
        );
        let stale = snap(99, &raws);
        assert_eq!(
            ioq3_encode_fields(100, 106, 0, Vec::new(), &full, &stale),
            Err(NetError::BadState)
        );
        assert_eq!(ioq3_decode_fields(&fields, &stale), Err(NetError::BadState));
        let mut msg = Ioq3Msg::default();
        msg.push_bytes(4, &[1, 2, 3]);
        msg.push_bytes(4, &[4, 5]);
        assert!(msg.overflowed);
        assert_eq!(msg.take(), Err(NetError::TooLarge));
    }

    #[test]
    fn unchanged_slots_omitted_and_delta_is_smaller() {
        let baseline = snap(
            20,
            &[
                (0, [0.0, 0.0, 0.0], 0.0, 1000, 100),
                (1, [5.0, 0.0, 0.0], 1.0, 1000, 100),
                (2, [-5.0, 0.0, 0.0], 2.0, 1000, 100),
                (3, [0.0, 0.0, 5.0], 3.0, 1000, 100),
            ],
        );
        let mut full = baseline.clone();
        full.tick = 21;
        full.fighters[2].hp = 880;
        let delta = encode_delta(&full, &baseline).expect("encode");
        assert_eq!(delta.changed_slots.len(), 1);
        assert_eq!(delta.changed_slots[0].0, 2);
        assert_eq!(delta.changed_slots[0].1.mask, MASK_HP);
        // One masked u16 on the wire beats four full slots.
        assert!(
            delta_wire_len(&delta) < full.full_wire_len(),
            "delta {} must beat full {}",
            delta_wire_len(&delta),
            full.full_wire_len()
        );
        assert_eq!(decode_delta(&delta, &baseline).expect("decode"), full);
    }

    #[test]
    fn sixteen_slot_delta_roundtrip() {
        // Deltas validate against the canonical cap only — a full
        // 16-slot snapshot diffs and rebuilds roundtrip-exact.
        let slots: Vec<(u8, [f32; 3], f32, u16, u16)> = (0..engine_input::MAX_PARTICIPANTS)
            .map(|slot| {
                (
                    slot,
                    [f32::from(slot), 0.0, 0.0],
                    f32::from(slot) * 0.1,
                    1000,
                    100,
                )
            })
            .collect();
        let baseline = snap(50, &slots);
        assert_eq!(
            baseline.fighters.len(),
            engine_input::MAX_PARTICIPANTS as usize
        );
        let mut moved = baseline.clone();
        moved.tick = 51;
        for fighter in &mut moved.fighters {
            fighter.x_mm += 10;
        }
        let delta = encode_delta(&moved, &baseline).expect("encode");
        assert_eq!(
            delta.changed_slots.len(),
            engine_input::MAX_PARTICIPANTS as usize
        );
        assert_eq!(decode_delta(&delta, &baseline).expect("decode"), moved);
        // One changed slot omits fifteen: the delta beats the full wire size.
        let mut single = baseline.clone();
        single.tick = 51;
        single.fighters[15].hp = 700;
        let small = encode_delta(&single, &baseline).expect("encode");
        assert_eq!(small.changed_slots.len(), 1);
        assert_eq!(small.changed_slots[0].0, 15);
        assert!(
            delta_wire_len(&small) < single.full_wire_len(),
            "delta {} must beat full {}",
            delta_wire_len(&small),
            single.full_wire_len()
        );
        assert_eq!(decode_delta(&small, &baseline).expect("decode"), single);
        // Slot 16 sits outside the canonical cap.
        assert_eq!(
            FighterSnap::quantize(engine_input::MAX_PARTICIPANTS, [0.0; 3], 0.0, 1000, 100)
                .map(|_| ()),
            Err(NetError::BadInput)
        );
    }

    #[test]
    fn interpolation_midpoint_exact_on_scripted_samples() {
        let mut interp = Interpolator::new(100);
        assert!(interp.is_empty());
        let a = snap(30, &[(0, [0.0, 0.0, 0.0], 0.0, 1000, 100)]);
        let b = snap(36, &[(0, [1.0, 0.0, 0.0], 1.0, 880, 40)]);
        assert!(interp.push(1000, a));
        assert!(interp.push(1200, b));
        assert_eq!(interp.len(), 2);
        let mid = interp.at(1100).expect("midpoint");
        assert!(!mid.clamped);
        assert_eq!(mid.state.fighters[0].x_mm, 500);
        assert_eq!(mid.state.fighters[0].y_mm, 0);
        // Half of quantized yaw(1.0 rad): integer lerp of the endpoints.
        let yaw_a = quantize_yaw_rad(0.0);
        let yaw_b = quantize_yaw_rad(1.0);
        let want_yaw = lerp_yaw(yaw_a, yaw_b, 100, 200);
        assert_eq!(mid.state.fighters[0].yaw_ddeg, want_yaw);
        // Discrete combat state steps: the earlier sample holds mid-blend.
        assert_eq!(mid.state.fighters[0].hp, 1000);
        assert_eq!(mid.state.fighters[0].stamina, 100);
        // Exact sample hits are not clamps.
        assert!(!interp.at(1000).expect("first").clamped);
        assert!(!interp.at(1200).expect("last").clamped);
        // Constant-delay query subtracts the configured delay.
        let delayed = interp.delayed(1200).expect("delayed");
        assert_eq!(delayed.state.fighters[0].x_mm, 500);
    }

    #[test]
    fn no_extrapolation_clamp_is_flagged() {
        let mut interp = Interpolator::new(100);
        assert_eq!(interp.at(999), None);
        let a = snap(40, &[(0, [2.0, 0.0, 0.0], 0.0, 1000, 100)]);
        let b = snap(41, &[(0, [4.0, 0.0, 0.0], 0.0, 1000, 100)]);
        assert!(interp.push(1000, a.clone()));
        // One sample: the sample itself, flagged.
        let solo = interp.at(1000).expect("solo");
        assert!((solo.clamped, solo.state) == (true, a.clone()));
        assert!(interp.push(1200, b.clone()));
        // Past the newest sample: newest state, flagged, never extrapolated.
        let past = interp.at(5000).expect("past");
        assert!(past.clamped);
        assert_eq!(past.state, b);
        // Before the oldest sample: oldest state, flagged.
        let early = interp.at(0).expect("early");
        assert!(early.clamped);
        assert_eq!(early.state, a);
    }

    #[test]
    fn yaw_takes_the_short_path_across_the_seam() {
        // 1700 -> -1700 ddeg is +200 the short way (through 1800), not -3400.
        assert_eq!(lerp_yaw(1700, -1700, 1, 2), -1800);
        assert_eq!(lerp_yaw(-1700, 1700, 1, 2), -1800);
    }

    #[test]
    fn integer_lerp_rounds_and_saturates_instead_of_wrapping() {
        assert_eq!(lerp_i32(10, 20, 1, 2), 15);
        assert_eq!(lerp_i32(-10, 10, 1, 4), -5);
        assert_eq!(lerp_i32(i32::MIN, i32::MAX, 1, 2), 0);
        assert_eq!(lerp_i32(7, 7, 5, 1), 7);
        // Degenerate spans return the start; a blend factor past 1 pins to the rails.
        assert_eq!(lerp_i32(5, 9, 0, 1), 5);
        assert_eq!(lerp_i32(5, 9, 3, 0), 5);
        assert_eq!(lerp_i32(0, i32::MAX, 10, 1), i32::MAX);
        assert_eq!(lerp_i32(0, i32::MIN, 10, 1), i32::MIN);
    }

    #[test]
    fn quantize_spells_out_nan_and_range_rules() {
        assert_eq!(quantize_pos_m(1.5), 1500);
        assert_eq!(quantize_pos_m(f32::NAN), 0);
        assert_eq!(quantize_pos_m(f32::INFINITY), 0);
        assert_eq!(quantize_yaw_rad(f32::NAN), 0);
        assert_eq!(quantize_yaw_rad(0.0), 0);
        assert_eq!(quantize_yaw_rad(core::f32::consts::PI), -1800);
    }

    #[test]
    fn bad_deltas_reject_loudly() {
        let baseline = base_tick0();
        let mut full = baseline.clone();
        full.tick = 11;
        full.fighters[0].x_mm += 1;
        let mut delta = encode_delta(&full, &baseline).expect("encode");
        // Wrong baseline tick.
        let stale = FullSnap::new(9, baseline.fighters.clone()).expect("stale");
        assert_eq!(decode_delta(&delta, &stale), Err(NetError::BadState));
        // Unknown slot in the delta.
        delta.changed_slots.push((
            3,
            FighterDelta {
                mask: MASK_HP,
                x_mm: 0,
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
                hp: 1,
                stamina: 0,
            },
        ));
        delta.changed_slots.sort_by_key(|(slot, _)| *slot);
        assert_eq!(decode_delta(&delta, &baseline), Err(NetError::BadState));
        // A target older than the baseline is not a forward diff.
        assert_eq!(encode_delta(&stale, &baseline), Err(NetError::BadState));
        // An empty population is not a snapshot.
        assert_eq!(FullSnap::new(0, Vec::new()), Err(NetError::BadState));
    }

    #[test]
    fn interpolator_bounds_and_rejects_gracefully() {
        let tiny = Interpolator::with_cap(100, 0);
        assert_eq!(tiny.len(), 0);
        let mut interp = Interpolator::with_cap(100, 3);
        for t in [1000, 1100, 1200, 1300] {
            assert!(interp.push(t, base_tick0()));
        }
        assert_eq!(interp.len(), 3);
        // Same-time push replaces instead of growing.
        assert!(interp.push(1300, base_tick0()));
        assert_eq!(interp.len(), 3);
        // Invalid snapshots never enter the buffer.
        assert!(!interp.push(
            1400,
            FullSnap {
                tick: 0,
                fighters: Vec::new()
            }
        ));
        assert_eq!(interp.len(), 3);
    }
}
