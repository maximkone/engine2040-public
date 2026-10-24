//! engine-live: presentation knobs reloaded from a small dylib while the arena runs (row SW32).
//!
//! The dev builds a tiny crate that exports `e2040_live_tune(_, *mut LiveTune)`;
//! [`Slot`] watches that artifact, waits [`DEBOUNCE_MS`] after the last rewrite,
//! then loads a copy of it and asks it to fill a [`LiveTune`]. A load that fails
//! (half-written file, wrong symbol, a lib that returns a non-finite knob) keeps
//! the last good tune and its library, so a bad edit never blanks the picture.
//!
//! Limits the donor (rksm/hot-lib-reloader-rs `src/lib.rs`) documents and this
//! crate inherits: the reloaded code holds no state between loads, exports no
//! generics and touches no global, and a signature change needs a host restart.
//!
//! Determinism: presentation only. No sim value, tick counter or golden hash
//! depends on a tune, so the same input fixture hashes the same with or without
//! a lib. Each generation is copied to its own file first because Windows locks
//! a loaded DLL, so a rebuild cannot overwrite the one in memory.

#![expect(
    unsafe_code,
    reason = "loading and calling a dev-built tuning dylib is FFI by definition; the two call sites state why the artifact and its repr(C) out-parameter are trusted"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// A rebuilt artifact is loaded only after this much quiet (row: 300 ms).
pub const DEBOUNCE_MS: u64 = 300;
/// The symbol a live lib exports: `unsafe extern "C" fn(*mut LiveTune)`.
pub const SYMBOL: &[u8] = b"e2040_live_tune\0";
/// Subdirectory beside the artifact holding one copy per loaded generation.
const GEN_DIR: &str = "live-gen";

/// The knobs a live lib fills. `#[repr(C)]` so host and lib agree on layout;
/// every knob is a multiplier over the shipped look, 1.0 leaving it alone.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveTune {
    /// Camera boom: eye distance from the target, scaled.
    pub cam_boom: f32,
    /// Hit shake amplitude, scaled.
    pub shake_scale: f32,
    /// Exposure lift on a hit flash, scaled.
    pub flash_lift: f32,
}

impl Default for LiveTune {
    fn default() -> Self {
        Self::SHIPPED
    }
}

impl LiveTune {
    /// The shipped look: every knob 1.0, so the picture is unchanged.
    pub const SHIPPED: Self = Self {
        cam_boom: 1.0,
        shake_scale: 1.0,
        flash_lift: 1.0,
    };

    /// Whether a lib returned numbers the camera and the post chain can use: a
    /// NaN or a collapsed boom would poison every later frame.
    #[must_use]
    pub fn is_sane(&self) -> bool {
        self.cam_boom.is_finite()
            && self.cam_boom > 0.0
            && self.shake_scale.is_finite()
            && self.shake_scale >= 0.0
            && self.flash_lift.is_finite()
            && self.flash_lift >= 0.0
    }

    /// Where the eye stands when the boom knob scales its distance from the
    /// target. The look direction never changes.
    #[must_use]
    pub fn boom_eye(&self, eye: [f32; 3], target: [f32; 3]) -> [f32; 3] {
        [
            target[0] + (eye[0] - target[0]) * self.cam_boom,
            target[1] + (eye[1] - target[1]) * self.cam_boom,
            target[2] + (eye[2] - target[2]) * self.cam_boom,
        ]
    }
}

/// The FFI shape a lib exports.
type Fill = unsafe extern "C" fn(*mut LiveTune);

/// Why a generation was refused; the last good tune stays in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError(String);

impl LoadError {
    fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(err: std::io::Error) -> Self {
        Self::new(err.to_string())
    }
}

impl From<libloading::Error> for LoadError {
    fn from(err: libloading::Error) -> Self {
        Self::new(err.to_string())
    }
}

/// Last write time of the watched artifact, or `None` while it does not exist.
fn mtime_of(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// The watched tuning artifact, its loaded library and the last good tune.
pub struct Slot {
    src: PathBuf,
    gen_dir: PathBuf,
    generation: u32,
    /// Write time of the artifact the current tune came from.
    seen: Option<SystemTime>,
    /// When a rewrite was first noticed; the debounce clock.
    pending: Option<Instant>,
    tune: LiveTune,
    /// Kept alive for as long as its tune is in force; dropping it unloads.
    lib: Option<libloading::Library>,
    reloads: u32,
    last_error: Option<String>,
}

impl Slot {
    /// Watch `src` and try to load it now. A missing or broken artifact leaves
    /// the shipped tune in place; the first good generation replaces it.
    #[must_use]
    pub fn open(src: &Path) -> Self {
        let gen_dir = src.parent().unwrap_or_else(|| Path::new(".")).join(GEN_DIR);
        let mut slot = Self {
            src: src.to_path_buf(),
            gen_dir,
            generation: 0,
            seen: None,
            pending: None,
            tune: LiveTune::SHIPPED,
            lib: None,
            reloads: 0,
            last_error: None,
        };
        slot.seen = mtime_of(&slot.src);
        if slot.seen.is_some() {
            match slot.install() {
                Ok(tune) => {
                    slot.tune = tune;
                    slot.reloads = 1;
                }
                Err(err) => slot.last_error = Some(err.to_string()),
            }
        }
        slot
    }

    /// The tune in force: the last good generation, else [`LiveTune::SHIPPED`].
    #[must_use]
    pub fn tune(&self) -> LiveTune {
        self.tune
    }

    /// Generations loaded since open.
    #[must_use]
    pub fn reloads(&self) -> u32 {
        self.reloads
    }

    /// Why the most recent attempt was refused, taken so one caller logs it once.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// One poll at the wall clock. See [`Self::poll_at`].
    pub fn poll(&mut self) -> Option<LiveTune> {
        self.poll_at(Instant::now())
    }

    /// One poll at `now`: `Some(tune)` when a fresh generation was installed.
    /// A rewrite arms the debounce; the load happens on the first poll at least
    /// [`DEBOUNCE_MS`] after the rewrite was noticed, so a save burst costs one
    /// load. A refused generation records its reason and leaves the tune be.
    pub fn poll_at(&mut self, now: Instant) -> Option<LiveTune> {
        let mtime = mtime_of(&self.src)?;
        if self.seen == Some(mtime) {
            self.pending = None;
            return None;
        }
        let since = *self.pending.get_or_insert(now);
        if now.saturating_duration_since(since) < Duration::from_millis(DEBOUNCE_MS) {
            return None;
        }
        self.pending = None;
        self.seen = Some(mtime);
        match self.install() {
            Ok(tune) => {
                self.tune = tune;
                self.reloads = self.reloads.saturating_add(1);
                self.last_error = None;
                Some(tune)
            }
            Err(err) => {
                self.last_error = Some(err.to_string());
                None
            }
        }
    }

    /// Copy the artifact to a fresh generation file and load that copy, so the
    /// artifact itself is never locked and a rebuild always lands.
    fn install(&mut self) -> Result<LiveTune, LoadError> {
        fs::create_dir_all(&self.gen_dir)?;
        self.generation = self.generation.saturating_add(1);
        let copy = self.gen_dir.join(format!(
            "gen-{}.{}",
            self.generation,
            std::env::consts::DLL_EXTENSION
        ));
        fs::copy(&self.src, &copy)?;
        // SAFETY: the artifact is a library the dev built on this machine from
        // their own source, the same trust as running its tests; nothing here
        // is fetched. Loading runs its initialisers.
        let lib = unsafe { libloading::Library::new(&copy) }?;
        // SAFETY: a live lib is documented to export this symbol with the
        // `Fill` shape; a lib without it is refused below.
        let fill = unsafe { lib.get::<Fill>(SYMBOL) }?;
        let mut tune = LiveTune::SHIPPED;
        // SAFETY: `tune` is a live, aligned `#[repr(C)]` out-parameter and the
        // callee only writes the three floats it is given room for.
        unsafe { fill(&mut tune) };
        if !tune.is_sane() {
            return Err(LoadError::new("a knob is not finite or is negative"));
        }
        self.lib = Some(lib);
        Ok(tune)
    }

    /// One receipt line: the tune in force and what the watching did.
    #[must_use]
    pub fn line(&self) -> String {
        let t = self.tune;
        let last = self.last_error.as_deref().unwrap_or("-");
        format!(
            "cam_boom={:.3} shake_scale={:.3} flash_lift={:.3} reloads={} last={last}",
            t.cam_boom, t.shake_scale, t.flash_lift, self.reloads
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_tune_is_identity_and_sane() {
        let tune = LiveTune::SHIPPED;
        assert!(tune.is_sane());
        assert_eq!(
            tune.boom_eye([0.0, 2.0, 4.0], [0.0, 2.0, 0.0]),
            [0.0, 2.0, 4.0]
        );
    }

    #[test]
    fn boom_scales_the_eye_about_the_target() {
        let tune = LiveTune {
            cam_boom: 0.5,
            ..LiveTune::SHIPPED
        };
        assert_eq!(
            tune.boom_eye([0.0, 2.0, 4.0], [0.0, 2.0, 0.0]),
            [0.0, 2.0, 2.0]
        );
    }

    #[test]
    fn a_nan_or_collapsed_boom_is_refused() {
        assert!(!LiveTune {
            cam_boom: f32::NAN,
            ..LiveTune::SHIPPED
        }
        .is_sane());
        assert!(!LiveTune {
            cam_boom: 0.0,
            ..LiveTune::SHIPPED
        }
        .is_sane());
    }

    #[test]
    fn a_missing_artifact_keeps_the_shipped_tune() {
        let mut slot = Slot::open(Path::new("does-not-exist-e2040.dll"));
        assert_eq!(slot.tune(), LiveTune::SHIPPED);
        assert_eq!(slot.poll_at(Instant::now()), None);
        assert_eq!(slot.reloads(), 0);
    }
}
