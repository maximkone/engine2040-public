//! finisher_dof: the focus depth the depth-of-field pass reads (SW173).
//!
//! While [`FinisherBeat`](crate::finisher_beat::FinisherBeat) is live, the
//! frame focuses on the finisher's impact point: the render side's half-res
//! CoC pass (`render::post::post_dof`) takes that point's view depth as its
//! focus distance and blurs everything far from it. Outside the beat the
//! answer is `None`, so the post chain keeps its sharp frame and the arena
//! pays nothing.
//!
//! Owner: game. Units: metres along the camera axis. Presentation only, never
//! read by the sim: no tick, no hash, no allocation.

use crate::finisher_beat::FinisherBeat;

/// Distance in metres from `eye` to `point` along the camera axis: the focus
/// depth of a depth-of-field lens looking down `forward` (SW173). Sideways
/// offset never changes it; a point behind the lens or a degenerate axis gives
/// `None`, so a caller never focuses on something it cannot see. Pure; a finite
/// input gives a finite answer.
#[must_use]
pub fn impact_focus_m(eye: [f32; 3], forward: [f32; 3], point: [f32; 3]) -> Option<f32> {
    let [fx, fy, fz] = forward;
    let len = (fx * fx + fy * fy + fz * fz).sqrt();
    if !len.is_finite() || len <= 1e-6 {
        return None;
    }
    let along =
        ((point[0] - eye[0]) * fx + (point[1] - eye[1]) * fy + (point[2] - eye[2]) * fz) / len;
    if along > 0.0 && along.is_finite() {
        Some(along)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finisher_beat::BEAT_END_TICKS;

    const EYE: [f32; 3] = [0.0, 1.6, -4.0];
    const FORWARD: [f32; 3] = [0.0, 0.0, 1.0];

    /// SW173 accept: the impact point sets the focus distance in metres, and a
    /// point behind the lens focuses on nothing.
    #[test]
    fn impact_point_sets_the_focus_depth_in_metres() {
        assert_eq!(impact_focus_m(EYE, FORWARD, [0.0, 1.6, 6.0]), Some(10.0));
        assert_eq!(
            impact_focus_m(EYE, FORWARD, [3.0, 1.6, 6.0]),
            Some(10.0),
            "a sideways offset does not change the view depth"
        );
        assert_eq!(impact_focus_m(EYE, FORWARD, [0.0, 1.6, -9.0]), None);
        assert_eq!(impact_focus_m(EYE, [0.0, 0.0, 0.0], [0.0, 0.0, 6.0]), None);
    }

    /// The focus exists only while the beat is live: before the strike and once
    /// the frame is clear, the lens is sharp.
    #[test]
    fn dof_focus_exists_only_while_the_beat_is_live() {
        let mut beat = FinisherBeat::new();
        let impact = [0.0, 1.6, 6.0];
        assert_eq!(beat.dof_focus_m(EYE, FORWARD, impact), None);
        beat.strike();
        assert_eq!(beat.dof_focus_m(EYE, FORWARD, impact), Some(10.0));
        for _ in 0..=BEAT_END_TICKS {
            beat.step();
        }
        assert_eq!(
            beat.dof_focus_m(EYE, FORWARD, impact),
            None,
            "the frame is sharp again after the beat"
        );
    }
}
