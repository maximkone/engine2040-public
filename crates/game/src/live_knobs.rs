//! live_knobs: the presentation knobs a rebuilt dylib can change while the arena
//! runs (row SW32), and the one seam the window binary polls.
//!
//! The dev builds a tiny lib exporting `e2040_live_tune` and names it in
//! [`LIB_ENV`]; [`LiveKnobs::from_env`] watches that artifact, and the arena
//! polls it beside the 30-frame content poll. A missing variable, a missing
//! artifact or a refused generation all leave the shipped look in force, so a
//! plain run costs one `var_os` and nothing else.
//!
//! Determinism: presentation only. No tick, sim value or golden hash reads a
//! knob; the camera it feeds is the one the presentation leg already shakes.

use std::path::Path;
use std::time::Instant;

use engine_live::{LiveTune, Slot};
use engine_render::v1::Camera;

/// Environment variable naming the built tuning artifact (a `.dll`/`.so`/`.dylib`).
pub const LIB_ENV: &str = "E2040_LIVE_TUNE";

/// The live tuning seam the window owns for the whole run.
pub struct LiveKnobs {
    slot: Option<Slot>,
}

impl LiveKnobs {
    /// No artifact: every knob shipped, every poll a no-op.
    #[must_use]
    pub fn off() -> Self {
        Self { slot: None }
    }

    /// Watch the artifact named by [`LIB_ENV`]; no variable means [`Self::off`].
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var_os(LIB_ENV) {
            Some(path) => Self::open(Path::new(&path)),
            None => Self::off(),
        }
    }

    /// Watch one artifact path (tests, or a tool that knows the path).
    #[must_use]
    pub fn open(path: &Path) -> Self {
        Self {
            slot: Some(Slot::open(path)),
        }
    }

    /// The tune in force: the last good generation, else the shipped look.
    #[must_use]
    pub fn tune(&self) -> LiveTune {
        self.slot.as_ref().map_or(LiveTune::SHIPPED, Slot::tune)
    }

    /// One poll at the wall clock: `Some(tune)` when a fresh generation loaded.
    pub fn poll(&mut self) -> Option<LiveTune> {
        self.slot.as_mut().and_then(Slot::poll)
    }

    /// One poll at `now`, so a test drives the debounce without sleeping.
    pub fn poll_at(&mut self, now: Instant) -> Option<LiveTune> {
        self.slot.as_mut().and_then(|slot| slot.poll_at(now))
    }

    /// Why the most recent generation was refused, taken: one log line per
    /// refused generation, none on a quiet poll.
    pub fn take_error(&mut self) -> Option<String> {
        self.slot.as_mut().and_then(Slot::take_error)
    }

    /// Dolly the camera about its target by the boom knob; the look direction
    /// and the shake the feel flow adds afterwards both hold.
    /// Dolly the camera about its target by the boom knob; the look direction
    /// holds and the shake the feel flow adds afterwards still rides on top.
    pub fn apply(&self, camera: &mut Camera) {
        let tune = self.tune();
        if tune.cam_boom == 1.0 {
            return;
        }
        let eye = tune.boom_eye(camera.eye.to_array(), camera.target.to_array());
        camera.eye = eye.into();
    }

    /// One receipt line for the reload log.
    #[must_use]
    pub fn line(&self) -> String {
        match &self.slot {
            Some(slot) => slot.line(),
            None => "off".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    /// A lib that fills the boom knob from `BOOM` and leaves the rest shipped.
    const FIXTURE: &str = r#"
#[repr(C)]
pub struct Tune {
    cam_boom: f32,
    shake_scale: f32,
    flash_lift: f32,
}

#[unsafe(no_mangle)]
pub extern "C" fn e2040_live_tune(out: *mut Tune) {
    let tune = Tune { cam_boom: BOOM, shake_scale: 1.0, flash_lift: 1.0 };
    // SAFETY: the host passes a live, aligned Tune for the length of this call.
    unsafe { out.write(tune) };
}
"#;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("e2040-live-{}-{name}", std::process::id()))
    }

    /// Build the fixture at `boom` and return the artifact path.
    fn build(dir: &Path, boom: &str) -> PathBuf {
        fs::create_dir_all(dir).expect("scratch dir");
        let src = dir.join("tune.rs");
        let out = dir.join(format!("tune.{}", std::env::consts::DLL_EXTENSION));
        fs::write(&src, FIXTURE.replace("BOOM", boom)).expect("fixture source");
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        let status = Command::new(rustc)
            .args(["--crate-type", "cdylib", "--edition", "2021", "-O", "-o"])
            .arg(&out)
            .arg(&src)
            .status()
            .expect("rustc runs");
        assert!(status.success(), "the fixture lib did not build");
        // A rewrite must be visible as a new write time on every filesystem.
        sleep(Duration::from_millis(30));
        out
    }

    #[test]
    fn live_reload_observes_a_new_knob_without_a_restart() {
        let dir = scratch("reload");
        let _ = fs::remove_dir_all(&dir);
        let out = build(&dir, "1.0");
        let mut knobs = LiveKnobs::open(&out);
        assert_eq!(knobs.tune().cam_boom, 1.0, "the first build loads at open");
        build(&dir, "2.0");
        let t0 = Instant::now();
        assert!(
            knobs.poll_at(t0).is_none(),
            "the 300 ms debounce must hold the load"
        );
        assert!(
            knobs.poll_at(t0 + Duration::from_millis(120)).is_none(),
            "still inside the debounce"
        );
        assert!(
            knobs.poll_at(t0 + Duration::from_millis(400)).is_some(),
            "one load once the write is quiet"
        );
        assert_eq!(knobs.tune().cam_boom, 2.0, "the rebuilt lib is in force");
        assert_eq!(
            knobs.tune().shake_scale,
            1.0,
            "untouched knobs stay shipped"
        );
        assert!(knobs.line().contains("reloads="), "{}", knobs.line());
    }

    #[test]
    fn a_broken_artifact_keeps_the_last_good_tune() {
        let dir = scratch("broken");
        let _ = fs::remove_dir_all(&dir);
        let out = build(&dir, "1.0");
        let mut knobs = LiveKnobs::open(&out);
        build(&dir, "2.0");
        let t0 = Instant::now();
        assert!(knobs.poll_at(t0).is_none(), "the rewrite arms the debounce");
        assert!(knobs.poll_at(t0 + Duration::from_millis(400)).is_some());
        assert_eq!(knobs.tune().cam_boom, 2.0);
        // A half-written artifact is refused; the picture keeps generation two.
        sleep(Duration::from_millis(30));
        fs::write(&out, b"not a library").expect("write garbage");
        sleep(Duration::from_millis(30));
        let t1 = Instant::now();
        assert!(knobs.poll_at(t1).is_none());
        assert!(knobs.poll_at(t1 + Duration::from_millis(400)).is_none());
        assert_eq!(knobs.tune().cam_boom, 2.0, "last good stays in force");
        assert!(knobs.line().contains("last="), "{}", knobs.line());
    }
}
