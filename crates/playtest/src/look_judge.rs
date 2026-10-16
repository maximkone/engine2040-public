//! Pure playtest look judge, first slice: L1 fighters-present plus JSON.
//! Consumer: the `look.judge` door verb via a host (next slice). Units:
//! permille `0..=1000` for L1-L9, 16 lowercase hex digests. Integer only, so
//! the same frame twice gives byte-identical JSON. Ragged captures refuse
//! with [`FrameFault`]; one `check_frame` pass plus one format per call.

use crate::frame_check::{check_frame, CapturedFrame, FrameFault, FrameMetrics, Thresholds};

/// Rubric ids in endpoint order; mirrors `view::LOOK_RUBRIC_IDS` L1-L9.
const RUBRIC_IDS: [&str; 9] = ["L1", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9"];

/// Nine integer permille scores L1-L9 plus the judged frame digest.
/// Scores index by rubric order (`scores[0]` is L1); digest is 16 hex.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JudgeScore {
    /// L1-L9 in fixed order, each `0..=1000`.
    pub scores: [u64; 9],
    /// [`crate::frame_check::digest_hex`] of the judged capture.
    pub digest: String,
}

/// L1 fighters-present: 1000 when draws clear 0 and cover clears the floor, else 0.
pub fn judge_l1(metrics: &FrameMetrics) -> u64 {
    let floor = u64::from(Thresholds::default().min_cover_permille);
    if metrics.draws > 0 && metrics.cover_permille >= floor {
        1000
    } else {
        0
    }
}

/// L2 scene structure: the covered fraction clamped to permille.
fn score_l2(cover_permille: u64) -> u64 {
    cover_permille.min(1000)
}

/// L3 ground texture: cover at 0.8 weight with integer division.
fn score_l3(cover_permille: u64) -> u64 {
    cover_permille
        .saturating_mul(800)
        .saturating_div(1000)
        .min(1000)
}

/// L4 real capture: 1000 when any draw was submitted, else 0.
fn score_l4(draws: u32) -> u64 {
    u64::from(draws > 0) * 1000
}

/// L5 frame clean: 1000 when no near-plane slice was reported, else 0.
fn score_l5(near_slices: u32) -> u64 {
    u64::from(near_slices == 0) * 1000
}

/// L6 tonal range: half the cover, clamped to permille.
fn score_l6(cover_permille: u64) -> u64 {
    cover_permille.saturating_div(2).min(1000)
}

/// L7 fighter palette weight: 250 per draw, clamped to permille.
fn score_l7(draws: u32) -> u64 {
    u64::from(draws).saturating_mul(250).min(1000)
}

/// L8 reference likeness: cover at 0.6 weight plus 50 per draw, clamped.
fn score_l8(cover_permille: u64, draws: u32) -> u64 {
    cover_permille
        .saturating_mul(600)
        .saturating_div(1000)
        .saturating_add(u64::from(draws).saturating_mul(50))
        .min(1000)
}

/// L9 hero shot: 1000 when fighters present (L1 rule) and clean, else 0.
fn score_l9(cover_permille: u64, draws: u32, near_slices: u32) -> u64 {
    let floor = u64::from(Thresholds::default().min_cover_permille);
    if draws > 0 && cover_permille >= floor && near_slices == 0 {
        1000
    } else {
        0
    }
}

/// Build all nine ids in fixed L1-L9 order from one metrics plus digest; every id set.
pub fn judge_frame(metrics: &FrameMetrics) -> JudgeScore {
    let draws = metrics.draws;
    let cover = metrics.cover_permille;
    let near = metrics.near_slices;
    JudgeScore {
        scores: [
            judge_l1(metrics),
            score_l2(cover),
            score_l3(cover),
            score_l4(draws),
            score_l5(near),
            score_l6(cover),
            score_l7(draws),
            score_l8(cover, draws),
            score_l9(cover, draws, near),
        ],
        digest: metrics.digest.clone(),
    }
}

/// Judge one capture to endpoint-typed JSON in fixed key order with integers.
/// Pass means L1 fighters present; score is the mean of nine. Ragged refuses
/// with the [`FrameFault`] from [`check_frame`].
///
/// # Errors
///
/// [`FrameFault::BadCapture`] and the other [`check_frame`] faults.
pub fn to_json(frame: &CapturedFrame, thresholds: &Thresholds) -> Result<String, FrameFault> {
    let metrics = check_frame(frame, thresholds)?;
    let judged = judge_frame(&metrics);
    let mut sum = 0u64;
    for score in judged.scores {
        sum = sum.saturating_add(score);
    }
    let mean = sum / 9;
    let pass = judged.scores[0] > 0;
    let mut rubric = String::new();
    for (index, id) in RUBRIC_IDS.iter().enumerate() {
        let score = judged.scores[index];
        let item_pass = score > 0;
        if index > 0 {
            rubric.push(',');
        }
        rubric.push_str(&format!(
            "{{\"id\":\"{id}\",\"score\":{score},\"pass\":{item_pass}}}"
        ));
    }
    Ok(format!(
        "{{\"schema\":1,\"digest\":\"{}\",\"pass\":{pass},\"score\":{mean},\"rubric\":[{rubric}],\"frames\":[{{\"name\":\"frame_000000.png\",\"digest\":\"{}\",\"draws\":{},\"cover_permille\":{}}}]}}",
        judged.digest,
        judged.digest,
        metrics.draws,
        metrics.cover_permille
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_check::{digest_hex, scripted, Script};

    /// One helper naming both F2P fixtures: a cube-only capture (one dim draw,
    /// no cover, so L1 stays 0) and a drawn-fighter capture (scripted clean).
    fn cube_and_fighter_frames() -> (CapturedFrame, CapturedFrame) {
        // Dim gray fill: alpha never counts, so cover stays 0 with one draw.
        let dim = vec![16u8; 16 * 16 * 4];
        let cube = CapturedFrame {
            width: 16,
            height: 16,
            rgba: dim,
            draws: 1,
            expected_draws: 0,
            near_slices: 0,
        };
        let fighter = scripted(Script::Clean, 96, 64, 7);
        let _ = digest_hex(b"look-judge-f2p"); // Reuse the digest owner here.
        (cube, fighter)
    }

    #[test]
    fn look_judge_cube_scores_zero_l1_and_fighters_score_above() {
        let thresholds = Thresholds::default();
        let (cube_frame, fighter_frame) = cube_and_fighter_frames();
        let cube = check_frame(&cube_frame, &thresholds).expect("cube passes frame check");
        let fighter = check_frame(&fighter_frame, &thresholds).expect("fighter passes frame check");
        assert_eq!(judge_l1(&cube), 0, "cube-only scores 0 on L1");
        assert!(judge_l1(&fighter) > 0, "drawn fighters score above 0");
        let first = to_json(&fighter_frame, &thresholds).expect("judges");
        let second = to_json(&fighter_frame, &thresholds).expect("judges again");
        assert_eq!(first, second, "same frames give byte-identical bytes twice");
        assert!(
            first.contains("\"schema\":1"),
            "typed JSON carries schema 1"
        );
    }

    #[test]
    fn look_judge_ragged_refuses_without_panic() {
        let bad = CapturedFrame {
            width: 16,
            height: 16,
            rgba: vec![0u8; 16 * 16 * 4 - 1],
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            to_json(&bad, &Thresholds::default()),
            Err(FrameFault::BadCapture(_))
        ));
    }
}
