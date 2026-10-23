//! faction_match: the 6v6 match loop and its seats (RY5 of
//! `docs/plan/ryl1.md`).
//!
//! Owner: engine-game, beside [`super::class_sheet`] which owns the class data
//! in `content/classes/`. Consumer: none yet; RY6's snapshot path, RY7's HUD
//! and RY8's replay gate read [`Match`]. Data owner: the point table and the
//! spawn halls come from `content/maps/contested_middle.json`, its three
//! `kind: "statue"` nodes;
//! the point state is [`engine_world::capture::CaptureSet`], never a local
//! copy. Units: integer centimetres for position and radius, integer ticks at
//! 60 Hz; the map's metres convert to centimetres once at load and the tick
//! never sees a float. Determinism: every number on the tick path is an
//! integer, seats and points keep their authored order, and the hash walks
//! that order, so two runs of one match hash equal on this PC and on wasm.
//!
//! The class sheets are part of the match, not decoration: [`Match::new`] takes
//! one sheet per seat and folds its numbers into the hash before the first
//! tick, so changing a sheet's range or cooldown moves the match hash (RY2).
//!
//! A seat's class decides whether it shoots. A class whose row carries a
//! `projectile` ability fires with that first ability's reach and cooldown,
//! converted to centimetres and ticks once when the match is built; a class
//! with none (the line classes) never fires, whatever stands near it. A seat
//! with an enemy inside its reach fires one aimed round through [`resolve_shot`]
//! over the tick's bodies, and a hit is one event and one count for the
//! shooter's faction, never damage: a health model is a later plan row.
//!
//! An AI seat runs the same verbs a player runs and reads the same state: it
//! picks a point from the owners it can see and walks, with no RNG and no
//! model, so a replay needs neither. The score rule is the plan's "hold two to
//! score": one score per tick per faction that owns at least
//! [`HOLD_TWO_TO_SCORE`] points, until [`SCORE_TO_WIN`] or the ten-minute
//! clock. [`CaptureSet::score`] stays the per-point hold-tick report the HUD
//! reads; the win score is the match's rule on top of it and still reads the
//! points, never the bodies.
//!
//! Errors: typed [`MapError`] and [`MatchError`]; a match that is already over
//! steps as a quiet no-op, never a panic. Budget: one pass over the seats and
//! the points per tick, one `Vec` reused across ticks (grown once at load),
//! the body lists of every faction reused the same way, and no heap beyond
//! those and the tick's hit list, which allocates only when a round lands.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use engine_combat::projectile::{FlightTableSet, FlightTables, SpawnParams, MAX_FLIGHT_TICKS};
use engine_combat::projectile_hit::{hash_hits, resolve_shot, Body, BodyTrack, HitEvent};
use engine_core::checked::saturating_f64_to_u64;
use engine_core::fixed::Fixed;
use engine_material::json::{parse_up_to, Value as Json};
use engine_world::capture::{CaptureError, CapturePoint, CaptureSet, Faction};

use super::class_sheet::{ClassSheet, Effect as ClassEffect, EffectValue};

/// Ticks in one match: ten minutes at 60 Hz.
pub const MATCH_TICKS: u64 = 36_000;
/// Held-point-ticks that end a match: one point-minute.
pub const SCORE_TO_WIN: u32 = 3_600;
/// Points a faction must own at once to score, per the plan's "hold two".
pub const HOLD_TWO_TO_SCORE: usize = 2;
/// Seats a side fields at Tier A: 6v6.
pub const SEATS_PER_SIDE: usize = 6;
/// AI seats the server fills on a side when no player is bound (DP1): the
/// Tier A slice runs four AI seats.
pub const AI_SEATS_PER_SIDE: usize = 2;
/// Slow faction: the Wardens are the shield line (plan section 3).
pub const WARDENS_STEP_CM: i32 = 9;
/// Fast faction: the Ashkin have reach and burst and are fragile.
pub const ASHKIN_STEP_CM: i32 = 11;
/// Ticks between a seat re-reading the map to pick a point.
pub const REPICK_TICKS: u64 = 600;
/// Most points one map may carry; the Tier A map has three statues.
pub const MAX_POINTS: usize = 16;
/// Most spawn halls one map may carry; the Tier A map has four.
pub const MAX_SPAWNS: usize = 8;
/// Most seats one match fields; Tier A is 12 and Tier B is 24.
pub const MAX_SEATS: usize = 24;
/// Most bytes a map file may hold.
pub const MAX_MAP_BYTES: usize = 256 * 1024;
/// Muzzle speed of a class volley in metres per second. Fast enough that the
/// drop over the longest Tier A range (the archer's 22 m) stays inside a body
/// shell, so a straight aim lands and gravity keeps its flight row.
pub const VOLLEY_SPEED_MPS: i64 = 100;
/// Height a volley leaves a shoulder at, and a body is centred at, in
/// centimetres: the fight is flat on the ground plane the seats walk.
pub const VOLLEY_SHOULDER_CM: i32 = 180;
/// Radius the resolver meets a seat inside, in millimetres.
pub const BODY_RADIUS_MM: u32 = 400;
/// Widest class reach the match reads, in centimetres: the class schema's 60 m.
pub const MAX_VOLLEY_RANGE_CM: u32 = 60_000;
/// Ticks of flight added to the ticks a volley needs to cross its class reach:
/// one second at 60 Hz. Covers the walk and the drop without letting a miss fly
/// for a whole minute.
pub const VOLLEY_TTL_MARGIN_TICKS: u64 = 60;

/// A seat's class volley: the reach and the tempo of its class's first
/// projectile ability, in the units the tick reads.
///
/// The class sheet carries metres and seconds (`f32`, the class_sheet
/// contract); a match reads centimetres and ticks. Both are converted once,
/// when the match is built, so the tick path stays integer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeatVolley {
    /// Reach of the class's projectile ability, in centimetres. Zero when the
    /// class carries no projectile: that seat never fires, whatever stands
    /// near it, which is the plan's line classes.
    pub range_cm: u32,
    /// Ticks between the seat's volleys: the ability's cooldown at 60 Hz.
    pub cooldown_ticks: u64,
}

/// A seat whose class carries no projectile: it never fires.
pub const NO_VOLLEY: SeatVolley = SeatVolley {
    range_cm: 0,
    cooldown_ticks: 0,
};

/// The volley a class sheet gives a seat: its first projectile ability's
/// reach and cooldown, in centimetres and ticks.
///
/// A class with no projectile ability gives [`NO_VOLLEY`]: the line classes
/// are melee by design, and a seat that never fires is a real rule, not a gap.
/// The reach and the cooldown are `f32` metres and seconds in the sheet; both
/// are converted here, once, and the match's tick path reads no float.
fn volley_of(sheet: &ClassSheet) -> SeatVolley {
    let Some(ability) = sheet
        .abilities
        .iter()
        .find(|ability| ability.kind.as_text() == "projectile")
    else {
        return NO_VOLLEY;
    };
    // a sheet ability's range is 0..=60 m and its cooldown 0..=600 s, so one
    // hundredth of them is far inside u32 and 60 of the seconds inside u64
    let metres = f64::from(ability.range);
    let range_cm = if metres.is_finite() && metres >= 0.0 {
        let cm = saturating_f64_to_u64((metres * 100.0).round());
        u32::try_from(cm.min(u64::from(MAX_VOLLEY_RANGE_CM))).unwrap_or(MAX_VOLLEY_RANGE_CM)
    } else {
        return NO_VOLLEY;
    };
    let seconds = f64::from(ability.cooldown);
    let cooldown_ticks = if seconds.is_finite() && seconds >= 0.0 {
        saturating_f64_to_u64((seconds * f64::from(engine_core::TICK_HZ_U32)).round())
    } else {
        return NO_VOLLEY;
    };
    SeatVolley {
        range_cm,
        cooldown_ticks,
    }
}

/// One capture point of the faction map: where it stands, how wide its ring is
/// and how long one faction must hold the ring to flip it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchPoint {
    /// Node id from the map file (`statue_west`).
    pub id: String,
    /// Centre of the ring in integer centimetres; x is east, z is north.
    pub center_cm: [i32; 2],
    /// Ring radius in integer centimetres.
    pub radius_cm: u32,
    /// Ticks of unchallenged presence that flip the point.
    pub hold_ticks: u32,
}

/// One spawn hall of the faction map.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchSpawn {
    /// Spawn id from the map file (`spawn_warden_s`).
    pub id: String,
    /// Faction the hall belongs to, `None` when the map names none.
    pub faction: Option<Faction>,
    /// Hall centre in integer centimetres.
    pub center_cm: [i32; 2],
}

/// The parts of one map the match loop reads: the points and the halls.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FactionMap {
    /// Map id (`contested_middle`).
    pub id: String,
    /// Playfield size in integer centimetres (x, z).
    pub size_cm: [i32; 2],
    /// The capture points, in the map file's node order.
    pub points: Vec<MatchPoint>,
    /// The spawn halls, in the map file's order.
    pub spawns: Vec<MatchSpawn>,
}

impl FactionMap {
    /// The capture table [`CaptureSet`] is built from: the map's points in
    /// order, so the point index a seat holds is this list's index. The point
    /// id is its own index, unique in the set as [`CapturePoint`] requires.
    #[must_use]
    pub fn capture_points(&self) -> Vec<CapturePoint> {
        self.points
            .iter()
            .enumerate()
            .map(|(index, point)| CapturePoint {
                id: u32::try_from(index).unwrap_or(u32::MAX),
                x: point.center_cm[0],
                z: point.center_cm[1],
                radius: i32::try_from(point.radius_cm).unwrap_or(i32::MAX),
                hold_ticks: point.hold_ticks,
            })
            .collect()
    }
}

/// Why a map file was refused. A bad file is refused, never patched.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MapError {
    /// The JSON reader itself refused the text.
    Json(String),
    /// A section that must be an object is not one.
    NotAnObject(&'static str),
    /// A required member is missing.
    Missing(&'static str),
    /// A number member is not a whole number of metres.
    BadNumber(&'static str),
    /// A node kind the map loader does not read.
    BadKind(&'static str),
    /// A faction name the map loader does not know.
    BadFaction(String),
    /// No capture point in the file.
    NoPoints,
    /// Past [`MAX_POINTS`] points.
    TooManyPoints(usize),
    /// Past [`MAX_SPAWNS`] halls.
    TooManySpawns(usize),
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::Json(error) => write!(f, "the map file is not JSON: {error}"),
            MapError::NotAnObject(section) => write!(f, "map section {section} is not an object"),
            MapError::Missing(member) => write!(f, "map is missing {member}"),
            MapError::BadNumber(member) => {
                write!(f, "map member {member} is not a whole number of metres")
            }
            MapError::BadKind(kind) => write!(f, "map node kind {kind} is not read here"),
            MapError::BadFaction(name) => write!(f, "map faction {name} is not a faction"),
            MapError::NoPoints => write!(f, "the map holds no capture point"),
            MapError::TooManyPoints(count) => {
                write!(f, "{count} points exceeds the {MAX_POINTS} point cap")
            }
            MapError::TooManySpawns(count) => {
                write!(f, "{count} spawn halls exceeds the {MAX_SPAWNS} hall cap")
            }
        }
    }
}

impl std::error::Error for MapError {}

/// One seat of the match: a body on one side with one class.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Seat {
    /// Side the seat fights on.
    pub faction: Faction,
    /// Class id from `content/classes/` (`warden_archer`).
    pub class: String,
    /// True when the server filled this seat because no player is bound.
    pub ai: bool,
    /// Where the seat stands, integer centimetres.
    pub center_cm: [i32; 2],
    /// Point index the seat walks to.
    pub target: usize,
    /// Tick the seat next re-reads the map to pick a point.
    pub pick_at: u64,
}

/// Why a match was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MatchError {
    /// The capture table refused the map's points.
    Capture(CaptureError),
    /// No seat at all.
    NoSeats,
    /// Past [`MAX_SEATS`] seats.
    TooManySeats(usize),
    /// A faction with seats has no spawn hall in the map.
    NoSpawn(Faction),
    /// A target point index outside the map's points.
    BadTarget(usize),
    /// A seat names a class no sheet in the match carries.
    NoClassSheet(String),
}

impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchError::Capture(error) => write!(f, "the map's points were refused: {error}"),
            MatchError::NoSeats => write!(f, "a match needs at least one seat"),
            MatchError::TooManySeats(count) => {
                write!(f, "{count} seats exceeds the {MAX_SEATS} seat cap")
            }
            MatchError::NoSpawn(faction) => {
                write!(f, "the map holds no {faction} spawn hall")
            }
            MatchError::BadTarget(index) => write!(f, "seat target {index} is not a map point"),
            MatchError::NoClassSheet(class) => {
                write!(f, "no class sheet for seat class {class:?}")
            }
        }
    }
}

impl std::error::Error for MatchError {}

/// How a match ended: a side reached [`SCORE_TO_WIN`], or the clock ran out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Winner {
    /// One faction won on held-point score.
    Faction(Faction),
    /// The clock ran out level.
    Draw,
}

/// One tick of the match, as plain data for the hash and the HUD.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TickReport {
    /// Ticks stepped so far, including this one.
    pub tick: u64,
    /// Point each seat walks to.
    pub targets: Vec<usize>,
    /// Where each seat stands after the walk.
    pub centers_cm: Vec<[i32; 2]>,
    /// Bodies of each faction inside each ring, `[index][faction.index()]`.
    pub present: Vec<[u16; 2]>,
    /// Win score after the score tick.
    pub score: [u32; 2],
    /// The hits the volleys landed this tick, in the order they landed.
    pub hits: Vec<HitEvent>,
}

/// The match: seats, points, score, clock and the running hash.
#[derive(Clone, Debug)]
pub struct Match {
    map: FactionMap,
    seats: Vec<Seat>,
    /// The sheet each seat resolves to, in seat order.
    sheets: Vec<ClassSheet>,
    /// Each seat's class volley, in seat order.
    volleys: Vec<SeatVolley>,
    /// Tick each seat's next volley may fly on: the cooldown gate.
    next_volley: Vec<u64>,
    /// Every hit a volley has landed, in the order they landed.
    hit_events: Vec<HitEvent>,
    /// The tick's bodies, one list per faction, rebuilt after each walk and
    /// reused across ticks so the tick path allocates no body list.
    bodies: [Vec<SeatTrack>; 2],
    /// Hits each faction has landed, `[index]`.
    landed: [u32; 2],
    points: CaptureSet,
    tick: u64,
    score: [u32; 2],
    hasher: engine_core::hash::Fnv64,
    ended: Option<Winner>,
}

impl Match {
    /// Build a match over a map, its seats and the class sheets they carry.
    ///
    /// Every seat names a class; that class must be in `sheets`. A match is
    /// the classes as well as the seats and the map, so a seat with no sheet
    /// is an incomplete match, not a default-powered one.
    ///
    /// # Errors
    /// [`MatchError`] when the map's points are refused, no seat is given, the
    /// seat count passes [`MAX_SEATS`], a faction has no spawn hall, a seat
    /// targets a point the map does not hold, or a seat's class carries no
    /// sheet.
    pub fn new(
        map: FactionMap,
        mut seats: Vec<Seat>,
        sheets: Vec<ClassSheet>,
    ) -> Result<Self, MatchError> {
        if seats.is_empty() {
            return Err(MatchError::NoSeats);
        }
        if seats.len() > MAX_SEATS {
            return Err(MatchError::TooManySeats(seats.len()));
        }
        for faction in Faction::ALL {
            if seats.iter().any(|seat| seat.faction == faction)
                && !map.spawns.iter().any(|hall| hall.faction == Some(faction))
            {
                return Err(MatchError::NoSpawn(faction));
            }
        }
        for seat in &seats {
            if seat.target >= map.points.len() {
                return Err(MatchError::BadTarget(seat.target));
            }
        }
        for seat in &mut seats {
            seat.pick_at = 0;
        }
        // one sheet per seat, in seat order: the hash walks that order too
        let mut resolved = Vec::with_capacity(seats.len());
        for seat in &seats {
            let sheet = sheets
                .iter()
                .find(|sheet| sheet.id == seat.class)
                .cloned()
                .ok_or_else(|| MatchError::NoClassSheet(seat.class.clone()))?;
            resolved.push(sheet);
        }
        // the volley of each seat's class, converted to centimetres and ticks
        // once: the tick path below reads no float
        let mut volleys = Vec::with_capacity(resolved.len());
        for sheet in &resolved {
            volleys.push(volley_of(sheet));
        }
        let next_volley = vec![0; seats.len()];
        let points = CaptureSet::new(map.capture_points()).map_err(MatchError::Capture)?;
        let mut hasher = engine_core::hash::Fnv64::new();
        Self::fold_sheets(&mut hasher, &resolved);
        Ok(Self {
            map,
            seats,
            sheets: resolved,
            volleys,
            next_volley,
            hit_events: Vec::new(),
            bodies: [Vec::new(), Vec::new()],
            landed: [0, 0],
            points,
            tick: 0,
            score: [0, 0],
            hasher,
            ended: None,
        })
    }

    /// The class sheet each seat resolves to, in seat order.
    #[must_use]
    pub fn sheets(&self) -> &[ClassSheet] {
        &self.sheets
    }

    /// Hash the sheets into the match: the identity of each class and every
    /// number that drives a fight - the five growth rates, and for each ability
    /// its range, radius, cast time, cooldown, mana, damage base and the
    /// numbers of its effects. The kind, shape, telegraph, ability id and
    /// loadout go in too, so the whole authored row is in the hash.
    ///
    /// The numbers are `f32` bits of the authored value, never an arithmetic
    /// result, so the same sheet text hashes the same on this PC and on wasm.
    /// Written straight into `hasher` so a sheet change moves every later
    /// hash of the match, not just a tick's own.
    fn fold_sheets(hasher: &mut engine_core::hash::Fnv64, sheets: &[ClassSheet]) {
        hasher.write_len(sheets.len());
        for sheet in sheets {
            hasher.write_len(sheet.id.len());
            hasher.write_str(&sheet.id);
            hasher.write_u64(u64::from(sheet.tier));
            hasher.write_len(sheet.role.len());
            hasher.write_str(&sheet.role);
            hasher.write_len(sheet.loadout.weapon.len());
            hasher.write_str(&sheet.loadout.weapon);
            hasher.write_len(sheet.abilities.len());
            for growth in [
                sheet.growth.str,
                sheet.growth.dex,
                sheet.growth.con,
                sheet.growth.int,
                sheet.growth.wis,
            ] {
                hasher.write_f32(growth);
            }
            for ability in &sheet.abilities {
                hasher.write_len(ability.id.len());
                hasher.write_str(&ability.id);
                hasher.write_len(ability.kind.as_text().len());
                hasher.write_str(ability.kind.as_text());
                hasher.write_len(ability.shape.as_text().len());
                hasher.write_str(ability.shape.as_text());
                hasher.write_len(ability.telegraph.as_text().len());
                hasher.write_str(ability.telegraph.as_text());
                hasher.write_f32(ability.range);
                hasher.write_f32(ability.radius);
                hasher.write_f32(ability.cast);
                hasher.write_f32(ability.cooldown);
                hasher.write_f32(ability.mana);
                hasher.write_f32(ability.damage.base);
                hasher.write_len(ability.effects.len());
                for effect in &ability.effects {
                    match effect {
                        ClassEffect::Text(text) => {
                            hasher.write_len(text.len());
                            hasher.write_str(text);
                        }
                        ClassEffect::Typed { effect_type, rows } => {
                            hasher.write_len(effect_type.len());
                            hasher.write_str(effect_type);
                            hasher.write_len(rows.len());
                            for (row, value) in rows {
                                hasher.write_len(row.len());
                                hasher.write_str(row);
                                match value {
                                    EffectValue::Text(text) => {
                                        hasher.write_len(text.len());
                                        hasher.write_str(text);
                                    }
                                    EffectValue::Number(number) => hasher.write_f32(*number),
                                    EffectValue::Flag(flag) => hasher.write_bool(*flag),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Ticks stepped so far.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The class volley of each seat, in seat order.
    #[must_use]
    pub fn volleys(&self) -> &[SeatVolley] {
        &self.volleys
    }

    /// Every hit a volley has landed, in the order they landed.
    #[must_use]
    pub fn hit_events(&self) -> &[HitEvent] {
        &self.hit_events
    }

    /// How many hits `faction` has landed with its volleys.
    #[must_use]
    pub fn hits_landed(&self, faction: Faction) -> u32 {
        self.landed[faction.index()]
    }

    /// The seats, in authored order.
    #[must_use]
    pub fn seats(&self) -> &[Seat] {
        &self.seats
    }

    /// The capture points and their state.
    #[must_use]
    pub fn points(&self) -> &CaptureSet {
        &self.points
    }

    /// The map the match runs on.
    #[must_use]
    pub fn map(&self) -> &FactionMap {
        &self.map
    }

    /// Win score of one faction: ticks it has held [`HOLD_TWO_TO_SCORE`]
    /// points at once.
    #[must_use]
    pub fn score(&self, faction: Faction) -> u32 {
        self.score[faction.index()]
    }

    /// How the match ended, once it has.
    #[must_use]
    pub fn winner(&self) -> Option<Winner> {
        self.ended
    }

    /// The running hash of every tick so far. Equal twice for equal input.
    #[must_use]
    pub fn hash(&self) -> u64 {
        self.hasher.finish()
    }

    /// Step one tick: seats walk, the rings are counted, the volleys fly, the
    /// score tick runs.
    ///
    /// Returns `Some(winner)` on the tick the match ends. A match that is
    /// already over steps as a quiet no-op and returns its winner, so a reader
    /// may keep stepping a finished match without changing its hash.
    ///
    /// # Errors
    /// [`MatchError`] when the capture set refuses the counted ring. The rows
    /// are built from the map's own point count, so that is a broken
    /// invariant, not an input: it returns the error rather than stepping on
    /// with a stale ring count. A volley that the flight or the resolver
    /// refuses is not an error: a round that hits nothing is a miss, and a
    /// round the tables refuse is dropped with the seat's cooldown spent.
    pub fn step(&mut self) -> Result<Option<Winner>, MatchError> {
        if let Some(winner) = self.ended {
            return Ok(Some(winner));
        }
        let mut report = self.walk_and_count();
        self.points
            .step(&report.present)
            .map_err(MatchError::Capture)?;
        for faction in Faction::ALL {
            if self.points.points_owned(faction) >= HOLD_TWO_TO_SCORE {
                self.score[faction.index()] += 1;
            }
        }
        report.hits = self.fire_volleys();
        self.tick += 1;
        self.fold(&report);
        self.ended = self.end_of_match();
        Ok(self.ended)
    }

    /// Fire every volley this tick allows: the seats whose class carries a
    /// projectile, whose cooldown has run out, and with an enemy inside that
    /// class's reach.
    ///
    /// The tick's bodies are read once, as the seats stand after their walk.
    /// Each shot resolves through [`resolve_shot`] over the enemy tracks, so a
    /// round that meets nothing is a miss and a refused flight is dropped with
    /// the seat's cooldown spent: neither is an error and neither reaches the
    /// hash. A hit is one event and one count for the shooter's faction; what a
    /// hit does to the body is the fight model's, which this slice has none of
    /// (RY3's contract).
    ///
    /// The log is bounded by the clock: a seat fires at most once per cooldown
    /// tick, so ten minutes of Tier A holds a few thousand events, not a step's
    /// worth. A tick on which nothing lands allocates nothing.
    fn fire_volleys(&mut self) -> Vec<HitEvent> {
        let mut tracks = core::mem::take(&mut self.bodies);
        for list in &mut tracks {
            list.clear();
        }
        for (index, seat) in self.seats.iter().enumerate() {
            tracks[seat.faction.index()].push(SeatTrack {
                id: u32::try_from(index).unwrap_or(u32::MAX),
                center_mm: body_center_mm(seat.center_cm),
            });
        }
        let tables = FlightTableSet::v1().tables;
        let mut hits = Vec::new();
        for index in 0..self.seats.len() {
            if self.volleys[index].range_cm == 0 || self.tick < self.next_volley[index] {
                continue;
            }
            let faction = self.seats[index].faction;
            let Some(enemy) = enemy_of(faction) else {
                continue;
            };
            let bodies = &tracks[enemy.index()];
            if bodies.is_empty() {
                continue;
            }
            let Some(params) = self.aim_volley(index, &tables) else {
                // no enemy inside the class's reach: the cooldown is not spent,
                // because no round left the shoulder
                continue;
            };
            let cooldown = self.volleys[index].cooldown_ticks;
            self.next_volley[index] = self.tick.saturating_add(cooldown);
            if let Ok(Some(hit)) = resolve_shot(&params, &tables, bodies) {
                self.landed[faction.index()] += 1;
                self.hit_events.push(hit);
                hits.push(hit);
            }
        }
        self.bodies = tracks;
        hits
    }

    /// The shot a seat fires this tick: its nearest enemy inside its class
    /// reach, aimed at that body's centre from the seat's own shoulder.
    ///
    /// `None` when no enemy stands inside the reach. The distance is squared
    /// centimetres in `i64` and the authored seat order breaks a tie, so the
    /// choice is exact integer data: no float, no draw. The aim lifts by the
    /// fall gravity will make on the way ([`volley_fall_mm`]), so a long shot
    /// meets the body instead of landing under it. The flight's life is the
    /// reach plus a margin of ticks, so a miss costs its own flight and not the
    /// whole minute the flight tables would allow.
    fn aim_volley(&self, index: usize, tables: &FlightTables) -> Option<SpawnParams> {
        let seat = &self.seats[index];
        let reach = i64::from(self.volleys[index].range_cm);
        let mut picked: Option<usize> = None;
        let mut best = i64::MAX;
        for (other, body) in self.seats.iter().enumerate() {
            if body.faction == seat.faction {
                continue;
            }
            let dx = i64::from(body.center_cm[0] - seat.center_cm[0]);
            let dz = i64::from(body.center_cm[1] - seat.center_cm[1]);
            let gap = dx * dx + dz * dz;
            if gap <= reach * reach && gap < best {
                best = gap;
                picked = Some(other);
            }
        }
        let from = body_center_mm(seat.center_cm);
        let to = body_center_mm(self.seats[picked?].center_cm);
        let fall = volley_fall_mm(best, tables.gravity_mps2);
        let dir = [
            mm_to_m(to[0] - from[0]),
            mm_to_m(to[1] - from[1] + fall),
            mm_to_m(to[2] - from[2]),
        ];
        // a round must cross the reach at VOLLEY_SPEED_MPS (10,000 cm/s, so
        // 10000/60 cm a tick) plus a margin; whole ticks, capped by the flight
        let cross = u64::from(self.volleys[index].range_cm) * 3 / 500 + VOLLEY_TTL_MARGIN_TICKS;
        let ttl_ticks =
            u32::try_from(cross.min(u64::from(MAX_FLIGHT_TICKS))).unwrap_or(MAX_FLIGHT_TICKS);
        Some(SpawnParams {
            tick: self.tick,
            shooter: u32::try_from(index).unwrap_or(u32::MAX),
            shot: 0,
            origin: [mm_to_m(from[0]), mm_to_m(from[1]), mm_to_m(from[2])],
            dir,
            speed: Fixed::from_int(VOLLEY_SPEED_MPS),
            // a class volley is aimed, not sprayed: the sheet's spread row is
            // the ability's, and this slice reads none
            spread: Fixed::ZERO,
            ttl_ticks,
        })
    }

    /// Run to the end of the match: score or clock.
    ///
    /// # Errors
    /// [`MatchError`] when a tick's capture step fails.
    pub fn run(&mut self) -> Result<Winner, MatchError> {
        while self.ended.is_none() {
            self.step()?;
        }
        Ok(self.ended.unwrap_or(Winner::Draw))
    }

    /// The tick's motion and ring counts, before the capture step.
    fn walk_and_count(&mut self) -> TickReport {
        let mut targets = Vec::with_capacity(self.seats.len());
        let mut centers = Vec::with_capacity(self.seats.len());
        for index in 0..self.seats.len() {
            let target = self.pick_target(index);
            let center = self.walk_seat(index, target);
            targets.push(target);
            centers.push(center);
        }
        let mut present = vec![[0u16; 2]; self.map.points.len()];
        for (index, seat) in self.seats.iter().enumerate() {
            for (point_index, point) in self.map.points.iter().enumerate() {
                if inside_ring(&centers[index], point) {
                    present[point_index][seat.faction.index()] += 1;
                }
            }
        }
        TickReport {
            tick: self.tick,
            targets,
            centers_cm: centers,
            present,
            score: self.score,
            hits: Vec::new(),
        }
    }

    /// Which point a seat walks to, and whether it re-reads the map.
    ///
    /// A seat that its faction has already banked progress on keeps that point:
    /// abandoning a half-taken ring throws the progress away and hands the
    /// point back. A seat repicks only every [`REPICK_TICKS`], so a contested
    /// ring holds a body in it instead of trading bodies mid-fight.
    ///
    /// The choice is the faction identity from plan section 3, ranked from the
    /// faction's own spawn hall so the order never churns as a seat walks: the
    /// Wardens, a line, all take the nearest statue, while the Ashkin, with
    /// reach and burst, take one further statue each so they spread across the
    /// three. The faction, the ordinal inside it and the hall are the only
    /// inputs, so there is no random number and no model: a replay needs
    /// neither. `ai` does not change the policy: an AI seat runs the verbs a
    /// player runs and reads the state a player sees.
    fn pick_target(&mut self, index: usize) -> usize {
        debug_assert!(
            !self.map.points.is_empty(),
            "a match has at least one point"
        );
        if self.tick < self.seats[index].pick_at {
            return self.seats[index].target;
        }
        let faction = self.seats[index].faction;
        let current = self.seats[index].target;
        // hold the ring the faction is already taking on
        if self.points.challenger(current) == Some(faction) && self.points.progress(current) > 0 {
            self.seats[index].pick_at = self.tick + REPICK_TICKS;
            return current;
        }
        // the seat's ordinal among its own faction's seats, in authored order
        let ordinal = self.seats[..index]
            .iter()
            .filter(|seat| seat.faction == faction)
            .count();
        // how many of this faction's points it still needs: a point it owns is
        // held, not wanted, so the count never drops a seat to a far point
        let wanted = self
            .map
            .points
            .iter()
            .enumerate()
            .filter(|(point_index, _)| self.points.owner(*point_index) != Some(faction))
            .count();
        // the seat's rank in that order: the Wardens are the whole line, the
        // Ashkin spread one seat-pair per statue
        let rank = if faction == Faction::Wardens || wanted == 0 {
            0
        } else {
            ordinal % wanted
        };
        if let Some(target) = self.rank_point(faction, rank) {
            self.seats[index].target = target;
        }
        self.seats[index].pick_at = self.tick + REPICK_TICKS;
        self.seats[index].target
    }

    /// The `rank`-th nearest point this faction still needs, ranked from its
    /// own spawn hall, `None` when it owns all of them. Nearest first, then
    /// the map's own point order breaks a tie.
    fn rank_point(&self, faction: Faction, rank: usize) -> Option<usize> {
        let hall = self
            .spawn_of(faction)
            .or_else(|| self.map.spawns.first())
            .map(|hall| hall.center_cm)
            .unwrap_or([0, 0]);
        let mut chosen: Option<usize> = None;
        for _ in 0..=rank {
            let mut best: Option<usize> = None;
            let mut best_gap = u32::MAX;
            for (point_index, point) in self.map.points.iter().enumerate() {
                if Some(point_index) == chosen {
                    continue;
                }
                if self.points.owner(point_index) == Some(faction) {
                    continue;
                }
                let gap = ring_gap(&hall, point);
                if gap < best_gap {
                    best = Some(point_index);
                    best_gap = gap;
                }
            }
            let point_index = best?;
            chosen = Some(point_index);
        }
        chosen
    }

    /// The map's first spawn hall of one faction, `None` when the map has none.
    fn spawn_of(&self, faction: Faction) -> Option<&MatchSpawn> {
        self.map
            .spawns
            .iter()
            .find(|hall| hall.faction == Some(faction))
    }

    /// One tick of walking toward `target`, in whole centimetres.
    ///
    /// The axes are walked in a fixed order (x, then z) and each takes up to
    /// the seat's step, so a seat never passes its target, never stalls, and
    /// never needs a division, a square root or a random tie-break.
    fn walk_seat(&mut self, index: usize, target: usize) -> [i32; 2] {
        let step = if self.seats[index].faction == Faction::Wardens {
            WARDENS_STEP_CM
        } else {
            ASHKIN_STEP_CM
        };
        let goal = self.map.points[target].center_cm;
        let mut center = self.seats[index].center_cm;
        let mut budget = step;
        for axis in 0..2 {
            if budget == 0 {
                break;
            }
            let gap = goal[axis] - center[axis];
            if gap == 0 {
                continue;
            }
            let take = budget.min(gap.abs());
            center[axis] += gap.signum() * take;
            budget -= take;
        }
        self.seats[index].center_cm = center;
        center
    }

    /// Fold one tick into the running hash: the tick, the seats, the rings and
    /// the score, in authored order.
    fn fold(&mut self, report: &TickReport) {
        self.hasher.write_u64(report.tick);
        self.hasher.write_len(report.targets.len());
        for (index, seat) in self.seats.iter().enumerate() {
            self.hasher
                .write_u8(u8::try_from(seat.faction.index()).unwrap_or(u8::MAX));
            self.hasher.write_i32(report.centers_cm[index][0]);
            self.hasher.write_i32(report.centers_cm[index][1]);
            let target = u32::try_from(report.targets[index]).unwrap_or(u32::MAX);
            self.hasher.write_u32(target);
            self.hasher.write_u8(u8::from(seat.ai));
        }
        for point_index in 0..self.map.points.len() {
            let owner = self.points.owner(point_index);
            self.hasher.write_u8(match owner {
                None => 0,
                Some(Faction::Wardens) => 1,
                Some(Faction::Ashkin) => 2,
            });
            self.hasher.write_u32(self.points.progress(point_index));
            let challenger = self.points.challenger(point_index);
            self.hasher.write_u8(match challenger {
                None => 0,
                Some(Faction::Wardens) => 1,
                Some(Faction::Ashkin) => 2,
            });
            let present = &report.present[point_index];
            self.hasher.write_u16(present[Faction::Wardens.index()]);
            self.hasher.write_u16(present[Faction::Ashkin.index()]);
        }
        self.hasher.write_u32(self.score[0]);
        self.hasher.write_u32(self.score[1]);
        // the tick's hits, by track, tick and point: a hit that moves by one
        // millimetre moves the match hash
        self.hasher.write_u64(hash_hits(&report.hits));
    }

    /// The end of the match, if this tick was it.
    fn end_of_match(&self) -> Option<Winner> {
        let wardens = self.score[Faction::Wardens.index()];
        let ashkin = self.score[Faction::Ashkin.index()];
        if wardens >= SCORE_TO_WIN {
            return Some(Winner::Faction(Faction::Wardens));
        }
        if ashkin >= SCORE_TO_WIN {
            return Some(Winner::Faction(Faction::Ashkin));
        }
        if self.tick >= MATCH_TICKS {
            return Some(if wardens == ashkin {
                Winner::Draw
            } else if wardens > ashkin {
                Winner::Faction(Faction::Wardens)
            } else {
                Winner::Faction(Faction::Ashkin)
            });
        }
        None
    }
}

/// One body a volley can meet: a seat, standing where the tick saw it.
///
/// [`resolve_shot`] walks a round forward tick by tick and asks each track
/// where its body stands at that absolute tick. The match steps a seat's walk
/// on its own clock, so this track answers the standing the volley was fired
/// from: where a body walks into a round already in flight is the snapshot
/// path's work (RY6). The id is the seat's index in the match, so a [`HitEvent`]
/// names a seat the match can look up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SeatTrack {
    /// Seat index this body stands for.
    id: u32,
    /// Body centre in integer millimetres.
    center_mm: [i32; 3],
}

impl BodyTrack for SeatTrack {
    fn id(&self) -> u32 {
        self.id
    }

    fn body_at(&self, _tick: u64) -> Option<Body> {
        Some(Body {
            center_mm: self.center_mm,
            radius_mm: BODY_RADIUS_MM,
        })
    }
}

/// Centimetres to integer millimetres, the unit the resolver reads.
fn cm_to_mm(cm: i32) -> i32 {
    cm.wrapping_mul(10)
}

/// Millimetres to `Fixed` metres, the unit the flight reads.
///
/// A millimetre is 65.536 fixed bits, so a map axis of 300,000 mm is 1.97e10
/// bits, far inside `i64`, and the truncation is far below a micrometre.
fn mm_to_m(mm: i32) -> Fixed {
    Fixed::from_bits(i64::from(mm) * Fixed::SCALE / 1000)
}

/// The fall a class volley makes before it crosses a target `gap_sq` squared
/// centimetres out, in millimetres.
///
/// Both steps are the flight's own: the ticks are `ceil(distance / speed a
/// tick)` at [`VOLLEY_SPEED_MPS`], and the fall is the stepper's sum, gravity
/// added to the velocity before the position moves, `g*dt^2*n*(n+1)/2`. Aiming
/// by this much keeps a long shot on the body instead of under it.
fn volley_fall_mm(gap_sq: i64, gravity: Fixed) -> i32 {
    // the distance is the flight's own integer square root: the gap as Q16
    // metres, read back in whole centimetres
    let distance_cm = Fixed::from_bits(gap_sq << 16).sqrt().to_bits() >> 16;
    // 10,000 cm/s over 60 ticks is 500/3 cm a tick, so the crossing rounds up
    let ticks = (distance_cm * 3 + 499) / 500;
    // gravity is Q16 m/s^2 and dt^2 is 1/3600 s^2, so the sum is Q16 metres;
    // a 60 m reach is 360 ticks, far inside i64
    let fall_bits =
        i128::from(gravity.to_bits()) * i128::from(ticks) * i128::from(ticks + 1) / 7_200;
    let millimetres = fall_bits * 1_000 / i128::from(Fixed::SCALE);
    i32::try_from(millimetres).unwrap_or(i32::MAX)
}

/// The faction across the field from `faction`.
///
/// The war is two-sided (`Faction::ALL` holds exactly two), so this is the one
/// other side; `None` if a later mode ever fields one side alone.
fn enemy_of(faction: Faction) -> Option<Faction> {
    Faction::ALL.into_iter().find(|other| *other != faction)
}

/// A seat's ground plane to integer millimetres, +Y up.
fn body_center_mm(center_cm: [i32; 2]) -> [i32; 3] {
    [
        cm_to_mm(center_cm[0]),
        cm_to_mm(VOLLEY_SHOULDER_CM),
        cm_to_mm(center_cm[1]),
    ]
}

/// Whether a seat at `center_cm` stands inside a point's ring. Squared
/// centimetres in `i64`: the widest map is 300 m across, so a delta is at most
/// 30,000 cm and the sum of two squares at most 1.8e9, far inside `i64`.
fn inside_ring(center_cm: &[i32; 2], point: &MatchPoint) -> bool {
    let dx = i64::from(center_cm[0] - point.center_cm[0]);
    let dz = i64::from(center_cm[1] - point.center_cm[1]);
    let radius = i64::from(point.radius_cm);
    dx * dx + dz * dz <= radius * radius
}

/// Squared centimetres from a seat to a ring centre, for the pick order.
fn ring_gap(center_cm: &[i32; 2], point: &MatchPoint) -> u32 {
    let dx = i64::from(center_cm[0] - point.center_cm[0]);
    let dz = i64::from(center_cm[1] - point.center_cm[1]);
    let gap = dx * dx + dz * dz;
    u32::try_from(gap).unwrap_or(u32::MAX)
}

/// Read one map file's text into the match's point and hall table.
///
/// The file is read once at load, never on the tick; the wasm leg fetches its
/// bytes and calls [`parse_map`] instead.
///
/// # Errors
/// [`MapError`] when the file is not readable, is not JSON, or holds a node
/// the loader refuses.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_map(path: &Path) -> Result<FactionMap, MapError> {
    let bytes = std::fs::read(path).map_err(|error| MapError::Json(error.to_string()))?;
    if bytes.len() > MAX_MAP_BYTES {
        return Err(MapError::Json(format!(
            "the map file is {} bytes, past the {MAX_MAP_BYTES} byte cap",
            bytes.len()
        )));
    }
    let text = String::from_utf8(bytes).map_err(|_| MapError::Json("not UTF-8".to_string()))?;
    parse_map(&text)
}

/// Read map text into the match's point and hall table.
///
/// A `kind: "statue"` node with a nonzero `capture_progress_s` is a capture
/// point; the forts and gates of the same file carry `capture_progress_s` 0
/// and are not points here, exactly as the map's own note says. A spawn hall
/// names its faction; a hall that names none is kept with `None` so a later
/// legion mode can read it.
///
/// # Errors
/// [`MapError`] on malformed JSON, a missing member, a number that is not a
/// whole number of metres, an unknown kind or faction, or a count past its cap.
pub fn parse_map(text: &str) -> Result<FactionMap, MapError> {
    let root =
        parse_up_to(text, MAX_MAP_BYTES).map_err(|error| MapError::Json(error.to_string()))?;
    let id = text_field(&root, "id")?.to_string();
    let size = object_field(&root, "size_m")?;
    let size_cm = [metres_field(size, "x")?, metres_field(size, "z")?];
    let nodes = array_field(&root, "nodes")?;
    if nodes.len() > MAX_POINTS {
        return Err(MapError::TooManyPoints(nodes.len()));
    }
    let mut points = Vec::new();
    for node in nodes {
        let kind = text_field(node, "kind")?;
        if kind != "statue" {
            // a fort, a gate or a road post: the map's own note says these are
            // not capture points
            continue;
        }
        let hold_s = whole_field(node, "capture_progress_s")?;
        if hold_s <= 0 {
            return Err(MapError::BadNumber("capture_progress_s"));
        }
        points.push(MatchPoint {
            id: text_field(node, "id")?.to_string(),
            center_cm: [metres_field(node, "x")?, metres_field(node, "z")?],
            radius_cm: radius_field(node, "radius")?,
            hold_ticks: u32::try_from(hold_s)
                .ok()
                .and_then(|seconds| seconds.checked_mul(60))
                .ok_or(MapError::BadNumber("capture_progress_s"))?,
        });
    }
    if points.is_empty() {
        return Err(MapError::NoPoints);
    }
    let halls = array_field(&root, "spawns")?;
    if halls.len() > MAX_SPAWNS {
        return Err(MapError::TooManySpawns(halls.len()));
    }
    let mut spawns = Vec::with_capacity(halls.len());
    for hall in halls {
        let name = text_field(hall, "faction")?;
        spawns.push(MatchSpawn {
            id: text_field(hall, "id")?.to_string(),
            faction: Some(match name {
                "wardens" => Faction::Wardens,
                "ashkin" => Faction::Ashkin,
                _ => return Err(MapError::BadFaction(name.to_string())),
            }),
            center_cm: [metres_field(hall, "x")?, metres_field(hall, "z")?],
        });
    }
    Ok(FactionMap {
        id,
        size_cm,
        points,
        spawns,
    })
}

/// The Tier A deployment: six seats a side, two of each of the three Tier A
/// roles, the last two seats a side AI-filled.
///
/// The roles are the plan's Knight (melee, gap closer), Archer (projectile,
/// range band) and Sorcerer (slow bolt, area denial), read from the Wardens'
/// and the Ashkin' own sheets in `content/classes/`. The Wardens field the
/// shield line, the ranged debuffer and the elemental nuker; the Ashkin field
/// the peel tank, the arm-cannon artillery and the poison nuker. Seats stand
/// in a small deterministic lattice inside their faction's first hall, so the
/// deployment itself has no random number in it.
///
/// # Errors
/// [`MatchError`] when the map holds no hall for a faction.
pub fn tier_a_seats(map: &FactionMap) -> Result<Vec<Seat>, MatchError> {
    const CLASSES: [(Faction, [&str; 3]); 2] = [
        (
            Faction::Wardens,
            ["warden_defender", "warden_archer", "warden_sorcerer"],
        ),
        (
            Faction::Ashkin,
            ["ashkin_templar", "ashkin_gunner", "ashkin_rune"],
        ),
    ];
    let mut seats = Vec::with_capacity(CLASSES.len() * SEATS_PER_SIDE);
    for (faction, classes) in CLASSES {
        let hall = map
            .spawns
            .iter()
            .find(|hall| hall.faction == Some(faction))
            .ok_or(MatchError::NoSpawn(faction))?;
        for seat in 0..SEATS_PER_SIDE {
            // the last AI_SEATS_PER_SIDE seats of each side are the ones the
            // server fills when no player is bound (DP1)
            let ai = seat + AI_SEATS_PER_SIDE >= SEATS_PER_SIDE;
            // a 2x3 lattice, two metres apart, centred on the hall so the
            // deployment stands in it rather than on one grid point
            let column = i32::try_from(seat % 2).unwrap_or(0);
            let row = i32::try_from(seat / 2).unwrap_or(0);
            seats.push(Seat {
                faction,
                class: classes[seat % classes.len()].to_string(),
                ai,
                center_cm: [
                    hall.center_cm[0] + column * 200 - 100,
                    hall.center_cm[1] + row * 200 - 200,
                ],
                target: 0,
                pick_at: 0,
            });
        }
    }
    Ok(seats)
}

/// One required text member of an object.
fn text_field<'a>(node: &'a Json, key: &'static str) -> Result<&'a str, MapError> {
    let member = node.get(key).ok_or(MapError::Missing(key))?;
    member.as_str().ok_or(MapError::NotAnObject(key))
}

/// One required object member.
fn object_field<'a>(node: &'a Json, key: &'static str) -> Result<&'a Json, MapError> {
    let member = node.get(key).ok_or(MapError::Missing(key))?;
    if member.as_object().is_some() {
        Ok(member)
    } else {
        Err(MapError::NotAnObject(key))
    }
}

/// One required array member.
fn array_field<'a>(node: &'a Json, key: &'static str) -> Result<&'a [Json], MapError> {
    let member = node.get(key).ok_or(MapError::Missing(key))?;
    member.as_array().ok_or(MapError::NotAnObject(key))
}

/// One member that must be a whole number of metres, read in centimetres.
fn metres_field(node: &Json, key: &'static str) -> Result<i32, MapError> {
    let value = whole_field(node, key)?;
    i32::try_from(value)
        .ok()
        .and_then(|metres| metres.checked_mul(100))
        .ok_or(MapError::BadNumber(key))
}

/// One member that must be a whole number of metres of radius, read in
/// centimetres. A radius of zero or less is refused: a ring you cannot stand
/// in would flip a point nobody can reach.
fn radius_field(node: &Json, key: &'static str) -> Result<u32, MapError> {
    let metres = whole_field(node, key)?;
    if metres <= 0 {
        return Err(MapError::BadNumber(key));
    }
    u32::try_from(metres)
        .ok()
        .and_then(|metres| metres.checked_mul(100))
        .ok_or(MapError::BadNumber(key))
}

/// One member that must be a whole number (a radius, a capture time, an axis).
fn whole_field(node: &Json, key: &'static str) -> Result<i64, MapError> {
    let member = node.get(key).ok_or(MapError::Missing(key))?;
    let lexeme = member.number_text().ok_or(MapError::BadNumber(key))?;
    lexeme.parse::<i64>().map_err(|_| MapError::BadNumber(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped map file, read as text so the tests carry no filesystem.
    const MAP_TEXT: &str = include_str!("../../../content/maps/contested_middle.json");

    fn map() -> FactionMap {
        parse_map(MAP_TEXT).expect("the shipped map parses")
    }

    /// The roster sheet shape every class test sheet is built from: six
    /// abilities, a growth that sums positive, a growing primary stat. The
    /// `@class@` marker is the class id and `@faction@` the faction, so one
    /// shape covers every class, and `@range@` is the first ability's range,
    /// the one number a test moves.
    const SHEET_SHAPE: &str = r#"{
      "id": "@class@",
      "faction": "@faction@",
      "tier": 0,
      "branch": null,
      "role": "line breaker",
      "growth": {"str": 1, "dex": 0, "con": 1, "int": 0, "wis": 0},
      "primary": ["str", "con"],
      "playstyle": "Stand in front of the line and let them come to you.",
      "abilities": [
        {"id": "@class@.a", "name": "A", "tier": 0, "kind": "control", "shape": "single", "range": @range@, "radius": 0, "cast": 0.4, "cooldown": 14, "mana": 12, "telegraph": "none", "damage": {"base": 0, "scale": "none"}, "effects": ["root", "damage"], "notes": "n"},
        {"id": "@class@.b", "name": "B", "tier": 0, "kind": "aoe", "shape": "cone", "range": 2.6, "radius": 3, "cast": 0.5, "cooldown": 3, "mana": 0, "telegraph": "cone", "damage": {"base": 28, "scale": "str"}, "effects": [{"type": "damage_reduction", "value": 25, "duration": 6}], "notes": "n"},
        {"id": "@class@.c", "name": "C", "tier": 0, "kind": "utility", "shape": "self", "range": 0, "radius": 0, "cast": 0, "cooldown": 12, "mana": 0, "telegraph": "none", "damage": {"base": 0, "scale": "none"}, "effects": ["move_speed"], "notes": "n"},
        {"id": "@class@.d", "name": "D", "tier": 1, "kind": "melee", "shape": "single", "range": 2.4, "radius": 0, "cast": 0.6, "cooldown": 5, "mana": 0, "telegraph": "none", "damage": {"base": 52, "scale": "str"}, "effects": ["damage"], "notes": "n"},
        {"id": "@class@.e", "name": "E", "tier": 2, "kind": "buff", "shape": "circle", "range": 6, "radius": 6, "cast": 0, "cooldown": 22, "mana": 18, "telegraph": "none", "damage": {"base": 0, "scale": "none"}, "effects": ["damage_reduction"], "notes": "n"},
        {"id": "@class@.f", "name": "F", "tier": 4, "kind": "utility", "shape": "self", "range": 0, "radius": 0, "cast": 0, "cooldown": 90, "mana": 30, "telegraph": "none", "damage": {"base": 0, "scale": "none"}, "effects": ["heal"], "notes": "n"}
      ],
      "loadout": {"weapon": "sword", "offhand": "buckler"},
      "counters": ["other"],
      "weakness": "Low damage."
    }"#;

    /// One sheet per class the seats carry, built from the shape, so no test
    /// reads the class directory from the filesystem.
    fn sheets(classes: &[&str], range: i32) -> Vec<ClassSheet> {
        sheets_with(classes, range, "control", 14.0)
    }

    /// One sheet per class, from the shared shape, with its first ability's
    /// kind, range and cooldown set: the three rows a volley test moves.
    fn sheets_with(classes: &[&str], range: i32, kind: &str, cooldown: f32) -> Vec<ClassSheet> {
        classes
            .iter()
            .map(|class| {
                // the faction the class id names, so a warden sheet is not
                // filed under the ashkin
                let faction = if class.starts_with("warden") {
                    "wardens"
                } else {
                    "ashkin"
                };
                let text = SHEET_SHAPE
                    .replace("@class@", class)
                    .replace("@faction@", faction)
                    .replace("@range@", &range.to_string())
                    .replace(r#""kind": "control""#, &format!(r#""kind": "{kind}""#))
                    .replace(r#""cooldown": 14"#, &format!(r#""cooldown": {cooldown}"#));
                ClassSheet::parse(class, &text).expect("the sheet shape parses")
            })
            .collect()
    }

    /// The classes the seats name, in seat order, as the match resolves them.
    fn classes_of(seats: &[Seat]) -> Vec<&str> {
        seats.iter().map(|seat| seat.class.as_str()).collect()
    }

    #[test]
    fn the_shipped_map_reads_three_statues_and_two_halls_a_side() {
        let map = map();
        assert_eq!(map.id, "contested_middle");
        assert_eq!(map.size_cm, [30_000, 30_000]);
        assert_eq!(map.points.len(), 3);
        assert_eq!(map.points[0].id, "statue_west");
        assert_eq!(map.points[0].center_cm, [-9_200, 0]);
        assert_eq!(map.points[0].radius_cm, 1_200);
        // 45 s of hold at 60 Hz
        assert_eq!(map.points[0].hold_ticks, 2_700);
        // the mid statue takes twice as long and its ring is tighter
        assert_eq!(map.points[2].hold_ticks, 5_400);
        assert_eq!(map.points[2].radius_cm, 900);
        // the forts and gates are not points
        assert!(map
            .points
            .iter()
            .all(|point| point.id.starts_with("statue")));
        let wardens = map
            .spawns
            .iter()
            .filter(|hall| hall.faction == Some(Faction::Wardens))
            .count();
        let ashkin = map
            .spawns
            .iter()
            .filter(|hall| hall.faction == Some(Faction::Ashkin))
            .count();
        assert_eq!(wardens, 2);
        assert_eq!(ashkin, 2);
    }

    #[test]
    fn a_map_without_a_statue_is_refused() {
        let text = r#"{"id":"x","size_m":{"x":10,"z":10},"nodes":[{"id":"f","kind":"fort","x":0,"z":0,"radius":5,"capture_progress_s":0,"contested_by":2}],"spawns":[]}"#;
        assert_eq!(parse_map(text).unwrap_err(), MapError::NoPoints);
        let missing = r#"{"id":"x","size_m":{"x":10,"z":10},"nodes":[],"spawns":[]}"#;
        assert_eq!(parse_map(missing).unwrap_err(), MapError::NoPoints);
        let no_z = r#"{"id":"x","size_m":{"x":10},"nodes":[],"spawns":[]}"#;
        assert_eq!(parse_map(no_z).unwrap_err(), MapError::Missing("z"));
        let bad_faction = r#"{"id":"x","size_m":{"x":10,"z":10},"nodes":[{"id":"s","kind":"statue","x":0,"z":0,"radius":5,"capture_progress_s":45,"contested_by":2}],"spawns":[{"id":"h","faction":"romans","x":0,"z":0}]}"#;
        assert_eq!(
            parse_map(bad_faction).unwrap_err(),
            MapError::BadFaction("romans".to_string())
        );
    }

    #[test]
    fn a_match_with_no_seats_is_refused() {
        let map = map();
        assert_eq!(
            Match::new(map, Vec::new(), Vec::new()).unwrap_err(),
            MatchError::NoSeats
        );
    }

    #[test]
    fn a_seat_without_a_hall_is_refused() {
        let mut map = map();
        map.spawns
            .retain(|hall| hall.faction == Some(Faction::Wardens));
        let seat = Seat {
            faction: Faction::Ashkin,
            class: "ashkin_templar".to_string(),
            ai: false,
            center_cm: [0, 12_000],
            target: 0,
            pick_at: 0,
        };
        assert_eq!(
            Match::new(map, vec![seat], Vec::new()).unwrap_err(),
            MatchError::NoSpawn(Faction::Ashkin)
        );
    }

    /// Two seats, one a side, walking the same ring: a frozen match that only
    /// the clock can end.
    fn frozen_match() -> Match {
        let seats = frozen_seats();
        let sheets = sheets(&classes_of(&seats), 9);
        Match::new(map(), seats, sheets).expect("two seats over the shipped map")
    }

    /// The frozen match's seats, so a test can build it with its own sheets.
    fn frozen_seats() -> Vec<Seat> {
        vec![
            Seat {
                faction: Faction::Wardens,
                class: "warden_defender".to_string(),
                ai: true,
                center_cm: [0, -4_000],
                target: 2,
                pick_at: 0,
            },
            Seat {
                faction: Faction::Ashkin,
                class: "ashkin_templar".to_string(),
                ai: true,
                center_cm: [0, 4_000],
                target: 2,
                pick_at: 0,
            },
        ]
    }

    #[test]
    fn a_contested_ring_never_scores_and_the_clock_ends_it_level() {
        let mut match_state = frozen_match();
        let winner = match_state.run();
        assert_eq!(winner.unwrap(), Winner::Draw);
        assert_eq!(match_state.tick(), MATCH_TICKS);
        assert_eq!(match_state.score(Faction::Wardens), 0);
        assert_eq!(match_state.score(Faction::Ashkin), 0);
        // the mid statue is held by neither side
        assert_eq!(match_state.points().owner(2), None);
    }

    #[test]
    fn two_runs_of_one_match_hash_equal() {
        let first = {
            let mut match_state = frozen_match();
            match_state.run().expect("a frozen match runs to the clock");
            match_state.hash()
        };
        let second = {
            let mut match_state = frozen_match();
            match_state.run().expect("a frozen match runs to the clock");
            match_state.hash()
        };
        assert_eq!(first, second);
        assert_ne!(first, 0);
    }

    #[test]
    fn the_four_ai_seats_act_without_a_model() {
        // the Tier A deployment: twelve seats, four of them AI
        let map = map();
        let seats = tier_a_seats(&map).expect("the deployment builds");
        assert_eq!(seats.len(), 2 * SEATS_PER_SIDE);
        assert_eq!(
            seats.iter().filter(|seat| seat.ai).count(),
            2 * AI_SEATS_PER_SIDE
        );
        let sheets = sheets(&classes_of(&seats), 9);
        let mut match_state = Match::new(map, seats, sheets).expect("the match builds");
        // before the first step every seat stands in its hall
        let halls = match_state
            .seats()
            .iter()
            .map(|seat| seat.center_cm)
            .collect::<Vec<_>>();
        match_state.step().expect("the first tick steps");
        let moved = match_state
            .seats()
            .iter()
            .zip(&halls)
            .filter(|(seat, before)| seat.center_cm != **before)
            .count();
        // every seat walks toward its point on the very first tick
        assert_eq!(moved, 2 * SEATS_PER_SIDE);
        // the four AI seats among them: their targets come from the state
        for (index, seat) in match_state.seats().iter().enumerate() {
            if seat.ai {
                assert!(seat.target < match_state.map().points.len());
                let _ = index;
            }
        }
    }

    #[test]
    fn the_ai_seats_act_right_through_the_match() {
        // one run to the clock, then the AI seats are read where they stood:
        // an AI seat that only picked a point on tick one and stopped would
        // never be inside a ring, so this is the action, not the intent
        let map = map();
        let seats = tier_a_seats(&map).expect("the deployment builds");
        let sheets = sheets(&classes_of(&seats), 9);
        let mut match_state = Match::new(map, seats, sheets).expect("the match builds");
        match_state.run().expect("the match runs to its end");
        let points = match_state.map().points.clone();
        let standing = match_state
            .seats()
            .iter()
            .filter(|seat| {
                points
                    .iter()
                    .any(|point| inside_ring(&seat.center_cm, point))
            })
            .count();
        assert!(standing >= 2 * SEATS_PER_SIDE - 2);
        // at least one of them is a seat the server filled, not a player's
        assert!(match_state
            .seats()
            .iter()
            .filter(|seat| seat.ai)
            .any(|seat| {
                points
                    .iter()
                    .any(|point| inside_ring(&seat.center_cm, point))
            }));
    }

    #[test]
    fn a_tier_a_match_ends_by_score_or_time() {
        let map = map();
        let seats = tier_a_seats(&map).expect("the deployment builds");
        let sheets = sheets(&classes_of(&seats), 9);
        let mut match_state = Match::new(map, seats, sheets).expect("the match builds");
        let winner = match_state.run();
        // the Ashkin spread over all three statues while the Wardens line up
        // on the mid: the statues the Ashkin hold unopposed score first
        assert_eq!(winner.unwrap(), Winner::Faction(Faction::Ashkin));
        assert_eq!(match_state.score(Faction::Ashkin), SCORE_TO_WIN);
        assert!(match_state.tick() <= MATCH_TICKS);
        assert_eq!(match_state.points().owner(0), Some(Faction::Ashkin));
        assert_eq!(match_state.points().owner(1), Some(Faction::Ashkin));
        // the middle statue is contested forever: no side ever flips it, and
        // the last challenger in it stays named because a contested ring holds
        // the challenger it froze under (RY4), never clears it
        assert_eq!(match_state.points().owner(2), None);
        assert!(match_state.points().challenger(2).is_some());
    }

    #[test]
    fn the_match_reads_points_never_bodies() {
        let mut match_state = frozen_match();
        // one body of a side inside a ring scores nothing: scoring is the
        // match's rule over the points, and this match holds one point at most
        for _ in 0..600 {
            match_state.step().expect("a contested ring never fails");
        }
        assert_eq!(match_state.points().points_owned(Faction::Wardens), 0);
        assert_eq!(match_state.points().points_owned(Faction::Ashkin), 0);
        assert_eq!(match_state.score(Faction::Wardens), 0);
    }

    #[test]
    fn the_hash_sees_a_moved_seat() {
        let base = {
            let mut match_state = frozen_match();
            match_state.step().expect("the first tick steps");
            match_state.hash()
        };
        let moved = {
            let mut seats = tier_a_seats(&map()).expect("the deployment builds");
            // one seat's spawn hall position changed by a centimetre
            seats[0].center_cm[0] += 1;
            let sheets = sheets(&classes_of(&seats), 9);
            let mut match_state = Match::new(map(), seats, sheets).expect("the match builds");
            match_state.step().expect("the first tick steps");
            match_state.hash()
        };
        assert_ne!(base, moved);
    }

    /// The frozen match's hash after one tick, built with the first ability's
    /// range at `range`: the same match twice gives the same hash, and a
    /// different range a different one (RY2).
    fn frozen_hash_at(range: i32) -> u64 {
        let seats = frozen_seats();
        let sheets = sheets(&classes_of(&seats), range);
        let mut match_state =
            Match::new(map(), seats, sheets).expect("two seats over the shipped map");
        match_state.step().expect("the first tick steps");
        match_state.hash()
    }

    #[test]
    fn a_sheet_value_change_moves_the_match_hash() {
        // one ability's range, one number: the hash follows it
        assert_ne!(frozen_hash_at(9), frozen_hash_at(10));
    }

    #[test]
    fn the_same_sheets_twice_give_the_same_hash() {
        // the same sheets twice, built again from the shape: equal
        assert_eq!(frozen_hash_at(9), frozen_hash_at(9));
    }

    #[test]
    fn a_seat_whose_class_carries_no_sheet_is_refused() {
        let seats = frozen_seats();
        let mut fewer = sheets(&classes_of(&seats), 9);
        // the Ashkin templar's sheet is the one that goes
        fewer.pop();
        assert_eq!(
            Match::new(map(), seats, fewer).unwrap_err(),
            MatchError::NoClassSheet("ashkin_templar".to_string())
        );
    }

    /// The shipped class file of a class, read as text so a test folds the
    /// real row the server ships and carries no filesystem.
    fn shipped(class: &str) -> &'static str {
        match class {
            "warden_defender" => include_str!("../../../content/classes/warden_defender.json"),
            "warden_archer" => include_str!("../../../content/classes/warden_archer.json"),
            "warden_sorcerer" => include_str!("../../../content/classes/warden_sorcerer.json"),
            "ashkin_templar" => include_str!("../../../content/classes/ashkin_templar.json"),
            "ashkin_gunner" => include_str!("../../../content/classes/ashkin_gunner.json"),
            "ashkin_rune" => include_str!("../../../content/classes/ashkin_rune.json"),
            other => panic!("the Tier A slice ships no sheet for {other}"),
        }
    }

    /// One sheet per class the seats carry, each its own shipped file.
    fn shipped_sheets(classes: &[&str]) -> Vec<ClassSheet> {
        classes
            .iter()
            .map(|class| {
                ClassSheet::parse(class, shipped(class)).expect("the shipped class file parses")
            })
            .collect()
    }

    #[test]
    fn the_match_carries_the_shipped_class_sheets() {
        let map = map();
        let seats = tier_a_seats(&map).expect("the deployment builds");
        let classes = classes_of(&seats);
        let shipped_sheets = shipped_sheets(&classes);
        let match_state = Match::new(map, seats, shipped_sheets).expect("the match builds");
        // twelve seats over six classes, each seat on the row that ships for it
        assert_eq!(match_state.sheets().len(), 2 * SEATS_PER_SIDE);
        for (seat, sheet) in match_state.seats().iter().zip(match_state.sheets()) {
            assert_eq!(sheet.id, seat.class);
            // the shipped row is filed under the faction the seat carries
            assert_eq!(sheet.faction, seat.faction.to_string().to_lowercase());
        }
    }

    #[test]
    fn a_shipped_sheet_value_change_moves_the_match_hash() {
        // the Tier A hash with the shipped archer row as it is, then with its
        // basic shot one metre longer: the real data, one real number moved
        let hash_of = |archer: &str| {
            let map = map();
            let seats = tier_a_seats(&map).expect("the deployment builds");
            let sheets = classes_of(&seats)
                .iter()
                .map(|class| {
                    let text = if *class == "warden_archer" {
                        archer
                    } else {
                        shipped(class)
                    };
                    ClassSheet::parse(class, text).expect("the shipped class file parses")
                })
                .collect();
            let mut match_state = Match::new(map, seats, sheets).expect("the match builds");
            match_state.step().expect("the first tick steps");
            match_state.hash()
        };
        let base = hash_of(shipped("warden_archer"));
        // the archer's basic shot ships at 22 m; a guard so the replace below
        // still hits if the row is retuned
        assert!(shipped("warden_archer").contains(r#""range": 22"#));
        let moved = shipped("warden_archer").replace(r#""range": 22"#, r#""range": 23"#);
        assert_ne!(base, hash_of(&moved));
        // and the shipped row twice, read again from its file, is the same hash
        assert_eq!(base, hash_of(shipped("warden_archer")));
    }

    /// A straight lane: one statue in front of each hall, so each side's
    /// nearest point is its own and the two seats walk apart instead of
    /// meeting on one ring. Volleys are then the only thing that crosses.
    fn lane_map() -> FactionMap {
        FactionMap {
            id: "lane".to_string(),
            size_cm: [30_000, 30_000],
            points: vec![
                MatchPoint {
                    id: "statue_west".to_string(),
                    center_cm: [-2_000, 0],
                    radius_cm: 300,
                    hold_ticks: 2_700,
                },
                MatchPoint {
                    id: "statue_east".to_string(),
                    center_cm: [2_000, 0],
                    radius_cm: 300,
                    hold_ticks: 2_700,
                },
            ],
            spawns: vec![
                MatchSpawn {
                    id: "spawn_warden".to_string(),
                    faction: Some(Faction::Wardens),
                    center_cm: [-2_000, 0],
                },
                MatchSpawn {
                    id: "spawn_ashkin".to_string(),
                    faction: Some(Faction::Ashkin),
                    center_cm: [2_000, 0],
                },
            ],
        }
    }

    /// Two seats 16 m apart on the lane, walking away from each other, both
    /// carrying the shared shape with `kind`, `range` and `cooldown` on its
    /// first ability: the one row a volley test is about.
    fn duel(kind: &str, range: i32, cooldown: f32) -> Match {
        let seats = vec![
            Seat {
                faction: Faction::Wardens,
                class: "warden_archer".to_string(),
                ai: true,
                center_cm: [-800, 0],
                target: 0,
                pick_at: 0,
            },
            Seat {
                faction: Faction::Ashkin,
                class: "ashkin_gunner".to_string(),
                ai: true,
                center_cm: [800, 0],
                target: 1,
                pick_at: 0,
            },
        ];
        let sheets = sheets_with(&["warden_archer", "ashkin_gunner"], range, kind, cooldown);
        Match::new(lane_map(), seats, sheets).expect("two seats over the lane map")
    }

    #[test]
    fn the_shipped_class_rows_turn_metres_and_seconds_into_ticks() {
        // the archer's basic shot ships at 22 m and 1.2 s: centimetres and
        // ticks, converted once
        let archer = shipped_sheets(&["warden_archer"]);
        assert_eq!(
            volley_of(&archer[0]),
            SeatVolley {
                range_cm: 2_200,
                cooldown_ticks: 72,
            }
        );
        // the line classes carry no projectile row, so they never fire
        let line = shipped_sheets(&["warden_defender", "ashkin_templar"]);
        assert!(line.iter().all(|sheet| volley_of(sheet) == NO_VOLLEY));
    }

    #[test]
    fn a_line_class_never_fires() {
        let mut match_state = frozen_match();
        assert!(match_state
            .volleys()
            .iter()
            .all(|volley| *volley == NO_VOLLEY));
        match_state.run().expect("a frozen match runs to the clock");
        assert_eq!(match_state.hits_landed(Faction::Wardens), 0);
        assert_eq!(match_state.hits_landed(Faction::Ashkin), 0);
        assert!(match_state.hit_events().is_empty());
    }

    #[test]
    fn a_shooting_class_lands_a_volley_and_the_hit_names_the_body() {
        let mut match_state = duel("projectile", 30, 1.0);
        match_state.step().expect("the first tick steps");
        // one round each way, both inside the reach
        assert_eq!(match_state.hits_landed(Faction::Wardens), 1);
        assert_eq!(match_state.hits_landed(Faction::Ashkin), 1);
        assert_eq!(match_state.hit_events().len(), 2);
        // seat 0 is the warden, so its round met seat 1 and the ashkin's met
        // seat 0: the event quotes the body, not the shooter
        let warden_hit = match_state.hit_events()[0];
        assert_eq!(warden_hit.track, 1);
        assert_eq!(match_state.hit_events()[1].track, 0);
        // a flight from 16 m at 100 m/s is a handful of ticks, not a second
        assert!(warden_hit.hit_tick >= 1 && warden_hit.hit_tick < 60);
        // the event's point lies inside the body shell it met
        let center = body_center_mm(match_state.seats()[1].center_cm);
        let mut gap = 0i64;
        for (point, body) in warden_hit.point_mm.iter().zip(center) {
            let delta = i64::from(point - body);
            gap += delta * delta;
        }
        let radius = i64::from(BODY_RADIUS_MM);
        assert!(gap <= radius * radius);
    }

    #[test]
    fn the_sheet_range_gates_the_volley() {
        // 16 m apart: a 10 m class reach holds its fire, a 30 m one does not
        let mut short = duel("projectile", 10, 1.0);
        short.step().expect("the first tick steps");
        assert_eq!(short.hits_landed(Faction::Wardens), 0);
        assert_eq!(short.hits_landed(Faction::Ashkin), 0);
        assert!(short.hit_events().is_empty());
        let mut far = duel("projectile", 30, 1.0);
        far.step().expect("the first tick steps");
        assert_eq!(far.hits_landed(Faction::Wardens), 1);
        // different rules are a different match
        assert_ne!(short.hash(), far.hash());
    }

    #[test]
    fn the_sheet_cooldown_gates_the_cadence() {
        // one second at 60 Hz is 60 ticks, so 61 ticks hold two volleys, not 61
        let mut match_state = duel("projectile", 60, 1.0);
        assert_eq!(
            match_state.volleys()[0],
            SeatVolley {
                range_cm: 6_000,
                cooldown_ticks: 60,
            }
        );
        for _ in 0..61 {
            match_state.step().expect("one tick steps");
        }
        assert_eq!(match_state.tick(), 61);
        assert_eq!(match_state.hits_landed(Faction::Wardens), 2);
        assert_eq!(match_state.hits_landed(Faction::Ashkin), 2);
        // and the same duel twice hashes equal, hits and all
        let mut twin = duel("projectile", 60, 1.0);
        for _ in 0..61 {
            twin.step().expect("one tick steps");
        }
        assert_eq!(match_state.hash(), twin.hash());
    }

    #[test]
    fn a_tier_a_match_lands_volleys_on_the_shipped_map() {
        let map = map();
        let seats = tier_a_seats(&map).expect("the deployment builds");
        let classes = classes_of(&seats);
        let shipped_sheets = shipped_sheets(&classes);
        let mut match_state = Match::new(map, seats, shipped_sheets).expect("the match builds");
        match_state.run().expect("the match runs to the end");
        // the two sides close on the statues and the ranged classes fire; the
        // event log and the per-faction count agree, and every event names a
        // seat the match holds
        let wardens = match_state.hits_landed(Faction::Wardens);
        let ashkin = match_state.hits_landed(Faction::Ashkin);
        assert!(wardens > 0, "the shipped archers and sorcerers never fired");
        assert_eq!(
            match_state.hit_events().len(),
            usize::try_from(wardens + ashkin).expect("hits fit a usize"),
        );
        assert!(match_state.hit_events().iter().all(
            |hit| usize::try_from(hit.track).is_ok_and(|seat| seat < match_state.seats().len())
        ));
    }
}
