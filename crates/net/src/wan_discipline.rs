//! wan_discipline: datagram discipline for the WAN replication stream.
//!
//! Five send classes share one datagram: the viewer's own input echo, combat
//! words, near poses, far poses and the idle crowd. A class batch never
//! fragments past one datagram; a batch bigger than the cap is refused as
//! [`WanDisciplineError::OverBudget`](crate::wan_discipline::WanDisciplineError::OverBudget) and counted, never split. Under
//! congestion the idle crowd sheds first, then far poses, then near poses;
//! each waits 2-3 ticks (its own cadence) before it expires, while inputs and
//! combat repeat [`REDUNDANCY`](crate::session::REDUNDANCY) times and never expire, so a single lost
//! datagram cannot starve them (RFC 9221 datagram rules). Owner: net lane.
//! Units: bytes, ticks at 60 Hz, integer kilobits/s x100. Determinism:
//! integer ops and stable id order only, no map order, no wall clock; the
//! report carries integer kilobits/s x100 and a tuple digest. Errors:
//! [`WanDisciplineError`](crate::wan_discipline::WanDisciplineError), never panics. Crate edge: reuses `interest.rs`
//! relevancy and `scale.rs` constants, never mirrors them.

use super::interest::{
    dist2_mm, entity_cost_bytes, in_front, octant_dir, relevancy, InterestCfg, Relevancy,
    ON9_CAP_KBPS_X100, SNAPSHOT_EVERY_TICKS, STARVE_TICKS,
};
use super::scale::{MAX_DATAGRAM_BYTES, TICK_HZ};
use super::tick_budget::{ENTITIES, PLAYERS};
use super::wan_soak::{JITTER_TICKS, LOSS_PCT, RTT_TICKS};
use engine_core::mix64;
use std::fmt::{self, Display, Formatter};

/// Wire bytes of one input packet: slot, tick and the wish words.
pub const INPUT_BYTES: usize = 24;
/// How many datagrams owe a copy of an input or combat batch.
pub const REDUNDANCY: u8 = 2;
/// Ticks a far pose batch waits for a datagram before it expires.
pub const POSE_FAR_EXPIRY_TICKS: u32 = 2;
/// Ticks an idle crowd batch waits for a datagram before it expires.
pub const IDLE_EXPIRY_TICKS: u32 = 3;
/// Ticks a near pose batch waits: one snapshot cadence.
pub const POSE_NEAR_EXPIRY_TICKS: u32 = SNAPSHOT_EVERY_TICKS;
/// Longest tick count one discipline run accepts (one minute at 60 Hz).
pub const DISCIPLINE_MAX_TICKS: u32 = 3_600;

/// Send classes in keep order: a later class is shed first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Class {
    /// The viewer's own input echo: never expired, sent redundantly.
    Input,
    /// Combat words: hp and stamina changes, never expired, redundant.
    Combat,
    /// Poses inside the near ring.
    PoseNear,
    /// Poses inside the far ring, offered at the far LOD cadence.
    PoseFar,
    /// The idle crowd: an unchanged pose beyond the far ring.
    Idle,
}

impl Class {
    /// Every class in keep order.
    pub const ALL: [Self; 5] = [
        Self::Input,
        Self::Combat,
        Self::PoseNear,
        Self::PoseFar,
        Self::Idle,
    ];

    /// Slot for the per-class working arrays.
    const fn slot(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Combat => 1,
            Self::PoseNear => 2,
            Self::PoseFar => 3,
            Self::Idle => 4,
        }
    }

    /// Short label for a posted report line.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Combat => "combat",
            Self::PoseNear => "pose_near",
            Self::PoseFar => "pose_far",
            Self::Idle => "idle",
        }
    }

    /// Datagrams that owe a copy of a batch: inputs and combat repeat, poses
    /// ship once.
    #[must_use]
    pub const fn redundancy(self) -> u8 {
        match self {
            Self::Input | Self::Combat => REDUNDANCY,
            Self::PoseNear | Self::PoseFar | Self::Idle => 1,
        }
    }

    /// Ticks an unsent batch waits before it expires; `None` never expires.
    #[must_use]
    pub const fn expiry_ticks(self) -> Option<u32> {
        match self {
            Self::Input | Self::Combat => None,
            Self::PoseNear => Some(POSE_NEAR_EXPIRY_TICKS),
            Self::PoseFar => Some(POSE_FAR_EXPIRY_TICKS),
            Self::Idle => Some(IDLE_EXPIRY_TICKS),
        }
    }

    /// True when a batch of this class is never dropped by age.
    #[must_use]
    pub const fn never_expires(self) -> bool {
        matches!(self, Self::Input | Self::Combat)
    }
}

impl Display for Class {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Discipline failure: the datagram rule or a bar refused, with both sides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WanDisciplineError {
    /// A class batch is bigger than one datagram: dropped and counted.
    OverBudget {
        /// Bytes the batch needs.
        need: usize,
        /// Bytes one datagram carries.
        budget: usize,
    },
    /// Tick length outside `1..=DISCIPLINE_MAX_TICKS`.
    BadTicks {
        /// Ticks the caller offered.
        got: u32,
    },
    /// A combat entity went longer than the starve bar unsent.
    Starved {
        /// Ticks the slowest combat entity waited.
        got: u32,
        /// Ticks the bar allows.
        bar: u32,
    },
    /// A client carried more than the per-client bandwidth bar.
    OverCap {
        /// Measured kilobits/s x100.
        got_x100: u64,
        /// Bar in kilobits/s x100.
        cap_x100: u64,
    },
}

impl Display for WanDisciplineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OverBudget { need, budget } => {
                write!(
                    f,
                    "class batch needs {need} bytes over one datagram {budget}"
                )
            }
            Self::BadTicks { got } => {
                write!(
                    f,
                    "discipline ticks {got} outside 1..={DISCIPLINE_MAX_TICKS}"
                )
            }
            Self::Starved { got, bar } => {
                write!(f, "combat starved {got} ticks over the {bar}-tick bar")
            }
            Self::OverCap { got_x100, cap_x100 } => write!(
                f,
                "client carried {got_x100} kbps x100 over the {cap_x100} bar"
            ),
        }
    }
}

impl std::error::Error for WanDisciplineError {}

/// One class's pending bytes: the fresh batch waiting for a datagram, the
/// written prefix that one more datagram still owes (redundancy), the tick the
/// fresh batch first became owed and the last tick it was offered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ClassState {
    owed: usize,
    repeat: usize,
    since: u32,
    last_tick: u32,
}

impl ClassState {
    /// Nothing owed, nothing counted.
    const EMPTY: Self = Self {
        owed: 0,
        repeat: 0,
        since: 0,
        last_tick: 0,
    };
}

/// What one [`DatagramQueue::flush`] packed, in bytes per class.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FlushReport {
    written: usize,
    by_class: [usize; 5],
}

impl FlushReport {
    /// Datagram size in bytes.
    #[must_use]
    pub const fn written(&self) -> usize {
        self.written
    }

    /// Bytes one class put into the datagram.
    #[must_use]
    pub const fn class_bytes(&self, class: Class) -> usize {
        self.by_class[class.slot()]
    }
}

/// Per-client send queue: one pending batch per class plus the shed ledger.
/// The queue is plain integer state, so the same offers draw the same
/// datagram on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DatagramQueue {
    classes: [ClassState; 5],
    over_budget_drops: u64,
    superseded_bytes: u64,
    shed_bytes: u64,
    expired_batches: u64,
}

impl DatagramQueue {
    /// An empty queue: nothing owed, nothing counted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            classes: [ClassState::EMPTY; 5],
            over_budget_drops: 0,
            superseded_bytes: 0,
            shed_bytes: 0,
            expired_batches: 0,
        }
    }

    /// Offer `bytes` of `class` at `tick`. A batch bigger than one datagram is
    /// refused as [`WanDisciplineError::OverBudget`] and counted, never split;
    /// the first offer of a new tick supersedes the previous tick's unsent
    /// bytes of that class, so fresh state never queues behind stale state.
    pub fn offer(
        &mut self,
        class: Class,
        bytes: usize,
        tick: u32,
    ) -> Result<(), WanDisciplineError> {
        if bytes > MAX_DATAGRAM_BYTES {
            self.over_budget_drops = self.over_budget_drops.saturating_add(1);
            return Err(WanDisciplineError::OverBudget {
                need: bytes,
                budget: MAX_DATAGRAM_BYTES,
            });
        }
        let state = &mut self.classes[class.slot()];
        if state.last_tick != tick {
            self.superseded_bytes = self
                .superseded_bytes
                .saturating_add(u64::try_from(state.owed).unwrap_or(0));
            state.owed = 0;
            state.since = tick;
            state.last_tick = tick;
        }
        state.owed = state.owed.saturating_add(bytes);
        Ok(())
    }

    /// Pack one datagram of at most `budget_bytes` in class keep order: input
    /// echo, combat, near poses, far poses, idle crowd. Unsent batches older
    /// than their class expiry shed first; inputs and combat never expire.
    pub fn flush(&mut self, tick: u32, budget_bytes: usize) -> FlushReport {
        self.shed_expired(tick);
        let mut report = FlushReport::default();
        let mut room = budget_bytes;
        for class in Class::ALL {
            let used = self.pack(class, room, &mut report);
            room = room.saturating_sub(used);
        }
        report
    }

    /// Expire an unsent shed-able batch older than its class expiry.
    fn shed_expired(&mut self, tick: u32) {
        for class in [Class::PoseFar, Class::PoseNear, Class::Idle] {
            let state = &mut self.classes[class.slot()];
            if state.owed == 0 {
                continue;
            }
            let Some(expiry) = class.expiry_ticks() else {
                continue;
            };
            if tick.saturating_sub(state.since) >= expiry {
                self.shed_bytes = self
                    .shed_bytes
                    .saturating_add(u64::try_from(state.owed).unwrap_or(0));
                self.expired_batches = self.expired_batches.saturating_add(1);
                state.owed = 0;
            }
        }
    }

    /// Move up to `room` bytes of one class into the datagram: fresh bytes
    /// first, then the one redundant repeat. A redundant class owes the fresh
    /// prefix one more datagram, so a single loss cannot starve it.
    fn pack(&mut self, class: Class, room: usize, report: &mut FlushReport) -> usize {
        if room == 0 {
            return 0;
        }
        let state = &mut self.classes[class.slot()];
        let fresh = state.owed.min(room);
        state.owed -= fresh;
        let repeat = state.repeat.min(room - fresh);
        state.repeat -= repeat;
        if class.never_expires() && fresh > 0 {
            state.repeat = fresh;
        } else if !class.never_expires() {
            state.repeat = 0;
        }
        let used = fresh + repeat;
        if used > 0 {
            report.written = report.written.saturating_add(used);
            report.by_class[class.slot()] = report.by_class[class.slot()].saturating_add(used);
        }
        used
    }

    /// Bytes a class still owes a datagram.
    #[must_use]
    pub const fn pending_bytes(&self, class: Class) -> usize {
        let state = &self.classes[class.slot()];
        state.owed.saturating_add(state.repeat)
    }

    /// Batches refused for being bigger than one datagram.
    #[must_use]
    pub const fn over_budget_drops(&self) -> u64 {
        self.over_budget_drops
    }

    /// Bytes replaced by fresher state before a datagram could carry them.
    #[must_use]
    pub const fn superseded_bytes(&self) -> u64 {
        self.superseded_bytes
    }

    /// Bytes dropped by class expiry.
    #[must_use]
    pub const fn shed_bytes(&self) -> u64 {
        self.shed_bytes
    }

    /// Batches dropped by class expiry.
    #[must_use]
    pub const fn expired_batches(&self) -> u64 {
        self.expired_batches
    }
}

impl Default for DatagramQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// One siege entity as the discipline layer sees it: feet position plus the
/// combat word the caller folds hp and stamina into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Crowd {
    x_mm: i32,
    z_mm: i32,
    combat: u32,
}

/// Posted discipline evidence for one siege run. Every field is integer and
/// deterministic per `(seed, ticks)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DisciplineReport {
    /// Tape seed.
    pub seed: u64,
    /// Ticks stepped.
    pub ticks: u32,
    /// Connections replicated to (the siege's 64 players).
    pub viewers: u32,
    /// Datagrams packed across all viewers.
    pub datagrams: u64,
    /// Class bytes offered, refused ones included.
    pub submitted_bytes: u64,
    /// Bytes actually written into a datagram.
    pub written_bytes: u64,
    /// Batches refused for being bigger than one datagram.
    pub refused_batches: u64,
    /// Bytes replaced by fresher state before a datagram could carry them.
    pub superseded_bytes: u64,
    /// Bytes dropped by class expiry.
    pub shed_bytes: u64,
    /// Batches dropped by class expiry.
    pub expired_batches: u64,
    /// Input bytes written.
    pub input_bytes: u64,
    /// Combat bytes written.
    pub combat_bytes: u64,
    /// Largest datagram packed (never over the cap).
    pub max_datagram_bytes: usize,
    /// Datagrams the seeded fault schedule lost.
    pub lost_datagrams: u64,
    /// Longest wait any combat entity saw, in ticks.
    pub combat_starve_max: u32,
    /// Worst per-client rate across the run, in integer kilobits/s x100.
    pub kbps_x100_max: u64,
}

impl DisciplineReport {
    /// Check the datagram rule plus both bars. Fails loudly with both sides.
    pub fn check(&self) -> Result<(), WanDisciplineError> {
        if self.max_datagram_bytes > MAX_DATAGRAM_BYTES {
            return Err(WanDisciplineError::OverBudget {
                need: self.max_datagram_bytes,
                budget: MAX_DATAGRAM_BYTES,
            });
        }
        if self.combat_starve_max > STARVE_TICKS {
            return Err(WanDisciplineError::Starved {
                got: self.combat_starve_max,
                bar: STARVE_TICKS,
            });
        }
        if self.kbps_x100_max > ON9_CAP_KBPS_X100 {
            return Err(WanDisciplineError::OverCap {
                got_x100: self.kbps_x100_max,
                cap_x100: ON9_CAP_KBPS_X100,
            });
        }
        Ok(())
    }

    /// Deterministic projection for x2 asserts: every integer counter.
    #[must_use]
    pub const fn digest_key(self) -> (u64, u32, u32, u64, u64, u64, u64, u64, u64, u64, u64, u32) {
        (
            self.seed,
            self.ticks,
            self.viewers,
            self.datagrams,
            self.submitted_bytes,
            self.written_bytes,
            self.refused_batches,
            self.superseded_bytes,
            self.shed_bytes,
            self.expired_batches,
            self.lost_datagrams,
            self.combat_starve_max,
        )
    }

    /// Posted lines: the datagram ledger, the bars and the kbps number.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "wan_discipline seed={} ticks={} viewers={} datagrams={} submitted={} written={} refused={} superseded={} shed={} expired={}\n\
             wan_discipline input={} combat={} max_datagram={} (cap {}) lost={} starve_max={} (bar {}) kbps_x100_max={} (cap {})",
            self.seed,
            self.ticks,
            self.viewers,
            self.datagrams,
            self.submitted_bytes,
            self.written_bytes,
            self.refused_batches,
            self.superseded_bytes,
            self.shed_bytes,
            self.expired_batches,
            self.input_bytes,
            self.combat_bytes,
            self.max_datagram_bytes,
            MAX_DATAGRAM_BYTES,
            self.lost_datagrams,
            self.combat_starve_max,
            STARVE_TICKS,
            self.kbps_x100_max,
            ON9_CAP_KBPS_X100,
        )
    }
}

/// Small signed offset in `-half..=half` from a hash word, no float, no cast.
fn signed_half(word: u64, half: u32) -> i32 {
    let span = u64::from(half) * 2 + 1;
    let raw = u32::try_from(word % span).unwrap_or(0);
    i32::try_from(raw).unwrap_or(0) - i32::try_from(half).unwrap_or(0)
}

/// Player spawn box half-extent in millimetres (16 m).
const SIEGE_HALF_MM: u32 = 16_000;
/// Siege crowd half-extent in millimetres (60 m), dense enough that the near
/// pose batch of one viewer exceeds one datagram, so the shed rule runs.
const CROWD_HALF_MM: u32 = 60_000;

/// Deterministic spawn in millimetres: players sit in a 16 m box, the AI
/// crowd fills a 60 m field.
fn spawn_xz(id: usize, seed: u64) -> (i32, i32) {
    let half = if id < PLAYERS {
        SIEGE_HALF_MM
    } else {
        CROWD_HALF_MM
    };
    let word = mix64(u64::try_from(id).unwrap_or(0) ^ seed);
    (signed_half(word, half), signed_half(word >> 32, half))
}

/// Deterministic one-datagram drift in millimetres (at most a metre).
fn drift_xz(id: usize, tick: u32, seed: u64) -> (i32, i32) {
    let lane = u64::from(tick)
        .wrapping_mul(0x9E37_79B1_7F4A_7C15)
        .wrapping_add(
            u64::try_from(id)
                .unwrap_or(0)
                .wrapping_mul(0xC2B2_AE35_0D29_C127),
        )
        ^ seed;
    (
        signed_half(mix64(lane), 1_000),
        signed_half(mix64(lane ^ 0xA5A5_5A5A_5A5A_5A5A), 1_000),
    )
}

/// Combat word of one siege entity: a player slot trades blows often, the AI
/// crowd rarely, so the combat class stays small and bursty.
fn siege_combat(id: usize, tick: u32) -> u32 {
    if id < PLAYERS {
        1_000u32.saturating_sub((tick.wrapping_add(u32::try_from(id).unwrap_or(0) * 37)) % 53)
    } else {
        1_000u32.saturating_sub((tick / 97) % 4)
    }
}

/// Ring edge squared in millimetres at 1000 mm per metre.
fn ring2(metres: u32) -> i64 {
    let edge = i64::from(metres) * 1_000;
    edge.saturating_mul(edge)
}

/// Seeded fault draw for one `(tick, viewer)` lane: loss first, then reorder.
fn fault_draw(seed: u64, tick: u32, viewer: usize) -> u64 {
    let lane = seed
        .wrapping_add(u64::from(tick).wrapping_mul(0x9E37_79B1_7F4A_7C15))
        .wrapping_add(
            u64::try_from(viewer)
                .unwrap_or(0)
                .wrapping_mul(0xD1B5_4A32_D192_ED03),
        );
    mix64(lane)
}

/// One snapshot of the siege: position, combat word and class of every entity.
fn refresh_crowd(roster: &mut [Crowd], tick: u32, seed: u64) {
    for (id, entity) in roster.iter_mut().enumerate() {
        let (sx, sz) = spawn_xz(id, seed);
        let (dx, dz) = drift_xz(id, tick, seed);
        entity.x_mm = sx.saturating_add(dx);
        entity.z_mm = sz.saturating_add(dz);
        entity.combat = siege_combat(id, tick);
    }
}

/// Per-viewer deliveries the seeded network still owes, and the combat ids
/// each carries.
type Inflight = Vec<(u32, Vec<u16>)>;

/// Ring geometry plus the cadence flags of one snapshot.
#[derive(Clone, Copy)]
struct Rings {
    near2: i64,
    far2: i64,
    far_due: bool,
    idle_due: bool,
}

/// Whole-run counters the snapshot loop folds.
#[derive(Clone, Copy, Default)]
struct Totals {
    datagrams: u64,
    submitted_bytes: u64,
    written_bytes: u64,
    refused_batches: u64,
    lost_datagrams: u64,
    input_bytes: u64,
    combat_bytes: u64,
    max_datagram_bytes: usize,
    combat_starve_max: u32,
}

/// Siege working set: the crowd, one send queue and delivery ledger per
/// viewer, and the scratch buffers a snapshot reuses.
struct Siege {
    roster: Vec<Crowd>,
    prev: Vec<Crowd>,
    queues: Vec<DatagramQueue>,
    written_per_viewer: Vec<u64>,
    /// Combat entities a viewer still owes a delivered copy, with the tick the
    /// copy was last sent: the starve bar measures that wait.
    owed: Vec<Vec<(u16, u32)>>,
    inflight: Vec<Inflight>,
    combat_ids: Vec<u16>,
    payload: Vec<u16>,
}

impl Siege {
    /// Spawn the crowd and size one queue and ledger per viewer.
    fn new(seed: u64) -> Self {
        let roster: Vec<Crowd> = (0..ENTITIES)
            .map(|id| {
                let (x, z) = spawn_xz(id, seed);
                Crowd {
                    x_mm: x,
                    z_mm: z,
                    combat: siege_combat(id, 0),
                }
            })
            .collect();
        let prev = roster.clone();
        Self {
            roster,
            prev,
            queues: vec![DatagramQueue::new(); PLAYERS],
            written_per_viewer: vec![0; PLAYERS],
            owed: (0..PLAYERS).map(|_| Vec::new()).collect(),
            inflight: (0..PLAYERS).map(|_| Vec::new()).collect(),
            combat_ids: Vec::with_capacity(ENTITIES),
            payload: Vec::with_capacity(ENTITIES),
        }
    }

    /// Move the crowd to `tick` and replicate every viewer once.
    fn snapshot(&mut self, seed: u64, tick: u32, rings: Rings, totals: &mut Totals) {
        refresh_crowd(&mut self.roster, tick, seed);
        let dir = octant_dir(0);
        for viewer in 0..PLAYERS {
            self.replicate(viewer, seed, tick, rings, dir, totals);
        }
        self.prev.copy_from_slice(&self.roster);
    }

    /// One viewer's snapshot: drain arrivals, pack the datagram, fold the
    /// counters and owe every combat id the datagram carried.
    fn replicate(
        &mut self,
        viewer: usize,
        seed: u64,
        tick: u32,
        rings: Rings,
        dir: (i32, i32),
        totals: &mut Totals,
    ) {
        let Self {
            roster,
            prev,
            queues,
            written_per_viewer,
            owed,
            inflight,
            combat_ids,
            payload,
        } = self;
        collect_deliveries(
            &mut inflight[viewer],
            &mut owed[viewer],
            tick,
            &mut totals.combat_starve_max,
        );
        let (written, offered, refused, input, combat) = replicate_viewer(
            &mut queues[viewer],
            ReplicateArgs {
                roster,
                prev,
                viewer,
                tick,
                near2: rings.near2,
                far2: rings.far2,
                far_due: rings.far_due,
                idle_due: rings.idle_due,
                dir,
            },
            combat_ids,
        );
        let written64 = u64::try_from(written).unwrap_or(0);
        totals.datagrams = totals.datagrams.saturating_add(1);
        totals.submitted_bytes = totals.submitted_bytes.saturating_add(offered);
        totals.written_bytes = totals.written_bytes.saturating_add(written64);
        totals.refused_batches = totals.refused_batches.saturating_add(refused);
        totals.input_bytes = totals.input_bytes.saturating_add(input);
        totals.combat_bytes = totals.combat_bytes.saturating_add(combat);
        written_per_viewer[viewer] = written_per_viewer[viewer].saturating_add(written64);
        if written > totals.max_datagram_bytes {
            totals.max_datagram_bytes = written;
        }
        // A datagram carrying combat bytes carries this snapshot's combat ids
        // plus every id still owed a copy: an unacked combat word repeats
        // until a delivery lands, so one lost datagram costs one cadence.
        payload.clear();
        if combat > 0 {
            payload.extend_from_slice(combat_ids);
            for (id, _) in owed[viewer].iter() {
                if !payload.contains(id) {
                    payload.push(*id);
                }
            }
            for id in payload.iter() {
                match owed[viewer].iter_mut().find(|(owed_id, _)| owed_id == id) {
                    Some(entry) => entry.1 = tick,
                    None => owed[viewer].push((*id, tick)),
                }
            }
        }
        mark_inflight(
            &mut inflight[viewer],
            seed,
            tick,
            viewer,
            written,
            payload,
            &mut totals.lost_datagrams,
        );
    }

    /// Worst wait any combat entity still owed a copy saw at the end.
    fn worst_wait(&self, ticks: u32) -> u32 {
        let mut worst = 0u32;
        for viewer in 0..PLAYERS {
            worst = worst.max(settle(&self.owed[viewer], ticks));
        }
        worst
    }

    /// Worst per-client rate over the run, in integer kilobits/s x100.
    fn rate_max(&self, ticks: u32) -> u64 {
        let mut worst = 0u64;
        for written in self.written_per_viewer.iter() {
            let rate = u128::from(*written)
                .saturating_mul(u128::from(TICK_HZ))
                .saturating_mul(800)
                / (u128::from(ticks) * 1_000);
            worst = worst.max(u64::try_from(rate).unwrap_or(u64::MAX));
        }
        worst
    }
}

/// Run the discipline proof over the siege population: 64 viewers, 64
/// fighters and 500 AI soldiers, under the seeded 150 ms / 30 ms / 2% fault
/// schedule. Each viewer packs at most one datagram per snapshot; poses shed,
/// inputs and combat do not. Deterministic for a given `(seed, ticks)`.
pub fn run_discipline(seed: u64, ticks: u32) -> Result<DisciplineReport, WanDisciplineError> {
    if ticks == 0 || ticks > DISCIPLINE_MAX_TICKS {
        return Err(WanDisciplineError::BadTicks { got: ticks });
    }
    let cfg = InterestCfg::default_siege();
    let near2 = ring2(cfg.near_m);
    let far2 = ring2(cfg.far_m);
    let every_far = cfg.every_far();
    let every_idle = every_far.saturating_mul(4);
    let cadence = usize::try_from(SNAPSHOT_EVERY_TICKS).unwrap_or(1);

    let mut siege = Siege::new(seed);
    let mut totals = Totals::default();
    for tick in (0..ticks).step_by(cadence) {
        let rings = Rings {
            near2,
            far2,
            far_due: tick % every_far == 0,
            idle_due: tick % every_idle == 0,
        };
        siege.snapshot(seed, tick, rings, &mut totals);
    }
    let report = DisciplineReport {
        seed,
        ticks,
        viewers: u32::try_from(PLAYERS).unwrap_or(u32::MAX),
        datagrams: totals.datagrams,
        submitted_bytes: totals.submitted_bytes,
        written_bytes: totals.written_bytes,
        refused_batches: totals.refused_batches,
        superseded_bytes: siege
            .queues
            .iter()
            .map(DatagramQueue::superseded_bytes)
            .sum(),
        shed_bytes: siege.queues.iter().map(DatagramQueue::shed_bytes).sum(),
        expired_batches: siege
            .queues
            .iter()
            .map(DatagramQueue::expired_batches)
            .sum(),
        input_bytes: totals.input_bytes,
        combat_bytes: totals.combat_bytes,
        max_datagram_bytes: totals.max_datagram_bytes,
        lost_datagrams: totals.lost_datagrams,
        combat_starve_max: totals.combat_starve_max.max(siege.worst_wait(ticks)),
        kbps_x100_max: siege.rate_max(ticks),
    };
    report.check()?;
    Ok(report)
}

/// Arguments one viewer's replication pass reads.
struct ReplicateArgs<'a> {
    roster: &'a [Crowd],
    prev: &'a [Crowd],
    viewer: usize,
    tick: u32,
    near2: i64,
    far2: i64,
    far_due: bool,
    idle_due: bool,
    dir: (i32, i32),
}

/// Offer every relevant entity of one viewer into its queue, then pack one
/// datagram. `combat_ids` is cleared and filled with this snapshot's combat
/// entities. Returns written, offered, refused, input bytes and combat bytes.
fn replicate_viewer(
    queue: &mut DatagramQueue,
    args: ReplicateArgs<'_>,
    combat_ids: &mut Vec<u16>,
) -> (usize, u64, u64, u64, u64) {
    combat_ids.clear();
    // Input echo always ships, whatever the congestion.
    let _ = queue.offer(Class::Input, INPUT_BYTES, args.tick);
    let mut offered = INPUT_BYTES;
    let viewer_pos = args.roster[args.viewer];
    let mut refused = 0u64;
    for id in 0..args.roster.len() {
        if id == args.viewer {
            continue;
        }
        let entity = args.roster[id];
        let before = args.prev[id];
        let combat_dirty = entity.combat != before.combat;
        let pose_dirty = entity.x_mm != before.x_mm || entity.z_mm != before.z_mm;
        let cost = entity_cost_bytes(false, pose_dirty, combat_dirty);
        let class = if combat_dirty {
            combat_ids.push(u16::try_from(id).unwrap_or(u16::MAX));
            Class::Combat
        } else {
            let d2 = dist2_mm(entity.x_mm, entity.z_mm, viewer_pos.x_mm, viewer_pos.z_mm);
            let front = in_front(
                i64::from(entity.x_mm) - i64::from(viewer_pos.x_mm),
                i64::from(entity.z_mm) - i64::from(viewer_pos.z_mm),
                args.dir.0,
                args.dir.1,
            );
            match relevancy(d2, args.near2, args.far2, front, false) {
                Relevancy::Near | Relevancy::Combat => Class::PoseNear,
                Relevancy::Far => {
                    if !args.far_due {
                        continue;
                    }
                    Class::PoseFar
                }
                Relevancy::Culled => {
                    if !args.idle_due {
                        continue;
                    }
                    Class::Idle
                }
            }
        };
        offered = offered.saturating_add(cost);
        if queue.offer(class, cost, args.tick).is_err() {
            refused = refused.saturating_add(1);
        }
    }
    let flushed = queue.flush(args.tick, MAX_DATAGRAM_BYTES);
    (
        flushed.written(),
        u64::try_from(offered).unwrap_or(0),
        refused,
        u64::try_from(flushed.class_bytes(Class::Input)).unwrap_or(0),
        u64::try_from(flushed.class_bytes(Class::Combat)).unwrap_or(0),
    )
}

/// Drain every arrival due at `tick`: each carried combat id stops owing and
/// its wait folds into the bar.
fn collect_deliveries(
    inflight: &mut Inflight,
    owed: &mut Vec<(u16, u32)>,
    tick: u32,
    starve_max: &mut u32,
) {
    let mut at = 0usize;
    while at < inflight.len() {
        if inflight[at].0 > tick {
            at += 1;
            continue;
        }
        let (_, ids) = inflight.swap_remove(at);
        for id in ids {
            if let Some(pos) = owed.iter().position(|(owed_id, _)| *owed_id == id) {
                let (_, since) = owed.swap_remove(pos);
                *starve_max = (*starve_max).max(tick.saturating_sub(since));
            }
        }
    }
}

/// Wait of every combat entity still owed at the end of the run.
fn settle(owed: &[(u16, u32)], ticks: u32) -> u32 {
    let mut worst = 0u32;
    let last = ticks.saturating_sub(1);
    for (_, since) in owed {
        worst = worst.max(last.saturating_sub(*since));
    }
    worst
}

/// Schedule this datagram's arrival: `arrival` is the send tick plus the
/// injected round trip and jitter, with a reorder pushing it two ticks out or
/// a loss dropping it whole. Only combat ids need the schedule.
fn mark_inflight(
    inflight: &mut Inflight,
    seed: u64,
    tick: u32,
    viewer: usize,
    written: usize,
    combat_ids: &[u16],
    lost: &mut u64,
) {
    if written == 0 {
        return;
    }
    let draw = fault_draw(seed, tick, viewer);
    if draw % 100 < LOSS_PCT {
        *lost = lost.saturating_add(1);
        return;
    }
    let jitter = u32::try_from((draw >> 8) % (u64::from(JITTER_TICKS) * 2 + 1)).unwrap_or(0);
    let reorder = (draw >> 16) % 100 < 5;
    let mut arrival = tick.saturating_add(RTT_TICKS).saturating_add(jitter);
    if reorder {
        arrival = arrival.saturating_add(2);
    }
    inflight.push((arrival, combat_ids.to_vec()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tick_budget::SIEGE_SEED;

    const ACCEPT_TICKS: u32 = 240;

    #[test]
    fn discipline_refuses_an_oversize_batch_and_counts_it() {
        let mut queue = DatagramQueue::new();
        assert_eq!(
            queue.offer(Class::Combat, MAX_DATAGRAM_BYTES + 1, 0),
            Err(WanDisciplineError::OverBudget {
                need: MAX_DATAGRAM_BYTES + 1,
                budget: MAX_DATAGRAM_BYTES,
            })
        );
        assert_eq!(queue.over_budget_drops(), 1, "the drop is counted");
        assert_eq!(
            queue.pending_bytes(Class::Combat),
            0,
            "refused bytes never wait for a datagram"
        );
        assert_eq!(queue.offer(Class::Combat, MAX_DATAGRAM_BYTES, 0), Ok(()));
        let flushed = queue.flush(0, MAX_DATAGRAM_BYTES);
        assert_eq!(flushed.written(), MAX_DATAGRAM_BYTES, "the cap is exact");
    }

    #[test]
    fn discipline_sheds_far_then_idle_before_combat() {
        let mut queue = DatagramQueue::new();
        // Combat fills the whole datagram, so both pose batches wait and age.
        queue
            .offer(Class::Combat, MAX_DATAGRAM_BYTES, 0)
            .expect("combat is a legal batch");
        queue
            .offer(Class::PoseFar, 600, 0)
            .expect("far pose is a legal batch");
        queue
            .offer(Class::Idle, 600, 0)
            .expect("idle batch is a legal batch");
        let first = queue.flush(0, MAX_DATAGRAM_BYTES);
        assert_eq!(first.class_bytes(Class::Combat), MAX_DATAGRAM_BYTES);
        assert_eq!(
            first.class_bytes(Class::PoseFar),
            0,
            "a pose waits behind it"
        );
        assert_eq!(first.class_bytes(Class::Idle), 0);
        // The redundant repeat still owns the next datagram: one tick in.
        let second = queue.flush(1, MAX_DATAGRAM_BYTES);
        assert_eq!(second.class_bytes(Class::Combat), MAX_DATAGRAM_BYTES);
        assert_eq!(
            queue.expired_batches(),
            0,
            "one tick is inside every expiry"
        );
        assert_eq!(queue.pending_bytes(Class::PoseFar), 600);
        // Combat owns the datagram again: the poses keep aging.
        queue
            .offer(Class::Combat, MAX_DATAGRAM_BYTES, POSE_FAR_EXPIRY_TICKS)
            .expect("combat is a legal batch");
        queue.flush(POSE_FAR_EXPIRY_TICKS, MAX_DATAGRAM_BYTES);
        assert_eq!(
            queue.pending_bytes(Class::PoseFar),
            0,
            "a far pose expires at its 2-tick cadence"
        );
        assert_eq!(queue.expired_batches(), 1, "the shed batch is counted");
        assert_eq!(
            queue.pending_bytes(Class::Idle),
            600,
            "the idle crowd still holds at 2 ticks"
        );
        queue.flush(IDLE_EXPIRY_TICKS, MAX_DATAGRAM_BYTES);
        assert_eq!(queue.pending_bytes(Class::Idle), 0, "idle expires at 3");
        assert_eq!(
            queue.pending_bytes(Class::Combat),
            0,
            "combat drained, never expired"
        );
        assert_eq!(queue.expired_batches(), 2, "two shed batches are counted");
        assert_eq!(queue.shed_bytes(), 1_200, "both shed byte totals");
        assert!(Class::Combat.never_expires() && Class::Input.never_expires());
    }

    #[test]
    fn discipline_inputs_and_combat_repeat_once() {
        let mut queue = DatagramQueue::new();
        queue
            .offer(Class::Input, 100, 0)
            .expect("input is a legal batch");
        queue
            .offer(Class::Combat, 200, 0)
            .expect("combat is a legal batch");
        queue
            .offer(Class::PoseNear, 300, 0)
            .expect("near pose is a legal batch");
        let first = queue.flush(0, MAX_DATAGRAM_BYTES);
        assert_eq!(first.class_bytes(Class::Input), 100);
        assert_eq!(first.class_bytes(Class::Combat), 200);
        assert_eq!(first.class_bytes(Class::PoseNear), 300);
        // The redundant classes owe one more datagram; the pose does not.
        let second = queue.flush(1, MAX_DATAGRAM_BYTES);
        assert_eq!(second.class_bytes(Class::Input), 100, "input repeats once");
        assert_eq!(
            second.class_bytes(Class::Combat),
            200,
            "combat repeats once"
        );
        assert_eq!(
            second.class_bytes(Class::PoseNear),
            0,
            "a near pose ships once"
        );
        let third = queue.flush(2, MAX_DATAGRAM_BYTES);
        assert_eq!(third.written(), 0, "two datagrams settle every class");
        assert_eq!(queue.pending_bytes(Class::Combat), 0);
        assert_eq!(queue.pending_bytes(Class::PoseNear), 0);
    }

    #[test]
    fn discipline_replaces_stale_state_before_a_new_batch() {
        let mut queue = DatagramQueue::new();
        queue
            .offer(Class::PoseFar, 900, 0)
            .expect("far pose is a legal batch");
        // The 500-byte budget carries a 500-byte prefix; 400 bytes wait, then
        // fresher state arrives at the next cadence.
        queue.flush(0, 500);
        assert_eq!(queue.pending_bytes(Class::PoseFar), 400);
        queue
            .offer(Class::PoseFar, 100, SNAPSHOT_EVERY_TICKS)
            .expect("fresher far pose is a legal batch");
        assert_eq!(
            queue.superseded_bytes(),
            400,
            "stale bytes are counted, never queued"
        );
        assert_eq!(queue.pending_bytes(Class::PoseFar), 100);
        let flushed = queue.flush(SNAPSHOT_EVERY_TICKS, 500);
        assert_eq!(flushed.class_bytes(Class::PoseFar), 100);
    }

    #[test]
    fn discipline_converges_the_siege_wan_matrix_x2() {
        // Convergence half: the 64-client WAN fault matrix holds one
        // checkpoint under 150 ms, 30 ms jitter, 2% loss and reorder.
        let matrix = crate::wan_soak::run_matrix(SIEGE_SEED, ACCEPT_TICKS).expect("matrix");
        let again = crate::wan_soak::run_matrix(SIEGE_SEED, ACCEPT_TICKS).expect("matrix x2");
        for (first, second) in matrix.iter().zip(again.iter()) {
            first.check().expect("every fault holds its bars");
            assert_eq!(
                first.digest_key(),
                second.digest_key(),
                "same seed, same delivery ledger"
            );
            assert_eq!(
                first.hash, matrix[0].hash,
                "every fault holds the authoritative checkpoint"
            );
        }
        // Discipline half: the siege population under the same fault lane.
        let first = run_discipline(SIEGE_SEED, ACCEPT_TICKS).expect("discipline run");
        let second = run_discipline(SIEGE_SEED, ACCEPT_TICKS).expect("discipline run x2");
        assert_eq!(
            first.digest_key(),
            second.digest_key(),
            "same seed gives the same datagram ledger"
        );
        first.check().expect("discipline bars must hold");
        assert_eq!(first.viewers, u32::try_from(PLAYERS).expect("64 viewers"));
        assert_eq!(first.datagrams, u64::try_from(PLAYERS).expect("64") * 40);
        assert!(
            first.max_datagram_bytes <= MAX_DATAGRAM_BYTES,
            "a datagram never fragments past the cap, got {}",
            first.max_datagram_bytes
        );
        assert_eq!(
            first.refused_batches, 0,
            "no class batch needs to split into two datagrams"
        );
        assert!(
            first.expired_batches > 0,
            "congestion must shed far and idle poses"
        );
        assert!(
            first.superseded_bytes > 0,
            "a busy class replaces stale bytes"
        );
        assert!(first.input_bytes > 0 && first.combat_bytes > 0);
        assert!(
            first.combat_starve_max <= STARVE_TICKS,
            "no combat entity starves over 500 ms, got {} ticks",
            first.combat_starve_max
        );
        assert!(
            first.kbps_x100_max <= ON9_CAP_KBPS_X100,
            "a client carried {} kbps x100 over the 128 kbps bar",
            first.kbps_x100_max
        );
        println!("{}", first.summary());
    }

    #[test]
    fn discipline_report_bars_fail_loudly() {
        let good = run_discipline(SIEGE_SEED, 60).expect("a short run holds its bars");
        let mut bad = good;
        bad.combat_starve_max = STARVE_TICKS + 1;
        assert_eq!(
            bad.check(),
            Err(WanDisciplineError::Starved {
                got: STARVE_TICKS + 1,
                bar: STARVE_TICKS,
            })
        );
        bad.combat_starve_max = 0;
        bad.kbps_x100_max = ON9_CAP_KBPS_X100 + 1;
        assert_eq!(
            bad.check(),
            Err(WanDisciplineError::OverCap {
                got_x100: ON9_CAP_KBPS_X100 + 1,
                cap_x100: ON9_CAP_KBPS_X100,
            })
        );
        bad.kbps_x100_max = 0;
        bad.max_datagram_bytes = MAX_DATAGRAM_BYTES + 1;
        assert_eq!(
            bad.check(),
            Err(WanDisciplineError::OverBudget {
                need: MAX_DATAGRAM_BYTES + 1,
                budget: MAX_DATAGRAM_BYTES,
            })
        );
    }

    #[test]
    fn discipline_refuses_bad_shapes_loudly() {
        assert_eq!(
            run_discipline(SIEGE_SEED, 0),
            Err(WanDisciplineError::BadTicks { got: 0 })
        );
        assert_eq!(
            run_discipline(SIEGE_SEED, DISCIPLINE_MAX_TICKS + 1),
            Err(WanDisciplineError::BadTicks {
                got: DISCIPLINE_MAX_TICKS + 1
            })
        );
        assert!(format!("{}", WanDisciplineError::OverBudget { need: 5, budget: 1 }).contains('5'));
        assert!(format!("{}", Class::PoseFar).contains("pose_far"));
        assert_eq!(Class::ALL.len(), 5, "one class per send lane");
        for class in Class::ALL {
            assert!(class.redundancy() >= 1);
            assert_eq!(class.never_expires(), class.expiry_ticks().is_none());
        }
    }
}
