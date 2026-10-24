//! scene_desk: the steer half of the scene tools (rows SP62, EG35).
//!
//! The scene report says what the frame holds; this desk lets an agent change
//! how the frame is drawn and look again: pin the camera to a named shot or an
//! explicit pose, switch one render pass, take a capture, freeze the sim on its
//! tick, and undo each of those. The agent speaks to a live window over the
//! agent seat, one line each way: `scene [pixels]` asks for a report, `steer
//! <op> ..` steers, and the window answers `scene <json>` or `steer <json>`.
//! This module owns the pure half: the line grammar, the steer state with its
//! undo stack, the named shots and the reply shapes. The window glue (applying
//! a pass to the presenter, rendering a capture) lives in the arena binary.
//!
//! Grammar, one line, ASCII, fields split on spaces:
//! - `scene` or `scene pixels` (the second also tallies the id buffer);
//! - `steer shot <auto|hero|wide|action|top|side|low|front|sun|close:N>`;
//! - `steer pose <ex> <ey> <ez> <tx> <ty> <tz> [fov_deg]`;
//! - `steer pass <name> <on|off>` (names: `engine_render::present::PASS_NAMES`);
//! - `steer capture [tag]`, `steer hold`, `steer resume`, `steer step`,
//!   `steer state`, `steer undo`.
//!
//! Units: meters, degrees. Presentation only: no steer reaches the sim or its
//! hash; `hold` and `step` pace wall time, never the content of a tick.
//! Determinism: the same state and roots give the same camera. Errors: a line
//! outside the grammar is a typed refusal that names the legal forms.

use std::path::PathBuf;

use engine_render::v1::Camera;

use crate::fighter_look::{shot_name, shot_named, Shot};
use crate::scene_report::emit::{n3, obj, q, v3};
use crate::scene_report::geom::{add, dist, finite, scale, unit, P3};

/// Longest capture tag, characters.
pub const MAX_TAG_CHARS: usize = 32;
/// Where captures land, relative to the working directory.
pub const CAPTURE_DIR: &str = "target/scene-captures";
/// Named camera shots beyond the director's three; `close:N` also takes a slot.
pub const NAMED_SHOTS: [&str; 5] = ["top", "side", "low", "front", "sun"];
/// Largest coordinate or fov a pose may carry.
const POSE_LIMIT: f32 = 1000.0;

/// One request line from the agent.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// A scene report; `pixels` also tallies the renderer's draw-id buffer.
    Scene {
        /// Take the id-buffer pixel counts.
        pixels: bool,
    },
    /// A steer operation.
    Steer(Steer),
}

/// What a `steer shot` selects.
#[derive(Clone, Debug, PartialEq)]
pub enum ShotPick {
    /// Release the pin: the director aims again.
    Auto,
    /// One of the director's three shots.
    Director(Shot),
    /// A named pose computed from where the fighters stand.
    Named(String),
}

/// One steer operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Steer {
    /// Pin or release the camera shot.
    Shot(ShotPick),
    /// Pin the camera to an explicit pose.
    Pose {
        /// Eye in meters.
        eye: P3,
        /// Look point in meters.
        target: P3,
        /// Vertical field of view in degrees; `None` keeps the frame's.
        fov_deg: Option<f32>,
    },
    /// Switch one render pass.
    Pass {
        /// Pass name.
        name: String,
        /// New state.
        on: bool,
    },
    /// Render the frame offscreen to a PNG under [`CAPTURE_DIR`].
    Capture {
        /// File name tag.
        tag: String,
    },
    /// Freeze the sim on its current tick; the picture keeps drawing.
    Hold,
    /// Let the sim run again.
    Resume,
    /// Run exactly one tick, then hold.
    Step,
    /// Read the steer state.
    State,
    /// Undo the last camera, pass or pause change.
    Undo,
}

/// The camera pin.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Pin {
    /// The director aims.
    #[default]
    Auto,
    /// A director shot held fixed.
    Director(Shot),
    /// A named pose.
    Named(String),
    /// An explicit pose.
    Pose {
        /// Eye in meters.
        eye: P3,
        /// Look point in meters.
        target: P3,
        /// Vertical field of view in degrees.
        fov_deg: Option<f32>,
    },
}

/// One reversible change.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// A pass flipped; `was` is the state to restore.
    Pass {
        /// Pass name.
        name: String,
        /// State before the change.
        was: bool,
    },
    /// The camera pin before it changed.
    Pin(Pin),
    /// The pause tick before it changed.
    Pause(Option<u64>),
}

/// The steer state the window carries between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SteerState {
    /// The camera pin.
    pub pin: Pin,
    /// Changes `steer undo` takes back, newest last.
    pub undo: Vec<Undo>,
    /// Captures taken so far.
    pub captures: u32,
}

/// A refused request line, or a scene the window cannot report: a sentence
/// that names the cause and, for a bad line, the legal forms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeskError(String);

impl DeskError {
    /// A refusal with `why` as its sentence.
    #[must_use]
    pub fn new(why: impl Into<String>) -> Self {
        Self(why.into())
    }
}

impl std::fmt::Display for DeskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeskError {}

/// The legal forms, for a refusal.
const FORMS: &str = "scene [pixels] | steer shot <auto|hero|wide|action|top|side|low|front|sun|close:N> | steer pose ex ey ez tx ty tz [fov] | steer pass <name> <on|off> | steer capture [tag] | steer hold|resume|step|state|undo";

fn bad(why: impl std::fmt::Display) -> DeskError {
    DeskError::new(format!("{why}; the forms are: {FORMS}"))
}

/// One finite number inside the pose limit.
fn number(field: Option<&str>, name: &str) -> Result<f32, DeskError> {
    let text = field.ok_or_else(|| bad(format!("{name} is missing")))?;
    let value: f32 = text
        .parse()
        .map_err(|_| bad(format!("{name} `{text}` is not a number")))?;
    if value.is_finite() && value.abs() <= POSE_LIMIT {
        Ok(value)
    } else {
        Err(bad(format!("{name} {value} is outside +-{POSE_LIMIT}")))
    }
}

/// A shot name: a director shot, `auto`, a fixed named pose, or `close:N`.
fn shot_pick(name: &str) -> Result<ShotPick, DeskError> {
    if name == "auto" {
        return Ok(ShotPick::Auto);
    }
    if let Some(shot) = shot_named(name) {
        return Ok(ShotPick::Director(shot));
    }
    let close = name
        .strip_prefix("close:")
        .is_some_and(|n| n.parse::<u8>().is_ok());
    if NAMED_SHOTS.contains(&name) || close {
        return Ok(ShotPick::Named(name.to_owned()));
    }
    Err(bad(format!("no shot `{name}`")))
}

/// A capture tag: letters, digits, `_` and `-`, at most [`MAX_TAG_CHARS`].
fn tag_of(field: Option<&str>) -> Result<String, DeskError> {
    let tag = field.unwrap_or("shot");
    let ok = !tag.is_empty()
        && tag.chars().count() <= MAX_TAG_CHARS
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(tag.to_owned())
    } else {
        Err(bad(format!(
            "capture tag `{tag}` must be 1..={MAX_TAG_CHARS} of letters, digits, _ and -"
        )))
    }
}

/// Parse one request line.
///
/// # Errors
///
/// A refusal naming the field that failed and the legal forms.
pub fn parse_request(line: &str) -> Result<Request, DeskError> {
    let mut parts = line.split_whitespace();
    match parts.next() {
        Some("scene") => match parts.next() {
            None => Ok(Request::Scene { pixels: false }),
            Some("pixels") if parts.next().is_none() => Ok(Request::Scene { pixels: true }),
            Some(other) => Err(bad(format!(
                "scene takes `pixels` or nothing, got `{other}`"
            ))),
        },
        Some("steer") => parse_steer(&mut parts).map(Request::Steer),
        Some(other) => Err(bad(format!("`{other}` is not a request"))),
        None => Err(bad("empty request")),
    }
}

/// The steer half of the grammar.
fn parse_steer<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Result<Steer, DeskError> {
    let op = parts
        .next()
        .ok_or_else(|| bad("steer needs an operation"))?;
    let steer = match op {
        "shot" => Steer::Shot(shot_pick(
            parts.next().ok_or_else(|| bad("shot needs a name"))?,
        )?),
        "pose" => {
            let n: Vec<f32> = ["ex", "ey", "ez", "tx", "ty", "tz"]
                .iter()
                .map(|name| number(parts.next(), name))
                .collect::<Result<_, _>>()?;
            let fov_deg = parts.next().map(|t| number(Some(t), "fov")).transpose()?;
            if fov_deg.is_some_and(|f| !(1.0..=170.0).contains(&f)) {
                return Err(bad("fov must be 1..=170 degrees"));
            }
            Steer::Pose {
                eye: [n[0], n[1], n[2]],
                target: [n[3], n[4], n[5]],
                fov_deg,
            }
        }
        "pass" => {
            let name = parts.next().ok_or_else(|| bad("pass needs a name"))?;
            let on = match parts.next() {
                Some("on") => true,
                Some("off") => false,
                other => return Err(bad(format!("pass needs on or off, got {other:?}"))),
            };
            Steer::Pass {
                name: name.to_owned(),
                on,
            }
        }
        "capture" => Steer::Capture {
            tag: tag_of(parts.next())?,
        },
        "hold" => Steer::Hold,
        "resume" => Steer::Resume,
        "step" => Steer::Step,
        "state" => Steer::State,
        "undo" => Steer::Undo,
        other => return Err(bad(format!("no steer operation `{other}`"))),
    };
    if parts.next().is_some() {
        return Err(bad("extra fields after the operation"));
    }
    Ok(steer)
}

/// The capture file for `tag` at `tick`, the `n`th of the run.
#[must_use]
pub fn capture_path(tag: &str, tick: u32, n: u32) -> PathBuf {
    PathBuf::from(CAPTURE_DIR).join(format!("{tag}-t{tick}-{n}.png"))
}

/// A camera from an eye and a look point on the frame's lens, clip and aspect.
fn camera_at(frame: &Camera, eye: P3, target: P3, up: P3, fov_deg: Option<f32>) -> Option<Camera> {
    if !(finite(eye) && finite(target)) || dist(eye, target) < 0.05 {
        return None;
    }
    let fov = fov_deg.map_or(frame.fov_y, f32::to_radians);
    let mut camera = Camera::new(eye, target, up, fov);
    camera.aspect = frame.aspect;
    camera.near = frame.near;
    camera.far = frame.far;
    camera.validate().ok().map(|()| camera)
}

/// The camera of a named shot, from the lens of `frame`, the roots of the
/// fighters (slot, feet in meters) and the direction toward the sun. `None`
/// for an unknown name, a slot nobody holds or an unusable pose.
#[must_use]
pub fn named_camera(name: &str, frame: &Camera, roots: &[(u8, P3)], sun: P3) -> Option<Camera> {
    let n = f32::from(u8::try_from(roots.len()).unwrap_or(u8::MAX)).max(1.0);
    let sum = roots.iter().fold([0.0; 3], |acc, (_, r)| add(acc, *r));
    let centre = if roots.is_empty() {
        [0.0; 3]
    } else {
        scale(sum, 1.0 / n)
    };
    let reach = roots
        .iter()
        .map(|(_, r)| dist([r[0], 0.0, r[2]], [centre[0], 0.0, centre[2]]))
        .fold(1.5f32, f32::max);
    let look = add(centre, [0.0, 1.0, 0.0]);
    let up = [0.0, 1.0, 0.0];
    match name {
        "top" => camera_at(
            frame,
            add(centre, [0.0, 8.0 + 2.0 * reach, 0.0]),
            centre,
            [0.0, 0.0, -1.0],
            None,
        ),
        "side" => camera_at(
            frame,
            add(centre, [10.0 + 2.0 * reach, 1.6, 0.0]),
            look,
            up,
            None,
        ),
        "front" => camera_at(
            frame,
            add(centre, [0.0, 1.7, 9.0 + 2.0 * reach]),
            look,
            up,
            None,
        ),
        "low" => camera_at(
            frame,
            add(centre, [-(2.0 + reach), 0.35, 3.5 + reach]),
            look,
            up,
            None,
        ),
        "sun" => {
            let toward = unit(sun)?;
            let eye = add(look, scale(toward, 25.0 + 2.0 * reach));
            let up = if toward[1].abs() > 0.95 {
                [1.0, 0.0, 0.0]
            } else {
                up
            };
            camera_at(frame, eye, look, up, None)
        }
        close => {
            let slot: u8 = close.strip_prefix("close:")?.parse().ok()?;
            let root = roots.iter().find(|(s, _)| *s == slot)?.1;
            camera_at(
                frame,
                add(root, [1.8, 1.5, 1.8]),
                add(root, [0.0, 1.0, 0.0]),
                up,
                None,
            )
        }
    }
}

impl SteerState {
    /// The director shot to force, when the pin holds one.
    #[must_use]
    pub fn director_shot(&self) -> Option<Shot> {
        match self.pin {
            Pin::Director(shot) => Some(shot),
            _ => None,
        }
    }

    /// The camera that replaces the frame's, when the pin holds a pose or a
    /// named shot the fighters' places and the sun can resolve.
    #[must_use]
    pub fn camera(&self, frame: &Camera, roots: &[(u8, P3)], sun: P3) -> Option<Camera> {
        match &self.pin {
            Pin::Auto | Pin::Director(_) => None,
            Pin::Named(name) => named_camera(name, frame, roots, sun),
            Pin::Pose {
                eye,
                target,
                fov_deg,
            } => camera_at(frame, *eye, *target, [0.0, 1.0, 0.0], *fov_deg),
        }
    }

    /// The pin as a wire name: `auto`, a shot name or `pose`.
    #[must_use]
    pub fn pin_name(&self) -> String {
        match &self.pin {
            Pin::Auto => "auto".to_owned(),
            Pin::Director(shot) => shot_name(*shot).to_owned(),
            Pin::Named(name) => name.clone(),
            Pin::Pose { .. } => "pose".to_owned(),
        }
    }

    /// Apply a camera pick and record the pin it replaced.
    pub fn pick(&mut self, pick: &ShotPick) {
        let next = match pick {
            ShotPick::Auto => Pin::Auto,
            ShotPick::Director(shot) => Pin::Director(*shot),
            ShotPick::Named(name) => Pin::Named(name.clone()),
        };
        self.set_pin(next);
    }

    /// Pin an explicit pose and record the pin it replaced.
    pub fn pose(&mut self, eye: P3, target: P3, fov_deg: Option<f32>) {
        self.set_pin(Pin::Pose {
            eye,
            target,
            fov_deg,
        });
    }

    fn set_pin(&mut self, next: Pin) {
        let was = std::mem::replace(&mut self.pin, next);
        self.undo.push(Undo::Pin(was));
    }

    /// Take back the newest change; `None` when there is nothing to undo.
    /// A pin is restored here; a pass or pause is returned for the caller,
    /// which owns the presenter and the pause tick.
    pub fn undo(&mut self) -> Option<Undo> {
        let last = self.undo.pop()?;
        if let Undo::Pin(pin) = &last {
            self.pin = pin.clone();
        }
        Some(last)
    }
}

/// An `ok` reply: `{"ok":true,"op":..,<fields>}`.
#[must_use]
pub fn reply_ok(op: &str, tick: u32, fields: &[(&str, String)]) -> String {
    let mut pairs: Vec<(&str, String)> = vec![
        ("ok", "true".to_owned()),
        ("op", q(op)),
        ("tick", tick.to_string()),
    ];
    pairs.extend(fields.iter().map(|(k, v)| (*k, v.clone())));
    obj(&pairs)
}

/// A refusal reply: `{"ok":false,"op":..,"code":..,"why":..}`.
#[must_use]
pub fn reply_err(op: &str, code: &str, why: &str) -> String {
    obj(&[
        ("ok", "false".to_owned()),
        ("op", q(op)),
        ("code", q(code)),
        ("why", q(why)),
    ])
}

/// The camera as a reply field.
#[must_use]
pub fn camera_field(camera: &Camera) -> String {
    obj(&[
        ("eye", v3(camera.eye.to_array())),
        ("target", v3(camera.target.to_array())),
        ("fov_deg", n3(camera.fov_y.to_degrees())),
    ])
}

#[cfg(test)]
mod tests;
