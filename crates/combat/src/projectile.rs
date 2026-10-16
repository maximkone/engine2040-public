//! projectile: fixed-point round flight over caller-owned casts.
//!
//! Owner: engine-combat. Consumer: `step_volley` (production volley step)
//! over caller casts; the live game uses `NullCaster` (game/ballistics.rs:643).
//! `WallCaster` stays the single-wall harness oracle. Units: metres, metres per
//! second, ticks at 60 Hz; spread is the tangent of the half-angle
//! (`0..=0.25`). Determinism: `Fixed` Q16 math with `+ - * /` and `sqrt`
//! only, spread forked from `(tick, shooter, shot)`; the same tables plus
//! the same spawn triple give the same trajectory on every platform.
//! Errors: `FlightError` at every boundary; stepping a spent round is a
//! silent no-op, never a panic. Budget: O(rounds) integer work per tick;
//! slices in, slices out, no heap on the tick path.

use engine_core::fixed::Fixed;
use engine_core::hash::{hash_u64s, Fnv64};
use engine_core::Rng;

/// Version tag of the shipped flight rows.
pub const FLIGHT_VERSION_V1: u32 = 1;
/// Hard bound on rounds stepped in one call; bounds caller buffers up front.
pub const MAX_ROUNDS_PER_TICK: usize = 2048;
/// Longest round life in ticks (60 s at 60 Hz); bounds every flight.
pub const MAX_FLIGHT_TICKS: u32 = 3600;
/// Fastest muzzle velocity in m/s; covers rifles with margin.
pub const MAX_SPEED_MPS: Fixed = Fixed::from_int(2000);
/// Farthest spawn coordinate from the origin in m, per axis.
pub const MAX_ORIGIN_M: Fixed = Fixed::from_int(4000);
/// Widest accepted spread as tangent of the half-angle (about 14 deg).
pub const MAX_SPREAD: Fixed = Fixed::from_bits(16384);
/// Shipped gravity: 9.81 m/s² rounded down to Q16 (9.81 * 65536 = 642908.16).
pub const GRAVITY_V1: Fixed = Fixed::from_bits(642908);
/// The shipped rows as one static row.
pub const FLIGHT_V1: FlightTables = FlightTables::v1();
/// Fixed-point tick step: 1/60 s truncated deterministically (65536/60).
const DT: Fixed = Fixed::from_bits(Fixed::ONE.to_bits() / TICK_HZ_RAW);
/// Tick rate as a raw Q16 divisor; mirrors `engine_core::tick::TICK_HZ`.
const TICK_HZ_RAW: i64 = 60;
/// Spread-stream domain tag: arbitrary, separates spread draws from any
/// other forked stream.
const SPREAD_DOMAIN: u64 = 0x5350_5245_4107_0001;

/// Tunable flight rows: every number the stepper reads, in one place.
///
/// Units: `gravity_mps2` is downward acceleration in m/s²;
/// `linear_drag_per_s` and `quad_drag_per_m` shape the per-second velocity
/// loss `drag = lin + quad * speed`. Plain `Fixed` only, so sets compare
/// and clone deterministically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlightTables {
    /// Downward acceleration in m/s², `0..=32`.
    pub gravity_mps2: Fixed,
    /// Linear drag in 1/s, `0..=8`.
    pub linear_drag_per_s: Fixed,
    /// Quadratic drag in 1/m, `0..=4`.
    pub quad_drag_per_m: Fixed,
}

impl FlightTables {
    /// Largest sane gravity in m/s².
    pub const MAX_GRAVITY: Fixed = Fixed::from_int(32);
    /// Largest sane linear drag in 1/s.
    pub const MAX_LINEAR_DRAG: Fixed = Fixed::from_int(8);
    /// Largest sane quadratic drag in 1/m.
    pub const MAX_QUAD_DRAG: Fixed = Fixed::from_int(4);
    /// The shipped rows: Earth gravity, no drag. Flat-fire rounds match the
    /// vacuum curve; drag is tuned per round later.
    pub const fn v1() -> Self {
        Self {
            gravity_mps2: GRAVITY_V1,
            linear_drag_per_s: Fixed::ZERO,
            quad_drag_per_m: Fixed::ZERO,
        }
    }
}

/// A versioned set of flight rows: the current tables as version 1, or a
/// replacement built from explicit rows.
///
/// `version` 0 is reserved and rejected; use [`FlightTableSet::v1`] for the
/// shipped rules or [`FlightTableSet::new`] with an explicit nonzero
/// version, then [`validate_flight_tables`] before stepping under them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlightTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// The tunable rows stepped under this version.
    pub tables: FlightTables,
}

impl FlightTableSet {
    /// The shipped rules as version 1.
    pub const fn v1() -> Self {
        Self {
            version: FLIGHT_VERSION_V1,
            tables: FLIGHT_V1,
        }
    }

    /// Constructor from explicit rows and a nonzero version. Validation is
    /// separate: call [`validate_flight_tables`]; only validated sets step.
    pub const fn new(version: u32, tables: FlightTables) -> Self {
        Self { version, tables }
    }
}

/// Range-check a versioned flight set. First failure wins, deterministically.
///
/// Rules: version is nonzero; gravity is `0..=32` m/s²; linear drag is
/// `0..=8` 1/s; quadratic drag is `0..=4` 1/m.
pub fn validate_flight_tables(set: &FlightTableSet) -> Result<(), FlightError> {
    if set.version == 0 {
        return Err(FlightError::BadVersion(0));
    }
    let tables = &set.tables;
    if tables.gravity_mps2 < Fixed::ZERO || tables.gravity_mps2 > FlightTables::MAX_GRAVITY {
        return Err(FlightError::BadValue {
            field: "gravity_mps2",
            value: tables.gravity_mps2.to_bits(),
        });
    }
    if tables.linear_drag_per_s < Fixed::ZERO
        || tables.linear_drag_per_s > FlightTables::MAX_LINEAR_DRAG
    {
        return Err(FlightError::BadValue {
            field: "linear_drag_per_s",
            value: tables.linear_drag_per_s.to_bits(),
        });
    }
    if tables.quad_drag_per_m < Fixed::ZERO || tables.quad_drag_per_m > FlightTables::MAX_QUAD_DRAG
    {
        return Err(FlightError::BadValue {
            field: "quad_drag_per_m",
            value: tables.quad_drag_per_m.to_bits(),
        });
    }
    Ok(())
}

/// Resolve a version tag to its static rows: 1 gives the shipped row,
/// anything else is unknown. Replacement sets travel as [`FlightTableSet`]
/// and step directly; they are never statics.
pub fn flight_for_version(version: u32) -> Option<&'static FlightTables> {
    match version {
        FLIGHT_VERSION_V1 => Some(&FLIGHT_V1),
        _ => None,
    }
}

/// Bounded flight failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlightError {
    /// Table set carries version 0, which is reserved and never valid anew.
    BadVersion(u32),
    /// One tunable row or spawn field is outside its sane range; field name
    /// and culprit carried. First failure wins, deterministically.
    BadValue {
        /// Table or spawn field name, e.g. `"gravity_mps2"`.
        field: &'static str,
        /// Culprit value (raw Q16 bits for fixed rows, plain count for ticks).
        value: i64,
    },
    /// Version tag with no known rows.
    UnknownVersion(u32),
    /// More rounds than [`MAX_ROUNDS_PER_TICK`] in one call, length carried.
    TooManyRounds(usize),
    /// Caller hit buffer length differs from the round count; both carried.
    LengthMismatch {
        /// Round count.
        rounds: usize,
        /// Hit buffer length.
        hits: usize,
    },
    /// Muzzle speed outside `0..=MAX_SPEED_MPS`; culprit raw bits carried.
    BadSpeed(i64),
    /// Spread outside `0..=MAX_SPREAD`; culprit raw bits carried.
    BadSpread(i64),
    /// Aim direction normalizes to zero.
    BadDirection,
    /// A position is outside the coursed range; culprit bits carried.
    BadPosition(i64),
    /// A cast answered past the segment end; answer and length carried.
    BadHitDistance {
        /// Answer distance in mm.
        dist_mm: u32,
        /// Segment length in mm.
        seg_mm: u64,
    },
    /// A checked fixed-point operation overflowed.
    Overflow,
}

impl core::fmt::Display for FlightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => {
                write!(f, "flight table version 0 is reserved ({version})")
            }
            Self::BadValue { field, value } => {
                write!(f, "flight field {field} out of range ({value})")
            }
            Self::UnknownVersion(version) => {
                write!(f, "unknown flight table version ({version})")
            }
            Self::TooManyRounds(len) => write!(f, "too many rounds ({len})"),
            Self::LengthMismatch { rounds, hits } => {
                write!(
                    f,
                    "hit buffer length ({hits}) differs from rounds ({rounds})"
                )
            }
            Self::BadSpeed(bits) => write!(f, "muzzle speed out of range ({bits})"),
            Self::BadSpread(bits) => write!(f, "spread out of range ({bits})"),
            Self::BadDirection => write!(f, "aim direction normalizes to zero"),
            Self::BadPosition(bits) => write!(f, "position out of range ({bits})"),
            Self::BadHitDistance { dist_mm, seg_mm } => {
                write!(f, "cast answered past the segment ({dist_mm} > {seg_mm})")
            }
            Self::Overflow => write!(f, "fixed-point flight math overflowed"),
        }
    }
}

impl std::error::Error for FlightError {}

/// One-tick segment query owned by the caller (scene, fixture or harness).
///
/// The stepper hands integer-millimetre endpoints in world space, +Y up; the
/// caster answers the nearest hit with `dist_mm` inside the segment, or
/// `None` for a clean miss. Implementations must be deterministic and must
/// not allocate on the tick path.
pub trait SegmentCast {
    /// Cast the segment; the nearest hit inside it, if any.
    fn cast(&self, from_mm: [i32; 3], to_mm: [i32; 3]) -> Option<CastHit>;
}

/// A stopped round: where along the segment it stopped and what it met.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CastHit {
    /// Distance from the segment start in mm, inside the segment.
    pub dist_mm: u32,
    /// Caller material id at the hit.
    pub material: u16,
}

/// Harness cast that never hits; the vacuum-flight oracle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct NullCaster;

impl SegmentCast for NullCaster {
    fn cast(&self, _from_mm: [i32; 3], _to_mm: [i32; 3]) -> Option<CastHit> {
        None
    }
}

/// Harness cast stopping at one axis-aligned wall plane.
///
/// The plane is `x == wall_mm`; a segment with endpoints on opposite sides
/// of it (or touching it) answers the crossing distance with `material`,
/// else `None`. The distance is `|wall - from.x|`, always inside the
/// segment on a crossing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WallCaster {
    /// Wall plane `x` in mm.
    pub wall_mm: i32,
    /// Material id reported on a hit.
    pub material: u16,
}

impl SegmentCast for WallCaster {
    fn cast(&self, from_mm: [i32; 3], to_mm: [i32; 3]) -> Option<CastHit> {
        let crossed = (from_mm[0] < self.wall_mm) != (to_mm[0] < self.wall_mm);
        if !crossed {
            return None;
        }
        let gap = i64::from(self.wall_mm) - i64::from(from_mm[0]);
        let dist = gap.checked_abs()?;
        u32::try_from(dist).ok().map(|dist_mm| CastHit {
            dist_mm,
            material: self.material,
        })
    }
}

/// One round in flight: fixed-point state stepped one segment per tick.
///
/// Positions are metres, velocities metres per second, both `Fixed` Q16;
/// +Y is up. `alive` is false once a cast stops the round or `ttl_ticks`
/// expires; spent rounds step quietly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Round {
    /// Position in metres, +Y up.
    pub pos: [Fixed; 3],
    /// Velocity in m/s.
    pub vel: [Fixed; 3],
    /// Ticks stepped so far.
    pub age_ticks: u32,
    /// Ticks of life; `1..=MAX_FLIGHT_TICKS`, set at spawn.
    pub ttl_ticks: u32,
    /// False once a cast stops the round or the life runs out.
    pub alive: bool,
}

/// Spawn inputs for one round; more than seven fields, so one struct.
///
/// The `(tick, shooter, shot)` triple seeds the spread draw: the same
/// triple gives the same perturbation, a different triple a different one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpawnParams {
    /// Tick that fires the round; part of the spread seed.
    pub tick: u64,
    /// Shooter slot; part of the spread seed.
    pub shooter: u32,
    /// Shot counter per shooter; part of the spread seed.
    pub shot: u32,
    /// Muzzle position in metres, each axis within `MAX_ORIGIN_M`.
    pub origin: [Fixed; 3],
    /// Aim direction, nonzero, scale-free; spread perturbs around it.
    pub dir: [Fixed; 3],
    /// Muzzle speed in m/s, `0..=MAX_SPEED_MPS`.
    pub speed: Fixed,
    /// Spread as tangent of the half-angle, `0..=MAX_SPREAD`.
    pub spread: Fixed,
    /// Ticks of life, `1..=MAX_FLIGHT_TICKS`.
    pub ttl_ticks: u32,
}

/// Spawn one round: validate, normalize the aim, fork the spread.
///
/// A zero spread flies exactly along the normalized aim; a nonzero spread
/// perturbs it by two seeded perpendicular offsets and renormalizes.
/// Never panics on user data.
///
/// ```rust
/// use engine_combat::projectile::{spawn, FlightTableSet, NullCaster, SpawnParams};
/// use engine_core::fixed::Fixed;
///
/// let params = SpawnParams {
///     tick: 7,
///     shooter: 1,
///     shot: 0,
///     origin: [Fixed::ZERO; 3],
///     dir: [Fixed::ONE, Fixed::ZERO, Fixed::ZERO],
///     speed: Fixed::from_int(100),
///     spread: Fixed::ZERO,
///     ttl_ticks: 600,
/// };
/// let round = spawn(&params).unwrap();
/// assert!(round.alive);
/// assert_eq!(round.vel[0], Fixed::from_int(100));
/// ```
pub fn spawn(params: &SpawnParams) -> Result<Round, FlightError> {
    for component in params.origin {
        if component.abs() > MAX_ORIGIN_M {
            return Err(FlightError::BadPosition(component.to_bits()));
        }
    }
    if params.speed < Fixed::ZERO || params.speed > MAX_SPEED_MPS {
        return Err(FlightError::BadSpeed(params.speed.to_bits()));
    }
    if params.spread < Fixed::ZERO || params.spread > MAX_SPREAD {
        return Err(FlightError::BadSpread(params.spread.to_bits()));
    }
    if params.ttl_ticks == 0 || params.ttl_ticks > MAX_FLIGHT_TICKS {
        return Err(FlightError::BadValue {
            field: "ttl_ticks",
            value: i64::from(params.ttl_ticks),
        });
    }
    let aim = normalize(params.dir).ok_or(FlightError::BadDirection)?;
    let biased = apply_spread(aim, params.spread, params.tick, params.shooter, params.shot)?;
    let mut vel = [Fixed::ZERO; 3];
    let mut index = 0;
    while index < vel.len() {
        vel[index] = biased[index]
            .checked_mul(params.speed)
            .ok_or(FlightError::Overflow)?;
        index += 1;
    }
    Ok(Round {
        pos: params.origin,
        vel,
        age_ticks: 0,
        ttl_ticks: params.ttl_ticks,
        alive: true,
    })
}

/// Step one round one tick: drag, gravity, integrate, then cast.
///
/// Returns the stopping hit, if any. A spent round is a silent no-op
/// returning `None`. A hit or an expired `ttl_ticks` retires the round;
/// the retired position is the impact point on a hit. Never panics on
/// user data; checked math reports [`FlightError::Overflow`].
pub fn step_round<C: SegmentCast>(
    round: &mut Round,
    tables: &FlightTables,
    caster: &C,
) -> Result<Option<CastHit>, FlightError> {
    if !round.alive {
        return Ok(None);
    }
    let next_vel = integrate_velocity(round.vel, tables)?;
    // Semi-implicit order: fresh velocity moves the position.
    let next_pos = integrate_position(round.pos, next_vel)?;
    let from_mm = to_mm_triple(round.pos)?;
    let to_mm = to_mm_triple(next_pos)?;
    let hit = if from_mm == to_mm {
        None
    } else {
        caster.cast(from_mm, to_mm)
    };
    // The landing point resolves before any field moves, so a refused cast
    // answer leaves the round untouched.
    let landed = match hit {
        Some(found) => impact_point(from_mm, to_mm, found.dist_mm)?,
        None => next_pos,
    };
    round.vel = next_vel;
    round.pos = landed;
    if hit.is_some() {
        round.alive = false;
    }
    round.age_ticks = round.age_ticks.saturating_add(1);
    if round.age_ticks >= round.ttl_ticks {
        round.alive = false;
    }
    Ok(hit)
}

/// Step every round one tick into the caller hit buffer.
///
/// `hits` must match `rounds` in length; both are borrowed and nothing
/// allocates. `TooManyRounds` is reported before `LengthMismatch`,
/// deterministically.
pub fn step_rounds<C: SegmentCast>(
    rounds: &mut [Round],
    tables: &FlightTables,
    caster: &C,
    hits: &mut [Option<CastHit>],
) -> Result<(), FlightError> {
    if rounds.len() > MAX_ROUNDS_PER_TICK {
        return Err(FlightError::TooManyRounds(rounds.len()));
    }
    if hits.len() != rounds.len() {
        return Err(FlightError::LengthMismatch {
            rounds: rounds.len(),
            hits: hits.len(),
        });
    }
    for (round, slot) in rounds.iter_mut().zip(hits.iter_mut()) {
        *slot = step_round(round, tables, caster)?;
    }
    Ok(())
}

/// Aggregate report for one production volley step; counts only, no heap.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct VolleyReport {
    /// Rounds stepped this call.
    pub stepped: usize,
    /// Hits reported this call.
    pub stopped: usize,
    /// Rounds still alive after the step.
    pub alive: usize,
}

/// Step a caller-owned volley one tick: the production consumer shape.
///
/// Wraps [`step_rounds`] over the caller round and hit buffers, then folds
/// the aggregate into `report` (overwritten, never grown). Bounds and
/// mismatch failures come from [`step_rounds`] before any round moves; on
/// failure `report` is left untouched. Slices in, slices out, no heap on
/// the tick path.
pub fn step_volley<C: SegmentCast>(
    rounds: &mut [Round],
    tables: &FlightTables,
    caster: &C,
    hits: &mut [Option<CastHit>],
    report: &mut VolleyReport,
) -> Result<(), FlightError> {
    step_rounds(rounds, tables, caster, hits)?;
    let mut stopped: usize = 0;
    let mut alive: usize = 0;
    let mut index = 0;
    while index < rounds.len() {
        if hits[index].is_some() {
            stopped = stopped.saturating_add(1);
        }
        if rounds[index].alive {
            alive = alive.saturating_add(1);
        }
        index += 1;
    }
    *report = VolleyReport {
        stepped: rounds.len(),
        stopped,
        alive,
    };
    Ok(())
}

/// Hash every round's full state in order: positions, velocities, ages,
/// lives and remaining lives. Same rounds give the same digest.
pub fn hash_rounds(rounds: &[Round]) -> u64 {
    let mut hasher = Fnv64::new();
    for round in rounds {
        for component in round.pos.iter().chain(round.vel.iter()) {
            hasher.write_i64(component.to_bits());
        }
        hasher.write_u32(round.age_ticks);
        hasher.write_u32(round.ttl_ticks);
        hasher.write_bool(round.alive);
    }
    hasher.finish()
}

/// Unit vector, or `None` when the input is zero (to Q16 rounding).
fn normalize(vec: [Fixed; 3]) -> Option<[Fixed; 3]> {
    let square = vec[0]
        .checked_mul(vec[0])?
        .checked_add(vec[1].checked_mul(vec[1])?)?
        .checked_add(vec[2].checked_mul(vec[2])?)?;
    let len = square.sqrt();
    if len == Fixed::ZERO {
        return None;
    }
    Some([
        vec[0].checked_div(len)?,
        vec[1].checked_div(len)?,
        vec[2].checked_div(len)?,
    ])
}

/// Cross product with overflow reported as `None`.
fn cross(left: [Fixed; 3], right: [Fixed; 3]) -> Option<[Fixed; 3]> {
    Some([
        left[1]
            .checked_mul(right[2])?
            .checked_sub(left[2].checked_mul(right[1])?)?,
        left[2]
            .checked_mul(right[0])?
            .checked_sub(left[0].checked_mul(right[2])?)?,
        left[0]
            .checked_mul(right[1])?
            .checked_sub(left[1].checked_mul(right[0])?)?,
    ])
}

/// Map a stream word to `[-1, 1)` in Q16 without a numeric cast.
fn unit_offset(word: u32) -> Fixed {
    Fixed::from_bits((i64::from(word) - (1i64 << 31)) / 32768)
}

/// Perturb a unit aim by two seeded perpendicular offsets, then renormalize.
///
/// The stream forks from `(tick, shooter, shot)` under a fixed domain tag,
/// so the same triple draws the same offsets on every platform. Offsets use
/// a cross-product basis, never angles: no transcendental ever runs. A zero
/// spread returns the aim unchanged.
fn apply_spread(
    dir: [Fixed; 3],
    spread: Fixed,
    tick: u64,
    shooter: u32,
    shot: u32,
) -> Result<[Fixed; 3], FlightError> {
    if spread == Fixed::ZERO {
        return Ok(dir);
    }
    let mixed = hash_u64s(&[tick, u64::from(shooter), u64::from(shot)]);
    let mut rng = Rng::new(SPREAD_DOMAIN).fork(mixed);
    let off_x = unit_offset(rng.next_u32())
        .checked_mul(spread)
        .ok_or(FlightError::Overflow)?;
    let off_y = unit_offset(rng.next_u32())
        .checked_mul(spread)
        .ok_or(FlightError::Overflow)?;
    let abs = [dir[0].abs(), dir[1].abs(), dir[2].abs()];
    let axis = if abs[0] <= abs[1] && abs[0] <= abs[2] {
        0
    } else if abs[1] <= abs[2] {
        1
    } else {
        2
    };
    let mut helper = [Fixed::ZERO; 3];
    helper[axis] = Fixed::ONE;
    let side = cross(dir, helper).ok_or(FlightError::Overflow)?;
    let tangent = normalize(side).ok_or(FlightError::Overflow)?;
    let bitangent = cross(dir, tangent).ok_or(FlightError::Overflow)?;
    let mut bent = [Fixed::ZERO; 3];
    let mut index = 0;
    while index < bent.len() {
        let push = tangent[index]
            .checked_mul(off_x)
            .and_then(|along| {
                bitangent[index]
                    .checked_mul(off_y)
                    .and_then(|across| along.checked_add(across))
            })
            .ok_or(FlightError::Overflow)?;
        bent[index] = dir[index].checked_add(push).ok_or(FlightError::Overflow)?;
        index += 1;
    }
    normalize(bent).ok_or(FlightError::Overflow)
}

/// Speed as the `sqrt` of the squared velocity; overflow is reported.
fn speed_of(vel: [Fixed; 3]) -> Result<Fixed, FlightError> {
    let square = vel[0]
        .checked_mul(vel[0])
        .and_then(|xx| vel[1].checked_mul(vel[1]).and_then(|yy| xx.checked_add(yy)))
        .and_then(|xxyy| {
            vel[2]
                .checked_mul(vel[2])
                .and_then(|zz| xxyy.checked_add(zz))
        })
        .ok_or(FlightError::Overflow)?;
    Ok(square.sqrt())
}

/// Semi-implicit Euler velocity: shed `vel * (lin + quad * speed) * dt`,
/// then sink by `gravity * dt` on Y. Only `+ - * /` and `sqrt` run.
///
/// The quadratic term alone reads the speed, so drag-free and linear-only
/// tables skip the root entirely; the elided product is exactly zero, so
/// the fast path steps bit-identical velocities to the general one.
fn integrate_velocity(vel: [Fixed; 3], tables: &FlightTables) -> Result<[Fixed; 3], FlightError> {
    let loss = if tables.quad_drag_per_m == Fixed::ZERO {
        tables
            .linear_drag_per_s
            .checked_mul(DT)
            .ok_or(FlightError::Overflow)?
    } else {
        let speed = speed_of(vel)?;
        tables
            .quad_drag_per_m
            .checked_mul(speed)
            .and_then(|quad| quad.checked_add(tables.linear_drag_per_s))
            .and_then(|drag| drag.checked_mul(DT))
            .ok_or(FlightError::Overflow)?
    };
    let sink = tables
        .gravity_mps2
        .checked_mul(DT)
        .ok_or(FlightError::Overflow)?;
    let shed = shed_velocity(vel, loss)?;
    let down = shed[1].checked_sub(sink).ok_or(FlightError::Overflow)?;
    Ok([shed[0], down, shed[2]])
}

/// `vel - vel * loss` component-wise with overflow reported.
fn shed_velocity(vel: [Fixed; 3], loss: Fixed) -> Result<[Fixed; 3], FlightError> {
    let mut out = [Fixed::ZERO; 3];
    let mut index = 0;
    while index < out.len() {
        let shed = vel[index].checked_mul(loss).ok_or(FlightError::Overflow)?;
        out[index] = vel[index].checked_sub(shed).ok_or(FlightError::Overflow)?;
        index += 1;
    }
    Ok(out)
}

/// Semi-implicit Euler position: `pos + vel * dt` after the velocity step.
fn integrate_position(pos: [Fixed; 3], vel: [Fixed; 3]) -> Result<[Fixed; 3], FlightError> {
    let mut out = [Fixed::ZERO; 3];
    let mut index = 0;
    while index < out.len() {
        let step = vel[index].checked_mul(DT).ok_or(FlightError::Overflow)?;
        out[index] = pos[index].checked_add(step).ok_or(FlightError::Overflow)?;
        index += 1;
    }
    Ok(out)
}

/// `Fixed` metres to integer millimetres; overflow is reported.
fn fixed_to_mm(value: Fixed) -> Result<i32, FlightError> {
    let bits = value.to_bits();
    let scaled = bits.checked_mul(1000).ok_or(FlightError::Overflow)?;
    i32::try_from(scaled / 65536).map_err(|_| FlightError::BadPosition(bits))
}

/// Integer millimetres to `Fixed` metres; overflow is reported.
fn mm_to_fixed(mm: i32) -> Result<Fixed, FlightError> {
    let scaled = i64::from(mm)
        .checked_mul(65536)
        .ok_or(FlightError::Overflow)?;
    Ok(Fixed::from_bits(scaled / 1000))
}

/// One triple from metres to integer millimetres for the cast.
fn to_mm_triple(pos: [Fixed; 3]) -> Result<[i32; 3], FlightError> {
    Ok([
        fixed_to_mm(pos[0])?,
        fixed_to_mm(pos[1])?,
        fixed_to_mm(pos[2])?,
    ])
}

/// Segment length in mm by integer root-sum-of-squares (`std::isqrt`).
fn seg_len_mm(from_mm: [i32; 3], to_mm: [i32; 3]) -> Result<u64, FlightError> {
    let mut sum: u128 = 0;
    let mut index = 0;
    while index < 3 {
        let delta = i64::from(to_mm[index]) - i64::from(from_mm[index]);
        let mag = delta.checked_abs().ok_or(FlightError::Overflow)?;
        let mag_wide = u128::try_from(mag).map_err(|_| FlightError::Overflow)?;
        sum += mag_wide * mag_wide;
        index += 1;
    }
    u64::try_from(sum.isqrt()).map_err(|_| FlightError::Overflow)
}

/// Impact point `dist_mm` along the segment, back in `Fixed` metres.
///
/// Answers past the segment end are refused; the point arithmetic stays in
/// `i128` so only a truly out-of-range answer reports `BadPosition`.
fn impact_point(
    from_mm: [i32; 3],
    to_mm: [i32; 3],
    dist_mm: u32,
) -> Result<[Fixed; 3], FlightError> {
    let seg = seg_len_mm(from_mm, to_mm)?;
    if u64::from(dist_mm) > seg {
        return Err(FlightError::BadHitDistance {
            dist_mm,
            seg_mm: seg,
        });
    }
    let mut out = [Fixed::ZERO; 3];
    let mut index = 0;
    while index < 3 {
        let delta = i64::from(to_mm[index]) - i64::from(from_mm[index]);
        let scaled = i128::from(delta) * i128::from(dist_mm);
        let advance = scaled / i128::from(seg);
        let point = i128::from(from_mm[index]) + advance;
        let point_mm = i32::try_from(point)
            .map_err(|_| FlightError::BadPosition(if point < 0 { i64::MIN } else { i64::MAX }))?;
        out[index] = mm_to_fixed(point_mm)?;
        index += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::tick::TICK_HZ;

    /// Flat-fire spawn helper: due +X at the origin with no spread.
    fn flat(tick: u64, shooter: u32, shot: u32, speed_mps: i64, ttl: u32) -> SpawnParams {
        SpawnParams {
            tick,
            shooter,
            shot,
            origin: [Fixed::ZERO; 3],
            dir: [Fixed::ONE, Fixed::ZERO, Fixed::ZERO],
            speed: Fixed::from_int(speed_mps),
            spread: Fixed::ZERO,
            ttl_ticks: ttl,
        }
    }

    /// Step one round `ticks` times through the vacuum harness.
    fn fly(params: &SpawnParams, tables: &FlightTables, ticks: u32) -> Round {
        let mut round = spawn(params).unwrap();
        let caster = NullCaster;
        for _ in 0..ticks {
            let hit = step_round(&mut round, tables, &caster).unwrap();
            assert_eq!(hit, None);
        }
        round
    }

    /// Discrete vacuum drop in mm for the semi-implicit stepper:
    /// `g * dt² * n * (n + 1) / 2` in integer math. Velocity steps before
    /// position, so the fall sums 1..=n; analytic `n² / 2` sits 0.8% low
    /// at 120 ticks and hid the Euler-order bias.
    fn vacuum_drop_mm(gravity: Fixed, ticks: u32) -> i64 {
        let wide = i128::from(gravity.to_bits())
            * i128::from(ticks)
            * (i128::from(ticks) + 1)
            * i128::from(DT.to_bits())
            * i128::from(DT.to_bits());
        let bits = wide / (2 * i128::from(Fixed::ONE.to_bits()) * i128::from(Fixed::ONE.to_bits()));
        i64::try_from(bits * 1000 / i128::from(Fixed::ONE.to_bits())).unwrap()
    }

    #[test]
    fn tick_step_matches_engine_rate() {
        assert_eq!(u64::try_from(TICK_HZ_RAW).unwrap(), TICK_HZ);
        assert_eq!(DT.to_bits(), Fixed::ONE.to_bits() / TICK_HZ_RAW);
    }

    #[test]
    fn shipped_tables_validate_and_resolve() {
        assert_eq!(FLIGHT_VERSION_V1, 1);
        assert_eq!(FLIGHT_V1, FlightTables::v1());
        assert_eq!(FlightTableSet::v1(), FlightTableSet::new(1, FLIGHT_V1));
        assert_eq!(validate_flight_tables(&FlightTableSet::v1()), Ok(()));
        assert_eq!(flight_for_version(0), None);
        assert_eq!(flight_for_version(1), Some(&FLIGHT_V1));
        assert_eq!(flight_for_version(7), None);
    }

    #[test]
    fn invalid_tables_rejected_per_rule() {
        let mut rows = FLIGHT_V1;
        assert_eq!(
            validate_flight_tables(&FlightTableSet::new(0, rows)),
            Err(FlightError::BadVersion(0))
        );
        rows.gravity_mps2 = Fixed::from_int(-1);
        assert!(matches!(
            validate_flight_tables(&FlightTableSet::new(2, rows)),
            Err(FlightError::BadValue {
                field: "gravity_mps2",
                ..
            })
        ));
        rows = FLIGHT_V1;
        rows.gravity_mps2 = Fixed::from_int(33);
        assert!(matches!(
            validate_flight_tables(&FlightTableSet::new(2, rows)),
            Err(FlightError::BadValue {
                field: "gravity_mps2",
                ..
            })
        ));
        rows = FLIGHT_V1;
        rows.linear_drag_per_s = Fixed::from_int(9);
        assert!(matches!(
            validate_flight_tables(&FlightTableSet::new(2, rows)),
            Err(FlightError::BadValue {
                field: "linear_drag_per_s",
                ..
            })
        ));
        rows = FLIGHT_V1;
        rows.quad_drag_per_m = Fixed::from_int(5);
        assert!(matches!(
            validate_flight_tables(&FlightTableSet::new(2, rows)),
            Err(FlightError::BadValue {
                field: "quad_drag_per_m",
                ..
            })
        ));
    }

    #[test]
    fn spawn_rejects_bad_inputs() {
        let good = flat(0, 0, 0, 100, 600);
        assert!(spawn(&good).is_ok());
        let mut bad = good;
        bad.dir = [Fixed::ZERO; 3];
        assert_eq!(spawn(&bad), Err(FlightError::BadDirection));
        bad = good;
        bad.speed = Fixed::from_int(2001);
        assert!(matches!(spawn(&bad), Err(FlightError::BadSpeed(_))));
        bad = good;
        bad.spread = Fixed::from_bits(16385);
        assert!(matches!(spawn(&bad), Err(FlightError::BadSpread(_))));
        bad = good;
        bad.origin = [Fixed::from_int(4001), Fixed::ZERO, Fixed::ZERO];
        assert!(matches!(spawn(&bad), Err(FlightError::BadPosition(_))));
        bad = good;
        bad.ttl_ticks = 0;
        assert!(matches!(
            spawn(&bad),
            Err(FlightError::BadValue {
                field: "ttl_ticks",
                ..
            })
        ));
        bad = good;
        bad.ttl_ticks = MAX_FLIGHT_TICKS + 1;
        assert!(matches!(
            spawn(&bad),
            Err(FlightError::BadValue {
                field: "ttl_ticks",
                ..
            })
        ));
    }

    #[test]
    fn zero_spread_flies_true() {
        let round = spawn(&flat(3, 1, 2, 100, 600)).unwrap();
        assert_eq!(round.vel[0], Fixed::from_int(100));
        assert_eq!(round.vel[1], Fixed::ZERO);
        assert_eq!(round.vel[2], Fixed::ZERO);
        assert!(round.alive);
    }

    #[test]
    fn spread_seeded_per_triple() {
        let spread = Fixed::from_bits(8192);
        let aimed = |tick: u64, shooter: u32, shot: u32| {
            let mut params = flat(tick, shooter, shot, 100, 600);
            params.spread = spread;
            spawn(&params).unwrap().vel
        };
        let first = aimed(11, 2, 5);
        assert_eq!(first, aimed(11, 2, 5));
        assert_ne!(first, aimed(11, 2, 6));
        assert_ne!(first, aimed(11, 3, 5));
        assert_ne!(first, aimed(12, 2, 5));
        // A spread round still leaves near full speed: the bend changes the
        // aim, not the muzzle energy (within fixed-point rounding, 64 bits
        // is a thousandth of a m/s here).
        let speed = speed_of(first).unwrap();
        let drift = (speed.to_bits() - Fixed::from_int(100).to_bits()).abs();
        assert!(drift <= 64);
    }

    #[test]
    fn thousand_shot_range_hash_replays() {
        let tables = FLIGHT_V1;
        let run = || {
            let mut rounds: Vec<Round> = Vec::new();
            for shot in 0..1000u32 {
                let params = flat(u64::from(shot), shot % 16, shot, 120, 240);
                rounds.push(spawn(&params).unwrap());
            }
            let caster = NullCaster;
            let mut hits: Vec<Option<CastHit>> = Vec::new();
            hits.resize(rounds.len(), None);
            for _ in 0..120 {
                step_rounds(&mut rounds, &tables, &caster, &mut hits).unwrap();
            }
            hash_rounds(&rounds)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        assert_ne!(first, Fnv64::new().finish());
    }

    #[test]
    fn drop_matches_discrete_curve_exactly() {
        let tables = FLIGHT_V1;
        let round = fly(&flat(0, 0, 0, 100, 600), &tables, 120);
        let drop_mm = -i64::from(fixed_to_mm(round.pos[1]).unwrap());
        let oracle_mm = vacuum_drop_mm(tables.gravity_mps2, 120);
        assert!(oracle_mm > 1000);
        let gap = (drop_mm - oracle_mm).abs();
        // Millimetre rounding only; the discrete sum is exact.
        assert!(gap <= 2, "drop {drop_mm}mm vs vacuum {oracle_mm}mm");
    }

    #[test]
    fn drag_shortens_range() {
        let damp = FlightTables {
            gravity_mps2: Fixed::ZERO,
            linear_drag_per_s: Fixed::from_bits(32768),
            quad_drag_per_m: Fixed::ZERO,
        };
        assert_eq!(
            validate_flight_tables(&FlightTableSet::new(9, damp)),
            Ok(())
        );
        let params = flat(0, 0, 0, 300, 600);
        let vacuum = fly(&params, &FLIGHT_V1, 120);
        let damped = fly(&params, &damp, 120);
        let reach = fixed_to_mm(vacuum.pos[0]).unwrap();
        let braked = fixed_to_mm(damped.pos[0]).unwrap();
        assert!(reach > 590_000);
        assert!(braked > 300_000);
        assert!(braked < reach);
        // The quadratic path reads the speed root and still brakes.
        let quad = FlightTables {
            gravity_mps2: Fixed::ZERO,
            linear_drag_per_s: Fixed::ZERO,
            quad_drag_per_m: Fixed::from_bits(131),
        };
        assert_eq!(
            validate_flight_tables(&FlightTableSet::new(10, quad)),
            Ok(())
        );
        let dragged = fly(&params, &quad, 120);
        let pulled = fixed_to_mm(dragged.pos[0]).unwrap();
        assert!(pulled > 0);
        assert!(pulled < reach);
        assert_ne!(hash_rounds(&[dragged]), hash_rounds(&[vacuum]));
    }

    #[test]
    fn wall_stops_round_and_vacuum_passes_through() {
        let wall = WallCaster {
            wall_mm: 10000,
            material: 7,
        };
        let params = flat(0, 0, 0, 100, 600);
        let mut round = spawn(&params).unwrap();
        let mut hit: Option<CastHit> = None;
        let mut before_mm = 0;
        for _ in 0..60 {
            before_mm = fixed_to_mm(round.pos[0]).unwrap();
            hit = step_round(&mut round, &FLIGHT_V1, &wall).unwrap();
            if hit.is_some() {
                break;
            }
        }
        let found = hit.unwrap();
        assert_eq!(found.material, 7);
        // The wall answers the exact crossing distance it saw.
        assert_eq!(found.dist_mm, u32::try_from(10000 - before_mm).unwrap());
        assert!(!round.alive);
        // Millimetre truncation moves the landing point at most 2 mm.
        assert!((fixed_to_mm(round.pos[0]).unwrap() - 10000).abs() <= 2);
        assert!(round.pos[1] < Fixed::ZERO);

        let free = fly(&params, &FLIGHT_V1, 7);
        assert!(free.alive);
        assert!(fixed_to_mm(free.pos[0]).unwrap() > 10000);
    }

    #[test]
    fn spent_rounds_retire_quietly() {
        let mut round = spawn(&flat(0, 0, 0, 100, 5)).unwrap();
        let caster = NullCaster;
        for _ in 0..5 {
            assert_eq!(step_round(&mut round, &FLIGHT_V1, &caster).unwrap(), None);
        }
        assert!(!round.alive);
        let frozen = round;
        assert_eq!(step_round(&mut round, &FLIGHT_V1, &caster).unwrap(), None);
        assert_eq!(round, frozen);
    }

    #[test]
    fn step_buffers_reject_mismatch_and_overflow() {
        let tables = FLIGHT_V1;
        let caster = NullCaster;
        let mut rounds: Vec<Round> = Vec::new();
        for shot in 0..4u32 {
            rounds.push(spawn(&flat(0, 0, shot, 50, 60)).unwrap());
        }
        let capacity = rounds.capacity();
        let mut short: Vec<Option<CastHit>> = Vec::new();
        short.resize(3, None);
        assert_eq!(
            step_rounds(&mut rounds, &tables, &caster, &mut short),
            Err(FlightError::LengthMismatch { rounds: 4, hits: 3 })
        );
        let mut hits: Vec<Option<CastHit>> = Vec::new();
        hits.resize(rounds.len(), None);
        assert_eq!(
            step_rounds(&mut rounds, &tables, &caster, &mut hits),
            Ok(())
        );
        assert_eq!(rounds.capacity(), capacity);
        assert!(hits.iter().all(|slot| slot.is_none()));

        let mut swarm: Vec<Round> = Vec::new();
        swarm.resize(MAX_ROUNDS_PER_TICK + 1, rounds[0]);
        let mut swarm_hits: Vec<Option<CastHit>> = Vec::new();
        swarm_hits.resize(swarm.len(), None);
        assert_eq!(
            step_rounds(&mut swarm, &tables, &caster, &mut swarm_hits),
            Err(FlightError::TooManyRounds(MAX_ROUNDS_PER_TICK + 1))
        );
    }

    #[test]
    fn lying_cast_is_refused() {
        struct FarCaster;
        impl SegmentCast for FarCaster {
            fn cast(&self, _from_mm: [i32; 3], _to_mm: [i32; 3]) -> Option<CastHit> {
                Some(CastHit {
                    dist_mm: u32::MAX,
                    material: 1,
                })
            }
        }
        let mut round = spawn(&flat(0, 0, 0, 100, 600)).unwrap();
        let far = FarCaster;
        assert!(matches!(
            step_round(&mut round, &FLIGHT_V1, &far),
            Err(FlightError::BadHitDistance { .. })
        ));
        assert!(round.alive);
    }

    #[test]
    fn volley_report_counts_step_and_stop() {
        let scene = WallCaster {
            wall_mm: 10000,
            material: 7,
        };
        let tables = FLIGHT_V1;
        let mut rounds: Vec<Round> = Vec::new();
        for shot in 0..3u32 {
            // Zero spread: identical trajectories, one shared stopping tick.
            rounds.push(spawn(&flat(0, 0, shot, 100, 600)).unwrap());
        }
        let mut hits: Vec<Option<CastHit>> = Vec::new();
        hits.resize(rounds.len(), None);
        let mut report = VolleyReport::default();
        step_volley(&mut rounds, &tables, &scene, &mut hits, &mut report).unwrap();
        assert_eq!(
            report,
            VolleyReport {
                stepped: 3,
                stopped: 0,
                alive: 3,
            }
        );
        let mut stopping = VolleyReport::default();
        for _ in 0..60 {
            step_volley(&mut rounds, &tables, &scene, &mut hits, &mut stopping).unwrap();
            if stopping.stopped > 0 {
                break;
            }
        }
        assert_eq!(
            stopping,
            VolleyReport {
                stepped: 3,
                stopped: 3,
                alive: 0,
            }
        );
        assert!(hits.iter().all(|slot| slot.is_some()));
        assert!(rounds.iter().all(|round| !round.alive));
    }

    #[test]
    fn volley_rejects_bad_buffers_without_touching_report() {
        let scene = NullCaster;
        let tables = FLIGHT_V1;
        let mut rounds: Vec<Round> = Vec::new();
        for shot in 0..2u32 {
            rounds.push(spawn(&flat(0, 0, shot, 50, 60)).unwrap());
        }
        let before = rounds.clone();
        let mut short: Vec<Option<CastHit>> = Vec::new();
        short.resize(1, None);
        let mut report = VolleyReport {
            stepped: 9,
            stopped: 9,
            alive: 9,
        };
        assert_eq!(
            step_volley(&mut rounds, &tables, &scene, &mut short, &mut report),
            Err(FlightError::LengthMismatch { rounds: 2, hits: 1 })
        );
        assert_eq!(rounds, before);
        assert_eq!(
            report,
            VolleyReport {
                stepped: 9,
                stopped: 9,
                alive: 9,
            }
        );

        let mut swarm: Vec<Round> = Vec::new();
        swarm.resize(MAX_ROUNDS_PER_TICK + 1, rounds[0]);
        let mut swarm_hits: Vec<Option<CastHit>> = Vec::new();
        swarm_hits.resize(swarm.len(), None);
        assert_eq!(
            step_volley(&mut swarm, &tables, &scene, &mut swarm_hits, &mut report),
            Err(FlightError::TooManyRounds(MAX_ROUNDS_PER_TICK + 1))
        );
        assert_eq!(
            report,
            VolleyReport {
                stepped: 9,
                stopped: 9,
                alive: 9,
            }
        );
    }

    #[test]
    fn error_text_names_the_fault() {
        assert_eq!(
            FlightError::BadVersion(0).to_string(),
            "flight table version 0 is reserved (0)"
        );
        assert_eq!(
            FlightError::TooManyRounds(2049).to_string(),
            "too many rounds (2049)"
        );
        assert_eq!(
            FlightError::BadDirection.to_string(),
            "aim direction normalizes to zero"
        );
        assert_eq!(
            FlightError::Overflow.to_string(),
            "fixed-point flight math overflowed"
        );
    }

    #[ignore = "timing: 2048-round step budget, run explicitly under --release"]
    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn step_2048_rounds_within_budget() {
        let tables = FLIGHT_V1;
        let caster = NullCaster;
        let mut pristine: Vec<Round> = Vec::new();
        for shot in 0..u32::try_from(MAX_ROUNDS_PER_TICK).unwrap() {
            pristine.push(spawn(&flat(0, shot % 32, shot, 150, 600)).unwrap());
        }
        let mut probe: Vec<Option<CastHit>> = Vec::new();
        probe.resize(pristine.len(), None);
        for _ in 0..3 {
            let mut warm = pristine.clone();
            step_rounds(&mut warm, &tables, &caster, &mut probe).unwrap();
        }
        let mut best_ns = u64::MAX;
        for _ in 0..7 {
            let mut rounds = pristine.clone();
            let started = std::time::Instant::now();
            step_rounds(&mut rounds, &tables, &caster, &mut probe).unwrap();
            let spent = u64::try_from(started.elapsed().as_nanos()).unwrap();
            if spent < best_ns {
                best_ns = spent;
            }
        }
        assert!(pristine.iter().all(|round| round.alive));
        assert!(
            best_ns <= 300_000,
            "2048 rounds took {best_ns}ns, budget is 300000ns"
        );
    }
}
