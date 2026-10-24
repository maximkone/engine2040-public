//! The legionary's first person on the window: the camera the frame takes from the head's own eye
//! point (row POV2), the spawn that looks into the arena, and the view laws folded with the camera
//! that is drawn. The rig, the spring and the laws live in `engine_game::fps_view::pov`; this is
//! the window's use of them. Presentation only: the simulation, the tape and every hash are
//! untouched.

use super::*;
use engine_game::fps_view::pov::{spawn_yaw, PovLens};

impl ArenaApp {
    /// Aim the frame at the player's eye while the legionary mode plays in first person. The eye
    /// looks along the lock-on bearing the sim swings at (so a spawn never stares into a wall)
    /// from the head's own eye point riding the mount spring (row POV2); the aim ray is the FV1
    /// eye and never rides the spring, so the picture shakes and the aim does not. The eye-space
    /// trauma kick folds into the picture only. The lens and the far plane stay the ones of the
    /// frame it replaces, so the third person toggle changes the place and nothing else.
    /// Returns `true` when the frame camera moved to the eye.
    pub(crate) fn aim_legionary_eye(&mut self, list: &mut DrawList) -> bool {
        self.sync_pov_slot();
        if self.cam_mode == CameraMode::Free {
            return false;
        }
        let Some(mode) = self.legionary.as_ref() else {
            return false;
        };
        if mode.view != LegionaryView::Eye {
            self.pov_rig.reset();
            return false;
        }
        let slot = self.app.human_slot();
        let Some(view) = self.app.fighter_views().iter().find(|one| one.slot == slot) else {
            return false;
        };
        // Only the blows the player is in, decaying between ticks: every cue in the arena for
        // one tick snapped the view in a crowded fight (see `EyeTrauma`).
        let trauma = self
            .eye_trauma
            .feed(self.ticks_done, &self.tick_scratch.cues, slot);
        let rival = nearest_rival_facing(slot, self.app.fighter_views(), self.app.hp());
        let yaw = spawn_yaw(view.pos_m, view.yaw, rival, self.config.half_extent_m);
        let body = self
            .look
            .as_ref()
            .and_then(|look| look.view_truth().iter().find(|one| one.slot == slot))
            .and_then(|truth| truth.pov.as_deref());
        let eye = match self.pov_rig.eye(
            view.pos_m,
            yaw,
            body,
            trauma,
            self.ticks_done,
            self.cam_dt_s,
        ) {
            Ok(pose) => pose.view,
            Err(err) => {
                tell!("arena: legionary eye: {err}");
                return false;
            }
        };
        let mut camera = match engine_game::fps_view::camera_for_eye(
            &eye,
            list.camera.fov_y,
            list.camera.aspect,
        ) {
            Ok(camera) => camera,
            Err(err) => {
                tell!("arena: legionary camera: {err}");
                return false;
            }
        };
        // The wearer's head is hidden from its own eye, so the near clip stands close; the far
        // plane stays the one of the frame this camera replaces.
        camera.near = LEGIONARY_EYE_NEAR_M;
        camera.far = list.camera.far;
        if let Err(err) = camera.validate() {
            tell!("arena: legionary camera: {err}");
            return false;
        }
        list.camera = camera;
        self.note_pov_frame(slot, &list.camera);
        true
    }

    /// Tell the look and the view laws whether the next frame is the first person: the human's
    /// body then draws without its head and carries its own pose, and the director's frame is
    /// not folded into the laws (the eye's own camera is, in `note_pov_frame`). Runs every frame
    /// from `aim_legionary_eye`, so a toggle takes hold on the next frame.
    fn sync_pov_slot(&mut self) {
        let slot = self.pov_frame().then(|| self.app.human_slot());
        if let Some(look) = self.look.as_mut() {
            look.set_pov_slot(slot);
        }
        self.view.set_pov_slot(slot);
    }

    /// True when this frame draws the legionary's first person: the mode is on, its view is the
    /// eye and the free camera is not out. Presentation only.
    pub(crate) fn pov_frame(&self) -> bool {
        self.cam_mode != CameraMode::Free
            && self
                .legionary
                .as_ref()
                .is_some_and(|mode| mode.view == LegionaryView::Eye)
    }

    /// Fold the first-person frame into the view laws with the camera that is drawn: the near
    /// plane cuts, the screen centre and the share of the picture the wearer's own geometry covers
    /// count into `eye_in_body` (`ViewLaws::note_frame_pov`). The director's frame is not folded
    /// in first person (`ViewLaws::set_pov_slot`): that camera is not the one on screen. A run
    /// with no look bound (the staged boxes) folds nothing.
    fn note_pov_frame(&mut self, slot: u8, camera: &engine_render::v1::Camera) {
        let Some(look) = self.look.as_ref() else {
            return;
        };
        let t = look.sim_time(self.last_alpha);
        let lens = PovLens::look_at(
            camera.eye.to_array(),
            camera.target.to_array(),
            camera.up.to_array(),
            camera.fov_y,
            camera.aspect,
            camera.near,
        );
        // No cover yet (the first frame, before the look knew the slot): fold nothing.
        if let Some(cover) = lens.and_then(|lens| look.pov_cover(slot, lens)) {
            self.view
                .note_frame_pov(camera, t, look.view_truth(), slot, &cover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> ArenaApp {
        let args: Vec<String> = ["arena", "--frames", "60", "--exit"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let args = WinArgs::parse(&args).expect("args parse");
        let config = GameConfig {
            seed: 2040,
            ticks: 24,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = ArenaApp::new(config, args, None).expect("session builds");
        app.enable_legionary();
        app
    }

    #[test]
    fn the_first_person_eye_faces_the_rival_from_the_first_tick() {
        // The audit's spawn: the sim's yaw reads 0 on tick 1 (along the wall), and the eye used to
        // stare along it. It looks at the rival the sim swings at.
        let mut app = app();
        app.run_sim_ticks(1, app.human().expect("human slot"));
        let mut list = hud_draw_list(1280, 720).expect("hud list builds");
        assert!(app.aim_legionary_eye(&mut list), "the eye takes the frame");
        let views = app.app.fighter_views();
        let slot = app.app.human_slot();
        let me = views.iter().find(|one| one.slot == slot).expect("me");
        let rival = views.iter().find(|one| one.slot != slot).expect("rival");
        let (dx, dz) = (rival.pos_m[0] - me.pos_m[0], rival.pos_m[2] - me.pos_m[2]);
        let want = dx.hypot(dz);
        let eye = list.camera.eye;
        let look = list.camera.target;
        let (fx, fz) = (look.x - eye.x, look.z - eye.z);
        let got = fx.hypot(fz);
        let facing = (dx * fx + dz * fz) / (want * got);
        assert!(
            facing > 0.999,
            "the eye faces {facing} of the way to the rival"
        );
    }

    #[test]
    fn the_first_person_camera_sits_at_the_eye_with_the_close_near_plane() {
        let mut app = app();
        app.run_sim_ticks(1, app.human().expect("human slot"));
        let mut list = hud_draw_list(1280, 720).expect("hud list builds");
        assert!(app.aim_legionary_eye(&mut list));
        assert!(list.camera.eye.y > 1.0, "the eye sits over the feet");
        assert_eq!(list.camera.near, LEGIONARY_EYE_NEAR_M);
        assert!(
            list.camera.near < 0.15,
            "the head is hidden from its own eye, so the plane stands close"
        );
    }

    #[test]
    fn the_third_person_toggle_hands_the_frame_back_and_lets_the_spring_go() {
        let mut app = app();
        app.run_sim_ticks(1, app.human().expect("human slot"));
        assert!(app.pov_frame());
        app.toggle_legionary_view();
        assert!(!app.pov_frame(), "third person is not the eye's frame");
        let mut list = hud_draw_list(1280, 720).expect("hud list builds");
        assert!(!app.aim_legionary_eye(&mut list), "the eye steps aside");
        app.toggle_legionary_view();
        assert!(app.pov_frame());
    }
}
