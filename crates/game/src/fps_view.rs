//! fps_view: first-person eye pose, presentation side.
//! Status: the legionary mode's eye feed (the arena bin calls it) plus the
//! helmet the first-person view wears: the aperture, the rim's own depth
//! range and the lens the view takes.
//! Owner: engine-game. Units: mm in, m out for the camera feed; yaw and pitch in rad.
//! Determinism: presentation only, never hashed; same deltas give same bytes twice.
use crate::camera::camera_rig::{smooth_trauma_offset, trauma_gain};
use crate::camera::CameraError;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
/// The pad rumbles on a hit: the mapping from this eye's trauma to a rumble, and the gilrs call.
#[path = "pad_rumble.rs"]
pub mod pad_rumble;
/// The first person played well (row POV2 and the look audit of 2026-10-24): the eye on the
/// head's real eye point riding a halflife spring, the aim ray that stays put, a spawn that
/// looks into the arena, the reticle and the view laws.
#[path = "fps_pov.rs"]
pub mod fps_pov;
/// The short name the window and the view laws use: `fps_view::pov`.
pub use self::fps_pov as pov;
/// Mouse sensitivity.
pub const SENSITIVITY: f32 = 0.001;
/// Pitch pad from the pole.
pub const ANGLE_EPSILON: f32 = 0.001953125;
/// Eye height over feet, mm.
pub const EYE_HEIGHT_MM: i32 = 1600;
/// Interp scale: permille 0..=1000 maps to alpha 0..=1.
pub const INTERP_SCALE: u32 = 1000;
/// Eye shake kick cap, meters: the table's `near_clip_mm` 50 row (`docs/plan/fps-view.md:130`).
pub const EYE_SHAKE_MAX_M: f32 = 0.05;
/// Eye roll cap, radians: the roll never passes a tenth of a radian.
pub const EYE_ROLL_MAX_RAD: f32 = 0.10;
/// FOV punch cap, radians: the table's `fov_punch_permille` 60 row (`docs/plan/fps-view.md:131`).
pub const FOV_PUNCH_MAX_RAD: f32 = 0.060;
/// Unity gain in permille for the shake dial `gain_q` and the punch's gain scale.
pub const EYE_SHAKE_GAIN_Q: u16 = 1000;
/// Look state: yaw and pitch radians.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookState {
    pub yaw_rad: f32,
    pub pitch_rad: f32,
}
impl LookState {
    /// Fresh look facing forward.
    pub fn neutral() -> Self {
        Self {
            yaw_rad: 0.0,
            pitch_rad: 0.0,
        }
    }
    /// Fold one mouse delta into yaw and pitch; pitch clamps, yaw wraps to pi range.
    pub fn apply_mouse(&mut self, dx_px: f32, dy_px: f32) {
        if !dx_px.is_finite() || !dy_px.is_finite() {
            return;
        }
        let limit = FRAC_PI_2 - ANGLE_EPSILON;
        self.pitch_rad = (self.pitch_rad - dy_px * SENSITIVITY).clamp(-limit, limit);
        self.yaw_rad -= dx_px * SENSITIVITY;
        if self.yaw_rad.abs() > PI {
            self.yaw_rad = self.yaw_rad.rem_euclid(TAU);
            if self.yaw_rad > PI {
                self.yaw_rad -= TAU;
            }
        }
    }
}
/// Logical frame: current plus previous feet mm with look.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LogicalFrame {
    pub pos_mm: [i32; 3],
    pub prev_mm: [i32; 3],
    pub look: LookState,
}
/// The helmet the first-person view wears: the aperture the wearer looks
/// through, the depth range its rim draws in and the lens the view takes.
///
/// The rim never leans on the world's clip. The world's near plane sits
/// inside the rim's range, so world geometry can stand right in front of
/// the eye; the rim draws in its own range on its own depth target, so
/// nothing the world draws can cover it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HelmetWorn {
    /// The aperture the wearer has.
    pub preset: engine_render::viewmodel::AperturePreset,
    /// The depth range the rim draws in, in meters.
    pub depth: engine_render::viewmodel::DepthRange,
    /// The lens the first-person view takes.
    pub lens: engine_render::lens::LensPreset,
}

impl HelmetWorn {
    /// The helmet of the legionary kit: the galea with cheek guards, the
    /// rim's own depth range and the helmet lens.
    pub const fn galea() -> Self {
        Self {
            preset: engine_render::viewmodel::AperturePreset::GaleaCheekGuards,
            depth: engine_render::viewmodel::VIEWMODEL_DEPTH,
            lens: engine_render::lens::LensPreset::Helmet,
        }
    }
}
/// Render eye: interpolated feet plus height, with look attached and the
/// trauma roll (`0` when calm). Millimetre lanes are `f32`, the feed's own
/// unit: whole millimetres land exact (`i16` covers every arena span) and a
/// sub-millimetre shake kick keeps seven digits, so the camera feed narrows
/// nothing and no lane needs a numeric `as` cast.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RenderEye {
    pub eye_mm: [f32; 3],
    pub yaw_rad: f32,
    pub pitch_rad: f32,
    /// Roll about the view axis from the shake, always `0` in [`render_eye`].
    pub roll_rad: f32,
    /// The helmet the wearer has: the rim the view draws, with the depth
    /// range it draws in.
    pub helmet: HelmetWorn,
}
/// Whole millimetres as the lane's own type: `i16` holds 32.7 m, wider than any
/// arena span, and the `f32` step is `From`, so a lane narrows without a
/// numeric `as` cast.
fn mm_lane(mm: i32) -> f32 {
    f32::from(i16::try_from(mm).unwrap_or(i16::MAX))
}
/// [`INTERP_SCALE`] as the lane's own type; the test below keeps the one number
/// in step with the public `u32`.
const INTERP_SCALE_Q: u16 = 1000;
fn lerp_lane(a: i32, b: i32, alpha_q: u32) -> f32 {
    let t = f32::from(u16::try_from(alpha_q.min(INTERP_SCALE)).unwrap_or(INTERP_SCALE_Q))
        / f32::from(INTERP_SCALE_Q);
    let (a_f, b_f) = (mm_lane(a), mm_lane(b));
    a_f + (b_f - a_f) * t
}
/// Build the render eye at overstep alpha permille; adds height over interpolated feet.
pub fn render_eye(frame: &LogicalFrame, alpha_q: u32, height_mm: i32) -> RenderEye {
    RenderEye {
        eye_mm: [
            lerp_lane(frame.prev_mm[0], frame.pos_mm[0], alpha_q),
            lerp_lane(frame.prev_mm[1], frame.pos_mm[1], alpha_q) + mm_lane(height_mm),
            lerp_lane(frame.prev_mm[2], frame.pos_mm[2], alpha_q),
        ],
        yaw_rad: frame.look.yaw_rad,
        pitch_rad: frame.look.pitch_rad,
        roll_rad: 0.0,
        helmet: HelmetWorn::galea(),
    }
}
/// Eye-space shake for one tick (kick mm, roll and FOV punch rad): presentation only.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EyeShake {
    /// Kick in mm about the eye, eye-space axes: `x` right, `y` up, `z` forward.
    pub kick_mm: [f32; 3],
    /// Roll about the view axis, at or under [`EYE_ROLL_MAX_RAD`].
    pub roll_rad: f32,
    /// FOV punch, at or under [`FOV_PUNCH_MAX_RAD`].
    pub fov_punch_rad: f32,
}
impl EyeShake {
    /// Calm: every channel exactly zero, so a caller never special-cases it.
    #[must_use]
    pub fn none() -> Self {
        Self {
            kick_mm: [0.0, 0.0, 0.0],
            roll_rad: 0.0,
            fov_punch_rad: 0.0,
        }
    }
}
/// Shake gain dial in `0..=1` from `gain_q` permille, clamped at unity.
fn eye_shake_dial(gain_q: u32) -> f32 {
    let dial_q = u16::try_from(gain_q.min(u32::from(EYE_SHAKE_GAIN_Q))).unwrap_or(EYE_SHAKE_GAIN_Q);
    f32::from(dial_q) / f32::from(EYE_SHAKE_GAIN_Q)
}
/// Eye-space shake from one trauma value at `tick` through the shared trauma
/// curve ([`smooth_trauma_offset`], no fork): pure integer inputs, no wall
/// clock, same triple same bytes twice. `gain_q` is the caller's dial in
/// permille (1000 unity, higher clamps); trauma `0` is [`EyeShake::none`].
#[must_use]
pub fn eye_shake_for_trauma(trauma_permille: u32, tick: u32, gain_q: u32) -> EyeShake {
    if trauma_permille == 0 {
        return EyeShake::none();
    }
    // Phase sits on the integer tick, so a replayed tick gives the same bytes.
    let Ok(shake) = smooth_trauma_offset(trauma_permille, tick, 0.0, EYE_SHAKE_MAX_M) else {
        // Constant arguments, always in range: unreachable; calm keeps caps honest.
        return EyeShake::none();
    };
    // The curve carries the trauma gain; the dial scales kick, roll and punch.
    let dial = eye_shake_dial(gain_q);
    let gain = trauma_gain(trauma_permille) * dial;
    let mm = dial * 1000.0;
    EyeShake {
        kick_mm: [
            shake.offset_m.x * mm,
            shake.offset_m.y * mm,
            shake.offset_m.z * mm,
        ],
        roll_rad: (gain * EYE_ROLL_MAX_RAD).min(EYE_ROLL_MAX_RAD),
        fov_punch_rad: (gain * FOV_PUNCH_MAX_RAD).min(FOV_PUNCH_MAX_RAD),
    }
}
/// Rotate an eye-space kick (mm: `x` right, `y` up, `z` forward) into world
/// axes for a view yawed and pitched by the frame; right-handed, `+y` up, yaw
/// `0` facing `+z` (the rig's boom axis, `camera_rig.rs:716`).
fn eye_space_kick_mm(kick: [f32; 3], yaw_rad: f32, pitch_rad: f32) -> [f32; 3] {
    let (sy, cy) = yaw_rad.sin_cos();
    let (sp, cp) = pitch_rad.sin_cos();
    let (kx, ky, kz) = (kick[0], kick[1], kick[2]);
    // right = (cy, 0, -sy), up = (-sy sp, cp, -cy sp), forward = (sy cp, sp, cy cp)
    [
        cy * kx - (sy * sp) * ky + (sy * cp) * kz,
        cp * ky + sp * kz,
        -sy * kx - (cy * sp) * ky + (cy * cp) * kz,
    ]
}
/// Render eye with the eye-space trauma kick folded in: the pose from
/// [`render_eye`] plus the shake's kick (rotated into world by the view) and
/// roll. Trauma `0` leaves every byte exactly as [`render_eye`] gives it.
#[must_use]
pub fn render_eye_shaken(
    frame: &LogicalFrame,
    alpha_q: u32,
    height_mm: i32,
    trauma_permille: u32,
    tick: u32,
    gain_q: u32,
) -> RenderEye {
    let mut eye = render_eye(frame, alpha_q, height_mm);
    if trauma_permille == 0 {
        return eye;
    }
    let shake = eye_shake_for_trauma(trauma_permille, tick, gain_q);
    let kick = eye_space_kick_mm(shake.kick_mm, eye.yaw_rad, eye.pitch_rad);
    eye.eye_mm[0] += kick[0];
    eye.eye_mm[1] += kick[1];
    eye.eye_mm[2] += kick[2];
    eye.roll_rad = shake.roll_rad;
    eye
}
/// Eye triple in meters for the render camera feed: the lanes are already the
/// feed's `f32` millimetres, so the divide is one step and needs no narrowing.
pub fn eye_m(eye: &RenderEye) -> [f32; 3] {
    [
        eye.eye_mm[0] / 1000.0,
        eye.eye_mm[1] / 1000.0,
        eye.eye_mm[2] / 1000.0,
    ]
}
/// First-person eye over one fighter body: eye sits `height_mm` over `pos_m` with yaw from facing.
/// Units: `pos_m` meters, `height_mm` mm in 1..=2500, yaw rad; pitch is 0.
/// Errors: `BadParam` naming `FV1` plus the bad field for non-finite input or bad height.
/// Determinism: presentation only, never hashed; same inputs give same bytes twice; zero heap.
pub fn eye_for_fighter(
    pos_m: [f32; 3],
    facing_yaw_rad: f32,
    height_mm: i32,
) -> Result<RenderEye, CameraError> {
    if !pos_m[0].is_finite() || !pos_m[1].is_finite() || !pos_m[2].is_finite() {
        return Err(CameraError::BadParam("FV1 pos_m"));
    }
    if !facing_yaw_rad.is_finite() {
        return Err(CameraError::BadParam("FV1 facing_yaw_rad"));
    }
    if !(1..=2500).contains(&height_mm) {
        return Err(CameraError::BadParam("FV1 height_mm"));
    }
    Ok(RenderEye {
        eye_mm: [
            pos_m[0] * 1000.0,
            pos_m[1] * 1000.0 + mm_lane(height_mm),
            pos_m[2] * 1000.0,
        ],
        yaw_rad: facing_yaw_rad,
        pitch_rad: 0.0,
        roll_rad: 0.0,
        helmet: HelmetWorn::galea(),
    })
}
/// Map a render-camera refusal onto the game camera error: the eye feed adds
/// no error type, it names the field the render camera rejected.
#[cfg(feature = "app")]
fn camera_feed_refused(refused: engine_render::v1::CameraError) -> CameraError {
    use engine_render::v1::CameraError as RenderRefusal;
    match refused {
        RenderRefusal::BadFov(_) => CameraError::BadParam("FV3 fov_y_rad"),
        RenderRefusal::BadAspect(_) => CameraError::BadParam("FV3 aspect"),
        RenderRefusal::BadUp => CameraError::BadParam("FV3 up"),
        RenderRefusal::BadClip { .. } => CameraError::BadParam("FV3 clip"),
        RenderRefusal::NonFinite(_) | RenderRefusal::DegenerateView => {
            CameraError::BadParam("FV3 eye_mm")
        }
    }
}
/// Camera feed from the fighter eye: the eye becomes a validated
/// `engine_render::v1::Camera` for the submit path.
/// Units: eye mm in, meters out; yaw, pitch, roll and `fov_y_rad` in rad,
/// `aspect` width over height.
/// Frame: right-handed, `+y` up, yaw `0` faces `+z` (the rig's boom axis); the
/// target sits one meter along the eye's forward, and `roll_rad` turns the up
/// axis about that forward, so the roll never moves the eye or the target.
/// Errors: [`CameraError::BadParam`] naming `FV3` plus the bad field for a
/// non-finite eye, yaw, pitch or roll, `fov_y_rad` outside `(0, pi)` or
/// `aspect` outside `[1e-3, 1e4]`; a camera the render camera itself refuses
/// (a view straight along the world up) keeps its field name. Never clamps
/// silently. Determinism: presentation only, never hashed; identical eyes give
/// bit-identical view-proj matrices twice; O(1) time, zero heap; the consumer
/// is the windowed leg, so the feed rides the `app` feature.
#[cfg(feature = "app")]
pub fn camera_for_eye(
    eye: &RenderEye,
    fov_y_rad: f32,
    aspect: f32,
) -> Result<engine_render::v1::Camera, CameraError> {
    if !eye.eye_mm[0].is_finite() || !eye.eye_mm[1].is_finite() || !eye.eye_mm[2].is_finite() {
        return Err(CameraError::BadParam("FV3 eye_mm"));
    }
    for (field, value) in [
        ("FV3 yaw_rad", eye.yaw_rad),
        ("FV3 pitch_rad", eye.pitch_rad),
        ("FV3 roll_rad", eye.roll_rad),
    ] {
        if !value.is_finite() {
            return Err(CameraError::BadParam(field));
        }
    }
    if !fov_y_rad.is_finite() || !(0.0..PI).contains(&fov_y_rad) {
        return Err(CameraError::BadParam("FV3 fov_y_rad"));
    }
    if !aspect.is_finite() || !(1e-3..=1e4).contains(&aspect) {
        return Err(CameraError::BadParam("FV3 aspect"));
    }
    // The lanes are already the feed's millimetres (`RenderEye::eye_mm`), so the
    // metres are one divide, no narrowing cast; a lane past `f32` range still
    // reaches the render camera's own finite check, which refuses it by name.
    let position = eye_m(eye);
    let (sy, cy) = eye.yaw_rad.sin_cos();
    let (sp, cp) = eye.pitch_rad.sin_cos();
    // Forward from yaw and pitch: yaw 0 level faces +z, and the triple is unit
    // length, so the target stays exactly one meter from the eye.
    let forward = [sy * cp, sp, cy * cp];
    let target = [
        position[0] + forward[0],
        position[1] + forward[1],
        position[2] + forward[2],
    ];
    // Rodrigues over the world up about the forward axis: the roll turns the
    // up axis only. Roll 0 is exact, `sin_cos(0.0)` giving (0.0, 1.0).
    let (sr, cr) = eye.roll_rad.sin_cos();
    let along_up = forward[1];
    let up = [
        -forward[2] * sr + forward[0] * along_up * (1.0 - cr),
        cr + forward[1] * along_up * (1.0 - cr),
        forward[0] * sr + forward[2] * along_up * (1.0 - cr),
    ];
    let mut camera = engine_render::v1::Camera::new(position, target, up, fov_y_rad);
    camera.aspect = aspect;
    camera.validate().map_err(camera_feed_refused)?;
    Ok(camera)
}

/// The view the legionary mode plays: the eye of the fighter body (first
/// person) or the fight camera (third person, the replay view).
///
/// Presentation only: the view never enters the simulation, the tape or any
/// hash, so a toggle cannot move a match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LegionaryView {
    /// First person: the camera sits at the fighter's eye.
    #[default]
    Eye,
    /// Third person: the fight camera frames the bout (the replay view).
    Third,
}

impl LegionaryView {
    /// The name the mode line prints: `fps` or `third`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Eye => "fps",
            Self::Third => "third",
        }
    }

    /// The other view: the toggle between the eye and the fight camera.
    /// Presentation only; the simulation never sees it.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Eye => Self::Third,
            Self::Third => Self::Eye,
        }
    }
}

/// The shipped weapons table (`content/design/weapons.rows`): the design
/// data the kit's sword rides. Embedded at build time, so the tick never
/// reads a file and every load sees the same bytes.
const WEAPONS_ROWS: &str = include_str!("../../../content/design/weapons.rows");

/// One row of the weapons table by its id (`gladius`), or `None` when the
/// table has no such row. The id is the piece name, so the kit's sword and
/// the design row can never drift apart without the mode saying so.
fn weapons_row<'a>(text: &'a str, id: &str) -> Option<&'a str> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with(id) && line[id.len()..].starts_with(' '))
}

/// The kit one legionary wears: the galea on the head, the gladius in the
/// hand, the scutum on the arm.
///
/// The names are the shipped ones. The numbers they stand for ride the
/// engine's own rows (the anatomy row's armor bits, the weapons table's
/// gladius row), so the mode never invents a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegionaryKit {
    /// The helmet on the head: `galea`.
    pub helmet: &'static str,
    /// The sword in the hand: `gladius`.
    pub weapon: &'static str,
    /// The shield on the arm: `scutum`.
    pub shield: &'static str,
    /// The armor bits this kit covers in the shipped anatomy row: the galea
    /// on the head and the scutum on the shield arm. The bits are the combat
    /// crate's own constants, never a local copy.
    pub worn: u8,
}

impl LegionaryKit {
    /// The kit of a legionary of the line: galea, gladius, scutum.
    pub const LEGIONARY: Self = Self {
        helmet: "galea",
        weapon: "gladius",
        shield: "scutum",
        worn: engine_combat::ARMOR_GALEA | engine_combat::ARMOR_SCUTUM,
    };

    /// The three pieces as one line: `galea,gladius,scutum`.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{},{},{}", self.helmet, self.weapon, self.shield)
    }

    /// True when the shipped data still backs the kit: the weapons table
    /// still has the row the sword rides, and the armor bits are the
    /// combat crate's own. The bits name what the kit covers in the
    /// shipped anatomy row; the row names the sword's own numbers.
    #[must_use]
    pub fn backed_by_shipped_data(&self) -> bool {
        weapons_row(WEAPONS_ROWS, self.weapon).is_some()
            && self.worn & !engine_combat::ARMOR_ALL == 0
    }
}

/// Trauma the player's own eye takes off a 60 Hz tick, per tick, in permille: a full jolt
/// (1000) fades out in 50 ticks, a hit (180) in 9.
pub const EYE_TRAUMA_DECAY_PER_TICK: u32 = 20;

/// The first-person eye's trauma: one level that rises with the blows the player is in and
/// fades smoothly between them.
///
/// The eye used to take `trauma_for_cues(this tick's cues)`: every cue in the arena, rivals'
/// hits on each other included, for that one tick only. In a crowded fight nearly every tick
/// carried a cue, so the view snapped every tick and each snap vanished the next ("camera
/// shake as fuck from my pov", owner 2026-10-21). Now a blow the player takes counts in full,
/// a blow the player lands counts a third (the jolt of connecting), anything between other
/// fighters counts nothing, and the level decays [`EYE_TRAUMA_DECAY_PER_TICK`] per tick, so
/// the shake fades instead of popping. Feed it once per frame; a repeated tick is a no-op.
/// Presentation only, never hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EyeTrauma {
    permille: u32,
    tick: Option<u32>,
}

impl EyeTrauma {
    /// Advance to `tick`: decay by the ticks since the last feed, then add the trauma of the
    /// cues `slot` is in. Returns the level to hand [`legionary_eye`].
    pub fn feed(&mut self, tick: u32, cues: &[engine_audio::FightCue], slot: u8) -> u32 {
        if self.tick == Some(tick) {
            return self.permille;
        }
        let elapsed = self
            .tick
            .map_or(1, |last| tick.wrapping_sub(last).clamp(1, 1_000));
        self.permille = self
            .permille
            .saturating_sub(EYE_TRAUMA_DECAY_PER_TICK.saturating_mul(elapsed));
        for cue in cues {
            self.permille = self.permille.saturating_add(own_trauma(*cue, slot));
        }
        self.permille = self.permille.min(crate::camera::TRAUMA_SCALE);
        self.tick = Some(tick);
        self.permille
    }

    /// The current level, permille.
    #[must_use]
    pub fn permille(&self) -> u32 {
        self.permille
    }
}

/// The trauma one cue gives the eye of `slot`: taken in full, dealt at a third, otherwise none.
fn own_trauma(cue: engine_audio::FightCue, slot: u8) -> u32 {
    use engine_audio::FightCue as C;
    let full = crate::camera_feed::trauma_for_cue(cue);
    let (attacker, victim) = match cue {
        C::Hit {
            attacker, victim, ..
        }
        | C::Blocked {
            attacker, victim, ..
        }
        | C::Parried { attacker, victim }
        | C::Dodged { attacker, victim }
        | C::Knockdown { attacker, victim } => (Some(attacker), Some(victim)),
        C::StaminaOut { .. } => (None, None),
    };
    if victim == Some(slot) {
        full
    } else if attacker == Some(slot) {
        full / 3
    } else {
        0
    }
}

/// The first-person eye of one fighter body with the eye-space trauma kick
/// folded in: the shake kicks about the eye instead of moving the world.
///
/// Units: `pos_m` meters over the feet, `facing_yaw_rad` radians,
/// `trauma_permille` permille (`0` is calm), `tick` the fixed tick.
/// Errors: [`CameraError::BadParam`] naming `FV1` for a non-finite position
/// or facing (the same refusal the eye pose gives).
/// Determinism: presentation only, never hashed; the same inputs give the
/// same bytes twice; zero heap.
pub fn legionary_eye(
    pos_m: [f32; 3],
    facing_yaw_rad: f32,
    trauma_permille: u32,
    tick: u32,
) -> Result<RenderEye, CameraError> {
    let mut eye = eye_for_fighter(pos_m, facing_yaw_rad, EYE_HEIGHT_MM)?;
    if trauma_permille == 0 {
        return Ok(eye);
    }
    let shake = eye_shake_for_trauma(trauma_permille, tick, u32::from(EYE_SHAKE_GAIN_Q));
    let kick = eye_space_kick_mm(shake.kick_mm, eye.yaw_rad, eye.pitch_rad);
    eye.eye_mm[0] += kick[0];
    eye.eye_mm[1] += kick[1];
    eye.eye_mm[2] += kick[2];
    eye.roll_rad = shake.roll_rad;
    Ok(eye)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::camera_rig::trauma_gain;
    use crate::camera::TRAUMA_SCALE;
    use crate::camera_feed::{trauma_for_cue, trauma_for_cues};
    use engine_audio::FightCue;
    #[test]
    fn eye_trauma_ignores_rivals_and_fades_without_popping() {
        let me = 0u8;
        let mut eye = EyeTrauma::default();
        // Two rivals trading blows next to the player: the player's eye stays calm.
        let rivals = [FightCue::Hit {
            attacker: 3,
            victim: 4,
            damage: 200,
        }];
        assert_eq!(eye.feed(1, &rivals, me), 0);
        // A blow taken jolts in full, then fades by the decay every tick, never popping back.
        let taken = [FightCue::Hit {
            attacker: 3,
            victim: me,
            damage: 200,
        }];
        let jolt = eye.feed(2, &taken, me);
        assert_eq!(jolt, crate::camera_feed::TRAUMA_HIT);
        // The same tick fed again (a second frame of the tick) changes nothing.
        assert_eq!(eye.feed(2, &taken, me), jolt);
        let mut last = jolt;
        for tick in 3..20 {
            let now = eye.feed(tick, &[], me);
            assert!(
                now <= last && last - now <= EYE_TRAUMA_DECAY_PER_TICK,
                "tick {tick}: {last} -> {now}"
            );
            last = now;
        }
        assert_eq!(last, 0, "a hit's shake must be gone inside 20 ticks");
        // A blow the player lands jolts at a third.
        let dealt = [FightCue::Hit {
            attacker: me,
            victim: 3,
            damage: 200,
        }];
        assert_eq!(eye.feed(30, &dealt, me), crate::camera_feed::TRAUMA_HIT / 3);
        // Knockdowns stack but cap at full.
        let floored = [FightCue::Knockdown {
            attacker: 3,
            victim: me,
        }; 5];
        assert_eq!(eye.feed(31, &floored, me), crate::camera::TRAUMA_SCALE);
    }
    #[test]
    fn interp_scale_lane_type_holds_the_public_number() {
        // The lane type carries its own permille scale (`INTERP_SCALE_Q`): a
        // retune of the public `u32` must move both, never one.
        assert_eq!(
            INTERP_SCALE,
            u32::from(INTERP_SCALE_Q),
            "lane scale drifted"
        );
    }
    #[test]
    fn fps_look_clamps_and_wraps() {
        let mut look = LookState::neutral();
        look.apply_mouse(0.0, -100000.0);
        let top = FRAC_PI_2 - ANGLE_EPSILON;
        assert!(
            (look.pitch_rad - top).abs() < 0.0001,
            "pitch not clamped: {:?}",
            look.pitch_rad
        );
        look.apply_mouse(f32::NAN, 0.0);
        assert!(
            (look.pitch_rad - top).abs() < 0.0001,
            "nan moved pitch: {:?}",
            look.pitch_rad
        );
        look.apply_mouse(20000.0, 0.0);
        assert!(
            look.yaw_rad >= -PI && look.yaw_rad <= PI,
            "yaw not wrapped: {:?}",
            look.yaw_rad
        );
        let frame = LogicalFrame {
            pos_mm: [1000, 0, 0],
            prev_mm: [0, 0, 0],
            look,
        };
        let e0 = render_eye(&frame, 0, EYE_HEIGHT_MM);
        let e1 = render_eye(&frame, 1000, EYE_HEIGHT_MM);
        assert_eq!(e0.eye_mm[0], 0.0, "alpha 0 missed prev");
        assert_eq!(e1.eye_mm[0], 1000.0, "alpha max missed current");
        assert_eq!(
            render_eye(&frame, 500, EYE_HEIGHT_MM),
            render_eye(&frame, 500, EYE_HEIGHT_MM),
            "eye replay diverged"
        );
    }
    #[test]
    fn eye_for_fighter_puts_eye_at_head_height() {
        let eye = eye_for_fighter([1.0, 2.0, 3.0], 0.75, EYE_HEIGHT_MM).expect("valid eye");
        assert_eq!(eye.eye_mm[0], 1000.0, "x missed pos");
        assert_eq!(
            eye.eye_mm[1],
            2000.0 + mm_lane(EYE_HEIGHT_MM),
            "y missed height"
        );
        assert_eq!(eye.eye_mm[2], 3000.0, "z missed pos");
        assert_eq!(eye.yaw_rad, 0.75, "yaw missed facing");
        assert_eq!(eye.pitch_rad, 0.0, "pitch must be 0");
    }
    #[test]
    fn eye_for_fighter_refuses_bad_params() {
        for result in [
            eye_for_fighter([f32::NAN, 0.0, 0.0], 0.0, EYE_HEIGHT_MM),
            eye_for_fighter([0.0, 0.0, 0.0], f32::NAN, EYE_HEIGHT_MM),
            eye_for_fighter([0.0, 0.0, 0.0], 0.0, 0),
            eye_for_fighter([0.0, 0.0, 0.0], 0.0, 3000),
        ] {
            assert!(
                matches!(result, Err(CameraError::BadParam(row)) if row.contains("FV1")),
                "bad input accepted: {result:?}"
            );
        }
    }
    #[test]
    fn eye_for_fighter_replays_byte_exact_twice() {
        let first = eye_for_fighter([1.5, 0.5, -2.0], -1.2, 1600).expect("valid eye");
        let second = eye_for_fighter([1.5, 0.5, -2.0], -1.2, 1600).expect("valid eye");
        assert_eq!(first, second, "eye replay diverged");
    }
    #[test]
    fn eye_for_fighter_allocates_nothing() {
        let (_, probe) = engine_core::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert_ne!(probe.calls, 0, "sanity: probe sees heap");
        let (eye, steady) = engine_core::alloc_probe::measure(|| {
            eye_for_fighter([1.0, 2.0, 3.0], 0.75, EYE_HEIGHT_MM).expect("valid eye")
        });
        assert_eq!(steady.calls, 0, "eye pose must not allocate");
        assert_eq!(eye.eye_mm[0], 1000.0, "sanity: eye moved");
    }
    /// Kick length in mm: the L2 norm of the eye-space kick triple.
    fn kick_len_mm(kick: &[f32; 3]) -> f32 {
        (kick[0] * kick[0] + kick[1] * kick[1] + kick[2] * kick[2]).sqrt()
    }
    #[test]
    fn shake_eye_kick_and_fov_punch_hold_gp21_bar() {
        let gain_q = u32::from(EYE_SHAKE_GAIN_Q);
        let calm = eye_shake_for_trauma(0, 91, gain_q);
        assert_eq!(calm, EyeShake::none(), "calm is not zero");
        // GP21 bar, the C2 charter oracle mirrored (playtest charter.rs:174):
        // light x 4 <= heavy in mm. Both permille values come from the feed.
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        let jab = trauma_for_cue(hit);
        let down = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        let burst = trauma_for_cues(&[down; 3]);
        assert!(burst > TRAUMA_SCALE, "the burst must saturate");
        let (tick, super_trauma) = (120, burst.min(TRAUMA_SCALE));
        let jab_shake = eye_shake_for_trauma(jab, tick, gain_q);
        let super_shake = eye_shake_for_trauma(super_trauma, tick, gain_q);
        let (light_mm, heavy_mm) = (
            kick_len_mm(&jab_shake.kick_mm).ceil(),
            kick_len_mm(&super_shake.kick_mm).ceil(),
        );
        assert!(heavy_mm > 0.0, "the super kick must move the eye");
        assert!(light_mm * 4.0 <= heavy_mm, "jab over a quarter of super");
        // The bar holds at every tick: the same tick feeds both curves the same
        // noise phase, so only the squared gain differs between the two kicks.
        for tick in [0u32, 300, 4096] {
            let light = kick_len_mm(&eye_shake_for_trauma(jab, tick, gain_q).kick_mm);
            let heavy = kick_len_mm(&eye_shake_for_trauma(super_trauma, tick, gain_q).kick_mm);
            assert!(light * 4.0 <= heavy, "the bar slipped at tick {tick}");
        }
        // Roll bounded by 0.10 rad times gain; both channels stay inside caps.
        let bar = trauma_gain(jab) * EYE_ROLL_MAX_RAD;
        let punch_cap = FOV_PUNCH_MAX_RAD;
        assert!(jab_shake.roll_rad <= bar, "jab roll over 0.10 x gain");
        assert!(jab_shake.fov_punch_rad <= punch_cap, "jab punch over cap");
        assert!(super_shake.roll_rad <= EYE_ROLL_MAX_RAD, "roll over cap");
        assert!(super_shake.fov_punch_rad <= punch_cap, "punch over cap");
        assert!(super_shake.roll_rad > 0.0, "the shake must roll");
        assert!(super_shake.fov_punch_rad > 0.0, "the shake must punch");
        let again = eye_shake_for_trauma(jab, tick, gain_q);
        assert_eq!(again, jab_shake, "shake replay split");
    }
    #[test]
    fn shake_eye_replays_exactly_twice() {
        let mut look = LookState::neutral();
        look.yaw_rad = 0.4;
        look.pitch_rad = -0.1;
        let frame = LogicalFrame {
            pos_mm: [1000, 0, 0],
            prev_mm: [0, 0, 0],
            look,
        };
        let gain_q = u32::from(EYE_SHAKE_GAIN_Q);
        let track: Vec<(u32, u32)> = (0..30u32).map(|tick| (tick, 120 + tick * 20)).collect();
        let bits = |track: &[(u32, u32)]| -> Vec<u64> {
            let mut out = Vec::new();
            for (tick, trauma) in track {
                let eye = render_eye_shaken(&frame, 500, EYE_HEIGHT_MM, *trauma, *tick, gain_q);
                out.extend(eye.eye_mm.iter().map(|lane| u64::from(lane.to_bits())));
                let shake = eye_shake_for_trauma(*trauma, *tick, gain_q);
                out.push(u64::from(eye.roll_rad.to_bits()));
                out.push(u64::from(shake.fov_punch_rad.to_bits()));
            }
            out
        };
        assert_eq!(bits(&track), bits(&track), "the 30-tick tape must replay");
        let first = bits(&track);
        assert!(
            first.iter().skip(1).any(|byte| *byte != first[0]),
            "the shaken track must move the eye"
        );
        // Trauma 0 never moves the eye: byte-identical to the plain render eye.
        for tick in 0..30u32 {
            let calm = render_eye_shaken(&frame, 500, EYE_HEIGHT_MM, 0, tick, gain_q);
            assert_eq!(calm, render_eye(&frame, 500, EYE_HEIGHT_MM), "calm moved");
            assert_eq!(calm.roll_rad, 0.0, "calm rolled");
        }
    }
    /// The eye-to-camera feed: `eye_for_fighter` -> [`camera_for_eye`] ->
    /// `v1::Camera`. The module name carries the `eye_for_fighter` prefix the
    /// row's filter command selects.
    #[cfg(feature = "app")]
    mod eye_for_fighter_camera_feed {
        use super::super::{
            camera_for_eye, eye_for_fighter, RenderEye, EYE_HEIGHT_MM, EYE_ROLL_MAX_RAD,
        };
        use crate::camera::CameraError;
        use std::f32::consts::{FRAC_PI_2, PI};
        use std::hint::black_box;

        /// The `BadParam` field the feed refused with; `None` when it accepted
        /// or refused some other way.
        fn refused(eye: &RenderEye, fov_y_rad: f32, aspect: f32) -> Option<&'static str> {
            match camera_for_eye(eye, fov_y_rad, aspect) {
                Err(CameraError::BadParam(row)) => Some(row),
                _ => None,
            }
        }

        #[test]
        fn eye_for_fighter_feeds_a_validated_camera_for_eye() {
            let eye = eye_for_fighter([1.0, 2.0, 3.0], 0.75, EYE_HEIGHT_MM).expect("valid eye");
            let a = camera_for_eye(&eye, 1.0, 16.0 / 9.0).expect("valid camera");
            let b = camera_for_eye(&eye, 1.0, 16.0 / 9.0).expect("valid camera");
            assert_eq!(a, b, "camera replay split");
            assert_eq!(a.eye.to_array(), [1.0, 3.6, 3.0], "eye missed");
            // Row bar: identical eyes give bit-identical view-proj bytes twice.
            let ma = a.view_proj().expect("valid").to_cols_array();
            let mb = b.view_proj().expect("valid").to_cols_array();
            assert!(ma.iter().all(|v| v.is_finite()), "not finite");
            assert_eq!(ma, mb, "matrix split");
            // A level eye at yaw 0.75 looks one meter along (sin, 0, cos).
            let forward = [a.target.x - a.eye.x, a.target.z - a.eye.z];
            assert!((forward[0] - 0.75f32.sin()).abs() < 1e-6, "yaw x");
            assert_eq!(a.target.y - a.eye.y, 0.0, "level eye pitched");
            assert!((forward[1] - 0.75f32.cos()).abs() < 1e-6, "yaw z");
        }

        #[test]
        fn camera_for_eye_level_yaw_zero_faces_plus_z() {
            // The frame the render crate's test-only constructor mirrors: yaw 0
            // looks down +z, the target sits one meter ahead and the up axis
            // is +y. Both crates pin the same literals, so a drift on either
            // side turns one of the two tests red.
            let eye = eye_for_fighter([1.0, 2.0, 3.0], 0.0, EYE_HEIGHT_MM).expect("valid eye");
            let camera = camera_for_eye(&eye, 1.0, 16.0 / 9.0).expect("valid camera");
            assert_eq!(camera.eye.to_array(), [1.0, 3.6, 3.0], "eye missed");
            assert_eq!(
                camera.target.to_array(),
                [1.0, 3.6, 4.0],
                "yaw 0 must face +z one meter ahead"
            );
            assert_eq!(
                camera.up.to_array(),
                [0.0, 1.0, 0.0],
                "a level roll must keep +y up"
            );
        }

        #[test]
        fn camera_for_eye_replays_and_allocates_nothing() {
            let eye = eye_for_fighter([1.5, 0.5, -2.0], -1.2, EYE_HEIGHT_MM).expect("valid eye");
            let (_, probe) = engine_core::alloc_probe::measure(|| black_box(vec![0u8; 64]));
            assert_ne!(probe.calls, 0, "sanity: probe sees heap");
            let feed = || camera_for_eye(&eye, 1.0, 1.5).expect("valid camera");
            let (a, steady) = engine_core::alloc_probe::measure(feed);
            assert_eq!(steady.calls, 0, "the feed must not allocate");
            let b = camera_for_eye(&eye, 1.0, 1.5).expect("valid camera");
            let ma = a.view_proj().expect("valid").to_cols_array();
            let mb = b.view_proj().expect("valid").to_cols_array();
            assert_eq!(ma, mb, "matrix split");
        }

        #[test]
        fn camera_for_eye_refuses_bad_fov_aspect_and_eye() {
            let good = eye_for_fighter([0.0, 0.0, 0.0], 0.0, EYE_HEIGHT_MM).expect("valid eye");
            let nan_eye = RenderEye {
                eye_mm: [f32::NAN, 0.0, 0.0],
                ..good
            };
            let nan_yaw = RenderEye {
                yaw_rad: f32::NAN,
                ..good
            };
            let inf_pitch = RenderEye {
                pitch_rad: f32::INFINITY,
                ..good
            };
            let nan_roll = RenderEye {
                roll_rad: f32::NAN,
                ..good
            };
            assert_eq!(refused(&nan_eye, 1.0, 1.5), Some("FV3 eye_mm"));
            assert_eq!(refused(&nan_yaw, 1.0, 1.5), Some("FV3 yaw_rad"));
            assert_eq!(refused(&inf_pitch, 1.0, 1.5), Some("FV3 pitch_rad"));
            assert_eq!(refused(&nan_roll, 1.0, 1.5), Some("FV3 roll_rad"));
            // Every out-of-range fov or aspect refuses; no silent clamp.
            for (fov_y_rad, aspect) in [
                (0.0, 1.5),
                (PI, 1.5),
                (f32::NAN, 1.5),
                (1.0, 0.0),
                (1.0, 0.5e-3),
                (1.0, 1.0e5),
                (1.0, f32::NAN),
            ] {
                assert!(refused(&good, fov_y_rad, aspect).is_some(), "accepted");
            }
            // Straight up leaves no up axis beside the view: the render
            // camera's own refusal reaches the caller typed, never a NaN matrix.
            let up_look = RenderEye {
                pitch_rad: FRAC_PI_2,
                ..good
            };
            assert_eq!(refused(&up_look, 1.0, 1.5), Some("FV3 up"));
        }

        #[test]
        fn camera_for_eye_roll_never_moves_eye_or_target() {
            let level = eye_for_fighter([2.0, 0.25, -1.0], 1.1, EYE_HEIGHT_MM).expect("valid eye");
            let flat = camera_for_eye(&level, 1.05, 1.5).expect("valid camera");
            let turned = RenderEye {
                roll_rad: EYE_ROLL_MAX_RAD,
                ..level
            };
            let rolled = camera_for_eye(&turned, 1.05, 1.5).expect("valid camera");
            assert_eq!(flat.eye, rolled.eye, "roll moved the eye");
            assert_eq!(flat.target, rolled.target, "roll moved the target");
            assert_ne!(flat.up, rolled.up, "roll left the up axis alone");
            // The rolled up axis stays unit length and square to the view.
            let up = rolled.up.normalize();
            let view = (rolled.target - rolled.eye).normalize();
            assert!(up.dot(view).abs() < 1e-6, "up left the view plane");
        }
    }

    /// The legionary mode: the kit the player wears, the eye it looks
    /// through and the camera that eye becomes. The module name carries the
    /// `legionary` prefix the row's filter command selects.
    mod legionary_mode {
        use super::super::{
            legionary_eye, mm_lane, LegionaryKit, LegionaryView, EYE_HEIGHT_MM, EYE_ROLL_MAX_RAD,
        };
        use crate::camera::CameraError;

        #[test]
        fn legionary_kit_names_the_shipped_pieces_and_their_armor_bits() {
            let kit = LegionaryKit::LEGIONARY;
            assert_eq!(kit.line(), "galea,gladius,scutum", "the kit line");
            assert_eq!(kit.helmet, "galea");
            assert_eq!(kit.weapon, "gladius");
            assert_eq!(kit.shield, "scutum");
            // The armor bits are the combat crate's own constants: the galea
            // on the head and the scutum on the shield arm, nothing else.
            assert_eq!(
                kit.worn & engine_combat::ARMOR_GALEA,
                engine_combat::ARMOR_GALEA
            );
            assert_eq!(
                kit.worn & engine_combat::ARMOR_SCUTUM,
                engine_combat::ARMOR_SCUTUM
            );
            assert_eq!(
                kit.worn & !engine_combat::ARMOR_ALL,
                0,
                "bits outside the armor set"
            );
            // The shipped anatomy row really covers those joints with them.
            let anatomy = engine_combat::AnatomyTable::rome_kit();
            let bare = anatomy.protection_q16(engine_combat::JointId::Neck, 0);
            let worn = anatomy.protection_q16(engine_combat::JointId::Neck, kit.worn);
            assert!(worn > bare, "the galea must guard the neck");
        }

        /// The kit's sword is the shipped weapons row, not a free name: the
        /// gladius row of `content/design/weapons.rows` carries the sword's
        /// own numbers, and the mode says so when the table loses it.
        #[test]
        fn legionary_gladius_is_the_shipped_weapons_row() {
            let kit = LegionaryKit::LEGIONARY;
            let row = super::super::weapons_row(super::super::WEAPONS_ROWS, kit.weapon)
                .expect("the weapons table still names the gladius");
            assert!(
                row.contains("damage_permille=") && row.contains("speed_ticks="),
                "the gladius row carries its own numbers: {row}"
            );
            assert!(
                kit.backed_by_shipped_data(),
                "the shipped data backs the kit"
            );
            // A table without the row is drift the mode must report.
            let bare = super::super::weapons_row("pugio type=1 damage_permille=80", kit.weapon);
            assert!(bare.is_none(), "a table without the gladius names nothing");
        }

        #[test]
        fn legionary_view_toggles_between_the_eye_and_the_fight_camera() {
            assert_eq!(
                LegionaryView::default(),
                LegionaryView::Eye,
                "the eye is the start"
            );
            assert_eq!(LegionaryView::Eye.name(), "fps");
            assert_eq!(LegionaryView::Third.name(), "third");
            let round = LegionaryView::Eye.toggled().toggled();
            assert_eq!(round, LegionaryView::Eye, "the toggle comes back");
            assert_eq!(LegionaryView::Eye.toggled(), LegionaryView::Third);
        }

        #[test]
        fn legionary_eye_sits_at_head_height_and_kicks_with_trauma() {
            let calm = legionary_eye([1.0, 0.0, 2.0], 0.5, 0, 7).expect("valid eye");
            assert_eq!(calm.eye_mm[0], 1000.0, "x missed the feet");
            assert_eq!(
                calm.eye_mm[1],
                mm_lane(EYE_HEIGHT_MM),
                "y missed the height"
            );
            assert_eq!(calm.eye_mm[2], 2000.0, "z missed the feet");
            assert_eq!(calm.yaw_rad, 0.5, "yaw missed the facing");
            assert_eq!(calm.roll_rad, 0.0, "a calm eye does not roll");
            // Trauma moves the eye about itself and rolls the view.
            let hit = legionary_eye([1.0, 0.0, 2.0], 0.5, 400, 7).expect("valid eye");
            assert_ne!(hit.eye_mm, calm.eye_mm, "the kick must move the eye");
            assert!(hit.roll_rad > 0.0, "the kick must roll the view");
            assert!(
                hit.roll_rad <= EYE_ROLL_MAX_RAD,
                "the roll stays under the cap"
            );
            // The same inputs give the same bytes twice.
            let again = legionary_eye([1.0, 0.0, 2.0], 0.5, 400, 7).expect("valid eye");
            assert_eq!(hit, again, "the eye replay split");
            // A bad position refuses by name, never a NaN pose.
            for bad in [[f32::NAN, 0.0, 0.0], [0.0, f32::INFINITY, 0.0]] {
                assert!(matches!(
                    legionary_eye(bad, 0.0, 0, 0),
                    Err(CameraError::BadParam(row)) if row.contains("FV1")
                ));
            }
        }

        /// The first-person view wears the galea, and the rim draws in its
        /// own depth range: a wall five centimetres in front of the eye
        /// cannot cover it, and the rim sits at the edge of the frame where
        /// the reference pictures show it.
        #[test]
        fn legionary_eye_wears_the_galea_and_its_rim_draws_in_its_own_depth_range() {
            use engine_render::viewmodel::{DepthRange, DepthTarget};
            let eye = super::super::legionary_eye([1.0, 0.0, 2.0], 0.5, 0, 7).expect("valid eye");
            assert_eq!(
                eye.helmet.preset,
                engine_render::viewmodel::AperturePreset::GaleaCheekGuards,
                "the legionary wears the galea"
            );
            assert_eq!(
                eye.helmet.depth,
                engine_render::viewmodel::VIEWMODEL_DEPTH,
                "the rim draws in its own depth range"
            );
            assert_eq!(
                eye.helmet.lens,
                engine_render::lens::LensPreset::Helmet,
                "the helmet lens rides the view"
            );
            // The world may draw right in front of the eye: the world's
            // near plane sits inside the rim's range, so the rim cannot
            // lean on the world's clip.
            let mut world = DepthTarget::new(DepthRange::new(0.001, 100.0).expect("a world range"));
            assert!(
                world.write(0.01),
                "the wall one centimetre in front of the eye is drawn"
            );
            let mut rim = DepthTarget::new(eye.helmet.depth);
            assert!(rim.write(0.060), "the rim is drawn");
            assert!(
                !world.write(0.060),
                "the wall covers the rim in the world's target"
            );
            // The rim is in view: its inner edge sits inside the frame and
            // its outer edge past it, so the rim frames the picture.
            let row = eye.helmet.preset.table_row();
            let projection =
                engine_render::viewmodel::viewmodel_view_proj(eye.helmet.depth, 1.05, 16.0 / 9.0)
                    .expect("a valid projection");
            let edge = |y_mm: f32| {
                let y = y_mm / 1000.0;
                let z = row.z_front_mm / 1000.0;
                (projection[5] * y) / (projection[11] * z)
            };
            let inner = row.half_height_mm - row.band_mm;
            assert!(
                edge(inner).abs() < 1.0,
                "the opening's edge must be inside the frame, got {}",
                edge(inner)
            );
            assert!(
                edge(row.half_height_mm).abs() > 1.0,
                "the rim must reach past the frame's edge, got {}",
                edge(row.half_height_mm)
            );
            // The rim's own mesh builds from the table row the eye names.
            let mesh = engine_render::viewmodel::build_rim_mesh(eye.helmet.preset)
                .expect("the table row builds");
            assert!(mesh.triangle_count() > 0, "the rim has triangles");
            for vertex in &mesh.vertices {
                assert!(
                    vertex.position_m[2] > eye.helmet.depth.near_m,
                    "the rim must sit inside its own range"
                );
            }
        }
    }
}
