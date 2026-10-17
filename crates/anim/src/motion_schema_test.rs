//! LM4: the LM1 feature schema is frozen and the per-character search cost
//! has a kill bar. Test-only. The schema lines come from the live constants
//! and the live lane packing; any drift fails with a line diff. The cost bar
//! counts integer lane ops (`SearchReceipt::lane_ops`), exact on every
//! machine, so it needs no wall clock; PF1 posts the milliseconds.

use crate::motion_match::{
    bake, build_query, default_weights, make_feature, search, Entry, MotionDb, PoseSample, Query,
    Trajectory, FEAT_N, HORIZONS_S, MAX_WEIGHT, NO_HIST, POSE_LANES, Q, Q_MAX, TRAJ_N,
};
use engine_core::CounterCursor;

/// The frozen LM1 schema. Changing any line moves every receipt on file:
/// bump it only with a deliberate row.
const FROZEN: &[&str] = &[
    "feat_n=14 pose_lanes=8 traj_n=3",
    "q=1024 q_max=8388608 max_weight=4096",
    "horizons_s=0.25,0.5,1",
    "weights=1,1,1,1,1,1,1,1,2,2,2,2,2,2",
    "pack=1,2,3,4,5,6,7,8,9,10,11,12,13,14",
];

/// Schema lines from the live constants and the live lane packing: a probe
/// pose holds `k / Q` in lane `k`, so `make_feature` returns `k` where lane
/// `k` really lands.
fn live() -> Vec<String> {
    let k = |n: i32| n as f32 / Q as f32;
    let pose = PoseSample {
        hip_vel: [k(1), k(2)],
        foot_l: [k(3), k(4), k(5)],
        foot_r: [k(6), k(7), k(8)],
        contact: 0,
    };
    let traj = Trajectory {
        points: [[k(9), k(10)], [k(11), k(12)], [k(13), k(14)]],
    };
    let pack = make_feature(&pose, &traj).expect("probe valid");
    let join = |v: Vec<String>| v.join(",");
    vec![
        format!("feat_n={FEAT_N} pose_lanes={POSE_LANES} traj_n={TRAJ_N}"),
        format!("q={Q} q_max={Q_MAX} max_weight={MAX_WEIGHT}"),
        format!(
            "horizons_s={}",
            join(HORIZONS_S.iter().map(|h| h.to_string()).collect())
        ),
        format!(
            "weights={}",
            join(default_weights().iter().map(|w| w.to_string()).collect())
        ),
        format!(
            "pack={}",
            join(pack.iter().map(|v| v.to_string()).collect())
        ),
    ]
}

/// Line diff of `frozen` against `now`: `-` frozen, `+` live; empty when equal.
fn diff(frozen: &[&str], now: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..frozen.len().max(now.len()) {
        let (a, b) = (frozen.get(i), now.get(i));
        if a.map(|s| s.to_string()) != b.cloned() {
            out.extend(a.map(|s| format!("-{s}")));
            out.extend(b.map(|s| format!("+{s}")));
        }
    }
    out
}

#[test]
fn lm1_schema_is_frozen() {
    let d = diff(FROZEN, &live());
    assert!(
        d.is_empty(),
        "LM1 feature schema changed:\n{}",
        d.join("\n")
    );
}

#[test]
fn a_schema_change_refuses_with_a_diff() {
    let mut now = live();
    now[0] = now[0].replace("traj_n=3", "traj_n=4");
    now.swap(3, 4);
    let d = diff(FROZEN, &now);
    assert_eq!(d.len(), 6, "{d:?}");
    assert!(d[0].starts_with("-feat_n=14") && d[1].starts_with("+feat_n=14"));
    assert!(
        d[2].starts_with("-weights") && d[3].starts_with("+pack"),
        "{d:?}"
    );
}

/// 1024 seeded entries, 16 clips of 64 ticks; features span +-4 m.
fn seeded_db() -> MotionDb {
    let mut c = CounterCursor::new(2040);
    let span = 8 * Q as u64;
    let entries = (0..1024u32)
        .map(|i| {
            let mut feat = [0i32; FEAT_N];
            for lane in feat.iter_mut() {
                *lane = c.next_range(0, span) as i32 - 4 * Q;
            }
            Entry {
                feat,
                clip: (i / 64) as u16,
                tick: (i % 64) as u16,
                contact: c.next_range(0, 4) as u8,
            }
        })
        .collect();
    bake(entries).expect("seeded db")
}

fn seeded_query(seed: u64, db: &MotionDb) -> Query {
    let mut c = CounterCursor::new(seed);
    let e = db.entries[c.next_range(0, db.entries.len() as u64) as usize];
    // A near-miss of a real entry, the way a continuing character queries.
    let mut q = build_query(
        &PoseSample {
            hip_vel: [0.0; 2],
            foot_l: [0.0; 3],
            foot_r: [0.0; 3],
            contact: e.contact,
        },
        &Trajectory {
            points: [[0.0; 2]; TRAJ_N],
        },
        NO_HIST,
        &default_weights(),
        0,
        0,
    )
    .expect("query");
    q.feat = e.feat;
    q.feat[0] += c.next_range(0, 64) as i32;
    q
}

/// Kill bar: the accelerated search keeps at least half of the brute-force
/// work out. A search whose bound stops cutting costs `entries * FEAT_N`.
fn lane_op_bar(db: &MotionDb) -> u64 {
    (db.entries.len() * FEAT_N / 2) as u64
}

#[test]
fn search_cost_stays_under_the_kill_bar_for_eight_fighters() {
    let db = seeded_db();
    let bar = lane_op_bar(&db);
    let mut total = 0u64;
    for fighter in 0..8u64 {
        let (hit, receipt) = search(&db, &seeded_query(100 + fighter, &db)).expect("search");
        assert!(
            receipt.cost_per_character() <= bar,
            "fighter {fighter}: {receipt:?} past {bar}"
        );
        // Replay x2: the same query spends the same ops on the same pick.
        let again = search(&db, &seeded_query(100 + fighter, &db)).expect("search");
        assert_eq!((hit, receipt), again);
        total += receipt.cost_per_character();
    }
    assert!(total <= 8 * bar, "8-fighter tick {total} past {}", 8 * bar);
}

#[test]
fn a_search_without_the_bound_fails_the_bar() {
    let db = seeded_db();
    let brute = (db.entries.len() * FEAT_N) as u64;
    assert!(
        brute > lane_op_bar(&db),
        "the bar must reject a brute-force scan"
    );
}
