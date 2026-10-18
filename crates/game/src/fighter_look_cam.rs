//! fighter_look_cam: the window's stateful fight camera (GP43).
//!
//! Owner: game lane, a child of `fighter_look`. Consumer: `bin/arena.rs` aims
//! every presented frame through [`FighterLook::aim_camera`]. The stateless
//! [`FighterLook::fight_camera`] picks the eye and look point for one instant,
//! which is a hard cut whenever the shot, the pair or the pit wall changes it.
//! The follower here runs that calm shot (no shake) through the exact halflife
//! [`FightCamSpring`] on yaw, distance and look point, damps eye height and
//! the lens the same way, then adds what must stay sharp after the smoothing:
//! the LF11 shake, a roll from the trauma noise and a short lens punch.
//! Units: meters, radians, seconds of wall time per presented frame.
//! Determinism: presentation only; the same shots, trauma and frame times
//! give the same camera twice, and the tick hash never sees it. At any frame
//! rate the same shot track ends on the same camera (the spring is exact in
//! `dt`). Errors: a frame time or shot the spring refuses hands back the
//! unsmoothed shot, never a broken picture. Budget: a few dozen float ops, no
//! heap.

use engine_render::v1::Camera;

use super::{FighterLook, LookFrame, SHAKE_MAX_M};
use crate::camera::camera_rig::{
    damper_step, halflife_to_damping, smooth_trauma_offset, trauma_gain, FightCamSpring,
};
use crate::camera::{CameraError, Vec3};

/// Halflife of yaw, distance, look point and eye height, seconds: a cut or a
/// moving pair settles over about half a second, a steady drift lags a few
/// tenths of a second behind its target, never a metre.
const POSE_HALFLIFE_S: f32 = 0.2;
/// Halflife of the lens, seconds: short, so a hit's punch reads before it fades.
const LENS_HALFLIFE_S: f32 = 0.06;
/// Share of the lens a full trauma narrows: a Hit (400 permille) pushes in by
/// about one degree on the 60 degree fight lens, a knockdown by more.
const LENS_PUNCH: f32 = 0.08;
/// Shortest eye-to-look reach on the floor with a bearing to follow, meters; a
/// shot looking straight down has none and passes through unsmoothed.
const MIN_REACH_M: f32 = 0.05;
/// Frame time bounds, seconds: a stall of a second smooths as a tenth of one
/// (the pair is long since elsewhere) and a zero or negative time still steps.
const MIN_DT_S: f32 = 1.0e-4;
const MAX_DT_S: f32 = 0.1;

/// The smoothed state of the window's fight camera: the spring lanes plus the
/// eye height and lens damped beside them. One per window; a fresh one snaps
/// to its first shot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamFollow {
    spring: FightCamSpring,
    lens_damping: f32,
    height_m: f32,
    fov_y: f32,
}

impl CamFollow {
    /// A follower with the shipped halflives, empty until its first shot.
    /// Errors: none for the shipped constants; a bad halflife is refused
    /// loudly by the spring.
    pub fn new() -> Result<Self, CameraError> {
        Ok(Self {
            spring: FightCamSpring::new(POSE_HALFLIFE_S)?,
            lens_damping: halflife_to_damping(LENS_HALFLIFE_S)?,
            height_m: 0.0,
            fov_y: 0.0,
        })
    }

    /// Fold `shot` into the smoothed state over `dt_s` seconds and return the
    /// smoothed camera: `shot`'s clip planes, aspect and up, the spring's
    /// look point and bearing, the damped eye height and `lens_fov_y`
    /// damped. The first shot snaps.
    /// Errors: [`CameraError::BadParam`] for a bad `dt_s` or non-finite shot.
    fn follow(&mut self, shot: &Camera, lens_fov_y: f32, dt_s: f32) -> Result<Camera, CameraError> {
        let offset = shot.eye - shot.target;
        let reach = offset.x.hypot(offset.z);
        if reach < MIN_REACH_M {
            return Ok(shot.clone());
        }
        let snap = self.spring.smoothed().is_none();
        let look = Vec3::new(shot.target.x, shot.target.y, shot.target.z);
        let (yaw, dist, look) = self
            .spring
            .step(offset.x.atan2(offset.z), reach, look, dt_s)?;
        let mut camera = shot.clone();
        if snap {
            // The first shot is the shot itself, bit for bit.
            self.height_m = offset.y;
            self.fov_y = lens_fov_y;
            camera.fov_y = lens_fov_y;
            return Ok(camera);
        }
        self.height_m = damper_step(self.height_m, offset.y, self.spring.damping(), dt_s)?;
        self.fov_y = damper_step(self.fov_y, lens_fov_y, self.lens_damping, dt_s)?;
        camera.target = [look.x, look.y, look.z].into();
        camera.eye = [
            look.x + yaw.sin() * dist,
            look.y + self.height_m,
            look.z + yaw.cos() * dist,
        ]
        .into();
        camera.fov_y = self.fov_y;
        Ok(camera)
    }
}

/// Hold the eye inside the square pit of half-width `limit`, meters, so the
/// smoothed arc around a moving look point never swings through the wall.
fn hold_in_pit(camera: &mut Camera, limit: f32) {
    camera.eye.x = camera.eye.x.clamp(-limit, limit);
    camera.eye.z = camera.eye.z.clamp(-limit, limit);
}

impl FighterLook {
    /// The fight camera for one presented frame, smoothed over `dt_s` seconds
    /// of wall time through `follow` (GP43). The calm shot of
    /// [`Self::fight_camera`] goes through the springs; then the eye is held
    /// inside the pit, the LF11 shake moves eye and target together (so it is
    /// never low-passed away), the trauma noise rolls the horizon and a hit
    /// pushes the lens in. Zero trauma leaves a settled shot exactly the
    /// smoothed one. Presentation only.
    #[must_use]
    pub fn aim_camera(
        &self,
        follow: &mut CamFollow,
        base: &Camera,
        frame: &LookFrame,
        dt_s: f32,
    ) -> Camera {
        let calm = self.fight_camera_with(base, frame.shot, frame.fighters, frame.frame, [0.0; 3]);
        let lens = calm.fov_y * (1.0 - LENS_PUNCH * trauma_gain(self.shake_trauma_permille));
        let dt_s = dt_s.clamp(MIN_DT_S, MAX_DT_S);
        let Ok(mut camera) = follow.follow(&calm, lens, dt_s) else {
            return self.fight_camera(base, frame.shot, frame.fighters, frame.frame);
        };
        hold_in_pit(&mut camera, self.pit_limit_m());
        let shake = self.fight_shake_vec();
        for point in [&mut camera.eye, &mut camera.target] {
            *point = [point.x + shake[0], point.y + shake[1], point.z + shake[2]].into();
        }
        let roll = smooth_trauma_offset(
            self.shake_trauma_permille,
            self.tick,
            frame.alpha.clamp(0.0, 1.0),
            SHAKE_MAX_M,
        )
        .map_or(0.0, |trauma| trauma.roll_rad);
        // Roll turns the up vector about the view axis (Rodrigues).
        let forward = (camera.target - camera.eye).normalize_or_zero();
        let (sin, cos) = roll.sin_cos();
        let up = camera.up;
        camera.up = up * cos + forward.cross(up) * sin + forward * (forward.dot(up) * (1.0 - cos));
        if camera.validate().is_ok() {
            camera
        } else {
            self.fight_camera(base, frame.shot, frame.fighters, frame.frame)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fighter_look::Shot;
    use crate::FighterView;
    use engine_audio::FightCue;

    const FRAME_30_S: f32 = 1.0 / 30.0;
    const FRAME_60_S: f32 = 1.0 / 60.0;
    const FRAME_144_S: f32 = 1.0 / 144.0;

    fn base() -> Camera {
        Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        )
    }

    fn frame() -> LookFrame {
        LookFrame {
            fighters: 2,
            alpha: 0.5,
            shot: Shot::Action,
            frame: 0,
        }
    }

    fn two_views() -> [FighterView; 2] {
        [
            FighterView {
                slot: 0,
                pos_m: [-1.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [1.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ]
    }

    fn follower() -> CamFollow {
        CamFollow::new().expect("follower builds")
    }

    /// Put the hero and the rival on the floor at `(x, z)`, this very frame.
    fn place(look: &mut FighterLook, hero: [f32; 2], rival: [f32; 2]) {
        for (slot, at) in look.slots.iter_mut().zip([hero, rival]) {
            slot.pos = [at[0], 0.0, at[1]];
            slot.prev_pos = slot.pos;
        }
    }

    fn eye_gap_m(a: &Camera, b: &Camera) -> f32 {
        (a.eye - b.eye).length()
    }

    /// Run the follower over `steps` frames of `dt_s` with the rival walking
    /// away along +x at 1 m/s, and return every camera.
    fn run_drift(dt_s: f32, steps: usize) -> Vec<Camera> {
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        (0..steps)
            .map(|n| {
                place(&mut look, [-1.0, 0.0], [1.0 + n as f32 * dt_s, 0.5]);
                look.aim_camera(&mut follow, &base(), &frame(), dt_s)
            })
            .collect()
    }

    #[test]
    fn aim_camera_first_frame_is_the_stateless_shot_and_a_still_pair_stays_there() {
        let look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let raw = look.fight_camera(&base(), Shot::Action, 2, 0);
        let first = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        assert_eq!(first, raw, "the first shot snaps, bit for bit");
        for _ in 0..120 {
            let held = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            assert!(
                eye_gap_m(&held, &raw) < 1.0e-4,
                "a still pair holds the shot"
            );
            assert!((held.fov_y - raw.fov_y).abs() < 1.0e-6);
        }
    }

    #[test]
    fn aim_camera_glides_a_cut_the_stateless_shot_jumps() {
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let _ = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        // The rival is shoved across the pit in one tick: the raw shot hard-cuts.
        let before = look.fight_camera(&base(), Shot::Action, 2, 0);
        place(&mut look, [-1.0, 0.0], [5.0, 4.0]);
        let raw = look.fight_camera(&base(), Shot::Action, 2, 0);
        assert!(eye_gap_m(&before, &raw) > 1.0, "the raw shot jumps");
        let mut prev = before;
        let mut worst = 0.0f32;
        for _ in 0..240 {
            let now = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            worst = worst.max(eye_gap_m(&prev, &now));
            prev = now;
        }
        assert!(
            worst <= 0.25,
            "the eye moves {worst} m in one frame, over 25 cm"
        );
        assert!(eye_gap_m(&prev, &raw) < 0.01, "and lands on the new shot");
    }

    #[test]
    fn aim_camera_ends_on_the_same_camera_at_30_and_144_fps() {
        // Both end at t = 3 s exactly, the drift still running.
        let (slow, fast) = (run_drift(FRAME_30_S, 91), run_drift(FRAME_144_S, 433));
        let (a, b) = (slow.last().expect("frames"), fast.last().expect("frames"));
        assert!(
            eye_gap_m(a, b) <= 0.01,
            "30 vs 144 fps eyes {} m apart",
            eye_gap_m(a, b)
        );
        assert!(
            (a.target - b.target).length() <= 0.01,
            "and the look points"
        );
        assert_eq!(
            run_drift(FRAME_30_S, 91),
            slow,
            "the same track replays exactly"
        );
    }

    #[test]
    fn aim_camera_keeps_the_circling_pair_in_the_picture() {
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let (mut both, mut frames) = (0u32, 0u32);
        // The pair circles its midpoint at 120 degrees a second for six seconds.
        for n in 0..360u32 {
            let angle = n as f32 * FRAME_60_S * 120.0f32.to_radians();
            let (sin, cos) = angle.sin_cos();
            place(&mut look, [-1.5 * cos, -1.5 * sin], [1.5 * cos, 1.5 * sin]);
            let camera = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            let in_view = look.fighters_in_view(&camera, 2);
            assert!(in_view >= 1, "frame {n} shows nobody");
            both += u32::from(in_view == 2);
            frames += 1;
        }
        assert!(
            both * 100 >= frames * 90,
            "both fighters in {both} of {frames} frames"
        );
    }

    /// One tick of the human's input: a world wish and no buttons.
    fn walk_input(tick: u32, mx: i8, mz: i8) -> engine_input::FighterInput {
        engine_input::FighterInput {
            tick,
            fighter: engine_input::FighterId::new(crate::HUMAN_SLOT).expect("human slot"),
            mx,
            mz,
            facing: 0,
            buttons: engine_input::Buttons(0),
        }
    }

    /// Frames of the 600-tick two-fighter arena tape of `seed` (the 20 m pit)
    /// that show both fighters, through the whole live stack: the sim, the
    /// look folded tick by tick and the follower aiming one present a tick.
    /// The human walks `legs` (a world wish held 120 ticks each, cycling).
    fn frames_with_both_in_view(seed: u64, legs: &[(i8, i8)]) -> u32 {
        let mut app = crate::App::new(crate::GameConfig {
            seed,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        })
        .expect("session builds");
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let mut both = 0;
        for tick in 0..600u32 {
            let (mx, mz) = legs[(tick / 120) as usize % legs.len()];
            let outcome = app.tick(walk_input(tick, mx, mz));
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            let camera = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            both += u32::from(look.fighters_in_view(&camera, 2) == 2);
        }
        both
    }

    #[test]
    fn aim_camera_keeps_both_fighters_in_frame_on_the_real_arena_tapes() {
        // PB57: the seeds the playtests lost the fight on (6800 spawn and the walk
        // after it, 6900 drift), idle and walking the pit, 600 of 600 ticks.
        let walks: [&[(i8, i8)]; 3] = [
            &[(0, 0)],
            &[(100, 0), (0, 100), (-100, 0), (0, -100)],
            &[(-100, -100), (100, 100)],
        ];
        for seed in [6800u64, 6900] {
            for legs in walks {
                let both = frames_with_both_in_view(seed, legs);
                assert_eq!(
                    both, 600,
                    "seed {seed} legs {legs:?}: both in {both} of 600"
                );
            }
        }
    }

    /// A look about to take trauma and a calm twin, each with its own follower,
    /// both warmed on half a second of the same calm duel (ticks 0..30).
    struct Twins {
        hit: FighterLook,
        hit_follow: CamFollow,
        calm: FighterLook,
        calm_follow: CamFollow,
    }

    impl Twins {
        fn new() -> Self {
            let views = two_views();
            let mut twins = Self {
                hit: super::super::tests::balance_look_two(),
                hit_follow: follower(),
                calm: super::super::tests::balance_look_two(),
                calm_follow: follower(),
            };
            for tick in 0..30 {
                twins.hit.observe_tick(tick, &views, &[1000, 1000], &[]);
                twins.calm.observe_tick(tick, &views, &[1000, 1000], &[]);
                twins.aim();
            }
            twins
        }

        /// One presented frame on each look: the trauma twin, then the calm one.
        fn aim(&mut self) -> (Camera, Camera) {
            (
                self.hit
                    .aim_camera(&mut self.hit_follow, &base(), &frame(), FRAME_60_S),
                self.calm
                    .aim_camera(&mut self.calm_follow, &base(), &frame(), FRAME_60_S),
            )
        }

        /// Tick `tick`: the first look takes two knockdowns, the twin none.
        fn knock_down(&mut self, tick: u32) {
            let views = two_views();
            let knockdown = FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            };
            self.hit
                .observe_tick(tick, &views, &[1000, 0], &[knockdown, knockdown]);
            self.calm.observe_tick(tick, &views, &[1000, 0], &[]);
        }
    }

    #[test]
    fn aim_camera_adds_the_shake_after_the_smoothing_and_punches_the_lens() {
        let mut twins = Twins::new();
        twins.knock_down(30);
        assert_eq!(
            twins.hit.shake_trauma_permille, 1000,
            "two knockdowns saturate"
        );
        let (mut roll_peak, mut punch_first, mut punch_peak) = (0.0f32, None, 0.0f32);
        for n in 0..30 {
            let (a, b) = twins.aim();
            // The pose lanes do not know about trauma: the whole eye gap is the shake.
            let shake = twins.hit.fight_shake_vec();
            let gap = a.eye - b.eye;
            for (got, want) in [(gap.x, shake[0]), (gap.y, shake[1]), (gap.z, shake[2])] {
                assert!(
                    (got - want).abs() < 1.0e-4,
                    "frame {n}: shake {got} vs {want}"
                );
            }
            roll_peak = roll_peak.max(a.up.dot(b.up).clamp(-1.0, 1.0).acos());
            let punch = b.fov_y - a.fov_y;
            punch_first.get_or_insert(punch);
            punch_peak = punch_peak.max(punch);
        }
        assert!(
            roll_peak > 1.0e-4,
            "the horizon rolls, peak {roll_peak} rad"
        );
        assert!(roll_peak <= 0.1 + 1.0e-4, "inside the documented roll cap");
        // The lens eases in over its short halflife and ends on the full punch.
        let first = punch_first.expect("frames ran");
        assert!(
            first > 0.0 && first < 0.5 * punch_peak,
            "punch {first} then {punch_peak}"
        );
        assert!(
            punch_peak > 0.04,
            "the lens pushes in, peak {punch_peak} rad"
        );
    }

    #[test]
    fn aim_camera_settles_back_to_the_calm_shot_when_the_trauma_is_gone() {
        let mut twins = Twins::new();
        twins.knock_down(30);
        let views = two_views();
        for tick in 31..=110 {
            twins.hit.observe_tick(tick, &views, &[1000, 0], &[]);
            twins.calm.observe_tick(tick, &views, &[1000, 0], &[]);
            twins.aim();
        }
        assert_eq!(
            twins.hit.shake_trauma_permille, 0,
            "a second of decay clears it"
        );
        let (a, b) = twins.aim();
        assert!(eye_gap_m(&a, &b) < 1.0e-4 && (a.fov_y - b.fov_y).abs() < 1.0e-4);
        assert!(
            (a.up - b.up).length() < 1.0e-4,
            "the horizon is level again"
        );
    }

    #[test]
    fn aim_camera_refuses_a_bad_frame_time_by_showing_the_stateless_shot() {
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let _ = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        place(&mut look, [-1.0, 0.0], [4.0, 3.0]);
        let raw = look.fight_camera(&base(), Shot::Action, 2, 0);
        let nan = look.aim_camera(&mut follow, &base(), &frame(), f32::NAN);
        assert_eq!(nan, raw, "a NaN frame time shows the unsmoothed shot");
        // A zero or negative time steps as the shortest legal one, never panics.
        let zero = look.aim_camera(&mut follow, &base(), &frame(), 0.0);
        assert!(zero.validate().is_ok());
    }

    #[test]
    fn hold_in_pit_clamps_each_floor_axis_and_leaves_the_height() {
        let mut camera = Camera::new([12.0, 3.0, -15.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0);
        hold_in_pit(&mut camera, 9.4);
        assert_eq!(camera.eye.to_array(), [9.4, 3.0, -9.4]);
        hold_in_pit(&mut camera, f32::MAX);
        assert_eq!(camera.eye.to_array(), [9.4, 3.0, -9.4], "no ring, no clamp");
    }

    #[test]
    fn a_straight_down_shot_has_no_bearing_and_passes_through() {
        let mut follow = follower();
        let down = Camera::new([0.0, 20.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.0], 1.0);
        let out = follow
            .follow(&down, down.fov_y, FRAME_60_S)
            .expect("passes");
        assert_eq!(out, down);
    }
}
