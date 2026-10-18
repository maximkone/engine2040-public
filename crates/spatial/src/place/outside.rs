//! SPX10: the `outside` want, preferred pose first, and an honest failure reason.
//!
//! Consumer: `scene.place` (lane A adds the want parser; see
//! `docs/plan/spatial-entry-points.md`). The frozen [`Want`] vocabulary is not
//! touched. `outside` keeps the object's plan footprint clear of a target
//! (an object's footprint at any height, or a rectangle in cm). The current
//! pose is tried first, so "leave it where it is" works: it comes back
//! unchanged when it already satisfies every want and verifies. A failure says
//! which kind: `does_not_fit_on_support` (the object is larger than the
//! surface it was asked to rest on) or `no_free_spot` (crowded).
//! Determinism: z ascending, then x ascending, first match wins; integer cm.

use super::*;

/// What an `outside` want keeps the object away from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutsideTarget {
    /// Another object's plan footprint, at any height (a room floor is free space).
    Object(u32),
    /// A rectangle in cm, `[min_x, max_x] x [min_z, max_z]`.
    Region {
        min_x: i32,
        max_x: i32,
        min_z: i32,
        max_z: i32,
    },
}

/// Refusal: the kind, and the minimal wants that cannot hold together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutsideConflict {
    /// `does_not_fit_on_support`, `no_free_spot`, or a refusal code.
    pub reason: &'static str,
    /// Indices into `intent.wants`, in order.
    pub unmet_wants: Vec<usize>,
    /// Indices into the `outside` list, in order.
    pub unmet_outside: Vec<usize>,
}

fn rect_of(target: &OutsideTarget, ctx: &Ctx<'_>) -> Option<[i32; 4]> {
    match *target {
        OutsideTarget::Object(id) => ctx
            .boxes
            .get(&id)
            .map(|b| [b.min_x, b.max_x, b.min_z, b.max_z]),
        OutsideTarget::Region {
            min_x,
            max_x,
            min_z,
            max_z,
        } => Some([min_x, max_x, min_z, max_z]),
    }
}

/// Yaw index (45 degree steps) nearest the actor's current yaw.
fn yaw_idx_of(yaw: f32) -> i32 {
    let (fx, fz) = octant(yaw);
    (0..YAW_STEPS)
        .find(|k| octant(yaw_of(*k)) == (fx, fz))
        .unwrap_or(0)
}

/// Solve `intent` plus `outside` wants; see the module docs.
pub fn place_outside(
    intent: &Intent,
    outside: &[OutsideTarget],
    actors: &[Actor],
) -> Result<TransformOut, OutsideConflict> {
    let refuse = |reason: &'static str| OutsideConflict {
        reason,
        unmet_wants: (0..intent.wants.len()).collect(),
        unmet_outside: (0..outside.len()).collect(),
    };
    let obj = resolve_object(intent, actors).map_err(|_| refuse("unknown-object"))?;
    let bounds = room_bounds(actors).ok_or_else(|| refuse("no-bounds"))?;
    let ctx = context(actors, bounds);
    let rects: Vec<[i32; 4]> = outside
        .iter()
        .map(|t| rect_of(t, &ctx))
        .collect::<Option<_>>()
        .ok_or_else(|| refuse("unknown-target"))?;
    let rest: Vec<Actor> = actors.iter().filter(|a| a.id != obj.id).cloned().collect();
    let index = Index3::build(&rest);
    let (hx, hz) = (to_cm(obj.sx).abs() / 2, to_cm(obj.sz).abs() / 2);
    let clear = |cx: i32, cz: i32, r: &[i32; 4]| {
        cx + hx <= r[0] || r[1] <= cx - hx || cz + hz <= r[2] || r[3] <= cz - hz
    };
    let grid = grid_of(&obj, &bounds);
    let current = actors
        .iter()
        .find(|a| a.id == obj.id)
        .filter(|a| pos_known(a));
    let search = |wants: &[usize], outs: &[usize]| -> Option<TransformOut> {
        let list: Vec<Want> = wants.iter().map(|&i| intent.wants[i]).collect();
        let rest_cm = rest_for(&list, &obj, &ctx);
        let stacked_y_m = match rest_cm {
            None => obj.y_m,
            Some(rest) => f32::from(i16::try_from(rest).ok()?) / 100.0,
        };
        let stacked = Placed {
            id: obj.id,
            sx: obj.sx,
            sy: obj.sy,
            sz: obj.sz,
            y_m: stacked_y_m,
        };
        let ok = |cx: i32, cz: i32, yaw_idx: i32| {
            wants_hold(&list, cx, cz, yaw_idx, &stacked, &ctx)
                && outs.iter().all(|&i| clear(cx, cz, &rects[i]))
        };
        // The current pose first, returned unchanged when it already works.
        if let Some(a) = current {
            let (cx, cz) = (to_cm(a.x), to_cm(a.z));
            let here = TransformOut {
                id: a.id,
                x: a.x,
                y: a.y,
                z: a.z,
                yaw: a.yaw,
            };
            if ok(cx, cz, yaw_idx_of(a.yaw)) && verifies(&index, &stacked, &here, rest_cm) {
                return Some(here);
            }
        }
        let grid = grid.as_ref()?;
        let mut cz = grid.z_lo;
        while cz <= grid.z_hi {
            let mut cx = grid.x_lo;
            while cx <= grid.x_hi {
                for yaw_idx in 0..YAW_STEPS {
                    if ok(cx, cz, yaw_idx) {
                        let out = transform_of(&stacked, cx, cz, yaw_idx);
                        if verifies(&index, &stacked, &out, rest_cm) {
                            return Some(out);
                        }
                    }
                }
                cx += GRID_CM;
            }
            cz += GRID_CM;
        }
        None
    };
    let all_w: Vec<usize> = (0..intent.wants.len()).collect();
    let all_o: Vec<usize> = (0..outside.len()).collect();
    if let Some(out) = search(&all_w, &all_o) {
        return Ok(out);
    }
    let (keep_w, keep_o) = minimal(all_w, all_o, |w, o| search(w, o).is_none());
    Err(OutsideConflict {
        reason: if too_big_for_support(intent, &obj, &ctx) {
            "does_not_fit_on_support"
        } else {
            "no_free_spot"
        },
        unmet_wants: keep_w,
        unmet_outside: keep_o,
    })
}

/// Shrink by removal in order: drop each want, then each outside, while the
/// rest stays unsatisfiable (`fails` is true).
fn minimal(
    mut keep_w: Vec<usize>,
    mut keep_o: Vec<usize>,
    fails: impl Fn(&[usize], &[usize]) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    for i in keep_w.clone() {
        let trial: Vec<usize> = keep_w.iter().copied().filter(|&j| j != i).collect();
        if fails(&trial, &keep_o) {
            keep_w = trial;
        }
    }
    for i in keep_o.clone() {
        let trial: Vec<usize> = keep_o.iter().copied().filter(|&j| j != i).collect();
        if fails(&keep_w, &trial) {
            keep_o = trial;
        }
    }
    (keep_w, keep_o)
}

/// An `OnTopOf` want whose supporter's top is smaller than the object.
fn too_big_for_support(intent: &Intent, obj: &Placed, ctx: &Ctx<'_>) -> bool {
    intent.wants.iter().any(|w| {
        w.relation == Relation::OnTopOf
            && matches!(w.target, Target::Object(id) if ctx.boxes.get(&id).is_some_and(|b| {
                to_cm(obj.sx).abs() > b.max_x - b.min_x || to_cm(obj.sz).abs() > b.max_z - b.min_z
            }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn actor(id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y: 0.5,
            z,
            sx,
            sy: 1.0,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// Room of four thin walls (10 m), a zone footprint box, a mover.
    fn room(mover_at: (f32, f32)) -> Vec<Actor> {
        vec![
            actor(1, "wall_n", 0.0, -5.0, 10.0, 0.2),
            actor(2, "wall_s", 0.0, 5.0, 10.0, 0.2),
            actor(3, "wall_w", -5.0, 0.0, 0.2, 10.0),
            actor(4, "wall_e", 5.0, 0.0, 0.2, 10.0),
            {
                let mut zone = actor(5, "zone", 0.0, 0.0, 4.0, 4.0);
                zone.physical = false;
                zone
            },
            actor(9, "mover", mover_at.0, mover_at.1, 1.0, 1.0),
        ]
    }

    fn intent(wants: Vec<Want>) -> Intent {
        Intent {
            object: ObjectRef::Id(9),
            wants,
        }
    }

    #[test]
    fn a_free_preferred_pose_comes_back_exact() {
        let actors = room((3.2, 3.2));
        let out = place_outside(&intent(vec![]), &[OutsideTarget::Object(5)], &actors).unwrap();
        assert_eq!((out.x, out.y, out.z, out.yaw), (3.2, 0.5, 3.2, 0.0));
    }

    #[test]
    fn a_spawn_inside_the_zone_is_moved_outside_and_hugs_it() {
        let actors = room((0.0, 0.0));
        let next = Want {
            relation: Relation::NextTo,
            target: Target::Object(5),
            distance_cm: Some((0, 20)),
        };
        let out = place_outside(&intent(vec![next]), &[OutsideTarget::Object(5)], &actors).unwrap();
        let (cx, cz) = (to_cm(out.x), to_cm(out.z));
        // Zone spans +-200: the 100 cm mover must be fully outside on x or z.
        assert!(
            cx.abs() >= 250 || cz.abs() >= 250,
            "({cx},{cz}) overlaps the zone"
        );
        let gap = (cx.abs() - 250).max(cz.abs() - 250);
        assert!((0..=20).contains(&gap), "hugs within 2 cells, gap {gap}");
        assert_eq!(
            out,
            place_outside(&intent(vec![next]), &[OutsideTarget::Object(5)], &actors).unwrap()
        );
    }

    #[test]
    fn failures_name_their_kind() {
        let actors = room((3.2, 3.2));
        // The whole room is the region: crowded.
        let whole = OutsideTarget::Region {
            min_x: -600,
            max_x: 600,
            min_z: -600,
            max_z: 600,
        };
        let e = place_outside(&intent(vec![]), &[whole], &actors).unwrap_err();
        assert_eq!(
            (e.reason, e.unmet_outside.clone()),
            ("no_free_spot", vec![0])
        );
        // A 1 m crate cannot rest on a 40 cm stool and stay clear of it.
        let mut with_stool = actors.clone();
        with_stool.push(actor(7, "stool", -3.0, -3.0, 0.4, 0.4));
        let top = Want {
            relation: Relation::OnTopOf,
            target: Target::Object(7),
            distance_cm: None,
        };
        let e = place_outside(&intent(vec![top]), &[OutsideTarget::Object(7)], &with_stool)
            .unwrap_err();
        assert_eq!(e.reason, "does_not_fit_on_support");
        // The same crate on a wide table is fine.
        let mut with_table = actors.clone();
        with_table.push(actor(8, "table", -3.0, -3.0, 2.0, 2.0));
        let on = Want {
            relation: Relation::OnTopOf,
            target: Target::Object(8),
            distance_cm: None,
        };
        assert!(place_outside(&intent(vec![on]), &[], &with_table).is_ok());
        let e = place_outside(&intent(vec![]), &[OutsideTarget::Object(99)], &actors).unwrap_err();
        assert_eq!(e.reason, "unknown-target");
    }

    /// Brute force: scan the grid with plain rectangle math, first free wins.
    #[test]
    fn place_outside_equals_an_exhaustive_scan_on_seeded_rooms() {
        let mut rng = Rng::new(0x0757);
        for case in 0..20 {
            let at = (
                (rng.next_u32() % 9) as f32 - 4.0,
                (rng.next_u32() % 9) as f32 - 4.0,
            );
            let mut actors = room(at);
            actors[4].x = ((rng.next_u32() % 5) as f32 - 2.0) * 0.5;
            let region = OutsideTarget::Region {
                min_x: -300 + 50 * (rng.next_u32() % 5) as i32,
                max_x: 100 + 50 * (rng.next_u32() % 9) as i32,
                min_z: -300 + 50 * (rng.next_u32() % 5) as i32,
                max_z: 100 + 50 * (rng.next_u32() % 9) as i32,
            };
            let targets = [OutsideTarget::Object(5), region];
            let got = place_outside(&intent(vec![]), &targets, &actors);
            let free = |x: i32, z: i32| {
                let hit =
                    |r: [i32; 4]| x + 50 > r[0] && r[1] > x - 50 && z + 50 > r[2] && r[3] > z - 50;
                let zone = &actors[4];
                let zr = [to_cm(zone.x) - 200, to_cm(zone.x) + 200, -200, 200];
                let OutsideTarget::Region {
                    min_x,
                    max_x,
                    min_z,
                    max_z,
                } = region
                else {
                    unreachable!()
                };
                let walls_ok = actors[..4].iter().all(|w| {
                    let (hx, hz) = (to_cm(w.sx) / 2, to_cm(w.sz) / 2);
                    let r = [
                        to_cm(w.x) - hx,
                        to_cm(w.x) + hx,
                        to_cm(w.z) - hz,
                        to_cm(w.z) + hz,
                    ];
                    !hit(r)
                });
                walls_ok && !hit(zr) && !hit([min_x, max_x, min_z, max_z])
            };
            let here = (to_cm(at.0), to_cm(at.1));
            let want = if free(here.0, here.1) {
                Some((at.0, at.1))
            } else {
                (-460..=460)
                    .step_by(10)
                    .flat_map(|z| (-460..=460).step_by(10).map(move |x| (x, z)))
                    .find(|&(x, z)| free(x, z))
                    .map(|(x, z)| (x as f32 / 100.0, z as f32 / 100.0))
            };
            match (got, want) {
                (Ok(out), Some((x, z))) => assert_eq!((out.x, out.z), (x, z), "case {case}"),
                (Err(_), None) => {}
                (g, w) => panic!("case {case}: solver {g:?} vs scan {w:?}"),
            }
        }
    }
}
