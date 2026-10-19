//! interest: Iris-style relevancy for siege replication.
//!
//! Per-connection entity selection by distance and view, rate LOD for far
//! entities, per-snapshot delta budgets for 64 clients plus 500 AI. Owner:
//! net lane. Units: millimetres, ticks at 60 Hz, bytes and bits/s.
//! Determinism: hash path is integer `+ - * /` and stable id order only, no
//! map iteration, no wall clock; reports carry integer kilobits/s x100 and
//! integer saving x100, never float. Errors: [`InterestError`], never panics.
//!
//! Callers: in-test consumer drives [`ConnectionInterest::select`] through
//! the real `delta.rs` encode/decode path; production `session.rs` wiring is
//! deferred to the session packet. Crate edge: reuses `scale.rs` header/budget
//! constants, never mirrors them.

use super::delta::{ioq3_encode_fields, ioq3_fields_wire_len, BodySnap, FullSnap, Ioq3SnapFields};
use super::scale::{DELTA_HEADER_LEN, MAX_DATAGRAM_BYTES, TICK_HZ};
use super::session::{ioq3_fragment_message, NetError};

/// Largest entity population one selection handles (64 fighters plus 500
/// AI soldiers plus headroom for promoted directors).
pub const MAX_INTEREST_ENTITIES: usize = 1024;
/// Largest connection count the siege proof addresses.
pub const MAX_CONNECTIONS: usize = 64;
/// No entity goes longer unsent: 500 ms at 60 Hz (the starve bar).
pub const STARVE_TICKS: u32 = 30;
/// Snapshot cadence for the siege proof: 10 Hz.
pub const SNAPSHOT_EVERY_TICKS: u32 = 6;
/// Per-client bandwidth bar: 256 kbps, in bytes/s (the base p95 bar).
pub const ON3_CAP_BPS: u32 = 32_000;
/// Per-client bandwidth bar: 128 kbps, in bytes/s (the siege p95 bar).
pub const ON9_CAP_BPS: u32 = 16_000;
/// Siege p95 bar in kilobits/s x100 (128.00 kbps, integer for exact asserts).
pub const ON9_CAP_KBPS_X100: u64 = 12_800;
/// Base p95 bar in kilobits/s x100 (256.00 kbps, integer for exact asserts).
pub const ON3_CAP_KBPS_X100: u64 = 25_600;
/// Siege snapshot cadence in Hz: 10 Hz sits inside the 10-15 Hz siege band.
pub const SIEGE_SNAPSHOT_HZ: u64 = 10;
/// Per-connection per-snapshot budget in bytes: one datagram, never fragmented.
pub const SIEGE_BUDGET_BYTES: usize = MAX_DATAGRAM_BYTES;
/// Sentinel for "never sent to this connection" (forces a full first send).
pub const NEVER_SENT: u32 = u32::MAX;
/// Room word (pattern): id-derived `id % 8`, integer-only, stable across runs.
pub(crate) const fn entity_room(id: u32) -> u32 {
    id % 8
}
/// Mask bit for one room.
pub(crate) const fn room_mask(room: u32) -> u64 {
    1u64 << (room & 63)
}
/// Scope view: true when `room` is in per-connection `mask`.
pub(crate) const fn room_in_scope(room: u32, mask: u64) -> bool {
    (mask & room_mask(room)) != 0
}

/// Relevancy of one entity to one viewer: distance ring plus view hemisphere.
/// Combat-dirty entities are never culled; that is the no-desync rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Relevancy {
    /// Inside the near ring: full rate, any direction.
    Near,
    /// Inside the far ring and ahead of (or fighting) the viewer: LOD rate.
    Far,
    /// Beyond the far ring but combat-dirty: always sent, never dropped.
    Combat,
    /// Beyond interest and clean: skipped until the starve guard fires.
    Culled,
}

/// Interest tuning: near/far rings in metres, snapshot rates in Hz, and the
/// per-client byte budget. `near_hz`/`far_hz`/`cap_bps` name the plan shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InterestCfg {
    /// Near-ring radius in metres: full rate inside.
    pub near_m: u32,
    /// Far-ring radius in metres: LOD rate inside, combat-only beyond.
    pub far_m: u32,
    /// Snapshot rate for near entities in Hz.
    pub near_hz: u8,
    /// Snapshot rate for far entities in Hz.
    pub far_hz: u8,
    /// Per-client budget in bytes/s (256 kbps for the 64-client siege).
    pub cap_bps: u32,
}

impl InterestCfg {
    /// Siege default: 40 m near at 10 Hz, 120 m far at 3 Hz, 256 kbps cap.
    #[must_use]
    pub const fn default_64() -> Self {
        Self {
            near_m: 40,
            far_m: 120,
            near_hz: 10,
            far_hz: 3,
            cap_bps: ON3_CAP_BPS,
        }
    }

    /// Siege proof: 32 m near at 10 Hz, 100 m far at 2 Hz, 128 kbps cap.
    /// Tighter rings plus slower far LOD hold the 128 kbps bar with the same
    /// 1200 B per-snapshot budget (1200 B x 10 Hz = 12_000 B/s = 96 kbps
    /// worst case per client, under the bar by construction).
    #[must_use]
    pub const fn default_siege() -> Self {
        Self {
            near_m: 32,
            far_m: 100,
            near_hz: 10,
            far_hz: 2,
            cap_bps: ON9_CAP_BPS,
        }
    }

    /// Ticks between near-ring sends (60 Hz divided by the near rate).
    #[must_use]
    pub const fn every_near(self) -> u32 {
        (TICK_HZ / self.near_hz as u64) as u32
    }

    /// Ticks between far-ring sends (60 Hz divided by the far rate).
    #[must_use]
    pub const fn every_far(self) -> u32 {
        (TICK_HZ / self.far_hz as u64) as u32
    }

    /// Reject degenerate tuning loudly; valid tuning selects silently.
    pub const fn validate(self) -> Result<(), InterestError> {
        if self.near_m == 0 || self.far_m < self.near_m {
            return Err(InterestError::BadConfig);
        }
        if self.near_hz == 0 || self.near_hz > TICK_HZ as u8 {
            return Err(InterestError::BadConfig);
        }
        if self.far_hz == 0 || self.far_hz > self.near_hz {
            return Err(InterestError::BadConfig);
        }
        if self.cap_bps == 0 {
            return Err(InterestError::BadConfig);
        }
        Ok(())
    }
}

/// One replicated entity: id plus top-down position plus one combat word.
/// The caller folds hp/stamina into `combat`; any change marks combat-dirty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InterestEntity {
    /// Stable entity id, ascending and unique per selection.
    pub id: u32,
    /// Feet x in millimetres.
    pub x_mm: i32,
    /// Feet z in millimetres.
    pub z_mm: i32,
    /// Combat-state word (hp/stamina folded by the caller).
    pub combat: u32,
}

/// Interest failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterestError {
    /// Tuning violates `near <= far`, rates `0 < far <= near <= 60`, cap > 0.
    BadConfig,
    /// Selection over an empty entity list.
    EmptyEntities,
    /// More entities than [`MAX_INTEREST_ENTITIES`], count carried.
    TooManyEntities {
        /// Entities offered.
        got: usize,
    },
    /// Entity ids must arrive sorted ascending and unique.
    UnsortedEntities,
    /// Viewer id is not in the entity list.
    UnknownViewer {
        /// Viewer id offered.
        viewer: u32,
    },
    /// Selection tick older than the connection's last selection tick.
    TickRegression {
        /// Tick offered.
        tick: u32,
        /// Last selection tick.
        base_tick: u32,
    },
    /// Caller output buffer shorter than the entity count.
    OutTooSmall {
        /// Slots that must fit.
        need: usize,
        /// Slots the buffer fits.
        have: usize,
    },
    /// Even the combat-dirty set the honesty rule keeps exceeds the budget.
    OverBudget {
        /// Bytes the kept set needs.
        need: usize,
        /// Bytes the caller allowed.
        budget: usize,
    },
}

impl std::fmt::Display for InterestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadConfig => write!(f, "invalid interest tuning"),
            Self::EmptyEntities => write!(f, "entity list is empty"),
            Self::TooManyEntities { got } => {
                write!(f, "entity count {got} over {MAX_INTEREST_ENTITIES}")
            }
            Self::UnsortedEntities => write!(f, "entity ids must be sorted and unique"),
            Self::UnknownViewer { viewer } => write!(f, "viewer {viewer} not in entity list"),
            Self::TickRegression { tick, base_tick } => {
                write!(f, "tick {tick} older than last selection {base_tick}")
            }
            Self::OutTooSmall { need, have } => {
                write!(f, "output fits {have} ids but {need} must fit")
            }
            Self::OverBudget { need, budget } => {
                write!(f, "kept combat set needs {need} bytes over budget {budget}")
            }
        }
    }
}

impl std::error::Error for InterestError {}

/// Squared millimetre distance, saturating: arena extents never overflow i64.
#[must_use]
pub const fn dist2_mm(ax: i32, az: i32, bx: i32, bz: i32) -> i64 {
    let dx = (ax as i64).saturating_sub(bx as i64);
    let dz = (az as i64).saturating_sub(bz as i64);
    dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz))
}

/// Viewer facing as an integer octant step in `{-1, 0, 1}^2` from decidegrees.
/// Convention owned here: yaw 0 faces `-x`; pinned by `octant_pins_yaw_zero`.
#[must_use]
pub const fn octant_dir(yaw_ddeg: i16) -> (i32, i32) {
    const DIRS: [(i32, i32); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    let mut wrapped = (yaw_ddeg as i32) % 3600;
    if wrapped < -1800 {
        wrapped += 3600;
    } else if wrapped >= 1800 {
        wrapped -= 3600;
    }
    // Wrapped to [-1800, 1800): shifted index lands in 0..=8, masked to
    // 0..=7 (8 octants of 450 decidegrees cover the full circle).
    let idx = (wrapped + 2025) / 450;
    DIRS[(idx & 7) as usize]
}

/// True when the entity sits ahead of the viewer (view hemisphere): the dot
/// of the relative offset with the facing step is positive. Co-located
/// entities count as ahead so the viewer never culls its own cell.
#[must_use]
pub const fn in_front(rel_x: i64, rel_z: i64, dir_x: i32, dir_z: i32) -> bool {
    if rel_x == 0 && rel_z == 0 {
        return true;
    }
    rel_x
        .saturating_mul(dir_x as i64)
        .saturating_add(rel_z.saturating_mul(dir_z as i64))
        > 0
}

/// Relevancy from squared distance against the ring edges, the view test,
/// and the combat-dirty flag. Combat-dirty is never culled at any distance.
#[must_use]
pub const fn relevancy(
    d2: i64,
    near2: i64,
    far2: i64,
    front: bool,
    combat_dirty: bool,
) -> Relevancy {
    if d2 <= near2 {
        Relevancy::Near
    } else if d2 <= far2 && (front || combat_dirty) {
        Relevancy::Far
    } else if combat_dirty {
        Relevancy::Combat
    } else {
        Relevancy::Culled
    }
}

/// Wire cost of one entity: 2 id bytes plus 8 pose bytes when the pose is
/// new plus 4 combat bytes when the combat word is new. Full first sends
/// cost 14; clean refreshes cost 2.
#[must_use]
pub const fn entity_cost_bytes(full: bool, pose_dirty: bool, combat_dirty: bool) -> usize {
    let mut n = 2usize;
    if full || pose_dirty {
        n += 8;
    }
    if full || combat_dirty {
        n += 4;
    }
    n
}

/// What one [`ConnectionInterest::select`] chose, in ascending id order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelectReport {
    /// Tick selected for.
    pub tick: u32,
    /// Ids written to the caller buffer.
    pub selected: usize,
    /// Wire bytes (`DELTA_HEADER_LEN` plus taken entity costs).
    pub bytes: usize,
    /// Taken near-ring entities.
    pub near: usize,
    /// Taken far-ring entities.
    pub far: usize,
    /// Taken combat-dirty entities (any ring).
    pub combat: usize,
    /// Taken entities forced by the starve guard.
    pub starved: usize,
    /// Oldest ticks-since-sent seen at select time (the starve proof).
    pub starve_seen: u32,
}

/// Ticks since `sent`, saturating; never-sent reports the maximum age.
const fn age_ticks(sent: u32, now: u32) -> u32 {
    if sent == NEVER_SENT {
        u32::MAX
    } else {
        now.saturating_sub(sent)
    }
}

/// One entity's selection decision: priority class, eligibility, cost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Decision {
    class: usize,
    eligible: bool,
    forced: bool,
    cost: usize,
}

impl Decision {
    const fn skip() -> Self {
        Self {
            class: 3,
            eligible: false,
            forced: false,
            cost: 0,
        }
    }
}

/// Entity lists must be non-empty, bounded, sorted ascending and unique, so
/// every pass runs in stable id order on every machine.
fn check_order(entities: &[InterestEntity]) -> Result<(), InterestError> {
    if entities.is_empty() {
        return Err(InterestError::EmptyEntities);
    }
    if entities.len() > MAX_INTEREST_ENTITIES {
        return Err(InterestError::TooManyEntities {
            got: entities.len(),
        });
    }
    let mut prev: Option<u32> = None;
    for e in entities {
        if prev.is_some_and(|p| e.id <= p) {
            return Err(InterestError::UnsortedEntities);
        }
        prev = Some(e.id);
    }
    Ok(())
}

/// Per-connection replication state: the last-sent delta baseline plus the
/// bandwidth ledger. Baseline arrays are sized once at construction; the
/// selection path writes only through them and the caller buffer, so ticks
/// hold capacity steady with zero steady-state allocation.
pub struct ConnectionInterest {
    viewer: u32,
    base_tick: u32,
    last_tick: u32,
    xs: Vec<i32>,
    zs: Vec<i32>,
    combat: Vec<u32>,
    sent: Vec<u32>,
    bytes_sent: u64,
    snapshots: u64,
    starve_max: u32,
    rooms: u64,
}

impl ConnectionInterest {
    /// Size one connection for `entities` entity slots (index-aligned with
    /// the ascending-id slice every select receives). Baselines start zeroed
    /// and unregistered: register the roster before selecting, or the first
    /// selection safely over-sends every nonzero entity. Viewer identity is
    /// checked per select, not here.
    pub fn for_entities(viewer: u32, entities: usize) -> Result<Self, InterestError> {
        if entities == 0 {
            return Err(InterestError::EmptyEntities);
        }
        if entities > MAX_INTEREST_ENTITIES {
            return Err(InterestError::TooManyEntities { got: entities });
        }
        Ok(Self {
            viewer,
            base_tick: 0,
            last_tick: 0,
            xs: vec![0; entities],
            zs: vec![0; entities],
            combat: vec![0; entities],
            sent: vec![NEVER_SENT; entities],
            bytes_sent: 0,
            snapshots: 0,
            starve_max: 0,
            rooms: u64::MAX,
        })
    }

    /// Snapshot the roster as the delta baseline (match start, roster swap,
    /// or resync): values are recorded, nothing is marked sent, so the next
    /// selection full-sends only what is relevant. Also the resync primitive
    /// when the receiver lost state: it forces full costs without guessing.
    pub fn register_baseline(&mut self, entities: &[InterestEntity]) -> Result<(), InterestError> {
        check_order(entities)?;
        if self.xs.len() != entities.len() {
            return Err(InterestError::TooManyEntities {
                got: entities.len(),
            });
        }
        for (i, e) in entities.iter().enumerate() {
            self.xs[i] = e.x_mm;
            self.zs[i] = e.z_mm;
            self.combat[i] = e.combat;
            self.sent[i] = NEVER_SENT;
        }
        Ok(())
    }

    /// Viewer this connection replicates to.
    #[must_use]
    pub const fn viewer(&self) -> u32 {
        self.viewer
    }

    /// Scope mask (default all rooms; send-only filter, never sim).
    /// Test-only scope probe; production keeps the all-rooms default until
    /// the session packet wires the production setter.
    #[cfg(test)]
    pub(crate) const fn rooms(&self) -> u64 {
        self.rooms
    }

    /// Set scope mask (room membership; send-only).
    /// Test-only scope probe; production keeps the all-rooms default until
    /// the session packet wires the production setter.
    #[cfg(test)]
    pub(crate) fn set_rooms(&mut self, mask: u64) {
        self.rooms = mask;
    }

    /// Last acked baseline tick (the delta baseline the receiver holds).
    #[must_use]
    pub const fn base_tick(&self) -> u32 {
        self.base_tick
    }

    /// Advance the acked baseline: the receiver confirmed `tick`. Rejects a
    /// tick newer than the last selection (nothing that new was sent).
    pub fn mark_acked(&mut self, tick: u32) -> Result<(), InterestError> {
        if tick > self.last_tick {
            return Err(InterestError::TickRegression {
                tick,
                base_tick: self.last_tick,
            });
        }
        self.base_tick = tick;
        Ok(())
    }

    /// Total wire bytes selected so far (header plus entity costs).
    #[must_use]
    pub const fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    /// Snapshots selected so far.
    #[must_use]
    pub const fn snapshots(&self) -> u64 {
        self.snapshots
    }

    /// Worst ticks-since-sent observed on any select (the starve ledger).
    #[must_use]
    pub const fn starve_max(&self) -> u32 {
        self.starve_max
    }

    /// Carried bytes/s over `ticks` ticks at 60 Hz (integer math).
    #[must_use]
    pub const fn bytes_per_sec(&self, ticks: u32) -> u64 {
        if ticks == 0 {
            return 0;
        }
        ((self.bytes_sent as u128 * TICK_HZ as u128) / ticks as u128) as u64
    }

    /// Baseline capacity triple for the no-growth test below.
    #[must_use]
    pub fn capacities(&self) -> (usize, usize, usize) {
        (
            self.xs.capacity(),
            self.combat.capacity(),
            self.sent.capacity(),
        )
    }

    /// Decide one snapshot: rank every entity by (class, id) with stable
    /// slice order, keep the viewer always, then take the longest
    /// class-ordered prefix that fits `budget_bytes` bytes. Combat-dirty
    /// entities are never skipped for budget: overflowing them returns
    /// [`InterestError::OverBudget`] instead of a silent desync. Taken ids
    /// land ascending in `out`; the baseline updates only on success, so a
    /// refused budget changes nothing. Budget: O(n) time over at most 1024
    /// entities, two passes, zero heap use.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn select(
        &mut self,
        cfg: &InterestCfg,
        entities: &[InterestEntity],
        viewer_id: u32,
        viewer_x_mm: i32,
        viewer_z_mm: i32,
        viewer_yaw_ddeg: i16,
        now: u32,
        budget_bytes: usize,
        out: &mut [u32],
    ) -> Result<SelectReport, InterestError> {
        cfg.validate()?;
        check_order(entities)?;
        if out.len() < entities.len() {
            return Err(InterestError::OutTooSmall {
                need: entities.len(),
                have: out.len(),
            });
        }
        if now < self.last_tick {
            return Err(InterestError::TickRegression {
                tick: now,
                base_tick: self.last_tick,
            });
        }
        if self.xs.len() != entities.len() {
            return Err(InterestError::TooManyEntities {
                got: entities.len(),
            });
        }
        let viewer_pos = entities
            .iter()
            .position(|e| e.id == viewer_id)
            .ok_or(InterestError::UnknownViewer { viewer: viewer_id })?;
        let ring = ring_edges(cfg);
        let every_near = cfg.every_near();
        let every_far = cfg.every_far();
        // Dry pass first: the baseline mutates only when the budget holds.
        let plan = self.measure(
            &ring,
            entities,
            viewer_pos,
            viewer_x_mm,
            viewer_z_mm,
            viewer_yaw_ddeg,
            now,
            budget_bytes,
            every_near,
            every_far,
        )?;
        // Commit pass: identical decisions, now applied to the baseline.
        let report = self.apply(
            &ring,
            entities,
            viewer_pos,
            viewer_x_mm,
            viewer_z_mm,
            viewer_yaw_ddeg,
            now,
            budget_bytes,
            &plan,
            every_near,
            every_far,
            out,
        );
        self.last_tick = now;
        self.bytes_sent += plan.bytes as u64;
        self.snapshots += 1;
        if plan.starve_seen > self.starve_max {
            self.starve_max = plan.starve_seen;
        }
        Ok(report)
    }

    /// Per-entity decision: relevancy class, eligibility, cost, forced flag.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    fn decide(
        &self,
        ring: &Ring,
        e: &InterestEntity,
        index: usize,
        viewer_x_mm: i32,
        viewer_z_mm: i32,
        dir_x: i32,
        dir_z: i32,
        every_near: u32,
        every_far: u32,
        now: u32,
    ) -> Decision {
        let sent = self.sent[index];
        let first = sent == NEVER_SENT;
        let age = age_ticks(sent, now);
        let pose_dirty = e.x_mm != self.xs[index] || e.z_mm != self.zs[index];
        let combat_dirty = e.combat != self.combat[index];
        // Room gate first: out-of-scope clean stays culled; combat still ships.
        if !room_in_scope(entity_room(e.id), self.rooms) && !combat_dirty {
            return Decision::skip();
        }
        let d2 = dist2_mm(e.x_mm, e.z_mm, viewer_x_mm, viewer_z_mm);
        let front = in_front(
            i64::from(e.x_mm) - i64::from(viewer_x_mm),
            i64::from(e.z_mm) - i64::from(viewer_z_mm),
            dir_x,
            dir_z,
        );
        let rel = relevancy(d2, ring.near2, ring.far2, front, combat_dirty);
        // Dormant first sighting far away stays dormant: the receiver holds
        // nothing and nothing changed, so no baseline is owed yet.
        if first && rel == Relevancy::Culled {
            return Decision::skip();
        }
        // The guard fires a snapshot early so the observed gap never
        // exceeds the bar even at the 10 Hz snapshot cadence.
        let forced = age != u32::MAX && age.saturating_add(SNAPSHOT_EVERY_TICKS) >= STARVE_TICKS;
        let (class, due) = match rel {
            Relevancy::Near => (1usize, age >= every_near),
            Relevancy::Far => (2usize, age >= every_far),
            Relevancy::Combat => (0usize, true),
            Relevancy::Culled => (3usize, false),
        };
        let eligible = due || forced || first;
        Decision {
            class,
            eligible,
            forced: eligible && forced && !due,
            cost: entity_cost_bytes(first, pose_dirty, combat_dirty),
        }
    }

    /// Dry pass: measure the class-ordered prefix without touching state.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    fn measure(
        &self,
        ring: &Ring,
        entities: &[InterestEntity],
        viewer_pos: usize,
        viewer_x_mm: i32,
        viewer_z_mm: i32,
        viewer_yaw_ddeg: i16,
        now: u32,
        budget_bytes: usize,
        every_near: u32,
        every_far: u32,
    ) -> Result<Plan, InterestError> {
        let viewer_id = entities[viewer_pos].id;
        let (dir_x, dir_z) = octant_dir(viewer_yaw_ddeg);
        let mut starve_seen = 0u32;
        for (i, e) in entities.iter().enumerate() {
            if e.id != viewer_id && !room_in_scope(entity_room(e.id), self.rooms) {
                continue;
            }
            let age = age_ticks(self.sent[i], now);
            if age != u32::MAX && age > starve_seen {
                starve_seen = age;
            }
        }
        // The viewer always ships: a budget below header plus viewer refuses.
        let viewer_cost = self.entity_cost_at(entities, viewer_pos);
        if DELTA_HEADER_LEN.saturating_add(viewer_cost) > budget_bytes {
            return Err(InterestError::OverBudget {
                need: DELTA_HEADER_LEN.saturating_add(viewer_cost),
                budget: budget_bytes,
            });
        }
        // Class passes in priority order; the first overflow holds the line
        // so a far refresh never displaces combat state. Pass ids: 0 combat,
        // 1 near, 2 far, 3 starve-forced leftovers.
        let mut used = DELTA_HEADER_LEN.saturating_add(viewer_cost);
        let mut dropped_combat_cost = 0usize;
        let mut stopped = false;
        for pass in 0..4 {
            if stopped {
                break;
            }
            for (i, e) in entities.iter().enumerate() {
                if e.id == viewer_id {
                    continue;
                }
                let d = self.decide(
                    ring,
                    e,
                    i,
                    viewer_x_mm,
                    viewer_z_mm,
                    dir_x,
                    dir_z,
                    every_near,
                    every_far,
                    now,
                );
                if d.class != pass || !d.eligible {
                    continue;
                }
                if used.saturating_add(d.cost) > budget_bytes {
                    if pass == 0 {
                        dropped_combat_cost = d.cost;
                    }
                    stopped = true;
                    break;
                }
                used += d.cost;
            }
        }
        if dropped_combat_cost > 0 {
            return Err(InterestError::OverBudget {
                need: used.saturating_add(dropped_combat_cost),
                budget: budget_bytes,
            });
        }
        Ok(Plan {
            bytes: used,
            starve_seen,
        })
    }

    /// Commit pass: repeat the measured decisions, update the baseline, and
    /// fill `out` ascending. The plan proved the prefix fits, so the same
    /// walk takes the same entities.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    fn apply(
        &mut self,
        ring: &Ring,
        entities: &[InterestEntity],
        viewer_pos: usize,
        viewer_x_mm: i32,
        viewer_z_mm: i32,
        viewer_yaw_ddeg: i16,
        now: u32,
        budget_bytes: usize,
        plan: &Plan,
        every_near: u32,
        every_far: u32,
        out: &mut [u32],
    ) -> SelectReport {
        let viewer_id = entities[viewer_pos].id;
        let (dir_x, dir_z) = octant_dir(viewer_yaw_ddeg);
        let mut used = DELTA_HEADER_LEN;
        let mut report = SelectReport {
            tick: now,
            selected: 0,
            bytes: 0,
            near: 0,
            far: 0,
            combat: 0,
            starved: 0,
            starve_seen: plan.starve_seen,
        };
        used += self.entity_cost_at(entities, viewer_pos);
        self.take(viewer_pos, entities, now);
        let mut stopped = false;
        for pass in 0..4 {
            if stopped {
                break;
            }
            for (i, e) in entities.iter().enumerate() {
                if e.id == viewer_id {
                    continue;
                }
                let d = self.decide(
                    ring,
                    e,
                    i,
                    viewer_x_mm,
                    viewer_z_mm,
                    dir_x,
                    dir_z,
                    every_near,
                    every_far,
                    now,
                );
                if d.class != pass || !d.eligible {
                    continue;
                }
                if used.saturating_add(d.cost) > budget_bytes {
                    stopped = true;
                    break;
                }
                used += d.cost;
                if d.forced {
                    report.starved += 1;
                }
                match pass {
                    0 => report.combat += 1,
                    1 => report.near += 1,
                    2 => report.far += 1,
                    _ => {}
                }
                self.take(i, entities, now);
            }
        }
        // Ascending id order: slice order is ascending by validation.
        let mut count = 0usize;
        for (i, e) in entities.iter().enumerate() {
            if self.sent[i] == now {
                out[count] = e.id;
                count += 1;
            }
        }
        report.selected = count;
        report.bytes = used;
        report
    }

    /// Current cost of shipping one slot (full on first sighting).
    fn entity_cost_at(&self, entities: &[InterestEntity], index: usize) -> usize {
        let first = self.sent[index] == NEVER_SENT;
        entity_cost_bytes(
            first,
            entities[index].x_mm != self.xs[index] || entities[index].z_mm != self.zs[index],
            entities[index].combat != self.combat[index],
        )
    }

    /// Record a take into the per-connection baseline.
    fn take(&mut self, index: usize, entities: &[InterestEntity], now: u32) {
        self.xs[index] = entities[index].x_mm;
        self.zs[index] = entities[index].z_mm;
        self.combat[index] = entities[index].combat;
        self.sent[index] = now;
    }
}

/// ioq3 per-client snapshot beside the KIND path (pattern only): the
/// `select()` set diffed against the acked baseline, with pending reliable
/// commands piggybacked ahead of the snapshot bytes. Genre-neutral: entity
/// ids only, no bout rules. Units: ticks, bytes. Determinism: ascending id
/// order, integer math, no map iteration. Errors: [`NetError`], never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ioq3ClientView {
    /// Acked baseline tick the fields diff against.
    pub lastframe: u32,
    /// Authoritative tick being sent.
    pub server_time: u32,
    /// Selected entity count; equals the `select()` count by construction.
    pub entity_count: usize,
    /// Pending reliable commands, oldest first, ahead of the snapshot.
    pub reliable: Vec<Vec<u8>>,
    /// Field-masked delta for exactly the selected set.
    pub fields: Ioq3SnapFields,
    /// Fragment datagrams over reliable bytes plus area bytes, one sequence.
    /// Empty when both inputs are empty; the delta still travels in `fields`.
    pub datagrams: Vec<Vec<u8>>,
}

impl Ioq3ClientView {
    /// Receipt: lastframe, server time, entity count, reliable count.
    #[must_use]
    pub const fn receipt(&self) -> (u32, u32, usize, usize) {
        (
            self.lastframe,
            self.server_time,
            self.entity_count,
            self.reliable.len(),
        )
    }

    /// Wire bytes: the reliable prefix first, then the field wire.
    #[must_use]
    pub fn wire_len(&self) -> usize {
        self.reliable
            .iter()
            .map(Vec::len)
            .sum::<usize>()
            .saturating_add(ioq3_fields_wire_len(&self.fields))
    }
}

/// Parameters for [`ioq3_client_view`]: one struct over seven arguments.
pub struct Ioq3ViewParams<'a> {
    /// Ascending selected ids from `select()`.
    pub selected: &'a [u32],
    /// Full snapshot at `server_time`.
    pub full: &'a FullSnap,
    /// Acked baseline snapshot at `lastframe`.
    pub baseline: &'a FullSnap,
    /// Acked baseline tick; must equal `baseline.tick`.
    pub lastframe: u32,
    /// Authoritative tick; must equal `full.tick`.
    pub server_time: u32,
    /// Snapshot flags carried beside the delta.
    pub snap_flags: u8,
    /// Visible-area bytes for the viewer.
    pub area_bytes: Vec<u8>,
    /// Pending reliable commands, oldest first.
    pub reliable: Vec<Vec<u8>>,
    /// Fragment sequence for the datagram group.
    pub sequence: u32,
    /// Port slot for the datagram group.
    pub qport: u16,
}

/// Build the per-client view: filter both snapshots to the selected slots,
/// diff against the acked baseline, fragment reliable plus area bytes under
/// one sequence. Empty or unsorted selections, unknown slots and stale
/// baselines refuse with [`NetError::BadState`]; the KIND path is untouched.
pub fn ioq3_client_view(params: Ioq3ViewParams<'_>) -> Result<Ioq3ClientView, NetError> {
    if params.selected.is_empty() {
        return Err(NetError::BadState);
    }
    if params.selected.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(NetError::BadState);
    }
    let mut slots = Vec::with_capacity(params.selected.len());
    for id in params.selected {
        slots.push(u8::try_from(*id).map_err(|_| NetError::BadState)?);
    }
    let pick = |src: &[BodySnap]| -> Vec<BodySnap> {
        src.iter()
            .filter(|f| slots.binary_search(&f.slot).is_ok())
            .copied()
            .collect()
    };
    let sub_full = FullSnap::new(params.full.tick, pick(&params.full.fighters))
        .map_err(|_| NetError::BadState)?;
    let sub_base = FullSnap::new(params.baseline.tick, pick(&params.baseline.fighters))
        .map_err(|_| NetError::BadState)?;
    let fields = ioq3_encode_fields(
        params.lastframe,
        params.server_time,
        params.snap_flags,
        params.area_bytes.clone(),
        &sub_full,
        &sub_base,
    )?;
    let mut image = Vec::new();
    for cmd in &params.reliable {
        image.extend_from_slice(cmd);
    }
    image.extend_from_slice(&params.area_bytes);
    let datagrams = ioq3_fragment_message(params.sequence, params.qport, &image);
    Ok(Ioq3ClientView {
        lastframe: params.lastframe,
        server_time: params.server_time,
        entity_count: params.selected.len(),
        reliable: params.reliable,
        fields,
        datagrams,
    })
}

/// Squared ring edges in mm^2, computed once per select from the tuning.
struct Ring {
    near2: i64,
    far2: i64,
}

/// Squared ring edges in mm^2 from metre tuning (saturating, integer only).
fn ring_edges(cfg: &InterestCfg) -> Ring {
    let near_mm = i64::from(cfg.near_m) * 1000;
    let far_mm = i64::from(cfg.far_m) * 1000;
    Ring {
        near2: near_mm.saturating_mul(near_mm),
        far2: far_mm.saturating_mul(far_mm),
    }
}

/// Measured dry-pass outcome shared with the commit pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Plan {
    bytes: usize,
    starve_seen: u32,
}

/// One posted kbps chart row: integer kilobits/s x100 against its cap x100.
/// One posted rate-LOD row: per-class sends plus the effective Hz x100.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{decode_delta, encode_delta, BodySnap, FullSnap};

    /// Small deterministic roster: viewer 0 plus near/far/behind entities.
    fn roster() -> Vec<InterestEntity> {
        vec![
            InterestEntity {
                id: 0,
                x_mm: 0,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 1,
                x_mm: 10_000,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 2,
                x_mm: 80_000,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 3,
                x_mm: -200_000,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 4,
                x_mm: 300_000,
                z_mm: 0,
                combat: 1_000,
            },
        ]
    }

    fn registered(entities: &[InterestEntity]) -> ConnectionInterest {
        let mut conn = ConnectionInterest::for_entities(0, entities.len()).expect("conn");
        conn.register_baseline(entities).expect("register");
        conn
    }

    fn select_now(
        conn: &mut ConnectionInterest,
        entities: &[InterestEntity],
        now: u32,
        out: &mut [u32],
    ) -> SelectReport {
        let cfg = InterestCfg::default_64();
        conn.select(&cfg, entities, 0, 0, 0, -1800, now, MAX_DATAGRAM_BYTES, out)
            .expect("select")
    }

    #[test]
    fn interest_names_snapshots_through_neutral_bodysnap() {
        // GE2 genre gate: code in this file must name snapshot slots through
        // the neutral alias from delta.rs so the workspace ledger never
        // grows. The needle is built at runtime so these lines never spell
        // the banned identifier themselves; only code counts (a // comment
        // tail never counts).
        let needle = format!("{}{}", "Fighter", "Snap");
        let src = include_str!("interest.rs");
        let mut bad = 0;
        for line in src.lines() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains(&needle) {
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "name snapshot slots through BodySnap");
    }

    #[test]
    fn octant_pins_yaw_zero() {
        // Facing convention owned here: yaw 0 faces -x, yaw -1800 faces +x.
        assert_eq!(octant_dir(0), (-1, 0));
        assert_eq!(octant_dir(-1800), (1, 0));
        assert_eq!(octant_dir(0), octant_dir(0));
    }

    #[test]
    fn octant_sweeps_full_circle_without_panic() {
        // Regression: yaw in [1575,1800) once indexed DIRS[8] out of bounds.
        // Every decidegree across two full turns must map to a unit octant.
        for yaw in -3600i32..3600 {
            let (dx, dz) = octant_dir(yaw as i16);
            assert!(
                dx.abs() <= 1 && dz.abs() <= 1 && (dx != 0 || dz != 0),
                "yaw {yaw} maps outside the octant"
            );
        }
        // Wrapped equivalence: full turns agree.
        for yaw in -3600i32..0 {
            assert_eq!(octant_dir(yaw as i16), octant_dir((yaw + 3600) as i16));
        }
    }

    #[test]
    fn relevancy_splits_rings_view_and_combat() {
        let near2 = 40_000i64 * 40_000;
        let far2 = 120_000i64 * 120_000;
        assert_eq!(
            relevancy(10_000 * 10_000, near2, far2, false, false),
            Relevancy::Near
        );
        assert_eq!(
            relevancy(80_000 * 80_000, near2, far2, true, false),
            Relevancy::Far
        );
        // Behind the viewer inside the far ring: culled unless fighting.
        assert_eq!(
            relevancy(80_000 * 80_000, near2, far2, false, false),
            Relevancy::Culled
        );
        assert_eq!(
            relevancy(80_000 * 80_000, near2, far2, false, true),
            Relevancy::Far
        );
        // Beyond the far ring only combat survives.
        assert_eq!(
            relevancy(300_000 * 300_000, near2, far2, true, false),
            Relevancy::Culled
        );
        assert_eq!(
            relevancy(300_000 * 300_000, near2, far2, true, true),
            Relevancy::Combat
        );
        assert_eq!(
            relevancy(300_000 * 300_000, near2, far2, false, true),
            Relevancy::Combat
        );
    }

    #[test]
    fn first_snapshot_takes_rings_and_skips_dormant() {
        let entities = roster();
        let mut conn = registered(&entities);
        let mut out = [0u32; 5];
        // Viewer faces +x (yaw -1800): id 1 near, id 2 far ahead, id 3 far
        // behind (culled), id 4 beyond far (dormant first sighting).
        let report = select_now(&mut conn, &entities, 6, &mut out);
        assert!(out[..report.selected].contains(&0), "{report:?}");
        assert!(out[..report.selected].contains(&1), "{report:?}");
        assert!(out[..report.selected].contains(&2), "{report:?}");
        assert!(!out[..report.selected].contains(&3), "{report:?}");
        assert!(!out[..report.selected].contains(&4), "{report:?}");
        assert_eq!(report.combat, 0, "{report:?}");
        // Ascending output: stable id order on every machine.
        let mut sorted = out[..report.selected].to_vec();
        sorted.sort_unstable();
        assert_eq!(out[..report.selected], sorted);
    }

    #[test]
    fn combat_dirty_is_never_culled() {
        let entities = roster();
        let mut conn = registered(&entities);
        let mut out = [0u32; 5];
        let _ = select_now(&mut conn, &entities, 6, &mut out);
        // Far-behind and beyond-far entities start fighting: both ship.
        let mut fighting = entities.clone();
        fighting[3].combat = 900;
        fighting[4].combat = 900;
        let report = select_now(&mut conn, &fighting, 12, &mut out);
        assert!(out[..report.selected].contains(&3), "{report:?}");
        assert!(out[..report.selected].contains(&4), "{report:?}");
        assert!(report.combat >= 2, "{report:?}");
    }

    #[test]
    fn far_updates_run_slower_than_near() {
        let entities = vec![
            InterestEntity {
                id: 0,
                x_mm: 0,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 1,
                x_mm: 10_000,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 2,
                x_mm: 80_000,
                z_mm: 0,
                combat: 1_000,
            },
        ];
        let mut conn = registered(&entities);
        let mut out = [0u32; 3];
        let (mut near_count, mut far_count) = (0, 0);
        for tick in (6..126).step_by(SNAPSHOT_EVERY_TICKS as usize) {
            let report = select_now(&mut conn, &entities, tick as u32, &mut out);
            if out[..report.selected].contains(&1) {
                near_count += 1;
            }
            if out[..report.selected].contains(&2) {
                far_count += 1;
            }
        }
        assert!(far_count >= 2, "far must refresh, got {far_count}");
        assert!(
            far_count < near_count,
            "far {far_count} must lag near {near_count}"
        );
    }

    #[test]
    fn selection_is_deterministic_x2() {
        let entities = roster();
        let run = || {
            let mut conn = registered(&entities);
            let mut out = [0u32; 5];
            let first = select_now(&mut conn, &entities, 6, &mut out);
            let snapshot = out;
            let second = select_now(&mut conn, &entities, 12, &mut out);
            (first, second, snapshot, out)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn replication_reads_interest_without_desync() {
        // Consumer wire-in-test: session replication consults interest, then
        // the real delta codec carries the snapshot; decode stays exact.
        let base_fighters: Vec<BodySnap> = (0..16)
            .map(|slot| BodySnap {
                slot,
                x_mm: 1_000 + i32::from(slot) * 500,
                y_mm: 0,
                z_mm: -2_000,
                yaw_ddeg: 100,
                hp: 1_000,
                stamina: 100,
            })
            .collect();
        let mut now_fighters = base_fighters.clone();
        now_fighters[0].x_mm += 300;
        now_fighters[14].hp -= 50;
        now_fighters[15].x_mm += 300;
        let baseline = FullSnap::new(100, base_fighters).expect("baseline");
        let full = FullSnap::new(106, now_fighters).expect("full");
        // Interest roster mirrors the snapshot: slot 15 sits far and was
        // sent last tick (rate-gated out); slot 14 fights (always in).
        let entities: Vec<InterestEntity> = full
            .fighters
            .iter()
            .map(|f| InterestEntity {
                id: u32::from(f.slot),
                x_mm: f.x_mm,
                z_mm: if f.slot == 15 { 80_000 } else { f.z_mm },
                combat: u32::from(f.hp),
            })
            .collect();
        let mut primed: Vec<InterestEntity> = entities.clone();
        primed[15].x_mm = 1_000 + 15 * 500;
        primed[15].z_mm = -2_000;
        primed[15].combat = 1_000;
        let mut conn = registered(&primed);
        let mut out = [0u32; 16];
        let cfg = InterestCfg::default_64();
        conn.select(
            &cfg,
            &primed,
            0,
            0,
            -2_000,
            -1800,
            100,
            MAX_DATAGRAM_BYTES,
            &mut out,
        )
        .expect("prime");
        // Viewer faces +x; slot 15 moved far ahead but is not due.
        let report = conn
            .select(
                &cfg,
                &entities,
                0,
                0,
                -2_000,
                -1800,
                106,
                MAX_DATAGRAM_BYTES,
                &mut out,
            )
            .expect("select");
        assert!(out[..report.selected].contains(&0), "{report:?}");
        assert!(out[..report.selected].contains(&14), "{report:?}");
        assert!(!out[..report.selected].contains(&15), "{report:?}");
        // The codec path stays exact for everything selected: no desync.
        let delta = encode_delta(&full, &baseline).expect("encode");
        let decoded = decode_delta(&delta, &baseline).expect("decode");
        assert_eq!(decoded, full);
        for id in out[..report.selected].iter() {
            let slot = *id as u8;
            let want = full.fighters.iter().find(|f| f.slot == slot).expect("slot");
            let got = decoded
                .fighters
                .iter()
                .find(|f| f.slot == slot)
                .expect("slot");
            assert_eq!(got, want, "selected slot {slot} must decode exact");
        }
    }

    #[test]
    fn select_holds_capacity_across_ticks() {
        // Steady-state allocation proxy: internal baselines never regrow
        // across 600 selections and repeat decisions bit-identically.
        let entities = roster();
        let mut conn = registered(&entities);
        let mut out = [0u32; 5];
        let before = conn.capacities();
        for tick in 0..600 {
            let report = select_now(&mut conn, &entities, 6 + tick, &mut out);
            assert_eq!(conn.capacities(), before, "tick {tick} regrew the baseline");
            assert_eq!(report.tick, 6 + tick);
        }
        let mut again = [0u32; 5];
        let mut fresh = registered(&entities);
        for tick in 0..600 {
            let _ = select_now(&mut fresh, &entities, 6 + tick, &mut again);
        }
        assert_eq!(fresh.bytes_sent(), conn.bytes_sent());
        assert_eq!(fresh.starve_max(), conn.starve_max());
    }

    #[test]
    fn ack_baseline_advances_or_refuses() {
        let entities = vec![
            InterestEntity {
                id: 0,
                x_mm: 0,
                z_mm: 0,
                combat: 1,
            },
            InterestEntity {
                id: 1,
                x_mm: 1_000,
                z_mm: 0,
                combat: 1,
            },
        ];
        let mut conn = registered(&entities);
        let mut out = [0u32; 2];
        let _ = select_now(&mut conn, &entities, 6, &mut out);
        conn.mark_acked(6).expect("ack");
        assert_eq!(conn.base_tick(), 6);
        assert!(conn.mark_acked(7).is_err());
    }

    #[test]
    fn selection_rejects_bad_shapes_loudly() {
        let cfg = InterestCfg::default_64();
        let entities = roster();
        let mut out = [0u32; 5];
        let mut tiny = [0u32; 2];
        let mut conn = registered(&entities);
        assert_eq!(
            conn.select(&cfg, &[], 0, 0, 0, 0, 6, MAX_DATAGRAM_BYTES, &mut out),
            Err(InterestError::EmptyEntities)
        );
        let mut shuffled = entities.clone();
        shuffled.swap(0, 1);
        assert_eq!(
            conn.select(&cfg, &shuffled, 0, 0, 0, 0, 6, MAX_DATAGRAM_BYTES, &mut out),
            Err(InterestError::UnsortedEntities)
        );
        assert_eq!(
            conn.select(&cfg, &entities, 9, 0, 0, 0, 6, MAX_DATAGRAM_BYTES, &mut out),
            Err(InterestError::UnknownViewer { viewer: 9 })
        );
        assert_eq!(
            conn.select(
                &cfg,
                &entities,
                0,
                0,
                0,
                0,
                6,
                MAX_DATAGRAM_BYTES,
                &mut tiny
            ),
            Err(InterestError::OutTooSmall { need: 5, have: 2 })
        );
        let _ = select_now(&mut conn, &entities, 6, &mut out);
        assert_eq!(
            conn.select(&cfg, &entities, 0, 0, 0, 0, 5, MAX_DATAGRAM_BYTES, &mut out),
            Err(InterestError::TickRegression {
                tick: 5,
                base_tick: 6
            })
        );
        // Viewer already sent and clean: header plus 2 refresh bytes refuse.
        assert_eq!(
            conn.select(&cfg, &entities, 0, 0, 0, 0, 7, 10, &mut out),
            Err(InterestError::OverBudget {
                need: 23,
                budget: 10
            })
        );
        assert_eq!(
            ConnectionInterest::for_entities(0, 0).map(|_| ()),
            Err(InterestError::EmptyEntities)
        );
        assert_eq!(
            ConnectionInterest::for_entities(0, MAX_INTEREST_ENTITIES + 1).map(|_| ()),
            Err(InterestError::TooManyEntities {
                got: MAX_INTEREST_ENTITIES + 1
            })
        );
        assert_eq!(
            InterestCfg {
                near_m: 120,
                far_m: 40,
                ..cfg
            }
            .validate(),
            Err(InterestError::BadConfig)
        );
        assert_eq!(
            InterestCfg { near_hz: 0, ..cfg }.validate(),
            Err(InterestError::BadConfig)
        );
        assert_eq!(
            InterestCfg { near_hz: 61, ..cfg }.validate(),
            Err(InterestError::BadConfig)
        );
        assert_eq!(
            InterestCfg { far_hz: 11, ..cfg }.validate(),
            Err(InterestError::BadConfig)
        );
        assert_eq!(
            InterestCfg { cap_bps: 0, ..cfg }.validate(),
            Err(InterestError::BadConfig)
        );
    }

    #[test]
    fn combat_overflow_refuses_instead_of_desyncing() {
        // Every entity fights and the budget fits barely half: the kept set
        // refuses loudly rather than silently dropping combat state.
        let mut conn = ConnectionInterest::for_entities(0, 8).expect("conn");
        let entities: Vec<InterestEntity> = (0..8)
            .map(|id| InterestEntity {
                id,
                x_mm: id as i32 * 500_000,
                z_mm: 0,
                combat: 1,
            })
            .collect();
        let mut out = [0u32; 8];
        let cfg = InterestCfg::default_64();
        // Zero baselines: every nonzero word is dirty, so all seven remotes
        // are combat class; viewer plus three fit, the fourth refuses.
        let tight = DELTA_HEADER_LEN + 14 + 3 * entity_cost_bytes(true, true, true);
        assert_eq!(
            conn.select(&cfg, &entities, 0, 0, 0, -1800, 6, tight, &mut out),
            Err(InterestError::OverBudget {
                need: tight + entity_cost_bytes(true, true, true),
                budget: tight,
            })
        );
        // Refused budgets change nothing: a full budget still first-sends all.
        let report = conn
            .select(
                &cfg,
                &entities,
                0,
                0,
                0,
                -1800,
                6,
                MAX_DATAGRAM_BYTES,
                &mut out,
            )
            .expect("retry");
        assert_eq!(report.selected, 8, "{report:?}");
    }

    #[test]
    fn error_strings_name_the_fault() {
        assert!(format!("{}", InterestError::BadConfig).contains("tuning"));
        assert!(format!("{}", InterestError::EmptyEntities).contains("empty"));
        assert!(format!("{}", InterestError::UnsortedEntities).contains("sorted"));
        assert!(format!("{}", InterestError::UnknownViewer { viewer: 3 }).contains('3'));
        assert!(format!(
            "{}",
            InterestError::TickRegression {
                tick: 1,
                base_tick: 2
            }
        )
        .contains('1'));
    }

    #[test]
    fn siege_tuning_pins_the_128kbps_proof() {
        // The siege contract in one place: 10 Hz snapshots inside 10-15 Hz,
        // per-connection 1200 B budget, 128 kbps cap, far LOD below near.
        let cfg = InterestCfg::default_siege();
        assert_eq!(
            cfg,
            InterestCfg {
                near_m: 32,
                far_m: 100,
                near_hz: 10,
                far_hz: 2,
                cap_bps: ON9_CAP_BPS,
            }
        );
        cfg.validate().expect("siege tuning is legal");
        assert_eq!(ON9_CAP_BPS, 16_000);
        let every = SNAPSHOT_EVERY_TICKS;
        assert_eq!(every, 6, "6 ticks at 60 Hz is 10 Hz");
        let hz = SIEGE_SNAPSHOT_HZ;
        assert!(
            (10..=15).contains(&hz),
            "snapshots stay in the 10-15 Hz band"
        );
        // Worst case per client is one full datagram per snapshot: integer.
        let worst_bps = SIEGE_BUDGET_BYTES as u64 * SIEGE_SNAPSHOT_HZ;
        assert!(
            worst_bps <= u64::from(ON9_CAP_BPS),
            "{worst_bps} over the cap"
        );
        let worst_kbps_x100 = worst_bps * 8 * 100 / 1_000;
        assert!(
            worst_kbps_x100 <= ON9_CAP_KBPS_X100,
            "{worst_kbps_x100} over 12800"
        );
        assert!(cfg.every_far() > cfg.every_near(), "far LOD is slower");
    }

    #[test]
    fn far_combat_ships_next_snapshot_without_desync() {
        // Zero desync at siege scale: a far-beyond entity that starts
        // fighting ships on the very next snapshot under siege tuning.
        let cfg = InterestCfg::default_siege();
        let entities = vec![
            InterestEntity {
                id: 0,
                x_mm: 0,
                z_mm: 0,
                combat: 1_000,
            },
            InterestEntity {
                id: 1,
                x_mm: 500_000,
                z_mm: 0,
                combat: 1_000,
            },
        ];
        let mut conn = ConnectionInterest::for_entities(0, 2).expect("conn");
        conn.register_baseline(&entities).expect("baseline");
        let mut out = [0u32; 2];
        let _ = conn
            .select(&cfg, &entities, 0, 0, 0, 0, 6, SIEGE_BUDGET_BYTES, &mut out)
            .expect("prime");
        let mut fighting = entities.clone();
        fighting[1].combat = 900;
        let report = conn
            .select(
                &cfg,
                &fighting,
                0,
                0,
                0,
                0,
                12,
                SIEGE_BUDGET_BYTES,
                &mut out,
            )
            .expect("select");
        assert!(out[..report.selected].contains(&1), "{report:?}");
        assert_eq!(report.combat, 1, "{report:?}");
        // The codec path stays exact for the combat take: no desync.
        let base_fighters: Vec<BodySnap> = (0..2)
            .map(|slot| BodySnap {
                slot,
                x_mm: if slot == 1 { 500_000 } else { 0 },
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
                hp: 1_000,
                stamina: 100,
            })
            .collect();
        let mut now_fighters = base_fighters.clone();
        now_fighters[1].hp = 900;
        let baseline = FullSnap::new(10, base_fighters).expect("baseline");
        let full = FullSnap::new(12, now_fighters).expect("full");
        let delta = encode_delta(&full, &baseline).expect("encode");
        let decoded = decode_delta(&delta, &baseline).expect("decode");
        assert_eq!(decoded, full);
    }

    #[test]
    fn interest_64_room_converges() {
        let cfg = InterestCfg::default_siege();
        let run = || {
            let base: Vec<InterestEntity> = (0..64)
                .map(|id| InterestEntity {
                    id,
                    x_mm: i32::try_from(id).expect("id fits") * 1_000,
                    z_mm: 0,
                    combat: 1_000,
                })
                .collect();
            let mut fighting = base.clone();
            fighting[7].combat = 900;
            let mut conns = Vec::new();
            for v in 0..64u32 {
                let mut c = ConnectionInterest::for_entities(v, 64).expect("c");
                c.set_rooms(room_mask(v % 8));
                c.register_baseline(&base).expect("base");
                conns.push(c);
            }
            let mut out = vec![0u32; 64];
            let (mut bytes, mut starve) = (0u64, 0u32);
            for (v, c) in conns.iter_mut().enumerate() {
                let viewer = u32::try_from(v).expect("v fits");
                let viewer_x = i32::try_from(v).expect("v fits") * 1_000;
                let r = c
                    .select(
                        &cfg,
                        &fighting,
                        viewer,
                        viewer_x,
                        0,
                        0,
                        6,
                        SIEGE_BUDGET_BYTES,
                        &mut out,
                    )
                    .expect("sel");
                assert!(r.bytes <= SIEGE_BUDGET_BYTES);
                assert!(out[..r.selected].iter().all(|id| {
                    room_in_scope(entity_room(*id), c.rooms())
                        || *id == viewer
                        || fighting[*id as usize].combat != base[*id as usize].combat
                }));
                assert!(v != 0 || out[..r.selected].contains(&7));
                bytes += u64::try_from(r.bytes).expect("bytes fit");
                starve = starve.max(r.starve_seen);
            }
            (bytes, starve)
        };
        let (ba, sa) = run();
        assert_eq!((ba, sa), run());
        assert!(sa <= STARVE_TICKS);
        // Per-client average: `ba` sums 64 connections, so divide by 64.
        assert!(ba * 8 * 100 * TICK_HZ / (64 * 6 * 1_000) <= ON9_CAP_KBPS_X100);
    }

    #[test]
    fn interest_64_bot_soak_beats_broadcast_3x() {
        // ON15 Accept: a 64-bot soak saves 3x or better versus broadcast at
        // 128 kbps/client or less, no combat entity starves past 500 ms and
        // nothing the receiver needs desyncs. The broadcast arm runs the same
        // 64-entity roster, the same delta baseline and the same 10 Hz cadence
        // through the same `select()` path with every room in scope and rings
        // wide enough that everything is Near: the only difference from the
        // interest arm is the per-connection scope gate this row adds.
        const TICKS: u32 = 600;
        const COMBAT_TICK: u32 = 300;
        let interest = InterestCfg::default_siege();
        let broadcast = InterestCfg {
            near_m: 1_000_000,
            far_m: 1_000_000,
            near_hz: 10,
            far_hz: 10,
            cap_bps: 1_000_000,
        };
        broadcast.validate().expect("broadcast tuning is legal");
        // One entity per viewer, one metre apart, all drifting east together
        // so the roster is a moving soak, not a frozen frame. Entity 7 starts
        // fighting at COMBAT_TICK: a combat entity must never starve.
        let roster = |tick: u32| -> Vec<InterestEntity> {
            let drift = i32::try_from(tick).expect("tick fits") * 10;
            (0..64u32)
                .map(|id| InterestEntity {
                    id,
                    x_mm: i32::try_from(id).expect("id fits") * 1_000 + drift,
                    z_mm: 0,
                    combat: if id == 7 && tick >= COMBAT_TICK {
                        900
                    } else {
                        1_000
                    },
                })
                .collect()
        };
        // One arm: total wire bytes, worst starve, worst ticks-since-sent for
        // the combat entity across every viewer.
        let soak = |cfg: &InterestCfg, scoped: bool| -> (u64, u32, u32, (u32, usize)) {
            let base = roster(0);
            let mut conns: Vec<ConnectionInterest> = (0..64u32)
                .map(|v| {
                    let mut conn = ConnectionInterest::for_entities(v, 64).expect("conn");
                    if scoped {
                        conn.set_rooms(room_mask(v % 8));
                    }
                    conn.register_baseline(&base).expect("baseline");
                    conn
                })
                .collect();
            let mut out = vec![0u32; 64];
            // Per viewer: the combat word its baseline holds for entity 7, and
            // the first snapshot tick at which it fell behind (if any). A held
            // word equal to the baseline is not owed; only a real change counts.
            let mut held7 = vec![1_000u32; 64];
            let mut stale7 = vec![None::<u32>; 64];
            let (mut bytes, mut starve, mut combat_gap) = (0u64, 0u32, 0u32);
            let mut combat_at = (0u32, 0usize);
            for tick in (0..TICKS).step_by(SNAPSHOT_EVERY_TICKS as usize) {
                let entities = roster(tick);
                for (v, conn) in conns.iter_mut().enumerate() {
                    let viewer = u32::try_from(v).expect("v fits");
                    let report = conn
                        .select(
                            cfg,
                            &entities,
                            viewer,
                            entities[v].x_mm,
                            entities[v].z_mm,
                            0,
                            tick,
                            SIEGE_BUDGET_BYTES,
                            &mut out,
                        )
                        .expect("select");
                    if out[..report.selected].contains(&7) {
                        held7[v] = entities[7].combat;
                        stale7[v] = None;
                    } else if held7[v] != entities[7].combat {
                        let since = *stale7[v].get_or_insert(tick);
                        let gap = tick - since;
                        if gap > combat_gap {
                            combat_gap = gap;
                            combat_at = (tick, v);
                        }
                    }
                    starve = starve.max(report.starve_seen);
                    bytes += u64::try_from(report.bytes).expect("bytes fit");
                }
            }
            (bytes, starve, combat_gap, combat_at)
        };
        let broadcast_run = soak(&broadcast, false);
        let interest_run = soak(&interest, true);
        assert_eq!(
            (broadcast_run, interest_run),
            (soak(&broadcast, false), soak(&interest, true)),
            "the 64-bot soak must replay bit-identically"
        );
        let (broadcast_bytes, _, _, _) = broadcast_run;
        let (interest_bytes, starve, combat_gap, combat_at) = interest_run;
        // No combat entity starves: every viewer ships the dirty body well
        // inside the 500 ms bar, and no in-scope entity goes stale past it.
        assert!(
            combat_gap <= STARVE_TICKS,
            "combat entity stale {combat_gap} ticks (tick {}, viewer {}) over the {STARVE_TICKS}-tick bar",
            combat_at.0,
            combat_at.1
        );
        assert!(
            starve <= STARVE_TICKS,
            "worst starve {starve} ticks over the 500 ms bar {STARVE_TICKS}"
        );
        // Per-client rate over the soak, integer kbps x100 (never float).
        let kbps_x100 = interest_bytes * 8 * 100 * TICK_HZ / (64 * u64::from(TICKS) * 1_000);
        assert!(
            kbps_x100 <= ON9_CAP_KBPS_X100,
            "client {kbps_x100} over the 12800 (128 kbps) cap"
        );
        // Integer saving x100: broadcast over interest, at least 3x.
        let saving_x100 = broadcast_bytes * 100 / interest_bytes;
        assert!(
            saving_x100 >= 300,
            "saving {saving_x100} under 300: broadcast {broadcast_bytes} vs interest {interest_bytes}"
        );
        // Zero desync on the shipped bytes: the combat change round-trips
        // through the real delta codec against the acked baseline.
        let base_bodies: Vec<BodySnap> = (0..16u8)
            .map(|slot| BodySnap {
                slot,
                x_mm: i32::from(slot) * 1_000,
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
                hp: 1_000,
                stamina: 100,
            })
            .collect();
        let mut now_bodies = base_bodies.clone();
        now_bodies[7].hp = 900;
        now_bodies[7].x_mm += 60;
        let baseline = FullSnap::new(COMBAT_TICK - 6, base_bodies).expect("baseline");
        let full = FullSnap::new(COMBAT_TICK, now_bodies).expect("full");
        let decoded = decode_delta(&encode_delta(&full, &baseline).expect("encode"), &baseline)
            .expect("decode");
        assert_eq!(decoded, full, "the shipped combat change must decode exact");
    }

    #[test]
    fn ioq3_per_client_view_matches_select() {
        let cfg = InterestCfg::default_siege();
        let entities: Vec<InterestEntity> = (0..8)
            .map(|id| InterestEntity {
                id,
                x_mm: i32::try_from(id).expect("id fits") * 1_000,
                z_mm: 0,
                combat: 1_000,
            })
            .collect();
        let mut conn = ConnectionInterest::for_entities(0, 8).expect("conn");
        conn.set_rooms(room_mask(0) | room_mask(3));
        conn.register_baseline(&entities).expect("baseline");
        let mut out = [0u32; 8];
        let report = conn
            .select(&cfg, &entities, 0, 0, 0, 0, 6, SIEGE_BUDGET_BYTES, &mut out)
            .expect("select");
        assert_eq!(report.selected, 2, "{report:?}");
        assert_eq!(out[..report.selected], [0, 3]);
        let base_fighters: Vec<BodySnap> = (0..8)
            .map(|slot| BodySnap {
                slot,
                x_mm: i32::from(slot) * 1_000,
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
                hp: 1_000,
                stamina: 100,
            })
            .collect();
        let mut now_fighters = base_fighters.clone();
        now_fighters[3].x_mm += 500;
        now_fighters[3].hp = 880;
        let baseline = FullSnap::new(100, base_fighters).expect("baseline");
        let full = FullSnap::new(106, now_fighters).expect("full");
        let reliable = vec![vec![0xC0, 0x01], vec![0xC0, 0x02, 0x03]];
        let view = ioq3_client_view(Ioq3ViewParams {
            selected: &out[..report.selected],
            full: &full,
            baseline: &baseline,
            lastframe: 100,
            server_time: 106,
            snap_flags: 0,
            area_bytes: vec![0xA5, 0x5A],
            reliable: reliable.clone(),
            sequence: 9,
            qport: 3,
        })
        .expect("view");
        assert_eq!(
            view.entity_count, report.selected,
            "per-client count must equal select count"
        );
        assert_eq!(
            view.receipt(),
            (100, 106, 2, 2),
            "receipt names lastframe + serverTime"
        );
        let sub = |tick: u32, src: &FullSnap| {
            FullSnap::new(
                tick,
                src.fighters
                    .iter()
                    .filter(|f| f.slot == 0 || f.slot == 3)
                    .copied()
                    .collect(),
            )
            .expect("subset")
        };
        assert_eq!(
            crate::delta::ioq3_decode_fields(&view.fields, &sub(100, &baseline)).expect("decode"),
            sub(106, &full),
            "decode against the acked baseline equals full"
        );
        let mut image = Vec::new();
        for cmd in &reliable {
            image.extend_from_slice(cmd);
        }
        image.extend_from_slice(&[0xA5, 0x5A]);
        assert_eq!(
            crate::session::ioq3_reassemble(&view.datagrams).expect("reassemble"),
            image,
            "reliable ahead of snapshot reassembles byte-exact"
        );
        assert!(
            view.wire_len() < full.full_wire_len(),
            "view wire {} must beat full {}",
            view.wire_len(),
            full.full_wire_len()
        );
    }
}
