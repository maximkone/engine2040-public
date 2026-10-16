//! fps_visibility: per-viewer FPS sight sets plus interest digests.
//! Owner: spatial crate. Consumer: `fps_snapshot` below (the one non-test
//! call path); outside the crate the AI brains gate chase on membership and
//! net interest intersects its ring selection with the set.
//! Units: meters in, integer cm inside; ids ascending out.
//! Determinism: id-sorted output, FNV-1a over integer bytes, no float past
//! entry quantization, no clock, no map order.
//! Errors: typed [`SightError`](super::sight::SightError); stale roster ids
//! are skipped, a non-finite eye refuses.
//! Budget: one `visible` ray per candidate over the solid count, zero heap
//! use past the caller's reused `out`; rosters hold at most
//! [`FPS_MAX_CANDIDATES`] ids (the net interest cap).
//! Book: [handsrust | Plotting Fields of View] (dirty-gated recompute),
//! [handsrust | Hiding Entities] (draw gated on sight).

use super::sight::{SightError, SightWorld};
use engine_core::Fnv64;

/// Largest roster one snapshot filters: 64 fighters plus 500 AI soldiers
/// plus headroom, the same cap net interest selects under.
pub const FPS_MAX_CANDIDATES: usize = 1024;

/// Digest tag so FPS sight bytes never alias another domain hash.
const FPS_SIGHT_TAG: &[u8] = b"e2fpssight:1";

/// Visible ids for one FPS viewer: FOV cone plus occlusion per candidate.
///
/// Clears `out`, then pushes every candidate both inside the cone and
/// unoccluded, ascending with duplicates collapsed, so any candidate order
/// reports the same set. Units: meters in, integer cm inside. Errors: a
/// non-finite eye refuses with [`SightError::NonFinitePoint`] before `out`
/// is touched; stale roster ids are skipped, never an error (use
/// [`visible`](super::sight::visible) for loud single-id checks). A
/// non-finite yaw blinds the cone through the shared octant rule but still
/// evaluates occlusion, so the call stays `Ok`. Determinism: id-sorted
/// output, integer math only, no clock, no map order. Budget: one `visible`
/// ray per candidate over the solid count, zero heap use past the caller's
/// reused `out`.
pub fn fps_sight_set(
    world: &SightWorld,
    eye_m: [f32; 3],
    yaw: f32,
    candidates: &[u32],
    out: &mut Vec<u32>,
) -> Result<(), SightError> {
    if !eye_m.iter().all(|v| v.is_finite()) {
        return Err(SightError::NonFinitePoint);
    }
    out.clear();
    for id in candidates {
        let Ok(v) = super::sight::visible(world, eye_m, yaw, *id) else {
            continue;
        };
        if v.visible() {
            out.push(*id);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(())
}

/// Order-independent digest over one snapshot's visible set plus anchor.
///
/// Sorts a private copy, so any input order gives the same u64 for the same
/// anchor and members, and any flipped membership flips it. Snapshot
/// cadence only (10 Hz): the sort borrows nothing and allocates one small
/// vec per snapshot, never per entity per tick.
pub fn fps_sight_digest(anchor: u32, visible_sorted: &[u32]) -> u64 {
    let mut ids = visible_sorted.to_vec();
    ids.sort_unstable();
    let mut hash = Fnv64::new();
    hash.write(FPS_SIGHT_TAG);
    hash.write_u32(anchor);
    for id in &ids {
        hash.write_u32(*id);
    }
    hash.finish()
}

/// One interest snapshot for the tick consumer: the ascending visible set
/// in `out` plus its digest as the return.
///
/// The single non-test path through [`fps_sight_set`] and
/// [`fps_sight_digest`]: AI brains read membership out of `out` (chase only
/// what the set holds), net interest ships the intersection of its ring
/// selection with `out` (occlusion-aware culling), and replays compare the
/// digest. Same world, eye, yaw and candidates in any order give the same
/// bytes twice. Errors: as [`fps_sight_set`].
pub fn fps_snapshot(
    world: &SightWorld,
    anchor: u32,
    eye_m: [f32; 3],
    yaw: f32,
    candidates: &[u32],
    out: &mut Vec<u32>,
) -> Result<u64, SightError> {
    fps_sight_set(world, eye_m, yaw, candidates, out)?;
    Ok(fps_sight_digest(anchor, out))
}

#[cfg(test)]
mod tests {
    use super::super::sight::SightWorld;
    use super::super::Actor;
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn prop(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32, physical: bool) -> Actor {
        Actor {
            id,
            name: format!("p{id}"),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn target(id: u32, x: f32, z: f32) -> Actor {
        prop(id, x, 1.6, z, 1.0, 1.8, 1.0, true)
    }

    /// Watcher behind a wall at x = 5 m, eye (-6, 1.6, 0), facing +X:
    /// 20 hides behind the wall, 21 stands aside, 22 stands clear ahead,
    /// 23 sits behind the eye, 24 sits on the cone edge, 25 off it.
    fn watcher_world() -> SightWorld {
        SightWorld::build(&[
            prop(1, 5.0, 1.5, 0.0, 1.0, 3.0, 10.0, true),
            target(20, 10.0, 0.0),
            target(21, -6.0, 10.0),
            target(22, 2.0, 0.0),
            target(23, -10.0, 0.0),
            target(24, 2.0, 8.0),
            target(25, -2.0, 8.0),
        ])
    }

    const EYE: [f32; 3] = [-6.0, 1.6, 0.0];
    const YAW: f32 = 0.0;

    #[test]
    fn fps_combines_cone_and_occlusion() {
        // Oracle (fixed before the run): the wall keeps 20 out, the cone
        // keeps 21/23/25 out, so the set is exactly the clear pair.
        let world = watcher_world();
        let mut out = Vec::new();
        fps_snapshot(&world, 10, EYE, YAW, &[20, 21, 22, 23, 24, 25], &mut out)
            .expect("fixture snapshot runs");
        assert_eq!(out, vec![22, 24]);
    }

    #[test]
    fn fps_behind_viewer_is_unseen_despite_clear_ray() {
        // 23 has a clear ray but sits behind the eye: FOV gates LOS, so
        // the set stays empty and the digest still replays.
        let world = watcher_world();
        let mut out = Vec::new();
        let d = fps_snapshot(&world, 10, EYE, YAW, &[23], &mut out).expect("runs");
        assert!(out.is_empty());
        assert_eq!(d, fps_sight_digest(10, &out));
    }

    #[test]
    fn fps_order_never_moves_set_or_digest() {
        // Shuffled plus duplicated candidates report the same ascending set
        // and the same digest: input order never reaches state.
        let world = watcher_world();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let da =
            fps_snapshot(&world, 10, EYE, YAW, &[20, 21, 22, 23, 24, 25], &mut a).expect("runs");
        let db = fps_snapshot(
            &world,
            10,
            EYE,
            YAW,
            &[25, 24, 24, 23, 22, 21, 20, 22],
            &mut b,
        )
        .expect("runs");
        assert_eq!(a, b);
        assert_eq!(a, vec![22, 24]);
        assert_eq!(da, db, "order moved the digest");
    }

    #[test]
    fn fps_membership_flip_moves_digest() {
        // Dropping the visible pair flips the digest; the snapshot digest
        // always equals the digest over its own set.
        let world = watcher_world();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let da =
            fps_snapshot(&world, 10, EYE, YAW, &[20, 21, 22, 23, 24, 25], &mut a).expect("runs");
        let db = fps_snapshot(&world, 10, EYE, YAW, &[20, 21, 23, 25], &mut b).expect("runs");
        assert_eq!(a, vec![22, 24]);
        assert!(b.is_empty());
        assert_ne!(da, db, "flipped membership kept the digest");
        assert_eq!(da, fps_sight_digest(10, &a));
        assert_eq!(
            da,
            fps_sight_digest(10, &[24, 22]),
            "digest ignores input order"
        );
    }

    #[test]
    fn fps_failures_are_typed() {
        let world = watcher_world();
        // A non-finite eye refuses before touching the caller buffer.
        let mut out = vec![9u32];
        assert_eq!(
            fps_snapshot(&world, 10, [f32::NAN, 1.6, 0.0], YAW, &[22], &mut out),
            Err(SightError::NonFinitePoint)
        );
        assert_eq!(out, vec![9], "error leaves the buffer untouched");
        // Stale roster ids are skipped, never an error; use `visible` for
        // loud single-id checks.
        let mut out = Vec::new();
        fps_snapshot(&world, 10, EYE, YAW, &[22, 777], &mut out).expect("skips stale");
        assert_eq!(out, vec![22]);
        // Empty rosters report empty sets with a stable digest.
        let mut out = Vec::new();
        let d = fps_snapshot(&world, 10, EYE, YAW, &[], &mut out).expect("empty runs");
        assert!(out.is_empty());
        assert_eq!(d, fps_sight_digest(10, &[]));
        assert_eq!(d, fps_sight_digest(10, &[]));
    }

    #[test]
    fn fps_ai_chase_gates_on_membership() {
        // Brain shape: chase the target only while the snapshot holds it;
        // occlusion stops pursuit without any new query shape.
        let world = watcher_world();
        let mut out = Vec::new();
        fps_snapshot(&world, 10, EYE, YAW, &[22], &mut out).expect("runs");
        assert!(out.contains(&22), "clear sight starts pursuit");
        let mut out = Vec::new();
        fps_snapshot(&world, 10, EYE, YAW, &[20], &mut out).expect("runs");
        assert!(!out.contains(&20), "occlusion stops pursuit");
    }

    #[test]
    fn fps_net_prefilter_never_ships_occluded() {
        // Net shape: radius prefilter first (both within 20 m), then the
        // FPS set culls the occluded one before the snapshot ships.
        let world = watcher_world();
        let near: Vec<u32> = [20u32, 22]
            .into_iter()
            .filter(|id| {
                let b = world.box_of(*id).expect("fixture id");
                let cx = (b.min_x + b.max_x) / 2;
                let cz = (b.min_z + b.max_z) / 2;
                let dx = i64::from(cx) + 600; // eye x = -600 cm
                let dz = i64::from(cz);
                dx * dx + dz * dz <= 2_000i64 * 2_000
            })
            .collect();
        assert_eq!(near, vec![20, 22]);
        let mut out = Vec::new();
        fps_snapshot(&world, 7, EYE, YAW, &near, &mut out).expect("runs");
        assert_eq!(out, vec![22], "occluded id never ships");
    }

    #[test]
    fn fps_buffer_reuse_holds_capacity() {
        // The caller buffer is reused across snapshots: capacity set by the
        // first call never grows, so steady-state ticks allocate nothing.
        let world = watcher_world();
        let mut out = Vec::new();
        fps_snapshot(&world, 10, EYE, YAW, &[20, 21, 22, 23, 24, 25], &mut out).expect("runs");
        let cap = out.capacity();
        assert!(cap > 0);
        fps_snapshot(&world, 10, EYE, YAW, &[22], &mut out).expect("runs");
        assert_eq!(out, vec![22]);
        assert_eq!(out.capacity(), cap, "snapshot must reuse the caller buffer");
    }

    #[test]
    fn fps_sixty_four_viewers_replay() {
        // Scale shape: 64 fighters plus 500 AI soldiers as non-physical
        // roster ids (never block rays) over eight pillar solids; every
        // viewer snapshots the full roster and the digests replay twice.
        let mut actors = Vec::new();
        for i in 0..564u32 {
            let x = (i % 24) as f32 * 2.0 - 23.0;
            let z = (i / 24) as f32 * 2.0 - 23.0;
            actors.push(prop(100 + i, x, 1.6, z, 0.6, 1.8, 0.6, false));
        }
        for (k, (px, pz)) in [
            (-8.0f32, -8.0f32),
            (8.0, -8.0),
            (-8.0, 8.0),
            (8.0, 8.0),
            (-8.0, 0.0),
            (8.0, 0.0),
            (0.0, -8.0),
            (0.0, 8.0),
        ]
        .iter()
        .enumerate()
        {
            actors.push(prop(1 + k as u32, *px, 1.5, *pz, 1.0, 3.0, 1.0, true));
        }
        let world = SightWorld::build(&actors);
        let roster: Vec<u32> = (100..664).collect();
        assert!(roster.len() <= FPS_MAX_CANDIDATES);
        let run = || {
            let mut digests = Vec::with_capacity(64);
            let mut total = 0usize;
            let mut out = Vec::new();
            for v in 0..64u32 {
                let eye = [
                    (v % 8) as f32 * 5.0 - 17.5,
                    1.6,
                    (v / 8) as f32 * 5.0 - 17.5,
                ];
                let d = fps_snapshot(&world, v, eye, 0.0, &roster, &mut out).expect("runs");
                total += out.len();
                digests.push(d);
            }
            (digests, total)
        };
        let (first, total) = run();
        assert!(total > 0, "nobody sees anybody");
        assert_eq!(run().0, first, "viewer digests drifted mid-run");
        assert_eq!(run(), (first, total));
    }
}
