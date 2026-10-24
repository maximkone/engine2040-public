//! finisher_dof: the focus depth the depth-of-field pass reads (SW173).
//!
//! While [`FinisherBeat`] holds the strike, the frame focuses on the finisher's
//! impact point: the render side's half-res CoC pass (`render::post::post_dof`)
//! takes that point's view depth as its focus distance and blurs everything
//! that sits far from it. Outside the beat the answer is `None`, so the post
//! chain keeps its sharp frame and the arena pays nothing.
//!
//! Owner: game. Units: metres along the camera axis. Presentation only, never
//! read by the sim: no tick, no hash, no allocation.

use crate::finisher_beat::FinisherBeat;

/// View depth of the finisher's impact point from the beat camera: metres
/// along the camera's forward axis, so a point behind the lens never focuses.
/// `None` when the axis is degenerate or the point sits behind the camera.
#[must_use]
pub fn impact_focus_m(eye: [f32; 3], forward: [f32; 3], impact: [f32; 3]) -> Option<f32> {
    let len = (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2]).sqrt();
    if !len.is_finite() || len < 1e-6 {
        return None;
    }
    let depth = ((impact[0] - eye[0]) * forward[0]
        + (impact[1] - eye[1]) * forward[1]
        + (impact[2] - eye[2]) * forward[2])
        / len;
    if !depth.is_finite() || depth <= 0.0 {
        return None;
    }
    Some(depth)
}

/// The focus depth this frame's depth of field uses, or `None` while no beat
/// holds: the impact point's view depth only exists during the finisher.
#[must_use]
pub fn dof_focus_m(
    beat: &FinisherBeat,
    eye: [f32; 3],
    forward: [f32; 3],
    impact: [f32; 3],
) -> Option<f32> {
    if beat.holds() {
        impact_focus_m(eye, forward, impact)
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

    /// The focus exists only while the beat holds: before the strike and after
    /// the beat the frame is sharp.
    #[test]
    fn dof_focus_exists_only_while_the_beat_holds() {
        let mut beat = FinisherBeat::new();
        let impact = [0.0, 1.6, 6.0];
        assert_eq!(dof_focus_m(&beat, EYE, FORWARD, impact), None);
        beat.strike();
        assert_eq!(dof_focus_m(&beat, EYE, FORWARD, impact), Some(10.0));
        for _ in 0..=BEAT_END_TICKS {
            beat.step();
        }
        assert_eq!(dof_focus_m(&beat, EYE, FORWARD, impact), None);
    }
}
