//! projectile_hit: one round against the bodies it can meet: the tick it is
//! hit on, as an event (RY3 of `docs/plan/ryl1.md`).
//!
//! Owner: engine-combat, beside [`crate::projectile`], which owns the flight
//! and the cast contract. Consumer: none yet; RY5's match loop resolves its
//! volleys through [`BodyTrack`] over the tick's body state. Data owner: this
//! module; a body's path is data, the round's rows are the flight's
//! [`crate::projectile::FlightTables`]. Units: millimetres, integer ticks at
//! 60 Hz, plus the flight's `Fixed` Q16 metres. Determinism: `i128` integer
//! math and `Fixed` only, no float anywhere, tracks keep their order, so two
//! runs of one shot hit on the same tick on every platform, wasm included.
//!
//! The event is sim-owned data. The render thread reads a [`HitEvent`] and
//! never decides one: no hit tick, no impact point and no hit body are ever
//! computed on the render side.
//!
//! A round is walked tick by tick with the flight's own stepper under a void
//! cast, and each swept segment is tested against every track's body for that
//! tick. The first track the segment touches, in track order, wins. A flight
//! that meets nothing is `Ok(None)`, not a failure. Errors: typed
//! [`HitError`]. Budget: one pass over one flight's ticks times the tracks;
//! no heap, no allocation on the tick path.
//!
//! The segment test is exact integer math: the closest point of a segment to
//! a sphere centre, compared as squared distances in `i128`. A wrong answer
//! would need an overflow, and every product is bounded far below the `i128`
//! ceiling (worst case near 4e34 against 1.7e38).

use engine_core::fixed::Fixed;
use engine_core::hash::Fnv64;

use crate::projectile::{spawn, step_round, FlightError, FlightTables, NullCaster, SpawnParams};

/// Most tracks one shot resolves against; the Tier A slice is 6v6.
pub const MAX_TRACKS: usize = 64;
/// Most samples one scripted track carries; a 10-minute script at one sample
/// per tick is 36,000, so the cap holds it with room.
pub const MAX_SAMPLES: usize = 65_536;
/// Widest acceptd script radius in millimetres (100 m); bounds the squared
/// distance comparison.
pub const MAX_RADIUS_MM: u32 = 100_000;

/// Where a body stands at one instant, millimetres, +Y up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Body {
    /// Centre of the body, integer millimetres.
    pub center_mm: [i32; 3],
    /// Body radius in millimetres; a segment inside this shell is a hit.
    pub radius_mm: u32,
}

/// One body a round can hit. Implementations answer one question: where is
/// the body at absolute `tick`. `None` means the body is not there.
///
/// The live path (RY5) backs this with the tick's body snapshot; the scripted
/// path is [`ScriptTrack`]. Implementations must be deterministic and must
/// not allocate on the tick path.
pub trait BodyTrack {
    /// This body's id; quoted by every event and refusal.
    fn id(&self) -> u32;

    /// The body at absolute `tick`, or `None` when it is not there.
    fn body_at(&self, tick: u64) -> Option<Body>;
}

/// One sample of a scripted path: where the centre stands at `tick`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrackSample {
    /// Absolute tick this sample holds from, until the next one.
    pub tick: u64,
    /// Centre of the body at and after `tick`, integer millimetres.
    pub center_mm: [i32; 3],
}

/// A scripted body: a radius and a tick-ordered path, held between samples.
///
/// The path steps, never interpolates: a body stands at its last sample until
/// the next tick-ordered sample, so a script sampled every tick draws an
/// exact path and a script sampled sparsely draws a staircase. Both are
/// deterministic, and no division enters the answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScriptTrack {
    /// This body's id.
    pub id: u32,
    /// Body radius in millimetres.
    pub radius_mm: u32,
    /// Tick-ordered samples; the first tick is when the body appears.
    pub samples: Vec<TrackSample>,
}

impl BodyTrack for ScriptTrack {
    fn id(&self) -> u32 {
        self.id
    }

    fn body_at(&self, tick: u64) -> Option<Body> {
        // The last sample at or before `tick`; binary search over a sorted
        // path keeps a long script linear in the flight's ticks, not quadratic.
        let index = self.samples.partition_point(|sample| sample.tick <= tick);
        let sample = self.samples.get(index.checked_sub(1)?)?;
        Some(Body {
            center_mm: sample.center_mm,
            radius_mm: self.radius_mm,
        })
    }
}

/// The hit one round made: the body, the tick it was met on, and where.
///
/// `hit_tick` is the tick the round's segment was flown in: the round left
/// `point_mm`'s segment start at the top of that tick and met the body before
/// its end. The render thread reads this and decides nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HitEvent {
    /// The body that was hit, as [`BodyTrack::id`].
    pub track: u32,
    /// Absolute tick the segment was flown in.
    pub hit_tick: u64,
    /// Closest point of the swept segment to the body centre, millimetres.
    pub point_mm: [i32; 3],
}

/// Why a shot was refused. A flight that hits nothing is not one of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HitError {
    /// The flight itself refused the spawn or a step.
    Flight(FlightError),
    /// No track handed in: a shot resolves against at least one body.
    NoTracks,
    /// More than [`MAX_TRACKS`] tracks.
    TooManyTracks(usize),
    /// A track with no samples.
    EmptyTrack(u32),
    /// A track whose samples are not strictly tick-ordered.
    UnsortedTrack(u32),
    /// A track whose radius is zero or past [`MAX_RADIUS_MM`].
    BadRadius(u32),
}

impl std::fmt::Display for HitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HitError::Flight(error) => write!(f, "flight refused the shot: {error}"),
            HitError::NoTracks => write!(f, "a shot resolves against at least one track"),
            HitError::TooManyTracks(count) => {
                write!(f, "{count} tracks exceeds the {MAX_TRACKS} track cap")
            }
            HitError::EmptyTrack(id) => write!(f, "track {id} carries no samples"),
            HitError::UnsortedTrack(id) => write!(f, "track {id} samples are not tick-ordered"),
            HitError::BadRadius(id) => {
                write!(f, "track {id} radius is zero or past {MAX_RADIUS_MM} mm")
            }
        }
    }
}

impl std::error::Error for HitError {}

/// Validate one track before a shot is resolved against it.
///
/// # Errors
/// [`HitError::EmptyTrack`], [`HitError::UnsortedTrack`] or
/// [`HitError::BadRadius`], first failure in track order.
fn validate_track(track: &impl BodyTrack) -> Result<(), HitError> {
    // The radius question is only answerable for a track that says where it
    // stands; a live track with no body yet is legal (its radius is read at
    // the hit tick), so a missing body never refuses a shot.
    if let Some(body) = track.body_at(u64::MAX) {
        if body.radius_mm == 0 || body.radius_mm > MAX_RADIUS_MM {
            return Err(HitError::BadRadius(track.id()));
        }
    }
    Ok(())
}

/// Validate a scripted track's path: non-empty, tick-ordered, radius sane.
///
/// # Errors
/// [`HitError::EmptyTrack`], [`HitError::UnsortedTrack`] or
/// [`HitError::BadRadius`], first failure in sample order.
pub fn validate_script(script: &ScriptTrack) -> Result<(), HitError> {
    if script.samples.is_empty() {
        return Err(HitError::EmptyTrack(script.id));
    }
    if script.samples.len() > MAX_SAMPLES {
        return Err(HitError::TooManyTracks(script.samples.len()));
    }
    if script.radius_mm == 0 || script.radius_mm > MAX_RADIUS_MM {
        return Err(HitError::BadRadius(script.id));
    }
    for (index, sample) in script.samples.iter().enumerate() {
        if index > 0 && script.samples[index - 1].tick >= sample.tick {
            return Err(HitError::UnsortedTrack(script.id));
        }
    }
    Ok(())
}

/// Fixed-point metres to integer millimetres, truncating toward zero.
///
/// The flight carries its own conversion for its cast contract; this is the
/// resolver's, and both are pure integer shifts, so a position never lands on
/// a different millimetre on another platform.
fn to_mm(pos: [Fixed; 3]) -> Result<[i32; 3], FlightError> {
    let mut mm = [0i32; 3];
    for axis in 0..3 {
        let bits = pos[axis].to_bits();
        let value = (bits * 1000) >> 16;
        mm[axis] = i32::try_from(value).map_err(|_| FlightError::BadPosition(bits))?;
    }
    Ok(mm)
}

/// Closest point of segment `a..b` to `c`, or `None` when the sphere misses.
///
/// Exact integer math, no float: the projection parameter is carried as a
/// rational `t_num / t_den` and the distance comparison as squared distances
/// scaled by the denominator, all in `i128`. `None` is a clean miss, never an
/// overflow answer: every product stays far below the `i128` ceiling.
fn segment_sphere(a: [i32; 3], b: [i32; 3], c: [i32; 3], radius_mm: u32) -> Option<[i32; 3]> {
    let a = a.map(i64::from);
    let b = b.map(i64::from);
    let c = c.map(i64::from);
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let dot = |x: [i64; 3], y: [i64; 3]| -> i128 {
        i128::from(x[0]) * i128::from(y[0])
            + i128::from(x[1]) * i128::from(y[1])
            + i128::from(x[2]) * i128::from(y[2])
    };
    let len_sq = dot(d, d);
    let radius_sq = i128::from(radius_mm) * i128::from(radius_mm);
    let mut closest = a;
    let hit;
    if len_sq == 0 {
        // a spent segment: the sphere test is a point test
        hit = dot(w, w) <= radius_sq;
    } else {
        let along = dot(w, d);
        if along <= 0 {
            hit = dot(w, w) <= radius_sq;
        } else if along >= len_sq {
            let tail = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
            hit = dot(tail, tail) <= radius_sq;
            closest = b;
        } else {
            // the projection lands inside the segment: scale the comparison
            // by the denominator instead of dividing
            hit = dot(w, w) * len_sq - along * along <= radius_sq * len_sq;
            let mut point = [0i128; 3];
            for axis in 0..3 {
                point[axis] = i128::from(a[axis]) + i128::from(d[axis]) * along / len_sq;
            }
            for axis in 0..3 {
                closest[axis] = i64::try_from(point[axis]).ok()?;
            }
        }
    }
    if !hit {
        return None;
    }
    let mut out = [0i32; 3];
    for axis in 0..3 {
        out[axis] = i32::try_from(closest[axis]).ok()?;
    }
    Some(out)
}

/// Resolve one shot against the tracks it can meet.
///
/// The round is spawned from `params` and walked with
/// [`crate::projectile::step_round`] under a void cast, so nothing but a body
/// stops it. Each tick's swept segment is tested against every track's body
/// for that absolute tick; the first touch in track order is the event.
///
/// Returns `Ok(None)` when the flight ends without meeting a body, which is a
/// clean miss and not an error.
///
/// # Errors
/// [`HitError`] on a refused spawn, an empty or oversized track list, or a
/// malformed script.
///
/// ```rust
/// use engine_combat::projectile::{FlightTableSet, FlightTables, SpawnParams};
/// use engine_combat::projectile_hit::{resolve_shot, ScriptTrack, TrackSample};
/// use engine_core::fixed::Fixed;
///
/// // a body standing 20 m down the +X axis for the whole flight
/// let track = ScriptTrack {
///     id: 7,
///     radius_mm: 500,
///     samples: (0..120)
///         .map(|tick| TrackSample {
///             tick,
///             center_mm: [20_000, 0, 0],
///         })
///         .collect(),
/// };
/// let set = FlightTableSet::new(9, FlightTables {
///     gravity_mps2: Fixed::ZERO,
///     linear_drag_per_s: Fixed::ZERO,
///     quad_drag_per_m: Fixed::ZERO,
/// });
/// let params = SpawnParams {
///     tick: 100,
///     shooter: 3,
///     shot: 0,
///     origin: [Fixed::ZERO; 3],
///     dir: [Fixed::ONE, Fixed::ZERO, Fixed::ZERO],
///     speed: Fixed::from_int(100),
///     spread: Fixed::ZERO,
///     ttl_ticks: 600,
/// };
/// let hit = resolve_shot(&params, &set.tables, &[track]).unwrap().unwrap();
/// assert_eq!(hit.track, 7);
/// ```
pub fn resolve_shot<T: BodyTrack>(
    params: &SpawnParams,
    tables: &FlightTables,
    tracks: &[T],
) -> Result<Option<HitEvent>, HitError> {
    if tracks.is_empty() {
        return Err(HitError::NoTracks);
    }
    if tracks.len() > MAX_TRACKS {
        return Err(HitError::TooManyTracks(tracks.len()));
    }
    for track in tracks {
        validate_track(track)?;
    }
    let mut round = spawn(params).map_err(HitError::Flight)?;
    while round.alive {
        // the segment this tick flies: where the round stands now, and where
        // the flight's stepper puts it by the end of the tick
        let from_mm = to_mm(round.pos).map_err(HitError::Flight)?;
        let hit_tick = params.tick.saturating_add(u64::from(round.age_ticks));
        step_round(&mut round, tables, &NullCaster).map_err(HitError::Flight)?;
        let to_mm = to_mm(round.pos).map_err(HitError::Flight)?;
        for track in tracks {
            let Some(body) = track.body_at(hit_tick) else {
                continue;
            };
            if let Some(point_mm) = segment_sphere(from_mm, to_mm, body.center_mm, body.radius_mm) {
                return Ok(Some(HitEvent {
                    track: track.id(),
                    hit_tick,
                    point_mm,
                }));
            }
        }
    }
    Ok(None)
}

/// Hash every event in order: track, tick and point, plus the count.
///
/// The match hash (RY5) folds one volley's events this way, so a hit that
/// only moved by one millimetre still moves the hash.
pub fn hash_hits(events: &[HitEvent]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_len(events.len());
    for event in events {
        hasher.write_u32(event.track);
        hasher.write_u64(event.hit_tick);
        for axis in 0..3 {
            hasher.write_i32(event.point_mm[axis]);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectile::{validate_flight_tables, FlightTableSet};

    /// A flat vacuum: no gravity, no drag, so the intercept arithmetic is
    /// exact and the test's oracle can be counted by hand.
    fn vacuum() -> FlightTables {
        FlightTables {
            gravity_mps2: Fixed::ZERO,
            linear_drag_per_s: Fixed::ZERO,
            quad_drag_per_m: Fixed::ZERO,
        }
    }

    /// A body that stands still at `x_mm` for `ticks` ticks.
    fn still(id: u32, x_mm: i32, ticks: u64, radius_mm: u32) -> ScriptTrack {
        ScriptTrack {
            id,
            radius_mm,
            samples: (0..ticks)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [x_mm, 0, 0],
                })
                .collect(),
        }
    }

    fn flat_shot() -> SpawnParams {
        SpawnParams {
            tick: 1000,
            shooter: 3,
            shot: 0,
            origin: [Fixed::ZERO; 3],
            dir: [Fixed::ONE, Fixed::ZERO, Fixed::ZERO],
            speed: Fixed::from_int(100),
            spread: Fixed::ZERO,
            ttl_ticks: 600,
        }
    }

    /// A body walking +X at 20 m/s from 60 m, in millimetres, sampled per
    /// tick: `20_000 / 60` millimetres of walk per 60 Hz tick.
    fn walking_x(tick: u64) -> i32 {
        let ticks = i32::try_from(tick).expect("the script is 600 ticks long");
        60_000 + (20_000 / 60) * ticks
    }

    /// The same flat shot, fired at an earlier tick: the moving-body script
    /// is indexed from tick 0, so the shot has to start there too.
    fn early_shot() -> SpawnParams {
        SpawnParams {
            tick: 0,
            ..flat_shot()
        }
    }

    #[test]
    fn a_stationary_body_is_hit_on_the_tick_the_round_arrives() {
        // 100 m/s at the 60 Hz tick advances 100 * (65536/60) / 65536 m per
        // step, so the 30th step ends about 5 mm short of 50 m: inside the
        // 500 mm shell, and the 29th ends 1.67 m short of it. The 30th step
        // is flown in tick 1000 + 29, the age the round had before it.
        let tables = vacuum();
        let set = FlightTableSet::new(9, tables);
        validate_flight_tables(&set).unwrap();
        let hit = resolve_shot(&flat_shot(), &tables, &[still(7, 50_000, 600, 500)])
            .unwrap()
            .unwrap();
        assert_eq!(hit.track, 7);
        assert_eq!(hit.hit_tick, 1000 + 29);
        // the round stopped short of the centre, so the closest point is the
        // end of the swept segment
        assert!((49_900..=50_000).contains(&hit.point_mm[0]));
    }

    #[test]
    fn a_moving_body_is_hit_on_the_intercept_tick() {
        // the body starts at 60 m and walks +X at 20 m/s: 100t = 60 + 20t
        // puts the catch at t = 0.75 s. The round's own step is truncated, so
        // the round is level with the body on the 45th step, flown in tick 44.
        let tables = vacuum();
        let track = ScriptTrack {
            id: 5,
            radius_mm: 300,
            samples: (0..600)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [walking_x(tick), 0, 0],
                })
                .collect(),
        };
        validate_script(&track).unwrap();
        let hit = resolve_shot(&early_shot(), &tables, &[track])
            .unwrap()
            .unwrap();
        assert_eq!(hit.track, 5);
        assert_eq!(hit.hit_tick, 44);
    }

    #[test]
    fn the_same_shot_twice_gives_the_same_hit_tick() {
        // RY3's accept, first half: two runs of one scripted shot land on the
        // same tick with the same event.
        let tables = vacuum();
        let track = still(11, 90_000, 600, 750);
        let run = || resolve_shot(&flat_shot(), &tables, std::slice::from_ref(&track)).unwrap();
        let first = run().unwrap();
        let second = run().unwrap();
        assert_eq!(first, second);
        // and the events hash the same, which is what the match hash folds
        assert_eq!(hash_hits(&[first]), hash_hits(&[second]));
    }

    #[test]
    fn a_shot_that_meets_nothing_is_a_clean_miss() {
        let tables = vacuum();
        // the body stands past the round's life: 100 m/s for 600 ticks is
        // 1000 m, so a body at 2000 m is never reached
        let far = still(7, 2_000_000, 600, 100);
        assert_eq!(resolve_shot(&flat_shot(), &tables, &[far]).unwrap(), None);
        // the body only appears once the round has already flown past it
        let late = ScriptTrack {
            id: 8,
            radius_mm: 1_000,
            samples: (1030..1200)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [40_000, 0, 0],
                })
                .collect(),
        };
        assert_eq!(resolve_shot(&flat_shot(), &tables, &[late]).unwrap(), None);
    }

    #[test]
    fn the_first_track_the_segment_touches_wins() {
        // two bodies inside one swept segment: the order of the track list,
        // not the order of the ids, names the winner
        let tables = vacuum();
        let left = ScriptTrack {
            id: 1,
            radius_mm: 500,
            samples: (0..600)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [50_000, 0, 0],
                })
                .collect(),
        };
        let right = ScriptTrack {
            id: 2,
            radius_mm: 500,
            samples: (0..600)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [50_005, 0, 0],
                })
                .collect(),
        };
        let hit = resolve_shot(&flat_shot(), &tables, &[right.clone(), left.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(hit.track, 2);
        let hit = resolve_shot(&flat_shot(), &tables, &[left, right])
            .unwrap()
            .unwrap();
        assert_eq!(hit.track, 1);
    }

    #[test]
    fn a_script_is_refused_with_the_reason() {
        let empty = ScriptTrack {
            id: 1,
            radius_mm: 100,
            samples: Vec::new(),
        };
        assert_eq!(
            validate_script(&empty).unwrap_err(),
            HitError::EmptyTrack(1)
        );
        let unsorted = ScriptTrack {
            id: 2,
            radius_mm: 100,
            samples: vec![
                TrackSample {
                    tick: 10,
                    center_mm: [0, 0, 0],
                },
                TrackSample {
                    tick: 10,
                    center_mm: [1, 0, 0],
                },
            ],
        };
        assert_eq!(
            validate_script(&unsorted).unwrap_err(),
            HitError::UnsortedTrack(2)
        );
        let fat = ScriptTrack {
            id: 3,
            radius_mm: MAX_RADIUS_MM + 1,
            samples: vec![TrackSample {
                tick: 0,
                center_mm: [0, 0, 0],
            }],
        };
        assert_eq!(validate_script(&fat).unwrap_err(), HitError::BadRadius(3));
    }

    #[test]
    fn a_shot_is_refused_without_tracks() {
        let tables = vacuum();
        let none: [ScriptTrack; 0] = [];
        assert_eq!(
            resolve_shot(&flat_shot(), &tables, &none).unwrap_err(),
            HitError::NoTracks
        );
        let many: Vec<ScriptTrack> = (0..MAX_TRACKS + 1)
            .map(|i| ScriptTrack {
                id: u32::try_from(i).expect("the track count fits u32"),
                ..still(0, 1_000, 600, 100)
            })
            .collect();
        assert_eq!(
            resolve_shot(&flat_shot(), &tables, &many).unwrap_err(),
            HitError::TooManyTracks(MAX_TRACKS + 1)
        );
    }

    #[test]
    fn the_event_hash_sees_every_field() {
        let tables = vacuum();
        let hit = resolve_shot(&flat_shot(), &tables, &[still(7, 50_000, 600, 500)])
            .unwrap()
            .unwrap();
        let base = hash_hits(&[hit]);
        let moved = HitEvent {
            point_mm: [hit.point_mm[0] + 1, hit.point_mm[1], hit.point_mm[2]],
            ..hit
        };
        assert_ne!(base, hash_hits(&[moved]));
        let other_body = HitEvent {
            track: hit.track + 1,
            ..hit
        };
        assert_ne!(base, hash_hits(&[other_body]));
        let other_tick = HitEvent {
            hit_tick: hit.hit_tick + 1,
            ..hit
        };
        assert_ne!(base, hash_hits(&[other_tick]));
        assert_ne!(base, hash_hits(&[hit, hit]));
    }

    #[test]
    fn a_body_off_the_flight_line_is_missed() {
        let tables = vacuum();
        // one body 3 m to the side of a straight +X flight, walked at two
        // shell sizes: a 200 mm shell misses it, a 3 m shell meets it
        let aside = |radius_mm: u32| ScriptTrack {
            id: 4,
            radius_mm,
            samples: (0..600)
                .map(|tick| TrackSample {
                    tick,
                    center_mm: [40_000, 3_000, 0],
                })
                .collect(),
        };
        assert_eq!(
            resolve_shot(&flat_shot(), &tables, &[aside(200)]).unwrap(),
            None
        );
        let hit = resolve_shot(&flat_shot(), &tables, &[aside(3_000)])
            .unwrap()
            .unwrap();
        assert_eq!(hit.track, 4);
    }
}
