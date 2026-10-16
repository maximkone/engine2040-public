//! modes_world: arena, siege and district free-roam over the ladder.
//!
//! Consumer: the lobby screen plus the slice builder, which map lobby
//! start params into [`LobbyBoot`] and drive [`ModeSession`]. Data owner: this
//! module owns the mode enum, the lobby boot params, the district cell window
//! and the session wrapper; match truth stays in `engine-arena` (via [`App`]),
//! siege truth in [`crate::siege`], ladder truth in [`crate::ladder`]. Units:
//! fixed ticks at the arena rate, meters for extents, cell units for the
//! district window. Determinism: integer-only folds, stable input order, no
//! wall clock; same [`LobbyBoot`] plus same inputs gives the same digest.
//! Errors: [`WorldModeError`] at every boundary; replays mismatch loudly.
//!
//! District cells are the canonical world [`CellCoord`] (consumed from
//! `engine-world`, never mirrored); the 3x3 live window below is mode
//! logic over those coords. Modes never edit the
//! engine: they only construct validated configs and call public tick/run
//! entry points; the determinism test boots twice and pins equal digests.
//! Budget: boot is O(1) time and memory; tick cost is owned by arena/siege;
//! [`ModeSession::digest`] is O(ticks) integer folds with no allocation.

use crate::ladder::{Ladder, LadderError, LadderRecord, Outcome};
use crate::ludus_npcs::{ludus_contains, ludus_route, ludus_zone};
use crate::siege::{fold_word, run_siege, SiegeConfig, SiegeError, SiegeReport, SIEGE_SLOTS};
use crate::{App, GameConfig, GameError};
use engine_anim::traversal::{Probe, TraversalPlan, TraverseAction};
use engine_arena::Checkpoint;
use engine_core::hash::FNV64_OFFSET;
use engine_input::{FighterInput, MAX_PARTICIPANTS};
use engine_world::cells::{CellCoord, MAX_COORD};

/// District cell window: the live 3x3 around the roam center.
pub const DISTRICT_WINDOW: usize = 9;
/// District cell side in texels (inside the world `4..=64` bake range).
pub const DISTRICT_CELL_SIZE: u16 = 32;
/// Ticks a bot-filled siege test may run without slowing the gate.
pub const SIEGE_SMOKE_TICKS: u32 = 200;

/// District crossing TV1 length: links 1-3 in `content/maps/district/route_tv1.json`.
pub const DISTRICT_ROUTE_LEN: usize = 3;

/// Plan one TV1 link by id (`1..=3`) through the traversal chooser.
/// Caller: [`district_route_plans`] plus district tests. Units: meters.
pub fn district_plan_link(id: u8) -> Result<TraversalPlan, WorldModeError> {
    let (probe, edge_m, want) = match id {
        1 => (
            Probe {
                height_m: 0.95,
                depth_m: 0.6,
                gap_m: 0.4,
                speed_m_s: 2.0,
                has_hold: false,
            },
            [-27.5, -2.0],
            TraverseAction::Vault,
        ),
        2 => (
            Probe {
                height_m: 1.40,
                depth_m: 1.0,
                gap_m: 0.3,
                speed_m_s: 1.2,
                has_hold: false,
            },
            [-26.0, 0.9],
            TraverseAction::Mantle,
        ),
        3 => (
            Probe {
                height_m: 2.40,
                depth_m: 0.6,
                gap_m: 0.2,
                speed_m_s: 0.8,
                has_hold: true,
            },
            [-2.0, 0.3],
            TraverseAction::Climb,
        ),
        _ => return Err(WorldModeError::BadConfig("route_link")),
    };
    let planned = engine_anim::traversal::plan(&probe, edge_m)?;
    if planned.action != want {
        return Err(WorldModeError::ReplayMismatch("district"));
    }
    Ok(planned)
}

/// Plan the three TV1 links in route order and pin the TV1 bounds.
/// Caller: `ModeSession::boot` plus `ModeSession::replay` (District).
pub fn district_route_plans() -> Result<[TraversalPlan; DISTRICT_ROUTE_LEN], WorldModeError> {
    if engine_anim::traversal::VAULT_MAX_MM != 1250 {
        return Err(WorldModeError::BadConfig("vault_max"));
    }
    if engine_anim::traversal::MANTLE_MAX_MM != 1700 {
        return Err(WorldModeError::BadConfig("mantle_max"));
    }
    if engine_anim::traversal::CLIMB_MAX_MM != 2600 {
        return Err(WorldModeError::BadConfig("climb_max"));
    }
    Ok([
        district_plan_link(1)?,
        district_plan_link(2)?,
        district_plan_link(3)?,
    ])
}

/// Playable mode, selected in the lobby. One byte on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WorldMode {
    /// One human vs AI brains in a single arena match.
    Arena,
    /// Bot-filled 64-slot siege over four arena shards.
    Siege,
    /// Free-roam over a 3x3 district cell window with ladder practice.
    District,
    /// Ludus hub practice: drill floor plus pells plus rack over the ladder.
    Ludus,
}

impl WorldMode {
    /// Decode `0` (arena), `1` (siege), `2` (district) or `3` (ludus); else fails.
    pub fn decode(byte: u8) -> Result<Self, WorldModeError> {
        match byte {
            0 => Ok(Self::Arena),
            1 => Ok(Self::Siege),
            2 => Ok(Self::District),
            3 => Ok(Self::Ludus),
            other => Err(WorldModeError::BadMode(other)),
        }
    }

    /// Encode to the byte [`WorldMode::decode`] reads. Pure.
    #[must_use]
    pub fn encode(self) -> u8 {
        match self {
            Self::Arena => 0,
            Self::Siege => 1,
            Self::District => 2,
            Self::Ludus => 3,
        }
    }
}

/// Lobby-supplied boot params for one mode session. The lobby screen
/// and the slice builder fill this from the lobby start (seed, slots,
/// mode choice); [`ModeSession::boot`] validates it per mode. Plain data.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LobbyBoot {
    /// Mode selected in the lobby.
    pub mode: WorldMode,
    /// Master seed; expanded per shard/stream, never aliased.
    pub seed: u64,
    /// Fixed steps, `1..=100_000` (the arena/siege replay bound).
    pub ticks: u32,
    /// Fighters requested: `2..=16` for arena/district, exactly 64 for siege.
    pub fighters: u8,
    /// Arena half-extent in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
}

impl LobbyBoot {
    /// Pack lobby start params as plain data; validation happens in
    /// [`ModeSession::boot`] so the lobby screen gets one typed error. Pure.
    #[must_use]
    pub fn new(mode: WorldMode, seed: u64, ticks: u32, fighters: u8, half_extent_m: f32) -> Self {
        Self {
            mode,
            seed,
            ticks,
            fighters,
            half_extent_m,
        }
    }
}

/// Live 3x3 window around `center` in stable (x-major, then z) order over
/// the canonical [`CellCoord`]. In-range by construction:
/// [`DistrictWalk::new`] pins the center one cell inside the rim, and
/// [`DistrictWalk::walk`] re-pins every step, so every offset stays in
/// bounds. Pure mode logic; validation stays in `CellCoord::new`.
#[must_use]
pub fn cell_window(center: CellCoord) -> [CellCoord; DISTRICT_WINDOW] {
    let mut out = [CellCoord { x: 0, z: 0 }; DISTRICT_WINDOW];
    let mut index = 0;
    for dx in -1..=1 {
        for dz in -1..=1 {
            out[index] = cell_moved(center, dx, dz);
            index += 1;
        }
    }
    out
}

/// Offset a coord by one king-move step without re-validating (callers hold
/// the rim pin invariant documented on [`cell_window`]).
fn cell_moved(cell: CellCoord, dx: i32, dz: i32) -> CellCoord {
    CellCoord {
        x: cell.x + dx,
        z: cell.z + dz,
    }
}

/// Walker over the district window: the roam center plus its live 3x3. The
/// walker waits at the rim visibly — steps off the map or past one cell fail
/// loudly instead of guessing, mirroring the world `NotLive` contract.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DistrictWalk {
    /// Roam center cell; always one cell inside the map rim.
    center: CellCoord,
    /// Accepted single-cell steps so far.
    steps: u32,
}

impl DistrictWalk {
    /// Start roaming at `center`: the center must sit one cell inside the rim
    /// so the live window never leaves the map. Pure.
    pub fn new(center: CellCoord) -> Result<Self, WorldModeError> {
        let rim = MAX_COORD - 1;
        if !(-rim..=rim).contains(&center.x) || !(-rim..=rim).contains(&center.z) {
            return Err(WorldModeError::BadCoord);
        }
        Ok(Self { center, steps: 0 })
    }

    /// Roam center cell.
    #[must_use]
    pub fn center(self) -> CellCoord {
        self.center
    }

    /// Accepted steps so far.
    #[must_use]
    pub fn steps(self) -> u32 {
        self.steps
    }

    /// Live 3x3 window around the center, in stable order.
    #[must_use]
    pub fn live(self) -> [CellCoord; DISTRICT_WINDOW] {
        cell_window(self.center)
    }

    /// Step the center one cell king-move (`dx, dz` in `-1..=1`, not both
    /// zero). Longer strides are `NotLive` (the walker waits at the edge);
    /// steps off the pinned rim are `BadCoord`. Pure.
    pub fn walk(&mut self, dx: i32, dz: i32) -> Result<(), WorldModeError> {
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dz) || (dx == 0 && dz == 0) {
            return Err(WorldModeError::NotLive);
        }
        // Re-pin: the rim invariant must hold after the step, or the window
        // would leave the map. The counter survives the re-pin.
        let pinned = Self::new(cell_moved(self.center, dx, dz))?;
        self.center = pinned.center;
        self.steps = self.steps.saturating_add(1);
        Ok(())
    }
}

/// Walker over the ludus practice yard in meters. Steps off the yard or
/// past one cell fail loudly. Caller: [`ModeSession`] ludus arms. Pure.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LudusWalk {
    /// East position in meters, inside the practice yard.
    x: f32,
    /// South position in meters, inside the practice yard.
    z: f32,
    /// Accepted single-cell steps so far.
    steps: u32,
}

impl LudusWalk {
    /// Start at `(x, z)` meters inside the practice yard. Pure.
    pub fn new(x: f32, z: f32) -> Result<Self, WorldModeError> {
        let yard = ludus_zone("practice_yard").ok_or(WorldModeError::BadConfig("ludus_zone"))?;
        if !ludus_contains(yard, x, z) {
            return Err(WorldModeError::BadCoord);
        }
        Ok(Self { x, z, steps: 0 })
    }

    /// Drill-floor position in meters.
    #[must_use]
    pub fn pos(self) -> (f32, f32) {
        (self.x, self.z)
    }

    /// Accepted steps so far.
    #[must_use]
    pub fn steps(self) -> u32 {
        self.steps
    }

    /// Step one cell (`dx, dz` in `-1..=1`, not both zero). Pure.
    pub fn walk(&mut self, dx: i32, dz: i32) -> Result<(), WorldModeError> {
        let step_x = match dx {
            -1 => -1.0,
            0 => 0.0,
            1 => 1.0,
            _ => return Err(WorldModeError::NotLive),
        };
        let step_z = match dz {
            -1 => -1.0,
            0 => 0.0,
            1 => 1.0,
            _ => return Err(WorldModeError::NotLive),
        };
        if dx == 0 && dz == 0 {
            return Err(WorldModeError::NotLive);
        }
        let next_x = self.x + step_x;
        let next_z = self.z + step_z;
        let yard = ludus_zone("practice_yard").ok_or(WorldModeError::BadConfig("ludus_zone"))?;
        if !ludus_contains(yard, next_x, next_z) {
            return Err(WorldModeError::BadCoord);
        }
        self.x = next_x;
        self.z = next_z;
        self.steps = self.steps.saturating_add(1);
        Ok(())
    }
}

/// One arena/district match tape: the [`App`] plus its recorded human inputs
/// so the session rebuilds identically on replay. Opaque: only
/// [`ModeSession`] builds these; all fields stay private.
pub struct MatchTape {
    config: GameConfig,
    app: App,
    inputs: Vec<FighterInput>,
}

impl MatchTape {
    fn boot(config: GameConfig) -> Result<Self, WorldModeError> {
        Ok(Self {
            config,
            app: App::new(config).map_err(WorldModeError::Game)?,
            inputs: Vec::new(),
        })
    }

    fn tick(&mut self, input: FighterInput) -> Result<bool, WorldModeError> {
        let mut stamped = input;
        stamped.tick = self.app.cursor();
        let accepted = self.app.tick(stamped).input_accepted;
        if accepted {
            self.inputs.push(stamped);
        }
        Ok(accepted)
    }

    fn digest(&self) -> u64 {
        let mut hash = FNV64_OFFSET;
        for checkpoint in self.app.checkpoints() {
            hash = fold_word(hash, u64::from(checkpoint.tick));
            hash = fold_word(hash, checkpoint.hash);
        }
        hash
    }

    fn replay(&self) -> Result<u64, WorldModeError> {
        let mut app = App::new(self.config).map_err(WorldModeError::Game)?;
        for input in &self.inputs {
            if !app.tick(*input).input_accepted {
                return Err(WorldModeError::ReplayMismatch("arena"));
            }
        }
        if app.checkpoints() != self.app.checkpoints() || app.hp() != self.app.hp() {
            return Err(WorldModeError::ReplayMismatch("arena"));
        }
        Ok(Self {
            config: self.config,
            app,
            inputs: Vec::new(),
        }
        .digest())
    }
}

/// Mode failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldModeError {
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Mode byte outside `0..=3`, byte carried.
    BadMode(u8),
    /// Cell coordinate past the map rim.
    BadCoord,
    /// Walk stride is not a single king-move step.
    NotLive,
    /// Arena session failure from the packaged match.
    Game(GameError),
    /// Siege failure from the dedicated siege path.
    Siege(SiegeError),
    /// Ladder snapshot failure from the ladder table.
    Ladder(LadderError),
    /// District traversal plan failure from the TV1 crossing.
    Traversal(engine_anim::traversal::TraverseError),
    /// Replay diverged; the static names the mode that moved.
    ReplayMismatch(&'static str),
}

impl core::fmt::Display for WorldModeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad world-mode config ({row})"),
            Self::BadMode(byte) => write!(f, "bad world mode byte ({byte})"),
            Self::BadCoord => write!(f, "district cell past the map rim"),
            Self::NotLive => write!(f, "district walk is not a single live step"),
            Self::Game(err) => write!(f, "arena session error ({err})"),
            Self::Siege(err) => write!(f, "siege error ({err})"),
            Self::Ladder(err) => write!(f, "ladder error ({err})"),
            Self::Traversal(err) => write!(f, "district traversal error ({err})"),
            Self::ReplayMismatch(mode) => write!(f, "replay diverged ({mode})"),
        }
    }
}

impl std::error::Error for WorldModeError {}

impl From<GameError> for WorldModeError {
    fn from(err: GameError) -> Self {
        Self::Game(err)
    }
}

impl From<SiegeError> for WorldModeError {
    fn from(err: SiegeError) -> Self {
        Self::Siege(err)
    }
}

impl From<LadderError> for WorldModeError {
    fn from(err: LadderError) -> Self {
        Self::Ladder(err)
    }
}

impl From<engine_anim::traversal::TraverseError> for WorldModeError {
    fn from(err: engine_anim::traversal::TraverseError) -> Self {
        Self::Traversal(err)
    }
}

/// One booted mode session over the ladder: arena, siege, district
/// free-roam or ludus hub practice. Boot validates the [`LobbyBoot`] per
/// mode; [`ModeSession::tick`] drives arena/district/ludus matches,
/// [`ModeSession::finish_siege`] authors the bot-filled siege, and
/// [`ModeSession::replay`] re-runs the recorded session and fails loudly.
pub enum ModeSession {
    /// Single arena match plus its recorded tape.
    Arena {
        /// Recorded match tape.
        tape: MatchTape,
        /// Ladder folded by [`ModeSession::settle`].
        ladder: Ladder,
    },
    /// Bot-filled siege: config in, report out, replayed via [`run_siege`].
    Siege {
        /// Validated siege config from the lobby boot.
        config: SiegeConfig,
        /// Completed report after [`ModeSession::finish_siege`].
        report: Option<SiegeReport>,
        /// Ladder folded by [`ModeSession::settle`].
        ladder: Ladder,
    },
    /// Free-roam: arena match plus the district cell window.
    District {
        /// Recorded match tape.
        tape: MatchTape,
        /// Roam center plus live 3x3 window.
        walk: DistrictWalk,
        /// Boot center derived from the seed; replay rebuilds from here.
        origin: CellCoord,
        /// Recorded single-cell steps in order; replay re-applies them.
        walks: Vec<(i8, i8)>,
        /// Ladder folded by [`ModeSession::settle`].
        ladder: Ladder,
    },
    /// Ludus hub practice: arena match plus the drill-floor walk.
    Ludus {
        /// Recorded match tape.
        tape: MatchTape,
        /// Drill-floor walk over the practice yard.
        walk: LudusWalk,
        /// Boot position (player spawn); replay rebuilds from here.
        origin: (f32, f32),
        /// Recorded single-cell steps in order; replay re-applies them.
        walks: Vec<(i32, i32)>,
        /// Ladder folded by [`ModeSession::settle`].
        ladder: Ladder,
    },
}

impl ModeSession {
    /// Boot one mode from the lobby: validates ticks/extent for every mode,
    /// fighters `2..=16` for arena/district/ludus and exactly 64 for siege.
    /// The district center derives deterministically from the seed. Modes only
    /// construct validated configs and call public entry points — they never
    /// mutate engine state outside their own session.
    pub fn boot(lobby: LobbyBoot) -> Result<Self, WorldModeError> {
        if lobby.ticks == 0 || lobby.ticks > crate::MAX_TICKS {
            return Err(WorldModeError::BadConfig("ticks"));
        }
        if !lobby.half_extent_m.is_finite()
            || lobby.half_extent_m < 5.0
            || lobby.half_extent_m > 100.0
        {
            return Err(WorldModeError::BadConfig("half_extent_m"));
        }
        match lobby.mode {
            WorldMode::Arena => {
                if lobby.fighters < 2 || lobby.fighters > MAX_PARTICIPANTS {
                    return Err(WorldModeError::BadConfig("fighters"));
                }
                Ok(Self::Arena {
                    tape: MatchTape::boot(GameConfig {
                        seed: lobby.seed,
                        ticks: lobby.ticks,
                        fighters: lobby.fighters,
                        half_extent_m: lobby.half_extent_m,
                    })?,
                    ladder: Ladder::new(),
                })
            }
            WorldMode::Siege => {
                if lobby.fighters as usize != SIEGE_SLOTS {
                    return Err(WorldModeError::BadConfig("fighters"));
                }
                let config = SiegeConfig {
                    seed: lobby.seed,
                    ticks: lobby.ticks,
                    half_extent_m: lobby.half_extent_m,
                };
                config.validate().map_err(WorldModeError::Siege)?;
                Ok(Self::Siege {
                    config,
                    report: None,
                    ladder: Ladder::new(),
                })
            }
            WorldMode::District => {
                if lobby.fighters < 2 || lobby.fighters > MAX_PARTICIPANTS {
                    return Err(WorldModeError::BadConfig("fighters"));
                }
                let config = GameConfig {
                    seed: lobby.seed,
                    ticks: lobby.ticks,
                    fighters: lobby.fighters,
                    half_extent_m: lobby.half_extent_m,
                };
                // Center is a pure function of the seed inside the pinned rim.
                let rim = MAX_COORD - 1;
                let span = (2 * rim + 1) as u64;
                let x = (lobby.seed % span) as i32 - rim;
                let z = ((lobby.seed / span) % span) as i32 - rim;
                let center = CellCoord::new(x, z).map_err(|_| WorldModeError::BadCoord)?;
                // The crossing is part of the boot: a bound move refuses here.
                district_route_plans()?;
                Ok(Self::District {
                    tape: MatchTape::boot(config)?,
                    walk: DistrictWalk::new(center)?,
                    origin: center,
                    walks: Vec::new(),
                    ladder: Ladder::new(),
                })
            }
            WorldMode::Ludus => {
                if lobby.fighters < 2 || lobby.fighters > MAX_PARTICIPANTS {
                    return Err(WorldModeError::BadConfig("fighters"));
                }
                let config = GameConfig {
                    seed: lobby.seed,
                    ticks: lobby.ticks,
                    fighters: lobby.fighters,
                    half_extent_m: lobby.half_extent_m,
                };
                // Drill routes are part of the boot: a missing route refuses here.
                ludus_route("pell_west_stand").ok_or(WorldModeError::BadConfig("ludus_route"))?;
                let spawn = crate::ludus_npcs::LUDUS_SPAWNS
                    .iter()
                    .find(|spot| spot.name == "player")
                    .ok_or(WorldModeError::BadConfig("ludus_spawn"))?;
                let walk = LudusWalk::new(spawn.x, spawn.z)?;
                Ok(Self::Ludus {
                    tape: MatchTape::boot(config)?,
                    walk,
                    origin: (spawn.x, spawn.z),
                    walks: Vec::new(),
                    ladder: Ladder::new(),
                })
            }
        }
    }

    /// Mode booted here.
    #[must_use]
    pub fn mode(&self) -> WorldMode {
        match self {
            Self::Arena { .. } => WorldMode::Arena,
            Self::Siege { .. } => WorldMode::Siege,
            Self::District { .. } => WorldMode::District,
            Self::Ludus { .. } => WorldMode::Ludus,
        }
    }

    /// Fixed-tick cursor for arena/district/ludus; authored siege ticks for siege.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        match self {
            Self::Arena { tape, .. } => tape.app.cursor(),
            Self::Siege { config, report, .. } => {
                if report.is_some() {
                    config.ticks
                } else {
                    0
                }
            }
            Self::District { tape, .. } => tape.app.cursor(),
            Self::Ludus { tape, .. } => tape.app.cursor(),
        }
    }

    /// Ladder folded by [`ModeSession::settle`].
    #[must_use]
    pub fn ladder(&self) -> &Ladder {
        match self {
            Self::Arena { ladder, .. } => ladder,
            Self::Siege { ladder, .. } => ladder,
            Self::District { ladder, .. } => ladder,
            Self::Ludus { ladder, .. } => ladder,
        }
    }

    /// Storable ladder snapshot beside the progression payload.
    #[must_use]
    pub fn ladder_record(&self) -> LadderRecord {
        self.ladder().record()
    }

    /// Fold one decided match into the ladder at the session cursor.
    pub fn settle(&mut self, outcome: Outcome) {
        let tick = self.cursor();
        let ladder = match self {
            Self::Arena { ladder, .. } => ladder,
            Self::Siege { ladder, .. } => ladder,
            Self::District { ladder, .. } => ladder,
            Self::Ludus { ladder, .. } => ladder,
        };
        ladder.apply(outcome, tick);
    }

    /// Fold one decided match using designer rows text at the progression
    /// boundary. Reads the header-version row from `text` (shipped as
    /// `content/design/loop.rows`) and folds via
    /// `Ladder::apply_rows_text`, returning the rows rank. A refusal keeps
    /// last-good and reports the 1-based line error. Cold boundary only,
    /// never per tick. Golden unchanged: the compiled `Ladder::apply`
    /// path is untouched.
    pub fn settle_with_rows(
        &mut self,
        outcome: Outcome,
        text: &str,
    ) -> Result<u8, crate::loop_text::LoopRowsError> {
        let tick = self.cursor();
        let ladder = match self {
            Self::Arena { ladder, .. } => ladder,
            Self::Siege { ladder, .. } => ladder,
            Self::District { ladder, .. } => ladder,
            Self::Ludus { ladder, .. } => ladder,
        };
        ladder.apply_rows_text(outcome, tick, text)
    }

    /// Advance one fixed tick on a human input (arena/district/ludus). True when
    /// the input was accepted and recorded for replay. Siege is bot-filled:
    /// ticking it fails loudly; drive it with [`ModeSession::finish_siege`].
    pub fn tick(&mut self, input: FighterInput) -> Result<bool, WorldModeError> {
        match self {
            Self::Arena { tape, .. } => tape.tick(input),
            Self::District { tape, .. } => tape.tick(input),
            Self::Ludus { tape, .. } => tape.tick(input),
            Self::Siege { .. } => Err(WorldModeError::BadConfig("tick")),
        }
    }

    /// Step one cell (district roam or ludus drill walk); arena and siege
    /// sessions refuse loudly. Accepted steps are recorded for replay.
    pub fn walk(&mut self, dx: i32, dz: i32) -> Result<(), WorldModeError> {
        match self {
            Self::District { walk, walks, .. } => {
                walk.walk(dx, dz)?;
                walks.push((dx as i8, dz as i8));
                Ok(())
            }
            Self::Ludus { walk, walks, .. } => {
                walk.walk(dx, dz)?;
                walks.push((dx, dz));
                Ok(())
            }
            _ => Err(WorldModeError::BadConfig("walk")),
        }
    }

    /// Accepted roam steps (district only).
    pub fn district_steps(&self) -> Result<u32, WorldModeError> {
        match self {
            Self::District { walk, .. } => Ok(walk.steps()),
            _ => Err(WorldModeError::BadConfig("walk")),
        }
    }

    /// District roam center (district only).
    pub fn district_center(&self) -> Result<CellCoord, WorldModeError> {
        match self {
            Self::District { walk, .. } => Ok(walk.center()),
            _ => Err(WorldModeError::BadConfig("walk")),
        }
    }

    /// District live 3x3 window in stable order (district only).
    pub fn district_live(&self) -> Result<[CellCoord; DISTRICT_WINDOW], WorldModeError> {
        match self {
            Self::District { walk, .. } => Ok(walk.live()),
            _ => Err(WorldModeError::BadConfig("walk")),
        }
    }

    /// Author the bot-filled siege to its configured end (siege only).
    pub fn finish_siege(&mut self) -> Result<SiegeReport, WorldModeError> {
        match self {
            Self::Siege { config, report, .. } => {
                let done = run_siege(*config).map_err(WorldModeError::Siege)?;
                *report = Some(done.clone());
                Ok(done)
            }
            _ => Err(WorldModeError::BadConfig("finish_siege")),
        }
    }

    /// Siege report after [`ModeSession::finish_siege`] (siege only).
    pub fn siege_report(&self) -> Result<&SiegeReport, WorldModeError> {
        match self {
            Self::Siege { report, .. } => report
                .as_ref()
                .ok_or(WorldModeError::BadConfig("finish_siege")),
            _ => Err(WorldModeError::BadConfig("finish_siege")),
        }
    }

    /// Arena checkpoints for every simulated tick (arena/district/ludus).
    pub fn checkpoints(&self) -> Result<&[Checkpoint], WorldModeError> {
        match self {
            Self::Arena { tape, .. } => Ok(tape.app.checkpoints()),
            Self::District { tape, .. } => Ok(tape.app.checkpoints()),
            Self::Ludus { tape, .. } => Ok(tape.app.checkpoints()),
            Self::Siege { .. } => Err(WorldModeError::BadConfig("checkpoints")),
        }
    }

    /// Converged digest of the recorded session: checkpoint folds for
    /// arena/district/ludus (plus the walk position and step count), the
    /// hash for a finished siege. Integer ops only, no allocation.
    #[must_use]
    pub fn digest(&self) -> u64 {
        match self {
            Self::Arena { tape, .. } => tape.digest(),
            Self::District { tape, walk, .. } => {
                let center = walk.center();
                let hash = fold_word(tape.digest(), center.x as u64);
                let hash = fold_word(hash, center.z as u64);
                fold_word(hash, walk.steps() as u64)
            }
            Self::Ludus { tape, walk, .. } => {
                let (x, z) = walk.pos();
                let hash = fold_word(tape.digest(), u64::from(x.to_bits()));
                let hash = fold_word(hash, u64::from(z.to_bits()));
                fold_word(hash, u64::from(walk.steps()))
            }
            Self::Siege { report, .. } => report.as_ref().map_or(0, |done| done.hash),
        }
    }

    /// Replay the recorded session from its lobby params and fail loudly on
    /// any divergence: arena/district/ludus re-run the recorded inputs
    /// through a fresh match, siege re-authors the bot-filled war. Returns
    /// the replay digest, which must equal [`ModeSession::digest`].
    pub fn replay(&self) -> Result<u64, WorldModeError> {
        match self {
            Self::Arena { tape, .. } => {
                let again = tape.replay()?;
                if again != tape.digest() {
                    return Err(WorldModeError::ReplayMismatch("arena"));
                }
                Ok(again)
            }
            Self::District {
                tape,
                walk,
                origin,
                walks,
                ..
            } => {
                let again = tape.replay()?;
                // Re-apply the recorded steps from the boot origin: a dropped
                // or reordered step diverges here instead of replaying stale.
                let mut rewound = DistrictWalk::new(*origin)?;
                for (dx, dz) in walks.iter() {
                    rewound
                        .walk(i32::from(*dx), i32::from(*dz))
                        .map_err(|_| WorldModeError::ReplayMismatch("district"))?;
                }
                if rewound != *walk {
                    return Err(WorldModeError::ReplayMismatch("district"));
                }
                // The crossing replays too: a bound move diverges here.
                district_route_plans()?;
                let center = walk.center();
                let hash = fold_word(again, center.x as u64);
                let hash = fold_word(hash, center.z as u64);
                let hash = fold_word(hash, walk.steps() as u64);
                if hash != self.digest() {
                    return Err(WorldModeError::ReplayMismatch("district"));
                }
                Ok(hash)
            }
            Self::Ludus {
                tape,
                walk,
                origin,
                walks,
                ..
            } => {
                let again = tape.replay()?;
                let mut rewound = LudusWalk::new(origin.0, origin.1)?;
                for (dx, dz) in walks.iter() {
                    rewound
                        .walk(*dx, *dz)
                        .map_err(|_| WorldModeError::ReplayMismatch("ludus"))?;
                }
                if rewound != *walk {
                    return Err(WorldModeError::ReplayMismatch("ludus"));
                }
                ludus_route("pell_west_stand").ok_or(WorldModeError::ReplayMismatch("ludus"))?;
                let (x, z) = walk.pos();
                let hash = fold_word(again, u64::from(x.to_bits()));
                let hash = fold_word(hash, u64::from(z.to_bits()));
                let hash = fold_word(hash, u64::from(walk.steps()));
                if hash != self.digest() {
                    return Err(WorldModeError::ReplayMismatch("ludus"));
                }
                Ok(hash)
            }
            Self::Siege { config, report, .. } => {
                let done = report
                    .as_ref()
                    .ok_or(WorldModeError::BadConfig("finish_siege"))?;
                let again = run_siege(*config).map_err(WorldModeError::Siege)?;
                if again != *done {
                    return Err(WorldModeError::ReplayMismatch("siege"));
                }
                Ok(again.hash)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::{Buttons, FighterId};

    fn human(tick: u32, mx: i8, mz: i8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(crate::HUMAN_SLOT).expect("slot 0 names a fighter"),
            mx,
            mz,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    fn script(tick: u32) -> FighterInput {
        let phase = (tick / 15) % 4;
        let (mx, mz) = match phase {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        human(tick, mx, mz)
    }

    fn arena_boot() -> LobbyBoot {
        LobbyBoot::new(WorldMode::Arena, 2040, 64, 2, 20.0)
    }

    fn district_boot() -> LobbyBoot {
        LobbyBoot::new(WorldMode::District, 9182, 64, 2, 24.0)
    }

    fn ludus_boot() -> LobbyBoot {
        LobbyBoot::new(WorldMode::Ludus, 20260925, 64, 2, 20.0)
    }

    fn siege_boot() -> LobbyBoot {
        LobbyBoot::new(WorldMode::Siege, 777, SIEGE_SMOKE_TICKS, 64, 20.0)
    }

    fn drive(session: &mut ModeSession, ticks: u32) {
        for tick in 0..ticks {
            assert!(
                session.tick(script(tick)).expect("script inputs validate"),
                "script tick accepted"
            );
        }
    }

    #[test]
    fn arena_boots_from_lobby_and_replays() {
        let mut session = ModeSession::boot(arena_boot()).expect("valid lobby boot");
        assert_eq!(session.mode(), WorldMode::Arena);
        drive(&mut session, 64);
        assert_eq!(session.cursor(), 64);
        assert_eq!(session.checkpoints().expect("arena tips").len(), 64);
        let digest = session.digest();
        // Same boot twice is the same session: modes never edit the engine.
        let mut twin = ModeSession::boot(arena_boot()).expect("valid lobby boot");
        drive(&mut twin, 64);
        assert_eq!(twin.digest(), digest);
        assert_eq!(session.replay().expect("arena replays"), digest);
    }

    #[test]
    fn siege_boots_from_lobby_and_replays() {
        let mut session = ModeSession::boot(siege_boot()).expect("valid siege boot");
        assert_eq!(session.mode(), WorldMode::Siege);
        assert_eq!(session.cursor(), 0);
        let report = session.finish_siege().expect("siege authors");
        assert_eq!(session.cursor(), SIEGE_SMOKE_TICKS);
        assert!(report.shots > 0, "engines must fire on the smoke run");
        assert_eq!(session.digest(), report.hash);
        assert_eq!(session.replay().expect("siege replays"), report.hash);
        // Finishing twice converges: the ledger is a pure function of config.
        let again = session.finish_siege().expect("siege re-authors");
        assert_eq!(again, report);
    }

    #[test]
    fn district_boots_roams_and_replays() {
        let mut session = ModeSession::boot(district_boot()).expect("valid district boot");
        assert_eq!(session.mode(), WorldMode::District);
        // TV1 crossing: links 1-3 plan vault/mantle/climb against the bounds.
        let plans = district_route_plans().expect("TV1 route plans");
        assert_eq!(
            plans[0].action,
            TraverseAction::Vault,
            "link 1 vaults 0.95 m"
        );
        assert_eq!(
            plans[1].action,
            TraverseAction::Mantle,
            "link 2 mantles 1.40 m"
        );
        assert_eq!(
            plans[2].action,
            TraverseAction::Climb,
            "link 3 climbs 2.40 m hold"
        );
        assert_eq!(engine_anim::traversal::VAULT_MAX_MM, 1250);
        assert_eq!(engine_anim::traversal::MANTLE_MAX_MM, 1700);
        assert_eq!(engine_anim::traversal::CLIMB_MAX_MM, 2600);
        drive(&mut session, 64);
        session.walk(1, 0).expect("step east");
        session.walk(0, 1).expect("step north");
        session.walk(-1, 0).expect("step west");
        let center = session.district_center().expect("district center");
        assert_eq!(session.district_live().expect("live window").len(), 9);
        assert!(
            session
                .district_live()
                .expect("live window")
                .contains(&center),
            "window always contains its center"
        );
        let digest = session.digest();
        assert_eq!(session.replay().expect("district replays"), digest);
        // The window is a stable sort: reversed input cannot reorder it.
        let mut live = session.district_live().expect("live window");
        live.reverse();
        live.sort_unstable();
        assert_eq!(live, session.district_live().expect("live window"));
        assert_eq!(session.district_steps().expect("roam steps"), 3);
        // District cells match the world bake range the orchestrator will bind.
        assert!((4..=64).contains(&DISTRICT_CELL_SIZE));
    }

    #[test]
    fn district_route_refuses_bad_links() {
        assert!(district_plan_link(0).is_err(), "unknown link must refuse");
        assert!(district_plan_link(4).is_err(), "link past TV1 must refuse");
        let holdless = Probe {
            height_m: 2.40,
            depth_m: 0.6,
            gap_m: 0.2,
            speed_m_s: 0.8,
            has_hold: false,
        };
        assert_eq!(
            engine_anim::traversal::plan(&holdless, [-2.0, 0.3]),
            Err(engine_anim::traversal::TraverseError::NoHold),
            "holdless climb must refuse"
        );
    }

    #[test]
    fn lobby_rejects_bad_boots() {
        for bad in [
            LobbyBoot::new(WorldMode::Arena, 1, 0, 2, 20.0),
            LobbyBoot::new(WorldMode::Arena, 1, 64, 1, 20.0),
            LobbyBoot::new(WorldMode::Arena, 1, 64, 17, 20.0),
            LobbyBoot::new(WorldMode::Arena, 1, 64, 2, f32::NAN),
            LobbyBoot::new(WorldMode::Arena, 1, 64, 2, 4.9),
            LobbyBoot::new(WorldMode::Siege, 1, SIEGE_SMOKE_TICKS, 8, 20.0),
            LobbyBoot::new(WorldMode::District, 1, 64, 1, 20.0),
        ] {
            assert!(
                ModeSession::boot(bad).is_err(),
                "lobby boot accepted: {bad:?}"
            );
        }
        assert_eq!(
            WorldMode::decode(99),
            Err(WorldModeError::BadMode(99)),
            "unknown mode byte must fail loudly"
        );
        for mode in [
            WorldMode::Arena,
            WorldMode::Siege,
            WorldMode::District,
            WorldMode::Ludus,
        ] {
            assert_eq!(WorldMode::decode(mode.encode()), Ok(mode));
        }
    }

    #[test]
    fn wrong_mode_calls_refuse_loudly() {
        let mut arena = ModeSession::boot(arena_boot()).expect("arena boots");
        assert!(arena.walk(1, 0).is_err(), "arena has no roam window");
        assert!(arena.finish_siege().is_err(), "arena cannot author a siege");
        assert!(arena.siege_report().is_err(), "arena holds no siege report");
        let mut siege = ModeSession::boot(siege_boot()).expect("siege boots");
        assert!(
            siege.tick(script(0)).is_err(),
            "bot-filled siege takes no human ticks"
        );
        assert!(
            siege.checkpoints().is_err(),
            "siege tips live in the report"
        );
        assert!(siege.replay().is_err(), "unfinished siege cannot replay");
        assert!(siege.district_center().is_err(), "siege has no roam center");
    }

    #[test]
    fn district_refuses_steps_off_the_live_window() {
        let mut session = ModeSession::boot(district_boot()).expect("district boots");
        // Two-cell strides are not live steps: the walker waits, never leaps.
        assert_eq!(
            session.walk(2, 0),
            Err(WorldModeError::NotLive),
            "two-cell stride must refuse"
        );
        assert_eq!(
            session.walk(0, 0),
            Err(WorldModeError::NotLive),
            "zero stride must refuse"
        );
        assert_eq!(session.district_steps().expect("roam steps"), 0);
        // Rim pin: past the rim cannot name a cell; on the rim a cell names
        // fine but cannot roam, and walking keeps the pin.
        assert_eq!(
            CellCoord::new(MAX_COORD + 1, 0).map_err(|_| WorldModeError::BadCoord),
            Err(WorldModeError::BadCoord)
        );
        let rim = CellCoord::new(MAX_COORD, 0).expect("rim names a cell");
        assert!(
            DistrictWalk::new(rim).is_err(),
            "rim center leaves the window off-map"
        );
        let edge = CellCoord::new(MAX_COORD - 1, 0).expect("pinned rim boots");
        let mut walk = DistrictWalk::new(edge).expect("pinned rim boots");
        assert_eq!(
            walk.walk(1, 0),
            Err(WorldModeError::BadCoord),
            "stepping off the map must refuse"
        );
        assert_eq!(walk.center(), edge, "refused step keeps the center");
    }

    #[test]
    fn ludus_boots_drills_and_replays() {
        let mut session = ModeSession::boot(ludus_boot()).expect("ludus boots from lobby");
        assert_eq!(session.mode(), WorldMode::Ludus);
        assert_eq!(WorldMode::decode(3), Ok(WorldMode::Ludus));
        assert_eq!(WorldMode::Ludus.encode(), 3);
        // Player spawn -48,0 yaw 90 on the drill floor inside the nav rect.
        match session {
            ModeSession::Ludus { walk, origin, .. } => {
                assert_eq!(walk.pos(), (-48.0, 0.0));
                assert_eq!(origin, (-48.0, 0.0));
            }
            _ => panic!("ludus boots"),
        }
        drive(&mut session, 64);
        // Drill route to pell_west_stand -8,-2: 40 east plus 2 south.
        for _ in 0..40 {
            session.walk(1, 0).expect("step east toward the pell");
        }
        for _ in 0..2 {
            session.walk(0, -1).expect("step south to the stand");
        }
        match session {
            ModeSession::Ludus { walk, .. } => assert_eq!(walk.pos(), (-8.0, -2.0)),
            _ => panic!("ludus boots"),
        }
        assert_eq!(session.walk(2, 0), Err(WorldModeError::NotLive));
        assert_eq!(session.walk(0, 0), Err(WorldModeError::NotLive));
        let digest = session.digest();
        assert_eq!(session.replay().expect("ludus replays"), digest);
        let mut twin = ModeSession::boot(ludus_boot()).expect("ludus boots");
        drive(&mut twin, 64);
        for _ in 0..40 {
            twin.walk(1, 0).expect("twin east");
        }
        for _ in 0..2 {
            twin.walk(0, -1).expect("twin south");
        }
        assert_eq!(twin.digest(), digest);
        session.settle(Outcome::Win);
        assert_eq!(session.ladder().wins, 1);
    }

    #[test]
    fn ladder_folds_over_every_mode() {
        let mut arena = ModeSession::boot(arena_boot()).expect("arena boots");
        drive(&mut arena, 8);
        for _ in 0..8 {
            arena.settle(Outcome::Win);
        }
        // Eight wins from rung zero complete one top-rung loop and arm repair.
        assert_eq!(arena.ladder().loops, 1);
        assert_eq!(arena.ladder().pending_repairs(), 1);
        let record = arena.ladder_record();
        let mut restored = Ladder::restore(&record).expect("snapshot restores");
        assert_eq!(restored.record(), record);
        assert!(restored.take_repair(), "one repair owed per loop");
        assert!(!restored.take_repair(), "repairs never double-fire");
        // Losses retreat without touching the loop count.
        arena.settle(Outcome::Loss);
        assert_eq!((arena.ladder().wins, arena.ladder().losses), (8, 1));
        assert_eq!(arena.ladder().loops, 1);

        let mut district = ModeSession::boot(district_boot()).expect("district boots");
        district.settle(Outcome::Win);
        assert_eq!(district.ladder().rung, 1);
        assert_eq!(
            Ladder::rank_name(district.ladder().rank()),
            "Tiro",
            "one win stays at the first rank floor"
        );

        let mut siege = ModeSession::boot(siege_boot()).expect("siege boots");
        siege.finish_siege().expect("siege authors");
        siege.settle(Outcome::Win);
        assert_eq!(siege.ladder().wins, 1);
        assert_eq!(siege.ladder().last_tick, SIEGE_SMOKE_TICKS);
    }

    #[test]
    fn settle_with_rows_lands_shipped_and_refuses_bad() {
        let text = include_str!("../../../content/design/loop.rows");
        let mut session = ModeSession::boot(arena_boot()).expect("arena boots");
        drive(&mut session, 8);
        let rank = session
            .settle_with_rows(Outcome::Win, text)
            .expect("shipped rows settle");
        assert_eq!(session.ladder().wins, 1);
        assert_eq!(rank, session.ladder().rank());
        let mut compiled = ModeSession::boot(arena_boot()).expect("arena boots");
        drive(&mut compiled, 8);
        compiled.settle(Outcome::Win);
        assert_eq!(session.ladder(), compiled.ladder());
        let before = session.ladder_record();
        assert!(session
            .settle_with_rows(Outcome::Win, "table loop version 0\nv0 glory_per_win=1")
            .is_err());
        assert_eq!(
            session.ladder_record(),
            before,
            "refused rows keep last-good"
        );
    }
}
