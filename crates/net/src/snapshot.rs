//! snapshot: save/load plus delayed confirmed history with a timeline sampler.
//!
//! One owner for snapshot bytes in tests: [`save_snapshot`] encodes a
//! [`FullSnap`] with the frozen session codec,
//! [`load_snapshot`] decodes it back. File helpers cover temp-dir fixtures.
//! [`ConfirmedHistory`] holds delayed authoritative snapshots with
//! unchanged-marker compression; [`TimelineSampler`] trails the newest
//! confirmed tick by a delay and samples the history for presentation.
//!
//! PORT(cBournhonesque/lightyear@96ded4ed27a8d6a465cbc10382518a7a371107f4:
//! crates/core/core/src/confirmed_history.rs,
//! crates/replication/interpolation/src/timeline.rs) MIT OR Apache-2.0, read
//! live 2026-10-09; reimplemented std-only over [`FullSnap`]
//! and `u32` ticks, zero donor bytes.
//!
//! Consumer: net tests plus [`TimelineSampler`], which samples
//! [`ConfirmedHistory`]. Data owner: the server owns snapshots; this
//! module only copies bytes. Units: ticks plus quantized mm/ddeg/hp/stamina.
//! Determinism scope: same snapshot gives the same bytes on every machine;
//! same history inserts give the same anchors (integer ticks only).
//! Error type: [`NetError`] for bytes,
//! [`std::io::Error`] for files. Invalid bytes reject loudly, never panic.

use std::collections::VecDeque;
use std::path::Path;

use super::session::{decode_snapshot, encode_snapshot, encode_state_delta, NetError};
use crate::delay::{clamp_delay, DEFAULT_DELAY};
use crate::delta::{encode_delta, FighterSnap, FullSnap};
use engine_arena::Checkpoint;
use engine_core::fixed::Fixed;

/// Encode one full snapshot to its canonical wire bytes.
///
/// # Errors
///
/// Returns [`NetError`] when the snapshot fails validation or misses the
/// size budget.
pub fn save_snapshot(full: &FullSnap) -> Result<Vec<u8>, NetError> {
    full.validate()?;
    encode_snapshot(full)
}

/// Decode one snapshot saved by [`save_snapshot`]; the result equals the
/// saved value field for field.
///
/// # Errors
///
/// Returns [`NetError`] for foreign versions, bad magic, truncated or
/// over-budget bytes, or invalid slot order.
pub fn load_snapshot(bytes: &[u8]) -> Result<FullSnap, NetError> {
    let snap = decode_snapshot(bytes)?;
    snap.validate()?;
    Ok(snap)
}

/// Write one snapshot to a file with [`save_snapshot`] bytes.
pub fn save_to_file(full: &FullSnap, path: &Path) -> Result<(), SaveError> {
    let bytes = save_snapshot(full).map_err(SaveError::Encode)?;
    std::fs::write(path, bytes).map_err(SaveError::Io)?;
    Ok(())
}

/// Read one snapshot written by [`save_to_file`].
pub fn load_from_file(path: &Path) -> Result<FullSnap, SaveError> {
    let bytes = std::fs::read(path).map_err(SaveError::Io)?;
    load_snapshot(&bytes).map_err(SaveError::Encode)
}

/// Save/load file failure: bad snapshot bytes or an IO error.
#[derive(Debug)]
pub enum SaveError {
    /// Snapshot bytes failed validation.
    Encode(NetError),
    /// File read or write failed.
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(err) => write!(f, "snapshot encode: {err}"),
            Self::Io(err) => write!(f, "snapshot file: {err}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(err) => Some(err),
            Self::Io(err) => Some(err),
        }
    }
}

/// Upper bound on confirmed anchors held per [`ConfirmedHistory`]: 64 snapshots
/// of at most 1200 bytes each, so one history costs at most ~77 KB.
pub const MAX_CONFIRMED_SAMPLES: usize = 64;

/// Authoritative snapshot state at one tick: a present snapshot or a removal.
#[derive(Clone, PartialEq, Eq, Debug)]
enum SnapState {
    /// Authoritative snapshot for the tick.
    Present(FullSnap),
    /// Authoritative removal: no snapshot is live at the tick.
    Removed,
}

/// Raw history entry: an explicit state or an unchanged-from-precedent marker.
#[derive(Clone, PartialEq, Eq, Debug)]
enum RawEntry {
    /// Explicit authoritative state received from replication.
    Explicit(SnapState),
    /// Unchanged from the closest preceding explicit entry. Resolves
    /// dynamically, so a late explicit insert before this tick rewrites it.
    SameAsPrecedent,
}

/// Delayed confirmed history over authoritative snapshots.
///
/// Anchors stay ordered oldest to newest by tick; late (out-of-order) inserts
/// keep the order, and an insert that equals the fighter state strictly before
/// its tick compresses to a `RawEntry::SameAsPrecedent` marker (anchors
/// compare fighter state, never the tick key). Past the cap the oldest anchor
/// evicts and the next marker materializes, so lookups keep resolving.
/// Integer ticks only, no wall clock: the same inserts give the same anchors.
pub struct ConfirmedHistory {
    cap: usize,
    buffer: VecDeque<(u32, RawEntry)>,
}

impl ConfirmedHistory {
    /// Open a history holding at most `cap` anchors, clamped to
    /// `2..=MAX_CONFIRMED_SAMPLES` so the buffer stays bounded.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(2, MAX_CONFIRMED_SAMPLES),
            buffer: VecDeque::new(),
        }
    }

    /// Buffered anchor count.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True when no anchor arrived yet.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Anchor bound from [`Self::new`].
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Oldest buffered tick.
    pub fn oldest_tick(&self) -> Option<u32> {
        self.buffer.front().map(|(tick, _)| *tick)
    }

    /// Newest buffered tick.
    pub fn newest_tick(&self) -> Option<u32> {
        self.buffer.back().map(|(tick, _)| *tick)
    }

    /// Insert one authoritative snapshot, compressing to an unchanged marker
    /// when its fighter state equals the state strictly before `tick`.
    /// Invalid snapshots reject with `false` and change nothing.
    pub fn insert(&mut self, tick: u32, snap: FullSnap) -> bool {
        if snap.validate().is_err() {
            return false;
        }
        let same = match self.state_before(tick) {
            Some(SnapState::Present(prev)) => prev.fighters == snap.fighters,
            Some(SnapState::Removed) | None => false,
        };
        let entry = if same {
            RawEntry::SameAsPrecedent
        } else {
            RawEntry::Explicit(SnapState::Present(snap))
        };
        self.insert_raw(tick, entry);
        true
    }

    /// Insert one authoritative snapshot without unchanged compression.
    /// Invalid snapshots reject with `false` and change nothing.
    pub fn insert_explicit(&mut self, tick: u32, snap: FullSnap) -> bool {
        if snap.validate().is_err() {
            return false;
        }
        self.insert_raw(tick, RawEntry::Explicit(SnapState::Present(snap)));
        true
    }

    /// Insert an authoritative removal, compressing to an unchanged marker
    /// when the state strictly before `tick` is already removed.
    pub fn insert_removed(&mut self, tick: u32) -> bool {
        let entry = if matches!(self.state_before(tick), Some(SnapState::Removed)) {
            RawEntry::SameAsPrecedent
        } else {
            RawEntry::Explicit(SnapState::Removed)
        };
        self.insert_raw(tick, entry);
        true
    }

    /// Mark `tick` unchanged on external proof that the authoritative state
    /// did not move there. Answers `false` (changing nothing) when `tick` is
    /// already anchored or no state precedes it.
    pub fn add_unchanged(&mut self, tick: u32) -> bool {
        if self.state_at(tick).is_some() || self.state_before(tick).is_none() {
            return false;
        }
        self.insert_raw(tick, RawEntry::SameAsPrecedent);
        true
    }

    /// Advance the newest present snapshot to `tick` from a monotonic
    /// completeness signal (for example the latest fully-acked server tick).
    /// Answers `None` (changing nothing) on an empty history, a removed
    /// newest state, or a tick at or below the newest anchor; otherwise the
    /// previous newest tick.
    pub fn push_unchanged(&mut self, tick: u32) -> Option<u32> {
        let newest = self.newest_tick()?;
        let present = self
            .resolve_at_index(self.buffer.len() - 1)
            .is_some_and(|state| matches!(state, SnapState::Present(_)));
        if tick <= newest || !present {
            return None;
        }
        self.add_unchanged(tick).then_some(newest)
    }

    /// Resolved present snapshot exactly at `tick`; `None` for removals,
    /// stateless markers and unanchored ticks. A marker resolves to the
    /// preceding explicit snapshot, which keeps its authored tick.
    pub fn get_at(&self, tick: u32) -> Option<&FullSnap> {
        let pos = self.buffer.partition_point(|(anchor, _)| *anchor < tick);
        let index = self
            .buffer
            .get(pos)
            .and_then(|(anchor, _)| (*anchor == tick).then_some(pos))?;
        self.resolve_at_index(index).and_then(|state| match state {
            SnapState::Present(snap) => Some(snap),
            SnapState::Removed => None,
        })
    }

    /// Resolved present snapshot at or before `tick`; `None` when the floor
    /// is a removal or no anchor precedes the tick. A marker floor resolves
    /// to the preceding explicit snapshot, which keeps its authored tick.
    pub fn get_at_or_before(&self, tick: u32) -> Option<&FullSnap> {
        self.state_at_or_before(tick).and_then(|state| match state {
            SnapState::Present(snap) => Some(snap),
            SnapState::Removed => None,
        })
    }

    /// True when the authoritative state at or before `tick` is a removal.
    pub fn is_removed_at_or_before(&self, tick: u32) -> bool {
        matches!(self.state_at_or_before(tick), Some(SnapState::Removed))
    }

    /// Whether the anchor at `tick` is a compressed marker: `Some(true)` for
    /// a marker, `Some(false)` for an explicit anchor, `None` when absent.
    pub fn is_unchanged_marker(&self, tick: u32) -> Option<bool> {
        let pos = self.buffer.partition_point(|(anchor, _)| *anchor < tick);
        self.buffer.get(pos).and_then(|(anchor, raw)| {
            (*anchor == tick).then_some(matches!(raw, RawEntry::SameAsPrecedent))
        })
    }

    /// Drop every anchor strictly older than `tick`, keeping the floor state
    /// as the new oldest anchor so at-or-before lookups still resolve.
    pub fn clear_until_tick(&mut self, tick: u32) {
        let floor = self.state_at_or_before(tick).cloned();
        let cut = self.buffer.partition_point(|(anchor, _)| *anchor < tick);
        if cut > 0 {
            self.buffer.drain(..cut);
        }
        if let Some(state) = floor {
            match self.buffer.front() {
                Some((front, _)) if *front == tick => self.materialize_front(state),
                Some((front, _)) if *front > tick => {
                    self.buffer.push_front((tick, RawEntry::Explicit(state)));
                }
                None => {
                    self.buffer.push_front((tick, RawEntry::Explicit(state)));
                }
                Some(_) => {}
            }
        }
    }

    /// Pop the oldest present snapshot; a following marker materializes to the
    /// popped state so the remaining anchors keep resolving. Removals, bare
    /// markers and an empty history pop to `None`.
    pub fn pop_oldest(&mut self) -> Option<(u32, FullSnap)> {
        let head = self.buffer.front().and_then(|(_, raw)| match raw {
            RawEntry::Explicit(state) => Some(state.clone()),
            RawEntry::SameAsPrecedent => None,
        });
        let popped = match self.buffer.pop_front() {
            Some((tick, RawEntry::Explicit(SnapState::Present(snap)))) => Some((tick, snap)),
            _ => None,
        };
        if let Some(state) = head {
            self.materialize_front(state);
        }
        popped
    }

    /// Drop every anchor.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Latest authoritative state at or before `tick`, markers resolved.
    fn state_at_or_before(&self, tick: u32) -> Option<&SnapState> {
        let index = self
            .buffer
            .partition_point(|(anchor, _)| *anchor <= tick)
            .checked_sub(1)?;
        self.resolve_at_index(index)
    }

    /// Latest authoritative state strictly before `tick`, markers resolved.
    fn state_before(&self, tick: u32) -> Option<&SnapState> {
        let index = self
            .buffer
            .partition_point(|(anchor, _)| *anchor < tick)
            .checked_sub(1)?;
        self.resolve_at_index(index)
    }

    /// Authoritative state exactly at `tick`, markers resolved.
    fn state_at(&self, tick: u32) -> Option<&SnapState> {
        let pos = self.buffer.partition_point(|(anchor, _)| *anchor < tick);
        let index = self
            .buffer
            .get(pos)
            .and_then(|(anchor, _)| (*anchor == tick).then_some(pos))?;
        self.resolve_at_index(index)
    }

    /// Walk back to the closest preceding explicit entry.
    fn resolve_at_index(&self, index: usize) -> Option<&SnapState> {
        (0..=index)
            .rev()
            .find_map(|i| match &self.buffer.get(i)?.1 {
                RawEntry::Explicit(state) => Some(state),
                RawEntry::SameAsPrecedent => None,
            })
    }

    /// Sorted insert (same-tick anchors replace); past the cap the oldest
    /// anchor evicts so the buffer stays bounded.
    fn insert_raw(&mut self, tick: u32, entry: RawEntry) {
        let pos = self.buffer.partition_point(|(anchor, _)| *anchor < tick);
        if self
            .buffer
            .get(pos)
            .is_some_and(|(anchor, _)| *anchor == tick)
        {
            self.buffer[pos] = (tick, entry);
        } else {
            self.buffer.insert(pos, (tick, entry));
        }
        while self.buffer.len() > self.cap {
            self.evict_oldest();
        }
    }

    /// Drop the oldest anchor; a following marker materializes to the dropped
    /// state so the remaining anchors keep resolving.
    fn evict_oldest(&mut self) {
        let head = self.buffer.front().and_then(|(_, raw)| match raw {
            RawEntry::Explicit(state) => Some(state.clone()),
            RawEntry::SameAsPrecedent => None,
        });
        self.buffer.pop_front();
        if let Some(state) = head {
            self.materialize_front(state);
        }
    }

    /// Turn a leading unchanged marker into the given explicit state.
    fn materialize_front(&mut self, state: SnapState) {
        if let Some((_, raw)) = self.buffer.front_mut() {
            if matches!(raw, RawEntry::SameAsPrecedent) {
                *raw = RawEntry::Explicit(state);
            }
        }
    }
}

impl Default for ConfirmedHistory {
    fn default() -> Self {
        Self::new(MAX_CONFIRMED_SAMPLES)
    }
}

/// Presentation-timeline sampler over [`ConfirmedHistory`].
///
/// The presentation cursor trails the newest confirmed tick by `delay` ticks
/// (the donor `InterpolationTimeline` objective: newest remote estimate minus
/// the interpolation delay), so remote snapshots render behind the jitter
/// window instead of at its edge. Integer ticks only: [`Self::advance`] snaps
/// the cursor to each newest tick the way a resync does, and [`Self::sample`]
/// answers the delayed query against the history. This is the one production
/// timeline sampler in `engine-net`: it borrows the history and holds no
/// snapshots itself. The delay clamps to `MIN_DELAY..=MAX_DELAY` from
/// [`crate::delay`], so the cursor never outruns the rollback window.
pub struct TimelineSampler {
    delay: u32,
    present: Option<u32>,
    synced: bool,
}

impl TimelineSampler {
    /// Open an unsynced sampler trailing by `delay` ticks, clamped to range.
    pub fn new(delay: u32) -> Self {
        Self {
            delay: clamp_delay(delay),
            present: None,
            synced: false,
        }
    }

    /// Trailing delay in ticks.
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Set the trailing delay, clamped to `MIN_DELAY..=MAX_DELAY`.
    pub fn set_delay(&mut self, delay: u32) {
        self.delay = clamp_delay(delay);
    }

    /// Presentation tick the sampler reads at, once synced.
    pub fn present_tick(&self) -> Option<u32> {
        self.present
    }

    /// True once [`Self::advance`] has run against a confirmed tick.
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// Drop the cursor back to unsynced; the configured delay is kept.
    pub fn reset(&mut self) {
        self.present = None;
        self.synced = false;
    }

    /// Move the cursor to `newest_tick` minus the delay and mark the sampler
    /// synced, returning the presentation tick.
    pub fn advance(&mut self, newest_tick: u32) -> u32 {
        let present = newest_tick.saturating_sub(self.delay);
        self.present = Some(present);
        self.synced = true;
        present
    }

    /// Sample the history at the presentation tick; `None` before the first
    /// [`Self::advance`], on an empty history, or across a removal floor.
    pub fn sample<'a>(&self, history: &'a ConfirmedHistory) -> Option<&'a FullSnap> {
        let present = self.present.filter(|_| self.synced)?;
        history.get_at_or_before(present)
    }

    /// Advance to `newest_tick` and sample in one call.
    pub fn sample_delayed<'a>(
        &mut self,
        history: &'a ConfirmedHistory,
        newest_tick: u32,
    ) -> Option<&'a FullSnap> {
        self.advance(newest_tick);
        self.sample(history)
    }
}

impl Default for TimelineSampler {
    fn default() -> Self {
        Self::new(DEFAULT_DELAY)
    }
}

/// Fixed two-fighter fixture for tests: slots 0 and 1, sorted by slot.
#[must_use]
pub fn sample_snapshot(tick: u32) -> FullSnap {
    FullSnap {
        tick,
        fighters: vec![
            FighterSnap {
                slot: 0,
                x_mm: 1000,
                y_mm: 0,
                z_mm: 2000,
                yaw_ddeg: 0,
                hp: 1000,
                stamina: 100,
            },
            FighterSnap {
                slot: 1,
                x_mm: -3000,
                y_mm: 0,
                z_mm: 4000,
                yaw_ddeg: -900,
                hp: 880,
                stamina: 60,
            },
        ],
    }
}

/// Quaternion blend switch: below this angle the cheap normalized lerp is
/// indistinguishable from slerp, at or above it slerp keeps angular velocity
/// constant. Donor `interpolate.rs` shape, tuned to the same 1.8 degrees.
pub const QUAT_NLERP_MAX_DEG: f32 = 1.8;

/// Bounded extrapolation horizon in ticks past the newest anchor: queries
/// past `newest + horizon` refuse with `None` so the caller holds its last
/// frame instead of flying blind.
pub const MAX_EXTRAPOLATE_TICKS: Fixed = Fixed::from_int(2);

/// Render-time query class from the extrapolate guard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryKind {
    /// The query lands on or between buffered anchors: blended or exact.
    Interpolate,
    /// The query leaves the buffered range but stays inside the horizon:
    /// the returned state clamps to the nearest edge anchor, unblended.
    Extrapolate,
}

/// Which quaternion path a yaw blend takes; see [`quat_path_for`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuatPath {
    /// Small angle: normalized lerp of the quaternion components.
    Nlerp,
    /// Wide angle: spherical lerp at constant angular velocity.
    Slerp,
}

/// Yaw-only unit quaternion for presentation blends: rotation about +Y,
/// right-handed with +Y up, matching the render stage convention.
/// Components are deterministic [`Fixed`] (LUT trig plus CORDIC atan2), so
/// the same yaws blend to the same decidegrees on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RenderQuat {
    /// X component (always zero for yaw-only rotations).
    pub x: Fixed,
    /// Y component (`sin(yaw / 2)`).
    pub y: Fixed,
    /// Z component (always zero for yaw-only rotations).
    pub z: Fixed,
    /// W component (`cos(yaw / 2)`).
    pub w: Fixed,
}

/// Half-angle of `yaw_ddeg` in radians as [`Fixed`]: `ddeg * PI / 3600`.
fn half_angle_rad(yaw_ddeg: i16) -> Fixed {
    Fixed::from_int(i64::from(crate::delta::wrap_decideg(i32::from(yaw_ddeg)))) * Fixed::PI
        / Fixed::from_int(3600)
}

impl RenderQuat {
    /// Build the yaw-only quaternion for `yaw_ddeg` decidegrees.
    #[must_use]
    pub fn from_yaw_ddeg(yaw_ddeg: i16) -> Self {
        let (s, c) = half_angle_rad(yaw_ddeg).sin_cos();
        Self {
            x: Fixed::ZERO,
            y: s,
            z: Fixed::ZERO,
            w: c,
        }
    }

    /// Shortest-arc angle between two yaws in degrees, exact for yaw-only
    /// rotations without any inverse trig.
    #[must_use]
    pub fn angle_deg(a_ddeg: i16, b_ddeg: i16) -> f32 {
        let diff = crate::delta::wrap_decideg(i32::from(a_ddeg) - i32::from(b_ddeg));
        (f32::from(diff).abs()) / 10.0
    }

    /// `b` or its negation, whichever lies in `a`'s hemisphere: `q` and `-q`
    /// are the same yaw, and blending toward the far one runs the long way
    /// round (a yaw wrapping across 180 degrees would spin backwards).
    fn aligned(a: Self, b: Self) -> Self {
        if a.y * b.y + a.w * b.w < Fixed::ZERO {
            Self {
                x: Fixed::ZERO - b.x,
                y: Fixed::ZERO - b.y,
                z: Fixed::ZERO - b.z,
                w: Fixed::ZERO - b.w,
            }
        } else {
            b
        }
    }

    /// Normalized lerp: component blend renormalized to unit length.
    /// `t` clamps to `0..=1`; a zero length falls back to `a`.
    #[must_use]
    pub fn nlerp(a: Self, b: Self, t: Fixed) -> Self {
        let b = Self::aligned(a, b);
        let t = t.clamp(Fixed::ZERO, Fixed::ONE);
        let y = a.y + (b.y - a.y) * t;
        let w = a.w + (b.w - a.w) * t;
        let len = (y * y + w * w).sqrt();
        if len == Fixed::ZERO {
            a
        } else {
            Self {
                x: Fixed::ZERO,
                y: y / len,
                z: Fixed::ZERO,
                w: w / len,
            }
        }
    }

    /// Spherical lerp at constant angular velocity over the wrapped yaw
    /// arc. `t` clamps to `0..=1`; a zero arc falls back to
    /// [`Self::nlerp`] so tiny angles never divide by a zero sine.
    #[must_use]
    pub fn slerp(a: Self, b: Self, t: Fixed) -> Self {
        let b = Self::aligned(a, b);
        let t = t.clamp(Fixed::ZERO, Fixed::ONE);
        let diff = i32::from(b.to_yaw_ddeg()) - i32::from(a.to_yaw_ddeg());
        let theta = Fixed::from_int(i64::from(crate::delta::wrap_decideg(diff))) * Fixed::PI
            / Fixed::from_int(3600);
        let sin_theta = theta.sin();
        if sin_theta == Fixed::ZERO {
            return Self::nlerp(a, b, t);
        }
        let wa = ((Fixed::ONE - t) * theta).sin() / sin_theta;
        let wb = (t * theta).sin() / sin_theta;
        // Yaw-only inputs stay yaw-only: x/z blend to zero, y/w carry it.
        Self {
            x: Fixed::ZERO,
            y: a.y * wa + b.y * wb,
            z: Fixed::ZERO,
            w: a.w * wa + b.w * wb,
        }
    }

    /// Blend two yaws, dispatching on the shortest-arc angle: [`QuatPath::Nlerp`]
    /// below [`QUAT_NLERP_MAX_DEG`], [`QuatPath::Slerp`] at or above it.
    #[must_use]
    pub fn blend(a: Self, b: Self, t: Fixed) -> Self {
        match quat_path_for_yaws(a, b) {
            QuatPath::Nlerp => Self::nlerp(a, b, t),
            QuatPath::Slerp => Self::slerp(a, b, t),
        }
    }

    /// Back to integer decidegrees wrapped to `[-1800, 1800)`, rounded half
    /// away from zero through deterministic [`Fixed`] math.
    #[must_use]
    pub fn to_yaw_ddeg(self) -> i16 {
        let yaw_rad = Fixed::atan2(self.y, self.w) * Fixed::from_int(2);
        let raw = (yaw_rad * Fixed::from_int(1800) / Fixed::PI).to_bits();
        let half = Fixed::SCALE / 2;
        let rounded = if raw >= 0 { raw + half } else { raw - half } / Fixed::SCALE;
        let clamped = rounded.clamp(i64::from(i16::MIN), i64::from(i16::MAX));
        crate::delta::wrap_decideg(i32::try_from(clamped).unwrap_or(0))
    }
}

/// Dispatch rule for a raw angle: [`QuatPath::Nlerp`] strictly below
/// [`QUAT_NLERP_MAX_DEG`], [`QuatPath::Slerp`] at or above it.
/// Non-finite angles take the slerp path (they never occur from real yaws).
#[must_use]
pub fn quat_path_for(angle_deg: f32) -> QuatPath {
    if angle_deg.is_finite() && angle_deg < QUAT_NLERP_MAX_DEG {
        QuatPath::Nlerp
    } else {
        QuatPath::Slerp
    }
}

/// Dispatch rule for two yaw quaternions via their shortest-arc angle.
#[must_use]
fn quat_path_for_yaws(a: RenderQuat, b: RenderQuat) -> QuatPath {
    let a_deg = a.to_yaw_ddeg();
    let b_deg = b.to_yaw_ddeg();
    quat_path_for(RenderQuat::angle_deg(a_deg, b_deg))
}

/// Cubic Hermite blend of one scalar channel in [`Fixed` millimeters:
/// endpoints `p0`/`p1` with tangents `m0`/`m1` (mm per tick) over span `h`
/// ticks at `s in 0..=1`. Out-of-range `s` clamps; a non-positive span
/// holds `p0`. Integer-only, so every machine blends identical bits.
#[must_use]
pub fn hermite_scalar(p0: Fixed, p1: Fixed, m0: Fixed, m1: Fixed, h: Fixed, s: Fixed) -> Fixed {
    if h <= Fixed::ZERO {
        return p0;
    }
    let s = s.clamp(Fixed::ZERO, Fixed::ONE);
    let s2 = s * s;
    let s3 = s2 * s;
    let two = Fixed::from_int(2);
    let three = Fixed::from_int(3);
    let h00 = two * s3 - three * s2 + Fixed::ONE;
    let h10 = s3 - two * s2 + s;
    let h01 = Fixed::ZERO - two * s3 + three * s2;
    let h11 = s3 - s2;
    h00 * p0 + h10 * h * m0 + h01 * p1 + h11 * h * m1
}

/// Catmull-Rom tangent at `points[i]` in mm per tick: centered difference
/// over the neighbor ticks, one-sided at the edges. Shared tangents at each
/// interior knot are what keep multi-span blends C1-continuous.
pub(crate) fn tangent_at(points: &[(u32, Fixed)], i: usize) -> Fixed {
    let n = points.len();
    if n < 2 {
        return Fixed::ZERO;
    }
    let span = |a: u32, b: u32| Fixed::from_int(i64::from(a.saturating_sub(b).max(1)));
    if i == 0 {
        (points[1].1 - points[0].1) / span(points[1].0, points[0].0)
    } else if i + 1 >= n {
        (points[n - 1].1 - points[n - 2].1) / span(points[n - 1].0, points[n - 2].0)
    } else {
        (points[i + 1].1 - points[i - 1].1) / span(points[i + 1].0, points[i - 1].0)
    }
}

/// Extrapolate guard over one buffered tick range: `Interpolate` inside
/// `oldest..=newest`, `Extrapolate` within [`MAX_EXTRAPOLATE_TICKS`] past
/// either edge, and `None` (refuse) beyond that horizon.
#[must_use]
pub fn classify_query(oldest: u32, newest: u32, t: Fixed) -> Option<QueryKind> {
    let oldest = Fixed::from_int(i64::from(oldest));
    let newest = Fixed::from_int(i64::from(newest));
    if t < oldest || t > newest {
        if t < oldest - MAX_EXTRAPOLATE_TICKS || t > newest + MAX_EXTRAPOLATE_TICKS {
            None
        } else {
            Some(QueryKind::Extrapolate)
        }
    } else {
        Some(QueryKind::Interpolate)
    }
}

/// Snapshots sent past the acked baseline before the sender stops trusting it
/// and sends full: 30 ticks of silence (the starve bar) at 10 Hz.
pub const ACK_STALE_SNAPSHOTS: usize =
    (crate::interest::STARVE_TICKS / crate::interest::SNAPSHOT_EVERY_TICKS) as usize;

/// One wire-ready snapshot packet from [`SnapshotSender::encode`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SnapshotPacket {
    /// State-delta datagram against the acked baseline ([`encode_state_delta`]).
    Delta(Vec<u8>),
    /// Full fighter snapshot ([`encode_snapshot`]): needs no baseline.
    Full(Vec<u8>),
}

/// Per-client snapshot sender: codes a delta only against a snapshot the
/// receiver acknowledged, and falls back to a full snapshot when there is
/// no ack yet, [`ACK_STALE_SNAPSHOTS`] snapshots went unacknowledged, or the
/// roster changed. A delta is never coded against an unacknowledged baseline,
/// so a lost ack costs bytes, never a wrong rebuild.
#[derive(Default)]
pub struct SnapshotSender {
    acked: Option<FullSnap>,
    sent: VecDeque<FullSnap>,
}

impl SnapshotSender {
    /// A sender with no baseline: the first packet is full.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tick of the acknowledged baseline, if any.
    #[must_use]
    pub fn baseline_tick(&self) -> Option<u32> {
        self.acked.as_ref().map(|base| base.tick)
    }

    /// Encode `full` (checkpoint hash `hash`) as a delta or full packet and
    /// remember it so a later [`Self::ack`] can promote it to the baseline.
    ///
    /// # Errors
    ///
    /// [`NetError`] when `full` does not validate or a packet exceeds
    /// [`MAX_PACKET_BYTES`](crate::session::MAX_PACKET_BYTES).
    pub fn encode(&mut self, full: &FullSnap, hash: u64) -> Result<SnapshotPacket, NetError> {
        let delta = self.acked.as_ref().and_then(|base| {
            let stale = self.sent.iter().filter(|s| s.tick > base.tick).count();
            (stale < ACK_STALE_SNAPSHOTS)
                .then(|| encode_delta(full, base).ok())
                .flatten()
        });
        let packet = match delta {
            Some(delta) => {
                let checkpoint = Checkpoint {
                    tick: full.tick,
                    hash,
                };
                SnapshotPacket::Delta(encode_state_delta(checkpoint, &delta)?)
            }
            None => SnapshotPacket::Full(encode_snapshot(full)?),
        };
        self.sent.retain(|s| s.tick != full.tick);
        self.sent.push_back(full.clone());
        while self.sent.len() > 2 * ACK_STALE_SNAPSHOTS {
            self.sent.pop_front();
        }
        Ok(packet)
    }

    /// The receiver confirmed the snapshot of `tick`: it becomes the delta
    /// baseline. Returns `false` (and changes nothing) for a tick that is
    /// unknown, evicted, or not newer than the current baseline.
    pub fn ack(&mut self, tick: u32) -> bool {
        if self.baseline_tick().is_some_and(|base| tick <= base) {
            return false;
        }
        let Some(snap) = self.sent.iter().find(|s| s.tick == tick) else {
            return false;
        };
        self.acked = Some(snap.clone());
        self.sent.retain(|s| s.tick >= tick);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved_snap(tick: u32) -> FullSnap {
        let mut snap = sample_snapshot(tick);
        for fighter in &mut snap.fighters {
            fighter.x_mm += tick as i32 * 10;
        }
        snap
    }

    #[test]
    fn sender_falls_back_to_full_when_no_ack_for_ten_snapshots() {
        let mut sender = SnapshotSender::new();
        let first = moved_snap(100);
        // No ack yet: nothing to delta against, so the first packet is full.
        let SnapshotPacket::Full(bytes) = sender.encode(&first, 1).expect("first") else {
            panic!("a sender with no acked baseline must send full");
        };
        assert_eq!(bytes, encode_snapshot(&first).expect("full"));
        assert!(!sender.ack(999), "an ack for a tick never sent is ignored");
        assert!(sender.ack(100));
        assert!(!sender.ack(100), "a repeated ack changes nothing");
        // Ten snapshots go out and every ack is lost.
        for k in 1..=10u32 {
            let snap = moved_snap(100 + 6 * k);
            let packet = sender.encode(&snap, u64::from(k)).expect("encode");
            match packet {
                SnapshotPacket::Delta(bytes) => {
                    assert!(
                        k as usize <= ACK_STALE_SNAPSHOTS,
                        "snapshot {k}: stale delta"
                    );
                    let delta = crate::session::decode_state(&bytes)
                        .expect("delta")
                        .delta
                        .expect("delta body");
                    assert_eq!(delta.base_tick, 100, "only the acked baseline is used");
                    assert_eq!(
                        crate::delta::decode_delta(&delta, &first).expect("rebuild"),
                        snap
                    );
                    assert!(bytes.len() < encode_snapshot(&snap).expect("full").len());
                }
                SnapshotPacket::Full(bytes) => {
                    assert!(
                        k as usize > ACK_STALE_SNAPSHOTS,
                        "snapshot {k}: fell back early"
                    );
                    assert_eq!(bytes, encode_snapshot(&snap).expect("full"));
                }
            }
        }
        // The late ack of a full snapshot re-bases the next delta on it.
        assert!(sender.ack(160));
        assert_eq!(sender.baseline_tick(), Some(160));
        let snap = moved_snap(166);
        let SnapshotPacket::Delta(bytes) = sender.encode(&snap, 11).expect("rebased") else {
            panic!("an acked baseline allows a delta again");
        };
        assert_eq!(
            crate::session::decode_state(&bytes)
                .expect("state")
                .delta
                .expect("delta")
                .base_tick,
            160
        );
        assert!(bytes.len() <= crate::session::MAX_PACKET_BYTES);
    }

    #[test]
    fn sender_sends_full_when_the_roster_changes() {
        let mut sender = SnapshotSender::new();
        sender.encode(&sample_snapshot(10), 1).expect("first");
        assert!(sender.ack(10));
        let mut fewer = sample_snapshot(16);
        fewer.fighters.pop();
        assert!(matches!(
            sender.encode(&fewer, 2).expect("encode"),
            SnapshotPacket::Full(_)
        ));
    }

    #[test]
    fn roundtrip_equals() {
        let full = sample_snapshot(12);
        let bytes = save_snapshot(&full).expect("save");
        let back = load_snapshot(&bytes).expect("load");
        assert_eq!(back, full);
        let again = save_snapshot(&back).expect("resave");
        assert_eq!(again, bytes);
    }

    #[test]
    fn file_roundtrip_equals() {
        let full = sample_snapshot(7);
        let path = std::env::temp_dir().join(format!(
            "engine-net-snapshot-{}-{}.bin",
            std::process::id(),
            full.tick
        ));
        save_to_file(&full, &path).expect("save file");
        let back = load_from_file(&path).expect("load file");
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, full);
    }

    #[test]
    fn corrupt_bytes_refuse() {
        let full = sample_snapshot(3);
        let mut bytes = save_snapshot(&full).expect("save");
        assert!(!bytes.is_empty());
        bytes.pop();
        assert!(load_snapshot(&bytes).is_err());
        assert!(load_snapshot(&[]).is_err());
    }

    fn moved_snapshot(tick: u32, x_mm: i32) -> FullSnap {
        let mut snap = sample_snapshot(tick);
        snap.fighters[0].x_mm = x_mm;
        snap
    }

    #[test]
    fn equal_snapshots_compress_to_marker() {
        let mut history = ConfirmedHistory::new(8);
        assert!(history.insert(1, sample_snapshot(1)));
        assert_eq!(history.is_unchanged_marker(1), Some(false));
        // Same fighters under a new tick key: the anchor compresses and
        // resolves to the authored tick-1 snapshot.
        assert!(history.insert(2, sample_snapshot(2)));
        assert_eq!(history.is_unchanged_marker(2), Some(true));
        assert_eq!(history.get_at_or_before(2), Some(&sample_snapshot(1)));
        // A moved snapshot stays explicit.
        assert!(history.insert(3, moved_snapshot(3, 4242)));
        assert_eq!(history.is_unchanged_marker(3), Some(false));
        assert_eq!(history.get_at(3).expect("moved").fighters[0].x_mm, 4242);
    }

    #[test]
    fn late_explicit_rewrites_unchanged_resolution() {
        let mut history = ConfirmedHistory::new(8);
        assert!(history.insert(1, sample_snapshot(1)));
        assert!(history.add_unchanged(7));
        assert_eq!(history.get_at_or_before(7), Some(&sample_snapshot(1)));
        // A delayed authoritative insert before the marker rewrites it: the
        // floor resolves to the tick-5 snapshot from here on.
        assert!(history.insert(5, moved_snapshot(5, 7777)));
        assert_eq!(history.is_unchanged_marker(5), Some(false));
        assert_eq!(
            history.get_at_or_before(7).expect("late").fighters[0].x_mm,
            7777
        );
        assert_eq!(history.get_at(5).expect("exact").fighters[0].x_mm, 7777);
    }

    #[test]
    fn sampler_trails_newest_by_delay() {
        let mut history = ConfirmedHistory::new(8);
        for tick in 10..=14 {
            assert!(history.insert(tick, moved_snapshot(tick, 1000 + tick as i32)));
        }
        let mut sampler = TimelineSampler::new(2);
        assert!(!sampler.is_synced());
        assert_eq!(sampler.sample(&history), None);
        assert_eq!(sampler.advance(14), 12);
        assert!(sampler.is_synced());
        assert_eq!(sampler.present_tick(), Some(12));
        let state = sampler.sample(&history).expect("delayed sample");
        assert_eq!(state.fighters[0].x_mm, 1012);
        // The one-call path agrees, and a fresh sampler starts unsynced.
        assert_eq!(
            sampler
                .sample_delayed(&history, 14)
                .expect("delayed")
                .fighters[0]
                .x_mm,
            1012
        );
        let mut fresh = TimelineSampler::default();
        assert_eq!(fresh.delay(), DEFAULT_DELAY);
        assert_eq!(fresh.sample(&history), None);
        fresh.set_delay(99);
        assert_eq!(fresh.delay(), crate::delay::MAX_DELAY);
    }

    #[test]
    fn removal_hides_present_until_next_present() {
        let mut history = ConfirmedHistory::new(8);
        assert!(history.insert(1, sample_snapshot(1)));
        assert!(history.insert_removed(3));
        assert_eq!(history.get_at_or_before(1), Some(&sample_snapshot(1)));
        assert_eq!(history.get_at_or_before(3), None);
        assert!(history.is_removed_at_or_before(3));
        assert!(!history.is_removed_at_or_before(1));
        assert!(history.insert(5, moved_snapshot(5, 5150)));
        assert!(!history.is_removed_at_or_before(5));
        assert_eq!(
            history.get_at_or_before(5).expect("next").fighters[0].x_mm,
            5150
        );
    }

    #[test]
    fn invalid_snapshots_reject_without_change() {
        let mut history = ConfirmedHistory::new(8);
        let bad = FullSnap {
            tick: 9,
            fighters: Vec::new(),
        };
        assert!(!history.insert(9, bad.clone()));
        assert!(!history.insert_explicit(9, bad));
        assert!(history.is_empty());
        assert_eq!(history.is_unchanged_marker(9), None);
    }

    #[test]
    fn cap_evicts_oldest_and_floor_survives_clear() {
        let mut history = ConfirmedHistory::new(2);
        assert_eq!(history.cap(), 2);
        for tick in 1..=3 {
            assert!(history.insert(tick, moved_snapshot(tick, 100 * tick as i32)));
        }
        assert_eq!(history.len(), 2);
        assert_eq!(
            (history.oldest_tick(), history.newest_tick()),
            (Some(2), Some(3))
        );
        assert_eq!(history.get_at_or_before(1), None);
        history.clear_until_tick(3);
        assert_eq!(history.len(), 1);
        assert_eq!(history.get_at(3).expect("kept").fighters[0].x_mm, 300);
        // Clearing past the newest keeps the floor state as the new anchor.
        history.clear_until_tick(9);
        assert_eq!(history.oldest_tick(), Some(9));
        assert_eq!(
            history.get_at_or_before(9).expect("floor").fighters[0].x_mm,
            300
        );
        assert_eq!(history.get_at_or_before(8), None);
    }

    #[test]
    fn push_unchanged_needs_monotonic_present() {
        let mut history = ConfirmedHistory::new(8);
        assert_eq!(history.push_unchanged(4), None);
        assert!(history.insert(4, sample_snapshot(4)));
        assert_eq!(history.push_unchanged(4), None);
        assert_eq!(history.push_unchanged(6), Some(4));
        assert_eq!(history.is_unchanged_marker(6), Some(true));
        assert_eq!(history.get_at_or_before(6), Some(&sample_snapshot(4)));
        assert!(history.insert_removed(8));
        assert_eq!(history.push_unchanged(9), None);
    }

    #[test]
    fn pop_oldest_materializes_next_marker() {
        let mut history = ConfirmedHistory::new(8);
        assert!(history.insert(1, sample_snapshot(1)));
        assert!(history.insert(2, sample_snapshot(2)));
        assert_eq!(history.is_unchanged_marker(2), Some(true));
        let (tick, snap) = history.pop_oldest().expect("oldest");
        assert_eq!(tick, 1);
        assert_eq!(snap.fighters, sample_snapshot(1).fighters);
        // The marker behind the popped anchor is now explicit.
        assert_eq!(history.is_unchanged_marker(2), Some(false));
        assert_eq!(history.get_at(2).expect("kept").fighters, snap.fighters);
        let (tick, _) = history.pop_oldest().expect("second");
        assert_eq!(tick, 2);
        assert_eq!(history.pop_oldest(), None);
    }

    #[test]
    fn snapshot_quat_dispatches_nlerp_below_1_8_degrees() {
        assert_eq!(quat_path_for(0.0), QuatPath::Nlerp);
        assert_eq!(quat_path_for(1.7), QuatPath::Nlerp);
        assert_eq!(quat_path_for(1.799), QuatPath::Nlerp);
        assert_eq!(quat_path_for(1.8), QuatPath::Slerp);
        assert_eq!(quat_path_for(90.0), QuatPath::Slerp);
        assert_eq!(quat_path_for(f32::NAN), QuatPath::Slerp);
        // Small-angle nlerp matches slerp to a tenth of a decidegree.
        let a = RenderQuat::from_yaw_ddeg(0);
        let b = RenderQuat::from_yaw_ddeg(15);
        let mid_n = RenderQuat::nlerp(a, b, Fixed::HALF).to_yaw_ddeg();
        let mid_s = RenderQuat::slerp(a, b, Fixed::HALF).to_yaw_ddeg();
        assert!((i32::from(mid_n) - i32::from(mid_s)).abs() <= 1);
        // Wide-angle slerp midpoint lands on the 45-degree mark.
        let wide = RenderQuat::slerp(
            RenderQuat::from_yaw_ddeg(0),
            RenderQuat::from_yaw_ddeg(900),
            Fixed::HALF,
        )
        .to_yaw_ddeg();
        assert!((i32::from(wide) - 450).abs() <= 2, "slerp mid {wide}");
        // Yaw roundtrips through the quaternion within one decidegree.
        for yaw in [-1799, -900, -15, 0, 7, 450, 1799] {
            let back = RenderQuat::from_yaw_ddeg(yaw).to_yaw_ddeg();
            assert!(
                (i32::from(back) - i32::from(yaw)).abs() <= 1,
                "yaw {yaw} gave {back}"
            );
        }
    }

    #[test]
    fn snapshot_guard_classify_query_edges() {
        let frac = |num: i64, den: i64| Fixed::from_int(num) / Fixed::from_int(den);
        assert_eq!(
            classify_query(2, 6, Fixed::from_int(2)),
            Some(QueryKind::Interpolate)
        );
        assert_eq!(
            classify_query(2, 6, Fixed::from_int(6)),
            Some(QueryKind::Interpolate)
        );
        assert_eq!(
            classify_query(2, 6, frac(9, 2)),
            Some(QueryKind::Interpolate)
        );
        assert_eq!(
            classify_query(2, 6, Fixed::from_int(8)),
            Some(QueryKind::Extrapolate)
        );
        assert_eq!(
            classify_query(2, 6, Fixed::ZERO),
            Some(QueryKind::Extrapolate)
        );
        assert_eq!(classify_query(2, 6, frac(17, 2)), None);
        assert_eq!(classify_query(2, 6, frac(-1, 2)), None);
    }
}
