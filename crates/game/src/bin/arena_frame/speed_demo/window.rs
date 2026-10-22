//! window (SD7): the speed demo's hooks on the arena window. `--speed-demo` turns it on;
//! the window steps it once per sim tick, hands it keys and clicks, lets it replace the
//! frame's draws and camera, and prints its exit line. Each hook is one call in the
//! window, so the pinned window file barely grows.

use engine_render::present::DrawList;
use winit::keyboard::KeyCode;

use super::scene::SpeedDemo;
use crate::ArenaApp;

impl ArenaApp {
    /// Build the demo world (`--speed-demo`; `--nyc` picks the procedural New York). A
    /// `hands_off` run (`--exit` captures) ignores keys and clicks: the autopilot drives it,
    /// whatever lands in the window while it has focus.
    pub(crate) fn enable_speed_demo(&mut self, nyc: bool, hands_off: bool) -> Result<(), String> {
        let mut demo = SpeedDemo::new(nyc)?;
        demo.hands_off = hands_off;
        demo.night = self.night;
        self.speed = Some(demo);
        say!("arena: speed demo: Tab fly/swing/drive | W/S speed (W reels the web) | A/D turn | arrows pitch | Space/Ctrl up/down, Space web | E or click sword | F always-faster");
        Ok(())
    }

    /// One fixed sim tick of the demo.
    pub(crate) fn step_speed_demo(&mut self) {
        if let Some(demo) = self.speed.as_mut() {
            demo.step(engine_core::DT);
        }
    }

    /// The demo replaces the frame's draws and camera; a frame it cannot make drops the
    /// demo with a notice (the arena underneath draws again).
    pub(crate) fn push_speed_demo(&mut self, list: &mut DrawList) {
        let (Some(demo), Some(presenter)) = (self.speed.as_mut(), self.presenter.as_mut()) else {
            return;
        };
        // The arena's fog is tuned for a 30 m ring; over 2 km it hides the city.
        presenter.set_fog_enabled(false);
        match demo.push(presenter, list) {
            Ok(()) => {
                demo.frame(&mut list.camera);
                // Night: the street lamps nearest the camera are the frame's lights.
                if demo.night {
                    let eye: [f32; 3] = list.camera.eye.into();
                    presenter.set_torch_lights(demo.torch_lights(eye));
                }
            }
            Err(why) => {
                presenter.set_fog_enabled(true);
                tell!("arena notice: speed demo dropped: {why}");
                self.speed = None;
            }
        }
    }

    /// A key went down or up.
    pub(crate) fn speed_demo_key(&mut self, code: KeyCode, down: bool) {
        if let Some(demo) = self.speed.as_mut() {
            demo.key(code, down);
        }
    }

    /// A mouse button went down (`pressed`) or up; a press swings.
    pub(crate) fn speed_demo_click(&mut self, pressed: bool) {
        if let Some(demo) = self.speed.as_mut().filter(|_| pressed) {
            demo.click();
        }
    }

    /// The exit line, when the demo ran.
    pub(crate) fn report_speed_demo(&self) {
        if let Some(demo) = self.speed.as_ref() {
            say!("{}", demo.line());
        }
    }
}
