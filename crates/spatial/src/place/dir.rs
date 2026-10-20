//! SPF3: facing-relative placement wants (`in_front_of`, `behind`, `left_of`,
//! `right_of` with an edge gap range).
//!
//! Consumer: `scene.place` (lane A adds the want grammar; see
//! `docs/plan/spatial-entry-points.md`). The frozen [`Want`] vocabulary is not
//! touched: these wants ride beside it. Each is solved on the same 10 cm grid
//! and confirmed by the exact predicate `scene.query kind=relation` uses
//! (`relation::holds`), so a placed pose always reads back true. `holds(s, rel,
//! o)` is "s is REL of o": the placed object is `s`, the anchor is `o`.
//! Determinism: z ascending, then x ascending, first match wins; integer cm.

use super::*;
use crate::relation::{holds, Frame, Rel};

/// Default edge gap range in cm for a facing-relative want.
pub const DEFAULT_DIR_GAP_CM: (i32, i32) = (0, 100);

/// One facing-relative want: the placed object is `word` of `anchor` in
/// `frame`, with its footprint's XZ edge gap to the anchor inside `gap_cm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirWant {
    /// `Rel::Front`, `Rel::Behind`, `Rel::Left` or `Rel::Right`.
    pub word: Rel,
    pub anchor: u32,
    /// Inclusive `(min, max)` edge gap in cm.
    pub gap_cm: (i32, i32),
    pub frame: Frame,
}

/// Minimal set of wants (by index) that cannot hold together, or a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirConflict {
    pub unmet: Vec<usize>,
    pub reason: String,
}

/// Unit step of the half-plane a word names, from the anchor's facing `f`.
fn word_dir(word: Rel, f: (i32, i32)) -> Option<(i32, i32)> {
    match word {
        Rel::Front => Some(f),
        Rel::Behind => Some((-f.0, -f.1)),
        Rel::Right => Some((-f.1, f.0)),
        Rel::Left => Some((f.1, -f.0)),
        _ => None,
    }
}

/// Facing of the reference: north for World, the anchor's or viewer's octant else.
fn facing(w: &DirWant, by_id: &BTreeMap<u32, &Actor>) -> Option<(i32, i32)> {
    match w.frame {
        Frame::World => Some((0, -1)),
        Frame::Subject => by_id.get(&w.anchor).map(|a| octant(a.yaw)),
        Frame::Viewer(v) => by_id.get(&v).map(|a| octant(a.yaw)),
    }
}

/// Place `object` so every want holds, or name the minimal conflicting wants.
pub fn place_dir(
    object: &ObjectRef,
    wants: &[DirWant],
    actors: &[Actor],
) -> Result<TransformOut, DirConflict> {
    let all = || (0..wants.len()).collect::<Vec<_>>();
    let refuse = |reason: String| DirConflict {
        unmet: all(),
        reason,
    };
    let intent = Intent {
        object: object.clone(),
        wants: Vec::new(),
    };
    let obj = resolve_object(&intent, actors).map_err(|e| refuse(e.into_reason()))?;
    let bounds = room_bounds(actors)
        .ok_or_else(|| refuse("no-bounds: no valid actor anchors the room".to_owned()))?;
    let ctx = context(actors, bounds);
    let by_id: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    for w in wants {
        if word_dir(w.word, (1, 0)).is_none() {
            return Err(refuse(
                "bad-word: use in_front_of, behind, left_of or right_of".to_owned(),
            ));
        }
        if w.anchor == obj.id || !ctx.boxes.contains_key(&w.anchor) || facing(w, &by_id).is_none() {
            return Err(refuse(format!("unknown-anchor id {}", w.anchor)));
        }
        if valid_range(Some(w.gap_cm)).is_none() {
            return Err(refuse(format!(
                "bad-gap {:?}: need 0 <= min <= max",
                w.gap_cm
            )));
        }
    }
    let rest: Vec<Actor> = actors.iter().filter(|a| a.id != obj.id).cloned().collect();
    let index = Index3::build(&rest);
    let grid = grid_of(&obj, &bounds);
    let search = |subset: &[usize]| -> Option<TransformOut> {
        let grid = grid.as_ref()?;
        let (hx, hz) = (to_cm(obj.sx).abs() / 2, to_cm(obj.sz).abs() / 2);
        let mut cz = grid.z_lo;
        while cz <= grid.z_hi {
            let mut cx = grid.x_lo;
            while cx <= grid.x_hi {
                if subset
                    .iter()
                    .all(|&i| fast_ok(&wants[i], cx, cz, hx, hz, &ctx, &by_id))
                {
                    let out = transform_of(&obj, cx, cz, 0);
                    if verifies(&ctx, &index, &obj, &out, None)
                        && confirmed(subset, wants, &out, actors)
                    {
                        return Some(out);
                    }
                }
                cx += GRID_CM;
            }
            cz += GRID_CM;
        }
        None
    };
    if let Some(out) = search(&all()) {
        return Ok(out);
    }
    let mut conflict = all();
    for i in 0..wants.len() {
        let trial: Vec<usize> = conflict.iter().copied().filter(|&j| j != i).collect();
        if search(&trial).is_none() {
            conflict = trial;
        }
    }
    Err(DirConflict {
        reason: format!(
            "no placement on {GRID_CM} cm grid satisfies {} facing want(s); minimal conflict covers {}",
            wants.len(),
            conflict.len()
        ),
        unmet: conflict,
    })
}

/// Cheap integer prefilter: right half-plane (World and Subject frames) and
/// edge gap inside the range. `holds` still has the last word.
fn fast_ok(
    w: &DirWant,
    cx: i32,
    cz: i32,
    hx: i32,
    hz: i32,
    ctx: &Ctx<'_>,
    by_id: &BTreeMap<u32, &Actor>,
) -> bool {
    let Some(anchor) = ctx.boxes.get(&w.anchor) else {
        return false;
    };
    let gap2 = edge_gap2(cx - hx, cx + hx, cz - hz, cz + hz, anchor);
    let (lo, hi) = (i64::from(w.gap_cm.0), i64::from(w.gap_cm.1));
    if gap2 < lo * lo || gap2 > hi * hi {
        return false;
    }
    if let (Frame::World | Frame::Subject, Some(f)) = (w.frame, facing(w, by_id)) {
        let Some(u) = word_dir(w.word, f) else {
            return false;
        };
        let (ax, az) = (
            (anchor.min_x + anchor.max_x) / 2,
            (anchor.min_z + anchor.max_z) / 2,
        );
        return i64::from(cx - ax) * i64::from(u.0) + i64::from(cz - az) * i64::from(u.1) > 0;
    }
    true
}

/// The query predicate, run on the scene with the object moved to `out`.
fn confirmed(subset: &[usize], wants: &[DirWant], out: &TransformOut, actors: &[Actor]) -> bool {
    let mut moved = actors.to_vec();
    if let Some(a) = moved.iter_mut().find(|a| a.id == out.id) {
        a.x = out.x;
        a.z = out.z;
    }
    subset.iter().all(|&i| {
        holds(
            &moved,
            out.id,
            wants[i].word,
            wants[i].anchor,
            wants[i].frame,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn actor(id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32, yaw: f32) -> Actor {
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
            yaw,
        }
    }

    /// A 12 m room of four thin walls, the anchor at the middle and a mover.
    fn room(anchor_yaw: f32) -> Vec<Actor> {
        vec![
            actor(1, "wall_n", 0.0, -6.0, 12.0, 0.2, 0.0),
            actor(2, "wall_s", 0.0, 6.0, 12.0, 0.2, 0.0),
            actor(3, "wall_w", -6.0, 0.0, 0.2, 12.0, 0.0),
            actor(4, "wall_e", 6.0, 0.0, 0.2, 12.0, 0.0),
            actor(10, "anchor", 0.0, 0.0, 1.0, 1.0, anchor_yaw),
            actor(11, "mover", 5.0, 5.0, 1.0, 1.0, 0.0),
        ]
    }

    /// Hand table of the eight octant facings, k * 45 degrees from +X toward +Z.
    const FACING: [(i64, i64); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];

    #[test]
    fn place_facing_eight_facings_by_four_words_land_in_the_named_half_plane() {
        for (k, f) in FACING.iter().enumerate() {
            let yaw = k as f32 * core::f32::consts::FRAC_PI_4;
            let actors = room(yaw);
            for (word, u) in [
                (Rel::Front, (f.0, f.1)),
                (Rel::Behind, (-f.0, -f.1)),
                (Rel::Right, (-f.1, f.0)),
                (Rel::Left, (f.1, -f.0)),
            ] {
                let want = DirWant {
                    word,
                    anchor: 10,
                    gap_cm: (30, 90),
                    frame: Frame::Subject,
                };
                let out = place_dir(&ObjectRef::Id(11), &[want], &actors)
                    .unwrap_or_else(|c| panic!("facing {k} {word:?}: {c:?}"));
                let (px, pz) = (i64::from(to_cm(out.x)), i64::from(to_cm(out.z)));
                assert!(
                    px * u.0 + pz * u.1 > 0,
                    "facing {k} {word:?}: ({px},{pz}) is not on the {u:?} side"
                );
                let gap = |d: i64| (d.abs() - 100).max(0);
                let g2 = gap(px).pow(2) + gap(pz).pow(2);
                assert!(
                    (900..=8100).contains(&g2),
                    "facing {k} {word:?}: gap^2 {g2}"
                );
                let mut moved = actors.clone();
                moved[5].x = out.x;
                moved[5].z = out.z;
                assert!(
                    holds(&moved, 11, word, 10, Frame::Subject),
                    "facing {k} {word:?}"
                );
                assert_eq!(
                    out,
                    place_dir(&ObjectRef::Id(11), &[want], &actors).unwrap(),
                    "replay x2"
                );
            }
        }
    }

    #[test]
    fn place_facing_world_frame_front_means_north_and_an_impossible_gap_is_a_conflict() {
        let actors = room(0.0);
        let north = DirWant {
            word: Rel::Front,
            anchor: 10,
            gap_cm: (50, 50),
            frame: Frame::World,
        };
        let out = place_dir(&ObjectRef::Name("mover".into()), &[north], &actors).unwrap();
        assert!(out.z < 0.0, "north is -z: {out:?}");
        // 20 m away does not fit a 12 m room: the single want is the conflict.
        let far = DirWant {
            gap_cm: (2000, 2100),
            ..north
        };
        let err = place_dir(&ObjectRef::Id(11), &[north, far], &actors).unwrap_err();
        assert_eq!(err.unmet, vec![1]);
        // Both words at once is a conflict of the pair.
        let behind = DirWant {
            word: Rel::Behind,
            gap_cm: (50, 50),
            ..north
        };
        let err = place_dir(&ObjectRef::Id(11), &[north, behind], &actors).unwrap_err();
        assert_eq!(err.unmet, vec![0, 1]);
        let bad = DirWant {
            word: Rel::Near,
            ..north
        };
        assert!(place_dir(&ObjectRef::Id(11), &[bad], &actors)
            .unwrap_err()
            .reason
            .starts_with("bad-word"));
    }

    /// Brute force: every grid cell through the exact predicate, first wins.
    #[test]
    fn place_facing_equals_an_exhaustive_scan_on_seeded_rooms() {
        let mut rng = Rng::new(0xd1a);
        let words = [Rel::Front, Rel::Behind, Rel::Left, Rel::Right];
        let frames = [Frame::World, Frame::Subject];
        for case in 0..20 {
            let k = rng.next_u32() % 8;
            let mut actors = room(k as f32 * core::f32::consts::FRAC_PI_4);
            actors[4].x = ((rng.next_u32() % 5) as f32 - 2.0) * 0.5;
            let want = DirWant {
                word: words[(rng.next_u32() % 4) as usize],
                anchor: 10,
                gap_cm: (
                    20 * (rng.next_u32() % 3) as i32,
                    120 + 20 * (rng.next_u32() % 3) as i32,
                ),
                frame: frames[(rng.next_u32() % 2) as usize],
            };
            let got = place_dir(&ObjectRef::Id(11), &[want], &actors);
            let mut want_out = None;
            'scan: for z in (-590..=590).step_by(10) {
                for x in (-590..=590).step_by(10) {
                    let (px, pz) = (x as f32 / 100.0, z as f32 / 100.0);
                    let mut moved = actors.clone();
                    moved[5].x = px;
                    moved[5].z = pz;
                    let clear = moved.iter().filter(|a| a.id != 11).all(|a| {
                        let (hx, hz) = (to_cm(a.sx) / 2, to_cm(a.sz) / 2);
                        let (ax, az) = (to_cm(a.x), to_cm(a.z));
                        !((x - 50) < ax + hx
                            && ax - hx < (x + 50)
                            && (z - 50) < az + hz
                            && az - hz < (z + 50))
                    });
                    let anchor = &actors[4];
                    let (ax, az) = (to_cm(anchor.x), to_cm(anchor.z));
                    let g = |d: i32| (d.abs() - 100).max(0);
                    let g2 = i64::from(g(x - ax)).pow(2) + i64::from(g(z - az)).pow(2);
                    let (lo, hi) = (i64::from(want.gap_cm.0), i64::from(want.gap_cm.1));
                    if clear
                        && lo * lo <= g2
                        && g2 <= hi * hi
                        && holds(&moved, 11, want.word, 10, want.frame)
                    {
                        want_out = Some((px, pz));
                        break 'scan;
                    }
                }
            }
            match (got, want_out) {
                (Ok(out), Some((x, z))) => assert_eq!((out.x, out.z), (x, z), "case {case}"),
                (Err(_), None) => {}
                (g, w) => panic!("case {case}: solver {g:?} vs scan {w:?}"),
            }
        }
    }
}
