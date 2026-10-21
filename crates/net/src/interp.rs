//! interp: presentation-time snapshot interpolation for remote fighters.
//!
//! Hermite C1 position blends, quaternion yaw blends with an nlerp/slerp
//! auto-dispatch, a bounded snapshot ring, and a guard that classifies each
//! render-time query as interpolation or extrapolation. Render path only:
//! ring keys and snapshot ticks stay integer (`u32`), blended floats never
//! enter the simulation hash, and nothing here runs on the fixed tick.
//!
//! PORT(cBournhonesque/lightyear@96ded4ed: crates/replication/interpolation/
//! src/interpolate.rs@89d9f26c plus timeline.rs@9ba03390) MIT OR Apache-2.0,
//! shape-only, zero donor bytes: Hermite positions, quaternion dispatch and
//! the snapshot ring plus extrapolate guard are reimplemented std-only over
//! [`FullSnap`](crate::delta::FullSnap) and integer ticks.
//!
//! Consumer: [`RemoteView`] is the LAN guest's remote presentation
//! (`engine-game` `NetGame::remote_frame`): one snapshot stream feeds the
//! frozen tick-clock `Interpolator`, the [`ConfirmedHistory`] that also
//! stores delta baselines, and [`SnapshotInterpolator`], which answers
//! render-time queries the multiplayer app renders.
//! Data owner: the server owns authoritative snapshots; the ring only copies
//! validated bytes. Units: millimeters, decidegrees, integer ticks for keys,
//! [`Fixed`] render alpha for sub-tick queries. Determinism scope: positions
//! blend with integer-only [`Fixed`] math (LUT trig plus CORDIC atan2 for
//! yaw) over integer ticks, so the same ring plus the same query gives the
//! same presentation bytes on every machine; presentation output still never
//! enters the simulation hash, and nothing here runs on the fixed tick.
//! Error type: [`NetError`](crate::session::NetError); invalid snapshots
//! reject loudly and change nothing, never panic. Budget: at most
//! [`MAX_INTERP_RING`] anchors of bounded slots each; one bounded
//! [`FullSnap`] clone per query (render-time only, never per simulation tick).

use std::collections::VecDeque;

use engine_core::fixed::Fixed;

use crate::delta::{decode_delta, BodySnap, Delta, FullSnap, InterpResult, Interpolator};
use crate::session::NetError;
use crate::snapshot::{classify_query, tangent_at, ConfirmedHistory, TimelineSampler};
pub use crate::snapshot::{
    hermite_scalar, quat_path_for, QuatPath, QueryKind, RenderQuat, MAX_EXTRAPOLATE_TICKS,
    QUAT_NLERP_MAX_DEG,
};

/// Upper bound on ring anchors per interpolator: 32 snapshots of at most 16
/// slots each, so one ring costs at most a few tens of KB.
pub const MAX_INTERP_RING: usize = 32;

/// Bounded ring of timestamped snapshots, oldest to newest by tick.
/// Same-tick pushes replace; past the cap the oldest anchor evicts.
pub struct InterpRing {
    cap: usize,
    buffer: VecDeque<(u32, FullSnap)>,
}

impl InterpRing {
    /// Open a ring holding at most `cap` anchors, clamped to
    /// `2..=MAX_INTERP_RING` so the buffer stays bounded.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(2, MAX_INTERP_RING),
            buffer: VecDeque::new(),
        }
    }

    /// Anchor bound from [`Self::new`].
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Buffered anchor count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True when no anchor arrived yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Oldest buffered tick.
    #[must_use]
    pub fn oldest_tick(&self) -> Option<u32> {
        self.buffer.front().map(|(tick, _)| *tick)
    }

    /// Newest buffered tick.
    #[must_use]
    pub fn newest_tick(&self) -> Option<u32> {
        self.buffer.back().map(|(tick, _)| *tick)
    }

    /// Store one snapshot; invalid snapshots reject with `false` and change
    /// nothing. Same-tick pushes replace the stored anchor.
    pub fn push(&mut self, tick: u32, snap: FullSnap) -> bool {
        if snap.validate().is_err() {
            return false;
        }
        let pos = self.buffer.iter().position(|(anchor, _)| *anchor >= tick);
        match pos {
            Some(index) if self.buffer[index].0 == tick => {
                self.buffer[index] = (tick, snap);
            }
            Some(index) => self.buffer.insert(index, (tick, snap)),
            None => self.buffer.push_back((tick, snap)),
        }
        while self.buffer.len() > self.cap {
            self.buffer.pop_front();
        }
        true
    }

    /// Bracket indices around [`Fixed`] tick `t`: `(lo, hi)` with
    /// `ticks[lo] <= t <= ticks[hi]`, or the nearest edge index twice when
    /// `t` leaves the range. `None` only on an empty ring.
    fn brackets(&self, t: Fixed) -> Option<(usize, usize)> {
        if self.buffer.is_empty() {
            return None;
        }
        let tick = |index: usize| Fixed::from_int(i64::from(self.buffer[index].0));
        let n = self.buffer.len();
        if t <= tick(0) {
            return Some((0, 0));
        }
        if t >= tick(n - 1) {
            return Some((n - 1, n - 1));
        }
        let upper = self
            .buffer
            .iter()
            .position(|(anchor, _)| Fixed::from_int(i64::from(*anchor)) > t)
            .unwrap_or(n - 1);
        Some((upper - 1, upper))
    }

    /// `(tick, position)` of `slot` on `axis` at ring index `index`, when
    /// that anchor holds the slot.
    fn point(&self, index: usize, slot: u8, axis: Axis) -> Option<(u32, Fixed)> {
        let (tick, snap) = self.buffer.get(index)?;
        let fighter = snap.fighters.iter().find(|f| f.slot == slot)?;
        Some((*tick, axis.of(fighter)))
    }

    /// Nearest anchor before ring index `before` that holds `slot`.
    fn prev_point(&self, before: usize, slot: u8, axis: Axis) -> Option<(u32, Fixed)> {
        (0..before)
            .rev()
            .find_map(|index| self.point(index, slot, axis))
    }

    /// Nearest anchor after ring index `after` that holds `slot`.
    fn next_point(&self, after: usize, slot: u8, axis: Axis) -> Option<(u32, Fixed)> {
        (after + 1..self.buffer.len()).find_map(|index| self.point(index, slot, axis))
    }
}

/// Position axis selector for [`InterpRing::point`].
#[derive(Clone, Copy)]
enum Axis {
    /// Feet x in millimeters.
    X,
    /// Feet y in millimeters.
    Y,
    /// Feet z in millimeters.
    Z,
}

impl Axis {
    /// Read this axis from a fighter snapshot as [`Fixed`] millimeters.
    fn of(self, fighter: &BodySnap) -> Fixed {
        let mm = match self {
            Self::X => fighter.x_mm,
            Self::Y => fighter.y_mm,
            Self::Z => fighter.z_mm,
        };
        Fixed::from_int(i64::from(mm))
    }
}

impl Default for InterpRing {
    fn default() -> Self {
        Self::new(MAX_INTERP_RING)
    }
}

/// Presentation answer: the blended (or edge-clamped) snapshot plus the
/// guard's classification. `state.tick` names the floor anchor tick;
/// blended floats live only in the position/yaw fields.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InterpFrame {
    /// Presentation snapshot for the render path (never hashed).
    pub state: FullSnap,
    /// How the guard classified the query.
    pub kind: QueryKind,
}

/// Header of a frame sampled into a caller buffer: the floor anchor tick and
/// the guard's classification.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameHead {
    /// Floor anchor tick.
    pub tick: u32,
    /// How the guard classified the query.
    pub kind: QueryKind,
}

/// Presentation interpolator over an [`InterpRing`]: Hermite C1 positions,
/// quaternion-dispatched yaw, stepped combat state, guarded extrapolation.
pub struct SnapshotInterpolator {
    ring: InterpRing,
}

impl SnapshotInterpolator {
    /// Open an interpolator with a ring of at most `cap` anchors.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            ring: InterpRing::new(cap),
        }
    }

    /// Buffered anchor count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when no anchor arrived yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Store one snapshot; invalid snapshots reject with `false`.
    pub fn push(&mut self, tick: u32, snap: FullSnap) -> bool {
        self.ring.push(tick, snap)
    }

    /// Copy every anchor from a [`ConfirmedHistory`] into the ring, oldest
    /// to newest. Returns the anchor count copied. This is the named-consumer
    /// wiring: the history owns authoritative snapshots, the interpolator
    /// only borrows them for presentation.
    pub fn feed_history(&mut self, history: &ConfirmedHistory) -> usize {
        let (Some(oldest), Some(newest)) = (history.oldest_tick(), history.newest_tick()) else {
            return 0;
        };
        let mut copied = 0;
        let mut tick = oldest;
        while tick <= newest {
            if let Some(snap) = history.get_at(tick) {
                if self.ring.push(tick, snap.clone()) {
                    copied += 1;
                }
            }
            if tick == newest {
                break;
            }
            tick = tick.saturating_add(1);
        }
        copied
    }

    /// Classify [`Fixed`] tick `t` without blending: `None` on an empty ring
    /// or past `newest + MAX_EXTRAPOLATE_TICKS`.
    #[must_use]
    pub fn classify(&self, t: Fixed) -> Option<QueryKind> {
        let oldest = self.ring.oldest_tick()?;
        let newest = self.ring.newest_tick()?;
        classify_query(oldest, newest, t)
    }

    /// Answer the render-time query at [`Fixed`] tick `t`: exact anchors
    /// return as-is, interior queries Hermite-blend positions and
    /// quaternion-blend yaw, edge queries clamp with
    /// [`QueryKind::Extrapolate`]. `None` on an empty ring or past the
    /// horizon. Combat state (hp/stamina) steps from the floor anchor, never
    /// blends. Callers build `t` from integers: an exact tick is
    /// `Fixed::from_int(tick)`, and a millisecond render clock maps through
    /// `Fixed::from_int(ms) * Fixed::from_int(TICK_HZ) / Fixed::from_int(1000)`
    /// with no float casts anywhere. Allocates the answer; the per-frame path
    /// is [`Self::sample_into`].
    #[must_use]
    pub fn sample_at(&self, t: Fixed) -> Option<InterpFrame> {
        let mut fighters = Vec::new();
        let head = self.sample_into(t, &mut fighters)?;
        Some(InterpFrame {
            state: FullSnap {
                tick: head.tick,
                fighters,
            },
            kind: head.kind,
        })
    }

    /// [`Self::sample_at`] into a caller buffer: `out` is cleared and refilled,
    /// so a buffer sized once makes every later query allocation-free. Returns
    /// the floor tick and the guard's class.
    pub fn sample_into(&self, t: Fixed, out: &mut Vec<BodySnap>) -> Option<FrameHead> {
        let kind = self.classify(t)?;
        let (lo, hi) = self.ring.brackets(t)?;
        let (floor_tick, floor) = self.ring.buffer.get(lo)?;
        out.clear();
        if kind == QueryKind::Extrapolate || lo == hi {
            out.extend_from_slice(&floor.fighters);
            return Some(FrameHead {
                tick: floor.tick,
                kind,
            });
        }
        let (hi_tick, hi_snap) = self.ring.buffer.get(hi)?;
        if !same_slots(floor, hi_snap) {
            out.extend_from_slice(&floor.fighters);
            return Some(FrameHead {
                tick: floor.tick,
                kind: QueryKind::Interpolate,
            });
        }
        let ta = Fixed::from_int(i64::from(*floor_tick));
        let span = (Fixed::from_int(i64::from(*hi_tick)) - ta).max(Fixed::ONE);
        let bracket = Bracket {
            lo,
            hi,
            floor_tick: *floor_tick,
            hi_tick: *hi_tick,
            s: ((t - ta) / span).clamp(Fixed::ZERO, Fixed::ONE),
            span,
        };
        for mine in &floor.fighters {
            let theirs = hi_snap
                .fighters
                .iter()
                .find(|f| f.slot == mine.slot)
                .unwrap_or(mine);
            out.push(blend_fighters(mine, theirs, &self.ring, &bracket));
        }
        Some(FrameHead {
            tick: floor.tick,
            kind: QueryKind::Interpolate,
        })
    }
}

impl Default for SnapshotInterpolator {
    fn default() -> Self {
        Self::new(MAX_INTERP_RING)
    }
}

/// True when two snapshots cover the same slots in the same order.
fn same_slots(a: &FullSnap, b: &FullSnap) -> bool {
    a.fighters.len() == b.fighters.len()
        && a.fighters
            .iter()
            .zip(b.fighters.iter())
            .all(|(x, y)| x.slot == y.slot)
}

/// One blend bracket: ring indices and ticks of both knots, the fraction `s`
/// in `0..=1` and the span in ticks.
struct Bracket {
    lo: usize,
    hi: usize,
    floor_tick: u32,
    hi_tick: u32,
    s: Fixed,
    span: Fixed,
}

/// Blend one fighter across a bracket: Hermite C1 on x/y/z with Catmull-Rom
/// tangents from the ring, quaternion-dispatched yaw, stepped hp/stamina.
fn blend_fighters(
    mine: &BodySnap,
    theirs: &BodySnap,
    ring: &InterpRing,
    bracket: &Bracket,
) -> BodySnap {
    let blend = |axis: Axis, a: i32, b: i32| {
        fixed_to_mm(blend_axis(
            ring,
            mine.slot,
            axis,
            bracket,
            (Fixed::from_int(i64::from(a)), Fixed::from_int(i64::from(b))),
        ))
    };
    let yaw = RenderQuat::blend(
        RenderQuat::from_yaw_ddeg(mine.yaw_ddeg),
        RenderQuat::from_yaw_ddeg(theirs.yaw_ddeg),
        bracket.s,
    )
    .to_yaw_ddeg();
    BodySnap {
        slot: mine.slot,
        x_mm: blend(Axis::X, mine.x_mm, theirs.x_mm),
        y_mm: blend(Axis::Y, mine.y_mm, theirs.y_mm),
        z_mm: blend(Axis::Z, mine.z_mm, theirs.z_mm),
        yaw_ddeg: yaw,
        hp: mine.hp,
        stamina: mine.stamina,
    }
}

/// [`Fixed`] millimeters back to integer millimeters: rounds half away from
/// zero, saturates at the `i32` bounds. The `try_from` below is infallible
/// after the clamp; the fallback names the bound instead of panicking.
fn fixed_to_mm(v: Fixed) -> i32 {
    let half = Fixed::SCALE / 2;
    let raw = v.to_bits();
    let rounded = if raw >= 0 { raw + half } else { raw - half } / Fixed::SCALE;
    let clamped = rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(clamped).unwrap_or(0)
}

/// Blend one position axis of one slot: the four knots around the bracket
/// (the nearest earlier and later anchors holding the slot, when there are
/// any) sit on the stack, Catmull-Rom tangents come from them, and the
/// bracket Hermite-blends between `ends` (floor, next). The knots are found
/// by ring index, never by value, which repeats whenever a body revisits a
/// position.
fn blend_axis(
    ring: &InterpRing,
    slot: u8,
    axis: Axis,
    bracket: &Bracket,
    ends: (Fixed, Fixed),
) -> Fixed {
    let mut knots = [(0u32, Fixed::ZERO); 4];
    let mut len = 0;
    if let Some(prev) = ring.prev_point(bracket.lo, slot, axis) {
        knots[len] = prev;
        len += 1;
    }
    let floor_at = len;
    knots[len] = (bracket.floor_tick, ends.0);
    knots[len + 1] = (bracket.hi_tick, ends.1);
    len += 2;
    if let Some(next) = ring.next_point(bracket.hi, slot, axis) {
        knots[len] = next;
        len += 1;
    }
    let m0 = tangent_at(&knots[..len], floor_at);
    let m1 = tangent_at(&knots[..len], floor_at + 1);
    hermite_scalar(ends.0, ends.1, m0, m1, bracket.span, bracket.s)
}

/// Per-frame retention of a leftover rollback correction, in permille. 940 per
/// 1000 leaves 2.4 percent of the jump after [`CORRECTION_FRAMES`] and moves
/// the first frame by 6.0 percent (the MP25 bar).
pub const CORRECTION_RETENTION_PERMILLE: u16 = 940;
/// Frames over which a correction falls to the 2.5 percent line.
pub const CORRECTION_FRAMES: u32 = 60;

/// Presentation-only offset between the drawn local body and the authoritative
/// pose after a rollback resimulation moved it.
///
/// The sim never reads this: it is added to the drawn pose only and decays one
/// step per 60 Hz frame, so an 8-tick resim that jumped 0.67 m shows a 4 cm
/// first-frame step instead of the whole jump. Units: metres, world axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Correction {
    err: [f32; 3],
}

impl Correction {
    /// A zero correction: nothing to hide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one resim jump into the offset (metres).
    pub fn note(&mut self, jump: [f32; 3]) {
        for (err, jump) in self.err.iter_mut().zip(jump) {
            *err += jump;
        }
    }

    /// The offset to add to the drawn pose this frame.
    #[must_use]
    pub fn offset(&self) -> [f32; 3] {
        self.err
    }

    /// One 60 Hz frame of decay.
    pub fn advance(&mut self) {
        let keep = f32::from(CORRECTION_RETENTION_PERMILLE) / 1000.0;
        for err in &mut self.err {
            *err *= keep;
        }
    }

    /// Fraction of `jump` still left to hide (1.0 right after [`Self::note`]).
    #[must_use]
    pub fn remaining(&self, jump: [f32; 3]) -> f32 {
        let span = (jump[0] * jump[0] + jump[1] * jump[1] + jump[2] * jump[2]).sqrt();
        if span <= 0.0 {
            return 0.0;
        }
        let left =
            (self.err[0] * self.err[0] + self.err[1] * self.err[1] + self.err[2] * self.err[2])
                .sqrt();
        left / span
    }
}

/// Remote presentation on the session LAN path: every authoritative snapshot
/// the client receives feeds the frozen [`Interpolator`] (tick-clock blend),
/// the [`ConfirmedHistory`] (delta baselines plus the trailing cursor) and the
/// [`SnapshotInterpolator`] (Hermite positions, quaternion yaw, extrapolate
/// guard). The three read the same bytes; none is forked or rewritten here.
pub struct RemoteView {
    legacy: Interpolator,
    history: ConfirmedHistory,
    sampler: TimelineSampler,
    smooth: SnapshotInterpolator,
}

impl RemoteView {
    /// Open a view: the frozen interpolator delays `legacy_delay_ticks` (it
    /// reads ticks as its time unit), the smooth path trails the newest
    /// confirmed tick by `cursor_delay_ticks` (clamped by the sampler).
    #[must_use]
    pub fn new(legacy_delay_ticks: u64, cursor_delay_ticks: u32) -> Self {
        Self {
            legacy: Interpolator::new(legacy_delay_ticks),
            history: ConfirmedHistory::default(),
            sampler: TimelineSampler::new(cursor_delay_ticks),
            smooth: SnapshotInterpolator::default(),
        }
    }

    /// Feed one authoritative snapshot to every path. Invalid snapshots
    /// reject with `false` and change nothing; duplicates replace in place,
    /// and late arrivals sort into the rings by tick.
    pub fn push(&mut self, snap: &FullSnap) -> bool {
        if snap.validate().is_err() {
            return false;
        }
        let tick = snap.tick;
        self.legacy.push(u64::from(tick), snap.clone());
        self.history.insert(tick, snap.clone());
        self.smooth.push(tick, snap.clone())
    }

    /// Rebuild one snapshot from a delta against the stored baseline at
    /// `delta.base_tick`, then feed it like [`Self::push`]. Returns the
    /// rebuilt tick.
    ///
    /// # Errors
    ///
    /// [`NetError::BadState`] when the baseline tick is not in the history or
    /// the delta disagrees with it; nothing is stored then.
    pub fn apply_delta(&mut self, delta: &Delta) -> Result<u32, NetError> {
        let mut baseline = self
            .history
            .get_at(delta.base_tick)
            .cloned()
            .ok_or(NetError::BadState)?;
        // A compressed anchor resolves to the explicit snapshot it repeats,
        // which keeps its authored tick; the delta names the anchor tick.
        baseline.tick = delta.base_tick;
        let full = decode_delta(delta, &baseline)?;
        if !self.push(&full) {
            return Err(NetError::BadState);
        }
        Ok(full.tick)
    }

    /// Anchors held by the smooth path.
    #[must_use]
    pub fn len(&self) -> usize {
        self.smooth.len()
    }

    /// True before the first snapshot lands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.smooth.is_empty()
    }

    /// Frozen tick-clock answer at `at_tick` (the delay is the
    /// `legacy_delay_ticks` of [`Self::new`]).
    #[must_use]
    pub fn legacy_at(&self, at_tick: u64) -> Option<InterpResult> {
        self.legacy.at(at_tick)
    }

    /// Whole-tick presentation cursor: newest confirmed tick minus the
    /// trailing delay. `None` before the first snapshot.
    pub fn present_tick(&mut self) -> Option<u32> {
        let newest = self.history.newest_tick()?;
        Some(self.sampler.advance(newest))
    }

    /// Smooth frame at the cursor plus `alpha` ticks of render sub-tick
    /// (clamped to `0..=1`; callers derive it from the render clock with
    /// integer math). `None` before the first snapshot or past the guard's
    /// horizon. Allocates the answer; the per-frame path is
    /// [`Self::frame_into`].
    pub fn present(&mut self, alpha: Fixed) -> Option<InterpFrame> {
        let mut fighters = Vec::new();
        let head = self.frame_into(alpha, &mut fighters)?;
        Some(InterpFrame {
            state: FullSnap {
                tick: head.tick,
                fighters,
            },
            kind: head.kind,
        })
    }

    /// [`Self::present`] into a caller buffer, allocation-free once the buffer
    /// holds the slot count.
    pub fn frame_into(&mut self, alpha: Fixed, out: &mut Vec<BodySnap>) -> Option<FrameHead> {
        let present = self.present_tick()?;
        let t = Fixed::from_int(i64::from(present)) + alpha.clamp(Fixed::ZERO, Fixed::ONE);
        self.smooth.sample_into(t, out)
    }

    /// Smooth frame at an explicit [`Fixed`] tick, bypassing the cursor.
    #[must_use]
    pub fn frame_at(&self, t: Fixed) -> Option<InterpFrame> {
        self.smooth.sample_at(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use engine_core::Fnv64;

    fn fighter(slot: u8, x_mm: i32, yaw_ddeg: i16) -> BodySnap {
        BodySnap {
            slot,
            x_mm,
            y_mm: 0,
            z_mm: 2000,
            yaw_ddeg,
            hp: 1000,
            stamina: 100,
        }
    }

    fn snap_at(tick: u32, x0_mm: i32, x1_mm: i32) -> FullSnap {
        let snap = FullSnap {
            tick,
            fighters: vec![fighter(0, x0_mm, 0), fighter(1, x1_mm, -900)],
        };
        snap.validate().expect("valid fixture");
        snap
    }

    fn tick_f(tick: u32) -> Fixed {
        Fixed::from_int(i64::from(tick))
    }

    fn curved_ring() -> SnapshotInterpolator {
        // Accelerating truth p(t) = 500 * t * (t + 1) mm at ticks 0..=4.
        let mut interp = SnapshotInterpolator::new(8);
        for tick in 0..=4 {
            let t = tick as f32;
            let p = (500.0 * t * (t + 1.0)).round() as i32;
            assert!(interp.push(tick, snap_at(tick, p, -p)));
        }
        interp
    }

    #[test]
    fn snapshot_hermite_endpoints_exact_and_c1_at_knot() {
        let interp = curved_ring();
        // Exact anchors reproduce byte-for-byte.
        for tick in 0..=4 {
            let frame = interp.sample_at(tick_f(tick)).expect("anchor");
            assert_eq!(frame.kind, QueryKind::Interpolate);
            let truth = (500.0 * tick as f32 * (tick as f32 + 1.0)).round() as i32;
            assert_eq!(frame.state.fighters[0].x_mm, truth);
        }
        // C1 across the shared knot: both brackets meeting at tick 2 carry
        // the same Catmull-Rom tangent, so left and right velocities agree.
        let chan: Vec<(u32, Fixed)> = (0..=4)
            .map(|tick| {
                let t = i64::from(tick);
                (tick, Fixed::from_int(500 * t * (t + 1)))
            })
            .collect();
        let (m_left, m_right) = (tangent_at(&chan, 1), tangent_at(&chan, 2));
        let (p1, p2, p3) = (
            Fixed::from_int(1000),
            Fixed::from_int(3000),
            Fixed::from_int(6000),
        );
        let h = Fixed::from_int(1) / Fixed::from_int(10_000);
        let one = Fixed::ONE;
        let v_left = (hermite_scalar(p1, p2, m_left, m_right, one, one)
            - hermite_scalar(p1, p2, m_left, m_right, one, one - h))
            / h;
        let m_next = tangent_at(&chan, 3);
        let v_right = (hermite_scalar(p2, p3, m_right, m_next, one, h)
            - hermite_scalar(p2, p3, m_right, m_next, one, Fixed::ZERO))
            / h;
        let scale = if v_left > v_right { v_left } else { v_right };
        let scale = if scale > one { scale } else { one };
        let rel = (v_left - v_right) / scale;
        let eps = Fixed::from_int(1) / Fixed::from_int(1000);
        assert!(
            rel < eps && rel > Fixed::ZERO - eps,
            "knot velocities differ: {} vs {}",
            v_left.to_bits(),
            v_right.to_bits()
        );
    }

    #[test]
    fn snapshot_guard_classifies_inside_edge_and_outside() {
        let interp = curved_ring();
        let q = |num: i64, den: i64| Fixed::from_int(num) / Fixed::from_int(den);
        assert_eq!(interp.classify(q(5, 2)), Some(QueryKind::Interpolate));
        assert_eq!(interp.classify(Fixed::ZERO), Some(QueryKind::Interpolate));
        assert_eq!(interp.classify(tick_f(4)), Some(QueryKind::Interpolate));
        assert_eq!(interp.classify(q(9, 2)), Some(QueryKind::Extrapolate));
        assert_eq!(
            interp.classify(Fixed::from_int(-1)),
            Some(QueryKind::Extrapolate)
        );
        // Past the horizon the guard refuses instead of misclassifying.
        // Fixed ticks cannot be non-finite, so NaN/infinity queries are
        // unrepresentable by construction.
        assert_eq!(
            interp.classify(tick_f(4) + MAX_EXTRAPOLATE_TICKS + Fixed::HALF),
            None
        );
        let empty = SnapshotInterpolator::new(8);
        assert_eq!(empty.classify(tick_f(2)), None);
        assert_eq!(empty.sample_at(tick_f(2)), None);
        // Extrapolation clamps to the edge anchor, unblended.
        let past = interp.sample_at(tick_f(5)).expect("clamped");
        assert_eq!(past.kind, QueryKind::Extrapolate);
        let newest = interp.sample_at(tick_f(4)).expect("newest");
        assert_eq!(past.state, newest.state);
    }

    #[test]
    fn two_snapshot_seeded_fixture_beats_step_baseline() {
        // Seeded curve coefficient from the engine hash stream (seed 2040).
        let coeff = 400.0 + (engine_core::splitmix64(2040) % 400) as f32;
        // Anchors two ticks apart on p(t) = coeff * t * (t + 1).
        let truth = |t: f32| coeff * t * (t + 1.0);
        let mut interp = SnapshotInterpolator::new(4);
        for tick in [0, 2] {
            let p = truth(tick as f32).round() as i32;
            assert!(interp.push(tick, snap_at(tick, p, 0)));
        }
        let mid_truth = truth(1.0);
        let blended = interp.sample_at(tick_f(1)).expect("mid").state.fighters[0].x_mm as f32;
        let step_err = (truth(0.0) - mid_truth).abs();
        let blend_err = (blended - mid_truth).abs();
        assert!(
            blend_err < step_err,
            "blend err {blend_err} should beat step err {step_err}"
        );
        // Same inputs twice give the same presentation digest.
        let digest = |interp: &SnapshotInterpolator| {
            let frame = interp.sample_at(tick_f(1)).expect("mid");
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&frame.state.tick.to_le_bytes());
            for f in &frame.state.fighters {
                bytes.push(f.slot);
                bytes.extend_from_slice(&f.x_mm.to_le_bytes());
                bytes.extend_from_slice(&f.y_mm.to_le_bytes());
                bytes.extend_from_slice(&f.z_mm.to_le_bytes());
                bytes.extend_from_slice(&f.yaw_ddeg.to_le_bytes());
                bytes.extend_from_slice(&f.hp.to_le_bytes());
                bytes.extend_from_slice(&f.stamina.to_le_bytes());
            }
            engine_core::hash_bytes(&bytes)
        };
        let mut again = SnapshotInterpolator::new(4);
        for tick in [0, 2] {
            let p = truth(tick as f32).round() as i32;
            assert!(again.push(tick, snap_at(tick, p, 0)));
        }
        assert_eq!(digest(&interp), digest(&again));
    }

    #[test]
    fn snapshot_invalid_push_and_slot_mismatch_hold_without_panic() {
        let mut interp = SnapshotInterpolator::new(4);
        let bad = FullSnap {
            tick: 3,
            fighters: Vec::new(),
        };
        assert!(!interp.push(3, bad));
        assert!(interp.is_empty());
        assert!(interp.push(1, snap_at(1, 1000, 0)));
        // Same tick replaces instead of duplicating.
        assert!(interp.push(1, snap_at(1, 1500, 0)));
        assert_eq!(interp.len(), 1);
        assert!(interp.push(2, snap_at(2, 2000, 0)));
        // Bracket with mismatched slots falls back to the floor anchor.
        let reshuffled = FullSnap {
            tick: 3,
            fighters: vec![fighter(0, 8888, 0), fighter(2, 9999, 0)],
        };
        assert!(interp.push(3, reshuffled));
        let frame = interp
            .sample_at(Fixed::from_int(5) / Fixed::from_int(2))
            .expect("fallback");
        assert_eq!(frame.kind, QueryKind::Interpolate);
        assert_eq!(frame.state.fighters[0].x_mm, 2000);
    }

    #[test]
    fn snapshot_feed_history_wires_confirmed_history() {
        let mut history = ConfirmedHistory::new(8);
        for tick in 10..=12 {
            assert!(history.insert(tick, snap_at(tick, 1000 + tick as i32 * 10, 0)));
        }
        let mut interp = SnapshotInterpolator::new(8);
        assert_eq!(interp.feed_history(&history), 3);
        assert_eq!(interp.len(), 3);
        let frame = interp.sample_at(tick_f(11)).expect("anchor");
        assert_eq!(frame.kind, QueryKind::Interpolate);
        assert_eq!(frame.state.fighters[0].x_mm, 1110);
        // Empty history feeds nothing and changes nothing.
        let vacant = ConfirmedHistory::new(8);
        assert_eq!(interp.feed_history(&vacant), 0);
        assert_eq!(interp.len(), 3);
    }

    #[test]
    fn snapshot_ring_bounds_and_eviction_hold() {
        let mut interp = SnapshotInterpolator::new(3);
        assert_eq!(interp.len(), 0);
        assert!(interp.is_empty());
        for tick in 0..5 {
            assert!(interp.push(tick, snap_at(tick, tick as i32, 0)));
        }
        assert_eq!(interp.len(), 3);
        // Evicted ticks refuse past the backward horizon instead of blending
        // stale clamps: oldest is 2 with a 2-tick horizon, so tick -1
        // refuses while tick 0.5 still clamps to the edge anchor.
        assert_eq!(interp.sample_at(Fixed::from_int(-1)), None);
        let edge = interp
            .sample_at(Fixed::ONE / Fixed::from_int(2))
            .expect("edge");
        assert_eq!(edge.kind, QueryKind::Extrapolate);
        assert_eq!(edge.state.tick, 2);
        assert!(interp
            .sample_at(Fixed::from_int(7) / Fixed::from_int(2))
            .is_some());
        // Out-of-order pushes sort; default cap clamps to the bound.
        let full = SnapshotInterpolator::default();
        assert_eq!(full.len(), 0);
        let tiny = InterpRing::new(0);
        assert_eq!(tiny.cap(), 2);
        assert_eq!(InterpRing::new(1 << 20).cap(), MAX_INTERP_RING);
    }

    // ---- Session-path fixture: 16 bodies, jittered snapshot stream --------

    use crate::delta::encode_delta;
    use crate::session::{
        decode_snapshot, decode_state, encode_snapshot, encode_state_delta, KIND_SNAPSHOT,
    };
    use engine_arena::Checkpoint;

    const BODIES: u8 = 16;
    const CADENCE: u32 = 6;
    const SNAPS: u32 = 20;
    const NEWEST: u32 = CADENCE * (SNAPS - 1);
    /// Digest of every fixture datagram in send order (the delta wire).
    const SNAPSHOT_WIRE_DIGEST: u64 = 0x0e38_ab48_d0ef_28cb;
    /// Digest of the presentation frames over the fixture query grid. Re-pinned 2026-10-21 from
    /// 0xea00_1b9b_8d43_6cea: `Fixed` products round to nearest instead of flooring, and the
    /// interpolation blends through `Fixed::lerp` (the wire digest above did not move).
    const SNAPSHOT_FRAME_DIGEST: u64 = 0x88a8_39e8_a491_3610;

    /// Exact integer truth at quarter-tick `q` (t = q / 4): a parabola about
    /// tick 57, so anchors 54 and 60 repeat the same position and a
    /// value-keyed tangent lookup would pick the wrong knot.
    fn truth_x(slot: u8, q: i64) -> i64 {
        let c = 40 + 10 * i64::from(slot);
        let d = q - 228;
        2000 * i64::from(slot) + (c * d * d + 8).div_euclid(16)
    }

    fn truth_z(slot: u8, q: i64) -> i64 {
        10 * q - 500 * i64::from(slot)
    }

    fn truth_yaw(slot: u8, q: i64) -> i16 {
        let raw = 40 * i64::from(slot) + 25 * q / 4;
        i16::try_from((raw + 1800).rem_euclid(3600) - 1800).expect("wrapped yaw fits")
    }

    fn truth_snap(tick: u32) -> FullSnap {
        let q = i64::from(tick) * 4;
        let fighters = (0..BODIES)
            .map(|slot| BodySnap {
                slot,
                x_mm: i32::try_from(truth_x(slot, q)).expect("x fits"),
                y_mm: 0,
                z_mm: i32::try_from(truth_z(slot, q)).expect("z fits"),
                yaw_ddeg: truth_yaw(slot, q),
                hp: 1000,
                stamina: 100,
            })
            .collect();
        FullSnap::new(tick, fighters).expect("fixture snapshot")
    }

    /// Server side: two full snapshots, then deltas against the snapshot two
    /// intervals back (the last one the client acked). Returns send order.
    fn server_wire() -> Vec<(u32, Vec<u8>)> {
        (0..SNAPS)
            .map(|k| {
                let tick = k * CADENCE;
                let full = truth_snap(tick);
                let bytes = if k < 2 {
                    encode_snapshot(&full).expect("full encodes")
                } else {
                    let base = truth_snap((k - 2) * CADENCE);
                    let delta = encode_delta(&full, &base).expect("delta encodes");
                    let hash = engine_core::hash_bytes(&tick.to_le_bytes());
                    encode_state_delta(Checkpoint { tick, hash }, &delta).expect("state encodes")
                };
                (tick, bytes)
            })
            .collect()
    }

    /// Seeded arrival order: 3 ticks of latency, 0..=3 ticks of jitter and 8
    /// extra ticks on odd snapshots, so neighbors (6 ticks apart) swap while a
    /// baseline (12 ticks back) always lands first.
    fn arrival_order(seed: u64) -> Vec<usize> {
        let mut keyed: Vec<(u64, usize)> = (0..SNAPS as usize)
            .map(|k| {
                let jitter =
                    engine_core::mix64(seed ^ k as u64) % 4 + if k % 2 == 1 { 8 } else { 0 };
                (u64::from(CADENCE) * k as u64 + 3 + jitter, k)
            })
            .collect();
        keyed.sort_unstable();
        keyed.into_iter().map(|(_, k)| k).collect()
    }

    /// Client side: route one datagram by kind exactly as the LAN guest does.
    fn receive(view: &mut RemoteView, bytes: &[u8]) -> Result<u32, NetError> {
        if bytes[3] == KIND_SNAPSHOT {
            let snap = decode_snapshot(bytes)?;
            return view
                .push(&snap)
                .then_some(snap.tick)
                .ok_or(NetError::BadState);
        }
        let packet = decode_state(bytes)?;
        view.apply_delta(&packet.delta.ok_or(NetError::BadKind(bytes[3]))?)
    }

    fn fixture_view(seed: u64) -> RemoteView {
        let wire = server_wire();
        let mut view = RemoteView::new(6, 8);
        for k in arrival_order(seed) {
            let tick = receive(&mut view, &wire[k].1).expect("datagram applies");
            assert_eq!(tick, wire[k].0);
        }
        view
    }

    fn quarter(q: i64) -> Fixed {
        Fixed::from_int(q) / Fixed::from_int(4)
    }

    fn digest_frames(view: &RemoteView) -> u64 {
        let mut bytes = Vec::new();
        for q in -12..=(i64::from(NEWEST) * 4 + 12) {
            match view.frame_at(quarter(q)) {
                Some(frame) => {
                    bytes.push(frame.kind as u8);
                    bytes.extend_from_slice(&frame.state.tick.to_le_bytes());
                    for f in &frame.state.fighters {
                        bytes.extend_from_slice(&f.x_mm.to_le_bytes());
                        bytes.extend_from_slice(&f.z_mm.to_le_bytes());
                        bytes.extend_from_slice(&f.yaw_ddeg.to_le_bytes());
                    }
                }
                None => bytes.push(0xFF),
            }
        }
        engine_core::hash_bytes(&bytes)
    }

    #[test]
    fn snapshot_yaw_blend_takes_the_short_way_across_the_seam() {
        let yaw = RenderQuat::from_yaw_ddeg;
        // 20, 10 and 20 decidegrees apart: slerp, nlerp, slerp across the
        // 180 degree seam. The midpoint sits on the seam, never at zero.
        for (from, to) in [(1790i16, -1790i16), (1795, -1795), (-1790, 1790)] {
            let mid = RenderQuat::blend(yaw(from), yaw(to), Fixed::HALF).to_yaw_ddeg();
            let off = (i32::from(mid) - 1800).rem_euclid(3600);
            assert!(off <= 3 || off >= 3597, "{from}->{to} mid {mid}");
        }
        let quarter =
            RenderQuat::blend(yaw(1790), yaw(-1790), Fixed::ONE / Fixed::from_int(4)).to_yaw_ddeg();
        assert!((i32::from(quarter) - 1795).abs() <= 3, "quarter {quarter}");
    }

    #[test]
    fn snapshot_wire_jitter_16_bodies_interpolates_c1() {
        let order = arrival_order(2040);
        assert!(
            order.windows(2).any(|pair| pair[0] > pair[1]),
            "the seeded jitter must reorder arrivals: {order:?}"
        );
        let view = fixture_view(2040);
        assert_eq!(view.len(), SNAPS as usize);
        // Interior brackets reproduce the analytic curve: Catmull-Rom on a
        // uniform quadratic is exact, so any wrong knot or tangent shows.
        for q in 24..=(i64::from(NEWEST) * 4 - 24) {
            let frame = view.frame_at(quarter(q)).expect("inside the ring");
            assert_eq!(frame.kind, QueryKind::Interpolate, "q={q}");
            for f in &frame.state.fighters {
                let want = truth_x(f.slot, q);
                assert!(
                    (i64::from(f.x_mm) - want).abs() <= 2,
                    "slot {} q {q}: x {} vs {want}",
                    f.slot,
                    f.x_mm
                );
                assert!((i64::from(f.z_mm) - truth_z(f.slot, q)).abs() <= 2);
                let dyaw = (i32::from(f.yaw_ddeg) - i32::from(truth_yaw(f.slot, q)) + 5400)
                    .rem_euclid(3600)
                    - 1800;
                assert!(dyaw.abs() <= 3, "slot {} q {q}: yaw off {dyaw}", f.slot);
                assert_eq!((f.hp, f.stamina), (1000, 100));
            }
        }
        // C1 at every interior knot: the one-sided slopes agree. Linear
        // blending would jump by 12 * c mm/tick (480 or more here).
        let eps = Fixed::ONE / Fixed::from_int(64);
        for knot in (CADENCE..NEWEST).step_by(CADENCE as usize) {
            let t = Fixed::from_int(i64::from(knot));
            let at = view.frame_at(t).expect("knot");
            let left = view.frame_at(t - eps).expect("left of knot");
            let right = view.frame_at(t + eps).expect("right of knot");
            for slot in 0..usize::from(BODIES) {
                let x = |frame: &InterpFrame| i64::from(frame.state.fighters[slot].x_mm);
                let v_left = (x(&at) - x(&left)) * 64;
                let v_right = (x(&right) - x(&at)) * 64;
                assert!(
                    (v_left - v_right).abs() <= 250,
                    "knot {knot} slot {slot}: slope {v_left} then {v_right}"
                );
            }
        }
    }

    #[test]
    fn snapshot_guard_classifies_interpolate_extrapolate_refuse() {
        let view = fixture_view(2040);
        let newest = Fixed::from_int(i64::from(NEWEST));
        let kind = |t: Fixed| view.frame_at(t).map(|frame| frame.kind);
        assert_eq!(kind(Fixed::ZERO), Some(QueryKind::Interpolate));
        assert_eq!(kind(newest), Some(QueryKind::Interpolate));
        assert_eq!(
            kind(quarter(i64::from(NEWEST) * 4 + 2)),
            Some(QueryKind::Extrapolate)
        );
        assert_eq!(
            kind(newest + MAX_EXTRAPOLATE_TICKS),
            Some(QueryKind::Extrapolate)
        );
        assert_eq!(kind(newest + MAX_EXTRAPOLATE_TICKS + Fixed::HALF), None);
        assert_eq!(kind(Fixed::from_int(-2)), Some(QueryKind::Extrapolate));
        assert_eq!(kind(Fixed::from_int(-3)), None);
        // Past the newest anchor the guard holds that anchor, never a guess.
        let held = view.frame_at(newest + Fixed::ONE).expect("held");
        assert_eq!(held.state, truth_snap(NEWEST));
    }

    #[test]
    fn snapshot_remote_view_keeps_frozen_interpolator_bytes() {
        let view = fixture_view(2040);
        let mut frozen = Interpolator::new(6);
        for tick in (0..SNAPS).map(|k| k * CADENCE) {
            assert!(frozen.push(u64::from(tick), truth_snap(tick)));
        }
        for at in 0..=u64::from(NEWEST) + 4 {
            assert_eq!(view.legacy_at(at), frozen.at(at), "tick {at}");
        }
    }

    #[test]
    fn snapshot_wire_bytes_and_presentation_are_pinned_x2() {
        // The delta wire is frozen: a codec change moves this digest.
        let mut all = Vec::new();
        for (_, bytes) in server_wire() {
            all.extend_from_slice(&bytes);
        }
        assert_eq!(engine_core::hash_bytes(&all), SNAPSHOT_WIRE_DIGEST);
        // Presentation is a pure function of the received set, whatever the
        // arrival order, and identical across independent runs.
        let first = digest_frames(&fixture_view(2040));
        assert_eq!(first, digest_frames(&fixture_view(2040)));
        assert_eq!(first, digest_frames(&fixture_view(7)));
        assert_eq!(first, SNAPSHOT_FRAME_DIGEST);
    }

    #[test]
    fn snapshot_frame_into_allocates_nothing_after_warmup() {
        let mut view = fixture_view(2040);
        let mut out: Vec<BodySnap> = Vec::with_capacity(usize::from(BODIES));
        let (last, heap) = engine_core::alloc_probe::measure(|| {
            let mut last = None;
            for step in 0..64 {
                let alpha = Fixed::from_int(step % 5) / Fixed::from_int(4);
                last = view.frame_into(alpha, &mut out);
            }
            last
        });
        assert_eq!(heap.calls, 0, "the per-frame path must not allocate");
        assert_eq!(last.map(|head| head.kind), Some(QueryKind::Interpolate));
        // The buffer path answers exactly what the allocating path answers.
        let owned = view.present(Fixed::ONE).expect("owned frame");
        let head = view.frame_into(Fixed::ONE, &mut out).expect("buffer frame");
        assert_eq!((head.tick, head.kind), (owned.state.tick, owned.kind));
        assert_eq!(out, owned.state.fighters);
        assert_eq!(out.len(), usize::from(BODIES));
    }

    #[test]
    fn snapshot_remote_view_cursor_trails_newest() {
        let mut view = fixture_view(2040);
        assert_eq!(view.present_tick(), Some(NEWEST - 8));
        let frame = view.present(Fixed::HALF).expect("cursor frame");
        assert_eq!(frame.kind, QueryKind::Interpolate);
        let q = (i64::from(NEWEST) - 8) * 4 + 2;
        assert!((i64::from(frame.state.fighters[3].x_mm) - truth_x(3, q)).abs() <= 2);
        // Alpha clamps to the tick: it never skips ahead of the next tick.
        let wide = view.present(Fixed::from_int(9)).expect("clamped alpha");
        let edge = view.present(Fixed::ONE).expect("alpha one");
        assert_eq!(wide, edge);
        let mut empty = RemoteView::new(6, 8);
        assert!(empty.is_empty());
        assert_eq!(empty.present_tick(), None);
        assert_eq!(empty.present(Fixed::HALF), None);
    }

    #[test]
    fn snapshot_remote_view_delta_needs_its_baseline() {
        let wire = server_wire();
        let delta_of = |k: usize| {
            decode_state(&wire[k].1)
                .expect("state decodes")
                .delta
                .expect("delta packet")
        };
        let mut view = RemoteView::new(6, 8);
        // Snapshot 3 diffs against snapshot 1: refused while that baseline is
        // missing, stored nowhere.
        assert_eq!(view.apply_delta(&delta_of(3)), Err(NetError::BadState));
        assert!(view.is_empty());
        receive(&mut view, &wire[1].1).expect("baseline lands");
        assert_eq!(view.apply_delta(&delta_of(3)), Ok(3 * CADENCE));
        // A duplicate replaces in place instead of growing the ring.
        assert_eq!(view.apply_delta(&delta_of(3)), Ok(3 * CADENCE));
        assert_eq!(view.len(), 2);
        // Compressed baselines resolve: an unchanged snapshot stores a marker
        // whose delta still names the marker tick.
        let still = FullSnap::new(100, truth_snap(0).fighters).expect("still");
        let again = FullSnap::new(106, truth_snap(0).fighters).expect("repeat");
        let moved = FullSnap::new(112, truth_snap(6).fighters).expect("moved");
        let mut calm = RemoteView::new(6, 8);
        assert!(calm.push(&still) && calm.push(&again));
        let delta = encode_delta(&moved, &again).expect("delta");
        assert_eq!(calm.apply_delta(&delta), Ok(112));
        assert_eq!(calm.len(), 3);
        // A delta that disagrees with its baseline refuses and stores nothing.
        let mut skewed = delta;
        skewed.changed_slots.truncate(1);
        skewed.changed_slots[0].0 = 40;
        assert!(calm.apply_delta(&skewed).is_err());
        assert_eq!(calm.len(), 3);
    }

    // ---- QN4: interpolation delay from loss data ---------------------------

    use crate::delta::DEFAULT_INTERP_DELAY_MS;
    use crate::link::{FaultLink, Frame, LinkProfile};

    /// Ticks soaked: 60 s at 60 Hz, 600 snapshots at the 10 Hz cadence.
    const SOAK_TICKS: u32 = 3600;
    /// Render samples start after the buffer had one second to fill.
    const SOAK_WARMUP_TICKS: u32 = 60;
    /// Delays the receipt posts, in ms.
    const POSTED_DELAYS_MS: [u64; 4] = [100, 150, 200, 300];
    /// Loss rates the receipt posts, in percent.
    const POSTED_LOSS_PCT: [u32; 2] = [2, 5];

    /// Server-clock milliseconds of a tick (60 Hz).
    fn tick_ms(tick: u32) -> u64 {
        u64::from(tick) * 1000 / 60
    }

    /// Render misses of one soak: the server sends a snapshot every
    /// `CADENCE` ticks over a seeded link (33 to 67 ms one way, `loss_pct`
    /// datagram loss); the client renders every tick at `now - delay_ms` on
    /// the server clock. A render misses when no buffered pair brackets the
    /// query, so it clamps to one stored snapshot instead of blending.
    fn bracket_misses(delay_ms: u64, loss_pct: u32) -> u32 {
        let profile = LinkProfile {
            latency: 2,
            jitter: 2,
            loss_pct,
            reorder_pct: 0,
        };
        let mut link = FaultLink::new(profile, 0x51ED_0004);
        let mut interp = Interpolator::new(delay_ms);
        let mut misses = 0;
        for tick in 0..SOAK_TICKS {
            if tick % CADENCE == 0 {
                link.send_datagram(&tick.to_le_bytes());
            }
            for frame in link.due() {
                let Frame::Datagram(bytes) = frame else {
                    continue;
                };
                let sent = u32::from_le_bytes(bytes[..4].try_into().expect("tick bytes"));
                let x = i32::try_from(sent).expect("tick fits");
                assert!(interp.push(tick_ms(sent), snap_at(sent, x, -x)));
            }
            if tick >= SOAK_WARMUP_TICKS {
                misses += u32::from(interp.delayed(tick_ms(tick)).is_none_or(|r| r.clamped));
            }
            link.tick();
        }
        misses
    }

    #[test]
    fn interp_delay_default_is_the_smallest_with_no_miss_at_2_percent_loss() {
        let samples = SOAK_TICKS - SOAK_WARMUP_TICKS;
        let table: Vec<(u32, u64, u32)> = POSTED_LOSS_PCT
            .iter()
            .flat_map(|&loss| POSTED_DELAYS_MS.iter().map(move |&delay| (loss, delay)))
            .map(|(loss, delay)| (loss, delay, bracket_misses(delay, loss)))
            .collect();
        for (loss, delay, misses) in &table {
            println!(
                "qn4 loss {loss}% delay {delay} ms: {misses}/{samples} renders with no bracket ({} bp)",
                misses * 10_000 / samples
            );
        }
        let misses_at = |loss: u32, delay: u64| {
            table
                .iter()
                .find(|row| (row.0, row.1) == (loss, delay))
                .map(|row| row.2)
                .expect("posted row")
        };
        for loss in POSTED_LOSS_PCT {
            for pair in POSTED_DELAYS_MS.windows(2) {
                assert!(
                    misses_at(loss, pair[1]) <= misses_at(loss, pair[0]),
                    "more delay never misses more at {loss}%"
                );
            }
        }
        let smallest = POSTED_DELAYS_MS
            .into_iter()
            .find(|&delay| misses_at(2, delay) == 0)
            .expect("some posted delay has no miss at 2% loss");
        assert_eq!(DEFAULT_INTERP_DELAY_MS, smallest);
        // The old 100 ms default misses over a third of the renders at 5%
        // loss; the new one holds below 1% (two lost snapshots in a row
        // still miss).
        assert!(misses_at(5, 100) * 3 > samples, "100 ms is too short");
        assert!(misses_at(5, DEFAULT_INTERP_DELAY_MS) * 100 < samples);
        assert_eq!(bracket_misses(smallest, 2), 0, "same seed, same soak");
    }

    #[test]
    fn rollback_correction_steps_six_percent_then_two_and_a_half_after_sixty() {
        // An 8-tick resim that moved the local fighter 0.67 m in one frame
        // (MP25). The drawn pose hides the jump over 60 frames.
        let jump = [0.67, 0.0, -0.12];
        let mut corr = Correction::new();
        assert_eq!(corr.offset(), [0.0; 3]);
        corr.note(jump);
        let before = corr.remaining(jump);
        corr.advance();
        let first = before - corr.remaining(jump);
        assert!(
            first <= 0.06 + 1e-4,
            "first frame moved {first} of the jump"
        );
        assert!(first > 0.0, "the first frame does move");
        for _ in 1..CORRECTION_FRAMES {
            corr.advance();
        }
        let left = corr.remaining(jump);
        assert!(
            left <= 0.025,
            "{left} left after {CORRECTION_FRAMES} frames"
        );
        // A second jump folds into the leftover instead of snapping.
        let folded = corr.offset();
        corr.note([0.1, 0.0, 0.0]);
        assert!((corr.offset()[0] - (folded[0] + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn rollback_correction_never_moves_the_authoritative_frame_digest() {
        // The correction is added to the drawn pose only: the interpolated
        // frame bytes are the same whether a correction is stepped or not, so
        // the sim and the replay hash never see it.
        let digest_of = |with_correction: bool| {
            let mut view = fixture_view(2040);
            let mut corr = Correction::new();
            let mut hash = Fnv64::new();
            for step in 0..CORRECTION_FRAMES {
                if with_correction {
                    corr.note([0.67, 0.0, 0.0]);
                    corr.advance();
                }
                let alpha = Fixed::from_int(i64::from(step % 5)) / Fixed::from_int(4);
                let frame = view.present(alpha).expect("a frame to draw");
                for body in &frame.state.fighters {
                    hash.write_i32(body.x_mm);
                }
            }
            hash.finish()
        };
        assert_eq!(digest_of(true), digest_of(false));
        // The frozen presentation digest did not move either.
        assert_eq!(digest_frames(&fixture_view(2040)), SNAPSHOT_FRAME_DIGEST);
    }
}
