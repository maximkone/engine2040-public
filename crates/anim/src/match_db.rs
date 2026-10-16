//! match_db: baked motion-match pose DB + trajectory query. Sorted bake, linear scan, lowest-index wins.

use engine_core::Fnv64;

/// Full feature dim: 15 pose + 12 trajectory (3 pos2D + 3 dir2D).
pub const FEAT_N: usize = 27;
/// No-history marker for [`Query::curr`].
pub const NO_HIST: u32 = u32::MAX;

fn san(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Build 27-D feature: hip vel, foot pos/vel (local), future traj pos/dir (root-relative).
pub fn make_feature(
    hip_vel: [f32; 3],
    fl_p: [f32; 3],
    fr_p: [f32; 3],
    fl_v: [f32; 3],
    fr_v: [f32; 3],
    traj_p: [[f32; 2]; 3],
    traj_d: [[f32; 2]; 3],
) -> [f32; FEAT_N] {
    let mut f = [0.0; FEAT_N];
    let parts = [hip_vel, fl_p, fr_p, fl_v, fr_v];
    for (j, part) in parts.iter().enumerate() {
        for (i, v) in part.iter().enumerate() {
            f[j * 3 + i] = san(*v);
        }
    }
    for i in 0..3 {
        f[15 + i * 2] = san(traj_p[i][0]);
        f[15 + i * 2 + 1] = san(traj_p[i][1]);
        f[21 + i * 2] = san(traj_d[i][0]);
        f[21 + i * 2 + 1] = san(traj_d[i][1]);
    }
    f
}

/// Default weights: pose 1.0, trajectory 2.0 (trajectory drives responsiveness).
pub fn default_weights() -> [f32; FEAT_N] {
    let mut w = [1.0; FEAT_N];
    for x in w[15..].iter_mut() {
        *x = 2.0;
    }
    w
}

/// One baked pose frame.
#[derive(Clone, Debug)]
pub struct PoseEntry {
    /// 27-D feature vector.
    pub feat: [f32; FEAT_N],
    /// Source clip id.
    pub clip: u16,
    /// Tick inside clip.
    pub tick: u16,
    /// Contact mask: bit0=L, bit1=R.
    pub contact: u8,
}

/// Baked DB: canonical (clip,tick) order. Bake offline, query online.
#[derive(Clone, Debug, Default)]
pub struct PoseDb {
    /// Poses in bake order.
    pub poses: Vec<PoseEntry>,
}

/// Query: desired feature + contact + continuity bias.
#[derive(Clone, Debug)]
pub struct Query {
    /// Desired 27-D feature.
    pub feat: [f32; FEAT_N],
    /// Desired contact mask.
    pub contact: u8,
    /// Current db index, or NO_HIST.
    pub curr: u32,
    /// Penalty for non-sequential jumps (continuity bias).
    pub jump: f32,
    /// Penalty for contact-mask mismatch (anti-slide gate).
    pub contact_cost: f32,
}

/// Bake offline: stable sort by (clip,tick) for canonical order.
pub fn bake(mut v: Vec<PoseEntry>) -> PoseDb {
    v.sort_by_key(|a| (a.clip, a.tick));
    PoseDb { poses: v }
}

/// Weighted squared distance, NaN-safe. Plain index loop in ascending
/// order so results are bit-identical to the elementwise definition.
/// Uses `dx * dx` (no `powi`) and accumulates in lane order.
pub fn dist2(a: &[f32; FEAT_N], b: &[f32; FEAT_N], w: &[f32; FEAT_N]) -> f32 {
    let mut sum = 0.0;
    for i in 0..FEAT_N {
        let dx = san(a[i]) - san(b[i]);
        sum += pick_weight(w[i]) * dx * dx;
    }
    sum
}

/// Sanitized weight: finite and non-negative, else 0.0.
fn pick_weight(wi: f32) -> f32 {
    if wi.is_finite() && wi >= 0.0 {
        wi
    } else {
        0.0
    }
}

/// 0 if `to` continues `from` in same clip, else `penalty`. History-free = 0.
pub fn transition_cost(db: &PoseDb, from: u32, to: usize, penalty: f32) -> f32 {
    if from == NO_HIST {
        return 0.0;
    }
    let f = from as usize;
    if f >= db.poses.len() || to >= db.poses.len() {
        return 0.0;
    }
    if to == f + 1 && db.poses[to].clip == db.poses[f].clip {
        0.0
    } else if penalty.is_finite() && penalty > 0.0 {
        penalty
    } else {
        0.0
    }
}

/// Brute-force best index. Empty = None. Ties -> lowest index. O(n), no alloc.
/// Weights and the query feature are sanitized once up front; the inner loop
/// is a plain index pass so debug builds stay within the perf gate.
pub fn query(db: &PoseDb, q: &Query, w: &[f32; FEAT_N]) -> Option<usize> {
    if db.poses.is_empty() {
        return None;
    }
    let mut qw = [0.0; FEAT_N];
    let mut qf = [0.0; FEAT_N];
    for i in 0..FEAT_N {
        qw[i] = pick_weight(w[i]);
        qf[i] = san(q.feat[i]);
    }
    let (mut best, mut bd) = (0usize, f32::INFINITY);
    for (i, e) in db.poses.iter().enumerate() {
        // Partial-sum early-out: feature terms are non-negative, so the
        // total `((feat + c) + t)` only grows from `((0 + c) + t)`.
        // Ties keep the lowest index (`<` only), matching the full scan.
        // Winners accumulate all 27 lanes in the original order, so their
        // sums are bit-identical to the full scan.
        let c = if e.contact != q.contact {
            q.contact_cost
        } else {
            0.0
        };
        let t = transition_cost(db, q.curr, i, q.jump);
        if ((0.0 + c) + t).partial_cmp(&bd) != Some(core::cmp::Ordering::Less) {
            continue;
        }
        let mut d = 0.0;
        let mut cut = false;
        for j in 0..FEAT_N {
            let diff = san(e.feat[j]) - qf[j];
            d += qw[j] * diff * diff;
            if ((d + c) + t).partial_cmp(&bd) != Some(core::cmp::Ordering::Less) {
                cut = true;
                break;
            }
        }
        if cut {
            continue;
        }
        d += c;
        d += t;
        if d < bd {
            bd = d;
            best = i;
        }
    }
    Some(best)
}

/// FNV-1a64 over entries (clip,tick,contact,feat bits).
pub fn db_hash(db: &PoseDb) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(db.poses.len() as u64);
    for e in &db.poses {
        h.write_u16(e.clip);
        h.write_u16(e.tick);
        h.write_u8(e.contact);
        for v in e.feat.iter() {
            h.write_f32(*v);
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(clip: u16, tick: u16, x: f32, c: u8) -> PoseEntry {
        let mut f = [0.0; FEAT_N];
        f[0] = x;
        PoseEntry {
            feat: f,
            clip,
            tick,
            contact: c,
        }
    }
    #[test]
    fn bake_sorted_hash_stable() {
        let (a, b) = (
            bake(vec![entry(1, 2, 1.0, 0), entry(0, 9, 2.0, 1)]),
            bake(vec![entry(0, 9, 2.0, 1), entry(1, 2, 1.0, 0)]),
        );
        assert_eq!(a.poses[0].clip, 0);
        assert_eq!(db_hash(&a), db_hash(&b));
    }
    #[test]
    fn query_exact_and_gating() {
        let db = bake(vec![
            entry(0, 0, 0.0, 0),
            entry(0, 1, 10.0, 1),
            entry(0, 2, 20.0, 1),
        ]);
        let w = default_weights();
        let mut f = [0.0; FEAT_N];
        f[0] = 10.0;
        let q = Query {
            feat: f,
            contact: 1,
            curr: NO_HIST,
            jump: 1.0,
            contact_cost: 100.0,
        };
        assert_eq!(query(&db, &q, &w), Some(1));
        assert_eq!(query(&PoseDb::default(), &q, &w), None);
        assert_eq!(transition_cost(&db, 0, 1, 5.0), 0.0);
        assert_eq!(transition_cost(&db, 0, 2, 5.0), 5.0);
        assert_eq!(transition_cost(&db, NO_HIST, 2, 5.0), 0.0);
    }
    #[test]
    fn query_traj_direction() {
        let (mut a, mut b) = ([0.0; FEAT_N], [0.0; FEAT_N]);
        a[15] = 1.0;
        b[15] = -1.0;
        let db = bake(vec![
            PoseEntry {
                feat: a,
                clip: 0,
                tick: 0,
                contact: 0,
            },
            PoseEntry {
                feat: b,
                clip: 0,
                tick: 1,
                contact: 0,
            },
        ]);
        let w = default_weights();
        let mut qf = [0.0; FEAT_N];
        qf[15] = -1.0;
        let q = Query {
            feat: qf,
            contact: 0,
            curr: NO_HIST,
            jump: 0.0,
            contact_cost: 0.0,
        };
        assert_eq!(query(&db, &q, &w), Some(1));
    }
    #[test]
    fn dist2_dxdx_bit_identical() {
        // `dx * dx` in lane order must match the elementwise definition bit-exactly.
        let edges = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            3.14159,
            1e10,
            1e20,
            f32::MIN_POSITIVE,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let wedges = [0.0, 1.0, 2.0, -1.0, f32::NAN, f32::INFINITY, 1e10];
        for (k, (&av, &bv)) in edges.iter().zip(edges.iter().rev()).enumerate() {
            let mut a = [0.0; FEAT_N];
            let mut b = [0.0; FEAT_N];
            let mut w = [1.0; FEAT_N];
            for i in 0..FEAT_N {
                a[i] = av + i as f32;
                b[i] = bv - i as f32;
                w[i] = wedges[(k + i) % wedges.len()];
            }
            let got = dist2(&a, &b, &w);
            let mut want = 0.0;
            for i in 0..FEAT_N {
                let dx = san(a[i]) - san(b[i]);
                want += pick_weight(w[i]) * dx * dx;
            }
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "lane-order dx*dx drift at case {k}"
            );
            assert!(!got.is_nan(), "sanitized dist2 must never be NaN");
        }
    }
    #[test]
    fn query_earlyout_matches_full_scan() {
        fn full_scan(db: &PoseDb, q: &Query, w: &[f32; FEAT_N]) -> Option<usize> {
            if db.poses.is_empty() {
                return None;
            }
            let mut qw = [0.0; FEAT_N];
            let mut qf = [0.0; FEAT_N];
            for i in 0..FEAT_N {
                qw[i] = pick_weight(w[i]);
                qf[i] = san(q.feat[i]);
            }
            let (mut best, mut bd) = (0usize, f32::INFINITY);
            for (i, e) in db.poses.iter().enumerate() {
                let mut d = 0.0;
                for j in 0..FEAT_N {
                    let diff = san(e.feat[j]) - qf[j];
                    d += qw[j] * diff * diff;
                }
                if e.contact != q.contact {
                    d += q.contact_cost;
                }
                d += transition_cost(db, q.curr, i, q.jump);
                if d < bd {
                    bd = d;
                    best = i;
                }
            }
            Some(best)
        }
        // Ties keep the lowest index.
        let dup = [1.0; FEAT_N];
        let db = bake(vec![
            PoseEntry {
                feat: dup,
                clip: 0,
                tick: 0,
                contact: 1,
            },
            PoseEntry {
                feat: dup,
                clip: 0,
                tick: 1,
                contact: 1,
            },
        ]);
        let w = default_weights();
        let q = Query {
            feat: dup,
            contact: 1,
            curr: NO_HIST,
            jump: 0.0,
            contact_cost: 0.0,
        };
        assert_eq!(query(&db, &q, &w), Some(0));
        assert_eq!(full_scan(&db, &q, &w), Some(0));
        // Fuzzed DBs vs the full scan, incl NaN/Inf feats and odd penalties.
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15 | 1;
        let mut nx = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let fee = [0.0, 1.0, -1.0, 100.0, f32::INFINITY, f32::NAN];
        for round in 0..24 {
            let n = 1 + (nx() % 12) as usize;
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let mut f = [0.0; FEAT_N];
                for x in f.iter_mut() {
                    *x = fee[(nx() % fee.len() as u64) as usize] + (nx() % 5) as f32;
                }
                v.push(PoseEntry {
                    feat: f,
                    clip: (i / 4) as u16,
                    tick: i as u16,
                    contact: (nx() % 3) as u8,
                });
            }
            let db = bake(v);
            let mut w = default_weights();
            w[round % FEAT_N] = if round % 3 == 0 { 0.0 } else { 2.0 };
            let mut qf = [0.0; FEAT_N];
            for x in qf.iter_mut() {
                *x = fee[(nx() % fee.len() as u64) as usize];
            }
            let cc = match round % 5 {
                0 => 0.0,
                1 => 10.0,
                2 => -5.0,
                3 => f32::INFINITY,
                _ => f32::NAN,
            };
            let jump = match round % 4 {
                0 => 0.0,
                1 => 2.5,
                2 => -1.0,
                _ => f32::NAN,
            };
            let q = Query {
                feat: qf,
                contact: (nx() % 3) as u8,
                curr: if round % 2 == 0 {
                    NO_HIST
                } else {
                    (nx() % 8) as u32
                },
                jump,
                contact_cost: cc,
            };
            assert_eq!(query(&db, &q, &w), full_scan(&db, &q, &w), "round {round}");
        }
    }
    #[test]
    #[ignore = "timing: run `node tools/proof.mjs --perf` (release, one thread)"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn query_p99_under_2ms() {
        let mut s: u64 = 0x0020_4077 | 1;
        let mut nx = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut v = Vec::with_capacity(512);
        for i in 0..512 {
            let mut f = [0.0; FEAT_N];
            for x in f.iter_mut() {
                *x = (nx() % 1000) as f32 / 1000.0;
            }
            v.push(PoseEntry {
                feat: f,
                clip: (i / 32) as u16,
                tick: (i % 32) as u16,
                contact: (i % 4) as u8,
            });
        }
        let db = bake(v);
        let w = default_weights();
        let q = Query {
            feat: db.poses[7].feat,
            contact: db.poses[7].contact,
            curr: NO_HIST,
            jump: 0.5,
            contact_cost: 10.0,
        };
        // Warmup: cold caches and parallel-harness contention skew the first
        // samples; the gate measures steady-state query cost only.
        for _ in 0..20 {
            assert!(query(&db, &q, &w).is_some());
        }
        let mut ns = Vec::with_capacity(200);
        for _ in 0..200 {
            let t = std::time::Instant::now();
            assert!(query(&db, &q, &w).is_some());
            ns.push(t.elapsed().as_nanos());
        }
        ns.sort_unstable();
        let p99 = ns[99 * ns.len() / 100];
        assert!(p99 < 2_000_000, "p99 {p99}ns >= 2ms");
    }
}
