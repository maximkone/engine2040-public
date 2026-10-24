//! photo_verb: the door's `photo.shoot` verb — one still an agent can ask for
//! (SW145).
//!
//! An agent that wants to prove "it looked like this" calls the verb with a
//! tick and a pose (`hero`, `overhead`, `blade`, or a free eye/target). The
//! verb parks on that tick, asks the host renderer for one supersampled frame
//! through the same post chain the window uses, writes the PNG plus its JSON
//! sidecar and answers `{path, tick, eye, fov, hash}`. The answer is
//! idempotent per `(tick, pose)`: the second call with the same inputs returns
//! the same path and hash and renders nothing, so a retry never doubles the
//! file.
//!
//! A door with no renderer (a headless session) refuses with
//! [`PhotoError::NoRenderer`] before writing anything, instead of hanging on a
//! frame that will never come. Presentation only: the tick is read, never
//! stepped, and nothing here enters a hash. Units: meters, radians, pixels.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};

/// The verb name, `pack.verb` shaped and equal to the tool's own name.
pub const VERB_PHOTO_SHOOT: &str = "photo.shoot";

/// Supersample factor of one still (the SW141 photo-mode donor's constant).
pub const SUPERSAMPLE: u32 = 2;

/// Largest capture side: 3840x2160 from a 1920x1080 base at 2x.
pub const MAX_CAPTURE_DIM: u32 = 3840;

/// A camera pose the verb can take: a named preset, or the photographer's own
/// eye and target.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PhotoPose {
    /// Three-quarter view from the fighters' left, low and close.
    Hero,
    /// Straight down over the middle of the arena.
    Overhead,
    /// Along the blade line: behind the attacker's shoulder.
    Blade,
    /// The caller's own eye and target in meters (SW141's photo mode).
    Free {
        /// Eye position, meters.
        eye: [f32; 3],
        /// Look point, meters.
        target: [f32; 3],
    },
}

impl PhotoPose {
    /// Parse a pose name; the refusal names the legal values.
    pub fn parse(name: &str) -> Result<Self, PhotoError> {
        match name {
            "hero" => Ok(PhotoPose::Hero),
            "overhead" => Ok(PhotoPose::Overhead),
            "blade" => Ok(PhotoPose::Blade),
            other => Err(PhotoError::BadPose(other.to_owned())),
        }
    }

    /// The camera row of this pose: eye and target in meters.
    #[must_use]
    pub fn eye_target(self) -> ([f32; 3], [f32; 3]) {
        match self {
            PhotoPose::Hero => ([3.2, 1.5, 2.4], [0.0, 1.0, 0.0]),
            PhotoPose::Overhead => ([0.0, 12.0, 0.0], [0.0, 0.0, 0.0]),
            PhotoPose::Blade => ([0.0, 1.7, -3.0], [0.0, 1.2, 0.0]),
            PhotoPose::Free { eye, target } => (eye, target),
        }
    }

    /// The pose's cache and file name part.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PhotoPose::Hero => "hero",
            PhotoPose::Overhead => "overhead",
            PhotoPose::Blade => "blade",
            PhotoPose::Free { .. } => "free",
        }
    }
}

/// One frame a host renderer produced: the PNG bytes, the sim hash it shows
/// and its pixel size (a supersampled size, see [`SUPERSAMPLE`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenderedFrame {
    /// The PNG file body.
    pub bytes: Vec<u8>,
    /// Sim hash at the parked tick.
    pub hash: u64,
    /// Capture width in pixels.
    pub width: u32,
    /// Capture height in pixels.
    pub height: u32,
}

/// Why a host renderer could not produce a frame: one short line, carried to
/// the caller as [`PhotoError::Render`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenderRefusal {
    /// What went wrong, in one line a model can read.
    pub reason: String,
}

/// The host's one-shot renderer: it renders the pose at the parked tick and
/// hands the bytes back. Endpoint never imports render; the callback owns the
/// pixels.
pub type PhotoRenderer<'a> = dyn Fn(&PhotoPose) -> Result<RenderedFrame, RenderRefusal> + 'a;

/// The answer: where the still is, on which tick, from where, and its hash.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhotoShot {
    /// The PNG path that exists on disk.
    pub path: String,
    /// The JSON sidecar beside it.
    pub sidecar: String,
    /// The parked tick.
    pub tick: u32,
    /// The pose name the still was taken from.
    pub pose: String,
    /// Eye position, meters.
    pub eye: [f32; 3],
    /// Look point, meters.
    pub target: [f32; 3],
    /// Vertical field of view, radians (the lens the frame was shot with).
    pub fov_y_rad: f32,
    /// Sim hash at the tick, as the renderer reported it.
    pub hash: u64,
    /// Capture width in pixels.
    pub width: u32,
    /// Capture height in pixels.
    pub height: u32,
}

/// What the caller asks for.
#[derive(Clone, PartialEq, Debug)]
pub struct PhotoRequest {
    /// Pose name (`hero`, `overhead`, `blade`) or `None` with [`Self::free`].
    pub pose: String,
    /// The free eye/target, when the caller frames it themselves.
    pub free: Option<([f32; 3], [f32; 3])>,
    /// The parked tick.
    pub tick: u32,
    /// Focal length in millimeters, `24..=200` (the SW141 lens).
    pub focal_mm: f32,
}

impl Default for PhotoRequest {
    fn default() -> Self {
        Self {
            pose: "hero".to_owned(),
            free: None,
            tick: 0,
            focal_mm: 50.0,
        }
    }
}

/// Failures a shot can hit; each names the value at fault.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PhotoError {
    /// The pose name is not one of the presets.
    BadPose(String),
    /// The focal length is outside `24..=200` mm.
    BadFocalMm,
    /// The call named no tick to park on.
    BadTick,
    /// The renderer refused or handed a bad frame.
    Render(String),
    /// The door has no renderer: a headless session cannot take a still.
    NoRenderer,
    /// The file could not be written.
    Write(String),
}

impl core::fmt::Display for PhotoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PhotoError::BadPose(got) => write!(
                f,
                "photo.shoot: pose `{got}` is not a preset; submit hero, overhead or blade \
                 (or a free eye and target)"
            ),
            PhotoError::BadFocalMm => {
                write!(f, "photo.shoot: focal_mm outside 24..=200; submit 24..=200")
            }
            PhotoError::BadTick => write!(
                f,
                "photo.shoot: no tick was given; submit the parked tick (re-try with `tick`)"
            ),
            PhotoError::Render(why) => {
                write!(
                    f,
                    "photo.shoot: the renderer refused the frame ({why}); re-try the shot"
                )
            }
            PhotoError::NoRenderer => write!(
                f,
                "photo.shoot: this door has no renderer, so no frame can be taken here; \
                 run the arena window or grant a renderer and re-try"
            ),
            PhotoError::Write(why) => {
                write!(
                    f,
                    "photo.shoot: cannot write the still ({why}); re-try with a writable dir"
                )
            }
        }
    }
}

impl std::error::Error for PhotoError {}

/// One session's shots: the idempotency cache keyed by `(tick, pose)`, so a
/// retry answers the same path and hash and renders nothing.
#[derive(Debug, Default)]
pub struct PhotoShots {
    taken: BTreeMap<(u32, String), PhotoShot>,
    renders: u64,
}

impl PhotoShots {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shots this session holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.taken.len()
    }

    /// True when no still was taken.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.taken.is_empty()
    }

    /// Frames the renderer was asked for: the proof that a retry renders
    /// nothing.
    #[must_use]
    pub fn renders(&self) -> u64 {
        self.renders
    }

    /// The still already taken for `(tick, pose)`, if any.
    #[must_use]
    pub fn get(&self, tick: u32, pose: &str) -> Option<&PhotoShot> {
        self.taken.get(&(tick, pose.to_owned()))
    }

    /// Take one still into `dir`: idempotent per `(tick, pose)`.
    ///
    /// A cached ask returns the stored shot and never calls the renderer. A
    /// door without a renderer refuses before any file is written.
    pub fn shoot(
        &mut self,
        request: &PhotoRequest,
        dir: &Path,
        renderer: Option<&PhotoRenderer<'_>>,
    ) -> Result<PhotoShot, PhotoError> {
        let pose = match &request.free {
            Some((eye, target)) => PhotoPose::Free {
                eye: *eye,
                target: *target,
            },
            None => PhotoPose::parse(&request.pose)?,
        };
        if !request.focal_mm.is_finite() || !(24.0..=200.0).contains(&request.focal_mm) {
            return Err(PhotoError::BadFocalMm);
        }
        let key = (request.tick, pose.as_str().to_owned());
        if let Some(shot) = self.taken.get(&key) {
            return Ok(shot.clone());
        }
        let renderer = renderer.ok_or(PhotoError::NoRenderer)?;
        let frame = renderer(&pose).map_err(|refusal| PhotoError::Render(refusal.reason))?;
        if frame.width == 0 || frame.height == 0 || frame.width.max(frame.height) > MAX_CAPTURE_DIM
        {
            return Err(PhotoError::Render(format!(
                "frame {}x{} is outside the capture cap {MAX_CAPTURE_DIM}",
                frame.width, frame.height
            )));
        }
        std::fs::create_dir_all(dir).map_err(|error| PhotoError::Write(error.to_string()))?;
        let name = format!(
            "photo-t{}-{}-{:016x}",
            request.tick,
            pose.as_str(),
            frame.hash
        );
        let path = dir.join(format!("{name}.png"));
        std::fs::write(&path, &frame.bytes)
            .map_err(|error| PhotoError::Write(error.to_string()))?;
        let (eye, target) = pose.eye_target();
        let fov_y_rad = lens_fov_y_rad(request.focal_mm);
        let sidecar = dir.join(format!("{name}.json"));
        let body = format!(
            "{{\"tool\":\"{VERB_PHOTO_SHOOT}\",\"tick\":{},\"pose\":\"{}\",\
             \"eye\":[{:.3},{:.3},{:.3}],\"target\":[{:.3},{:.3},{:.3}],\
             \"fov_y_rad\":{:.5},\"focal_mm\":{:.1},\"supersample\":{SUPERSAMPLE},\
             \"width\":{},\"height\":{},\"hash\":{}}}",
            request.tick,
            pose.as_str(),
            eye[0],
            eye[1],
            eye[2],
            target[0],
            target[1],
            target[2],
            fov_y_rad,
            request.focal_mm,
            frame.width,
            frame.height,
            frame.hash
        );
        std::fs::write(&sidecar, body.as_bytes())
            .map_err(|error| PhotoError::Write(error.to_string()))?;
        let shot = PhotoShot {
            path: path.to_string_lossy().into_owned(),
            sidecar: sidecar.to_string_lossy().into_owned(),
            tick: request.tick,
            pose: pose.as_str().to_owned(),
            eye,
            target,
            fov_y_rad,
            hash: frame.hash,
            width: frame.width,
            height: frame.height,
        };
        self.renders += 1;
        self.taken.insert(key, shot.clone());
        Ok(shot)
    }

    /// The PNG paths this session wrote, in tick and pose order.
    #[must_use]
    pub fn paths(&self) -> Vec<PathBuf> {
        self.taken
            .values()
            .map(|shot| PathBuf::from(&shot.path))
            .collect()
    }
}

/// Vertical field of view of a full-frame lens, radians:
/// `2 * atan(24 mm / (2 * focal))`.
#[must_use]
pub fn lens_fov_y_rad(focal_mm: f32) -> f32 {
    let fov = 2.0 * (24.0 / (2.0 * focal_mm)).atan();
    if fov.is_finite() {
        fov
    } else {
        0.0
    }
}

/// The door's renderer slot: a plain function pointer, so a session can hold
/// one without a closure lifetime. Hosts that render (the arena window)
/// install it; a headless door leaves it `None` and refuses by name.
pub type PhotoRendererFn = fn(&PhotoPose) -> Result<RenderedFrame, RenderRefusal>;

/// Args for `photo.shoot`: a pose name or a free eye and target, the parked
/// tick, the lens and the folder to write into.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhotoShootArgs {
    /// `hero`, `overhead` or `blade`; omitted with a free eye and target.
    pub pose: Option<String>,
    /// The parked sim tick the still shows.
    pub tick: Option<u32>,
    /// Focal length in millimeters, 24..=200 (default 50).
    pub focal_mm: Option<f32>,
    /// Folder to write the PNG and sidecar into; the session's default when
    /// omitted.
    pub dir: Option<String>,
    /// Free eye position, meters (with `target`).
    pub eye: Option<[f32; 3]>,
    /// Free look point, meters (with `eye`).
    pub target: Option<[f32; 3]>,
}

impl PhotoShootArgs {
    /// The typed request, or the refusal naming what is missing.
    pub fn request(&self) -> Result<PhotoRequest, PhotoError> {
        let free = match (self.eye, self.target) {
            (Some(eye), Some(target)) => Some((eye, target)),
            (None, None) => None,
            _ => {
                return Err(PhotoError::BadPose(
                    "eye and target must be given together".to_owned(),
                ))
            }
        };
        Ok(PhotoRequest {
            pose: self.pose.clone().unwrap_or_else(|| "hero".to_owned()),
            free,
            tick: self.tick.ok_or(PhotoError::BadTick)?,
            focal_mm: self.focal_mm.unwrap_or(50.0),
        })
    }

    /// Where the still lands: the folder named, else `photo` under the temp
    /// dir (a door with no project folder still answers with a real path).
    #[must_use]
    pub fn dir_path(&self) -> PathBuf {
        match &self.dir {
            Some(dir) => PathBuf::from(dir),
            None => std::env::temp_dir().join("e2040-photo"),
        }
    }
}

/// The tool error for a photo refusal: a static code plus the recovery line.
#[must_use]
pub fn photo_refusal(error: PhotoError) -> McpToolError {
    let code = match error {
        PhotoError::BadPose(_) => "bad_pose",
        PhotoError::BadFocalMm => "bad_focal",
        PhotoError::BadTick => "bad_tick",
        PhotoError::Render(_) => "render_refused",
        PhotoError::NoRenderer => "no_renderer",
        PhotoError::Write(_) => "write_failed",
    };
    McpToolError::new(code, error.to_string())
}

/// `photo.shoot` handler: the session's shots table takes one still through
/// the installed renderer, idempotent per tick and pose.
pub fn photo_shoot(service: &McpServer, args: &PhotoShootArgs) -> Result<PhotoShot, McpToolError> {
    service.dispatch_screened(VERB_PHOTO_SHOOT, args, |state| state.photo_shoot(args))
}

crate::e2_tool!(
    PhotoShootTool,
    name = "photo.shoot",
    description = "Take one still of the frozen fight: a pose (hero, overhead, blade or a \
                   free eye and target), the parked tick and the lens in; the PNG path, its \
                   JSON sidecar, the tick and the frame hash out. Idempotent per tick and pose, \
                   so a retry never doubles the file, and refused by name on a door with no \
                   renderer.",
    readonly = true,
    args = PhotoShootArgs,
    output = PhotoShot,
    invoke = photo_shoot
);
#[cfg(test)]
#[path = "photo_verb_tests.rs"]
mod tests;
