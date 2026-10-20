//! siege: 32v32 siege mode over four 16-fighter arena shards.
//!
//! Consumer: the dedicated-server operator and the replay proof —
//! [`SiegeServer`] authors a bot-filled 64-slot siege on four
//! [`HeadlessServer`](engine_net::dedicated::HeadlessServer)s and
//! [`run_siege`] replays it exactly. Data owner: this module owns the
//! siege config, the wall/bombardment ledger and the siege hash; shard
//! fight truth stays in `engine-arena`, replication budgets in
//! `engine-net`, cells in `engine-world`. Units: fixed ticks at the
//! arena rate, wall hp in integer points, entity positions in
//! millimetres. Determinism scope: same [`SiegeConfig`] gives identical
//! checkpoints, walls, events and interest ledgers — integer and SplitMix
//! math only (`+ - * /`, wrapping ops), stable shard order, no wall
//! clock, no float in hashed state. Errors: [`SiegeError`] at every
//! boundary; invalid input is rejected loudly, never skipped.
//!
//! Shape: 64 slots are four 16-fighter shards. The canonical
//! `MAX_PARTICIPANTS` cap is 32, and siege still composes four fixed
//! 16-fighter shards (a shard size, never a reopened cap). Shards 0–1 are the
//! attackers, shards 2–3 the defenders; global slot `g` is shard `g / 16`,
//! local slot `g % 16`. Every shard runs all-AI through the arena brain fill,
//! so a bot-filled siege needs no human inputs; connected humans stage
//! through [`SiegeServer::submit`] onto their shard's server. Walls are
//! eight integer-hp sections bombarded by four deterministic engines on
//! a fixed schedule with sorted `(tick, engine)` impacts; the arena's
//! own bonded prop already folds sword-hit breaks into every
//! shard checkpoint, and the engine ledger here is the schedule of
//! record for shard bombardment impacts. Replication samples the
//! interest API at the 10 Hz snapshot cadence over the 64 fighter
//! entities (the full 64-viewer + 500-soldier soak stays owned by
//! `engine-net`). Cells stream in `engine-world`: shards bind to
//! sectors by seed plus sector index and publish at frame boundaries;
//! this module keeps no cell types.
//!
//! Two drivers, one truth: the dedicated path authors tick by tick
//! through the dedicated servers (prefix re-runs, the net authority shape),
//! while [`run_siege`] evaluates each shard once and replays the same
//! ledger over the checkpoint streams. Both share [`Ledger`] and
//! [`InterestSample`], and the cross-path test pins them equal, so the
//! 10-minute soak is exactly what the dedicated server authors.
//!
//! Budget: one siege tick authors four arena ticks (the dedicated-server tick cost,
//! owned by net/arena) plus O(engines + wall sections) integer work and
//! one hash fold here — no heap use on the siege path once constructed.
//! Interest selects run every sixth tick per tracked viewer, O(64) with
//! caller-held buffers. Nothing here runs presentation.

use engine_arena::{run as arena_run, seed as arena_seed, ArenaConfig, Checkpoint, FighterTape};
use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
use engine_input::FighterInput;
use engine_net::dedicated::{HeadlessServer, ServerError};
use engine_net::interest::{
    ConnectionInterest, InterestCfg, InterestEntity, InterestError, SIEGE_BUDGET_BYTES,
    SNAPSHOT_EVERY_TICKS,
};

/// Arena shards composing one siege (4 x 16 = 64 slots).
pub const SIEGE_SHARDS: usize = 4;
/// Fighters per shard (a composition size, not the canonical match cap).
pub const SHARD_FIGHTERS: u8 = 16;
/// Global slots in one siege.
pub const SIEGE_SLOTS: usize = 64;
/// Ten minutes at the arena tick rate. Single owner is
/// `engine_net::tick_budget::SIEGE_TICKS_10MIN`. Value never changes.
pub const SIEGE_TICKS_10MIN: u32 = engine_net::tick_budget::SIEGE_TICKS_10MIN;
/// Largest siege accepted (mirrors the arena replay bound).
pub const SIEGE_MAX_TICKS: u32 = 100_000;
/// Wall sections in the bombardment ledger.
pub const WALL_SECTIONS: usize = 8;
/// Hit points per wall section (integer ledger, not physics): 40 engine
/// impacts at full damage, so breaches spread across a 10-minute siege.
pub const WALL_SECTION_HP: u32 = 6_000;
/// Bombardment engines (one per shard sector).
pub const SIEGE_ENGINES: usize = 4;
/// Engine cadence in ticks (3 s at 60 Hz).
pub const ENGINE_EVERY_TICKS: u32 = 180;
/// Damage per engine impact (integer ledger points).
pub const ENGINE_DAMAGE: u32 = 150;
/// Tracked replication viewers sampling the interest path.
pub const INTEREST_VIEWERS: usize = 8;

/// Actors in the siege placement probe (`placement_siege.json`).
pub const SIEGE_FIELD_ACTORS: usize = 7;
/// Gate x in meters (`fort_gate` in the probe).
pub const SIEGE_FIELD_GATE_X: f32 = -10.0;
/// Engine x in meters (`ballista_one` in the probe, firing through `SOCKET_muzzle`).
pub const SIEGE_FIELD_BALLISTA_X: f32 = 16.0;
/// Embedded siege placement probe bytes.
pub const SIEGE_PLACEMENT_JSON: &str = include_str!("../../../content/maps/placement_siege.json");

/// Validated siege field probe: gate plus engine x and actor count.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SiegeField {
    /// Gate x in meters.
    pub gate_x: f32,
    /// Engine x in meters.
    pub ballista_x: f32,
    /// Actors in the probe.
    pub actors: usize,
}

impl SiegeField {
    /// True when the gate sits inside the half-extent on the x axis.
    #[must_use]
    pub const fn gate_reachable(self, half_extent_m: f32) -> bool {
        self.gate_x >= -half_extent_m && self.gate_x <= half_extent_m
    }
}

/// Parse the x value following `"name": "<wanted>"` in placement text.
fn field_x_for(text: &str, wanted: &str) -> Result<f32, SiegeError> {
    let name_at = text
        .find(wanted)
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let tail = text
        .get(name_at..)
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let x_at = tail
        .find("\"x\"")
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let after = tail
        .get(x_at + 3..)
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let colon = after
        .find(':')
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let num = after
        .get(colon + 1..)
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let trimmed = num.trim_start();
    let mut end = trimmed.len();
    for (index, ch) in trimmed.char_indices() {
        if ch == ',' || ch == '}' {
            end = index;
            break;
        }
    }
    let token = trimmed
        .get(..end)
        .ok_or(SiegeError::BadConfig("siege_field"))?;
    let value: f32 = token
        .trim()
        .parse()
        .map_err(|_| SiegeError::BadConfig("siege_field"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(SiegeError::BadConfig("siege_field"))
    }
}

/// Validate one probe: 7 actors, gate at -10 plus engine at 16.
pub fn siege_field_from_json(text: &str) -> Result<SiegeField, SiegeError> {
    if text.match_indices("\"id\"").count() != SIEGE_FIELD_ACTORS {
        return Err(SiegeError::BadConfig("siege_field"));
    }
    let gate_x = field_x_for(text, "fort_gate")?;
    let ballista_x = field_x_for(text, "ballista_one")?;
    if (gate_x - SIEGE_FIELD_GATE_X).abs() > 0.001 {
        return Err(SiegeError::BadConfig("siege_field"));
    }
    if (ballista_x - SIEGE_FIELD_BALLISTA_X).abs() > 0.001 {
        return Err(SiegeError::BadConfig("siege_field"));
    }
    Ok(SiegeField {
        gate_x,
        ballista_x,
        actors: SIEGE_FIELD_ACTORS,
    })
}

/// Validate the embedded siege placement probe.
/// Same bytes give the same field; drift fails loudly at construction time.
pub fn siege_field() -> Result<SiegeField, SiegeError> {
    siege_field_from_json(SIEGE_PLACEMENT_JSON)
}

/// Siege configuration: one bot-filled war.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SiegeConfig {
    /// Master seed; expanded per shard with the non-aliasing v2 stream.
    pub seed: u64,
    /// Fixed steps, `1..=SIEGE_MAX_TICKS`.
    pub ticks: u32,
    /// Arena half-extent per shard in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
}

impl SiegeConfig {
    /// Range-check every field; NaN or out-of-range fails with the row.
    pub fn validate(self) -> Result<(), SiegeError> {
        if self.ticks == 0 || self.ticks > SIEGE_MAX_TICKS {
            return Err(SiegeError::BadConfig("ticks"));
        }
        if !self.half_extent_m.is_finite() || self.half_extent_m < 5.0 || self.half_extent_m > 100.0
        {
            return Err(SiegeError::BadConfig("half_extent_m"));
        }
        Ok(())
    }

    /// Lower one shard to its arena config with a non-aliased stream seed.
    fn shard_config(self, shard: usize) -> ArenaConfig {
        ArenaConfig {
            seed: arena_seed::expand(self.seed, SIEGE_SHARDS)[shard],
            ticks: self.ticks,
            fighters: SHARD_FIGHTERS,
            half_extent_m: self.half_extent_m,
            table_version: 1,
        }
    }
}

/// Siege failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiegeError {
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Dedicated-shard failure (bad input, stale token, desync report).
    Server(ServerError),
    /// Interest-selection failure from the sampled replication path.
    Interest(InterestError),
    /// The siege already authored `config.ticks` ticks.
    MatchOver,
    /// A shard run rejected its config or tape (never on valid input).
    SimRejected,
}

impl core::fmt::Display for SiegeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad siege config ({row})"),
            Self::Server(err) => write!(f, "siege server error ({err})"),
            Self::Interest(err) => write!(f, "siege interest error ({err})"),
            Self::MatchOver => write!(f, "siege already ran its configured ticks"),
            Self::SimRejected => write!(f, "siege shard run rejected"),
        }
    }
}

impl std::error::Error for SiegeError {}

impl From<ServerError> for SiegeError {
    fn from(err: ServerError) -> Self {
        Self::Server(err)
    }
}

impl From<InterestError> for SiegeError {
    fn from(err: InterestError) -> Self {
        Self::Interest(err)
    }
}

/// Bombardment/wall facts for one siege tick, in `(tick, engine)` order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SiegeEvent {
    /// An engine fired and damaged a wall section.
    EngineFired {
        /// Fixed tick the impact landed on.
        tick: u32,
        /// Engine index, `0..SIEGE_ENGINES`.
        engine: u8,
        /// Wall section hit, `0..WALL_SECTIONS`.
        section: u8,
        /// Ledger damage dealt (0 when the section already fell).
        damage: u32,
    },
    /// A wall section reached zero hp.
    WallBreached {
        /// Fixed tick the section fell on.
        tick: u32,
        /// Wall section, `0..WALL_SECTIONS`.
        section: u8,
    },
}

impl SiegeEvent {
    /// Fixed tick the event landed on.
    #[must_use]
    pub const fn tick(self) -> u32 {
        match self {
            Self::EngineFired { tick, .. } | Self::WallBreached { tick, .. } => tick,
        }
    }
}

/// Outcome of one authored siege tick: shard tips plus ledger counts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SiegeTick {
    /// Fixed tick just authored.
    pub tick: u32,
    /// Authoritative checkpoint per shard, in shard order.
    pub tips: [Checkpoint; SIEGE_SHARDS],
    /// Engines that fired this tick.
    pub fired: u8,
    /// Wall sections that fell this tick.
    pub breached: u8,
}

/// Completed-siege report: everything a replay must reproduce.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SiegeReport {
    /// Master seed the siege ran under.
    pub seed: u64,
    /// Ticks authored.
    pub ticks: u32,
    /// Converged siege hash (two runs must agree).
    pub hash: u64,
    /// Wall hp per section at the final tick.
    pub walls: [u32; WALL_SECTIONS],
    /// Engine impacts landed (including overkill on fallen sections).
    pub shots: u32,
    /// Wall sections at zero hp.
    pub breaches: u32,
    /// Tip checkpoint hash per shard, in shard order.
    pub shard_tips: [u64; SIEGE_SHARDS],
    /// Per-tracked-viewer wire bytes/s at the snapshot cadence.
    pub interest_bps: [u64; INTEREST_VIEWERS],
    /// Worst ticks-since-sent on any tracked viewer.
    pub starve_max: u32,
}

impl core::fmt::Display for SiegeReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SIEGE seed={:016x} ticks={} hash={:016x} shots={} breaches={} starve_max={}",
            self.seed, self.ticks, self.hash, self.shots, self.breaches, self.starve_max,
        )
    }
}

/// Global slot for a shard plus local slot pair.
#[must_use]
pub const fn global_slot(shard: usize, local: u8) -> u8 {
    (shard as u8) * SHARD_FIGHTERS + local
}

/// Shard owning a global slot.
#[must_use]
pub const fn shard_of(global: u8) -> usize {
    (global / SHARD_FIGHTERS) as usize
}

/// Army of a global slot: 0 attacks (shards 0–1), 1 defends (shards 2–3).
#[must_use]
pub const fn army_of(global: u8) -> u8 {
    if (global / SHARD_FIGHTERS) < 2 {
        0
    } else {
        1
    }
}

/// FNV-1a step over one whole `u64` word (xor, then prime multiply; not the
/// byte-wise fold of `Fnv64`). Integer ops only, stable across runs.
pub(crate) const fn fold_word(hash: u64, word: u64) -> u64 {
    (hash ^ word).wrapping_mul(FNV64_PRIME)
}

/// Fold one event into the siege hash as sequential words: discriminant,
/// tick plus section, then the payload. Integer ops only.
fn fold_event(hash: u64, event: &SiegeEvent) -> u64 {
    match *event {
        SiegeEvent::EngineFired {
            tick,
            engine,
            section,
            damage,
        } => {
            let hash = fold_word(hash, 0xF12E_0001);
            let hash = fold_word(hash, (u64::from(tick) << 16) | u64::from(section));
            fold_word(hash, (u64::from(engine) << 32) | u64::from(damage))
        }
        SiegeEvent::WallBreached { tick, section } => {
            let hash = fold_word(hash, 0xB2EA_C400);
            fold_word(hash, (u64::from(tick) << 16) | u64::from(section))
        }
    }
}

/// Deterministic fighter entity for the replication sample: formation base
/// per army plus an integer lattice drift that is a pure function of
/// `(seed, slot, tick)`. Positions never touch hashed fight state; the
/// combat word folds the shard tip so damage marks entities dirty and the
/// no-desync rule keeps them.
fn entity_for(seed: u64, global: u8, tick: u32, shard_tip: u64) -> InterestEntity {
    let rank = (global % 32) as u64;
    let side: i64 = if army_of(global) == 0 { -1 } else { 1 };
    let base_x = side * (20_000 + (rank % 8) as i64 * 3_000);
    let base_z = (rank / 8) as i64 * 8_000 - 12_000;
    let mix = (tick as u64)
        .wrapping_mul(2_654_435_761)
        .wrapping_add((global as u64).wrapping_mul(97_531))
        .wrapping_add(seed);
    let dx = (mix % 801) as i64 - 400;
    let dz = ((mix >> 11) % 801) as i64 - 400;
    InterestEntity {
        id: global as u32,
        x_mm: base_x.saturating_add(dx) as i32,
        z_mm: base_z.saturating_add(dz) as i32,
        combat: (shard_tip ^ ((tick as u64) << 1) ^ (global as u64)) as u32,
    }
}

/// Viewer yaw for the replication sample, in decidegrees.
const fn viewer_yaw_ddeg(global: u8) -> i16 {
    ((global as i32 * 450) % 3600 - 1800) as i16
}

/// Wall/bombardment ledger plus the siege hash: the siege-owned state both
/// drivers step. Integer only; the event log is pre-sized for the run so
/// the per-tick path never reallocates.
struct Ledger {
    engine_streams: [u64; SIEGE_ENGINES],
    engine_fires: [u32; SIEGE_ENGINES],
    walls: [u32; WALL_SECTIONS],
    events: Vec<SiegeEvent>,
    hash: u64,
    shots: u32,
}

impl Ledger {
    /// Open the ledger: per-engine bombardment streams offset from the
    /// master seed so neighbouring engines never share a section order.
    fn new(seed: u64, ticks: u32) -> Self {
        let streams = arena_seed::expand(seed ^ 0xB16B_00B5_1E5E_0001, SIEGE_ENGINES);
        let mut engine_streams = [0u64; SIEGE_ENGINES];
        engine_streams.copy_from_slice(&streams);
        Self {
            engine_streams,
            engine_fires: [0u32; SIEGE_ENGINES],
            walls: [WALL_SECTION_HP; WALL_SECTIONS],
            events: Vec::with_capacity((ticks as usize) / 20 + 32),
            hash: FNV64_OFFSET,
            shots: 0,
        }
    }

    /// Step one tick: due engines fire in engine order, then the hash fold
    /// over the tick, the shard tips and the wall ledger.
    fn step(&mut self, tick: u32, tips: &[Checkpoint; SIEGE_SHARDS]) -> (u8, u8) {
        let mut fired = 0u8;
        let mut breached = 0u8;
        for engine in 0..SIEGE_ENGINES {
            if !(tick + (engine as u32) * 37).is_multiple_of(ENGINE_EVERY_TICKS) {
                continue;
            }
            let word = arena_seed::next(&mut self.engine_streams[engine]);
            let section = (word % (WALL_SECTIONS as u64)) as u8;
            let before = self.walls[section as usize];
            let damage = if before == 0 {
                0
            } else {
                ENGINE_DAMAGE.min(before)
            };
            self.walls[section as usize] = before.saturating_sub(ENGINE_DAMAGE);
            self.engine_fires[engine] += 1;
            self.shots += 1;
            fired += 1;
            self.events.push(SiegeEvent::EngineFired {
                tick,
                engine: engine as u8,
                section,
                damage,
            });
            if before != 0 && self.walls[section as usize] == 0 {
                breached += 1;
                self.events.push(SiegeEvent::WallBreached { tick, section });
            }
        }
        let mut hash = fold_word(self.hash, u64::from(tick));
        for tip in tips {
            hash = fold_word(hash, tip.hash);
        }
        for hp in &self.walls {
            hash = fold_word(hash, u64::from(*hp));
        }
        for event in self.events.iter().filter(|event| event.tick() == tick) {
            hash = fold_event(hash, event);
        }
        self.hash = hash;
        (fired, breached)
    }
}

/// Replication sample over the interest path: one connection per
/// tracked viewer with caller-held buffers, so selects never allocate.
struct InterestSample {
    cfg: InterestCfg,
    conns: Vec<ConnectionInterest>,
    out: [u32; SIEGE_SLOTS],
}

impl InterestSample {
    /// Open the sample and register the tick-zero baseline (zero tips, the
    /// same baseline the dedicated path registers before its first tick).
    fn new(seed: u64) -> Result<Self, SiegeError> {
        let cfg = InterestCfg::default_64();
        cfg.validate()?;
        let mut sample = Self {
            cfg,
            conns: Vec::with_capacity(INTEREST_VIEWERS),
            out: [u32::MAX; SIEGE_SLOTS],
        };
        let entities = sample.entities_for(seed, 0, &[0u64; SIEGE_SHARDS]);
        for viewer in 0..INTEREST_VIEWERS {
            let global = (viewer * (SIEGE_SLOTS / INTEREST_VIEWERS)) as u32;
            let mut conn = ConnectionInterest::for_entities(global, SIEGE_SLOTS)?;
            conn.register_baseline(&entities)?;
            sample.conns.push(conn);
        }
        Ok(sample)
    }

    /// Current 64 fighter entities for a tick and its shard tip hashes.
    fn entities_for(
        &self,
        seed: u64,
        tick: u32,
        tips: &[u64; SIEGE_SHARDS],
    ) -> [InterestEntity; SIEGE_SLOTS] {
        let mut entities = [InterestEntity {
            id: 0,
            x_mm: 0,
            z_mm: 0,
            combat: 0,
        }; SIEGE_SLOTS];
        for (global, entity) in entities.iter_mut().enumerate() {
            let shard = global / (SHARD_FIGHTERS as usize);
            *entity = entity_for(seed, global as u8, tick, tips[shard]);
        }
        entities
    }

    /// One replication sample per tracked viewer.
    fn sample(
        &mut self,
        seed: u64,
        tick: u32,
        tips: &[u64; SIEGE_SHARDS],
    ) -> Result<(), SiegeError> {
        let entities = self.entities_for(seed, tick, tips);
        let cfg = self.cfg;
        let out = &mut self.out;
        for conn in self.conns.iter_mut() {
            let viewer = conn.viewer();
            let viewer_pos = entities
                .iter()
                .find(|entity| entity.id == viewer)
                .copied()
                .unwrap_or(entities[0]);
            conn.select(
                &cfg,
                &entities,
                viewer,
                viewer_pos.x_mm,
                viewer_pos.z_mm,
                viewer_yaw_ddeg(viewer as u8),
                tick,
                SIEGE_BUDGET_BYTES,
                out,
            )?;
        }
        Ok(())
    }

    /// Per-viewer bytes/s plus the worst starve gap.
    fn report(&self, ticks: u32) -> ([u64; INTEREST_VIEWERS], u32) {
        let mut bps = [0u64; INTEREST_VIEWERS];
        for (index, conn) in self.conns.iter().enumerate() {
            bps[index] = conn.bytes_per_sec(ticks);
        }
        let starve = self
            .conns
            .iter()
            .map(|conn| conn.starve_max())
            .max()
            .unwrap_or(0);
        (bps, starve)
    }
}

/// Bot-filled 64-slot siege on a dedicated server: four authoritative
/// arena shards plus the wall/bombardment ledger and the siege hash.
///
/// Slots are sticky per shard through the dedicated server: human tokens stage
/// with [`SiegeServer::submit`], drops AI-fill until reclaimed, and a
/// client mismatch resyncs from the shard snapshot. Ticks author shards
/// in fixed order, so the same config replays bit-identical.
pub struct SiegeServer {
    config: SiegeConfig,
    shards: Vec<HeadlessServer>,
    ledger: Ledger,
    interest: InterestSample,
    cursor: u32,
}

impl SiegeServer {
    /// Open a siege: validates the config, authors four empty (AI-filled)
    /// shard matches and registers the replication sample baselines.
    pub fn new(config: SiegeConfig) -> Result<Self, SiegeError> {
        config.validate()?;
        let _ = siege_field()?;
        let mut shards = Vec::with_capacity(SIEGE_SHARDS);
        for shard in 0..SIEGE_SHARDS {
            shards.push(HeadlessServer::new(config.shard_config(shard))?);
        }
        Ok(Self {
            config,
            shards,
            ledger: Ledger::new(config.seed, config.ticks),
            interest: InterestSample::new(config.seed)?,
            cursor: 0,
        })
    }

    /// Siege configuration.
    #[must_use]
    pub const fn config(&self) -> SiegeConfig {
        self.config
    }

    /// Next tick to author.
    #[must_use]
    pub const fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Wall hp per section.
    #[must_use]
    pub const fn wall_hp(&self) -> [u32; WALL_SECTIONS] {
        self.ledger.walls
    }

    /// Bombardment/wall events in `(tick, engine)` order.
    #[must_use]
    pub fn events(&self) -> &[SiegeEvent] {
        &self.ledger.events
    }

    /// Tip checkpoint per shard, in shard order.
    #[must_use]
    pub fn tips(&self) -> [Option<Checkpoint>; SIEGE_SHARDS] {
        let mut tips = [None; SIEGE_SHARDS];
        for (index, shard) in self.shards.iter().enumerate() {
            tips[index] = shard.checkpoints().last().copied();
        }
        tips
    }

    /// Engine impacts landed so far.
    #[must_use]
    pub const fn shots(&self) -> u32 {
        self.ledger.shots
    }

    /// Wall sections at zero hp.
    #[must_use]
    pub fn breaches(&self) -> u32 {
        self.ledger.walls.iter().filter(|hp| **hp == 0).count() as u32
    }

    /// Connect a human token to a shard slot (sticky; reclaim after drop).
    pub fn connect(&mut self, shard: usize, token: u64) -> Result<u8, SiegeError> {
        let server = self
            .shards
            .get_mut(shard)
            .ok_or(SiegeError::BadConfig("shard"))?;
        Ok(server.connect(token)?)
    }

    /// Mark a human token dropped; its slot AI-fills until reclaimed.
    pub fn disconnect(&mut self, shard: usize, token: u64) -> bool {
        self.shards
            .get_mut(shard)
            .is_some_and(|server| server.disconnect(token))
    }

    /// Stage one human input for the tick being authored on its shard.
    pub fn submit(
        &mut self,
        shard: usize,
        token: u64,
        input: FighterInput,
    ) -> Result<(), SiegeError> {
        let server = self
            .shards
            .get_mut(shard)
            .ok_or(SiegeError::BadConfig("shard"))?;
        Ok(server.submit(token, input)?)
    }

    /// Compare a client hash against shard authority (resync on mismatch).
    pub fn verify(&self, shard: usize, token: u64, tick: u32, hash: u64) -> Result<(), SiegeError> {
        let server = self
            .shards
            .get(shard)
            .ok_or(SiegeError::BadConfig("shard"))?;
        Ok(server.verify(token, tick, hash)?)
    }

    /// Author one fixed siege tick: every shard in order (AI fill covers
    /// empty and dropped slots), then the shared ledger, then the
    /// replication sample at the snapshot cadence.
    pub fn tick(&mut self) -> Result<SiegeTick, SiegeError> {
        if self.cursor >= self.config.ticks {
            return Err(SiegeError::MatchOver);
        }
        let tick = self.cursor;
        let mut tips = [Checkpoint { tick: 0, hash: 0 }; SIEGE_SHARDS];
        for (index, shard) in self.shards.iter_mut().enumerate() {
            let (checkpoint, _) = shard.tick()?;
            tips[index] = checkpoint;
        }
        let (fired, breached) = self.ledger.step(tick, &tips);
        if tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
            self.sample_tick(tick, &tips)?;
        }
        self.cursor += 1;
        Ok(SiegeTick {
            tick,
            tips,
            fired,
            breached,
        })
    }

    /// Sample the replication path for authored tips.
    fn sample_tick(
        &mut self,
        tick: u32,
        tips: &[Checkpoint; SIEGE_SHARDS],
    ) -> Result<(), SiegeError> {
        let hashes = [tips[0].hash, tips[1].hash, tips[2].hash, tips[3].hash];
        self.interest.sample(self.config.seed, tick, &hashes)
    }

    /// Drive the siege to its configured end and report.
    pub fn run(mut self) -> Result<SiegeReport, SiegeError> {
        while self.cursor < self.config.ticks {
            self.tick()?;
        }
        Ok(self.report())
    }

    /// Completed-siege report for the ticks authored so far.
    #[must_use]
    pub fn report(&self) -> SiegeReport {
        let mut shard_tips = [0u64; SIEGE_SHARDS];
        for (index, shard) in self.shards.iter().enumerate() {
            shard_tips[index] = shard.checkpoints().last().map_or(0, |tip| tip.hash);
        }
        let (interest_bps, starve_max) = self.interest.report(self.cursor);
        SiegeReport {
            seed: self.config.seed,
            ticks: self.cursor,
            hash: self.ledger.hash,
            walls: self.ledger.walls,
            shots: self.ledger.shots,
            breaches: self.breaches(),
            shard_tips,
            interest_bps,
            starve_max,
        }
    }
}

/// Play a whole siege and report: the acceptance entry — a bot-filled
/// 64-slot siege for `config.ticks` ticks that replays exactly (same
/// config twice gives the same report). Each shard evaluates once as an
/// all-AI arena run and the shared ledger replays over the checkpoint
/// streams; the cross-path test pins this equal to the dedicated
/// tick-by-tick authoring.
pub fn run_siege(config: SiegeConfig) -> Result<SiegeReport, SiegeError> {
    config.validate()?;
    let _ = siege_field()?;
    let mut streams = Vec::with_capacity(SIEGE_SHARDS);
    for shard in 0..SIEGE_SHARDS {
        let checkpoints = arena_run(&config.shard_config(shard), &FighterTape::default());
        if checkpoints.len() != config.ticks as usize {
            return Err(SiegeError::SimRejected);
        }
        streams.push(checkpoints);
    }
    let mut ledger = Ledger::new(config.seed, config.ticks);
    let mut interest = InterestSample::new(config.seed)?;
    for tick in 0..config.ticks {
        let tips = [
            streams[0][tick as usize],
            streams[1][tick as usize],
            streams[2][tick as usize],
            streams[3][tick as usize],
        ];
        ledger.step(tick, &tips);
        if tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
            let hashes = [tips[0].hash, tips[1].hash, tips[2].hash, tips[3].hash];
            interest.sample(config.seed, tick, &hashes)?;
        }
    }
    let mut shard_tips = [0u64; SIEGE_SHARDS];
    for (index, stream) in streams.iter().enumerate() {
        shard_tips[index] = stream.last().map_or(0, |tip| tip.hash);
    }
    let (interest_bps, starve_max) = interest.report(config.ticks);
    let breaches = ledger.walls.iter().filter(|hp| **hp == 0).count() as u32;
    Ok(SiegeReport {
        seed: config.seed,
        ticks: config.ticks,
        hash: ledger.hash,
        walls: ledger.walls,
        shots: ledger.shots,
        breaches,
        shard_tips,
        interest_bps,
        starve_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::TICK_HZ;
    use engine_core::alloc_probe;
    use engine_input::{Buttons, FighterId};
    use engine_net::interest::{ON3_CAP_BPS, STARVE_TICKS};

    fn config() -> SiegeConfig {
        SiegeConfig {
            seed: 2040,
            ticks: 192,
            half_extent_m: 20.0,
        }
    }

    fn small_config() -> SiegeConfig {
        SiegeConfig {
            seed: 7,
            ticks: 48,
            half_extent_m: 12.0,
        }
    }

    #[test]
    fn slot_topology_covers_64_without_reopening_the_cap() {
        assert_eq!(SIEGE_SHARDS * (SHARD_FIGHTERS as usize), SIEGE_SLOTS);
        assert_eq!(SIEGE_SLOTS, 64);
        for global in 0..SIEGE_SLOTS as u8 {
            let shard = shard_of(global);
            let local = global % SHARD_FIGHTERS;
            assert_eq!(global_slot(shard, local), global);
            assert!(shard < SIEGE_SHARDS);
        }
        // Shards 0-1 attack, 2-3 defend, 32 slots per army.
        assert!((0..32).all(|slot| army_of(slot) == 0));
        assert!((32..64).all(|slot| army_of(slot) == 1));
        // Ten minutes at the arena rate.
        assert_eq!(SIEGE_TICKS_10MIN, 10 * 60 * TICK_HZ);
    }

    #[test]
    fn rejects_bad_configs() {
        assert!(SiegeServer::new(config()).is_ok());
        assert!(run_siege(config()).is_ok());
        for bad in [
            SiegeConfig {
                ticks: 0,
                ..config()
            },
            SiegeConfig {
                ticks: SIEGE_MAX_TICKS + 1,
                ..config()
            },
            SiegeConfig {
                half_extent_m: f32::NAN,
                ..config()
            },
            SiegeConfig {
                half_extent_m: 4.9,
                ..config()
            },
            SiegeConfig {
                half_extent_m: 101.0,
                ..config()
            },
        ] {
            assert!(
                matches!(SiegeServer::new(bad), Err(SiegeError::BadConfig(_))),
                "server accepted: {bad:?}"
            );
            assert!(
                matches!(run_siege(bad), Err(SiegeError::BadConfig(_))),
                "soak accepted: {bad:?}"
            );
        }
        assert!(matches!(
            SiegeServer::new(config()).unwrap().connect(9, 1),
            Err(SiegeError::BadConfig("shard"))
        ));
    }

    #[test]
    fn shards_draw_non_aliasing_streams() {
        // Neighbouring shards must fight different battles: identical tips
        // across shards would prove the seed expansion aliased.
        let report = run_siege(small_config()).unwrap();
        let mut hashes = report.shard_tips.to_vec();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), SIEGE_SHARDS, "shard streams aliased");
        // The all-AI legions move: tips are not one repeated hash.
        assert_ne!(report.shard_tips[0], 0);
    }

    #[test]
    fn dedicated_ticks_match_the_fast_soak() {
        // The same config through the dedicated tick-by-tick authoring and the
        // single-evaluation soak gives the same report, so the 10-minute
        // soak is exactly what the dedicated server authors.
        let dedicated = SiegeServer::new(small_config()).unwrap().run().unwrap();
        let fast = run_siege(small_config()).unwrap();
        assert_eq!(dedicated, fast);
    }

    #[test]
    fn bot_siege_replays_exactly() {
        let first = run_siege(config()).unwrap();
        let second = run_siege(config()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.ticks, 192);
        assert!(first.shots > 0, "engines must fire over 192 ticks");
        // Every tracked viewer holds the interest bars.
        for bps in first.interest_bps {
            assert!(
                bps <= u64::from(ON3_CAP_BPS),
                "interest over budget: {bps} B/s"
            );
        }
        assert!(
            first.starve_max <= STARVE_TICKS,
            "starve guard fired: {}",
            first.starve_max
        );
    }

    #[test]
    fn siege_field_probe_matches_placement_and_boots_192() {
        let field = siege_field().unwrap();
        assert_eq!(field.actors, SIEGE_FIELD_ACTORS);
        assert_eq!(field.gate_x, SIEGE_FIELD_GATE_X);
        assert_eq!(field.ballista_x, SIEGE_FIELD_BALLISTA_X);
        assert!(field.gate_reachable(20.0));
        assert!(!field.gate_reachable(5.0));
        assert!(siege_field_from_json("{\"actors\": []}").is_err());
        assert_eq!(SIEGE_SHARDS * usize::from(SHARD_FIGHTERS), SIEGE_SLOTS);
        let cfg = SiegeConfig {
            seed: 2040,
            ticks: 192,
            half_extent_m: 20.0,
        };
        assert!(SiegeServer::new(cfg).is_ok());
    }

    #[test]
    fn wall_ledger_falls_monotonically_with_sorted_events() {
        let mut server = SiegeServer::new(small_config()).unwrap();
        let mut last_walls = server.wall_hp();
        let mut last_tick = 0u32;
        let mut seen_tick = false;
        while server.cursor() < small_config().ticks {
            let outcome = server.tick().unwrap();
            let walls = server.wall_hp();
            for (before, after) in last_walls.iter().zip(walls.iter()) {
                assert!(*after <= *before, "wall hp rose without repair");
            }
            last_walls = walls;
            // Events for this tick arrive in engine order after any earlier
            // tick, so the log stays `(tick, engine)` sorted.
            let mut last_engine = 0u8;
            for event in server
                .events()
                .iter()
                .filter(|event| event.tick() == outcome.tick)
            {
                if seen_tick && event.tick() == last_tick {
                    if let SiegeEvent::EngineFired { engine, .. } = *event {
                        assert!(engine >= last_engine, "impacts out of engine order");
                        last_engine = engine;
                    }
                } else {
                    assert!(
                        !seen_tick || event.tick() > last_tick,
                        "events moved backwards"
                    );
                    last_tick = event.tick();
                    last_engine = 0;
                    seen_tick = true;
                }
            }
            let fired = server
                .events()
                .iter()
                .filter(|event| {
                    event.tick() == outcome.tick && matches!(event, SiegeEvent::EngineFired { .. })
                })
                .count();
            assert_eq!(fired as u8, outcome.fired);
        }
        assert!(server.shots() > 0, "engines must fire over 48 ticks");
    }

    #[test]
    fn bombardment_breaks_the_same_sections_twice() {
        let siege = SiegeConfig {
            seed: 99,
            ticks: 7_200,
            half_extent_m: 20.0,
        };
        let first = run_siege(siege).unwrap();
        let second = run_siege(siege).unwrap();
        assert_eq!(first, second);
        // A two-minute bombardment scars the walls identically twice;
        // full breaches belong to the 10-minute acceptance soak below.
        let dealt: u64 = first
            .walls
            .iter()
            .map(|hp| u64::from(WALL_SECTION_HP - hp))
            .sum();
        assert!(dealt > 0, "bombardment must damage walls");
        assert!(dealt <= u64::from(first.shots) * u64::from(ENGINE_DAMAGE));
    }

    #[test]
    fn humans_hold_slots_while_legions_ai_fill() {
        let mut server = SiegeServer::new(small_config()).unwrap();
        // Two humans join shard 0; every other slot AI-fills.
        let token_a = 0xA11CEu64;
        let token_b = 0xB0B1Eu64;
        assert_eq!(server.connect(0, token_a).unwrap(), 0);
        assert_eq!(server.connect(0, token_b).unwrap(), 1);
        for tick in 0..8u32 {
            for (token, slot) in [(token_a, 0u8), (token_b, 1u8)] {
                server
                    .submit(
                        0,
                        token,
                        FighterInput {
                            tick,
                            fighter: FighterId::new(slot).unwrap(),
                            mx: 100,
                            mz: 0,
                            facing: 0,
                            buttons: Buttons::NONE,
                        },
                    )
                    .unwrap();
            }
            assert!(server.tick().unwrap().tips[0].tick == tick);
        }
        // Authority verifies clean; a tampered hash names its desync.
        let tip = server.tips()[0].unwrap();
        server.verify(0, token_a, tip.tick, tip.hash).unwrap();
        assert!(matches!(
            server.verify(0, token_a, tip.tick, tip.hash ^ 1),
            Err(SiegeError::Server(ServerError::Desync { .. }))
        ));
        // Unknown tokens go stale without touching authority.
        assert!(matches!(
            server.submit(
                0,
                0xDEAD,
                FighterInput {
                    tick: server.tips()[0].unwrap().tick,
                    fighter: FighterId::new(0).unwrap(),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                }
            ),
            Err(SiegeError::Server(ServerError::StaleClient { .. }))
        ));
        // Drop and reclaim: the slot AI-fills meanwhile, the token returns.
        assert!(server.disconnect(0, token_b));
        assert_eq!(server.connect(0, token_b).unwrap(), 1);
    }

    #[test]
    fn ticking_past_the_end_fails_without_advancing() {
        let cfg = SiegeConfig {
            ticks: 4,
            ..small_config()
        };
        let mut server = SiegeServer::new(cfg).unwrap();
        for _ in 0..4 {
            server.tick().unwrap();
        }
        assert_eq!(server.cursor(), 4);
        assert_eq!(server.tick(), Err(SiegeError::MatchOver));
        assert_eq!(server.cursor(), 4);
    }

    #[test]
    fn siege_owned_step_allocates_nothing_at_steady_state() {
        // Calibration: the shared probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        // Warm up past every siege-owned buffer (events, interest ledgers).
        let mut ledger = Ledger::new(small_config().seed, small_config().ticks);
        let mut interest = InterestSample::new(small_config().seed).unwrap();
        let tips = [Checkpoint {
            tick: 0,
            hash: 0x1234,
        }; SIEGE_SHARDS];
        for tick in 0..small_config().ticks {
            ledger.step(tick, &tips);
            if tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
                let hashes = [tips[0].hash, tips[1].hash, tips[2].hash, tips[3].hash];
                interest.sample(small_config().seed, tick, &hashes).unwrap();
            }
        }
        let cap_before = ledger.events.capacity();
        let events_before = ledger.events.len();

        // Steady state: the ledger step plus the interest sample perform
        // zero heap allocations inside the window.
        let ((), steady) = alloc_probe::measure(|| {
            for tick in small_config().ticks..small_config().ticks + 8 {
                ledger.step(tick, &tips);
            }
            let hashes = [tips[0].hash, tips[1].hash, tips[2].hash, tips[3].hash];
            interest
                .sample(small_config().seed, small_config().ticks, &hashes)
                .unwrap();
            std::hint::black_box(ledger.hash);
        });
        assert!(steady.calls == 0, "siege-owned step allocated");
        // The window did real work: the ledger moved and no buffer regrew.
        assert!(ledger.events.len() >= events_before);
        assert_eq!(ledger.events.capacity(), cap_before);
    }

    #[test]
    fn ten_minute_bot_siege_replays_exactly() {
        let full = SiegeConfig {
            seed: 2_040_326,
            ticks: SIEGE_TICKS_10MIN,
            half_extent_m: 40.0,
        };
        let first = run_siege(full).unwrap();
        assert_eq!(first.ticks, SIEGE_TICKS_10MIN);
        assert_eq!(first.breaches as usize, WALL_SECTIONS);
        for bps in first.interest_bps {
            assert!(
                bps <= u64::from(ON3_CAP_BPS),
                "interest over budget: {bps} B/s"
            );
        }
        assert!(first.starve_max <= STARVE_TICKS);
        let second = run_siege(full).unwrap();
        assert_eq!(first, second, "10-minute siege must replay exactly");
        println!("{first}");
    }
}
