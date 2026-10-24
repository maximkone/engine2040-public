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

use super::fighter_look_feet::ticks_f32;
use super::{FighterLook, LookFrame, SHAKE_MAX_M};
use crate::camera::camera_rig::{
    damper_step, halflife_to_damping, smooth_trauma_offset, trauma_gain, FightCamSpring,
};
use crate::camera::{CameraError, LookAhead, Vec3};
use crate::camera_frame::Lead;
use crate::camera_stack::{CameraStack, ShotOffer};

/// Which bodies the fight camera frames and how it holds them in the picture: the
/// soft refit before the springs and the containment after them.
#[path = "fighter_look_frame.rs"]
mod fighter_look_frame;

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
/// Walk bob, ported from the source game's `CameraBob` (`bobSpeed = 1`,
/// `mgtSpeed = 2`, `xAmp = 0.08`, `yAmp = 0.12`). The amplitude pair is a
/// normalized strength, not metres: at full strength the eye travels
/// `BOB_EYE_M` up and 2/3 of that across, which reads as a walk bob. Read as
/// metres the same pair shakes the picture like a hit; the source's units are
/// not in its assemblies, so this reading is the one assumption here.
const BOB_EYE_M: f32 = 0.02;
const BOB_SPEED: f32 = 1.0;
const BOB_MGT_SPEED: f32 = 2.0;
const BOB_X_AMP_M: f32 = BOB_EYE_M * (0.08 / 0.12);
const BOB_Y_AMP_M: f32 = BOB_EYE_M;
/// Stride the phase advances by, meters: one bob cycle per step of a walk.
const BOB_STRIDE_M: f32 = 0.75;
/// Hero step that counts as a full-speed walk, m/s: the bob fades in over
/// this and is full at the top walk speed of the demo.
const BOB_FULL_SPEED_MS: f32 = 5.0;
/// One frame's step longer than this is a respawn or a rope projection, not a
/// walk, meters: the bob holds instead of spinning.
const BOB_MAX_STEP_M: f32 = 0.35;
/// Feet above this are off the floor (a jump, a fall, a swing), meters: no
/// walk, no bob.
const BOB_AIRBORNE_M: f32 = 0.05;
/// FEEL7 landing dip: deepest eye drop, meters, on the landing tick.
const LAND_DIP_MAX_M: f32 = 0.12;
/// Fall height that earns the full landing dip, meters.
const LAND_DIP_FULL_FALL_M: f32 = 4.0;
/// Look ticks a landing dip eases out over.
const LAND_DIP_TICKS: u32 = 12;
/// Share of the dip a landing earns even from a small jump: the weight is
/// felt on every landing, and a long fall adds the rest.
const LAND_DIP_FLOOR: f32 = 0.3;

/// The smoothed state of the window's fight camera: the spring lanes plus the
/// eye height and lens damped beside them. One per window; a fresh one snaps
/// to its first shot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamFollow {
    spring: FightCamSpring,
    lens_damping: f32,
    height_m: f32,
    fov_y: f32,
    /// Walk bob phase, radians; advances with the distance the shot travels.
    bob_phase: f32,
    /// Walk bob amount in `0..=1`, smoothed over the hero's speed.
    bob_gain: f32,
    /// Damping of `bob_gain`, folded once from `BOB_MGT_SPEED`.
    bob_damping: f32,
    /// The hero's floor position last frame, for the step the bob follows.
    bob_last: Option<[f32; 2]>,
    /// Velocity look-ahead for the hero's walk; at rest unless armed.
    look_ahead: LookAhead,
    /// True when the look-ahead moves the shot (default off: today's camera).
    lead_armed: bool,
    /// Smoothed velocity of the framed bodies, for the framing's look-ahead.
    lead: Lead,
    /// The threat slot the fight shot frames, held while no other is clearly nearer.
    hint: Option<u8>,
    /// The director's shelf of shots (SW42): the shot this frame sits on it, a change
    /// of shot eases over its blend ticks, and the eye holds while the framed bodies
    /// stay inside the dead zone. Off until the window arms it.
    stack: CameraStack,
    /// True when the shelf moves the shot (arm with [`Self::with_camera_stack`]);
    /// an unarmed follower is the camera before SW42, byte for byte.
    stack_armed: bool,
    /// Test seam: the walk bob is held off, so a test can read the bob alone.
    #[cfg(test)]
    bob_off: bool,
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
            bob_phase: 0.0,
            bob_gain: 0.0,
            bob_damping: halflife_to_damping(1.0 / BOB_MGT_SPEED)?,
            bob_last: None,
            look_ahead: LookAhead::new(),
            lead_armed: false,
            lead: Lead::new(),
            hint: None,
            stack: CameraStack::default(),
            stack_armed: false,
            #[cfg(test)]
            bob_off: false,
        })
    }

    /// Test seam: the same follower with the walk bob held off, so a test can
    /// read the bob as the height gap against this twin on the same tape.
    #[cfg(test)]
    fn without_bob(mut self) -> Self {
        self.bob_off = true;
        self
    }

    /// Arm the 0.5 s velocity look-ahead ([`LookAhead`]): the presented eye
    /// and look point lead the hero's walk, ease in while it speeds up and
    /// decay when it stops. Off by default, so an existing follower shows
    /// today's camera; presentation only, never sim input or a hash.
    #[must_use]
    pub fn with_look_ahead(mut self) -> Self {
        self.lead_armed = true;
        self
    }

    /// Arm the director's shelf (SW42): the shot the frame asks for sits on it, a change
    /// of shot eases over its blend ticks instead of cutting, and the eye holds while the
    /// framed bodies stay inside the dead zone. Off by default, so every earlier camera
    /// and its tests show today's shot; presentation only, never sim input or a hash.
    #[must_use]
    pub fn with_camera_stack(mut self) -> Self {
        self.stack_armed = true;
        self
    }

    /// Current look-ahead offset, meters (zero while unarmed or at rest).
    #[must_use]
    pub fn lead_m(&self) -> Vec3 {
        self.look_ahead.offset_m()
    }

    /// Fold `shot` into the smoothed state over `dt_s` seconds and return the
    /// smoothed camera: `shot`'s clip planes, aspect and up, the spring's
    /// look point and bearing, the damped eye height and `lens_fov_y`
    /// damped. `hero` is the walking hero's root this frame (metres), which
    /// drives the walk bob. The first shot snaps.
    /// Errors: [`CameraError::BadParam`] for a bad `dt_s` or non-finite shot.
    fn follow(
        &mut self,
        shot: &Camera,
        lens_fov_y: f32,
        dt_s: f32,
        hero: [f32; 3],
    ) -> Result<Camera, CameraError> {
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
        // The hero's floor step drives both graders: the walk bob (distance)
        // and the look-ahead (velocity). A teleport is not motion.
        let delta = self.hero_step(hero);
        let step = delta[0].hypot(delta[1]);
        if self.lead_armed {
            let velocity = if step > BOB_MAX_STEP_M {
                Vec3::ZERO
            } else {
                Vec3::new(delta[0] / dt_s, 0.0, delta[1] / dt_s)
            };
            // Folded here and applied by `aim_camera` after the framing: a refit
            // that read the offset as part of its shot pulls the led picture apart.
            self.look_ahead.step(velocity, dt_s)?;
        }
        self.walk_bob(&mut camera, step, hero[1] <= BOB_AIRBORNE_M, dt_s)?;
        camera.fov_y = self.fov_y;
        Ok(camera)
    }

    /// The hero's planar step since the last frame, meters: `[0, 0]` on the
    /// first frame, after a teleport or for a non-finite draw. Also stores the
    /// position the next frame reads.
    fn hero_step(&mut self, hero: [f32; 3]) -> [f32; 2] {
        let at = [hero[0], hero[2]];
        let delta = self.bob_last.map_or([0.0, 0.0], |last| {
            let (dx, dz) = (at[0] - last[0], at[1] - last[1]);
            if dx.is_finite() && dz.is_finite() {
                [dx, dz]
            } else {
                [0.0, 0.0]
            }
        });
        self.bob_last = Some(at);
        delta
    }

    /// Walk bob (source game's `CameraBob`): the floor step the hero took this
    /// frame drives a stride-locked phase, the eye moves side to side once per
    /// stride and up and down twice, and a standing hero fades the bob to
    /// nothing. Distance driven, never scaled by the frame time, so the same
    /// walk has the same bob at any frame rate. A hero off the floor (jump,
    /// fall, swing) or one that teleports (respawn, rope projection) holds the
    /// bob still instead of shaking the picture.
    fn walk_bob(
        &mut self,
        camera: &mut Camera,
        step: f32,
        on_floor: bool,
        dt_s: f32,
    ) -> Result<(), CameraError> {
        let walking = on_floor && step <= BOB_MAX_STEP_M;
        let want = if walking && dt_s > 0.0 {
            (step / dt_s / BOB_FULL_SPEED_MS).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.bob_gain = damper_step(self.bob_gain, want, self.bob_damping, dt_s)?;
        let stride = if walking { step / BOB_STRIDE_M } else { 0.0 };
        self.bob_phase = (self.bob_phase + stride * BOB_SPEED * std::f32::consts::TAU)
            .rem_euclid(std::f32::consts::TAU);
        #[cfg(test)]
        if self.bob_off {
            return Ok(());
        }
        let gain = self.bob_gain;
        if gain <= 0.0 {
            return Ok(());
        }
        let lateral = BOB_X_AMP_M * gain * self.bob_phase.sin();
        let vertical = BOB_Y_AMP_M * gain * (2.0 * self.bob_phase).sin();
        let view = (camera.target - camera.eye).normalize_or_zero();
        let right = view.cross(camera.up).normalize_or_zero();
        let eye = camera.eye + right * lateral;
        camera.eye = [eye.x, eye.y + vertical, eye.z].into();
        Ok(())
    }
}

/// Hold the eye inside the square pit of half-width `limit`, meters, so the
/// smoothed arc around a moving look point never swings through the wall.
fn hold_in_pit(camera: &mut Camera, limit: f32) {
    camera.eye.x = camera.eye.x.clamp(-limit, limit);
    camera.eye.z = camera.eye.z.clamp(-limit, limit);
}

/// FEEL7 landing dip, meters, at `age` look ticks after a fall of `fall_m`:
/// full on the landing tick, eased to zero over [`LAND_DIP_TICKS`]. A jump
/// earns [`LAND_DIP_FLOOR`] of the dip; a 4 m fall or longer earns it all.
fn landing_dip_m(fall_m: f32, age: u32) -> f32 {
    if age >= LAND_DIP_TICKS || !fall_m.is_finite() {
        return 0.0;
    }
    let long_fall = (fall_m / LAND_DIP_FULL_FALL_M).clamp(0.0, 1.0);
    let strength = LAND_DIP_FLOOR + (1.0 - LAND_DIP_FLOOR) * long_fall;
    let decay = 1.0 - ticks_f32(age) / ticks_f32(LAND_DIP_TICKS);
    LAND_DIP_MAX_M * strength * decay
}

impl FighterLook {
    /// The walking hero's root this frame, metres: slot 0 is the window's
    /// human, drawn at the frame's sub-tick `alpha`.
    fn hero_root(&self, alpha: f32) -> [f32; 3] {
        match self.slots.first() {
            Some(slot) => super::slot_root(slot, alpha.clamp(0.0, 1.0)).0,
            None => [0.0; 3],
        }
    }
    /// The landing dip at `tick`, meters, when slot 0's feet hold a landing.
    fn landing_dip_at(&self, tick: u32) -> Option<f32> {
        self.slots
            .first()
            .and_then(|slot| slot.feet.land)
            .map(|land| landing_dip_m(land.fall_m, tick.saturating_sub(land.tick)))
    }
    /// Test-only: the camera dip the last landing earns at `tick`, meters.
    #[cfg(test)]
    pub(crate) fn feel7_landing_dip_m(&self, tick: u32) -> Option<f32> {
        self.landing_dip_at(tick)
    }
    /// The walking hero's floor speed, m/s: slot 0 is the window's human. A
    /// jump between ticks is clamped to the walk top speed, so a respawn or a
    /// cut never spins the bob.
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
        let hint = follow.hint;
        let calm = self.fight_camera_with(
            base,
            frame.shot,
            frame.fighters,
            frame.frame,
            [0.0; 3],
            hint,
        );
        let mut calm = self.frame_calm(follow, calm, frame);
        // The director's shelf (SW42), when the window arms it: the shot holds its place
        // while the framed bodies stay inside the dead zone, and a change of shot eases
        // over its blend ticks. It sits on the shot, before the springs, so the bob, the
        // shake and the look-ahead still move the eye on top of the held shot. An
        // unarmed follower (every test, the first frame of the window) takes the old path
        // byte for byte.
        if follow.stack_armed {
            let bodies = self.framed_subjects(follow, frame);
            let (key, priority) = Self::shot_offer(frame.shot);
            let offer = ShotOffer {
                key,
                priority,
                rig: Self::camera_rig(&calm),
            };
            if let Some(rig) = follow
                .stack
                .step(frame.frame, dt_s, &[offer], bodies.as_slice())
            {
                calm = Self::camera_at(&calm, &rig);
            }
        }
        let lens = calm.fov_y * (1.0 - LENS_PUNCH * trauma_gain(self.shake_trauma_permille));
        let dt_s = dt_s.clamp(MIN_DT_S, MAX_DT_S);
        let hero = self.hero_root(frame.alpha);
        let Ok(mut camera) = follow.follow(&calm, lens, dt_s, hero) else {
            return self.fight_camera(base, frame.shot, frame.fighters, frame.frame);
        };
        hold_in_pit(&mut camera, self.pit_limit_m());
        // The springs lag a swing or a tow: keep the framed bodies inside the safe frame now.
        self.frame_hold(follow, &mut camera, frame);
        // FEEL7: a fresh landing drops the eye for a beat, and the look point
        // follows a little, so the ground takes the weight the body does.
        let dip = self.landing_dip_at(self.tick).unwrap_or(0.0);
        if dip > 0.0 {
            camera.eye.y -= dip;
            camera.target.y -= dip * 0.35;
        }
        // The look-ahead is a final translate of the presented shot, after the framing
        // has read the picture: the refit must not see the led camera, or it reads the
        // offset as a body out of frame and pulls the armed shot away from the calm one.
        if follow.lead_armed {
            let lead = follow.lead_m();
            for point in [&mut camera.eye, &mut camera.target] {
                *point = [point.x + lead.x, point.y + lead.y, point.z + lead.z].into();
            }
        }
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
    fn feel7_a_landing_dips_the_eye_on_its_own_tick() {
        let mut look = super::super::tests::balance_look_two();
        let mut calm_follow = follower();
        let calm = look.aim_camera(&mut calm_follow, &base(), &frame(), FRAME_60_S);
        let fall_m = 1.5;
        look.tick = 10;
        look.slots[0].feet.land = Some(super::super::fighter_look_feet::Land { tick: 10, fall_m });
        let mut dipped_follow = follower();
        let dipped = look.aim_camera(&mut dipped_follow, &base(), &frame(), FRAME_60_S);
        assert!(
            dipped.eye.y < calm.eye.y - 0.02,
            "the landing dips the eye: {} vs {}",
            dipped.eye.y,
            calm.eye.y
        );
        assert!(landing_dip_m(0.4, 0) > 0.0, "a jump dips too");
        assert!(landing_dip_m(4.0, 0) > landing_dip_m(0.4, 0));
        assert_eq!(landing_dip_m(4.0, LAND_DIP_TICKS), 0.0);
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

    /// One look with an unarmed and an armed follower, both fed the same
    /// shots: any gap between them is the look-ahead alone.
    fn aim_pair(
        look: &mut FighterLook,
        plain: &mut CamFollow,
        lead: &mut CamFollow,
    ) -> (Camera, Camera) {
        (
            look.aim_camera(plain, &base(), &frame(), FRAME_60_S),
            look.aim_camera(lead, &base(), &frame(), FRAME_60_S),
        )
    }

    /// Walk the hero `walk_frames` frames at the full walk speed, then hold it
    /// still `rest_frames` frames; return both cameras per presented frame.
    fn run_lead_tape(walk_frames: usize, rest_frames: usize) -> Vec<(Camera, Camera)> {
        let mut look = super::super::tests::balance_look_two();
        let mut plain = follower();
        let mut lead = follower().with_look_ahead();
        let step_m = BOB_FULL_SPEED_MS * FRAME_60_S;
        place(&mut look, [0.0, 0.0], [1.0, 0.5]);
        let mut out = Vec::with_capacity(walk_frames + rest_frames + 1);
        for n in 0..walk_frames {
            walk_hero(&mut look, n as f32 * step_m, step_m);
            out.push(aim_pair(&mut look, &mut plain, &mut lead));
        }
        place(&mut look, [walk_frames as f32 * step_m, 0.0], [1.0, 0.5]);
        // One settle frame carries the half step to rest before the rest tape.
        let _ = aim_pair(&mut look, &mut plain, &mut lead);
        for _ in 0..rest_frames {
            out.push(aim_pair(&mut look, &mut plain, &mut lead));
        }
        out
    }

    #[test]
    fn look_ahead_leads_a_walking_hero_and_decays_when_stopped() {
        let walk_frames = 120;
        let tape = run_lead_tape(walk_frames, 90);
        let gap = |pair: &(Camera, Camera)| (pair.1.target - pair.0.target).length();
        assert!(
            gap(&tape[0]) < 1e-4,
            "the first frame snaps, no lead yet: {}",
            gap(&tape[0])
        );
        let peak = tape.iter().fold(0.0f32, |p, pair| p.max(gap(pair)));
        assert!(peak > 0.1, "a walking hero gets a lead, peak {peak} m");
        assert!(
            peak <= crate::camera::LEAD_MAX_M + 1e-3,
            "the lead is capped, peak {peak} m"
        );
        let last = gap(tape.last().expect("frames"));
        assert!(last < 1e-3, "a stopped hero drops the lead, {last} m");
        // The stop decays, never snaps: each rest frame is no larger.
        for pair in tape.windows(2).skip(walk_frames - 1) {
            assert!(
                gap(&pair[1]) <= gap(&pair[0]) + 1e-6,
                "the lead decays after the stop: {} then {}",
                gap(&pair[0]),
                gap(&pair[1])
            );
        }
        assert_eq!(
            tape,
            run_lead_tape(walk_frames, 90),
            "the same tape twice gives the same pose"
        );
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
            .follow(&down, down.fov_y, FRAME_60_S, [0.0; 3])
            .expect("passes");
        assert_eq!(out, down);
    }

    /// Walk the hero to `x` this frame with `step_m` of travel behind them.
    fn walk_hero(look: &mut FighterLook, x: f32, step_m: f32) {
        look.slots[0].pos = [x, 0.0, 0.0];
        look.slots[0].prev_pos = [x - step_m, 0.0, 0.0];
    }

    /// The bob comes from the hero's own walk: a standing hero holds the eye
    /// still, a walking hero lifts and drops it inside the source amplitudes,
    /// and it fades out when the walk stops. The bob is read against a twin
    /// follower with the bob off on the same tape, so the framing's own eye
    /// height (it follows the pair as they part) does not read as bob.
    #[test]
    fn walk_bob_moves_the_eye_only_while_the_hero_walks() {
        let mut look = super::super::tests::balance_look_two();
        let mut follow = follower();
        let mut twin = follower().without_bob();
        place(&mut look, [0.0, 0.0], [1.0, 0.5]);
        let mut still = Vec::new();
        for _ in 0..90 {
            let camera = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            let quiet = look.aim_camera(&mut twin, &base(), &frame(), FRAME_60_S);
            still.push(camera.eye.y - quiet.eye.y);
        }
        let worst = still.iter().fold(0.0f32, |w, v| w.max(v.abs()));
        assert!(worst < 1.0e-5, "a standing hero has no bob: {worst} m");

        let step = BOB_FULL_SPEED_MS * FRAME_60_S;
        let mut bob = Vec::new();
        for n in 0..90 {
            walk_hero(&mut look, n as f32 * step, step);
            let camera = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
            let quiet = look.aim_camera(&mut twin, &base(), &frame(), FRAME_60_S);
            bob.push(camera.eye.y - quiet.eye.y);
        }
        let (lo, hi) = bob
            .iter()
            .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(
            follow.bob_gain > 0.5,
            "the bob fades in: {}",
            follow.bob_gain
        );
        assert!(hi - lo > 0.01, "the walking eye bobs: range {} m", hi - lo);
        assert!(
            hi <= BOB_Y_AMP_M + 0.01 && lo >= -BOB_Y_AMP_M - 0.01,
            "inside the source amplitude: {lo}..{hi}"
        );

        place(&mut look, [90.0 * step, 0.0], [1.0, 0.5]);
        for _ in 0..240 {
            let _ = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        }
        assert!(
            follow.bob_gain < 0.01,
            "the bob fades out: {}",
            follow.bob_gain
        );

        // A hero off the floor never bobs, however fast it is drawn.
        for n in 0..60 {
            let x = 90.0 * step + n as f32 * step;
            walk_hero(&mut look, x, step);
            look.slots[0].pos[1] = 0.8;
            look.slots[0].prev_pos[1] = 0.8;
            let _ = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        }
        assert!(
            follow.bob_gain < 0.01,
            "airborne feet hold the bob: {}",
            follow.bob_gain
        );

        // A teleport (respawn, rope projection) is not a walk either.
        for n in 0..40 {
            let x = 200.0 + n as f32 * 12.0;
            walk_hero(&mut look, x, 12.0);
            let _ = look.aim_camera(&mut follow, &base(), &frame(), FRAME_60_S);
        }
        assert!(
            follow.bob_gain < 0.01,
            "a teleporting hero holds the bob: {}",
            follow.bob_gain
        );
    }
}
