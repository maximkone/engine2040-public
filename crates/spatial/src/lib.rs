//! spatial: deterministic scene reports. Same actors, any order -> same hash. Owns no sim state.

pub mod afford;
pub mod brief;
mod checks;
pub mod compose;
pub mod diff;
/// Cover exposure per tile: threat-ray fans over sight geometry.
pub mod exposure;
/// Stray-robust scene extent (SPX8).
pub mod extent;
/// Brief hygiene: fold look-alikes, state the extent, say the problems (SPF10).
pub mod fold;
/// FPS visibility: per-viewer FOV plus occlusion sets with interest digests.
pub mod fps_visibility;
/// Closed-form camera frame fit in integer cm (SPF2).
pub mod frame;
/// Deterministic fuzz harness: seeded op streams with first-divergence receipts.
pub mod fuzz;
mod index;
pub mod index3;
mod intmath;
pub mod isovist;
/// Integer-cm 3D k-d tree: median-split nearest and k-neighbor queries.
pub mod kdtree;
/// Surface gap, push direction and pairwise measure (SPF4, SPX11).
pub mod measure;
pub mod navcheck;
/// Distance, bearing and gap behind a directional answer (SPR2).
pub mod pair;
pub mod place;
pub mod probe;
pub mod relation;
pub mod repair;
/// Rooms as flood-filled regions of the wall boxes, with exits (SPR9).
pub mod rooms;
/// Free-space sampler: Poisson-disc placements, largest open spot.
pub mod sample;
/// Whole-scene unit-mismatch detector: median object size vs layout extent.
pub mod scale;
/// Pair sight with the occluder (SPR7) and why a thing is hidden (SPX6).
pub mod sees;
/// CPU sense verbs in plain text: view, lidar, map and one pixel ray (SP69).
pub mod sense_text;
/// Sight and reach: line of sight, visibility, cover, reachability.
pub mod sight;
/// Slide sweep (SPF6) and ground under a point (SPX5).
pub mod slide;
/// Repair-loop eval: ten seeded scenes through the repair loop.
pub mod spatial_eval;
pub mod staged;
/// Seated joints and the support chain (SPX2).
pub mod support;
/// Offline open-vocabulary label lift into node tag proposals (SP46).
pub mod tag_lift;
/// Finding triage by component role and z-fight ranking (SPF8).
pub mod triage;
/// Verdict allow-list and the family learner (SPF9).
pub mod verdict;
/// Why a thing is not seen from an eye: the first failing reason (SPX6).
pub mod why_hidden;

pub use brief::{brief, brief_stats, brief_text, estimate_tokens, Brief, BriefStats, Group};
pub use checks::{
    run_checks, run_checks_eps, run_spatial, OPT_IN_CHECKS, SPATIAL_CHECKS, ZFIGHT_EPS_CM,
};
pub use diff::{describe, diff_graph, dry_run_diff, summarize, Edge, SemanticDiff};
pub use frame::{frame_fit, FrameError, FrameFit, View, MIN_DISTANCE_CM};
pub use index::{
    density_grid, nearest, overlap_pairs, pos_known, size_known, sweep_pairs, to_cm, xyz_pairs,
    CmRect,
};
pub use navcheck::{Door, NavView, Tile};
pub use place::{
    enumerate_support_surfaces, solve, ConflictSet, Intent, ObjectRef, PlaceError, RepairHint,
    SupportKind, SupportSurface, Target, TransformOut, Wall, Want, DEFAULT_OBJECT_GAP_CM,
    DEFAULT_POINT_DIST_CM, DEFAULT_WALL_GAP_CM, GRID_CM, YAW_STEPS,
};
pub use relation::{build, digest, rebuild_cached, Graph, Layer, Relation};
pub use repair::{
    apply_preview, suggest, Proposal, Repair, RepairKind, StagedPreview, StructuredRefusal,
};
pub use sample::{
    largest_open_spot, sample_actor, sample_free_space, Sample, SampleError, SamplerConfig,
    SAMPLE_ACTOR_BASE,
};
#[cfg(test)]
pub use staged::full_check_timed;
pub use staged::{actors_from_snapshot, check_staged, staged_to_actors, SnapshotRow, StagedActor};
pub use tag_lift::{lift_captures, lift_label};

/// One scene actor snapshot. Units: meters, f32 in, integer-cm inside.
#[derive(Debug, Clone, PartialEq)]
pub struct Actor {
    pub id: u32,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    pub parent: Option<u32>,
    pub physical: bool,
    /// Horizontal velocity m/s. Swept by the `sweep` check, ignored elsewhere.
    pub vx: f32,
    pub vz: f32,
    /// Facing yaw radians. Read by `accessFront` (octant-snapped, libm-free).
    pub yaw: f32,
}

/// One finding. `b == u32::MAX` means single-actor finding.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub check: &'static str,
    pub a: u32,
    pub b: u32,
    pub pct: f32,
    pub severity: &'static str,
}

impl Finding {
    /// Brief row `finding <check> <a> <b|-> <severity> <pct>`, no newline;
    /// single-actor findings render `b` as `-`.
    pub fn row(&self) -> String {
        let b = if self.b == u32::MAX {
            "-".to_owned()
        } else {
            self.b.to_string()
        };
        format!(
            "finding {} {} {} {} {}",
            self.check, self.a, b, self.severity, self.pct
        )
    }
}

/// Full report: frozen-order findings + FNV-1a hash over canonical bytes.
#[derive(Debug, Clone)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub hash: u64,
}

impl Report {
    pub fn verdict(&self) -> &'static str {
        if self.findings.iter().any(|f| f.severity == "red") {
            "red"
        } else if self.findings.is_empty() {
            "clean"
        } else {
            "yellow"
        }
    }

    pub fn count(&self, check: &str) -> usize {
        self.findings.iter().filter(|f| f.check == check).count()
    }
}

/// Whole-catalog run over the 14 offline checks.
/// Nav phase-2 checks need a bake view: use `run_spatial` with `Some(view)`.
/// Unknown check names -> Err(legal list), never empty pass.
pub fn run(actors: &[Actor]) -> Report {
    let offline: Vec<&str> = SPATIAL_CHECKS
        .iter()
        .copied()
        .filter(|c| !checks::NAV_CHECKS.contains(c))
        .collect();
    run_checks(actors, &offline).expect("offline catalog is always legal")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn clean_scene_no_findings_hash_stable() {
        let actors = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 10.0, 10.0)];
        let r1 = run(&actors);
        let r2 = run(&actors);
        assert!(r1.findings.is_empty());
        assert_eq!(r1.hash, r2.hash);
        assert_eq!(r1.verdict(), "clean");
    }

    #[test]
    fn overlap_pair_found_with_pct() {
        let actors = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 0.5, 0.0)];
        let r = run(&actors);
        assert_eq!(r.count("overlap"), 1);
        let f = &r.findings[0];
        assert_eq!(f.severity, "red");
        assert!(f.pct > 0.0);
        assert_eq!(r.verdict(), "red");
    }

    #[test]
    fn input_order_does_not_move_hash() {
        let a = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 0.5, 0.0)];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(run(&a).hash, run(&b).hash);
    }

    #[test]
    fn floating_and_support_fire() {
        let f = Actor {
            y: 5.0,
            ..resting(0, "fly", 0.0, 0.0)
        };
        let r = run(std::slice::from_ref(&f));
        assert_eq!(r.count("floating"), 1);
        assert_eq!(r.count("support"), 1);
    }

    #[test]
    fn bounds_and_oob99_fire() {
        let f = Actor {
            x: 150.0,
            z: 150.0,
            ..resting(0, "far", 0.0, 0.0)
        };
        let r = run(std::slice::from_ref(&f));
        assert_eq!(r.count("bounds"), 1);
        assert_eq!(r.count("oob99"), 1);
    }

    #[test]
    fn nan_position_is_loud_not_panic() {
        let f = Actor {
            x: f32::NAN,
            ..resting(0, "nan", 0.0, 0.0)
        };
        let r = run(std::slice::from_ref(&f));
        assert_eq!(r.count("unknown-position"), 1);
    }

    #[test]
    fn bad_check_name_returns_legal_list() {
        let actors = vec![resting(0, "a", 0.0, 0.0)];
        let e = run_checks(&actors, &["nope"]).unwrap_err();
        assert!(e.contains(&"overlap"));
        assert!(e.contains(&"bounds"));
    }

    #[test]
    fn sweep_fires_before_contact() {
        let mover = Actor {
            vx: 2.0,
            ..resting(0, "mover", 0.0, 0.0)
        };
        let wall = resting(1, "wall", 1.5, 0.0);
        let r = run(&[mover, wall]);
        assert_eq!(r.count("overlap"), 0);
        assert_eq!(r.count("sweep"), 1);
        assert_eq!(
            r.findings
                .iter()
                .find(|f| f.check == "sweep")
                .unwrap()
                .severity,
            "yellow"
        );
    }

    #[test]
    fn density_fires_on_crowded_cell() {
        let actors: Vec<Actor> = (0..9)
            .map(|i| resting(i, &format!("n{i}"), f32::from(i as u8) * 0.1, 0.0))
            .collect();
        let r = run(&actors);
        assert_eq!(r.count("overlapDensity"), 1);
        let r2 = run(&actors);
        assert_eq!(r.hash, r2.hash);
    }

    #[test]
    fn grammar_rejects_bad_names() {
        let bad = resting(0, "Bad Name!", 0.0, 0.0);
        let good = resting(1, "good_name_01", 10.0, 10.0);
        let r = run(&[bad, good]);
        assert_eq!(r.count("grammar"), 1);
        assert_eq!(
            r.findings.iter().find(|f| f.check == "grammar").unwrap().a,
            0
        );
    }

    #[test]
    fn nearest_is_sorted_and_stable() {
        let actors = vec![
            resting(0, "a", 0.0, 0.0),
            resting(1, "b", 3.0, 0.0),
            resting(2, "c", 1.0, 0.0),
        ];
        assert_eq!(crate::nearest(&actors, 0, 2), vec![2, 1]);
        let mut rev = actors.clone();
        rev.reverse();
        assert_eq!(crate::nearest(&rev, 0, 2), vec![2, 1]);
    }

    fn nav_view_two_tiles() -> crate::NavView {
        // Walkable tiles (0,0) and (2,0); gap tile (1,0) unwalkable.
        let mut tiles = std::collections::BTreeMap::new();
        tiles.insert((0, 0), crate::Tile { walk: true });
        tiles.insert((2, 0), crate::Tile { walk: true });
        crate::NavView {
            tiles,
            links: vec![],
            flow: std::collections::BTreeMap::new(),
            doors: std::collections::BTreeMap::new(),
            spawn_id: 0,
        }
    }

    #[test]
    fn navconnect_fires_on_island() {
        // Actor 0 on tile (0,0); actor 1 on tile (2,0): disconnected.
        let actors = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 2.0, 0.0)];
        let r = crate::run_spatial(&actors, &["navConnect"], Some(&nav_view_two_tiles())).unwrap();
        assert_eq!(r.count("navConnect"), 1);
        assert_eq!(r.findings[0].a, 1);
    }

    #[test]
    fn navconnect_clean_when_linked() {
        let actors = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 2.0, 0.0)];
        let mut nv = nav_view_two_tiles();
        nv.links.push((0, 1));
        let r = crate::run_spatial(&actors, &["navConnect"], Some(&nv)).unwrap();
        assert_eq!(r.count("navConnect"), 0);
    }

    #[test]
    fn nav_needs_view_refuses_loud() {
        let actors = vec![resting(0, "a", 0.0, 0.0)];
        let e = crate::run_spatial(&actors, &["navConnect"], None).unwrap_err();
        assert!(e.contains(&"needs-nav-view"));
    }

    #[test]
    fn flowchart_fires_against_flow() {
        // Flow +X on tile (0,0); actor drives -X fast.
        let mut nv = nav_view_two_tiles();
        nv.flow.insert((0, 0), (1, 0));
        let mut a = resting(0, "a", 0.0, 0.0);
        a.vx = -2.0;
        let r = crate::run_spatial(std::slice::from_ref(&a), &["flowPath"], Some(&nv)).unwrap();
        assert_eq!(r.count("flowPath"), 1);
        a.vx = 2.0;
        let r2 = crate::run_spatial(std::slice::from_ref(&a), &["flowPath"], Some(&nv)).unwrap();
        assert_eq!(r2.count("flowPath"), 0);
    }

    #[test]
    fn accessfront_blocked_and_clear() {
        // Actor 0 faces +X (yaw 0); wall overlapping its nose probe.
        let me_ = resting(0, "me", 0.0, 0.0);
        let wall = resting(1, "wall", 1.2, 0.0);
        let nv = nav_view_two_tiles();
        let r = crate::run_spatial(&[me_, wall], &["accessFront"], Some(&nv)).unwrap();
        assert_eq!(r.count("accessFront"), 1);
        let me2 = resting(0, "me", 0.0, 0.0);
        let far = resting(1, "far", 50.0, 50.0);
        let r2 = crate::run_spatial(&[me2, far], &["accessFront"], Some(&nv)).unwrap();
        assert_eq!(r2.count("accessFront"), 0);
    }

    #[test]
    fn doorclear_narrow_red_wide_clean() {
        let mut nv = nav_view_two_tiles();
        nv.doors.insert(
            7,
            crate::Door {
                width_cm: 80,
                two_way: false,
            },
        );
        let actors = vec![resting(0, "a", 0.0, 0.0), resting(7, "door_main", 5.0, 5.0)];
        let r = crate::run_spatial(&actors, &["doorClear"], Some(&nv)).unwrap();
        assert_eq!(r.count("doorClear"), 1);
        assert_eq!(r.findings[0].severity, "red");
        nv.doors.insert(
            7,
            crate::Door {
                width_cm: 200,
                two_way: false,
            },
        );
        let r2 = crate::run_spatial(&actors, &["doorClear"], Some(&nv)).unwrap();
        assert_eq!(r2.count("doorClear"), 0);
    }

    #[test]
    fn xyz_pairs_catches_stacked_volumes() {
        let low = resting(0, "low", 5.0, 5.0);
        let mut high = resting(1, "high", 5.0, 5.0);
        high.y = 1.6; // min_y=1.1 > low max_y=1.0: no XYZ hit
        assert!(crate::xyz_pairs(&[low.clone(), high.clone()]).is_empty());
        high.y = 1.4; // overlap band 0.9..1.0
        assert_eq!(crate::xyz_pairs(&[low, high]), vec![(0, 1)]);
    }
}
