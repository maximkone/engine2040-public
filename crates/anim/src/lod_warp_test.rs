//! Warp-window accept: a traversal warp keeps its landing under the LOD
//! tiers. Pinned window ticks evaluate every tick, so every tier lands the
//! warped root inside the 1 cm edge bar; the same tape without the pin reads
//! a strobe-held root and misses it, so the gate can say no. Replay is by
//! table state hash: one seed twice is byte-identical, a second seed differs.

// The module is declared under `cfg(test)` in lib.rs; this marker keeps the
// determinism and genre scans from reading fixture math and names as
// production code.
#[cfg(test)]
use super::contract::LocalPose;
use super::lod::{assign_tiers, LodSlot, Tier, WarpWindow, FULL_RATE_NEAREST, MAX_FIGHTERS};
use super::traversal::{
    apply_warp, duration_ticks, plan, warp_error_mm, Probe, TraversalPlan, TraverseAction,
    WARP_TOL_MM,
};
use engine_core::{Fnv64, Rng};

/// Root advance per vault tick: 8 cm, so one held eval already misses the
/// 1 cm landing bar eight times over.
const ROOT_STRIDE_M: f32 = 0.08;
/// The warped fighter sits one past the full-rate set, so only the tier table
/// can give it a strobed tier.
const WARPED_SLOT: usize = FULL_RATE_NEAREST;
/// Plan commit tick of the base tape.
const WARP_START: u64 = 10;
/// Start-tick phases swept per tier: every point of every eval period.
const PHASES: u64 = 8;
/// Tier lanes: probe distances that read Full, Half, Quarter and Far at a
/// small screen fraction.
const TIER_LANES: [(&str, f32); 4] = [
    ("full", 5.0),
    ("half", 20.0),
    ("quarter", 45.0),
    ("far", 70.0),
];

/// Shipped TV1 fixture: a waist-high block at speed is a vault.
fn vault_probe() -> Probe {
    Probe {
        height_m: 1.0,
        depth_m: 0.6,
        gap_m: 1.2,
        speed_m_s: 2.0,
        has_hold: false,
    }
}

/// One committed vault plus the per-tick root path it warps onto its edge,
/// all drawn from `seed`.
struct WarpTape {
    plan: TraversalPlan,
    edge_m: [f32; 2],
    path: Vec<[f32; 2]>,
    joints: Vec<[f32; 3]>,
    start_tick: u64,
    landing_tick: u64,
}

/// Build the tape: the raw root walks `ROOT_STRIDE_M` a tick with a seeded
/// lateral lane, then the warp translates it onto the edge.
fn tape(seed: u64, start_tick: u64) -> WarpTape {
    let probe = vault_probe();
    let ticks = duration_ticks(TraverseAction::Vault);
    assert!(ticks > 1, "a vault owns more than one tick");
    let span = f32::from(u16::try_from(ticks - 1).unwrap_or(u16::MAX));
    // The edge sits just past the raw root end, so the warp is a small
    // integer translation, not a teleport.
    let edge_m = [ROOT_STRIDE_M * span + 0.05, 0.4];
    let plan = plan(&probe, edge_m).expect("a 1.0 m block at 2 m/s commits a vault");
    assert_eq!(plan.action, TraverseAction::Vault, "fixture is a vault");
    // The legacy stream aliases seeds 2k and 2k + 1 (`| 1`), so expand the
    // fixture seed: adjacent test seeds must build different tapes.
    let mut rng = Rng::new(seed.wrapping_mul(2));
    let mut raw = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
    let mut joints = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
    for k in 0..ticks {
        let kf = f32::from(u16::try_from(k).unwrap_or(u16::MAX));
        let lane = f32::from(u16::try_from(rng.next_u32() % 401).unwrap_or(0)) / 1000.0 - 0.2;
        raw.push([ROOT_STRIDE_M * kf, lane]);
        let bend = f32::from(u16::try_from(rng.next_u32() % 101).unwrap_or(0)) / 1000.0;
        joints.push([0.0, bend, 0.0]);
    }
    let path = apply_warp(&plan, &raw).expect("the vault path warps onto the edge");
    WarpTape {
        plan,
        edge_m,
        path,
        joints,
        start_tick,
        landing_tick: start_tick + u64::from(ticks - 1),
    }
}

/// Quaternion for a yaw of `deg` degrees: half-angle `[0, sin, 0, cos]`.
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture: builds quats; the window pin itself is arithmetic only"
)]
fn yaw_quat(deg: f32) -> [f32; 4] {
    let half = 0.5 * deg.to_radians();
    [0.0, half.sin(), 0.0, half.cos()]
}

/// Four-joint pose over `root` with a hip bend offset and a yaw.
fn pose(root: [f32; 3], bend: f32, yaw_deg: f32) -> LocalPose {
    LocalPose {
        positions: vec![
            root,
            [root[0] - 0.06, root[1] + 0.9 + bend, root[2]],
            [root[0] + 0.14, root[1] + 0.55 + bend, root[2] + 0.02],
            [root[0] + 0.18, root[1] + 0.02, root[2] - 0.04],
        ],
        rotations: vec![
            yaw_quat(yaw_deg),
            yaw_quat(0.5 * yaw_deg),
            yaw_quat(-yaw_deg),
            yaw_quat(2.0 * yaw_deg),
        ],
    }
}

/// The warped root at path sample `k`: rises over the block and settles.
fn warped_pose(tape: &WarpTape, k: usize) -> LocalPose {
    let last = tape.path.len().saturating_sub(1).max(1);
    let kf = f32::from(u16::try_from(k).unwrap_or(u16::MAX));
    let tf = f32::from(u16::try_from(last).unwrap_or(u16::MAX));
    let t = kf / tf;
    let out = 2.0 * t - 1.0;
    let arc = 0.35 * (1.0 - out * out);
    let root = [tape.path[k][0], 0.05 + arc, tape.path[k][1]];
    pose(root, tape.joints[k][1], 90.0)
}

/// Grounded approach pose before the vault commits.
fn walk_pose(tick: u64, start_tick: u64) -> LocalPose {
    let back = start_tick.saturating_sub(tick).min(u64::from(u16::MAX));
    let back_f = f32::from(u16::try_from(back).unwrap_or(u16::MAX));
    pose([-ROOT_STRIDE_M * back_f, 0.05, 0.0], 0.0, 90.0)
}

/// The warped fighter's full-rate evaluation on `tick`: walk before the
/// commit, warped path inside the window, landing pose after it.
fn probe_target(tape: &WarpTape, tick: u64) -> LocalPose {
    if tick < tape.start_tick {
        return walk_pose(tick, tape.start_tick);
    }
    let k = usize::try_from(tick - tape.start_tick).unwrap_or(usize::MAX);
    warped_pose(tape, k.min(tape.path.len() - 1))
}

/// Filler fighter pose: every tick moves, so a held eval leaves a real gap.
fn crowd_pose(slot: usize, tick: u64) -> LocalPose {
    let t = f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
    let s = f32::from(u16::try_from(slot).unwrap_or(u16::MAX));
    pose([0.02 * t + 0.01 * s, 0.05, 0.004 * s], 0.05 * t, 20.0 + s)
}

/// Empty pose: the caller's target table starts blank.
fn empty_pose() -> LocalPose {
    LocalPose {
        positions: Vec::new(),
        rotations: Vec::new(),
    }
}

/// Planar root distance in millimeters, X/Z only: a root lands on the ground
/// plane, so height is not part of the edge contract.
fn planar_mm(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    (dx * dx + dz * dz).sqrt() * 1000.0
}

/// Fold one slot's shown pose and tier into the replay hash.
fn fold_state(hash: &mut Fnv64, slot: &LodSlot) {
    for lane in &slot.shown().positions {
        for value in lane {
            hash.write_f32(*value);
        }
    }
    for lane in &slot.shown().rotations {
        for value in lane {
            hash.write_f32(*value);
        }
    }
    hash.write(&slot.tier().period_ticks().to_le_bytes());
}

/// One driven tape's landing read.
struct Landing {
    /// Pinned slot's shown root against the full-rate landing, mm.
    err_mm: f32,
    /// Root of the last evaluation against the full-rate landing, mm: the
    /// strobe alone, before any rebuild.
    strobe_mm: f32,
    /// Hash of every slot's shown state over the whole tape.
    state_hash: u64,
    /// True when the landing tick itself evaluated.
    landing_evals: bool,
}

/// Drive one 64-fighter tape with the warped fighter at `distance_m`, with or
/// without the warp-window pin, and read its landing.
fn drive(tape: &WarpTape, distance_m: f32, pinned: bool) -> Landing {
    let mut dists = vec![0.0f32; MAX_FIGHTERS];
    for (i, d) in dists.iter_mut().enumerate() {
        *d = if i < FULL_RATE_NEAREST {
            5.0
        } else if i == WARPED_SLOT {
            distance_m
        } else {
            90.0 + f32::from(u16::try_from(i).unwrap_or(u16::MAX))
        };
    }
    let screens = vec![0.002f32; MAX_FIGHTERS];
    let mut tiers = vec![Tier::Far; MAX_FIGHTERS];
    let mut slots: Vec<LodSlot> = (0..MAX_FIGHTERS).map(|_| LodSlot::new()).collect();
    let mut targets: Vec<LocalPose> = (0..MAX_FIGHTERS).map(|_| empty_pose()).collect();
    let window = WarpWindow::from_plan(&tape.plan, tape.start_tick);
    let mut hash = Fnv64::new();
    let mut eval_root = [0.0f32; 3];
    let mut landing_evals = false;
    for tick in 0..=tape.landing_tick {
        assign_tiers(&dists, &screens, &mut tiers).expect("warp tape assigns tiers");
        for i in 0..MAX_FIGHTERS {
            let warp_tick = pinned && i == WARPED_SLOT && window.contains(tick);
            let tier = if warp_tick {
                window.tier_for(tick, tiers[i])
            } else {
                tiers[i]
            };
            slots[i].begin_pinned(tier, warp_tick);
            // Consumed: this tape has no spring bones to re-seed.
            slots[i].take_reseed();
            if slots[i].tier().updates_this_tick(tick, i) {
                targets[i] = if i == WARPED_SLOT {
                    probe_target(tape, tick)
                } else {
                    crowd_pose(i, tick)
                };
                if i == WARPED_SLOT {
                    eval_root = targets[i].positions[0];
                }
            }
            slots[i].rebuild(&targets[i]);
            fold_state(&mut hash, &slots[i]);
        }
        if tick == tape.landing_tick {
            landing_evals = slots[WARPED_SLOT]
                .tier()
                .updates_this_tick(tick, WARPED_SLOT);
        }
    }
    let reference = warped_pose(tape, tape.path.len() - 1);
    let shown = slots[WARPED_SLOT].shown().positions[0];
    Landing {
        err_mm: planar_mm(shown, reference.positions[0]),
        strobe_mm: planar_mm(eval_root, reference.positions[0]),
        state_hash: hash.finish(),
        landing_evals,
    }
}

/// The row's accept: with the warp-window pin every landing tick evaluates and
/// the shown root lands within 1 cm of the full-rate landing, at all four AM13
/// tiers and every start phase. The same tape without the pin reads a held
/// root and must miss the bar, so the gate can say no.
#[test]
fn lod_warp_lands_within_1cm_under_every_tier() {
    let mut worst_pinned = 0.0f32;
    let mut worst_unpinned = 0.0f32;
    let mut stale_landings = 0u32;
    for (lane, distance_m) in TIER_LANES {
        let mut lane_worst = 0.0f32;
        for phase in 0..PHASES {
            let warp = tape(2040, WARP_START + phase);
            let window = WarpWindow::from_plan(&warp.plan, warp.start_tick);
            assert_eq!(
                window.landing_tick(),
                Some(warp.landing_tick),
                "{lane} phase {phase}: the window lands on the tape's last warp tick"
            );
            let reference = warped_pose(&warp, warp.path.len() - 1);
            let end = [reference.positions[0][0], reference.positions[0][2]];
            assert!(
                warp_error_mm(end, warp.edge_m) <= WARP_TOL_MM,
                "{lane} phase {phase}: the full-rate landing itself holds the edge bar"
            );
            let pinned = drive(&warp, distance_m, true);
            assert!(
                pinned.err_mm <= WARP_TOL_MM,
                "{lane} phase {phase}: pinned landing missed by {} mm",
                pinned.err_mm
            );
            assert!(
                pinned.landing_evals,
                "{lane} phase {phase}: a pinned landing tick always evaluates"
            );
            worst_pinned = worst_pinned.max(pinned.err_mm);
            let unpinned = drive(&warp, distance_m, false);
            lane_worst = lane_worst.max(unpinned.err_mm);
            worst_unpinned = worst_unpinned.max(unpinned.err_mm);
            if !unpinned.landing_evals {
                stale_landings += 1;
                assert!(
                    unpinned.strobe_mm > WARP_TOL_MM,
                    "{lane} phase {phase}: a held landing root read {} mm from the landing sample",
                    unpinned.strobe_mm
                );
            }
        }
        if lane != "full" {
            assert!(
                lane_worst > WARP_TOL_MM,
                "{lane}: without the warp pin the landing must miss the 1 cm bar, worst {} mm",
                lane_worst
            );
        }
    }
    assert!(
        worst_pinned <= WARP_TOL_MM,
        "the pin holds every landing within 1 cm, worst {worst_pinned} mm"
    );
    assert!(
        stale_landings > 0,
        "the phase sweep must land on held ticks, got none"
    );
    assert!(
        worst_unpinned > WARP_TOL_MM,
        "the same tape without the pin fails the 1 cm bar, worst {worst_unpinned} mm"
    );
}

/// Replay x2: the same seed builds the same tape and drives the same state
/// hash twice; a second seed moves the state. Both land inside the bar.
#[test]
fn lod_warp_replays_exactly_and_moves_with_the_seed() {
    let warp = tape(2040, WARP_START);
    let first = drive(&warp, 45.0, true);
    let again = drive(&warp, 45.0, true);
    assert_eq!(
        first.state_hash, again.state_hash,
        "one tape driven twice is byte-identical"
    );
    let rebuilt = tape(2040, WARP_START);
    let second = drive(&rebuilt, 45.0, true);
    assert_eq!(
        second.state_hash, first.state_hash,
        "the same seed rebuilds and replays the same state"
    );
    let other = tape(2041, WARP_START);
    let moved = drive(&other, 45.0, true);
    assert_ne!(
        moved.state_hash, first.state_hash,
        "a second seed must move the state"
    );
    for landing in [&first, &moved] {
        assert!(
            landing.err_mm <= WARP_TOL_MM,
            "a sealed landing missed by {} mm",
            landing.err_mm
        );
    }
}
