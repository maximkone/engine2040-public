//! AM32: world geometry in the motion-match cost.
//!
//! The acceptance fixture is a 3 m corridor with a 1 m arch 6 m down it. A
//! 600-tick tape walks at 1.5 m/s holding a straight-ahead wish; without the
//! world penalty the pick is always the straight clip and the capsule path
//! grinds through the arch pier, with the world penalty the pick veers through
//! the 1 m opening and touches nothing. The open floor must pick exactly what
//! the plain search picks and leave the database digest where it was.

use super::*;
use crate::clip::TICK_DT;

/// Walk speed of the tape (m/s).
const WALK_MPS: f32 = 1.5;
/// Tape length: the row's 600 ticks.
const TAPE_TICKS: usize = 600;
/// Capsule radius the collisions are judged with (m).
const CAPSULE_R_M: f32 = 0.35;
/// Steering margin the matcher plans with on top of the capsule (m), so a
/// threaded gap keeps clearance instead of grazing the pier.
const STEER_MARGIN_M: f32 = 0.05;
/// Corridor width is `2 * CORRIDOR_HALF_M` = 3 m.
const CORRIDOR_HALF_M: f32 = 1.5;
/// Arch opening, 1 m wide and centred on the corridor.
const ARCH_GAP_M: f32 = 1.0;
/// Arch distance down the corridor (m).
const ARCH_Z_M: f32 = 6.0;
/// Arch thickness along the walk (m).
const ARCH_THICK_M: f32 = 0.4;
/// The walk starts left of the arch opening, so the straight path meets a pier.
const START_X_M: f32 = -0.45;

/// Candidate trajectories in meters, root-relative, horizon order:
/// 0 straight, 1-3 veer right 6/12/25 deg, 4-5 veer left 6/12, 6 stop.
const CANDIDATES: [[[f32; 2]; TRAJ_N]; 7] = [
    [[0.0, 0.375], [0.0, 0.75], [0.0, 1.5]],
    [
        [0.039_198, 0.372_946],
        [0.078_396, 0.745_891],
        [0.156_793, 1.491_782],
    ],
    [
        [0.077_967, 0.366_805],
        [0.155_934, 0.733_611],
        [0.311_868, 1.467_222],
    ],
    [
        [0.158_482, 0.339_866],
        [0.316_964, 0.679_731],
        [0.633_927, 1.359_462],
    ],
    [
        [-0.039_198, 0.372_946],
        [-0.078_396, 0.745_891],
        [-0.156_793, 1.491_782],
    ],
    [
        [-0.077_967, 0.366_805],
        [-0.155_934, 0.733_611],
        [-0.311_868, 1.467_222],
    ],
    [[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
];

/// Fixed pose of the walker: the wish is forward at walk speed and both feet
/// are planted, so every candidate ties on the pose lanes and only the
/// trajectory lanes and the world penalty can separate them.
fn pose() -> PoseSample {
    PoseSample {
        hip_vel: [0.0, WALK_MPS],
        foot_l: [0.11, 0.05, 0.0],
        foot_r: [-0.11, 0.05, 0.0],
        contact: 3,
    }
}

/// The candidate library: clip id equals its row in [`CANDIDATES`].
fn fixture_db() -> MotionDb {
    let mut entries = Vec::with_capacity(CANDIDATES.len());
    for (clip, points) in CANDIDATES.iter().enumerate() {
        entries.push(Entry {
            feat: make_feature(&pose(), &Trajectory { points: *points }).expect("fixture valid"),
            clip: u16::try_from(clip).unwrap_or(0),
            tick: 0,
            contact: 3,
        });
    }
    bake(entries).expect("the candidate library bakes")
}

/// The tape's query: a straight-ahead future at walk speed, no history.
fn straight_query() -> Query {
    build_query(
        &pose(),
        &Trajectory {
            points: CANDIDATES[0],
        },
        NO_HIST,
        &default_weights(),
        cost_units(0.01),
        cost_units(0.04),
    )
    .expect("fixture query valid")
}

/// The corridor: two side walls 3 m apart, then an arch across it with a 1 m
/// opening centred on the corridor.
fn corridor_walls() -> WallSet {
    let mut walls = WallSet::new();
    let (half, gap) = (CORRIDOR_HALF_M, ARCH_GAP_M / 2.0);
    let arch = (ARCH_Z_M, ARCH_Z_M + ARCH_THICK_M);
    for (a, b) in [
        ([-half - 0.4, -2.0], [-half, 20.0]),
        ([half, -2.0], [half + 0.4, 20.0]),
        ([-half, arch.0], [-gap, arch.1]),
        ([gap, arch.0], [half, arch.1]),
    ] {
        walls
            .push(Wall::from_meters(a, b))
            .expect("the corridor holds four walls");
    }
    walls
}

/// Independent capsule oracle: sample the move segment 1/64 at a time and
/// compare the squared distance from the sample to each un-inflated wall with
/// the squared capsule radius. Sampling plus Euclidean distance is a different
/// route to the answer than the slab clip the penalty uses.
fn capsule_path_hits(a: [i32; 2], b: [i32; 2], walls: &[Wall], pad: i32) -> bool {
    const SAMPLES: i64 = 64;
    let pad = i64::from(pad);
    let pad_sq = pad * pad;
    let dx = i64::from(b[0]) - i64::from(a[0]);
    let dz = i64::from(b[1]) - i64::from(a[1]);
    for i in 0..=SAMPLES {
        let x = i64::from(a[0]) + dx * i / SAMPLES;
        let z = i64::from(a[1]) + dz * i / SAMPLES;
        for wall in walls {
            let cx = x.clamp(i64::from(wall.min[0]), i64::from(wall.max[0]));
            let cz = z.clamp(i64::from(wall.min[1]), i64::from(wall.max[1]));
            let (ex, ez) = (x - cx, z - cz);
            if ex * ex + ez * ez <= pad_sq {
                return true;
            }
        }
    }
    false
}

/// One tick of movement along the picked clip's first horizon point, at the
/// walk speed. A stop clip has no direction and moves nothing.
fn step_for(point: [f32; 2]) -> [f32; 2] {
    let len = (point[0] * point[0] + point[1] * point[1]).sqrt();
    if len <= 0.0 {
        return [0.0, 0.0];
    }
    [
        point[0] / len * WALK_MPS * TICK_DT,
        point[1] / len * WALK_MPS * TICK_DT,
    ]
}

/// What one tape run did: ticks whose capsule move touched a wall, where the
/// walker finished (Q10) and the clip picked on each tick.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Tape {
    hits: usize,
    end: [i32; 2],
    picks: Vec<usize>,
}

/// Run the 600-tick tape. `aware` selects the world search and carries the
/// walls, radius and hit cost; its root is replaced by the live root each tick.
/// `None` is the plain [`search`], the "before" run.
fn walk(db: &MotionDb, query: &Query, aware: Option<&WorldQuery>) -> Tape {
    let walls = corridor_walls();
    let pad = q(CAPSULE_R_M);
    let mut root = [START_X_M, 0.0];
    let mut tape = Tape {
        hits: 0,
        end: [0; 2],
        picks: Vec::with_capacity(TAPE_TICKS),
    };
    for _ in 0..TAPE_TICKS {
        let root_q = [q(root[0]), q(root[1])];
        let picked = match aware {
            Some(world) => {
                let mut frame = *world;
                frame.root = root_q;
                search_in_world(db, query, &frame).expect("world search").0
            }
            None => search(db, query).expect("plain search").0,
        };
        let clip = usize::from(db.entries[picked.index].clip);
        tape.picks.push(clip);
        let step = step_for(CANDIDATES[clip][0]);
        root[0] += step[0];
        root[1] += step[1];
        let next_q = [q(root[0]), q(root[1])];
        if capsule_path_hits(root_q, next_q, walls.walls(), pad) {
            tape.hits += 1;
        }
    }
    tape.end = [q(root[0]), q(root[1])];
    tape
}

/// The world the tape plans with: the corridor, the capsule plus steering
/// margin, and 2 m^2 of cost per blocked horizon (well above the feature gap
/// between neighbouring candidates).
fn corridor_world(root: [i32; 2]) -> WorldQuery {
    WorldQuery::new(
        root,
        q(CAPSULE_R_M + STEER_MARGIN_M),
        corridor_walls(),
        cost_units(2.0),
    )
}

#[test]
fn a_wall_ahead_picks_a_turning_candidate() {
    let db = fixture_db();
    let query = straight_query();
    // 4.2 m down the corridor: the 1 s horizon already reaches the arch, the
    // nearer horizons do not.
    let world = corridor_world([q(START_X_M), q(4.2)]);
    let plain = search(&db, &query).expect("plain search");
    let aware = search_in_world(&db, &query, &world).expect("world search");
    assert_eq!(usize::from(db.entries[plain.0.index].clip), 0);
    assert_ne!(
        usize::from(db.entries[aware.0.index].clip),
        0,
        "a wall ahead must not keep the straight clip"
    );
    assert!(world_penalty(&db.entries[plain.0.index].feat, &world) >= world.hit_cost);
    assert_eq!(
        world_penalty(&db.entries[aware.0.index].feat, &world),
        0,
        "the picked candidate must be clear of the wall"
    );
    assert_eq!(
        aware,
        search_in_world(&db, &query, &world).expect("repeat"),
        "the same query and world pick the same clip twice"
    );
}

#[test]
fn world_penalty_is_one_hit_cost_per_horizon() {
    // A wall 0.4 m to 0.6 m ahead, inflated by a 0.1 m capsule: 0.3 m to 0.7 m.
    let mut walls = WallSet::new();
    walls
        .push(Wall::from_meters([-1.0, 0.4], [1.0, 0.6]))
        .expect("one wall");
    let world = WorldQuery::new([0, 0], q(0.1), walls, cost_units(1.0));
    let hit = cost_units(1.0);
    let feat = |points: [[f32; 2]; TRAJ_N]| {
        make_feature(&pose(), &Trajectory { points }).expect("fixture valid")
    };
    // Three horizons cross the wall: three hits.
    assert_eq!(world_penalty(&feat(CANDIDATES[0]), &world), 3 * hit);
    // One horizon crosses it, two stay short: one hit.
    let one = [[0.0, 0.35], [0.0, 0.2], [0.0, 0.1]];
    assert_eq!(world_penalty(&feat(one), &world), hit);
    // Nothing crosses it, and an open floor never charges.
    let clear = [[0.0, 0.1], [0.0, 0.2], [0.0, 0.25]];
    assert_eq!(world_penalty(&feat(clear), &world), 0);
    assert_eq!(
        world_penalty(&feat(CANDIDATES[0]), &WorldQuery::open_floor([0, 0])),
        0
    );
    // A zero hit cost is an open floor too.
    let free = WorldQuery::new([0, 0], q(0.1), world.walls, 0);
    assert_eq!(world_penalty(&feat(CANDIDATES[0]), &free), 0);
}

#[test]
fn corridor_arch_tape_never_touches_a_wall() {
    let db = fixture_db();
    let query = straight_query();
    let digest = db_hash(&db);
    let plain = walk(&db, &query, None);
    let aware = walk(&db, &query, Some(&corridor_world([0, 0])));
    println!(
        "AM32 corridor arch: plain pick touches a wall on {} of {} ticks, world pick on {}",
        plain.hits, TAPE_TICKS, aware.hits
    );
    assert!(
        plain.hits > 0,
        "the fixture must fail without the world cost"
    );
    assert_eq!(aware.hits, 0, "the world pick must clear the 1 m arch");
    assert_eq!(
        walk(&db, &query, Some(&corridor_world([0, 0]))),
        aware,
        "the world tape replays identically"
    );
    assert_eq!(
        walk(&db, &query, None),
        plain,
        "the plain tape replays identically"
    );
    // 600 ticks at 1.5 m/s is 15 m: the walker threads the arch and walks on.
    assert!(
        aware.end[1] > q(ARCH_Z_M + 5.0),
        "the walker must pass the arch, ended at {:?}",
        aware.end
    );
    // It steers inside the 1 m opening, not through a pier.
    let gap_edge = q(ARCH_GAP_M / 2.0) - q(CAPSULE_R_M);
    assert!(
        aware.end[0].abs() <= gap_edge,
        "the walker left the arch opening: {:?}",
        aware.end
    );
    assert!(
        aware.picks.iter().any(|clip| *clip != 0),
        "the tape must steer to thread the arch"
    );
    // Open floor: the same clip every tick, and the database digest is where
    // it was, because the world cost never touches the baked entries.
    let open = walk(
        &db,
        &query,
        Some(&WorldQuery::open_floor([q(START_X_M), 0])),
    );
    assert_eq!(
        open.picks, plain.picks,
        "an open floor must not move the pick"
    );
    assert_eq!(
        db_hash(&db),
        digest,
        "the world cost never touches the database"
    );
}

#[test]
fn walls_beyond_every_horizon_are_free() {
    // A wall 20 m ahead cannot be reached by a 1.5 m horizon, so the pick is
    // the plain pick for every candidate query.
    let db = fixture_db();
    let mut walls = WallSet::new();
    walls
        .push(Wall::from_meters([-2.0, 20.0], [2.0, 21.0]))
        .expect("one far wall");
    let world = WorldQuery::new([0, 0], q(CAPSULE_R_M), walls, cost_units(2.0));
    for points in CANDIDATES {
        let query = build_query(
            &pose(),
            &Trajectory { points },
            NO_HIST,
            &default_weights(),
            cost_units(0.01),
            cost_units(0.04),
        )
        .expect("candidate query valid");
        assert_eq!(
            search_in_world(&db, &query, &world)
                .expect("world search")
                .0,
            search(&db, &query).expect("plain search").0
        );
    }
}

#[test]
fn bad_world_is_refused() {
    let db = fixture_db();
    let query = straight_query();
    let world = corridor_world([0, 0]);
    let refused = |world: &WorldQuery| search_in_world(&db, &query, world);
    let mut bad = world;
    bad.radius = -1;
    assert_eq!(refused(&bad), Err(MatchError::BadQuery("world radius")));
    bad = world;
    bad.radius = Q_MAX + 1;
    assert_eq!(refused(&bad), Err(MatchError::BadQuery("world radius")));
    bad = world;
    bad.hit_cost = PENALTY_MAX + 1;
    assert_eq!(refused(&bad), Err(MatchError::BadQuery("world penalty")));
    bad = world;
    bad.root = [Q_MAX + 1, 0];
    assert_eq!(refused(&bad), Err(MatchError::BadQuery("world root")));
    bad = world;
    let mut walls = WallSet::new();
    walls
        .push(Wall {
            min: [10, 0],
            max: [0, 10],
        })
        .expect("one wall");
    bad.walls = walls;
    assert_eq!(refused(&bad), Err(MatchError::BadQuery("world wall")));
    let mut full = WallSet::new();
    for _ in 0..WALL_MAX {
        full.push(Wall::from_meters([0.0, 0.0], [1.0, 1.0]))
            .expect("the set holds WALL_MAX walls");
    }
    assert_eq!(
        full.push(Wall::from_meters([0.0, 0.0], [1.0, 1.0])),
        Err(MatchError::BadQuery("world walls"))
    );
}
