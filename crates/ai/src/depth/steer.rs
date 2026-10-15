//! steer: foe-seeking + wall-escape steering over scored wishes.
//!
//! Consumer: [`super::DepthBrain::decide`] remaps each emitted wish through
//! [`steer_wish`]; the arena tournament consumes it once the arena feeds one
//! depth brain per fighter. Data owner: stateless pure functions own
//! nothing; brains own the arena half-extent. Units: millimetres for
//! positions, `-100..=100` wish percents. Determinism: integer math only
//! (`+ - * /`, comparisons); same inputs give byte-identical outputs. Errors:
//! total functions — every input maps to a legal wish, never panics. Budget:
//! O(1) integer ops per tick, zero allocation (no steady-state allocation).
//!
//! Fighting-game rule (Killer Instinct spacing): a wish that pushes into a
//! nearby wall slides off it — the blocked axis pushes straight inward
//! while the free axis keeps its scored slide — and a still wish with a far
//! foe becomes a closing wish. Scores, buttons and facing are never touched
//! here, so guard holds, strike timing and exchange answers survive; only
//! the planar wish vector changes, and only on wall contact or far-foe
//! stillness. The escape deliberately never beelines at the foe: pointing
//! both fighters head-on at each other synchronizes their approaches onto
//! the same strike tick, and simultaneous trades resolve in ID order.

use engine_input::fighter::WISH_RANGE;

/// Default arena half-extent in millimetres (the standard arena) until the
/// arena sets its own through the brain.
pub const DEFAULT_HALF_EXTENT_MM: i32 = 20_000;
/// Smallest arena half-extent the steering trusts (the arena range floor).
pub const MIN_HALF_EXTENT_MM: i32 = 5_000;
/// Largest arena half-extent the steering trusts (the arena range ceiling).
pub const MAX_HALF_EXTENT_MM: i32 = 100_000;
/// Wall-escape band: wishes pushing outward inside this distance (mm) from a
/// face are re-aimed. Wide enough to catch a spawn pin (leg 5 sits 740 mm
/// off the face) with margin for one full-speed tick (83 mm).
pub const WALL_PROX_MM: i32 = 2_000;
/// Foe-seeking range: a squared-planar separation beyond this (mm) counts as
/// far, matching the scripted far band the anti-stuck streak already uses.
pub const SEEK_MM: i32 = 8_000;
/// Flock radius in millimetres (fixed neighbourhood).
/// PORT(AizenKai1001/lunaris@56cc7e30:examples/flock `flock.rs`).
pub const FLOCK_RADIUS_MM: i32 = 4_000;
/// Inner separation band in millimetres (half the flock radius).
pub const FLOCK_INNER_MM: i32 = 2_000;
/// Separation weight (push apart inside the inner band).
pub const FLOCK_SEPARATION_W: i64 = 3;
/// Alignment weight (follow neighbour wishes).
pub const FLOCK_ALIGNMENT_W: i64 = 2;
/// Cohesion weight (pull toward the centre).
pub const FLOCK_COHESION_W: i64 = 2;
/// Position sanitize fence in millimetres (arena scale is +/-1000 m).
const POS_ABS_MAX_MM: i32 = 1_000_000;

/// Clamp a raw half-extent to the arena range; out-of-range input falls back
/// to [`DEFAULT_HALF_EXTENT_MM`] (refuse-and-fallback, like the curve
/// fallback). Total over all inputs.
pub const fn sanitize_half_extent_mm(raw_mm: i32) -> i32 {
    if raw_mm < MIN_HALF_EXTENT_MM || raw_mm > MAX_HALF_EXTENT_MM {
        DEFAULT_HALF_EXTENT_MM
    } else {
        raw_mm
    }
}

/// Clamp one coordinate into the sanitize fence.
const fn clamp_pos(coord_mm: i32) -> i32 {
    if coord_mm < -POS_ABS_MAX_MM {
        -POS_ABS_MAX_MM
    } else if coord_mm > POS_ABS_MAX_MM {
        POS_ABS_MAX_MM
    } else {
        coord_mm
    }
}

/// Clamp one wish axis to the contract range (`i8` carries `-128..=127`).
const fn clamp_wish(wish: i8) -> i8 {
    if wish < -WISH_RANGE {
        -WISH_RANGE
    } else if wish > WISH_RANGE {
        WISH_RANGE
    } else {
        wish
    }
}

/// Foe-ward wish in max-norm quantization: the dominant axis reads `+/-100`,
/// the other scales by integer division. Direction-exact, speed-full (the
/// arena normalizes the wish, so direction is what matters). `(0, 0)` only
/// when the fighters coincide; otherwise at least one axis is nonzero.
/// Integer math only; `m > 0` after the guard, so no division by zero.
fn foe_wish(dx_mm: i32, dz_mm: i32) -> (i8, i8) {
    let m = i64::from(dx_mm.abs()).max(i64::from(dz_mm.abs()));
    if m <= 0 {
        return (0, 0);
    }
    let qx = (i64::from(dx_mm) * 100 / m).clamp(-100, 100) as i8;
    let qz = (i64::from(dz_mm) * 100 / m).clamp(-100, 100) as i8;
    (qx, qz)
}

/// One flock neighbour: offset (neighbour minus self, mm) plus its wish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlockNeighbour {
    /// Neighbour x minus self x in millimetres.
    pub dx_mm: i32,
    /// Neighbour z minus self z in millimetres.
    pub dz_mm: i32,
    /// Neighbour wish x in `-100..=100`.
    pub mx: i8,
    /// Neighbour wish z in `-100..=100`.
    pub mz: i8,
}

/// Blend base wish with separation/alignment/cohesion over neighbours.
/// Empty or all outside radius returns base. Integer `+ - * /` only, no
/// alloc, O(neighbours). Caller: [`steer_wish`] (empty quiescent call).
/// PORT(AizenKai1001/lunaris@56cc7e30:examples/flock `flock.rs`).
pub fn flock_wish(base_mx: i8, base_mz: i8, neighbours: &[FlockNeighbour]) -> (i8, i8) {
    let (bx, bz) = (clamp_wish(base_mx), clamp_wish(base_mz));
    if neighbours.is_empty() {
        return (bx, bz);
    }
    let radius = i64::from(FLOCK_RADIUS_MM);
    let r2 = radius * radius;
    let in2 = i64::from(FLOCK_INNER_MM) * i64::from(FLOCK_INNER_MM);
    let (mut sepx, mut sepz, mut cohx, mut cohz, mut alix, mut aliz, mut n_in) =
        (0i64, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
    for n in neighbours {
        let dx = clamp_pos(n.dx_mm);
        let dz = clamp_pos(n.dz_mm);
        let d2 = i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz);
        if d2 > r2 {
            continue;
        }
        n_in += 1;
        cohx += i64::from(dx);
        cohz += i64::from(dz);
        alix += i64::from(i32::from(clamp_wish(n.mx)));
        aliz += i64::from(i32::from(clamp_wish(n.mz)));
        if d2 <= in2 {
            sepx -= i64::from(dx);
            sepz -= i64::from(dz);
        }
    }
    if n_in <= 0 {
        return (bx, bz);
    }
    let scale = radius / 100;
    let total_x = i64::from(i32::from(bx)) * scale
        + FLOCK_SEPARATION_W * sepx
        + FLOCK_COHESION_W * cohx
        + FLOCK_ALIGNMENT_W * alix * scale;
    let total_z = i64::from(i32::from(bz)) * scale
        + FLOCK_SEPARATION_W * sepz
        + FLOCK_COHESION_W * cohz
        + FLOCK_ALIGNMENT_W * aliz * scale;
    if total_x == 0 && total_z == 0 {
        return (bx, bz);
    }
    let m = total_x.abs().max(total_z.abs());
    if m <= 0 {
        return (bx, bz);
    }
    let qx = (total_x * 100 / m).clamp(-100, 100);
    let qz = (total_z * 100 / m).clamp(-100, 100);
    (i8::try_from(qx).unwrap_or(0), i8::try_from(qz).unwrap_or(0))
}

/// Steer one scored wish (`mx`, `mz`) from `self` toward `foe` inside an
/// arena of `half_extent_mm`:
///
/// * wall contact (within [`WALL_PROX_MM`] of a face while pushing outward):
///   the blocked axis is forced straight inward, the free axis keeps its
///   scored wish, so a wish grinding into the clamp always regains planar
///   motion (never a foe beeline: head-on sync loses on ID order — V1
///   foe-ward wall-escape / retreat-turn rejected, rates 632-665 vs bar);
/// * far drift (squared separation beyond [`SEEK_MM`] while holding still
///   `(0,0)`): becomes a closing wish via `foe_wish`, so a passive brain
///   re-acquires instead of drifting (backing-away wishes pass through);
/// * anything else passes through byte-identical, so mid-arena spacing,
///   strike timing and guard holds are untouched.
///
/// Total: every input maps to a wish inside `-100..=100`, never panics.
pub fn steer_wish(
    self_x_mm: i32,
    self_z_mm: i32,
    foe_x_mm: i32,
    foe_z_mm: i32,
    mx: i8,
    mz: i8,
    half_extent_mm: i32,
) -> (i8, i8) {
    let half = sanitize_half_extent_mm(half_extent_mm);
    let (sx, sz) = (clamp_pos(self_x_mm), clamp_pos(self_z_mm));
    let dx = clamp_pos(foe_x_mm).saturating_sub(sx);
    let dz = clamp_pos(foe_z_mm).saturating_sub(sz);
    let (mx, mz) = (clamp_wish(mx), clamp_wish(mz));
    // Near-face while pushing outward: the clamp would eat this axis.
    // `signum` is 0 at center, so mid-arena wishes never trip this.
    let blocked_x =
        sx.abs() >= half - WALL_PROX_MM && i32::from(mx.signum()) == sx.signum() && mx != 0;
    let blocked_z =
        sz.abs() >= half - WALL_PROX_MM && i32::from(mz.signum()) == sz.signum() && mz != 0;
    if blocked_x || blocked_z {
        // A blocked axis pushes straight off the wall; the free axis keeps
        // its scored slide. Deliberately NOT foe-ward: re-aiming both
        // fighters head-on at each other synchronizes their approaches onto
        // the same strike tick, and simultaneous trades resolve in ID order
        // (a structural slot-0 bias the win-rate bar forbids). Spacing first,
        // the mid-arena game re-engages at varied phases on its own.
        let ex = if blocked_x {
            -sx.signum() * 100
        } else {
            i32::from(mx)
        };
        let ez = if blocked_z {
            -sz.signum() * 100
        } else {
            i32::from(mz)
        };
        return (ex as i8, ez as i8);
    }
    // Far drift: a still wish with a far foe closes in. Scored retreats
    // keep their spacing direction (turning every back-away into a
    // head-on approach synchronizes both fighters onto the same strike
    // tick, and simultaneous trades resolve in ID order — a structural
    // slot-0 bias the win-rate bar forbids). Squared comparison, no
    // `sqrt`; `i64` so arena-scale squares fit.
    let far = i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
        > i64::from(SEEK_MM) * i64::from(SEEK_MM);
    if far && mx == 0 && mz == 0 {
        return foe_wish(dx, dz);
    }
    // Flock quiescent: no neighbours here, so the base wish passes through
    // byte-identical while `flock_wish` stays wired into the steer path.
    flock_wish(mx, mz, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_arena_wishes_pass_through_identical() {
        // Spacing, retreats, holds and approaches far from any face survive
        // byte-identical: steering is quiescent off the wall.
        let cases: Vec<(i32, i32, i32, i32, i8, i8)> = vec![
            (0, 0, 3_000, 4_000, 100, 0),            // advance at a near foe
            (0, 0, 3_000, 4_000, -80, 0),            // retreat from a near foe (spacing)
            (0, 0, 3_000, 4_000, 0, 0),              // guard hold near the foe
            (0, 0, 3_000, 4_000, 30, 0),             // strike step near the foe
            (0, 0, 10_000, 0, 100, 0),               // far foe, already closing
            (0, 0, 10_000, 0, 0, 100),               // far foe, lateral slide (motion)
            (0, 0, -10_000, 5_000, -100, 50),        // far foe, closing diagonal
            (-5_000, 7_000, 2_000, -1_000, 60, -80), // wander off the wall
            (17_900, 0, 19_000, 0, 100, 0),          // fast lane, still outside the band
        ];
        for (sx, sz, fx, fz, mx, mz) in cases {
            assert_eq!(
                steer_wish(sx, sz, fx, fz, mx, mz, 20_000),
                (mx, mz),
                "mid-arena ({sx},{sz})->({fx},{fz}) wish ({mx},{mz})"
            );
        }
        // Parallel near the face (sliding, not pushing) also survives.
        assert_eq!(
            steer_wish(19_000, 0, 5_000, 5_000, 0, 100, 20_000),
            (0, 100)
        );
    }

    #[test]
    fn wall_contact_slides_off_preserving_slide() {
        // Leg 1 shape: pinned east, pushing +X. The blocked axis goes
        // inward; the dead axis stays dead (no scored slide to keep).
        let (mx, mz) = steer_wish(19_360, 500, -10_000, -3_000, 100, 0, 20_000);
        assert_eq!((mx, mz), (-100, 0));
        // A scored slide along the wall survives the escape.
        assert_eq!(
            steer_wish(19_360, 500, -10_000, -3_000, 100, 60, 20_000),
            (-100, 60)
        );
        // Mirrors: west, north, south faces.
        assert_eq!(steer_wish(-19_360, 0, 10_000, 0, -100, 0, 20_000), (100, 0));
        assert_eq!(steer_wish(0, 19_360, 0, -10_000, 0, 100, 20_000), (0, -100));
        assert_eq!(steer_wish(0, -19_360, 0, 10_000, 0, -100, 20_000), (0, 100));
        // Corner grind: both axes forced inward.
        assert_eq!(
            steer_wish(19_500, -19_500, 0, 0, 100, -100, 20_000),
            (-100, 100)
        );
    }

    #[test]
    fn wall_grind_keeps_its_free_axis_motion() {
        // Legs 3/4 shape: both fighters east, pushing +X with a +Z slide.
        // The blocked axis goes inward while the slide carries on, so the
        // wall grind becomes wall motion instead of a beeline.
        assert_eq!(
            steer_wish(19_600, 11_060, 19_600, 17_290, 100, 40, 20_000),
            (-100, 40)
        );
    }

    #[test]
    fn foe_beyond_the_wall_still_escapes() {
        // Foe further outward than self (both near the face): the quantized
        // foe wish would push out, so the force-inward rule must win.
        let (mx, mz) = steer_wish(19_000, 0, 19_500, 200, 100, 0, 20_000);
        assert!(mx < 0, "inward wins over foe-beyond-wall: ({mx},{mz})");
    }

    #[test]
    fn coincident_fighters_at_wall_move_inward() {
        // Zero foe delta quantizes to (0, 0); the blocked axis still forces
        // inward so coincident wall grinds cannot sit still.
        let (mx, mz) = steer_wish(19_500, 0, 19_500, 0, 100, 0, 20_000);
        assert!(mx < 0, "coincident pin must move: ({mx},{mz})");
        // Coincident mid-arena with a live wish passes through.
        assert_eq!(steer_wish(0, 0, 0, 0, 100, 0, 20_000), (100, 0));
    }

    #[test]
    fn far_drift_closes_while_near_holds() {
        // Zero wish with a far foe becomes a closing wish (a passive foe
        // re-acquires instead of holding).
        let (mx, mz) = steer_wish(0, 0, 12_000, 0, 0, 0, 20_000);
        assert_eq!((mx, mz), (100, 0));
        let (mx, mz) = steer_wish(0, 0, -9_000, -9_000, 0, 0, 20_000);
        assert!(mx < 0 && mz < 0, "diagonal close: ({mx},{mz})");
        // A scored retreat keeps its spacing direction even far away:
        // re-aiming back-aways would lock both fighters onto the same
        // strike tick (ID-ordered trades, slot-0 bias).
        assert_eq!(steer_wish(0, 0, 12_000, 0, -80, 0, 20_000), (-80, 0));
        // Near-foe holds and spacing survive (win-rate legs untouched).
        assert_eq!(steer_wish(0, 0, 3_000, 0, 0, 0, 20_000), (0, 0));
        assert_eq!(steer_wish(0, 0, 3_000, 0, -80, 0, 20_000), (-80, 0));
    }

    #[test]
    fn every_wall_grind_regains_motion() {
        // Property over faces x outward wishes: wall contact never returns a
        // zero wish and always carries an inward component on each blocked
        // axis. A zero or still-outward row is the stuck bug back again.
        let faces: Vec<(i32, i32)> = vec![
            (19_600, 0),
            (-19_600, 0),
            (0, 19_600),
            (0, -19_600),
            (19_600, 19_600),
            (0, 0),
        ];
        let foes: Vec<(i32, i32)> = vec![(0, 0), (19_600, 0), (-19_600, 5_000), (3_000, -4_000)];
        let wishes: Vec<(i8, i8)> = vec![(100, 0), (-100, 0), (0, 100), (0, -100), (100, 100)];
        for (sx, sz) in &faces {
            for (fx, fz) in &foes {
                for (mx, mz) in &wishes {
                    let (ex, ez) = steer_wish(*sx, *sz, *fx, *fz, *mx, *mz, 20_000);
                    assert!(
                        (-100..=100).contains(&ex) && (-100..=100).contains(&ez),
                        "range: ({sx},{sz}) wish ({mx},{mz}) -> ({ex},{ez})"
                    );
                    let near_x = sx.abs() >= 20_000 - WALL_PROX_MM;
                    let near_z = sz.abs() >= 20_000 - WALL_PROX_MM;
                    let pushing_x = *mx != 0 && i32::from(mx.signum()) == sx.signum();
                    let pushing_z = *mz != 0 && i32::from(mz.signum()) == sz.signum();
                    if (near_x && pushing_x) || (near_z && pushing_z) {
                        assert!(
                            (ex, ez) != (0, 0),
                            "contact must move: ({sx},{sz}) wish ({mx},{mz})"
                        );
                        if near_x && pushing_x {
                            assert!(
                                i32::from(ex.signum()) == -sx.signum(),
                                "blocked x inward: ({sx},{sz}) -> ({ex},{ez})"
                            );
                        }
                        if near_z && pushing_z {
                            assert!(
                                i32::from(ez.signum()) == -sz.signum(),
                                "blocked z inward: ({sx},{sz}) -> ({ex},{ez})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sanitize_never_panics_and_stays_in_range() {
        // i8 extremes, arena-scale extremes, wild extents: always legal.
        let (ex, ez) = steer_wish(
            i32::MAX,
            i32::MIN,
            i32::MIN,
            i32::MAX,
            i8::MIN,
            i8::MAX,
            i32::MIN,
        );
        assert!((-100..=100).contains(&ex) && (-100..=100).contains(&ez));
        assert_eq!(sanitize_half_extent_mm(0), DEFAULT_HALF_EXTENT_MM);
        assert_eq!(sanitize_half_extent_mm(-7), DEFAULT_HALF_EXTENT_MM);
        assert_eq!(sanitize_half_extent_mm(4_999), DEFAULT_HALF_EXTENT_MM);
        assert_eq!(sanitize_half_extent_mm(5_000), 5_000);
        assert_eq!(sanitize_half_extent_mm(100_000), 100_000);
        assert_eq!(sanitize_half_extent_mm(100_001), DEFAULT_HALF_EXTENT_MM);
        // A cramped arena still escapes: 5 m faces, pinned at 4.6 m.
        let (mx, _) = steer_wish(4_600, 0, -4_000, 0, 100, 0, 5_000);
        assert!(mx < 0, "small-arena escape: ({mx},..)");
    }

    #[test]
    fn steering_is_deterministic_and_mirror_symmetric() {
        // Same inputs twice agree; mirroring x mirrors the answer exactly.
        let first = steer_wish(19_360, 500, -10_000, -3_000, 100, 0, 20_000);
        let second = steer_wish(19_360, 500, -10_000, -3_000, 100, 0, 20_000);
        assert_eq!(first, second);
        let (ax, az) = steer_wish(5_000, -2_000, -7_000, 3_000, -80, 20, 20_000);
        let (bx, bz) = steer_wish(-5_000, -2_000, 7_000, 3_000, 80, 20, 20_000);
        assert_eq!((ax, az), (-bx, bz), "x-mirror must mirror");
    }

    #[test]
    fn flock_blends_separation_cohesion_alignment() {
        assert_eq!(flock_wish(30, -40, &[]), (30, -40));
        let far = [FlockNeighbour {
            dx_mm: 10_000,
            dz_mm: 0,
            mx: 100,
            mz: 0,
        }];
        assert_eq!(flock_wish(30, -40, &far), (30, -40));
        let tight = [FlockNeighbour {
            dx_mm: 200,
            dz_mm: 0,
            mx: 0,
            mz: 0,
        }];
        let (sx, _) = flock_wish(0, 0, &tight);
        assert!(sx < 0, "crowded east must push west: ({sx},..)");
        let wide = [FlockNeighbour {
            dx_mm: 3_000,
            dz_mm: 0,
            mx: 0,
            mz: 0,
        }];
        let (cx, _) = flock_wish(0, 0, &wide);
        assert!(cx > 0, "far east must pull east: ({cx},..)");
        let lead = [FlockNeighbour {
            dx_mm: 0,
            dz_mm: 0,
            mx: 100,
            mz: 0,
        }];
        let (ax, _) = flock_wish(0, 0, &lead);
        assert!(ax > 0, "aligned east must steer east: ({ax},..)");
    }

    #[test]
    fn flock_wild_inputs_stay_in_range() {
        // Failure case: wild offsets and wishes never panic and stay legal.
        let wild = [
            FlockNeighbour {
                dx_mm: i32::MAX,
                dz_mm: i32::MIN,
                mx: i8::MIN,
                mz: i8::MAX,
            },
            FlockNeighbour {
                dx_mm: 0,
                dz_mm: 0,
                mx: 100,
                mz: -100,
            },
        ];
        let (ex, ez) = flock_wish(i8::MIN, i8::MAX, &wild);
        assert!((-100..=100).contains(&ex) && (-100..=100).contains(&ez));
        assert_eq!(flock_wish(0, 0, &wild), flock_wish(0, 0, &wild));
    }
}
