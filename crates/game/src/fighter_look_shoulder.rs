//! fighter_look_shoulder: the live duel's over-the-shoulder shot.
//!
//! Owner: game lane, a child of `fighter_look`. Consumer:
//! [`FighterLook::fight_camera`] for a two-fighter bout in the fight shot (the
//! window's default `--fighters 2` run). Before this shot the duel took a
//! side-on boom that backed off to hold both bodies, so two fighters standing
//! 15 m apart were two 60 px figures at the screen edges. The GP14 framing
//! (`engine_combat::camera_framing::frame_melee`) sits the eye behind the hero's
//! shoulder instead: the hero is big in the foreground, the rival ahead of the
//! look point, and a melee fills the picture.
//! Units: meters, radians. Determinism: presentation only, pure over the two
//! floor positions plus the pit limit and the shake sample; same inputs give
//! the same camera twice, and the tick hash never sees it.
//! Errors: none, `None` hands the shot back to the side-on fit. Budget: a few
//! dozen float ops, no heap.

use engine_combat::camera_framing::{frame_melee, Shoulder};
use engine_render::v1::Camera;

use super::{count_in_picture, FighterLook};
use crate::camera::to_mm;

/// Vertical lens of the duel shot, degrees: the fight lens of `shot_camera`.
const DUEL_FOV_Y_DEG: f32 = 60.0;
/// Chest height of a standing fighter that must land in the picture, meters.
const CHEST_M: f32 = 1.0;
/// Nearest the eye may sit to either body on the floor, meters: any nearer is
/// the camera inside a fighter.
const EYE_CLEAR_M: f32 = 1.1;
/// Shortest share of the framing's boom the pit wall may leave. The live
/// spawn puts the hero 1.2 m off the pit wall with the 4.5 m boom pointing
/// into it, so a quarter boom (1.1 m, the hero big at the screen's lower left)
/// still takes the shot; a shorter one would sit on the hero, and the other
/// shoulder or the side-on fit takes over.
const MIN_BOOM_SHARE: f32 = 0.25;
/// How far the eye sinks, meters, when the wall takes the whole boom: a short
/// boom from the full 1.9 m eye looks down past the hero's chest out of the
/// 60 degree lens, so the eye drops with the boom and keeps the torso in.
const EYE_DROP_M: f32 = 0.55;

/// Millimeters from the integer framing back to meters. The arena is under 33 m
/// across (an `i16` of millimeters); a farther value saturates, never wraps.
fn meters(mm: i32) -> f32 {
    let mm = i16::try_from(mm).unwrap_or(if mm < 0 { i16::MIN } else { i16::MAX });
    f32::from(mm) / 1000.0
}

/// Largest share (`0..=1`) of the way back along `along` (the boom behind the
/// hero, floor x/z) that keeps the eye `hero + lat + share * along` inside the
/// square pit of half-width `limit`; `lat` is the shoulder's sideways offset,
/// which the wall never shortens (it is what keeps the hero off the screen
/// center). Pulling the eye in along the boom keeps the shoulder angle, where
/// clamping each axis apart would swing the eye across the hero. A hero whose
/// shoulder spot is already outside the pit gives 0.
fn pit_back_share(hero: [f32; 2], along: [f32; 2], lat: [f32; 2], limit: f32) -> f32 {
    let mut share = 1.0f32;
    for axis in 0..2 {
        let base = hero[axis] + lat[axis];
        if base.abs() > limit {
            return 0.0;
        }
        if along[axis].abs() > 1e-6 {
            let edge = if along[axis] > 0.0 { limit } else { -limit };
            share = share.min((edge - base) / along[axis]);
        }
    }
    share.max(0.0)
}

/// The shoulder shot of `hero` against `rival` (floor positions, meters): the
/// eye over the hero's right shoulder first (the hero on the left of the
/// picture, the rival ahead of him), the other shoulder when the pit wall
/// blocks it, `None` when neither holds both bodies. The boom behind the hero is shortened to keep
/// the eye inside the square pit of half-width `limit` (never below
/// [`MIN_BOOM_SHARE`]; the shoulder offset stays whole, so a short boom keeps
/// the hero at the screen's left third instead of its center), the eye stays
/// [`EYE_CLEAR_M`] off both bodies, and `shake` moves eye and target together
/// (LF11). Pure presentation.
#[must_use]
pub(super) fn duel_shoulder(
    base: &Camera,
    hero: [f32; 3],
    rival: [f32; 3],
    limit: f32,
    shake: [f32; 3],
) -> Option<Camera> {
    let fov = DUEL_FOV_Y_DEG.to_radians();
    let half_tan_permille = to_mm((fov * 0.5).tan()).unsigned_abs();
    let ground = |at: [f32; 3]| [to_mm(at[0]), to_mm(at[2])];
    let chests = [[hero[0], CHEST_M, hero[2]], [rival[0], CHEST_M, rival[2]]];
    // `frame_melee` measures its shoulders on the ground plane in the combat
    // crate's handedness; seen through this camera (y up, -z forward) its
    // `Left` puts the eye on the hero's right, the hero on the picture's left.
    for shoulder in [Shoulder::Left, Shoulder::Right] {
        let framing = frame_melee(ground(hero), ground(rival), shoulder, half_tan_permille);
        // The framing's own eye splits into the boom along the hero-to-rival
        // line (the part the wall may shorten) and the sideways shoulder offset.
        let (dx, dz) = (rival[0] - hero[0], rival[2] - hero[2]);
        let span = dx.hypot(dz);
        let u = if span > 0.2 {
            [dx / span, dz / span]
        } else {
            [0.0, 1.0]
        };
        let eye_off = [
            meters(framing.eye_mm[0]) - hero[0],
            meters(framing.eye_mm[2]) - hero[2],
        ];
        let reach = eye_off[0] * u[0] + eye_off[1] * u[1];
        let along = [u[0] * reach, u[1] * reach];
        let lat = [eye_off[0] - along[0], eye_off[1] - along[1]];
        let share = pit_back_share([hero[0], hero[2]], along, lat, limit);
        if share < MIN_BOOM_SHARE {
            continue;
        }
        let eye_xz = [
            hero[0] + lat[0] + along[0] * share,
            hero[2] + lat[1] + along[1] * share,
        ];
        let clear = |at: [f32; 3]| (at[0] - eye_xz[0]).hypot(at[2] - eye_xz[1]) >= EYE_CLEAR_M;
        if !clear(hero) || !clear(rival) {
            continue;
        }
        let mut camera = base.clone();
        camera.eye = [
            eye_xz[0] + shake[0],
            meters(framing.eye_mm[1]) - EYE_DROP_M * (1.0 - share) + shake[1],
            eye_xz[1] + shake[2],
        ]
        .into();
        camera.target = [
            meters(framing.look_mm[0]) + shake[0],
            meters(framing.look_mm[1]) + shake[1],
            meters(framing.look_mm[2]) + shake[2],
        ]
        .into();
        camera.up = [0.0, 1.0, 0.0].into();
        camera.fov_y = fov;
        camera.near = 0.1;
        camera.far = camera.far.max(120.0);
        if camera.validate().is_ok() && count_in_picture(&camera, chests.into_iter()) == 2 {
            return Some(camera);
        }
    }
    None
}

impl FighterLook {
    /// The duel's shoulder shot when exactly two bodies are seen: the lower
    /// slot (the hero while it is seen) is the attacker, the eye stays inside
    /// the pit and moves with the LF11 shake. `None` for any other count or
    /// when the shot cannot hold both bodies, so the side-on fit takes over.
    #[must_use]
    pub(super) fn shoulder_duel(&self, base: &Camera, seen: &[[f32; 3]]) -> Option<Camera> {
        let [hero, rival] = seen else {
            return None;
        };
        let limit = self
            .ring
            .as_ref()
            .map_or(f32::MAX, |ring| ring.wall_m - 0.6);
        duel_shoulder(base, *hero, *rival, limit, self.fight_shake_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Camera {
        Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        )
    }

    /// Share of the frame height a standing fighter (feet to 1.8 m) fills, 0
    /// when the camera cannot build.
    fn height_share(camera: &Camera, at: [f32; 2]) -> f32 {
        let Ok(view) = camera.view_matrix() else {
            return 0.0;
        };
        let half = (camera.fov_y * 0.5).tan();
        let ndc_y = |y: f32| {
            let mut point = camera.eye;
            (point.x, point.y, point.z) = (at[0], y, at[1]);
            let local = view.transform_point3(point);
            local.y / (-local.z * half)
        };
        (ndc_y(1.8) - ndc_y(0.0)).abs() * 0.5
    }

    /// Horizontal screen position of a fighter's chest, `-1` the left edge and
    /// `1` the right, 0 when the camera cannot build.
    fn ndc_x(camera: &Camera, at: [f32; 2]) -> f32 {
        let Ok(view) = camera.view_matrix() else {
            return 0.0;
        };
        let half = (camera.fov_y * 0.5).tan() * camera.aspect;
        let mut point = camera.eye;
        (point.x, point.y, point.z) = (at[0], CHEST_M, at[1]);
        let local = view.transform_point3(point);
        local.x / (-local.z * half)
    }

    #[test]
    fn a_melee_fills_the_picture_with_both_fighters_large() {
        let cam = duel_shoulder(&base(), [0.0; 3], [2.5, 0.0, 0.0], 20.4, [0.0; 3])
            .expect("a close duel in the open holds");
        let (hero, rival) = (
            height_share(&cam, [0.0, 0.0]),
            height_share(&cam, [2.5, 0.0]),
        );
        assert!(hero > 0.4, "the hero fills {hero} of the frame height");
        assert!(rival > 0.22, "the rival fills {rival} of the frame height");
        assert!(cam.eye.y > 1.8 && cam.eye.y < 2.0, "shoulder-high eye");
    }

    #[test]
    fn two_fighters_across_the_pit_still_hold_the_hero_big() {
        // The old side-on fit backed off to ~48 m here: two specks at the edges.
        let cam = duel_shoulder(&base(), [-7.0, 0.0, 0.0], [7.0, 0.0, 0.0], 20.4, [0.0; 3])
            .expect("a long duel holds");
        assert!(
            height_share(&cam, [-7.0, 0.0]) > 0.28,
            "hero in the foreground"
        );
        assert!(
            height_share(&cam, [7.0, 0.0]) > 0.07,
            "rival still readable"
        );
        let eye = cam.eye.to_array();
        assert!(
            (eye[0] + 7.0).hypot(eye[2]) < 6.0,
            "eye rides on the hero's shoulder, not across the pit: {eye:?}"
        );
    }

    #[test]
    fn the_eye_stays_inside_the_pit_and_off_the_bodies() {
        // Hero against the +x wall, rival inward: the 4.5 m boom would leave
        // the pit, so it is pulled in rather than clamped across the hero.
        for hero_x in [14.0, 16.0, 18.0, 19.5] {
            let hero = [hero_x, 0.0, 3.0];
            let rival = [hero_x - 7.0, 0.0, -2.0];
            let Some(cam) = duel_shoulder(&base(), hero, rival, 20.4, [0.0; 3]) else {
                continue;
            };
            let eye = cam.eye.to_array();
            assert!(
                eye[0].abs() <= 20.4 + 1e-3 && eye[2].abs() <= 20.4 + 1e-3,
                "{eye:?}"
            );
            let off = |at: [f32; 3]| (at[0] - eye[0]).hypot(at[2] - eye[2]);
            assert!(off(hero) >= EYE_CLEAR_M && off(rival) >= EYE_CLEAR_M);
        }
        let near_wall = duel_shoulder(&base(), [18.0, 0.0, 3.0], [11.0, 0.0, -2.0], 20.4, [0.0; 3]);
        assert!(
            near_wall.is_some(),
            "a fight 2 m off the wall still gets the shoulder"
        );
    }

    #[test]
    fn the_live_spawn_pinned_to_the_wall_still_gets_the_shoulder() {
        // The arena's own start (seed 2040, 2 fighters): the hero 1.2 m off the
        // +x pit wall, the rival across the pit. Seen from behind the hero the
        // boom points into the wall, so it is pulled in (never through it).
        let (hero, rival) = ([19.185, 0.0, 3.438], [-16.6, 0.0, -11.156]);
        let cam = duel_shoulder(&base(), hero, rival, 20.4, [0.0; 3]).expect("the spawn holds");
        let eye = cam.eye.to_array();
        assert!(eye[0] <= 20.4 + 1e-3, "eye stays inside the pit: {eye:?}");
        assert!(
            (eye[0] - hero[0]).hypot(eye[2] - hero[2]) >= EYE_CLEAR_M,
            "off the hero's body: {eye:?}"
        );
        assert!(
            height_share(&cam, [hero[0], hero[2]]) > 0.4,
            "the hero fills the lower frame"
        );
        // Over the shoulder: the hero sits off the picture's center (the wall
        // takes the preferred shoulder here, so this one is the other), and the
        // rival ahead is not hidden behind the hero's back.
        assert!(
            ndc_x(&cam, [hero[0], hero[2]]).abs() > 0.2,
            "hero off the center of the frame: {}",
            ndc_x(&cam, [hero[0], hero[2]])
        );
    }

    /// The world wish W makes in the shoulder shot of `hero` against `rival`:
    /// the camera's floor heading through the window's camera-relative map.
    fn w_wish(hero: [f32; 3], rival: [f32; 3]) -> Option<(i8, i8)> {
        let cam = duel_shoulder(&base(), hero, rival, 20.4, [0.0; 3])?;
        let forward =
            crate::camera_wish::ground_forward(cam.eye.to_array(), cam.target.to_array())?;
        Some(crate::camera_wish::world_wish(forward, (100, 0)))
    }

    /// Walk W at 5 m/s, 60 Hz, re-aiming the shot from the hero's new spot
    /// each step, until the gap closes to `stop_m` or the shot lets go. Gives
    /// the hero's last spot and the steps taken.
    fn walk_w(mut hero: [f32; 3], rival: [f32; 3], stop_m: f32) -> ([f32; 3], u32) {
        let gap = |at: [f32; 3]| (at[0] - rival[0]).hypot(at[2] - rival[2]);
        let mut steps = 0;
        while gap(hero) > stop_m && steps < 60 * 20 {
            let Some((mx, mz)) = w_wish(hero, rival) else {
                break;
            };
            hero[0] += f32::from(mx) / 100.0 * 5.0 / 60.0;
            hero[2] += f32::from(mz) / 100.0 * 5.0 / 60.0;
            steps += 1;
        }
        (hero, steps)
    }

    #[test]
    fn w_at_the_live_spawn_heads_for_the_rival_and_not_into_the_wall() {
        // The owner's report (2026-10-18): the hero is pinned to the +x wall
        // and W, a world axis then, walked into it. Camera-relative, W points
        // the way the shoulder shot looks, at the rival across the pit.
        let (hero, rival) = ([19.185, 0.0, 3.438], [-16.6, 0.0, -11.156]);
        let (mx, mz) = w_wish(hero, rival).expect("the spawn holds");
        let (dx, dz) = (rival[0] - hero[0], rival[2] - hero[2]);
        let along = (f32::from(mx) * dx + f32::from(mz) * dz) / (100.0 * dx.hypot(dz));
        assert!(along > 0.9, "W points at the rival: {along} ({mx}, {mz})");
        assert!(mx < -50, "W leaves the +x wall behind: {mx}");
    }

    #[test]
    fn holding_w_in_the_shoulder_shot_walks_up_to_the_rival_from_any_side() {
        // Closed loop: the shot re-aims every step and W follows it. It must
        // converge on the rival, not orbit or drift, wherever he stands.
        let rival = [2.0, 0.0, -3.0];
        for turn in 0..8u8 {
            let angle = f32::from(turn) * core::f32::consts::FRAC_PI_4;
            let hero = [
                rival[0] + angle.cos() * 14.0,
                0.0,
                rival[2] + angle.sin() * 14.0,
            ];
            let (end, steps) = walk_w(hero, rival, 3.0);
            let gap = (end[0] - rival[0]).hypot(end[2] - rival[2]);
            assert!(gap <= 3.5, "side {turn}: {gap} m left after {steps} steps");
            assert!(
                steps < 60 * 6,
                "side {turn}: 14 m is under 6 s, took {steps}"
            );
        }
        // The pinned spawn, 38 m across the pit, arrives too.
        let (end, steps) = walk_w([19.185, 0.0, 3.438], [-16.6, 0.0, -11.156], 3.0);
        let gap = (end[0] + 16.6).hypot(end[2] + 11.156);
        assert!(gap <= 3.5, "spawn: {gap} m left after {steps} steps");
    }

    #[test]
    fn a_fighter_outside_the_pit_refuses_the_shot() {
        // Hero past the eye limit: no boom fits, the side-on fit decides.
        assert!(
            duel_shoulder(&base(), [21.0, 0.0, 0.0], [10.0, 0.0, 0.0], 20.4, [0.0; 3]).is_none()
        );
        // A non-finite position never builds a camera.
        assert!(duel_shoulder(
            &base(),
            [f32::NAN, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            20.4,
            [0.0; 3]
        )
        .is_none());
    }

    #[test]
    fn the_shot_never_flips_sides_and_replays_the_same_camera() {
        let a = duel_shoulder(&base(), [0.0; 3], [4.0, 0.0, 1.0], 20.4, [0.0; 3]).expect("holds");
        let b =
            duel_shoulder(&base(), [0.0; 3], [4.0, 0.0, 1.0], 20.4, [0.0; 3]).expect("holds x2");
        assert_eq!(a, b, "same inputs, same camera");
        // Over the hero's right shoulder: he stands in the left of the picture.
        assert!(
            ndc_x(&a, [0.0, 0.0]) < -0.1,
            "hero in the left of the frame: {}",
            ndc_x(&a, [0.0, 0.0])
        );
    }

    #[test]
    fn shake_moves_eye_and_target_together() {
        let calm =
            duel_shoulder(&base(), [0.0; 3], [3.0, 0.0, 0.0], 20.4, [0.0; 3]).expect("holds");
        let shaken = duel_shoulder(
            &base(),
            [0.0; 3],
            [3.0, 0.0, 0.0],
            20.4,
            [0.05, 0.02, -0.03],
        )
        .expect("holds");
        let (de, dt) = (shaken.eye - calm.eye, shaken.target - calm.target);
        assert!(
            (de - dt).length() < 1e-5,
            "the view direction holds: {de:?} vs {dt:?}"
        );
        assert!(de.length() > 0.01, "the frame did translate");
    }

    #[test]
    fn the_wall_shortens_the_boom_and_never_the_shoulder_offset() {
        // Clear pit: the whole boom. Wall at 10: the boom stops there, with the
        // sideways offset (0.8 m) kept whole.
        assert_eq!(
            pit_back_share([0.0, 0.0], [-4.0, 0.0], [0.0, 0.8], 20.0),
            1.0
        );
        let share = pit_back_share([8.0, 0.0], [4.0, 0.0], [0.0, 0.8], 10.0);
        assert!((share - 0.5).abs() < 1e-6, "2 m of 4 m: {share}");
        // A hero already past the wall, or whose shoulder spot is, gets none.
        assert_eq!(
            pit_back_share([11.0, 0.0], [1.0, 0.0], [0.0, 0.8], 10.0),
            0.0
        );
        assert_eq!(
            pit_back_share([0.0, 9.5], [-4.0, 0.0], [0.0, 0.8], 10.0),
            0.0
        );
    }
}
