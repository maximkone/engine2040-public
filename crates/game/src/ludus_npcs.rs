//! ludus_npcs: living ludus voices — trainer, blacksmith and crowd.
//!
//! Consumer: the ludus-visit slice staffs its practice corner with these
//! voices; the slice wires the visit. Data owner:
//! [`LudusNpcs`] owns three [`Mind`]s plus one [`MemoryStore`] and a
//! recording [`MindRuntime`]. Units: ticks are fixed steps, salience
//! `0..=1000`, fighter slots `0..MAX_PARTICIPANTS`. Determinism: integer math
//! and role order only, FNV-1a checkpoints, simulated-ms budgets, no wall
//! clock. Errors: [`LudusError`]; bad fights store nothing. Budget: 3 NPCs
//! share one store (128 NPCs x 128 records); visits run off-tick, so spoken
//! lines may allocate; recall serves owned pulls for dialogue.
//!
//! Map binding: stations, zones and spawns below repeat the canonical
//! `content/maps/ludus` layout (`tools/map_data.rs`, `map.json`): the yard,
//! forge and board the voices staff. Values are content data, not another
//! crate's types, so no new dependency edge is needed (`engine-world` owns
//! cells/streaming, never ludus stations). The mind and memory types are used directly
//! through the existing `engine-game -> engine-ai` edge; nothing is mirrored.

use engine_ai::apply::{apply_at, Applied, AppliedBatch, PresentationEvent, Refused};
use engine_ai::memory::{MemoryStore, Recall};
use engine_ai::mind::{
    ActionKind, ActionSource, MemoryKind, Mind, MindAdapter, MindError, MindRequest, MindRuntime,
    NpcId, Privacy,
};
use engine_core::Fnv64;
use engine_input::MAX_PARTICIPANTS;

/// Ticks after issue a visit line may land (`[issued, issued + this]`).
const VISIT_DEADLINE_TICKS: u64 = 8;
/// Simulated-ms budget per ask: above the slowest local backend (20 ms), so
/// mock and local arms serve while timeout arms still fall back honestly.
const VISIT_BUDGET_MS: u64 = 500;
/// Ask ids start here; every ask takes the next id, so replays never collide.
const FIRST_REQUEST_ID: u64 = 1;

/// Canonical ludus map name from `content/maps/ludus/map.json`.
pub const LUDUS_MAP_NAME: &str = "ludus";
/// Canonical ludus layout version from `content/maps/ludus/tools/map_data.rs`.
pub const LUDUS_MAP_VERSION: u32 = 1;

/// Trainer mind handle: drills recruits on the practice yard.
pub const TRAINER_ID: u32 = 1;
/// Blacksmith mind handle: keeps blades sharp at the forge corner.
pub const BLACKSMITH_ID: u32 = 2;
/// Crowd mind handle: chants from the party-board corner.
pub const CROWD_ID: u32 = 3;

/// One ludus voice: trainer, blacksmith or crowd. Fixed order
/// (trainer, blacksmith, crowd) decides every visit loop, so one seed plus
/// one script always gives one transcript.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NpcRole {
    /// Drills recruits; stations at the practice yard.
    Trainer,
    /// Mends blades; stations at the blacksmith corner.
    Blacksmith,
    /// Chants for the victor; stations at the party board.
    Crowd,
}

impl NpcRole {
    /// Every voice in visit order.
    pub const ALL: [NpcRole; 3] = [NpcRole::Trainer, NpcRole::Blacksmith, NpcRole::Crowd];

    /// Stable mind handle for this voice.
    pub const fn id(self) -> NpcId {
        match self {
            NpcRole::Trainer => NpcId(TRAINER_ID),
            NpcRole::Blacksmith => NpcId(BLACKSMITH_ID),
            NpcRole::Crowd => NpcId(CROWD_ID),
        }
    }

    /// Short label for logs and receipts.
    pub const fn label(self) -> &'static str {
        match self {
            NpcRole::Trainer => "trainer",
            NpcRole::Blacksmith => "blacksmith",
            NpcRole::Crowd => "crowd",
        }
    }

    /// Display voice for presentation lines.
    pub const fn voice(self) -> &'static str {
        match self {
            NpcRole::Trainer => "Trainer",
            NpcRole::Blacksmith => "Blacksmith",
            NpcRole::Crowd => "Crowd",
        }
    }

    /// Map zone this voice staffs (a [`LUDUS_ZONES`] name).
    pub const fn station(self) -> &'static str {
        match self {
            NpcRole::Trainer => "practice_yard",
            NpcRole::Blacksmith => "blacksmith",
            NpcRole::Crowd => "party_board",
        }
    }

    /// Per-voice fallback seed: the session seed mixed with the voice id, so
    /// offline scripted tables pick different lines per voice.
    pub const fn seed(self, base: u64) -> u64 {
        base.wrapping_add((self.id().0 as u64).wrapping_mul(0x9E3779B97F4A7C15))
    }
}

/// One ludus zone rect in meters: `x0 <= x <= x1`, `z0 <= z <= z1`.
/// Values repeat `content/maps/ludus/tools/map_data.rs`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ZoneRect {
    /// Zone name (`practice_yard`, `blacksmith`, `party_board`).
    pub name: &'static str,
    /// West edge in meters.
    pub x0: f32,
    /// East edge in meters.
    pub x1: f32,
    /// North edge in meters.
    pub z0: f32,
    /// South edge in meters.
    pub z1: f32,
}

/// Canonical ludus zones: the training grounds hold the workshop and
/// board corners, which are disjoint from each other.
pub const LUDUS_ZONES: [ZoneRect; 3] = [
    ZoneRect {
        name: "practice_yard",
        x0: -62.0,
        x1: -2.0,
        z0: -7.0,
        z1: 7.0,
    },
    ZoneRect {
        name: "blacksmith",
        x0: -24.0,
        x1: -15.6,
        z0: -5.0,
        z1: 5.0,
    },
    ZoneRect {
        name: "party_board",
        x0: -14.6,
        x1: -10.0,
        z0: -3.0,
        z1: 3.0,
    },
];

/// One ludus spawn in meters. Values repeat `tools/map_data.rs`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpawnPoint {
    /// Spawn name (`player`, `drill_a`, `drill_b`, `drill_c`).
    pub name: &'static str,
    /// East position in meters.
    pub x: f32,
    /// South position in meters.
    pub z: f32,
    /// Facing in degrees.
    pub yaw_deg: u32,
}

/// Canonical ludus spawns: every drill stands on the practice yard.
pub const LUDUS_SPAWNS: [SpawnPoint; 4] = [
    SpawnPoint {
        name: "player",
        x: -48.0,
        z: 0.0,
        yaw_deg: 90,
    },
    SpawnPoint {
        name: "drill_a",
        x: -20.0,
        z: -4.0,
        yaw_deg: 45,
    },
    SpawnPoint {
        name: "drill_b",
        x: -20.0,
        z: 4.0,
        yaw_deg: 135,
    },
    SpawnPoint {
        name: "drill_c",
        x: -38.0,
        z: 0.0,
        yaw_deg: 90,
    },
];

/// One ludus drill route in meters. Values repeat `content/maps/ludus/map.json`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LudusRoute {
    /// Spawn the route starts from (`player`).
    pub from: &'static str,
    /// Stand the route ends at (`pell_west_stand`, `pell_east_stand`, `rack_stand`).
    pub to: &'static str,
    /// East end in meters.
    pub to_x: f32,
    /// South end in meters.
    pub to_z: f32,
}

/// Canonical ludus drill routes: every end stands on the practice yard.
pub const LUDUS_ROUTES: [LudusRoute; 3] = [
    LudusRoute {
        from: "player",
        to: "pell_west_stand",
        to_x: -8.0,
        to_z: -2.0,
    },
    LudusRoute {
        from: "player",
        to: "pell_east_stand",
        to_x: -8.0,
        to_z: 2.0,
    },
    LudusRoute {
        from: "player",
        to: "rack_stand",
        to_x: -5.0,
        to_z: 1.2,
    },
];

/// Look up a ludus drill route by stand name; unknown names read `None`.
pub fn ludus_route(to: &str) -> Option<&'static LudusRoute> {
    LUDUS_ROUTES.iter().find(|route| route.to == to)
}

/// Look up a ludus zone by name; unknown names read `None`.
pub fn ludus_zone(name: &str) -> Option<&'static ZoneRect> {
    LUDUS_ZONES.iter().find(|zone| zone.name == name)
}

/// True when `(x, z)` in meters stands inside `zone`.
pub fn ludus_contains(zone: &ZoneRect, x: f32, z: f32) -> bool {
    x >= zone.x0 && x <= zone.x1 && z >= zone.z0 && z <= zone.z1
}

/// Why a ludus write was refused. Failures store nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LudusError {
    /// Fighter slot `>= MAX_PARTICIPANTS`, culprit carried.
    BadFighter(u8),
    /// Winner and loser name the same slot.
    SameFighters,
    /// Request-side mind failure (stale revision, armed cancel).
    Mind(MindError),
}

impl core::fmt::Display for LudusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadFighter(slot) => write!(
                f,
                "ludus: fighter slot {slot} outside 0..{MAX_PARTICIPANTS}"
            ),
            Self::SameFighters => write!(f, "ludus: winner and loser must differ"),
            Self::Mind(err) => write!(f, "ludus: {err}"),
        }
    }
}

impl std::error::Error for LudusError {}

impl From<MindError> for LudusError {
    fn from(err: MindError) -> Self {
        Self::Mind(err)
    }
}

/// One witnessed bout: who won, who fell, how long it ran. Every voice
/// remembers the same bout in its own words, so memories stay consistent
/// across adapters while lines stay voiced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FightReport {
    /// Fixed tick the bout ended at.
    pub tick: u64,
    /// Winner slot, `0..MAX_PARTICIPANTS`.
    pub winner: u8,
    /// Loser slot, `0..MAX_PARTICIPANTS`, never the winner.
    pub loser: u8,
    /// Blows traded; shapes salience only.
    pub blows: u32,
}

impl FightReport {
    /// Validated bout: slots below the canonical cap and never equal.
    pub fn new(tick: u64, winner: u8, loser: u8, blows: u32) -> Result<Self, LudusError> {
        if winner >= MAX_PARTICIPANTS {
            return Err(LudusError::BadFighter(winner));
        }
        if loser >= MAX_PARTICIPANTS {
            return Err(LudusError::BadFighter(loser));
        }
        if winner == loser {
            return Err(LudusError::SameFighters);
        }
        Ok(Self {
            tick,
            winner,
            loser,
            blows,
        })
    }
}

/// One spoken line from a visit: which voice, what it said, where it came
/// from. Presentation only: a line never moves a body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisitLine {
    /// Voice that spoke.
    pub role: NpcRole,
    /// Mind handle that spoke.
    pub npc: u32,
    /// Bounded spoken text (`1..=280` chars).
    pub text: String,
    /// Scripted fallback or validated provider output.
    pub source: ActionSource,
    /// Fixed tick the line lands at (`issued..=deadline` of its ask).
    pub tick_apply: u64,
}

impl VisitLine {
    /// Presentation shape: `Trainer @practice_yard: <text>`.
    pub fn display(&self) -> String {
        format!(
            "{} @{}: {}",
            self.role.voice(),
            self.role.station(),
            self.text
        )
    }
}

/// Format one drained presentation line for the caption feed, the same shape
/// as [`VisitLine::display`]: `Trainer @practice_yard: <text>`. The sink
/// reads [`PresentationEvent`] lines only: formatting one never moves a
/// body. Unknown minds read as a stranger in the yard, never refused.
#[must_use]
pub fn display_drained(event: &PresentationEvent) -> String {
    match NpcRole::ALL.iter().find(|role| role.id() == event.npc) {
        Some(role) => format!("{} @{}: {}", role.voice(), role.station(), event.text),
        None => format!("Stranger @yard: {}", event.text),
    }
}

/// Outcome of one visit: three voiced lines plus the session checkpoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisitOutcome {
    /// Fixed tick the visit ran at.
    pub tick: u64,
    /// One line per voice in [`NpcRole::ALL`] order.
    pub lines: Vec<VisitLine>,
    /// Session checkpoint after the visit (store plus recorded lines).
    pub checkpoint: u64,
    /// Lines that fell back to the scripted table.
    pub scripted: usize,
}

/// Living ludus: three voiced minds with fight memories and goals over the
/// ludus yard. Observe bouts, then visit: asks run off-tick through any
/// mind adapter, only validated tick-stamped lines are recorded, and providers-off
/// replays reproduce every checkpoint.
pub struct LudusNpcs {
    minds: [Mind; 3],
    store: MemoryStore,
    runtime: MindRuntime,
    revision: u64,
    next_request: u64,
    /// Ask windows for the tick-boundary drain: every issued ask stays here
    /// until no logged action references it, so a later drain refuses with
    /// the true cause instead of inventing a window.
    requests: Vec<MindRequest>,
    visits: u32,
    fights: u32,
}

impl LudusNpcs {
    /// Empty ludus for one session seed: three minds with role goals and
    /// regard, an empty store, providers on, revision pinned to the seed.
    pub fn new(seed: u64) -> Self {
        let mut minds = [
            Mind::new(NpcId(TRAINER_ID), NpcRole::Trainer.seed(seed)),
            Mind::new(NpcId(BLACKSMITH_ID), NpcRole::Blacksmith.seed(seed)),
            Mind::new(NpcId(CROWD_ID), NpcRole::Crowd.seed(seed)),
        ];
        // Role goals: each voice holds two active goals the asks carry.
        let goals: [[(&str, u32); 2]; 3] = [
            [
                ("drill recruits at the pells", 800),
                ("keep the yard sand raked", 400),
            ],
            [
                ("keep blades sharp for the bouts", 800),
                ("mend the weapon rack", 300),
            ],
            [
                ("chant for the victor", 700),
                ("fill the party board slips", 200),
            ],
        ];
        for (mind, role_goals) in minds.iter_mut().zip(goals.iter()) {
            for (label, priority) in role_goals.iter() {
                // Fixture labels and priorities are valid by construction.
                let _ = mind.add_goal(label, *priority);
            }
        }
        // Regard: the staff know each other; the crowd favors the trainer.
        let regard: [(usize, NpcId, i32, u32); 6] = [
            (0, NpcId(BLACKSMITH_ID), 300, 600),
            (0, NpcId(CROWD_ID), 100, 300),
            (1, NpcId(TRAINER_ID), 300, 600),
            (1, NpcId(CROWD_ID), 0, 200),
            (2, NpcId(TRAINER_ID), 500, 400),
            (2, NpcId(BLACKSMITH_ID), 200, 300),
        ];
        for (slot, other, affinity, trust) in regard.iter() {
            let _ = minds[*slot].relate(*other, *affinity, *trust);
        }
        let mut runtime = MindRuntime::new();
        runtime.set_revision(seed);
        Self {
            minds,
            store: MemoryStore::new(),
            runtime,
            revision: seed,
            next_request: FIRST_REQUEST_ID,
            requests: Vec::new(),
            visits: 0,
            fights: 0,
        }
    }

    /// Session seed this ludus was built for (also the revision pin).
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.revision
    }

    /// Visits run so far.
    #[must_use]
    pub fn visits(&self) -> u32 {
        self.visits
    }

    /// Bouts observed so far.
    #[must_use]
    pub fn fights(&self) -> u32 {
        self.fights
    }

    /// Asks served so far (provider plus fallback plus duplicate).
    #[must_use]
    pub fn served(&self) -> u64 {
        self.runtime.served()
    }

    /// Asks that fell back to the scripted table.
    #[must_use]
    pub fn scripted_count(&self) -> u64 {
        self.runtime.scripted_count()
    }

    /// Memory records held across all voices.
    #[must_use]
    pub fn total_records(&self) -> usize {
        self.store.total_records()
    }

    /// True when no voice remembers anything yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Pin a new world revision on the runtime and future asks. Asks built
    /// against the old revision go stale instead of serving.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
        self.runtime.set_revision(revision);
    }

    /// Remember one bout in every voice, in its own words, then distill each
    /// stream into the store. Returns newly stored records. Invalid bouts are
    /// refused before any voice hears them.
    pub fn observe_fight(&mut self, report: &FightReport) -> Result<usize, LudusError> {
        // Constructor-validated reports still re-check slots here: callers
        // may build the struct literally, and a bad slot must store nothing.
        if report.winner >= MAX_PARTICIPANTS {
            return Err(LudusError::BadFighter(report.winner));
        }
        if report.loser >= MAX_PARTICIPANTS {
            return Err(LudusError::BadFighter(report.loser));
        }
        if report.winner == report.loser {
            return Err(LudusError::SameFighters);
        }
        let bout = self.fights.saturating_add(1);
        // Integer salience only: hot bouts rank higher, always in range.
        let salience = 400 + (report.blows % 601);
        let texts = [
            (
                MemoryKind::Seen,
                format!(
                    "Saw bout {bout}: fighter {} beat fighter {} in {} blows at tick {}.",
                    report.winner, report.loser, report.blows, report.tick
                ),
            ),
            (
                MemoryKind::Heard,
                format!(
                    "Heard the yard roar: fighter {} downed fighter {} after {} blows at tick {}.",
                    report.winner, report.loser, report.blows, report.tick
                ),
            ),
            (
                MemoryKind::Heard,
                format!(
                    "Chanted as fighter {} felled fighter {} in {} blows at tick {}.",
                    report.winner, report.loser, report.blows, report.tick
                ),
            ),
        ];
        let mut fresh = 0usize;
        for (mind, (kind, text)) in self.minds.iter_mut().zip(texts.iter()) {
            mind.observe(report.tick, *kind, text, salience)?;
            fresh += self.store.remember_mind(mind, report.tick);
        }
        self.fights = bout;
        Ok(fresh)
    }

    /// Run one visit through `adapter`: each voice asks in role order under
    /// the local-only privacy bound (mock and local serve; remote falls back
    /// scripted without dispatching). Provider failures fall back to voiced
    /// scripted lines and are counted, never hidden.
    pub fn visit<A: MindAdapter>(
        &mut self,
        tick: u64,
        adapter: &mut A,
    ) -> Result<VisitOutcome, LudusError> {
        self.visit_inner(tick, Privacy::LocalOnly, adapter)
    }

    /// Run one visit with providers off: no adapter call, every voice speaks
    /// its scripted line. The yard still plays with no model in reach.
    pub fn visit_offline(&mut self, tick: u64) -> VisitOutcome {
        use engine_ai::mind::MockAdapter;
        let online = self.runtime.online();
        self.runtime.set_online(false);
        let mut silent = MockAdapter::new(Vec::new());
        let outcome = self
            .visit_inner(tick, Privacy::Offline, &mut silent)
            .expect("offline visits cannot fail");
        // The offline throwaway starts uncalled and the runtime never calls
        // it while offline; a call here would mean the fallback path dialed
        // out, which the conformance suite forbids.
        debug_assert_eq!(silent.calls(), 0, "offline visit called its adapter");
        self.runtime.set_online(online);
        outcome
    }

    fn visit_inner<A: MindAdapter>(
        &mut self,
        tick: u64,
        privacy: Privacy,
        adapter: &mut A,
    ) -> Result<VisitOutcome, LudusError> {
        let mut lines = Vec::with_capacity(NpcRole::ALL.len());
        for (slot, role) in NpcRole::ALL.iter().enumerate() {
            let req = MindRequest {
                id: self.next_request,
                npc: role.id(),
                issued_tick: tick,
                deadline_tick: tick.saturating_add(VISIT_DEADLINE_TICKS),
                budget_ms: VISIT_BUDGET_MS,
                base_revision: self.revision,
                privacy,
            };
            self.next_request = self.next_request.saturating_add(1).max(FIRST_REQUEST_ID);
            let outcome = self.runtime.ask(&self.minds[slot], &req, adapter)?;
            // Keep the window for the tick-boundary drain; it is pruned in
            // `drain_at` once no logged action references it.
            self.requests.push(req.clone());
            for action in outcome.actions {
                // Say and Gesture are presentation-only; a movement draft can
                // never arrive here because every ask records at most the
                // canned scripted or Say proposal, and anything else would
                // need a new adapter the suite never blesses.
                debug_assert!(
                    matches!(
                        action.kind,
                        ActionKind::Say | ActionKind::Gesture | ActionKind::Idle
                    ),
                    "ludus voices speak, never steer"
                );
                lines.push(VisitLine {
                    role: *role,
                    npc: role.id().0,
                    text: action.text,
                    source: action.source,
                    tick_apply: action.tick_apply,
                });
            }
        }
        self.visits = self.visits.saturating_add(1);
        let scripted = lines
            .iter()
            .filter(|line| line.source == ActionSource::Scripted)
            .count();
        let checkpoint = self.checkpoint();
        Ok(VisitOutcome {
            tick,
            lines,
            checkpoint,
            scripted,
        })
    }

    /// Lines due at `tick` (stamp `<= tick`), acceptance order. A
    /// presentation-only read: the recorded log is untouched.
    #[must_use]
    pub fn lines_due_at(&self, tick: u64) -> Vec<engine_ai::mind::MindAction> {
        self.runtime
            .recorded()
            .due_at(tick)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Tick-boundary drain through the apply door: moves recorded lines
    /// due at `tick` out of the runtime and sorts them into typed effects
    /// under their ask windows. Say/Gesture land in `batch.presented` for
    /// the caption feed only (see [`display_drained`]); voices never steer,
    /// so a movement intent rests in `batch.intended`, never applied to a
    /// body. Unknown ask ids refuse as malformed and out-of-window stamps
    /// refuse with their cause; drained lines never re-apply and future
    /// stamps stay logged. Budget: one pass over due lines; caption lines
    /// may allocate (visits run off-tick, never on the arena hot path, so
    /// the golden never wobbles).
    pub fn drain_at(&mut self, tick: u64) -> AppliedBatch {
        let due = self.runtime.drain_due(tick);
        let mut batch = AppliedBatch::new();
        for action in &due {
            // Mirrors `apply_one`: owner plus ask-id checks, then the frozen
            // `apply_at` door. Unknown asks refuse instead of a fresh window.
            let outcome = match self.requests.iter().find(|req| req.id == action.request) {
                Some(req) if action.npc == req.npc => {
                    apply_at(action, req.issued_tick, req.deadline_tick)
                }
                Some(_) => Err(MindError::UnknownNpc),
                None => Err(MindError::Malformed),
            };
            match outcome {
                Ok(Applied::Presented(event)) => batch.presented.push(event),
                Ok(Applied::Intended(intent)) => batch.intended.push(intent),
                Err(cause) => batch.refused.push(Refused {
                    action: action.clone(),
                    cause,
                }),
            }
        }
        // Windows with no logged action left are history; future stamps keep
        // theirs so a later drain refuses with the true cause, not malformed.
        self.requests.retain(|req| {
            self.runtime
                .recorded()
                .entries()
                .iter()
                .any(|entry| entry.request == req.id)
        });
        batch
    }

    /// Ranked recall pull for one voice at `tick`: the dialogue shape.
    #[must_use]
    pub fn recall(&self, role: NpcRole, tick: u64, limit: usize) -> Recall {
        self.store.recall(role.id(), tick, limit)
    }

    /// Contradictory memory pairs for one voice (eval input).
    #[must_use]
    pub fn contradictions(&self, role: NpcRole) -> usize {
        self.store.contradictions(role.id())
    }

    /// Deterministic memory bytes over every voice (memory store layout).
    #[must_use]
    pub fn memory_bytes(&self) -> Vec<u8> {
        self.store.memory_bytes()
    }

    /// Deterministic checkpoint over memory bytes alone.
    #[must_use]
    pub fn memory_checkpoint(&self) -> u64 {
        self.store.checkpoint()
    }

    /// Deterministic checkpoint over recorded lines alone.
    #[must_use]
    pub fn log_checkpoint(&self) -> u64 {
        self.runtime.recorded().checkpoint()
    }

    /// Recorded lines in acceptance order (the offline-replay input).
    #[must_use]
    pub fn recorded_entries(&self) -> Vec<engine_ai::mind::MindAction> {
        self.runtime.recorded().entries().to_vec()
    }

    /// Rebuild a log from recorded entries with no provider and return its
    /// checkpoint: providers-off replay reproduces the live log exactly.
    pub fn replay_checkpoint(entries: &[engine_ai::mind::MindAction]) -> Result<u64, MindError> {
        let (_, checkpoint) = MindRuntime::replay_offline(entries)?;
        Ok(checkpoint)
    }

    /// Session checkpoint: memory plus recorded lines plus visit counters in
    /// one FNV-1a fold. Same fights plus same visits always give the same
    /// checkpoint; any changed line changes it.
    #[must_use]
    pub fn checkpoint(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write(&self.memory_checkpoint().to_le_bytes());
        hash.write(&self.log_checkpoint().to_le_bytes());
        hash.write(&self.visits.to_le_bytes());
        hash.write(&self.fights.to_le_bytes());
        hash.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ai::mind::{LocalAdapter, MockAdapter, ScriptStep};

    const SEED: u64 = 2040;

    fn fight(visit: u64) -> FightReport {
        let winner = (visit % 2) as u8;
        FightReport::new(visit * 10, winner, 1 - winner, 5 + (visit as u32) * 3)
            .expect("fixture fight is valid")
    }

    fn twenty_visits<A: MindAdapter>(ludus: &mut LudusNpcs, adapter: &mut A) -> Vec<VisitOutcome> {
        let mut outcomes = Vec::with_capacity(20);
        for visit in 0..20u64 {
            let fresh = ludus
                .observe_fight(&fight(visit))
                .expect("fixture remembers");
            assert!(fresh > 0, "visit {visit} stores new memories");
            let outcome = ludus
                .visit(visit * 10 + 5, adapter)
                .expect("fixture visits");
            assert_eq!(outcome.lines.len(), 3, "every voice speaks");
            outcomes.push(outcome);
        }
        outcomes
    }

    #[test]
    fn ludus_twenty_visits_consistent_across_mock_and_local() {
        let mut mock_ludus = LudusNpcs::new(SEED);
        let mut mock = MockAdapter::new(Vec::new());
        let mock_outcomes = twenty_visits(&mut mock_ludus, &mut mock);

        let mut local_ludus = LudusNpcs::new(SEED);
        let mut local = LocalAdapter::new(Vec::new());
        let local_outcomes = twenty_visits(&mut local_ludus, &mut local);

        // Memories of the fights are adapter-blind: same bouts give the same
        // store bytes through mock and local.
        assert_eq!(mock_ludus.memory_bytes(), local_ludus.memory_bytes());
        assert_eq!(
            mock_ludus.memory_checkpoint(),
            local_ludus.memory_checkpoint()
        );
        assert_eq!(mock_ludus.total_records(), local_ludus.total_records());
        assert!(mock_ludus.total_records() > 0, "visits remember fights");

        // Both adapters really ran: every line is provider-sourced and each
        // receipt names its backend.
        for outcome in mock_outcomes.iter().chain(local_outcomes.iter()) {
            assert_eq!(outcome.scripted, 0, "clean script serves everywhere");
            assert_eq!(outcome.lines.len(), 3);
        }
        for line in mock_outcomes.iter().flat_map(|o| o.lines.iter()) {
            assert!(line.text.contains("mock:ok"), "mock receipt tags backend");
            assert!(line.text.contains("recalls"), "line cites its memory");
        }
        for line in local_outcomes.iter().flat_map(|o| o.lines.iter()) {
            assert!(line.text.contains("local:ok"), "local receipt tags backend");
        }
        // No voice contradicts itself across the run.
        for role in NpcRole::ALL {
            assert_eq!(mock_ludus.contradictions(role), 0);
            assert_eq!(local_ludus.contradictions(role), 0);
        }
        println!(
            "ludus_npcs visits=20 mock_records={} local_records={} memory_checkpoint={:016x} bytes={}",
            mock_ludus.total_records(),
            local_ludus.total_records(),
            mock_ludus.memory_checkpoint(),
            mock_ludus.memory_bytes().len(),
        );
    }

    #[test]
    fn ludus_same_backend_identical_twice() {
        let run = || {
            let mut ludus = LudusNpcs::new(SEED);
            let mut mock = MockAdapter::new(Vec::new());
            let outcomes = twenty_visits(&mut ludus, &mut mock);
            (ludus.checkpoint(), ludus.recorded_entries(), outcomes)
        };
        let (first_hash, first_log, first_outcomes) = run();
        let (second_hash, second_log, second_outcomes) = run();
        assert_eq!(first_hash, second_hash);
        assert_eq!(first_log, second_log);
        assert_eq!(first_outcomes, second_outcomes);
        println!(
            "ludus_npcs replay_x2 checkpoint={first_hash:016x} lines={}",
            first_log.len()
        );
    }

    #[test]
    fn ludus_providers_off_still_plays() {
        let mut ludus = LudusNpcs::new(SEED);
        let mut mock = MockAdapter::new(Vec::new());
        for visit in 0..5u64 {
            ludus
                .observe_fight(&fight(visit))
                .expect("fixture remembers");
            let outcome = ludus
                .visit(visit * 10 + 5, &mut mock)
                .expect("fixture visits");
            assert_eq!(outcome.scripted, 0);
        }
        let live_log = ludus.recorded_entries();
        let live_checkpoint = ludus.log_checkpoint();

        // Providers off: the yard still plays, every voice scripted.
        for visit in 5..8u64 {
            ludus
                .observe_fight(&fight(visit))
                .expect("fixture remembers");
            let outcome = ludus.visit_offline(visit * 10 + 5);
            assert_eq!(outcome.lines.len(), 3, "offline visit still speaks");
            assert_eq!(outcome.scripted, 3, "offline lines are scripted");
            for line in &outcome.lines {
                assert_eq!(line.source, ActionSource::Scripted);
                assert!(!line.text.is_empty(), "scripted line says something");
            }
        }

        // Recorded lines replay with no provider to the live checkpoint.
        let replayed = LudusNpcs::replay_checkpoint(&live_log).expect("replays offline");
        assert_eq!(
            replayed, live_checkpoint,
            "offline replay reproduces the log"
        );
        // And the full run (live plus offline) replays from its own record.
        let full = ludus.recorded_entries();
        let full_checkpoint = ludus.log_checkpoint();
        assert_eq!(
            LudusNpcs::replay_checkpoint(&full).expect("replays full"),
            full_checkpoint
        );
        println!(
            "ludus_npcs offline lines={} replay_checkpoint={replayed:016x}",
            full.len() - live_log.len(),
        );
    }

    #[test]
    fn ludus_provider_timeout_falls_back_but_remembers() {
        let mut ludus = LudusNpcs::new(SEED);
        let mut flaky = LocalAdapter::new(vec![
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
        ]);
        ludus.observe_fight(&fight(0)).expect("fixture remembers");
        let outcome = ludus.visit(5, &mut flaky).expect("timeout still visits");
        assert_eq!(outcome.lines.len(), 3);
        assert_eq!(outcome.scripted, 3, "blown budget falls back voiced");
        for line in &outcome.lines {
            assert_eq!(line.source, ActionSource::Scripted);
        }
        // The bout is remembered even though the provider failed.
        assert!(ludus.total_records() > 0, "fallback keeps memories");
        assert_eq!(ludus.scripted_count(), 3);
        println!(
            "ludus_npcs timeout scripted={} records={}",
            outcome.scripted,
            ludus.total_records()
        );
    }

    #[test]
    fn ludus_bad_fight_refused_without_memory() {
        let mut ludus = LudusNpcs::new(SEED);
        assert_eq!(
            FightReport::new(0, MAX_PARTICIPANTS, 0, 10),
            Err(LudusError::BadFighter(MAX_PARTICIPANTS))
        );
        assert_eq!(
            FightReport::new(0, 0, MAX_PARTICIPANTS, 10),
            Err(LudusError::BadFighter(MAX_PARTICIPANTS))
        );
        assert_eq!(FightReport::new(0, 1, 1, 10), Err(LudusError::SameFighters));
        // A literally built bad report is refused at observe time too and
        // stores nothing: the failure path writes no memory.
        let bad = FightReport {
            tick: 0,
            winner: 2,
            loser: 2,
            blows: 4,
        };
        assert_eq!(ludus.observe_fight(&bad), Err(LudusError::SameFighters));
        let wild = FightReport {
            tick: 0,
            winner: 99,
            loser: 0,
            blows: 4,
        };
        assert_eq!(ludus.observe_fight(&wild), Err(LudusError::BadFighter(99)));
        assert!(ludus.is_empty(), "refusals store nothing");
        assert_eq!(ludus.fights(), 0);
    }

    #[test]
    fn ludus_map_binding_matches_wd2() {
        assert_eq!(LUDUS_MAP_NAME, "ludus");
        assert_eq!(LUDUS_MAP_VERSION, 1);
        // Every voice staffs a known zone.
        for role in NpcRole::ALL {
            let zone = ludus_zone(role.station())
                .unwrap_or_else(|| panic!("station {} is a WD2 zone", role.station()));
            assert_eq!(zone.name, role.station());
        }
        assert!(ludus_zone("colosseum").is_none(), "unknown zones read None");
        // Corners nest inside the training grounds and stay disjoint.
        let yard = ludus_zone("practice_yard").expect("yard exists");
        for corner in ["blacksmith", "party_board"] {
            let zone = ludus_zone(corner).expect("corner exists");
            assert!(
                zone.x0 >= yard.x0 && zone.x1 <= yard.x1,
                "{corner} sits in the yard"
            );
            assert!(
                zone.z0 >= yard.z0 && zone.z1 <= yard.z1,
                "{corner} sits in the yard"
            );
        }
        let forge = ludus_zone("blacksmith").expect("forge exists");
        let board = ludus_zone("party_board").expect("board exists");
        assert!(
            forge.x1 <= board.x0 || board.x1 <= forge.x0,
            "corners never overlap"
        );
        // Every spawn stands on free yard ground.
        for spawn in LUDUS_SPAWNS {
            assert!(
                ludus_contains(yard, spawn.x, spawn.z),
                "spawn {} stands in the yard",
                spawn.name
            );
        }
        // The voices stand where they work: trainer in the yard, smith at
        // the forge, crowd at the board.
        assert!(ludus_contains(yard, -48.0, 0.0));
        assert!(ludus_contains(forge, -19.0, 0.0));
        assert!(ludus_contains(board, -13.0, 0.0));
        assert!(!ludus_contains(board, -48.0, 0.0), "yard is not the board");
        // Hub boot binding: yard nav rect, player spawn and drill routes.
        assert_eq!(
            (yard.x0, yard.x1, yard.z0, yard.z1),
            (-62.0, -2.0, -7.0, 7.0)
        );
        assert_eq!(
            LUDUS_SPAWNS[0],
            SpawnPoint {
                name: "player",
                x: -48.0,
                z: 0.0,
                yaw_deg: 90
            }
        );
        assert_eq!(LUDUS_ROUTES.len(), 3);
        for route in LUDUS_ROUTES {
            assert!(
                ludus_contains(yard, route.to_x, route.to_z),
                "route {} stands in the yard",
                route.to
            );
        }
        assert!(ludus_route("pell_west_stand").is_some());
        assert!(ludus_route("rack_stand").is_some());
        assert!(ludus_route("colosseum_stand").is_none());
    }

    #[test]
    fn ludus_lines_due_and_recall_deterministic() {
        let mut ludus = LudusNpcs::new(SEED);
        let mut mock = MockAdapter::new(Vec::new());
        twenty_visits(&mut ludus, &mut mock);
        // Tick-boundary reads serve every recorded line exactly once in
        // order, without mutating the log.
        let due = ludus.lines_due_at(195);
        assert_eq!(due.len(), 60, "20 visits x 3 voices are all due");
        assert_eq!(ludus.lines_due_at(195).len(), 60, "due reads never drain");
        assert!(
            ludus.lines_due_at(4).is_empty(),
            "nothing lands before tick 5"
        );
        // Recall pulls fight memories deterministically twice.
        let first = ludus.recall(NpcRole::Trainer, 195, 4);
        let second = ludus.recall(NpcRole::Trainer, 195, 4);
        assert_eq!(first, second);
        assert!(!first.is_empty(), "trainer remembers the bouts");
        assert!(
            first
                .hits
                .iter()
                .any(|hit| hit.text.contains("beat fighter")),
            "trainer recall cites a witnessed bout"
        );
        let smith = ludus.recall(NpcRole::Blacksmith, 195, 4);
        assert!(
            smith
                .hits
                .iter()
                .any(|hit| hit.text.contains("downed fighter")),
            "smith recall cites a heard bout"
        );
    }

    #[test]
    fn ludus_request_ids_unique_and_stamped() {
        let mut ludus = LudusNpcs::new(SEED);
        let mut mock = MockAdapter::new(Vec::new());
        twenty_visits(&mut ludus, &mut mock);
        let entries = ludus.recorded_entries();
        assert_eq!(entries.len(), 60);
        let mut ids: Vec<u64> = entries.iter().map(|entry| entry.request).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 60, "every ask takes a fresh request id");
        // Stamps land inside their ask window: issued tick 5..195, deadline
        // at most 8 ticks later.
        for entry in &entries {
            let visit = (entry.request - FIRST_REQUEST_ID) / 3;
            let issued = visit * 10 + 5;
            assert!(
                entry.tick_apply >= issued && entry.tick_apply <= issued + VISIT_DEADLINE_TICKS,
                "line stamped inside its ask window"
            );
        }
        // Presentation shapes name the voice and its station.
        let shown = VisitLine {
            role: NpcRole::Blacksmith,
            npc: BLACKSMITH_ID,
            text: "steel sings".to_owned(),
            source: ActionSource::Scripted,
            tick_apply: 5,
        };
        assert_eq!(shown.display(), "Blacksmith @blacksmith: steel sings");
    }

    /// Twenty providers-off visits drained through the real sink, twice.
    fn drained_twenty() -> (AppliedBatch, Vec<String>) {
        let mut ludus = LudusNpcs::new(SEED);
        for visit in 0..20u64 {
            ludus
                .observe_fight(&fight(visit))
                .expect("fixture remembers");
            let outcome = ludus.visit_offline(visit * 10 + 5);
            assert_eq!(outcome.lines.len(), 3, "offline visit still speaks");
            assert_eq!(outcome.scripted, 3, "offline lines are scripted");
        }
        let batch = ludus.drain_at(1_000);
        let shown = batch
            .presented
            .iter()
            .map(display_drained)
            .collect::<Vec<_>>();
        assert!(ludus.recorded_entries().is_empty(), "drain removes lines");
        (batch, shown)
    }

    #[test]
    fn drain_twenty_offline_visits_apply_identically_twice() {
        let (first, first_shown) = drained_twenty();
        let (second, second_shown) = drained_twenty();
        assert_eq!(first, second, "same asks drain identically twice");
        assert_eq!(
            first.checkpoint(),
            second.checkpoint(),
            "checkpoints match x2"
        );
        assert_eq!(first.presented.len(), 60, "20 visits x 3 voices present");
        assert!(first.intended.is_empty(), "voices speak, never steer");
        assert!(first.refused.is_empty(), "in-window stamps never refuse");
        assert_eq!(first_shown, second_shown);
        // Every caption names its voice and station; none is empty.
        for (line, event) in first_shown.iter().zip(first.presented.iter()) {
            assert!(line.contains('@'), "caption names a station: {line}");
            assert!(line.ends_with(&event.text), "caption carries the line");
            assert!(!event.text.is_empty(), "drained lines say something");
        }
        assert!(
            first_shown[0].starts_with("Trainer @practice_yard: "),
            "first voice staffs the yard: {}",
            first_shown[0]
        );
        // Divergence hygiene: one retargeted line changes the checkpoint.
        let mut changed = second.clone();
        changed.presented[4].text.push('!');
        assert_ne!(
            changed.checkpoint(),
            first.checkpoint(),
            "changed input changes the checkpoint"
        );
    }

    #[test]
    fn drain_say_never_moves_a_body() {
        use engine_ai::gladiator::Intent;
        let mut ludus = LudusNpcs::new(SEED);
        ludus.observe_fight(&fight(0)).expect("fixture remembers");
        ludus.visit_offline(5);
        // Bodies stand on their marks; only intents may steer them.
        let mut bodies = [(10i32, 20i32), (30, 40), (50, 60), (70, 80)];
        let before = bodies;
        let batch = ludus.drain_at(1_000);
        assert_eq!(batch.presented.len(), 3, "talk presents three lines");
        assert!(batch.intended.is_empty(), "talk never intends");
        assert!(batch.refused.is_empty());
        // The arena steers bodies from intents only: no intent, no motion.
        for intent in &batch.intended {
            let slot = (intent.npc.0 as usize) % bodies.len();
            bodies[slot].0 += match intent.brain_intent() {
                Intent::Advance => 1,
                Intent::Retreat => -1,
                _ => 0,
            };
        }
        assert_eq!(bodies, before, "Say moved no body");
        // The caption feed reads the same lines the visit recorded.

        assert_eq!(batch.presented.iter().map(display_drained).count(), 3);
        assert_eq!(
            ludus.lines_due_at(1_000).len(),
            0,
            "drained log stays empty"
        );
    }

    #[test]
    fn drain_future_stays_history_never_reapplies() {
        let mut ludus = LudusNpcs::new(SEED);
        ludus.observe_fight(&fight(0)).expect("fixture remembers");
        ludus.visit_offline(500);
        // Early drain: future stamps stay logged, nothing applies.
        let early = ludus.drain_at(100);
        assert!(early.is_empty() && early.refused.is_empty());
        assert_eq!(
            ludus.recorded_entries().len(),
            3,
            "future stamps stay history"
        );
        // Due drain presents all three; voices never steer, none refuse.
        let batch = ludus.drain_at(1_000);
        assert_eq!(batch.presented.len(), 3);
        assert!(batch.intended.is_empty() && batch.refused.is_empty());
        assert!(ludus.recorded_entries().is_empty(), "drain removes lines");
        // Same tick again: nothing due, nothing re-applied.
        let replay = ludus.drain_at(1_000);
        assert!(replay.is_empty() && replay.refused.is_empty());
    }

    /// NS14: a visit recorded with the provider on replays with the provider
    /// off, twice, and the talk log comes back byte for byte.
    #[test]
    fn roman_visit_talk_log_replays_twice_with_the_provider_off() {
        let mut ludus = LudusNpcs::new(SEED);
        let mut mock = MockAdapter::new(Vec::new());
        twenty_visits(&mut ludus, &mut mock);
        let talk = ludus.recorded_entries();
        assert_eq!(talk.len(), 60, "20 visits x 3 voices talk");
        assert!(talk
            .iter()
            .all(|line| line.source == ActionSource::Provider));

        let replay = || {
            let (log, checkpoint) = MindRuntime::replay_offline(&talk).expect("replays offline");
            (log.entries().to_vec(), checkpoint)
        };
        let (first, first_checkpoint) = replay();
        let (second, second_checkpoint) = replay();
        assert_eq!(first, talk, "talk log identical with the provider off");
        assert_eq!(second, talk, "and identical again");
        assert_eq!(first_checkpoint, ludus.log_checkpoint());
        assert_eq!(second_checkpoint, first_checkpoint);

        // Negative control: one changed byte of one line changes the checkpoint.
        let mut tampered = talk.clone();
        tampered[7].text.push('!');
        let (_, other) = MindRuntime::replay_offline(&tampered).expect("replays offline");
        assert_ne!(
            other, first_checkpoint,
            "a changed line shows in the checkpoint"
        );
    }

    const SOAK_SESSIONS: u64 = 64;
    const SOAK_BOUTS: u64 = 160;
    /// Sessions x bouts x three voices, one line each.
    const SOAK_LINES: usize = 64 * 160 * 3;

    /// One LW12 pass: 64 ludus sessions, each three voices over 160 bouts. At
    /// every tick boundary each voice pulls its top-8 recall and every line
    /// due drains through `apply_at`. Returns the folded checkpoint, the lines
    /// presented and the recall and drain timings in nanoseconds (timings
    /// are printed, never hashed).
    fn soak_pass() -> (u64, usize, Vec<u128>, Vec<u128>) {
        use engine_ai::memory::{MAX_RECALL, MAX_STORED_PER_NPC};
        let mut fold = Fnv64::new();
        let mut presented = 0;
        let (mut recall_ns, mut drain_ns) = (Vec::new(), Vec::new());
        for session in 0..SOAK_SESSIONS {
            let mut ludus = LudusNpcs::new(SEED + session);
            for bout in 0..SOAK_BOUTS {
                ludus
                    .observe_fight(&fight(bout))
                    .expect("fixture remembers");
                let tick = bout * 10 + 5;
                ludus.visit_offline(tick);
                let started = std::time::Instant::now();
                for role in NpcRole::ALL {
                    let recall = ludus.recall(role, tick, MAX_RECALL);
                    assert!(recall.len() <= MAX_RECALL, "recall stays inside top 8");
                    if bout >= 8 {
                        assert_eq!(recall.len(), MAX_RECALL, "a full store fills the top 8");
                    }
                }
                recall_ns.push(started.elapsed().as_nanos());
                let started = std::time::Instant::now();
                let batch = ludus.drain_at(tick + VISIT_DEADLINE_TICKS);
                drain_ns.push(started.elapsed().as_nanos());
                assert!(batch.refused.is_empty(), "in-window lines never refuse");
                assert!(batch.intended.is_empty(), "voices speak, never steer");
                presented += batch.presented.len();
            }
            assert!(
                ludus.total_records() <= NpcRole::ALL.len() * MAX_STORED_PER_NPC,
                "memory caps hold"
            );
            assert!(ludus.recorded_entries().is_empty(), "every line drained");
            fold.write(&ludus.checkpoint().to_le_bytes());
        }
        (fold.finish(), presented, recall_ns, drain_ns)
    }

    fn p95(samples: &mut [u128]) -> u128 {
        samples.sort_unstable();
        samples[samples.len() * 95 / 100]
    }

    #[test]
    fn ludus_soak_64_sessions_hold_caps_and_drain_clean_twice() {
        let (first, presented, mut recall_ns, mut drain_ns) = soak_pass();
        let (second, again, _, _) = soak_pass();
        assert_eq!(first, second, "the soak replays x2");
        assert_eq!(presented, again);
        assert_eq!(presented, SOAK_LINES, "every voice's line presented once");
        println!(
            "ludus_npcs soak sessions={SOAK_SESSIONS} bouts={SOAK_BOUTS} lines={presented} \
             recall_p95_ns={} drain_p95_ns={} checkpoint={first:016x}",
            p95(&mut recall_ns),
            p95(&mut drain_ns),
        );
    }
}
