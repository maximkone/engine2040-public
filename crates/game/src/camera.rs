//! camera: deterministic arena camera director plus data-table tuning
//! and fixture rule-set loading.
//!
//! Purpose: lock-on target choice, integer-tick trauma decay and pure shake
//! offsets for arena fight presentation; director params as versioned data
//! tables; a fixture rule set boots headless and replays with no engine
//! edits. Owner: `engine-game`.
//! Units: meters, radians, fixed ticks; trauma in permille (`0..=1000`).
//! Determinism: same fighter tracks plus same tick calls give identical
//! [`CameraFrame`] bytes twice; shake never feeds simulation state; fixture
//! sim uses `+ - * /` plus one presentation `sqrt`, SplitMix spawns and FNV
//! hashes, same bytes twice on one binary. Errors: [`CameraError`] and
//! [`FixtureError`] at every boundary; bad input fails loudly, never wraps.
//! Budget: 0.05 ms per frame, zero heap bytes per update (counted test);
//! fixture boot is O(runners + inputs), each step O(runners) with no heap
//! growth after boot sizing.
//! Consumer: the windowed leg (wired as `pub mod camera`); the
//! mode framework boots [`FixtureMode`] from rule-set text.

use engine_core::hash::{FNV64_OFFSET, GOLDEN_GAMMA};
use engine_core::{mix64, splitmix64};

/// Fixed byte length of [`CameraFrame::to_bytes`].
pub const FRAME_BYTES: usize = 46;
/// Trauma scale: permille `0..=1000` maps to shake `0..=1`.
pub const TRAUMA_SCALE: u32 = 1000;
/// Lock byte when no target is framed: present flag `0`, id byte `0`.
pub const LOCK_NONE_U8: u8 = 0xFF;

/// Position or offset in meters, right-handed with `+y` up.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec3 {
    /// Meters on the `x` axis.
    pub x: f32,
    /// Meters on the `y` axis (up).
    pub y: f32,
    /// Meters on the `z` axis.
    pub z: f32,
}

impl Vec3 {
    /// Zero offset.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Build a vector from component meters.
    #[must_use]
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// True when every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Squared length; never negative for finite inputs.
    #[must_use]
    pub fn len_sq(self) -> f32 {
        self.dot(self)
    }

    /// Length in meters.
    #[must_use]
    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }

    /// Unit vector, or [`CameraError::ZeroLength`] for the zero vector.
    pub fn normalize(self) -> Result<Self, CameraError> {
        let len_sq = self.len_sq();
        if len_sq == 0.0 {
            return Err(CameraError::ZeroLength);
        }
        Ok(self * (1.0 / len_sq.sqrt()))
    }

    /// Unit vector, or `fallback` for the zero vector (internal pose math
    /// must never fail after validation; the fallback keeps it total).
    fn normalize_or(self, fallback: Self) -> Self {
        let len_sq = self.len_sq();
        if len_sq == 0.0 {
            fallback
        } else {
            self * (1.0 / len_sq.sqrt())
        }
    }
}

impl core::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl core::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl core::ops::Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, scalar: f32) -> Self {
        Self::new(self.x * scalar, self.y * scalar, self.z * scalar)
    }
}

/// One fighter's presentation position for a tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FighterView {
    /// Fighter slot id; ties break toward the smaller id.
    pub id: u8,
    /// Feet position in meters.
    pub pos_m: Vec3,
    /// Dead fighters are never lock targets but are still framed.
    pub alive: bool,
}

/// Director tuning: field of view, framing fit, boom shape, shake cap and
/// the integer trauma decay applied per tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DirectorParams {
    /// Full vertical field of view in radians, `0 < fov < PI`.
    pub fov_y_rad: f32,
    /// Extra meters around the pair when fitting the framing distance.
    pub fit_margin_m: f32,
    /// Base camera distance in meters, inside `[min_dist_m, max_dist_m]`.
    pub base_dist_m: f32,
    /// Closest the camera ever sits to its look point, meters.
    pub min_dist_m: f32,
    /// Farthest the camera ever sits, also the lock-on range, meters.
    pub max_dist_m: f32,
    /// Boom height per meter of distance (eye sits `boom_up * dist` above).
    pub boom_up: f32,
    /// Boom pull-back per meter of distance (eye sits `boom_back * dist`
    /// behind on `+z`).
    pub boom_back: f32,
    /// Shake offset cap in meters at full trauma.
    pub max_shake_m: f32,
    /// Trauma permille decayed per tick; `0` never decays.
    pub decay_permille_per_tick: u32,
}

impl DirectorParams {
    /// Range-check every row; the culprit row name is carried.
    pub fn validate(&self) -> Result<(), CameraError> {
        if !self.fov_y_rad.is_finite()
            || self.fov_y_rad <= 0.0
            || self.fov_y_rad >= core::f32::consts::PI
        {
            return Err(CameraError::BadParam("fov_y_rad"));
        }
        if !self.fit_margin_m.is_finite() || self.fit_margin_m < 0.0 {
            return Err(CameraError::BadParam("fit_margin_m"));
        }
        if !self.base_dist_m.is_finite() || self.base_dist_m <= 0.0 {
            return Err(CameraError::BadParam("base_dist_m"));
        }
        if !self.min_dist_m.is_finite()
            || !self.max_dist_m.is_finite()
            || self.min_dist_m <= 0.0
            || self.min_dist_m > self.max_dist_m
        {
            return Err(CameraError::BadParam("min/max_dist_m"));
        }
        if self.base_dist_m < self.min_dist_m || self.base_dist_m > self.max_dist_m {
            return Err(CameraError::BadParam("base_dist_m"));
        }
        if !self.boom_up.is_finite()
            || !self.boom_back.is_finite()
            || self.boom_up < 0.0
            || self.boom_back < 0.0
            || (self.boom_up == 0.0 && self.boom_back == 0.0)
        {
            return Err(CameraError::BadParam("boom"));
        }
        if !self.max_shake_m.is_finite() || self.max_shake_m < 0.0 {
            return Err(CameraError::BadParam("max_shake_m"));
        }
        Ok(())
    }
}

/// Version of the [`DirectorParams`] data-table codec below.
pub const DIRECTOR_TABLE_VERSION: u8 = 1;
/// Exact byte length of [`DirectorParams::encode`]: version (1) plus eight
/// `f32` rows (32) plus the integer decay row (4).
pub const DIRECTOR_TABLE_BYTES: usize = 37;

/// Write one little-endian `f32` into a table slice at a fixed offset.
fn put_table_f32(out: &mut [u8], at: usize, value: f32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Read one little-endian `f32` from a table slice at a fixed offset.
fn get_table_f32(bytes: &[u8], at: usize) -> f32 {
    let mut quad = [0u8; 4];
    quad.copy_from_slice(&bytes[at..at + 4]);
    f32::from_le_bytes(quad)
}

impl DirectorParams {
    /// Arena table: identical to [`DirectorParams::default`], named so mode
    /// rule sets can pick it as data. Pure.
    #[must_use]
    pub fn arena() -> Self {
        Self::default()
    }

    /// Traversal-course table: third-person follow for a single agent, wider
    /// fov and longer rails than the arena duel table. Pure.
    #[must_use]
    pub fn traversal() -> Self {
        Self {
            fov_y_rad: 70.0f32.to_radians(),
            fit_margin_m: 0.5,
            base_dist_m: 6.0,
            min_dist_m: 2.5,
            max_dist_m: 40.0,
            boom_up: 0.55,
            boom_back: 0.9,
            max_shake_m: 0.25,
            decay_permille_per_tick: 45,
        }
    }

    /// Encode every tuning row as a versioned little-endian table: version
    /// (1) then `fov_y_rad`, `fit_margin_m`, `base_dist_m`, `min_dist_m`,
    /// `max_dist_m`, `boom_up`, `boom_back`, `max_shake_m` (8 x `f32`) then
    /// `decay_permille_per_tick` (`u32`). Same params give the same bytes
    /// twice; `decode(encode(p))` is `p`. Budget: 37 stack bytes, no heap.
    #[must_use]
    pub fn encode(self) -> [u8; DIRECTOR_TABLE_BYTES] {
        let mut out = [0u8; DIRECTOR_TABLE_BYTES];
        out[0] = DIRECTOR_TABLE_VERSION;
        put_table_f32(&mut out, 1, self.fov_y_rad);
        put_table_f32(&mut out, 5, self.fit_margin_m);
        put_table_f32(&mut out, 9, self.base_dist_m);
        put_table_f32(&mut out, 13, self.min_dist_m);
        put_table_f32(&mut out, 17, self.max_dist_m);
        put_table_f32(&mut out, 21, self.boom_up);
        put_table_f32(&mut out, 25, self.boom_back);
        put_table_f32(&mut out, 29, self.max_shake_m);
        out[33..37].copy_from_slice(&self.decay_permille_per_tick.to_le_bytes());
        out
    }

    /// Decode plus validate a table [`DirectorParams::encode`] wrote: short
    /// or long slices fail with [`CameraError::BadLength`], newer versions
    /// with [`CameraError::FutureVersion`], out-of-range rows with the
    /// culprit row name. Pure.
    pub fn decode(bytes: &[u8]) -> Result<Self, CameraError> {
        if bytes.len() != DIRECTOR_TABLE_BYTES {
            return Err(CameraError::BadLength(bytes.len()));
        }
        if bytes[0] != DIRECTOR_TABLE_VERSION {
            return Err(CameraError::FutureVersion(bytes[0]));
        }
        let mut decay = [0u8; 4];
        decay.copy_from_slice(&bytes[33..37]);
        let params = Self {
            fov_y_rad: get_table_f32(bytes, 1),
            fit_margin_m: get_table_f32(bytes, 5),
            base_dist_m: get_table_f32(bytes, 9),
            min_dist_m: get_table_f32(bytes, 13),
            max_dist_m: get_table_f32(bytes, 17),
            boom_up: get_table_f32(bytes, 21),
            boom_back: get_table_f32(bytes, 25),
            max_shake_m: get_table_f32(bytes, 29),
            decay_permille_per_tick: u32::from_le_bytes(decay),
        };
        params.validate()?;
        Ok(params)
    }
}

impl Default for DirectorParams {
    /// Arena defaults: 60-degree fov, 1 m margin, 4 m base in `[2, 30] m`,
    /// a low behind-and-above boom, 0.35 m shake cap, 60 permille decay.
    fn default() -> Self {
        Self {
            fov_y_rad: core::f32::consts::FRAC_PI_3,
            fit_margin_m: 1.0,
            base_dist_m: 4.0,
            min_dist_m: 2.0,
            max_dist_m: 30.0,
            boom_up: 0.45,
            boom_back: 1.0,
            max_shake_m: 0.35,
            decay_permille_per_tick: 60,
        }
    }
}

/// Camera failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraError {
    /// Tuning row outside its range, row name carried.
    BadParam(&'static str),
    /// Fighter position not finite, fighter id carried.
    BadPosition(u8),
    /// Camera eye not finite.
    BadEye,
    /// View forward not finite or zero length.
    BadForward,
    /// Zero-length vector has no direction.
    ZeroLength,
    /// Focus fighter absent from the tick slice, focus id carried.
    FocusMissing(u8),
    /// Director table length is not [`DIRECTOR_TABLE_BYTES`], length carried.
    BadLength(usize),
    /// Director table version is newer than [`DIRECTOR_TABLE_VERSION`].
    FutureVersion(u8),
    /// Ticks run forward only: `next` arrived after `last` was served.
    TickWentBackwards {
        /// Last tick served.
        last: u32,
        /// Rejected tick.
        next: u32,
    },
}

impl core::fmt::Display for CameraError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadParam(row) => write!(f, "bad camera param ({row})"),
            Self::BadPosition(id) => write!(f, "bad position for fighter ({id})"),
            Self::BadEye => write!(f, "bad camera eye"),
            Self::BadForward => write!(f, "bad camera forward"),
            Self::ZeroLength => write!(f, "zero-length vector"),
            Self::FocusMissing(id) => write!(f, "focus fighter missing ({id})"),
            Self::BadLength(len) => write!(f, "bad director table length ({len})"),
            Self::FutureVersion(version) => {
                write!(
                    f,
                    "director table version {version} is newer than v{DIRECTOR_TABLE_VERSION}"
                )
            }
            Self::TickWentBackwards { last, next } => {
                write!(f, "camera tick went backwards ({next} after {last})")
            }
        }
    }
}

impl std::error::Error for CameraError {}

/// Unshaken camera pose in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraPose {
    /// Camera eye in meters (shake never included here).
    pub eye_m: Vec3,
    /// Look point in meters.
    pub look_m: Vec3,
}

/// One presented frame: shaken eye plus the audit trail (tick, lock target,
/// trauma) that replays byte-exact.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraFrame {
    /// Fixed tick presented.
    pub tick: u32,
    /// Shaken eye in meters: pose eye plus [`shake_offset`].
    pub eye_m: Vec3,
    /// Look point in meters (never shaken).
    pub look_m: Vec3,
    /// Applied shake offset in meters (presentation only).
    pub shake_m: Vec3,
    /// Lock-on target this frame, if any fighter was in frustum.
    pub lock: Option<u8>,
    /// Trauma permille after this tick's decay.
    pub trauma_permille: u32,
}

impl CameraFrame {
    /// Fixed 46-byte little-endian layout: tick, eye xyz, look xyz, shake
    /// xyz, trauma, lock-present flag, lock id (`0` when none). Same tape
    /// plus same tick calls give the same bytes twice.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; FRAME_BYTES] {
        let mut out = [0u8; FRAME_BYTES];
        put_f32(&mut out, 4, self.eye_m.x);
        put_f32(&mut out, 8, self.eye_m.y);
        put_f32(&mut out, 12, self.eye_m.z);
        put_f32(&mut out, 16, self.look_m.x);
        put_f32(&mut out, 20, self.look_m.y);
        put_f32(&mut out, 24, self.look_m.z);
        put_f32(&mut out, 28, self.shake_m.x);
        put_f32(&mut out, 32, self.shake_m.y);
        put_f32(&mut out, 36, self.shake_m.z);
        out[0..4].copy_from_slice(&self.tick.to_le_bytes());
        out[40..44].copy_from_slice(&self.trauma_permille.to_le_bytes());
        match self.lock {
            Some(id) => {
                out[44] = 1;
                out[45] = id;
            }
            None => {
                out[44] = 0;
                out[45] = 0;
            }
        }
        out
    }
}

/// Write one little-endian `f32` at a fixed offset.
fn put_f32(out: &mut [u8; FRAME_BYTES], at: usize, value: f32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Nearest-in-frustum lock-on target: the closest alive candidate inside the
/// view cone and range, excluding the focus fighter. Bit-exact distance ties
/// break toward the smaller fighter id, never toward container order, so
/// shuffled slices pick the same target. Every position is validated even
/// when its fighter is skipped, so a NaN fails loudly instead of hiding.
pub fn select_lock_target(
    eye_m: Vec3,
    forward: Vec3,
    half_fov_rad: f32,
    max_dist_m: f32,
    focus_id: u8,
    candidates: &[FighterView],
) -> Result<Option<u8>, CameraError> {
    if !eye_m.is_finite() {
        return Err(CameraError::BadEye);
    }
    if !half_fov_rad.is_finite() || half_fov_rad <= 0.0 || half_fov_rad >= core::f32::consts::PI {
        return Err(CameraError::BadParam("half_fov_rad"));
    }
    if !max_dist_m.is_finite() || max_dist_m <= 0.0 {
        return Err(CameraError::BadParam("max_dist_m"));
    }
    for candidate in candidates {
        if !candidate.pos_m.is_finite() {
            return Err(CameraError::BadPosition(candidate.id));
        }
    }
    // Cosine of the half angle; trig is presentation-only (never sim state),
    // and the same binary replays the same bytes twice.
    let cos_half = half_fov_rad.cos();
    let fwd = match forward.normalize() {
        Ok(unit) => unit,
        Err(_) => return Err(CameraError::BadForward),
    };
    let max_sq = max_dist_m * max_dist_m;
    let mut best: Option<(u8, f32)> = None;
    for candidate in candidates {
        if candidate.id == focus_id || !candidate.alive {
            continue;
        }
        let delta = candidate.pos_m - eye_m;
        let dist_sq = delta.len_sq();
        if dist_sq > max_sq {
            continue;
        }
        let cos_angle = if dist_sq == 0.0 {
            1.0
        } else {
            delta.dot(fwd) / dist_sq.sqrt()
        };
        if cos_angle < cos_half {
            continue;
        }
        let take = match best {
            None => true,
            Some((id, sq)) => {
                dist_sq < sq || (dist_sq.to_bits() == sq.to_bits() && candidate.id < id)
            }
        };
        if take {
            best = Some((candidate.id, dist_sq));
        }
    }
    Ok(best.map(|(id, _)| id))
}

/// Frame a focus fighter plus an optional second fighter: the look point is
/// their midpoint and the distance fits their separation inside the vertical
/// fov with the configured margin, clamped to `[min_dist_m, max_dist_m]`.
/// The eye sits above and behind the look point along the fixed boom shape.
pub fn frame_pair(
    focus_m: Vec3,
    other_m: Option<Vec3>,
    params: &DirectorParams,
) -> Result<CameraPose, CameraError> {
    params.validate()?;
    if !focus_m.is_finite() {
        return Err(CameraError::BadParam("focus_m"));
    }
    if let Some(other) = other_m {
        if !other.is_finite() {
            return Err(CameraError::BadParam("other_m"));
        }
    }
    let look_m = match other_m {
        Some(other) => (focus_m + other) * 0.5,
        None => focus_m,
    };
    let sep_m = match other_m {
        Some(other) => (other - focus_m).len(),
        None => 0.0,
    };
    let half_fov = params.fov_y_rad * 0.5;
    // Unclamped, both fighters sit inside the cone with the full margin
    // (the boom is at least as long as the distance, so the half-angle each
    // fighter subtends stays under half the fov).
    let need_m = (sep_m * 0.5 + params.fit_margin_m) / half_fov.tan();
    let dist_m = (params.base_dist_m + need_m).clamp(params.min_dist_m, params.max_dist_m);
    let eye_m = Vec3::new(
        look_m.x,
        look_m.y + params.boom_up * dist_m,
        look_m.z + params.boom_back * dist_m,
    );
    Ok(CameraPose { eye_m, look_m })
}

/// Fewest fighters one framing fit covers: a lone fighter keeps the pair
/// framing with no anchor, so the group fit starts at two.
pub const FRAMING_MIN_FIGHTERS: usize = 2;
/// Most fighters one framing fit covers: the fixed scratch below stages
/// exactly this many positions with no heap.
pub const FRAMING_MAX_FIGHTERS: usize = 4;

/// Frame two to four fighters in one view: the look point is their centroid
/// and the distance fits the farthest fighter inside the vertical cone with
/// the configured margin, clamped to `[min_dist_m, max_dist_m]`. The eye sits
/// above and behind the look point along the fixed boom shape, exactly like
/// the pair framing, so a pair frames where the director already framed it
/// and a wider bout pulls back instead of clipping discs at the frame edge.
/// Pure over positions: simulation state untouched, same slices give the same
/// pose twice, and shuffled slices give the same pose (the scratch copy sorts
/// into a total coordinate order before the mean, so container order never
/// leaks into receipts). The vertical cone is the exact bound for a square
/// reference viewport and a conservative bound for any wider one, so one fit
/// covers both. Errors: bad tuning names its row, a wrong fighter count names
/// `fighters`, a non-finite position names its fighter id. Budget: O(fighters)
/// time, one 48-byte stack scratch, zero heap bytes per call. Consumer: the
/// raster view (wired by a later row; this fit is pure presentation math).
pub fn frame_group(
    fighters: &[FighterView],
    params: &DirectorParams,
) -> Result<CameraPose, CameraError> {
    params.validate()?;
    if !(FRAMING_MIN_FIGHTERS..=FRAMING_MAX_FIGHTERS).contains(&fighters.len()) {
        return Err(CameraError::BadParam("fighters"));
    }
    for fighter in fighters {
        if !fighter.pos_m.is_finite() {
            return Err(CameraError::BadPosition(fighter.id));
        }
    }
    let mut scratch = [Vec3::ZERO; FRAMING_MAX_FIGHTERS];
    for (slot, fighter) in fighters.iter().enumerate() {
        scratch[slot] = fighter.pos_m;
    }
    let points = &mut scratch[..fighters.len()];
    points.sort_unstable_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then_with(|| a.y.total_cmp(&b.y))
            .then_with(|| a.z.total_cmp(&b.z))
    });
    let mut sum = Vec3::ZERO;
    let mut count = 0.0f32;
    for point in points.iter() {
        sum = sum + *point;
        count += 1.0;
    }
    let look_m = sum * (1.0 / count);
    let mut radius_m = 0.0f32;
    for point in points.iter() {
        let spread_m = (*point - look_m).len();
        if spread_m > radius_m {
            radius_m = spread_m;
        }
    }
    let half_fov = params.fov_y_rad * 0.5;
    // Unclamped, the farthest fighter sits inside the cone with the full
    // margin (the boom is at least as long as the distance, so the half-angle
    // each fighter subtends stays under half the fov).
    let need_m = (radius_m + params.fit_margin_m) / half_fov.tan();
    let dist_m = (params.base_dist_m + need_m).clamp(params.min_dist_m, params.max_dist_m);
    let eye_m = Vec3::new(
        look_m.x,
        look_m.y + params.boom_up * dist_m,
        look_m.z + params.boom_back * dist_m,
    );
    Ok(CameraPose { eye_m, look_m })
}

/// Off-screen indicator for one fighter outside the presented frame: the
/// fighter id plus the unit screen-space direction (`dir_x` right, `dir_y`
/// up) the raster view draws the edge arrow along. Pure data, never hashed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OffScreenArrow {
    /// Fighter slot id the arrow points toward.
    pub id: u8,
    /// Screen-right component of the unit direction, `-1.0..=1.0`.
    pub dir_x: f32,
    /// Screen-up component of the unit direction, `-1.0..=1.0`.
    pub dir_y: f32,
}

/// Camera right/up basis from a unit forward: right is forward crossed with
/// world up, up completes the frame. Falls back to `+x` right when looking
/// straight up or down (the cross degenerates there). Pure, no heap.
fn camera_basis(forward: Vec3) -> (Vec3, Vec3) {
    let flat_sq = forward.x * forward.x + forward.z * forward.z;
    let right = if flat_sq == 0.0 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        let inv = 1.0 / flat_sq.sqrt();
        Vec3::new(-forward.z * inv, 0.0, forward.x * inv)
    };
    let up = Vec3::new(
        right.y * forward.z - right.z * forward.y,
        right.z * forward.x - right.x * forward.z,
        right.x * forward.y - right.y * forward.x,
    );
    (right, up)
}

/// Framing status of one fighter in a presented frame: `None` when the
/// fighter sits inside the view cone (exactly what the raster view consumes;
/// the lock range rail is not visibility, so it never unframes), else the
/// off-screen arrow for the edge indicator. A fighter exactly on the eye is
/// framed (the camera sits inside it); a fighter behind the eye is out with
/// the unit screen direction toward it (straight up on the view axis). Pure
/// over the presented eye: same frame plus same fighter gives the same
/// answer twice, and per-fighter answers never depend on slice order.
/// Errors: bad tuning names its row, a non-finite fighter position names its
/// id, a non-finite frame eye reports [`CameraError::BadEye`], a non-finite
/// look point [`CameraError::BadForward`]. Budget: O(1) time, no heap.
/// Consumer: the raster view edge-arrow pass (wired by a later row; this
/// oracle is pure presentation math).
pub fn fighter_framing(
    frame: &CameraFrame,
    fighter: &FighterView,
    params: &DirectorParams,
) -> Result<Option<OffScreenArrow>, CameraError> {
    params.validate()?;
    if !fighter.pos_m.is_finite() {
        return Err(CameraError::BadPosition(fighter.id));
    }
    if !frame.eye_m.is_finite() {
        return Err(CameraError::BadEye);
    }
    if !frame.look_m.is_finite() {
        return Err(CameraError::BadForward);
    }
    let forward = (frame.look_m - frame.eye_m).normalize_or(Vec3::new(0.0, 0.0, -1.0));
    let delta = fighter.pos_m - frame.eye_m;
    let dist_sq = delta.len_sq();
    if dist_sq == 0.0 {
        return Ok(None);
    }
    // Trig is presentation-only (never sim state); the same binary replays
    // the same answer twice.
    let cos_limit = (params.fov_y_rad * 0.5).cos();
    let depth = delta.dot(forward);
    if depth > 0.0 && depth / dist_sq.sqrt() >= cos_limit {
        return Ok(None);
    }
    let (right, up) = camera_basis(forward);
    let along_x = delta.dot(right);
    let along_y = delta.dot(up);
    let axis_sq = along_x * along_x + along_y * along_y;
    let (dir_x, dir_y) = if axis_sq == 0.0 || !axis_sq.is_finite() {
        (0.0, 1.0)
    } else {
        let inv = 1.0 / axis_sq.sqrt();
        (along_x * inv, along_y * inv)
    };
    Ok(Some(OffScreenArrow {
        id: fighter.id,
        dir_x,
        dir_y,
    }))
}

/// Fighters outside the presented frame: the acceptance oracle for the
/// follow-camera row (zero means every fighter is framed; any remainder
/// carries an [`OffScreenArrow`] from [`fighter_framing`]). Every position is
/// validated even when an earlier fighter is out, so a NaN fails loudly
/// instead of hiding. Pure, O(fighters) time, no heap.
pub fn count_unframed(
    frame: &CameraFrame,
    fighters: &[FighterView],
    params: &DirectorParams,
) -> Result<usize, CameraError> {
    let mut missing = 0usize;
    for fighter in fighters {
        if fighter_framing(frame, fighter, params)?.is_some() {
            missing += 1;
        }
    }
    Ok(missing)
}

/// Edge arrows for one presented frame: writes one [`OffScreenArrow`] per
/// unframed fighter into `out` in slice order and returns the count written.
/// `out` must hold at least `fighters.len()` slots (one arrow per fighter at
/// most); a short buffer fails with [`CameraError::BadParam`] naming
/// `arrows` before any framing work, and a bad position fails naming its
/// fighter id with nothing written. Pure, slices in and out, no heap.
pub fn off_screen_arrows(
    frame: &CameraFrame,
    fighters: &[FighterView],
    params: &DirectorParams,
    out: &mut [OffScreenArrow],
) -> Result<usize, CameraError> {
    if out.len() < fighters.len() {
        return Err(CameraError::BadParam("arrows"));
    }
    for fighter in fighters {
        if !fighter.pos_m.is_finite() {
            return Err(CameraError::BadPosition(fighter.id));
        }
    }
    let mut count = 0usize;
    for fighter in fighters {
        if let Some(arrow) = fighter_framing(frame, fighter, params)? {
            out[count] = arrow;
            count += 1;
        }
    }
    Ok(count)
}

// Shake sampling hashes through the core owner `splitmix64`: integer-only
// and deterministic, so the same tick gives the same direction twice.

/// Map a hash to `[-1.0, 1.0]` from its top 53 bits.
fn unit_from_hash(hash: u64) -> f32 {
    const SCALE: f32 = 1.0 / 9_007_199_000_000_000.0;
    ((hash >> 11) as f32) * SCALE * 2.0 - 1.0
}

/// Shake offset in meters as a pure function of (`trauma_permille`, `tick`):
/// magnitude follows the squared trauma curve (`trauma^2 * max`), direction
/// comes from integer hashes of the tick. Zero trauma (or a non-positive or
/// non-finite cap) gives exactly [`Vec3::ZERO`]. Presentation only: the
/// offset moves the presented eye, never simulation state.
#[must_use]
pub fn shake_offset(trauma_permille: u32, tick: u32, max_m: f32) -> Vec3 {
    let clamped = trauma_permille.min(TRAUMA_SCALE);
    if clamped == 0 || !max_m.is_finite() || max_m <= 0.0 {
        return Vec3::ZERO;
    }
    let t = clamped as f32 / TRAUMA_SCALE as f32;
    let mag_m = t * t * max_m;
    const AXIS_XOR: [u64; 3] = [
        0x243F_6A88_85A3_08D3,
        0x4528_21E6_38D0_1377,
        0x2B05_2E46_2FB8_8C9E,
    ];
    let tick64 = u64::from(tick);
    Vec3::new(
        mag_m * unit_from_hash(splitmix64(tick64.wrapping_add(AXIS_XOR[0]))),
        mag_m * unit_from_hash(splitmix64(tick64.wrapping_add(AXIS_XOR[1]))),
        mag_m * unit_from_hash(splitmix64(tick64.wrapping_add(AXIS_XOR[2]))),
    )
}

/// Arena camera director: per-tick lock-on, integer trauma decay and framed
/// poses with pure shake. State is presentation only: the simulation records
/// hits elsewhere and the windowed leg feeds them here through
/// [`Director::add_trauma`]. Ticks run forward only; every update validates
/// every position and allocates nothing.
#[derive(Clone, Debug)]
pub struct Director {
    params: DirectorParams,
    focus_id: u8,
    lock: Option<u8>,
    trauma_permille: u32,
    last_tick: Option<u32>,
    last_eye: Vec3,
    last_forward: Vec3,
    has_pose: bool,
}

impl Director {
    /// Build a director following `focus_id` under `params`.
    pub fn new(focus_id: u8, params: DirectorParams) -> Result<Self, CameraError> {
        params.validate()?;
        Ok(Self {
            params,
            focus_id,
            lock: None,
            trauma_permille: 0,
            last_tick: None,
            last_eye: Vec3::ZERO,
            last_forward: Vec3::new(0.0, 0.0, -1.0),
            has_pose: false,
        })
    }

    /// Followed fighter id.
    #[must_use]
    pub fn focus_id(&self) -> u8 {
        self.focus_id
    }

    /// Active tuning table (copy).
    #[must_use]
    pub fn params(&self) -> DirectorParams {
        self.params
    }

    /// Swap tuning at a tick boundary (mode rule sets pick tables as data):
    /// validates first, so a bad table fails and keeps the old one with no
    /// state moved. Lock, trauma, cursor and pose carry over; subsequent
    /// frames are deterministic for the same retune tick. Pure booking.
    pub fn retune(&mut self, params: DirectorParams) -> Result<(), CameraError> {
        params.validate()?;
        self.params = params;
        Ok(())
    }

    /// Current lock-on target, if any fighter was in frustum last update.
    #[must_use]
    pub fn lock(&self) -> Option<u8> {
        self.lock
    }

    /// Current trauma permille after decay.
    #[must_use]
    pub fn trauma_permille(&self) -> u32 {
        self.trauma_permille
    }

    /// Record a hit: saturating add, clamped to `0..=1000`. Presentation
    /// bookkeeping only; the sim records the hit itself.
    pub fn add_trauma(&mut self, amount_permille: u32) {
        self.trauma_permille = self
            .trauma_permille
            .saturating_add(amount_permille)
            .min(TRAUMA_SCALE);
    }

    /// Present one tick: decay trauma by the elapsed ticks, re-select the
    /// lock target from the last pose, frame the whole bout when two to four
    /// fighters are up (pair framing otherwise), then shake the eye. A
    /// clamped rail can still leave a fighter out of the cone; the frame
    /// stays honest and [`off_screen_arrows`] names the remainder. The shaken
    /// eye is reported but never stored: the next tick selects from the
    /// unshaken pose, so shake cannot steer.
    pub fn update(
        &mut self,
        fighters: &[FighterView],
        tick: u32,
    ) -> Result<CameraFrame, CameraError> {
        if let Some(last) = self.last_tick {
            if tick < last {
                return Err(CameraError::TickWentBackwards { last, next: tick });
            }
            let elapsed = tick - last;
            self.trauma_permille = self
                .trauma_permille
                .saturating_sub(elapsed.saturating_mul(self.params.decay_permille_per_tick));
        }
        let mut focus_m: Option<Vec3> = None;
        for fighter in fighters {
            if !fighter.pos_m.is_finite() {
                return Err(CameraError::BadPosition(fighter.id));
            }
            if fighter.id == self.focus_id && focus_m.is_none() {
                focus_m = Some(fighter.pos_m);
            }
        }
        let focus_m = match focus_m {
            Some(pos) => pos,
            None => return Err(CameraError::FocusMissing(self.focus_id)),
        };
        let (eye_hint, fwd_hint) = if self.has_pose {
            (self.last_eye, self.last_forward)
        } else {
            let eye = Vec3::new(
                focus_m.x,
                focus_m.y + self.params.boom_up * self.params.base_dist_m,
                focus_m.z + self.params.boom_back * self.params.base_dist_m,
            );
            let fwd = (focus_m - eye).normalize_or(Vec3::new(0.0, 0.0, -1.0));
            (eye, fwd)
        };
        let lock = select_lock_target(
            eye_hint,
            fwd_hint,
            self.params.fov_y_rad * 0.5,
            self.params.max_dist_m,
            self.focus_id,
            fighters,
        )?;
        // Framing fit: a two-to-four-fighter bout frames the whole group
        // (centroid plus farthest-fighter radius, the fallen included) so a
        // wide bout pulls back instead of clipping discs at the frame edge;
        // any other count keeps the pair framing (focus plus the lock target,
        // else the nearest alive rival so the frame can only gain a target,
        // never lose one).
        let pose = if (FRAMING_MIN_FIGHTERS..=FRAMING_MAX_FIGHTERS).contains(&fighters.len()) {
            frame_group(fighters, &self.params)?
        } else {
            let mut anchor: Option<Vec3> = None;
            let mut anchor_sq = f32::INFINITY;
            let mut anchor_id = u8::MAX;
            for fighter in fighters {
                let candidate = match lock {
                    Some(id) if fighter.id == id => Some(fighter.pos_m),
                    _ => None,
                };
                if let Some(pos) = candidate {
                    anchor = Some(pos);
                    break;
                }
                if fighter.id == self.focus_id || !fighter.alive {
                    continue;
                }
                let sq = (fighter.pos_m - focus_m).len_sq();
                if sq < anchor_sq || (sq.to_bits() == anchor_sq.to_bits() && fighter.id < anchor_id)
                {
                    anchor = Some(fighter.pos_m);
                    anchor_sq = sq;
                    anchor_id = fighter.id;
                }
            }
            frame_pair(focus_m, anchor, &self.params)?
        };
        let forward = (pose.look_m - pose.eye_m).normalize_or(Vec3::new(0.0, 0.0, -1.0));
        let shake_m = shake_offset(self.trauma_permille, tick, self.params.max_shake_m);
        let frame = CameraFrame {
            tick,
            eye_m: pose.eye_m + shake_m,
            look_m: pose.look_m,
            shake_m,
            lock,
            trauma_permille: self.trauma_permille,
        };
        self.lock = lock;
        self.last_eye = pose.eye_m;
        self.last_forward = forward;
        self.has_pose = true;
        self.last_tick = Some(tick);
        Ok(frame)
    }
}

// ---------------------------------------------------------------------------
// Fixture rule sets: the mode framework boots fixture rule
// sets through this loader with no engine edits. Rules arrive as data text
// in two schemas: the tag schema (`fixtures/games/second-game/rules.json`:
// tag mode, seed, ticks, runners, goal disc, follow-camera table, ordered
// input script) and the traversal-course schema
// (`fixtures/games/traversal-course/rules.json`: course mode, run speed,
// ledges, chase-camera table, run tweaks). Both validate at the boundary
// with typed errors, run headless in [`FixtureSession`] and replay with
// first-divergence naming. The camera tables are presentation only: two
// tables over one tape share every sim hash but frame differently.
// ---------------------------------------------------------------------------

/// Fixture rule-set version this loader boots (the tag-schema authority).
pub const FIXTURE_RULES_VERSION: u16 = 2;
/// Traversal-course rule-set version this loader boots (the GE1 authority).
pub const COURSE_RULES_VERSION: u16 = 3;
/// Runners per course, `1..=COURSE_MAX_RUNNERS` (GE1 mirror).
pub const COURSE_MAX_RUNNERS: u8 = 2;
/// Ledges per course, `1..=COURSE_MAX_LEDGES` (GE1 mirror).
pub const COURSE_MAX_LEDGES: usize = 16;
/// Course cruise-speed bounds, m/s (GE1 mirror).
pub const COURSE_SPEED_MIN_MPS: f32 = 0.5;
/// Course cruise-speed bounds, m/s (GE1 mirror).
pub const COURSE_SPEED_MAX_MPS: f32 = 10.0;
/// Course ledge x bounds, meters (GE1 mirror).
pub const COURSE_X_MIN_M: f32 = -50.0;
/// Course ledge x bounds, meters (GE1 mirror).
pub const COURSE_X_MAX_M: f32 = 120.0;
/// Course ledge top bounds, millimetres (GE1 mirror).
pub const COURSE_TOP_MIN_MM: f32 = 100.0;
/// Course ledge top bounds, millimetres (GE1 mirror).
pub const COURSE_TOP_MAX_MM: f32 = 3500.0;
/// Course chase-camera distance/height bounds, meters (GE1 mirror).
pub const COURSE_CAM_MIN_M: f32 = 1.0;
/// Course chase-camera distance/height bounds, meters (GE1 mirror).
pub const COURSE_CAM_MAX_M: f32 = 20.0;
/// Course spawn box half-extent, meters: the course schema carries no
/// playfield (GE1 owns bounds), so course runners seed-expand into this
/// fixed box instead of an extent-derived one.
pub const COURSE_SPAWN_LIMIT_M: f32 = 10.0;
/// Runners per fixture match, `2..=FIXTURE_MAX_RUNNERS`.
pub const FIXTURE_MAX_RUNNERS: u8 = 8;
/// Bound fixture ticks per match.
pub const FIXTURE_MAX_TICKS: u32 = 10_000;
/// Bound scripted moves per fixture match.
pub const FIXTURE_MAX_INPUTS: usize = 512;
/// Fastest legal fixture velocity assignment, m/s.
pub const FIXTURE_MAX_SPEED_MPS: f32 = 20.0;
/// Fixture fixed step in seconds (60 Hz, the engine tick).
pub const FIXTURE_DT_S: f32 = 1.0 / 60.0;
/// Goal disc radius bounds, meters.
pub const FIXTURE_GOAL_RADIUS_MIN_M: f32 = 0.5;
/// Goal disc radius bounds, meters.
pub const FIXTURE_GOAL_RADIUS_MAX_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
pub const FIXTURE_EXTENT_MIN_M: f32 = 5.0;
/// Playfield half-extent bounds, meters.
pub const FIXTURE_EXTENT_MAX_M: f32 = 50.0;
/// Follow-camera height bounds, meters.
pub const FIXTURE_CAM_HEIGHT_MIN_M: f32 = 2.0;
/// Follow-camera height bounds, meters.
pub const FIXTURE_CAM_HEIGHT_MAX_M: f32 = 30.0;
/// Follow-camera lead bounds, meters.
pub const FIXTURE_CAM_LEAD_MAX_M: f32 = 10.0;

/// Fixture rule-set failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FixtureError {
    /// Rules version is not [`FIXTURE_RULES_VERSION`], version carried.
    BadVersion(u16),
    /// Mode string is not `"tag"` (the only rule set this loader boots).
    UnknownMode,
    /// Tick count `0` or above [`FIXTURE_MAX_TICKS`].
    BadTicks(u32),
    /// Runner count outside `2..=FIXTURE_MAX_RUNNERS`.
    BadRunners(u8),
    /// Goal disc outside the playfield or with a bad radius.
    BadGoal,
    /// Half-extent NaN or outside `5.0..=50.0` meters.
    BadExtent,
    /// Course cruise speed NaN or outside `0.5..=10.0` m/s.
    BadSpeed,
    /// Course ledge count outside `1..=COURSE_MAX_LEDGES`, count carried.
    BadLedges(usize),
    /// More than [`COURSE_MAX_LEDGES`] ledges while scanning.
    TooManyLedges,
    /// Course ledge out of bounds, unordered or with a bad top, index carried.
    BadLedge(usize),
    /// Follow-camera row outside its range, row name carried.
    BadCamera(&'static str),
    /// Director table refused while booting a mode, reason carried.
    BadCameraTable(&'static str),
    /// More than [`FIXTURE_MAX_INPUTS`] moves.
    TooManyInputs,
    /// Tick at or past the match end, or body past the runners.
    OutOfRange,
    /// Non-finite or over-speed velocity.
    BadVelocity,
    /// Moves not strictly ordered by `(tick, body)`.
    Unordered,
    /// Input script is empty (schema parity with the tag-schema authority).
    EmptyScript,
    /// Needed key absent from the rules text, key name carried.
    MissingKey(&'static str),
    /// Needed value would not parse, key name carried.
    BadValue(&'static str),
    /// Malformed rules structure, site carried (`camera`, `inputs`, `mode`).
    BadRules(&'static str),
    /// Replay checkpoint count does not match the config.
    Count,
    /// First mismatching zero-based tick plus both hashes.
    Divergence {
        /// Zero-based tick of the first mismatch.
        tick: u32,
        /// Hash the tape recorded.
        expected: u64,
        /// Hash the re-run produced.
        actual: u64,
    },
    /// Win outcome differs at the named tick.
    Win(u32),
}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => write!(f, "fixture rules version {version}"),
            Self::UnknownMode => write!(f, "fixture mode must be \"tag\""),
            Self::BadTicks(ticks) => write!(f, "bad fixture ticks ({ticks})"),
            Self::BadRunners(runners) => write!(f, "bad fixture runners ({runners})"),
            Self::BadGoal => write!(f, "bad fixture goal disc"),
            Self::BadExtent => write!(f, "bad fixture half extent"),
            Self::BadSpeed => write!(f, "bad course run speed"),
            Self::BadLedges(count) => write!(f, "bad course ledge count ({count})"),
            Self::TooManyLedges => write!(f, "too many course ledges"),
            Self::BadLedge(index) => write!(f, "bad course ledge ({index})"),
            Self::BadCamera(row) => write!(f, "bad fixture camera row ({row})"),
            Self::BadCameraTable(reason) => write!(f, "bad fixture camera table ({reason})"),
            Self::TooManyInputs => write!(f, "too many fixture inputs"),
            Self::OutOfRange => write!(f, "fixture input tick or body out of range"),
            Self::BadVelocity => write!(f, "fixture velocity not finite or over speed"),
            Self::Unordered => write!(f, "fixture inputs not ordered by (tick, body)"),
            Self::EmptyScript => write!(f, "fixture inputs array is empty"),
            Self::MissingKey(key) => write!(f, "fixture rules missing key ({key})"),
            Self::BadValue(key) => write!(f, "fixture rules bad value for ({key})"),
            Self::BadRules(site) => write!(f, "fixture rules malformed ({site})"),
            Self::Count => write!(f, "fixture checkpoint count does not match ticks"),
            Self::Divergence {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "fixture replay diverged at tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
            Self::Win(tick) => write!(f, "fixture win outcome differs at tick {tick}"),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Follow-camera parameters as data (presentation only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FixtureCamera {
    /// Camera height over the plane in meters, `2.0..=30.0`.
    pub height_m: f32,
    /// Lead toward the current leader in meters, `0.0..=10.0`.
    pub lead_m: f32,
}

impl FixtureCamera {
    /// Range-check the camera data; a bad table fails before use. Pure.
    pub fn validate(self) -> Result<(), FixtureError> {
        if !self.height_m.is_finite()
            || !(FIXTURE_CAM_HEIGHT_MIN_M..=FIXTURE_CAM_HEIGHT_MAX_M).contains(&self.height_m)
        {
            return Err(FixtureError::BadCamera("height_m"));
        }
        if !self.lead_m.is_finite() || !(0.0..=FIXTURE_CAM_LEAD_MAX_M).contains(&self.lead_m) {
            return Err(FixtureError::BadCamera("lead_m"));
        }
        Ok(())
    }

    /// Follow-center over planar runner positions: mean plus a lead toward
    /// the goal-nearest runner. Pure presentation; never hashed, never sim
    /// input. Uses one `sqrt` for the lead direction under the same
    /// presentation-only license as the arena director's trig; same inputs
    /// give the same center twice. Budget: O(runners) time, no heap.
    #[must_use]
    pub fn frame(&self, positions: &[[f32; 2]], goal: [f32; 2]) -> [f32; 3] {
        let count = positions.len().max(1) as f32;
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        for pos in positions {
            sx += pos[0];
            sy += pos[1];
        }
        let avg = [sx / count, sy / count];
        let mut leader = avg;
        let mut best = f32::INFINITY;
        for pos in positions {
            let dx = pos[0] - goal[0];
            let dy = pos[1] - goal[1];
            let d2 = dx * dx + dy * dy;
            if d2 < best {
                best = d2;
                leader = *pos;
            }
        }
        let (ox, oy) = (leader[0] - avg[0], leader[1] - avg[1]);
        let len2 = ox * ox + oy * oy;
        if len2 <= 1e-12 || self.lead_m == 0.0 {
            [avg[0], avg[1], self.height_m]
        } else {
            let len = len2.sqrt();
            [
                avg[0] + self.lead_m * ox / len,
                avg[1] + self.lead_m * oy / len,
                self.height_m,
            ]
        }
    }
}

/// One fixture velocity assignment applied before the named zero-based tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FixtureInput {
    /// Application tick, strictly below the match ticks.
    pub tick: u32,
    /// Runner slot below the match runners.
    pub body: u8,
    /// Horizontal velocity in m/s, `|v| <= 20.0`.
    pub vx: f32,
    /// Vertical velocity in m/s, `|v| <= 20.0`.
    pub vy: f32,
}

/// Validated fixture match rules as data: everything a mode boot needs, no
/// code fork per rule set. Built only by [`FixtureRules::parse`], so every
/// instance is already range-checked.
#[derive(Clone, PartialEq, Debug)]
pub struct FixtureRules {
    version: u16,
    seed: u64,
    ticks: u32,
    runners: u8,
    goal: [f32; 2],
    goal_radius_m: f32,
    half_extent_m: f32,
    camera: FixtureCamera,
    inputs: Vec<FixtureInput>,
}

/// Parse one JSON number after `"key":` (whitespace-tolerant, std only; the
/// headless default stays std-only, so no serde here).
fn fixture_num_after(text: &str, key: &'static str) -> Result<f64, FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let colon = text[start..].find(':').ok_or(FixtureError::BadValue(key))?;
    let rest = text[start + colon + 1..].trim_start();
    let mut end = 0usize;
    for (index, ch) in rest.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    rest[..end]
        .parse::<f64>()
        .map_err(|_| FixtureError::BadValue(key))
}

/// Parse one JSON `[a, b]` pair after `"key":`.
fn fixture_pair_after(text: &str, key: &'static str) -> Result<[f64; 2], FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let open = text[start..].find('[').ok_or(FixtureError::BadRules(key))?;
    let rest = &text[start + open + 1..];
    let close = rest.find(']').ok_or(FixtureError::BadRules(key))?;
    let mut values = [0.0f64; 2];
    let mut count = 0usize;
    for part in rest[..close].split(',') {
        if count >= 2 {
            return Err(FixtureError::BadValue(key));
        }
        values[count] = part
            .trim()
            .parse::<f64>()
            .map_err(|_| FixtureError::BadValue(key))?;
        count += 1;
    }
    if count != 2 {
        return Err(FixtureError::BadValue(key));
    }
    Ok(values)
}

impl FixtureRules {
    /// Parse plus validate one tag-schema fixture rule-set text (the
    /// `fixtures/games/second-game/rules.json` schema). Unknown keys are
    /// ignored so newer tables still boot while the rows this loader needs
    /// are present; every needed row is range-checked and the input script
    /// must be non-empty and strictly ordered. Budget: O(inputs) time, one
    /// input `Vec` at the load boundary (never per tick).
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        let version = fixture_num_after(text, "version")? as u16;
        if version != FIXTURE_RULES_VERSION {
            return Err(FixtureError::BadVersion(version));
        }
        let mode_start = text
            .find("\"mode\"")
            .ok_or(FixtureError::MissingKey("mode"))?;
        let mode_colon = text[mode_start..]
            .find(':')
            .ok_or(FixtureError::BadRules("mode"))?;
        let mode_rest = text[mode_start + mode_colon + 1..].trim_start();
        if !mode_rest.starts_with("\"tag\"") {
            return Err(FixtureError::UnknownMode);
        }
        let seed = fixture_num_after(text, "seed")? as u64;
        let ticks = fixture_num_after(text, "ticks")? as u32;
        let runners = fixture_num_after(text, "runners")? as u8;
        let goal_pair = fixture_pair_after(text, "goal_m")?;
        let goal = [goal_pair[0] as f32, goal_pair[1] as f32];
        let goal_radius_m = fixture_num_after(text, "goal_radius_m")? as f32;
        let half_extent_m = fixture_num_after(text, "half_extent_m")? as f32;
        check_fixture_config(ticks, runners, goal, goal_radius_m, half_extent_m)?;
        let camera_block = text
            .find("\"camera\"")
            .and_then(|start| {
                text[start..]
                    .find('{')
                    .map(|open| &text[start + open + 1..])
            })
            .ok_or(FixtureError::MissingKey("camera"))?;
        let camera_end = camera_block
            .find('}')
            .ok_or(FixtureError::BadRules("camera"))?;
        let camera_text = &camera_block[..camera_end];
        let camera = FixtureCamera {
            height_m: fixture_num_after(camera_text, "height_m")? as f32,
            lead_m: fixture_num_after(camera_text, "lead_m")? as f32,
        };
        camera.validate()?;
        let inputs_start = text
            .find("\"inputs\"")
            .ok_or(FixtureError::MissingKey("inputs"))?;
        let inputs_open = text[inputs_start..]
            .find('[')
            .ok_or(FixtureError::BadRules("inputs"))?;
        let inputs_text = &text[inputs_start + inputs_open + 1..];
        let mut inputs = Vec::new();
        let mut cursor = 0usize;
        while let Some(open) = inputs_text[cursor..].find('{') {
            let obj_start = cursor + open;
            let obj_end = inputs_text[obj_start..]
                .find('}')
                .ok_or(FixtureError::BadRules("inputs"))?;
            let obj = &inputs_text[obj_start..obj_start + obj_end + 1];
            inputs.push(FixtureInput {
                tick: fixture_num_after(obj, "tick")? as u32,
                body: fixture_num_after(obj, "body")? as u8,
                vx: fixture_num_after(obj, "vx")? as f32,
                vy: fixture_num_after(obj, "vy")? as f32,
            });
            cursor = obj_start + obj_end + 1;
            if inputs.len() > FIXTURE_MAX_INPUTS {
                return Err(FixtureError::TooManyInputs);
            }
        }
        if inputs.is_empty() {
            return Err(FixtureError::EmptyScript);
        }
        check_fixture_inputs(ticks, runners, &inputs)?;
        Ok(Self {
            version,
            seed,
            ticks,
            runners,
            goal,
            goal_radius_m,
            half_extent_m,
            camera,
            inputs,
        })
    }
}

/// Range-check one fixture match config; NaN or out-of-range fails loudly.
fn check_fixture_config(
    ticks: u32,
    runners: u8,
    goal: [f32; 2],
    goal_radius_m: f32,
    half_extent_m: f32,
) -> Result<(), FixtureError> {
    if ticks == 0 || ticks > FIXTURE_MAX_TICKS {
        return Err(FixtureError::BadTicks(ticks));
    }
    if !(2..=FIXTURE_MAX_RUNNERS).contains(&runners) {
        return Err(FixtureError::BadRunners(runners));
    }
    if !half_extent_m.is_finite()
        || !(FIXTURE_EXTENT_MIN_M..=FIXTURE_EXTENT_MAX_M).contains(&half_extent_m)
    {
        return Err(FixtureError::BadExtent);
    }
    if !goal_radius_m.is_finite()
        || !(FIXTURE_GOAL_RADIUS_MIN_M..=FIXTURE_GOAL_RADIUS_MAX_M).contains(&goal_radius_m)
        || !goal[0].is_finite()
        || !goal[1].is_finite()
        || goal[0].abs() > half_extent_m
        || goal[1].abs() > half_extent_m
    {
        return Err(FixtureError::BadGoal);
    }
    Ok(())
}

/// Validate one fixture move list: bounds, finite in-speed velocities and
/// strict `(tick, body)` order.
fn check_fixture_inputs(
    ticks: u32,
    runners: u8,
    inputs: &[FixtureInput],
) -> Result<(), FixtureError> {
    if inputs.len() > FIXTURE_MAX_INPUTS {
        return Err(FixtureError::TooManyInputs);
    }
    for input in inputs {
        if input.tick >= ticks || input.body >= runners {
            return Err(FixtureError::OutOfRange);
        }
        if !input.vx.is_finite()
            || !input.vy.is_finite()
            || input.vx.abs() > FIXTURE_MAX_SPEED_MPS
            || input.vy.abs() > FIXTURE_MAX_SPEED_MPS
        {
            return Err(FixtureError::BadVelocity);
        }
    }
    if inputs
        .windows(2)
        .any(|pair| (pair[0].tick, pair[0].body) >= (pair[1].tick, pair[1].body))
    {
        return Err(FixtureError::Unordered);
    }
    Ok(())
}

/// One SplitMix64 step for non-aliasing fixture seed expansion behind the
/// core owner: the output equals `splitmix64` of the pre-step state, so
/// neighboring seeds draw disjoint streams and bytes don't move.
fn fixture_splitmix_next(state: &mut u64) -> u64 {
    let word = splitmix64(*state);
    *state = state.wrapping_add(GOLDEN_GAMMA);
    word
}

/// Map one stream word uniformly into `[0, 1]` as `f32` (the tag-schema recipe;
/// same binary gives the same spawns twice).
fn fixture_unit_f32(word: u64) -> f32 {
    (word as f64 / u64::MAX as f64) as f32
}

/// Normalize signed zero so `0.0` and `-0.0` hash identically.
fn fixture_canon_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Multiplier of the fixture hashes. Recorded fixture tapes pin hashes made
/// with it, so it stays distinct from the standard FNV-1a prime
/// (`engine_core::hash::FNV64_PRIME`); changing it re-records every tape.
const FIXTURE_PRIME: u64 = 0x1000_0000_01b3;

/// Byte fold (xor, then multiply by [`FIXTURE_PRIME`]) into the running
/// fixture hash.
fn fixture_hash_write(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FIXTURE_PRIME);
    }
}

/// Seed-expanded start positions: two stream words per runner mapped into
/// `[-limit, limit]`; yaw-free planar spawns in index order.
fn fixture_spawns(seed: u64, runners: u8, limit: f32) -> [[f32; 2]; 8] {
    let mut state = seed;
    let mut out = [[0.0f32; 2]; 8];
    for slot in out.iter_mut().take(usize::from(runners)) {
        let x = fixture_unit_f32(fixture_splitmix_next(&mut state)) * 2.0 - 1.0;
        let y = fixture_unit_f32(fixture_splitmix_next(&mut state)) * 2.0 - 1.0;
        slot[0] = fixture_canon_zero(x * limit);
        slot[1] = fixture_canon_zero(y * limit);
    }
    out
}

/// Checkpoint after one fixture tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FixtureCheckpoint {
    /// Zero-based fixed tick.
    pub tick: u32,
    /// State hash after the tick.
    pub hash: u64,
}

/// Recorded fixture match: one checkpoint per tick plus the sticky tag win.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FixtureTape {
    /// One post-step checkpoint per tick, in tick order.
    pub checkpoints: Vec<FixtureCheckpoint>,
    /// First tick a runner entered the goal disc, if any.
    pub win_tick: Option<u32>,
    /// Lowest-index runner inside the disc on `win_tick`, if any.
    pub winner: Option<u8>,
}

impl FixtureTape {
    /// Digest over checkpoints plus the win outcome: equal tapes give equal
    /// digests twice; different rules give different digests. Pure.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = FNV64_OFFSET;
        for point in &self.checkpoints {
            fixture_hash_write(&mut hash, &point.tick.to_le_bytes());
            fixture_hash_write(&mut hash, &point.hash.to_le_bytes());
        }
        let win_bits = match (self.win_tick, self.winner) {
            (Some(tick), Some(winner)) => {
                u64::from(tick).wrapping_mul(FIXTURE_PRIME) ^ u64::from(winner).wrapping_add(1)
            }
            _ => 0,
        };
        fixture_hash_write(&mut hash, &win_bits.to_le_bytes());
        hash
    }

    /// Replay a tape under its tag rules: re-simulate and stop at the first
    /// divergent tick, a count mismatch or a changed win. Pure booking over
    /// a fresh deterministic run.
    pub fn replay(&self, rules: &FixtureRules) -> Result<(), FixtureError> {
        if self.checkpoints.len() != rules.ticks as usize {
            return Err(FixtureError::Count);
        }
        let fresh = FixtureSession::boot(rules).run_to_tape();
        self.check_against(rules.ticks, &fresh)
    }

    /// Replay a tape under its course rules: same gates over a course boot
    /// (no goal disc, so the win arms only when both runs agree it is set).
    pub fn replay_course(&self, rules: &CourseRules) -> Result<(), FixtureError> {
        if self.checkpoints.len() != rules.ticks as usize {
            return Err(FixtureError::Count);
        }
        let fresh = FixtureSession::boot_course(rules).run_to_tape();
        self.check_against(rules.ticks, &fresh)
    }

    /// Shared replay tail: first-divergent-tick naming plus the win gate.
    fn check_against(&self, ticks: u32, fresh: &FixtureTape) -> Result<(), FixtureError> {
        for (want, got) in self.checkpoints.iter().zip(fresh.checkpoints.iter()) {
            if want != got {
                return Err(FixtureError::Divergence {
                    tick: want.tick,
                    expected: want.hash,
                    actual: got.hash,
                });
            }
        }
        if self.win_tick != fresh.win_tick || self.winner != fresh.winner {
            return Err(FixtureError::Win(self.win_tick.unwrap_or(ticks)));
        }
        Ok(())
    }
}

/// Live fixture match: planar runners with fixed-dt integration, a sticky
/// goal-disc win for tag rule sets (courses run open, no disc) and an FNV
/// hash chain over every future-affecting byte. Authoritative math is
/// `+ - * /` and comparisons only (no trig, no wall clock); positions stay
/// bounded (`|pos| <= extent + speed * ticks * dt`, far below overflow), so
/// every step is total. Built only from validated rules, hence infallible
/// construction.
pub struct FixtureSession {
    ticks: u32,
    runners: u8,
    goal: [f32; 2],
    goal_radius_m: f32,
    /// False for course rule sets: no goal disc, [`FixtureSession::tag_winner`]
    /// stays `None` (the goal fields below are inert then).
    has_goal: bool,
    inputs: Vec<FixtureInput>,
    cursor: usize,
    tick: u32,
    pos: [[f32; 2]; 8],
    vel: [[f32; 2]; 8],
    clock: u64,
    context: u64,
    checkpoints: Vec<FixtureCheckpoint>,
    win_tick: Option<u32>,
    winner: Option<u8>,
}

impl FixtureSession {
    /// Validated constructor: spawns, hash context and sized checkpoint log.
    /// Budget: O(runners + inputs) time, two small `Vec`s at the boot
    /// boundary (never per tick).
    #[must_use]
    pub fn boot(rules: &FixtureRules) -> Self {
        let mut context = FNV64_OFFSET;
        fixture_hash_write(&mut context, &rules.version.to_le_bytes());
        fixture_hash_write(&mut context, &rules.seed.to_le_bytes());
        fixture_hash_write(&mut context, &rules.ticks.to_le_bytes());
        fixture_hash_write(&mut context, &[rules.runners]);
        for axis in rules.goal {
            fixture_hash_write(&mut context, &axis.to_bits().to_le_bytes());
        }
        fixture_hash_write(&mut context, &rules.goal_radius_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, &rules.half_extent_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, b"tag");
        for input in &rules.inputs {
            fixture_hash_write(&mut context, &input.tick.to_le_bytes());
            fixture_hash_write(&mut context, &[input.body]);
            fixture_hash_write(&mut context, &input.vx.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &input.vy.to_bits().to_le_bytes());
        }
        Self {
            ticks: rules.ticks,
            runners: rules.runners,
            goal: rules.goal,
            goal_radius_m: rules.goal_radius_m,
            has_goal: true,
            inputs: rules.inputs.clone(),
            cursor: 0,
            tick: 0,
            pos: fixture_spawns(rules.seed, rules.runners, rules.half_extent_m - 0.4),
            vel: [[0.0; 2]; 8],
            clock: mix64(rules.seed),
            context,
            checkpoints: Vec::with_capacity(rules.ticks as usize),
            win_tick: None,
            winner: None,
        }
    }

    /// Course constructor: same planar script run with no goal disc; cruise
    /// speed, ledges and the chase table fold into the context as data, so
    /// different furniture gives different receipts. Vault and ledge
    /// resolution belong to the GE1 sim, not this loader: the course run
    /// proves the rule set loads, the camera tracks and the tape replays.
    /// Budget: O(runners + moves) time, two small `Vec`s at the boot
    /// boundary (never per tick).
    #[must_use]
    pub fn boot_course(rules: &CourseRules) -> Self {
        let mut context = FNV64_OFFSET;
        fixture_hash_write(&mut context, &rules.version.to_le_bytes());
        fixture_hash_write(&mut context, &rules.seed.to_le_bytes());
        fixture_hash_write(&mut context, &rules.ticks.to_le_bytes());
        fixture_hash_write(&mut context, &[rules.runners]);
        fixture_hash_write(&mut context, &rules.run_speed_mps.to_bits().to_le_bytes());
        fixture_hash_write(
            &mut context,
            &rules.camera.distance_m.to_bits().to_le_bytes(),
        );
        fixture_hash_write(&mut context, &rules.camera.height_m.to_bits().to_le_bytes());
        fixture_hash_write(&mut context, b"course");
        fixture_hash_write(&mut context, &(rules.ledges.len() as u64).to_le_bytes());
        for ledge in &rules.ledges {
            fixture_hash_write(&mut context, &ledge.x_m.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &ledge.top_m.to_bits().to_le_bytes());
        }
        for input in &rules.moves {
            fixture_hash_write(&mut context, &input.tick.to_le_bytes());
            fixture_hash_write(&mut context, &[input.body]);
            fixture_hash_write(&mut context, &input.vx.to_bits().to_le_bytes());
            fixture_hash_write(&mut context, &input.vy.to_bits().to_le_bytes());
        }
        Self {
            ticks: rules.ticks,
            runners: rules.runners,
            goal: [0.0, 0.0],
            goal_radius_m: 0.0,
            has_goal: false,
            inputs: rules.moves.clone(),
            cursor: 0,
            tick: 0,
            pos: fixture_spawns(rules.seed, rules.runners, COURSE_SPAWN_LIMIT_M),
            vel: [[0.0; 2]; 8],
            clock: mix64(rules.seed),
            context,
            checkpoints: Vec::with_capacity(rules.ticks as usize),
            win_tick: None,
            winner: None,
        }
    }

    /// Zero-based fixed tick about to simulate.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.tick >= self.ticks
    }

    /// Planar runner positions for the presentation cameras.
    #[must_use]
    pub fn positions(&self) -> &[[f32; 2]] {
        &self.pos[..usize::from(self.runners)]
    }

    /// Goal disc center `[x, y]` in meters.
    #[must_use]
    pub fn goal(&self) -> [f32; 2] {
        self.goal
    }

    /// First runner inside the goal disc after this tick, lowest index wins
    /// ties by strict `<` over squared distance (no square root in sim).
    /// Course runs carry no disc and always answer `None`.
    fn tag_winner(&self) -> Option<u8> {
        if !self.has_goal {
            return None;
        }
        let radius2 = self.goal_radius_m * self.goal_radius_m;
        let mut best: Option<u8> = None;
        let mut best_d2 = f32::INFINITY;
        for (index, pos) in self.pos.iter().enumerate().take(usize::from(self.runners)) {
            let dx = pos[0] - self.goal[0];
            let dy = pos[1] - self.goal[1];
            let d2 = dx * dx + dy * dy;
            if d2 <= radius2 && d2 < best_d2 {
                best_d2 = d2;
                best = Some(index as u8);
            }
        }
        best
    }

    /// One fixed tick: apply this tick's moves, integrate both planes,
    /// record the sticky tag, fold every future-affecting byte into the
    /// checkpoint. No heap growth: fixed arrays plus a pre-sized log.
    fn step(&mut self) {
        while self.cursor < self.inputs.len() && self.inputs[self.cursor].tick == self.tick {
            let input = self.inputs[self.cursor];
            let slot = usize::from(input.body);
            self.vel[slot] = [input.vx, input.vy];
            self.cursor += 1;
        }
        for index in 0..usize::from(self.runners) {
            self.pos[index][0] += self.vel[index][0] * FIXTURE_DT_S;
            self.pos[index][1] += self.vel[index][1] * FIXTURE_DT_S;
        }
        if self.win_tick.is_none() {
            if let Some(winner) = self.tag_winner() {
                self.win_tick = Some(self.tick);
                self.winner = Some(winner);
            }
        }
        let win_bits = match (self.win_tick, self.winner) {
            (Some(tick), Some(winner)) => {
                u64::from(tick).wrapping_mul(FIXTURE_PRIME) ^ u64::from(winner).wrapping_add(1)
            }
            _ => 0,
        };
        let mut step_hash = FNV64_OFFSET;
        fixture_hash_write(&mut step_hash, &self.tick.to_le_bytes());
        for index in 0..usize::from(self.runners) {
            for value in [
                self.pos[index][0],
                self.pos[index][1],
                self.vel[index][0],
                self.vel[index][1],
            ] {
                fixture_hash_write(
                    &mut step_hash,
                    &fixture_canon_zero(value).to_bits().to_le_bytes(),
                );
            }
        }
        fixture_hash_write(&mut step_hash, &win_bits.to_le_bytes());
        fixture_hash_write(&mut step_hash, &self.context.to_le_bytes());
        self.clock = mix64(self.clock ^ step_hash);
        self.checkpoints.push(FixtureCheckpoint {
            tick: self.tick,
            hash: self.clock,
        });
        self.tick += 1;
    }

    /// Drive the match to completion and seal the tape.
    fn run_to_tape(mut self) -> FixtureTape {
        while !self.is_done() {
            self.step();
        }
        FixtureTape {
            checkpoints: self.checkpoints,
            win_tick: self.win_tick,
            winner: self.winner,
        }
    }
}

/// One booted fixture mode: the authoritative [`FixtureSession`] plus the
/// presentation [`Director`] framing it and the follow-camera table from the
/// same rule set. The framework boots this from rule-set text (or tables)
/// and drives it tick by tick; camera records never enter the sim hash.
pub struct FixtureMode {
    session: FixtureSession,
    director: Director,
    camera: FixtureCamera,
    frames: Vec<CameraFrame>,
    follow: Vec<[f32; 3]>,
}

impl FixtureMode {
    /// Boot from validated rules under the traversal-course camera table.
    #[must_use]
    pub fn boot(rules: &FixtureRules) -> Self {
        // Traversal tables validate by construction (covered by the preset
        // test); a mode boot from validated rules is total.
        Self::boot_with_camera(rules, DirectorParams::traversal())
            .expect("traversal camera table is valid")
    }

    /// Boot from validated rules under an explicit director table (mode
    /// rule sets pick cameras as data): a bad table fails before any tick.
    /// Budget: O(runners + inputs) time, three small `Vec`s at the boot
    /// boundary (never per tick).
    pub fn boot_with_camera(
        rules: &FixtureRules,
        params: DirectorParams,
    ) -> Result<Self, CameraError> {
        let capacity = rules.ticks as usize;
        Ok(Self {
            session: FixtureSession::boot(rules),
            director: Director::new(0, params)?,
            camera: rules.camera,
            frames: Vec::with_capacity(capacity),
            follow: Vec::with_capacity(capacity),
        })
    }

    /// Parse rule-set text then boot under the traversal-course table.
    pub fn boot_from_text(rules_text: &str) -> Result<Self, FixtureError> {
        Ok(Self::boot(&FixtureRules::parse(rules_text)?))
    }

    /// Parse rule-set text plus a director data table
    /// ([`DirectorParams::decode`]) then boot: a bad table names its reason
    /// instead of simulating.
    pub fn boot_from_tables(rules_text: &str, camera_table: &[u8]) -> Result<Self, FixtureError> {
        let rules = FixtureRules::parse(rules_text)?;
        let params = DirectorParams::decode(camera_table).map_err(|err| match err {
            CameraError::BadLength(_) => FixtureError::BadCameraTable("length"),
            CameraError::FutureVersion(_) => FixtureError::BadCameraTable("version"),
            CameraError::BadParam(row) => FixtureError::BadCamera(row),
            _ => FixtureError::BadCameraTable("row"),
        })?;
        Self::boot_with_camera(&rules, params).map_err(|_| FixtureError::BadCameraTable("row"))
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.session.is_done()
    }

    /// Swap the presentation camera table mid-run at a tick boundary (data,
    /// not code): validates first, sim state untouched. Deterministic for
    /// the same retune tick.
    pub fn retune_camera(&mut self, params: DirectorParams) -> Result<(), CameraError> {
        self.director.retune(params)
    }

    /// Simulate one fixed tick plus both presentation cameras: the director
    /// frames the runners as [`FighterView`]s (focus 0) and the follow table
    /// centers them. Returns false once the match is complete. A non-finite
    /// sim position would fail loudly here; sim math keeps positions finite
    /// by construction (bounded well below overflow).
    pub fn step(&mut self) -> Result<bool, CameraError> {
        if self.session.is_done() {
            return Ok(false);
        }
        let tick = self.session.tick;
        self.session.step();
        let mut views = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 8];
        let runners = fixture_views_into(self.session.positions(), &mut views);
        let frame = self.director.update(&views[..runners], tick)?;
        self.frames.push(frame);
        self.follow.push(
            self.camera
                .frame(self.session.positions(), self.session.goal),
        );
        Ok(!self.session.is_done())
    }

    /// Drive the mode to completion: sealed tape plus both camera records.
    pub fn run(mut self) -> Result<FixturePlayback, CameraError> {
        while self.step()? {}
        let session = self.session;
        Ok(FixturePlayback {
            tape: FixtureTape {
                checkpoints: session.checkpoints,
                win_tick: session.win_tick,
                winner: session.winner,
            },
            frames: self.frames,
            follow: self.follow,
        })
    }
}

/// Played fixture mode: sealed sim tape plus the director frames and the
/// follow-camera centers recorded alongside it (presentation receipts).
#[derive(Clone, PartialEq, Debug)]
pub struct FixturePlayback {
    /// Authoritative sim record: replays byte-exact under the same rules.
    pub tape: FixtureTape,
    /// Director frame per tick (presentation only, never hashed).
    pub frames: Vec<CameraFrame>,
    /// Follow-camera center per tick (presentation only, never hashed).
    pub follow: Vec<[f32; 3]>,
}

impl FixturePlayback {
    /// Replay the authoritative tape under its rules with first-divergence
    /// naming; camera receipts are presentation and replay by equality
    /// (run twice, compare).
    pub fn replay(&self, rules: &FixtureRules) -> Result<(), FixtureError> {
        self.tape.replay(rules)
    }
}

/// Map planar runner positions onto [`FighterView`]s for the director
/// (focus 0, runner height 1 m, all alive): stack buffer, no heap. Returns
/// the runner count.
fn fixture_views_into(positions: &[[f32; 2]], views: &mut [FighterView; 8]) -> usize {
    let count = positions.len().min(views.len());
    for (index, plane) in positions.iter().enumerate().take(count) {
        views[index] = FighterView {
            id: index as u8,
            pos_m: Vec3::new(plane[0], 1.0, plane[1]),
            alive: true,
        };
    }
    count
}

// ---------------------------------------------------------------------------
// Traversal-course rule sets: the GE1 `course` schema (version 3) loads here
// so the mode framework boots it with no engine edits. Validation mirrors
// the GE1 authority (`tests/ge1_genres.rs`): ticks, 1..=2 runners, cruise
// speed, ordered ledges, chase-camera table, ordered run tweaks.
// ---------------------------------------------------------------------------

/// Chase-camera parameters as data (presentation only, never hashed): how
/// far behind the runner the eye sits and how high over it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CourseCamera {
    /// Chase distance behind the runner in meters, `1.0..=20.0`.
    pub distance_m: f32,
    /// Eye height over the runner in meters, `1.0..=20.0`.
    pub height_m: f32,
}

impl CourseCamera {
    /// Range-check the camera data; a bad table fails before use. Pure.
    pub fn validate(self) -> Result<(), FixtureError> {
        if !self.distance_m.is_finite()
            || !(COURSE_CAM_MIN_M..=COURSE_CAM_MAX_M).contains(&self.distance_m)
        {
            return Err(FixtureError::BadCamera("distance_m"));
        }
        if !self.height_m.is_finite()
            || !(COURSE_CAM_MIN_M..=COURSE_CAM_MAX_M).contains(&self.height_m)
        {
            return Err(FixtureError::BadCamera("height_m"));
        }
        Ok(())
    }

    /// Derive the director table for this chase camera as data: the boom
    /// shape reproduces the fixture geometry exactly (eye sits `height_m`
    /// over and `distance_m` behind the look point), the margin is zero so
    /// the follow distance never chases, and the rails bracket the eye
    /// length with headroom so the agent is always framable. Total over
    /// validated tables; the single `sqrt` runs at table-build time
    /// (presentation mapping, never sim state). Pure.
    #[must_use]
    pub fn director_table(&self) -> DirectorParams {
        let eye_len = (self.distance_m * self.distance_m + self.height_m * self.height_m).sqrt();
        DirectorParams {
            fov_y_rad: 70.0f32.to_radians(),
            fit_margin_m: 0.0,
            base_dist_m: self.distance_m,
            min_dist_m: self.distance_m * 0.5,
            max_dist_m: eye_len * 1.1,
            boom_up: self.height_m / self.distance_m,
            boom_back: 1.0,
            max_shake_m: 0.25,
            decay_permille_per_tick: 45,
        }
    }
}

/// One course ledge as data: position along the course and top height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CourseLedge {
    /// Ledge position along the course in meters.
    pub x_m: f32,
    /// Ledge top height in meters (authored in millimetres).
    pub top_m: f32,
}

/// Validated traversal-course rules as data (GE1 `course` schema, version
/// 3). Built only by [`CourseRules::parse`], so every instance is already
/// range-checked.
#[derive(Clone, PartialEq, Debug)]
pub struct CourseRules {
    version: u16,
    seed: u64,
    ticks: u32,
    runners: u8,
    run_speed_mps: f32,
    camera: CourseCamera,
    ledges: Vec<CourseLedge>,
    moves: Vec<FixtureInput>,
}

impl CourseRules {
    /// Parse plus validate one traversal-course rule-set text (the
    /// `fixtures/games/traversal-course/rules.json` schema). Ranges mirror
    /// the GE1 authority; unknown keys are ignored; the run-tweak script
    /// must be strictly ordered (it may be empty: a coasting course still
    /// simulates). Budget: O(ledges + moves) time, two small `Vec`s at the
    /// load boundary (never per tick).
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        let version = fixture_num_after(text, "version")? as u16;
        if version != COURSE_RULES_VERSION {
            return Err(FixtureError::BadVersion(version));
        }
        if !fixture_str_is(text, "fixture", "\"traversal-course\"")? {
            return Err(FixtureError::BadValue("fixture"));
        }
        if !fixture_str_is(text, "mode", "\"course\"")? {
            return Err(FixtureError::UnknownMode);
        }
        let seed = fixture_num_after(text, "seed")? as u64;
        let ticks = fixture_num_after(text, "ticks")? as u32;
        let runners = fixture_num_after(text, "runners")? as u8;
        let run_speed_mps = fixture_num_after(text, "run_speed_mps")? as f32;
        if ticks == 0 || ticks > FIXTURE_MAX_TICKS {
            return Err(FixtureError::BadTicks(ticks));
        }
        if !(1..=COURSE_MAX_RUNNERS).contains(&runners) {
            return Err(FixtureError::BadRunners(runners));
        }
        if !run_speed_mps.is_finite()
            || !(COURSE_SPEED_MIN_MPS..=COURSE_SPEED_MAX_MPS).contains(&run_speed_mps)
        {
            return Err(FixtureError::BadSpeed);
        }
        let ledges = parse_course_ledges(text)?;
        let camera_block = text
            .find("\"camera\"")
            .and_then(|start| {
                text[start..]
                    .find('{')
                    .map(|open| &text[start + open + 1..])
            })
            .ok_or(FixtureError::MissingKey("camera"))?;
        let camera_end = camera_block
            .find('}')
            .ok_or(FixtureError::BadRules("camera"))?;
        let camera_text = &camera_block[..camera_end];
        let camera = CourseCamera {
            distance_m: fixture_num_after(camera_text, "distance_m")? as f32,
            height_m: fixture_num_after(camera_text, "height_m")? as f32,
        };
        camera.validate()?;
        let moves = parse_course_moves(text)?;
        check_fixture_inputs(ticks, runners, &moves)?;
        Ok(Self {
            version,
            seed,
            ticks,
            runners,
            run_speed_mps,
            camera,
            ledges,
            moves,
        })
    }
}

/// True when `"key": "want"` (whitespace-tolerant, exact quoted match).
fn fixture_str_is(text: &str, key: &'static str, want: &str) -> Result<bool, FixtureError> {
    // One small parse-time allocation at the load boundary (never per tick).
    let pattern = ["\"", key, "\""].concat();
    let start = text.find(&pattern).ok_or(FixtureError::MissingKey(key))?;
    let colon = text[start..].find(':').ok_or(FixtureError::BadValue(key))?;
    Ok(text[start + colon + 1..].trim_start().starts_with(want))
}

/// Parse the course `ledges` array: `{x, top_mm}` objects in strictly
/// increasing x, tops in millimetres. Non-empty (GE1 mirror).
fn parse_course_ledges(text: &str) -> Result<Vec<CourseLedge>, FixtureError> {
    let start = text
        .find("\"ledges\"")
        .ok_or(FixtureError::MissingKey("ledges"))?;
    let open = text[start..]
        .find('[')
        .ok_or(FixtureError::BadRules("ledges"))?;
    let body = &text[start + open + 1..];
    let mut ledges = Vec::new();
    let mut cursor = 0usize;
    loop {
        let rest = &body[cursor..];
        let next_obj = rest.find('{');
        let next_close = rest.find(']').ok_or(FixtureError::BadRules("ledges"))?;
        match next_obj {
            Some(at) if at < next_close => {
                let obj_start = cursor + at;
                let obj_end = body[obj_start..]
                    .find('}')
                    .ok_or(FixtureError::BadRules("ledges"))?;
                let obj = &body[obj_start..obj_start + obj_end + 1];
                let x = fixture_num_after(obj, "x")? as f32;
                let top_mm = fixture_num_after(obj, "top_mm")? as f32;
                if !x.is_finite()
                    || !(COURSE_X_MIN_M..=COURSE_X_MAX_M).contains(&x)
                    || !top_mm.is_finite()
                    || !(COURSE_TOP_MIN_MM..=COURSE_TOP_MAX_MM).contains(&top_mm)
                    || ledges
                        .last()
                        .is_some_and(|prev: &CourseLedge| x <= prev.x_m)
                {
                    return Err(FixtureError::BadLedge(ledges.len()));
                }
                ledges.push(CourseLedge {
                    x_m: x,
                    top_m: top_mm / 1000.0,
                });
                cursor = obj_start + obj_end + 1;
                if ledges.len() > COURSE_MAX_LEDGES {
                    return Err(FixtureError::TooManyLedges);
                }
            }
            _ => break,
        }
    }
    if ledges.is_empty() {
        return Err(FixtureError::BadLedges(0));
    }
    Ok(ledges)
}

/// Parse the course `moves` array: `{tick, body, vx, vy}` run tweaks in
/// strictly increasing `(tick, body)` order (checked by the caller through
/// [`check_fixture_inputs`]). Empty scripts are legal: a coasting course
/// still simulates (GE1 mirror).
fn parse_course_moves(text: &str) -> Result<Vec<FixtureInput>, FixtureError> {
    let start = text
        .find("\"moves\"")
        .ok_or(FixtureError::MissingKey("moves"))?;
    let open = text[start..]
        .find('[')
        .ok_or(FixtureError::BadRules("moves"))?;
    let body = &text[start + open + 1..];
    let mut moves = Vec::new();
    let mut cursor = 0usize;
    loop {
        let rest = &body[cursor..];
        let next_obj = rest.find('{');
        let next_close = rest.find(']').ok_or(FixtureError::BadRules("moves"))?;
        match next_obj {
            Some(at) if at < next_close => {
                let obj_start = cursor + at;
                let obj_end = body[obj_start..]
                    .find('}')
                    .ok_or(FixtureError::BadRules("moves"))?;
                let obj = &body[obj_start..obj_start + obj_end + 1];
                moves.push(FixtureInput {
                    tick: fixture_num_after(obj, "tick")? as u32,
                    body: fixture_num_after(obj, "body")? as u8,
                    vx: fixture_num_after(obj, "vx")? as f32,
                    vy: fixture_num_after(obj, "vy")? as f32,
                });
                cursor = obj_start + obj_end + 1;
                if moves.len() > FIXTURE_MAX_INPUTS {
                    return Err(FixtureError::TooManyInputs);
                }
            }
            _ => break,
        }
    }
    Ok(moves)
}

/// One booted traversal course: the authoritative [`FixtureSession`] (open
/// run, no goal disc) plus the presentation [`Director`] framing it under
/// the rule set's own chase-camera table. The framework boots this from
/// rule-set text and drives it tick by tick; camera records never enter
/// the sim hash.
pub struct CourseMode {
    session: FixtureSession,
    director: Director,
    frames: Vec<CameraFrame>,
    /// Focus-agent plane per tick (the tracked agent's receipts).
    agents: Vec<[f32; 2]>,
}

impl CourseMode {
    /// Boot from validated course rules under the rule set's own
    /// chase-camera table ([`CourseCamera::director_table`]): a bad derived
    /// table fails before any tick. Budget: O(runners + moves) time, three
    /// small `Vec`s at the boot boundary (never per tick).
    pub fn boot(rules: &CourseRules) -> Result<Self, CameraError> {
        let params = rules.camera.director_table();
        params.validate()?;
        let capacity = rules.ticks as usize;
        Ok(Self {
            session: FixtureSession::boot_course(rules),
            director: Director::new(0, params)?,
            frames: Vec::with_capacity(capacity),
            agents: Vec::with_capacity(capacity),
        })
    }

    /// Parse course rule-set text then boot under its own chase table.
    pub fn boot_from_text(rules_text: &str) -> Result<Self, FixtureError> {
        let rules = CourseRules::parse(rules_text)?;
        Self::boot(&rules).map_err(|_| FixtureError::BadCameraTable("row"))
    }

    /// True once every configured tick has simulated.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.session.is_done()
    }

    /// Simulate one fixed tick plus the presentation camera: the director
    /// frames the runners (focus 0) and the focus plane is recorded.
    /// Returns false once the course is complete. Positions stay finite by
    /// construction (bounded well below overflow).
    pub fn step(&mut self) -> Result<bool, CameraError> {
        if self.session.is_done() {
            return Ok(false);
        }
        let tick = self.session.tick;
        self.session.step();
        let mut views = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 8];
        let runners = fixture_views_into(self.session.positions(), &mut views);
        let frame = self.director.update(&views[..runners], tick)?;
        self.frames.push(frame);
        self.agents.push(self.session.positions()[0]);
        Ok(!self.session.is_done())
    }

    /// Drive the course to completion: sealed tape plus the director frames
    /// and the focus-agent planes.
    pub fn run(mut self) -> Result<CoursePlayback, CameraError> {
        while self.step()? {}
        let session = self.session;
        Ok(CoursePlayback {
            tape: FixtureTape {
                checkpoints: session.checkpoints,
                win_tick: session.win_tick,
                winner: session.winner,
            },
            frames: self.frames,
            agents: self.agents,
        })
    }
}

/// Played traversal course: sealed sim tape plus the director frames and
/// the focus-agent planes recorded alongside it (presentation receipts).
#[derive(Clone, PartialEq, Debug)]
pub struct CoursePlayback {
    /// Authoritative sim record: replays byte-exact under the same rules.
    pub tape: FixtureTape,
    /// Director frame per tick (presentation only, never hashed).
    pub frames: Vec<CameraFrame>,
    /// Focus-agent plane per tick (presentation only, never hashed).
    pub agents: Vec<[f32; 2]>,
}

impl CoursePlayback {
    /// Replay the authoritative tape under its course rules with
    /// first-divergence naming; camera receipts replay by equality (run
    /// twice, compare).
    pub fn replay(&self, rules: &CourseRules) -> Result<(), FixtureError> {
        self.tape.replay_course(rules)
    }
}

// ---------------------------------------------------------------------------
// Staged rig, intent/smoothed damping and trauma-curve shake:
// named, ordered, bypassable presentation stages beside the frozen
// director. Tick math stays integer permille; all `f32` pose/shake/smoothing
// math is presentation-only and never feeds simulation state. Zero heap
// bytes per update; same inputs replay byte-exact twice. Consumer:
// `camera_feed` (`PresentationFeed`, `SmoothedFeed`) under the window loop.
// ---------------------------------------------------------------------------

/// GP19 rig stage: one named pure map in the follow -> lock -> shake chain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RigStage {
    /// Frame focus plus anchor (pose without lock choice or shake).
    Follow,
    /// Nearest-in-frustum lock selection from the unshaken pose.
    Lock,
    /// Trauma-squared shake offset on the presented eye only.
    Shake,
}

impl RigStage {
    /// Short stable name for receipts and reorder logs.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Lock => "lock",
            Self::Shake => "shake",
        }
    }
}

/// Ordered rig chain: follow feeds lock, lock feeds shake.
pub const RIG_STAGES: [RigStage; 3] = [RigStage::Follow, RigStage::Lock, RigStage::Shake];
/// Bypass bit that drops lock reporting (framing keeps the anchor).
pub const BYPASS_LOCK: u8 = 0x01;
/// Bypass bit that drops shake (the eye reports the unshaken pose).
pub const BYPASS_SHAKE: u8 = 0x02;

/// Follow stage as a pure map: frame `focus_m` plus `anchor_m`.
pub fn follow_stage(
    focus_m: Vec3,
    anchor_m: Option<Vec3>,
    params: &DirectorParams,
) -> Result<CameraPose, CameraError> {
    frame_pair(focus_m, anchor_m, params)
}

/// Lock stage as a pure map: nearest-in-frustum target from the pose.
pub fn lock_stage(
    eye_m: Vec3,
    forward: Vec3,
    params: &DirectorParams,
    focus_id: u8,
    fighters: &[FighterView],
) -> Result<Option<u8>, CameraError> {
    select_lock_target(
        eye_m,
        forward,
        params.fov_y_rad * 0.5,
        params.max_dist_m,
        focus_id,
        fighters,
    )
}

/// Shake stage as a pure map: trauma-squared offset for (`trauma`, `tick`).
#[must_use]
pub fn shake_stage(trauma_permille: u32, tick: u32, max_m: f32) -> Vec3 {
    shake_offset(trauma_permille, tick, max_m)
}

/// GP19 staged rig: the frozen [`Director`] driven as named ordered stages.
/// With no bypass the frames are byte-identical to the director; each bypass
/// bit drops one reporting stage while framing still holds, so a stage
/// reorder or removal needs only builder order, never new math.
#[derive(Clone, Debug)]
pub struct StagedRig {
    director: Director,
    bypass: u8,
}

impl StagedRig {
    /// Build a rig following `focus_id` under `params`, no stage bypassed.
    pub fn new(focus_id: u8, params: DirectorParams) -> Result<Self, CameraError> {
        Ok(Self {
            director: Director::new(focus_id, params)?,
            bypass: 0,
        })
    }

    /// Set the bypass mask (`BYPASS_LOCK | BYPASS_SHAKE`); unknown bits are
    /// kept but ignored, so newer stages never break older callers.
    pub fn set_bypass(&mut self, mask: u8) {
        self.bypass = mask;
    }

    /// Current bypass mask.
    #[must_use]
    pub fn bypass(&self) -> u8 {
        self.bypass
    }

    /// Ordered stage names served this update (follow, lock, shake).
    #[must_use]
    pub fn stage_names(&self) -> [&'static str; 3] {
        [
            RigStage::Follow.name(),
            RigStage::Lock.name(),
            RigStage::Shake.name(),
        ]
    }

    /// Followed fighter id.
    #[must_use]
    pub fn focus_id(&self) -> u8 {
        self.director.focus_id()
    }

    /// Active tuning table (copy).
    #[must_use]
    pub fn params(&self) -> DirectorParams {
        self.director.params()
    }

    /// Swap tuning at a tick boundary; validates first like the director.
    pub fn retune(&mut self, params: DirectorParams) -> Result<(), CameraError> {
        self.director.retune(params)
    }

    /// Current lock-on target (`None` while the lock stage is bypassed).
    #[must_use]
    pub fn lock(&self) -> Option<u8> {
        if self.bypass & BYPASS_LOCK != 0 {
            None
        } else {
            self.director.lock()
        }
    }

    /// Current trauma permille after decay.
    #[must_use]
    pub fn trauma_permille(&self) -> u32 {
        self.director.trauma_permille()
    }

    /// Record a hit: saturating add, clamped to `0..=1000`.
    pub fn add_trauma(&mut self, amount_permille: u32) {
        self.director.add_trauma(amount_permille);
    }

    /// Present one tick through the ordered stages: the director serves the
    /// follow-plus-lock core (frozen API), then the named shake stage
    /// re-derives the offset for audit (bit-exact: same pure inputs).
    /// Bypassed stages report neutral values while the unshaken pose still
    /// frames, so dropping lock or shake loses no framed tick.
    pub fn update(
        &mut self,
        fighters: &[FighterView],
        tick: u32,
    ) -> Result<CameraFrame, CameraError> {
        let mut frame = self.director.update(fighters, tick)?;
        let audited = shake_stage(
            frame.trauma_permille,
            tick,
            self.director.params().max_shake_m,
        );
        debug_assert_eq!(audited, frame.shake_m, "shake stage matches director");
        if self.bypass & BYPASS_SHAKE != 0 {
            frame.eye_m = frame.eye_m - frame.shake_m;
            frame.shake_m = Vec3::ZERO;
        }
        if self.bypass & BYPASS_LOCK != 0 {
            frame.lock = None;
        }
        Ok(frame)
    }
}

/// GP20 look intent: where the camera wants to be this tick (raw rig output,
/// unsmoothed). Presentation data only, never sim input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookIntent {
    /// Wanted eye in meters (shaken, as presented).
    pub eye_m: Vec3,
    /// Wanted look point in meters.
    pub look_m: Vec3,
}

impl LookIntent {
    /// Capture the raw intent from a presented frame (no smoothing).
    #[must_use]
    pub fn from_frame(frame: &CameraFrame) -> Self {
        Self {
            eye_m: frame.eye_m,
            look_m: frame.look_m,
        }
    }
}

/// GP20 damped sync: the smoothed scene transform kept apart from the raw
/// intent. The frame rate only decides how many `sync` calls run between
/// ticks, never the sim bytes. Gain is integer permille per sync call; the
/// `f32` approach math is presentation-only. Zero heap bytes per call.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DampedSync {
    eye_m: Vec3,
    look_m: Vec3,
    gain_permille: u32,
    ready: bool,
}

impl DampedSync {
    /// Build a sync with per-call gain `1..=1000` (1000 snaps every call).
    pub fn new(gain_permille: u32) -> Result<Self, CameraError> {
        if !(1..=1000).contains(&gain_permille) {
            return Err(CameraError::BadParam("damping_gain"));
        }
        Ok(Self {
            eye_m: Vec3::ZERO,
            look_m: Vec3::ZERO,
            gain_permille,
            ready: false,
        })
    }

    /// Per-call gain permille.
    #[must_use]
    pub fn gain_permille(&self) -> u32 {
        self.gain_permille
    }

    /// Smoothed pose so far, if any intent synced yet.
    #[must_use]
    pub fn smoothed(&self) -> Option<LookIntent> {
        if self.ready {
            Some(LookIntent {
                eye_m: self.eye_m,
                look_m: self.look_m,
            })
        } else {
            None
        }
    }

    /// Intent bypass: the raw intent passes through untouched (identical).
    #[must_use]
    pub fn bypass(intent: &LookIntent) -> LookIntent {
        *intent
    }

    /// Fold one intent into the smoothed transform: the first call snaps,
    /// later calls approach by `gain_permille / 1000` per call (full gain
    /// snaps, bit-exact). The same intent sequence gives the same smoothed
    /// bytes twice. No heap.
    pub fn sync(&mut self, intent: &LookIntent) -> LookIntent {
        if !self.ready || self.gain_permille == 1000 {
            self.eye_m = intent.eye_m;
            self.look_m = intent.look_m;
            self.ready = true;
            return *intent;
        }
        let t = (self.gain_permille as f32) / 1000.0;
        self.eye_m = self.eye_m + (intent.eye_m - self.eye_m) * t;
        self.look_m = self.look_m + (intent.look_m - self.look_m) * t;
        LookIntent {
            eye_m: self.eye_m,
            look_m: self.look_m,
        }
    }
}

/// Jab trauma: one landed light hit (matches the feed `TRAUMA_HIT`).
pub const TRAUMA_JAB_PERMILLE: u32 = 180;
/// Super trauma: a full-meter super (saturated trauma).
pub const TRAUMA_SUPER_PERMILLE: u32 = 1000;

/// Trauma response curve: squared trauma gain (`t^2`), so a jab moves
/// far less than a super. Pure presentation math, never sim state.
#[must_use]
pub fn trauma_gain(trauma_permille: u32) -> f32 {
    let t = trauma_permille.min(TRAUMA_SCALE) as f32 / TRAUMA_SCALE as f32;
    t * t
}

/// Restore the unshaken pose eye from a presented frame: the shake offset is
/// presentation-only, so removing it returns the pose eye the next tick
/// selects from (within one float rounding step).
#[must_use]
pub fn unshaken_eye(frame: &CameraFrame) -> Vec3 {
    frame.eye_m - frame.shake_m
}

/// Presented shake displacement in meters for one frame.
#[must_use]
pub fn shake_displacement_m(frame: &CameraFrame) -> f32 {
    frame.shake_m.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use std::f32::consts::{FRAC_PI_3, PI};

    fn eye() -> Vec3 {
        Vec3::ZERO
    }

    fn back() -> Vec3 {
        Vec3::new(0.0, 0.0, -1.0)
    }

    fn view(id: u8, x: f32, y: f32, z: f32) -> FighterView {
        FighterView {
            id,
            pos_m: Vec3::new(x, y, z),
            alive: true,
        }
    }

    fn half_30() -> f32 {
        FRAC_PI_3 * 0.5
    }

    /// Scripted two-fighter orbit, radius 5 m, opposite phases.
    fn scripted_pair(tick: u32) -> [FighterView; 2] {
        let angle = tick as f32 * 0.02;
        let radius = 5.0;
        [
            FighterView {
                id: 0,
                pos_m: Vec3::new(radius * angle.cos(), 1.0, radius * angle.sin()),
                alive: true,
            },
            FighterView {
                id: 1,
                pos_m: Vec3::new(
                    radius * (angle + PI).cos(),
                    1.0,
                    radius * (angle + PI).sin(),
                ),
                alive: true,
            },
        ]
    }

    #[test]
    fn defaults_validate() {
        DirectorParams::default()
            .validate()
            .expect("defaults are valid");
        Director::new(0, DirectorParams::default()).expect("director builds");
    }

    #[test]
    fn rejects_bad_params() {
        let good = DirectorParams::default();
        let bad_rows: [DirectorParams; 12] = [
            DirectorParams {
                fov_y_rad: 0.0,
                ..good
            },
            DirectorParams {
                fov_y_rad: PI,
                ..good
            },
            DirectorParams {
                fov_y_rad: f32::NAN,
                ..good
            },
            DirectorParams {
                fit_margin_m: -1.0,
                ..good
            },
            DirectorParams {
                base_dist_m: 0.0,
                ..good
            },
            DirectorParams {
                min_dist_m: 9.0,
                max_dist_m: 8.0,
                ..good
            },
            DirectorParams {
                base_dist_m: 1.0,
                ..good
            },
            DirectorParams {
                base_dist_m: 31.0,
                ..good
            },
            DirectorParams {
                boom_up: 0.0,
                boom_back: 0.0,
                ..good
            },
            DirectorParams {
                boom_up: f32::NAN,
                ..good
            },
            DirectorParams {
                max_shake_m: -0.1,
                ..good
            },
            DirectorParams {
                max_shake_m: f32::INFINITY,
                ..good
            },
        ];
        for bad in bad_rows {
            assert!(bad.validate().is_err(), "param accepted: {bad:?}");
            assert!(Director::new(0, bad).is_err(), "director accepted: {bad:?}");
        }
        // Zero decay is legal: trauma simply never fades.
        DirectorParams {
            decay_permille_per_tick: 0,
            ..good
        }
        .validate()
        .expect("zero decay is valid");
    }

    #[test]
    fn lock_picks_nearest_in_frustum() {
        let pair = [view(1, 0.0, 0.0, -10.0), view(2, 0.0, 0.0, -5.0)];
        let lock = select_lock_target(eye(), back(), half_30(), 30.0, 0, &pair).expect("select");
        assert_eq!(lock, Some(2));
    }

    #[test]
    fn lock_tie_breaks_by_fighter_id_regardless_of_order() {
        // Symmetric pair: bit-exact equal squared distance.
        let pair = [view(2, 1.0, 0.0, -4.0), view(1, -1.0, 0.0, -4.0)];
        let first = select_lock_target(eye(), back(), half_30(), 30.0, 0, &pair).expect("select");
        assert_eq!(first, Some(1));
        let reversed = [pair[1], pair[0]];
        let second =
            select_lock_target(eye(), back(), half_30(), 30.0, 0, &reversed).expect("select");
        assert_eq!(second, Some(1), "container order must not decide ties");
    }

    #[test]
    fn lock_skips_self_dead_behind_and_far() {
        let only_invalid = [
            FighterView {
                id: 0,
                pos_m: Vec3::new(0.0, 0.0, -1.0),
                alive: true,
            },
            FighterView {
                id: 1,
                pos_m: Vec3::new(0.0, 0.0, -1.0),
                alive: false,
            },
            view(2, 0.0, 0.0, 5.0),
            view(3, 0.0, 0.0, -500.0),
        ];
        let none =
            select_lock_target(eye(), back(), half_30(), 30.0, 0, &only_invalid).expect("select");
        assert_eq!(none, None);
        let mut with_valid = only_invalid.to_vec();
        with_valid.push(view(4, 0.5, 0.0, -6.0));
        let lock =
            select_lock_target(eye(), back(), half_30(), 30.0, 0, &with_valid).expect("select");
        assert_eq!(lock, Some(4));
    }

    #[test]
    fn lock_rejects_nan_position() {
        let clean = [view(1, 0.0, 0.0, -5.0)];
        assert_eq!(
            select_lock_target(
                Vec3::new(f32::NAN, 0.0, 0.0),
                back(),
                half_30(),
                30.0,
                0,
                &clean
            ),
            Err(CameraError::BadEye)
        );
        assert_eq!(
            select_lock_target(eye(), Vec3::ZERO, half_30(), 30.0, 0, &clean),
            Err(CameraError::BadForward)
        );
        let pair = [
            view(1, 0.0, 0.0, -5.0),
            FighterView {
                id: 2,
                pos_m: Vec3::new(f32::NAN, 0.0, -5.0),
                alive: false,
            },
        ];
        assert_eq!(
            select_lock_target(eye(), back(), half_30(), 30.0, 0, &pair),
            Err(CameraError::BadPosition(2)),
            "NaN fails even for skipped fighters"
        );
    }

    #[test]
    fn trauma_decays_in_exact_integer_ticks() {
        let params = DirectorParams {
            decay_permille_per_tick: 60,
            ..DirectorParams::default()
        };
        let mut director = Director::new(0, params).expect("director builds");
        let pair = scripted_pair(0);
        let frame = director.update(&pair, 0).expect("frame");
        assert_eq!(frame.trauma_permille, 0);
        director.add_trauma(200);
        director.add_trauma(900);
        assert_eq!(director.trauma_permille(), 1000, "trauma clamps at full");
        let frame = director.update(&pair, 0).expect("same tick keeps trauma");
        assert_eq!(frame.trauma_permille, 1000);
        let frame = director.update(&pair, 1).expect("one tick decays once");
        assert_eq!(frame.trauma_permille, 940);
        let frame = director.update(&pair, 2).expect("frame");
        assert_eq!(frame.trauma_permille, 880);
        let frame = director.update(&pair, 12).expect("jump decays ten ticks");
        assert_eq!(frame.trauma_permille, 280);
        let frame = director.update(&pair, 10_000).expect("frame");
        assert_eq!(frame.trauma_permille, 0, "decay saturates at zero");
        director.add_trauma(u32::MAX);
        assert_eq!(director.trauma_permille(), 1000, "giant hits clamp");
    }

    #[test]
    fn shake_is_pure_zero_at_rest_and_bounded() {
        assert_eq!(shake_offset(0, 77, 0.5), Vec3::ZERO);
        assert_eq!(shake_offset(1000, 77, 0.0), Vec3::ZERO);
        assert_eq!(shake_offset(1000, 77, -1.0), Vec3::ZERO);
        let a = shake_offset(750, 1234, 0.5);
        let b = shake_offset(750, 1234, 0.5);
        assert_eq!(a, b, "same inputs give the same offset");
        let mag = 0.5 * 0.75 * 0.75;
        for component in [a.x, a.y, a.z] {
            assert!(
                component >= -mag && component <= mag,
                "shake bounded by the trauma curve"
            );
        }
        let rest = shake_offset(750, 0, 0.5);
        let mut varied = false;
        for tick in 1..32u32 {
            if shake_offset(750, tick, 0.5) != rest {
                varied = true;
                break;
            }
        }
        assert!(varied, "shake changes across ticks");
    }

    #[test]
    fn framing_centers_pair_and_clamps() {
        let params = DirectorParams::default();
        let focus = Vec3::new(0.0, 1.0, 0.0);
        let other = Vec3::new(10.0, 1.0, 0.0);
        let pose = frame_pair(focus, Some(other), &params).expect("frame");
        assert_eq!(pose.look_m, Vec3::new(5.0, 1.0, 0.0));
        assert!(pose.eye_m.y > pose.look_m.y, "boom sits above");
        assert!(pose.eye_m.z > pose.look_m.z, "boom sits behind");
        // A continent-wide pair clamps to the far rail instead of chasing.
        let boom_len =
            (params.boom_up * params.boom_up + params.boom_back * params.boom_back).sqrt();
        let far = frame_pair(focus, Some(Vec3::new(1.0e6, 1.0, 0.0)), &params).expect("frame");
        let dist = (far.eye_m - far.look_m).len();
        assert!(
            (dist - params.max_dist_m * boom_len).abs() <= 1.0e-2,
            "distance clamps at the far rail"
        );
        // Solo focus frames at base plus the margin fit (headroom stays).
        let solo = frame_pair(focus, None, &params).expect("frame");
        let solo_dist = (solo.eye_m - solo.look_m).len();
        let half = params.fov_y_rad * 0.5;
        let expected = (params.base_dist_m + params.fit_margin_m / half.tan()) * boom_len;
        assert!(
            (solo_dist - expected).abs() <= 1.0e-4,
            "solo frame uses base plus margin fit"
        );
        assert_eq!(
            frame_pair(Vec3::new(f32::NAN, 0.0, 0.0), None, &params),
            Err(CameraError::BadParam("focus_m"))
        );
    }

    #[test]
    fn scripted_orbit_keeps_both_fighters_framed_600_ticks() {
        let params = DirectorParams::default();
        let mut director = Director::new(0, params).expect("director builds");
        let cos_limit = (params.fov_y_rad * 0.5).cos();
        let max_sq = params.max_dist_m * params.max_dist_m;
        let first_bytes = director
            .update(&scripted_pair(0), 0)
            .expect("first frame")
            .to_bytes();
        for tick in 1..600u32 {
            let pair = scripted_pair(tick);
            let frame = director.update(&pair, tick).expect("frame");
            assert_eq!(frame.lock, Some(1), "rival locked at tick {tick}");
            assert_eq!(frame.shake_m, Vec3::ZERO, "no trauma means no shake");
            let fwd = (frame.look_m - frame.eye_m)
                .normalize()
                .expect("non-degenerate view");
            for fighter in &pair {
                let delta = fighter.pos_m - frame.eye_m;
                let dist_sq = delta.len_sq();
                assert!(dist_sq <= max_sq, "fighter {} in range", fighter.id);
                let cos_angle = delta.dot(fwd) / dist_sq.sqrt();
                assert!(
                    cos_angle >= cos_limit,
                    "fighter {} framed at tick {tick}",
                    fighter.id
                );
            }
        }
        let last_bytes = director
            .update(&scripted_pair(600), 600)
            .expect("last frame")
            .to_bytes();
        assert!(first_bytes != last_bytes, "camera tracked the orbit");
    }

    #[test]
    fn same_tape_gives_same_camera_bytes_twice() {
        fn run() -> (Vec<[u8; FRAME_BYTES]>, Vec<u32>) {
            let mut director =
                Director::new(0, DirectorParams::default()).expect("director builds");
            let mut bytes = Vec::with_capacity(600);
            let mut trauma = Vec::with_capacity(600);
            for tick in 0..600u32 {
                if tick % 30 == 0 {
                    director.add_trauma(180);
                }
                let pair = scripted_pair(tick);
                let frame = director.update(&pair, tick).expect("frame");
                bytes.push(frame.to_bytes());
                trauma.push(frame.trauma_permille);
            }
            (bytes, trauma)
        }
        let (first_bytes, first_trauma) = run();
        let (second_bytes, second_trauma) = run();
        assert_eq!(first_bytes, second_bytes, "camera bytes replay exactly");
        assert_eq!(first_trauma, second_trauma, "trauma track replays exactly");
        assert!(
            first_bytes.first() != first_bytes.last(),
            "camera tracked the fight"
        );
        assert!(first_trauma.iter().any(|t| *t > 0), "trauma was recorded");
    }

    #[test]
    fn frame_bytes_hold_tick_trauma_and_lock() {
        let frame = CameraFrame {
            tick: 0x0102_0304,
            eye_m: Vec3::new(1.0, 2.0, 3.0),
            look_m: Vec3::new(4.0, 5.0, 6.0),
            shake_m: Vec3::ZERO,
            lock: Some(3),
            trauma_permille: 750,
        };
        assert_eq!(FRAME_BYTES, 46);
        let bytes = frame.to_bytes();
        assert_eq!(&bytes[0..4], &0x0102_0304u32.to_le_bytes());
        assert_eq!(&bytes[40..44], &750u32.to_le_bytes());
        assert_eq!([bytes[44], bytes[45]], [1, 3]);
        let unlocked = CameraFrame {
            lock: None,
            ..frame
        };
        let raw = unlocked.to_bytes();
        assert_eq!([raw[44], raw[45]], [0, 0]);
        assert_eq!(LOCK_NONE_U8, 0xFF);
    }

    #[test]
    fn focus_missing_and_backwards_ticks_fail() {
        let mut director = Director::new(7, DirectorParams::default()).expect("director builds");
        let pair = scripted_pair(0);
        assert_eq!(director.update(&pair, 0), Err(CameraError::FocusMissing(7)));
        let mut director = Director::new(0, DirectorParams::default()).expect("director builds");
        director.update(&pair, 5).expect("frame");
        assert_eq!(
            director.update(&pair, 4),
            Err(CameraError::TickWentBackwards { last: 5, next: 4 })
        );
        let nan_pair = [
            pair[0],
            FighterView {
                id: 9,
                pos_m: Vec3::new(f32::NAN, 0.0, 0.0),
                alive: true,
            },
        ];
        assert_eq!(
            director.update(&nan_pair, 5),
            Err(CameraError::BadPosition(9))
        );
    }

    #[test]
    fn steady_state_update_allocates_nothing() {
        let mut director = Director::new(0, DirectorParams::default()).expect("director builds");
        director
            .update(&scripted_pair(0), 0)
            .expect("warm-up frame");
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..=600u32 {
                let frame = director.update(&scripted_pair(tick), tick).expect("frame");
                let bytes = frame.to_bytes();
                let mut tick_bytes = [0u8; 8];
                tick_bytes.copy_from_slice(&bytes[0..8]);
                folded ^= u64::from_le_bytes(tick_bytes);
            }
            folded
        });
        assert_eq!(steady.calls, 0, "steady-state update must not allocate");
        assert!(folded != 0, "sanity: frames advanced during the window");
    }

    // -- director params as data tables ------------------------------------

    #[test]
    fn director_tables_roundtrip_byte_exact() {
        let arena = DirectorParams::arena();
        assert_eq!(arena, DirectorParams::default());
        arena.validate().expect("arena table is valid");
        let bytes = arena.encode();
        assert_eq!(bytes.len(), DIRECTOR_TABLE_BYTES);
        assert_eq!(bytes[0], DIRECTOR_TABLE_VERSION);
        assert_eq!(bytes, arena.encode(), "same table gives same bytes twice");
        assert_eq!(DirectorParams::decode(&bytes).expect("decode"), arena);
        let traversal = DirectorParams::traversal();
        traversal.validate().expect("traversal table is valid");
        assert_ne!(traversal, arena, "different camera, different table");
        assert_eq!(
            DirectorParams::decode(&traversal.encode()).expect("decode"),
            traversal
        );
    }

    #[test]
    fn director_table_refusals_name_the_row() {
        let good = DirectorParams::default().encode();
        for len in [0, 1, DIRECTOR_TABLE_BYTES - 1, DIRECTOR_TABLE_BYTES + 1] {
            let mut clipped = good.to_vec();
            clipped.resize(len, 0xAB);
            assert_eq!(
                DirectorParams::decode(&clipped),
                Err(CameraError::BadLength(len))
            );
        }
        for version in [DIRECTOR_TABLE_VERSION + 1, u8::MAX] {
            let mut bad = good;
            bad[0] = version;
            assert_eq!(
                DirectorParams::decode(&bad),
                Err(CameraError::FutureVersion(version))
            );
        }
        // Each row refuses with its name: flip one row out of range at a time.
        let mut bad_fov = good;
        put_table_f32(&mut bad_fov, 1, 0.0);
        assert_eq!(
            DirectorParams::decode(&bad_fov),
            Err(CameraError::BadParam("fov_y_rad"))
        );
        let mut bad_base = good;
        put_table_f32(&mut bad_base, 9, 500.0);
        assert_eq!(
            DirectorParams::decode(&bad_base),
            Err(CameraError::BadParam("base_dist_m"))
        );
        let mut bad_boom = good;
        put_table_f32(&mut bad_boom, 21, 0.0);
        put_table_f32(&mut bad_boom, 25, 0.0);
        assert_eq!(
            DirectorParams::decode(&bad_boom),
            Err(CameraError::BadParam("boom"))
        );
        // NaN rows fail loudly instead of smuggling non-determinism.
        let mut bad_nan = good;
        put_table_f32(&mut bad_nan, 29, f32::NAN);
        assert!(DirectorParams::decode(&bad_nan).is_err());
        assert!(format!("{}", CameraError::BadLength(3)).contains('3'));
        assert!(format!("{}", CameraError::FutureVersion(9)).contains('9'));
    }

    #[test]
    fn retune_swaps_at_a_boundary_and_refuses_bad() {
        let mut director = Director::new(0, DirectorParams::arena()).expect("director builds");
        assert_eq!(director.params(), DirectorParams::arena());
        director
            .retune(DirectorParams::traversal())
            .expect("traversal retune");
        assert_eq!(director.params(), DirectorParams::traversal());
        let bad = DirectorParams {
            base_dist_m: 0.0,
            ..DirectorParams::traversal()
        };
        assert_eq!(
            director.retune(bad),
            Err(CameraError::BadParam("base_dist_m"))
        );
        assert_eq!(
            director.params(),
            DirectorParams::traversal(),
            "failed retune keeps the old table"
        );
    }

    /// Solo traversal agent: a 6 m circle at 1 m height, one tick per step.
    fn scripted_runner(tick: u32) -> FighterView {
        let angle = tick as f32 * 0.02;
        FighterView {
            id: 0,
            pos_m: Vec3::new(6.0 * angle.cos(), 1.0, 6.0 * angle.sin()),
            alive: true,
        }
    }

    /// Assert one agent sits inside the frustum cone and the range rail.
    fn assert_framed(frame: &CameraFrame, agent: &FighterView, params: &DirectorParams, tick: u32) {
        let fwd = (frame.look_m - frame.eye_m)
            .normalize()
            .expect("non-degenerate view");
        let delta = agent.pos_m - frame.eye_m;
        let dist_sq = delta.len_sq();
        assert!(
            dist_sq <= params.max_dist_m * params.max_dist_m,
            "agent in range at tick {tick}"
        );
        let cos_angle = delta.dot(fwd) / dist_sq.sqrt();
        assert!(
            cos_angle >= (params.fov_y_rad * 0.5).cos(),
            "agent framed at tick {tick}"
        );
    }

    #[test]
    fn traversal_course_keeps_solo_agent_framed_600_ticks() {
        fn run() -> Vec<[u8; FRAME_BYTES]> {
            let params = DirectorParams::traversal();
            let mut director = Director::new(0, params).expect("director builds");
            let mut bytes = Vec::with_capacity(600);
            for tick in 0..600u32 {
                let agent = scripted_runner(tick);
                let frame = director.update(&[agent], tick).expect("frame");
                assert_eq!(frame.lock, None, "solo follow locks nothing");
                assert_eq!(frame.shake_m, Vec3::ZERO, "no trauma means no shake");
                assert_framed(&frame, &agent, &params, tick);
                bytes.push(frame.to_bytes());
            }
            bytes
        }
        let first = run();
        let second = run();
        assert_eq!(first, second, "traversal track replays exactly");
        assert!(first.first() != first.last(), "camera tracked the course");
    }

    // -- fixture rule-set loading, boot and replay --------------------------

    /// Read the real tag-schema fixture rule set (proves the mode boots with no
    /// engine edits; fails loudly naming the path when the fixture moves).
    fn fixture_file_text() -> String {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("games")
            .join("second-game")
            .join("rules.json");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    #[test]
    fn fixture_boots_real_rules_file_and_replays_twice() {
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        assert_eq!(rules.version, FIXTURE_RULES_VERSION);
        assert_eq!(rules.seed, 77);
        assert_eq!(rules.ticks, 120);
        assert_eq!(rules.runners, 4);
        assert_eq!(rules.goal, [8.0, 6.0]);
        assert_eq!(rules.goal_radius_m, 1.5);
        assert_eq!(rules.half_extent_m, 20.0);
        assert_eq!(
            rules.camera,
            FixtureCamera {
                height_m: 12.0,
                lead_m: 2.0,
            }
        );
        assert_eq!(rules.inputs.len(), 3);
        let first = FixtureMode::boot_from_text(&text)
            .expect("mode boots")
            .run()
            .expect("mode runs");
        let second = FixtureMode::boot_from_text(&text)
            .expect("mode boots")
            .run()
            .expect("mode runs");
        assert_eq!(first, second, "fixture mode replays exactly");
        assert_eq!(first.tape.checkpoints.len(), 120);
        assert_eq!(first.frames.len(), 120);
        assert_eq!(first.follow.len(), 120);
        let mut hashes: Vec<u64> = first
            .tape
            .checkpoints
            .iter()
            .map(|point| point.hash)
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "match never moved");
        assert_ne!(first.tape.digest(), 0, "digest folds real state");
        for frame in &first.frames {
            assert_eq!(frame.shake_m, Vec3::ZERO, "fixture lands no hits");
        }
        first.replay(&rules).expect("fixture replay");
    }

    #[test]
    fn different_goal_is_a_different_game() {
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        let mut moved = rules.clone();
        moved.goal = [-8.0, -6.0];
        let base = FixtureMode::boot(&rules).run().expect("base runs");
        let other = FixtureMode::boot(&moved).run().expect("moved runs");
        assert_ne!(
            base.tape.digest(),
            other.tape.digest(),
            "different goals give different receipts"
        );
        let again = FixtureMode::boot(&moved).run().expect("moved runs");
        assert_eq!(other, again, "moved rules still replay exactly");
        base.replay(&rules).expect("base replay");
        other.replay(&moved).expect("moved replay");
    }

    #[test]
    fn camera_tables_share_sim_but_frame_differently() {
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        let near = FixtureCamera {
            height_m: 6.0,
            lead_m: 0.0,
        };
        let far = FixtureCamera {
            height_m: 20.0,
            lead_m: 4.0,
        };
        near.validate().expect("near camera");
        far.validate().expect("far camera");
        assert!(FixtureCamera {
            height_m: 60.0,
            lead_m: 0.0
        }
        .validate()
        .is_err());
        let session = FixtureSession::boot(&rules);
        let positions: Vec<[f32; 2]> = session.positions().to_vec();
        let (near_center, far_center) = (
            near.frame(&positions, rules.goal),
            far.frame(&positions, rules.goal),
        );
        assert_ne!(near_center, far_center);
        assert_eq!(near_center[2], 6.0);
        assert_eq!(far_center[2], 20.0);
        // Two director tables over one rule set: same sim digest, different
        // presentation bytes (cameras as data, not forks).
        let arena_play = FixtureMode::boot_with_camera(&rules, DirectorParams::arena())
            .expect("arena boots")
            .run()
            .expect("arena runs");
        let traversal_play = FixtureMode::boot_with_camera(&rules, DirectorParams::traversal())
            .expect("traversal boots")
            .run()
            .expect("traversal runs");
        assert_eq!(
            arena_play.tape.digest(),
            traversal_play.tape.digest(),
            "camera never enters the sim hash"
        );
        assert_ne!(
            arena_play.frames, traversal_play.frames,
            "tables frame differently"
        );
        assert_eq!(
            arena_play.follow, traversal_play.follow,
            "same follow table gives same follow centers"
        );
    }

    #[test]
    fn boot_from_tables_loads_camera_as_data() {
        let text = fixture_file_text();
        let table = DirectorParams::traversal().encode();
        let from_tables = FixtureMode::boot_from_tables(&text, &table).expect("tables boot");
        let from_api = FixtureMode::boot_from_text(&text).expect("text boots");
        assert_eq!(
            from_tables.run().expect("runs"),
            from_api.run().expect("runs"),
            "table-driven boot equals the preset boot"
        );
        let short = &table[..DIRECTOR_TABLE_BYTES - 1];
        assert!(matches!(
            FixtureMode::boot_from_tables(&text, short),
            Err(FixtureError::BadCameraTable("length"))
        ));
        let mut future = table;
        future[0] = DIRECTOR_TABLE_VERSION + 1;
        assert!(matches!(
            FixtureMode::boot_from_tables(&text, &future),
            Err(FixtureError::BadCameraTable("version"))
        ));
        let mut bad_row = table;
        put_table_f32(&mut bad_row, 9, 500.0);
        assert!(matches!(
            FixtureMode::boot_from_tables(&text, &bad_row),
            Err(FixtureError::BadCamera("base_dist_m"))
        ));
    }

    #[test]
    fn retuned_midrun_mode_still_replays_twice() {
        fn run_retuned() -> FixturePlayback {
            let text = fixture_file_text();
            let rules = FixtureRules::parse(&text).expect("fixture rules parse");
            let mut mode = FixtureMode::boot(&rules);
            for _ in 0..30 {
                assert!(mode.step().expect("tick steps"));
            }
            mode.retune_camera(DirectorParams::arena())
                .expect("mid-run retune");
            assert_eq!(
                mode.retune_camera(DirectorParams {
                    base_dist_m: 0.0,
                    ..DirectorParams::arena()
                }),
                Err(CameraError::BadParam("base_dist_m"))
            );
            mode.run().expect("mode finishes")
        }
        let first = run_retuned();
        let second = run_retuned();
        assert_eq!(first, second, "retuned run replays exactly");
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        first.replay(&rules).expect("retuned tape replays");
        let plain = FixtureMode::boot(&rules).run().expect("plain runs");
        assert_eq!(
            first.tape.digest(),
            plain.tape.digest(),
            "retune steers presentation, never sim"
        );
        assert_ne!(first.frames, plain.frames, "retune visibly reframes");
    }

    #[test]
    fn neighboring_seeds_never_alias() {
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        let mut even = rules.clone();
        even.seed = 78;
        let mut odd = rules.clone();
        odd.seed = 79;
        let even_pos = FixtureSession::boot(&even).positions().to_vec();
        let odd_pos = FixtureSession::boot(&odd).positions().to_vec();
        assert_ne!(even_pos, odd_pos, "seeds draw disjoint spawns");
        assert_ne!(
            FixtureMode::boot(&even)
                .run()
                .expect("even runs")
                .tape
                .digest(),
            FixtureMode::boot(&odd)
                .run()
                .expect("odd runs")
                .tape
                .digest(),
        );
    }

    #[test]
    fn bad_fixture_text_fails_with_named_errors() {
        let base = fixture_file_text();
        let cases: [(&str, FixtureError); 11] = [
            (
                &base
                    .replace("\"version\": 2", "\"version\": 3")
                    .replace("\"version\":2", "\"version\":3"),
                FixtureError::BadVersion(3),
            ),
            (
                &base
                    .replace("\"mode\": \"tag\"", "\"mode\": \"duel\"")
                    .replace("\"mode\":\"tag\"", "\"mode\":\"duel\""),
                FixtureError::UnknownMode,
            ),
            (
                &base
                    .replace("\"ticks\": 120", "\"ticks\": 0")
                    .replace("\"ticks\":120", "\"ticks\":0"),
                FixtureError::BadTicks(0),
            ),
            (
                &base
                    .replace("\"runners\": 4", "\"runners\": 1")
                    .replace("\"runners\":4", "\"runners\":1"),
                FixtureError::BadRunners(1),
            ),
            (
                &base
                    .replace("[8.0, 6.0]", "[99.0, 0.0]")
                    .replace("[8,6]", "[99.0, 0.0]"),
                FixtureError::BadGoal,
            ),
            (
                &base
                    .replace("\"height_m\": 12.0", "\"height_m\": 60.0")
                    .replace("\"height_m\":12", "\"height_m\":60.0"),
                FixtureError::BadCamera("height_m"),
            ),
            (
                &base
                    .replace("\"tick\": 10", "\"tick\": 70")
                    .replace("\"tick\":10", "\"tick\":70"),
                FixtureError::Unordered,
            ),
            (
                &base
                    .replace("\"vx\": 3.0", "\"vx\": 99.0")
                    .replace("\"vx\":3", "\"vx\":99.0"),
                FixtureError::BadVelocity,
            ),
            (
                &base
                    .replace("\"body\": 2", "\"body\": 9")
                    .replace("\"body\":2", "\"body\":9"),
                FixtureError::OutOfRange,
            ),
            ("{\"version\": 2}", FixtureError::MissingKey("mode")),
            (
                "{\"version\": 2, \"mode\": \"tag\", \"seed\": \"hot\"}",
                FixtureError::BadValue("seed"),
            ),
        ];
        for (text, want) in cases {
            assert_eq!(
                FixtureRules::parse(text),
                Err(want),
                "text accepted: {text}"
            );
        }
        // Minimal but complete rule set still boots (key order free).
        let minimal = "{\"version\": 2, \"mode\": \"tag\", \"seed\": 77, \"ticks\": 120, \
            \"runners\": 4, \"goal_m\": [8.0, 6.0], \"goal_radius_m\": 1.5, \
            \"half_extent_m\": 20.0, \"camera\": {\"height_m\": 12.0, \"lead_m\": 2.0}, \
            \"inputs\": [{\"tick\": 10, \"body\": 0, \"vx\": 3.0, \"vy\": 2.0}]}";
        let rules = FixtureRules::parse(minimal).expect("minimal rules parse");
        let playback = FixtureMode::boot(&rules).run().expect("minimal runs");
        assert_eq!(playback.tape.checkpoints.len(), 120);
        playback.replay(&rules).expect("minimal replay");
        // Empty script, truncated camera and truncated input fail by name.
        let empty_inputs = minimal.replace(
            "\"inputs\": [{\"tick\": 10, \"body\": 0, \"vx\": 3.0, \"vy\": 2.0}]",
            "\"inputs\": []",
        );
        assert_eq!(
            FixtureRules::parse(&empty_inputs),
            Err(FixtureError::EmptyScript)
        );
        // Truncated blocks fail by name (full prefixes so parsing reaches
        // the truncated site instead of an earlier missing key).
        let prefix = "{\"version\": 2, \"mode\": \"tag\", \"seed\": 77, \"ticks\": 120, \
            \"runners\": 4, \"goal_m\": [8.0, 6.0], \"goal_radius_m\": 1.5, \
            \"half_extent_m\": 20.0, ";
        assert_eq!(
            FixtureRules::parse(&[prefix, "\"camera\": {\"height_m\": 12.0"].concat()),
            Err(FixtureError::BadRules("camera"))
        );
        assert_eq!(
            FixtureRules::parse(
                &[
                    prefix,
                    "\"camera\": {\"height_m\": 12.0, \"lead_m\": 2.0}, \"inputs\": [{\"tick\": 10"
                ]
                .concat()
            ),
            Err(FixtureError::BadRules("inputs"))
        );
        assert!(format!("{}", FixtureError::BadVersion(3)).contains('3'));
        assert!(format!("{}", FixtureError::UnknownMode).contains("tag"));
    }

    #[test]
    fn replay_names_first_divergence_and_count() {
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        let mut playback = FixtureMode::boot(&rules).run().expect("mode runs");
        playback.tape.checkpoints[40].hash ^= 1;
        let fresh = FixtureMode::boot(&rules).run().expect("mode runs");
        assert_eq!(
            playback.replay(&rules),
            Err(FixtureError::Divergence {
                tick: 40,
                expected: playback.tape.checkpoints[40].hash,
                actual: fresh.tape.checkpoints[40].hash,
            })
        );
        playback.tape.checkpoints.pop();
        assert_eq!(playback.replay(&rules), Err(FixtureError::Count));
        // A changed win under the same script fails on the win outcome: graft
        // the fresh checkpoints onto the moved tape and replay under the base
        // rules (count and ticks match, so only the win can fail).
        let mut moved = rules.clone();
        moved.goal = [-8.0, -6.0];
        let moved_play = FixtureMode::boot(&moved).run().expect("moved runs");
        let mut cross = moved_play.clone();
        cross.tape.checkpoints = fresh.tape.checkpoints.clone();
        if fresh.tape.win_tick != moved_play.tape.win_tick
            || fresh.tape.winner != moved_play.tape.winner
        {
            assert!(matches!(cross.replay(&rules), Err(FixtureError::Win(_))));
        } else {
            cross.replay(&rules).expect("same win replays clean");
        }
    }

    // -- traversal-course rule sets -----------------------------------------

    /// Read the real GE1 traversal-course rule set (proves the mode boots
    /// with no engine edits; fails loudly naming the path when it moves).
    fn traversal_file_text() -> String {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("games")
            .join("traversal-course")
            .join("rules.json");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    #[test]
    fn course_camera_table_maps_follow_geometry() {
        let camera = CourseCamera {
            distance_m: 6.0,
            height_m: 3.0,
        };
        camera.validate().expect("course camera");
        let table = camera.director_table();
        table.validate().expect("derived table is valid");
        assert_eq!(table.base_dist_m, 6.0);
        assert_eq!(table.fit_margin_m, 0.0);
        assert_eq!(table.boom_back, 1.0);
        assert_eq!(table.boom_up, 0.5);
        // Eye sits exactly height over and distance behind the look point.
        let eye_len = (6.0f32 * 6.0 + 3.0 * 3.0).sqrt();
        assert!((table.max_dist_m - eye_len * 1.1).abs() <= 1.0e-6);
        // Steep, flat and non-finite tables refuse with their row.
        assert_eq!(
            CourseCamera {
                distance_m: 0.0,
                height_m: 3.0
            }
            .validate(),
            Err(FixtureError::BadCamera("distance_m"))
        );
        assert_eq!(
            CourseCamera {
                distance_m: 6.0,
                height_m: 60.0
            }
            .validate(),
            Err(FixtureError::BadCamera("height_m"))
        );
        assert!(CourseCamera {
            distance_m: 6.0,
            height_m: f32::NAN
        }
        .validate()
        .is_err());
    }

    #[test]
    fn course_boots_traversal_file_frames_agent_every_tick_and_replays() {
        let text = traversal_file_text();
        let rules = CourseRules::parse(&text).expect("course rules parse");
        assert_eq!(rules.version, COURSE_RULES_VERSION);
        assert!(rules.ticks >= 600, "accept bar: 600 framed ticks");
        assert_eq!(
            rules.camera,
            CourseCamera {
                distance_m: 6.0,
                height_m: 3.0
            }
        );
        assert_eq!(rules.ledges.len(), 3);
        assert_eq!(rules.moves.len(), 2);
        let params = rules.camera.director_table();
        params.validate().expect("derived table is valid");
        let first = CourseMode::boot_from_text(&text)
            .expect("course boots")
            .run()
            .expect("course runs");
        let second = CourseMode::boot_from_text(&text)
            .expect("course boots")
            .run()
            .expect("course runs");
        assert_eq!(first, second, "course replays exactly");
        assert_eq!(first.tape.checkpoints.len(), rules.ticks as usize);
        assert_eq!(first.frames.len(), rules.ticks as usize);
        assert_eq!(first.agents.len(), rules.ticks as usize);
        // The rule set's own chase camera keeps its agent framed every tick.
        for (index, plane) in first.agents.iter().enumerate() {
            let agent = FighterView {
                id: 0,
                pos_m: Vec3::new(plane[0], 1.0, plane[1]),
                alive: true,
            };
            assert_framed(&first.frames[index], &agent, &params, index as u32);
        }
        assert!(
            first.frames.first() != first.frames.last(),
            "camera tracked the course"
        );
        let mut hashes: Vec<u64> = first
            .tape
            .checkpoints
            .iter()
            .map(|point| point.hash)
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "course never moved");
        assert_eq!(first.tape.win_tick, None, "open course sets no win");
        first.replay(&rules).expect("course replay");
    }

    #[test]
    fn different_ledges_give_a_different_course() {
        let text = traversal_file_text();
        let rules = CourseRules::parse(&text).expect("course rules parse");
        let mut moved = rules.clone();
        moved.ledges[0].top_m += 1.0;
        let base = CourseMode::boot(&rules)
            .expect("base boots")
            .run()
            .expect("base runs");
        let other = CourseMode::boot(&moved)
            .expect("moved boots")
            .run()
            .expect("moved runs");
        assert_ne!(
            base.tape.digest(),
            other.tape.digest(),
            "different furniture gives different receipts"
        );
        let again = CourseMode::boot(&moved)
            .expect("moved boots")
            .run()
            .expect("moved runs");
        assert_eq!(other, again, "moved course still replays exactly");
        base.replay(&rules).expect("base replay");
        other.replay(&moved).expect("moved replay");
    }

    #[test]
    fn schemas_never_cross_boot() {
        let tag_text = fixture_file_text();
        let course_text = traversal_file_text();
        assert!(matches!(
            FixtureRules::parse(&course_text),
            Err(FixtureError::BadVersion(3))
        ));
        assert!(matches!(
            CourseRules::parse(&tag_text),
            Err(FixtureError::BadVersion(2))
        ));
    }

    #[test]
    fn bad_course_text_fails_with_named_errors() {
        let minimal = "{\"version\": 3, \"fixture\": \"traversal-course\", \"mode\": \"course\", \
            \"seed\": 47, \"ticks\": 600, \"runners\": 1, \"run_speed_mps\": 4.0, \
            \"camera\": {\"distance_m\": 6.0, \"height_m\": 3.0}, \
            \"ledges\": [{\"x\": 8.0, \"top_mm\": 900}], \
            \"moves\": [{\"tick\": 100, \"body\": 0, \"vx\": 5.0, \"vy\": 0.0}]}";
        let rules = CourseRules::parse(minimal).expect("minimal course parse");
        let playback = CourseMode::boot(&rules)
            .expect("minimal boots")
            .run()
            .expect("minimal runs");
        assert_eq!(playback.tape.checkpoints.len(), 600);
        playback.replay(&rules).expect("minimal replay");
        // An empty tweak script still simulates (GE1 mirror: coasting course).
        let coasting = minimal.replace(
            "\"moves\": [{\"tick\": 100, \"body\": 0, \"vx\": 5.0, \"vy\": 0.0}]",
            "\"moves\": []",
        );
        let coasting_rules = CourseRules::parse(&coasting).expect("coasting parses");
        let coasting_play = CourseMode::boot(&coasting_rules)
            .expect("coasting boots")
            .run()
            .expect("coasting runs");
        coasting_play
            .replay(&coasting_rules)
            .expect("coasting replay");
        assert_ne!(
            playback.tape.digest(),
            coasting_play.tape.digest(),
            "script changes the receipts"
        );
        let cases: [(&str, FixtureError); 10] = [
            (
                &minimal.replace("\"version\": 3", "\"version\": 2"),
                FixtureError::BadVersion(2),
            ),
            (
                &minimal.replace("\"mode\": \"course\"", "\"mode\": \"tag\""),
                FixtureError::UnknownMode,
            ),
            (
                &minimal.replace(
                    "\"fixture\": \"traversal-course\"",
                    "\"fixture\": \"other\"",
                ),
                FixtureError::BadValue("fixture"),
            ),
            (
                &minimal.replace("\"runners\": 1", "\"runners\": 0"),
                FixtureError::BadRunners(0),
            ),
            (
                &minimal.replace("\"run_speed_mps\": 4.0", "\"run_speed_mps\": 99.0"),
                FixtureError::BadSpeed,
            ),
            (
                &minimal.replace("\"distance_m\": 6.0", "\"distance_m\": 60.0"),
                FixtureError::BadCamera("distance_m"),
            ),
            (
                &minimal.replace(
                    "\"ledges\": [{\"x\": 8.0, \"top_mm\": 900}]",
                    "\"ledges\": []",
                ),
                FixtureError::BadLedges(0),
            ),
            (
                &minimal.replace("\"top_mm\": 900", "\"top_mm\": 50"),
                FixtureError::BadLedge(0),
            ),
            (
                &minimal.replace("\"tick\": 100", "\"tick\": 700"),
                FixtureError::OutOfRange,
            ),
            (
                &minimal.replace("\"x\": 8.0", "\"x\": \"far\""),
                FixtureError::BadValue("x"),
            ),
        ];
        for (text, want) in cases {
            assert_eq!(CourseRules::parse(text), Err(want), "text accepted: {text}");
        }
        assert!(format!("{}", FixtureError::BadSpeed).contains("speed"));
        assert!(format!("{}", FixtureError::BadLedge(2)).contains('2'));
    }

    // GP19 staged rig: 600-tick orbit, bypass framing, replay, 0 allocs.
    fn run_staged_orbit(bypass: u8, trauma_every: u32) -> Vec<[u8; FRAME_BYTES]> {
        let params = DirectorParams::default();
        let mut rig = StagedRig::new(0, params).expect("rig builds");
        rig.set_bypass(bypass);
        assert_eq!(rig.bypass(), bypass);
        assert_eq!(rig.focus_id(), 0);
        assert_eq!(rig.stage_names(), ["follow", "lock", "shake"]);
        assert_eq!(
            RIG_STAGES,
            [RigStage::Follow, RigStage::Lock, RigStage::Shake]
        );
        let mut bytes = Vec::with_capacity(600);
        for tick in 0..600u32 {
            if trauma_every != 0 && tick.is_multiple_of(trauma_every) {
                rig.add_trauma(60);
            }
            let pair = scripted_pair(tick);
            let frame = rig.update(&pair, tick).expect("frame");
            if bypass & BYPASS_LOCK == 0 && tick > 0 {
                assert_eq!(frame.lock, Some(1), "rival locked at tick {tick}");
            }
            if bypass & BYPASS_LOCK != 0 {
                assert_eq!(frame.lock, None, "bypassed lock reports none");
                assert_eq!(rig.lock(), None);
            }
            if bypass & BYPASS_SHAKE != 0 {
                assert_eq!(frame.shake_m, Vec3::ZERO, "bypassed shake is zero");
            }
            for fighter in pair {
                assert_framed(&frame, &fighter, &params, tick);
            }
            bytes.push(frame.to_bytes());
        }
        bytes
    }

    #[test]
    fn gp19_staged_rig_frames_both_every_tick_600() {
        let bytes = run_staged_orbit(0, 0);
        assert_eq!(bytes.len(), 600);
        assert!(bytes.first() != bytes.last(), "rig tracked the orbit");
        // No bypass matches the frozen director byte for byte.
        let params = DirectorParams::default();
        let mut director = Director::new(0, params).expect("director builds");
        for (tick, want) in bytes.iter().enumerate() {
            let pair = scripted_pair(tick as u32);
            let frame = director.update(&pair, tick as u32).expect("frame");
            assert_eq!(&frame.to_bytes(), want, "rig diverges at tick {tick}");
        }
    }

    #[test]
    fn gp19_bypass_one_stage_still_frames() {
        run_staged_orbit(BYPASS_LOCK, 0);
        run_staged_orbit(BYPASS_SHAKE, 0);
        run_staged_orbit(BYPASS_LOCK | BYPASS_SHAKE, 0);
    }

    #[test]
    fn gp19_trauma_replays_exactly_twice() {
        let first = run_staged_orbit(0, 7);
        let second = run_staged_orbit(0, 7);
        assert_eq!(first, second, "trauma track replays exactly");
        let calm = run_staged_orbit(0, 0);
        assert_ne!(first, calm, "trauma moved the receipts");
    }

    #[test]
    fn gp19_stage_maps_are_pure_and_named() {
        let params = DirectorParams::default();
        let focus = Vec3::new(0.0, 1.0, 0.0);
        let anchor = Vec3::new(2.0, 1.0, 0.0);
        let a = follow_stage(focus, Some(anchor), &params).expect("follow");
        let b = follow_stage(focus, Some(anchor), &params).expect("follow");
        assert_eq!(a, b, "follow stage is pure");
        let fwd = (a.look_m - a.eye_m).normalize().expect("view");
        let pair = scripted_pair(9);
        let first = lock_stage(a.eye_m, fwd, &params, 0, &pair).expect("lock");
        let flipped = [pair[1], pair[0]];
        let second = lock_stage(a.eye_m, fwd, &params, 0, &flipped).expect("lock");
        assert_eq!(first, second, "lock ignores container order");
        assert_eq!(
            shake_stage(500, 11, params.max_shake_m),
            shake_stage(500, 11, params.max_shake_m)
        );
        assert_eq!(shake_stage(0, 11, params.max_shake_m), Vec3::ZERO);
        assert_eq!(RigStage::Shake.name(), "shake");
        // Bad tuning refuses at construction with no rig.
        assert!(StagedRig::new(
            0,
            DirectorParams {
                base_dist_m: 0.0,
                ..params
            }
        )
        .is_err());
        let mut rig = StagedRig::new(0, params).expect("rig builds");
        assert!(rig
            .retune(DirectorParams {
                base_dist_m: 0.0,
                ..params
            })
            .is_err());
        assert_eq!(rig.params(), params, "failed retune keeps the table");
    }

    #[test]
    fn gp19_staged_update_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut rig = StagedRig::new(0, DirectorParams::default()).expect("rig builds");
        let pair = scripted_pair(0);
        rig.update(&pair, 0).expect("warm-up frame");
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..600u32 {
                rig.add_trauma(40);
                let pair = scripted_pair(tick);
                let frame = rig.update(&pair, tick).expect("frame");
                let bytes = frame.to_bytes();
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[0..8]);
                folded ^= u64::from_le_bytes(word);
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state staged update must not allocate"
        );
        assert!(folded != 0, "sanity: frames advanced during the window");
    }

    // GP20 intent/smoothed split.
    #[test]
    fn gp20_intent_bypass_is_identical_to_raw() {
        let params = DirectorParams::default();
        let mut rig = StagedRig::new(0, params).expect("rig builds");
        let mut sync = DampedSync::new(250).expect("sync builds");
        assert_eq!(sync.gain_permille(), 250);
        assert_eq!(sync.smoothed(), None);
        for tick in 0..60u32 {
            let pair = scripted_pair(tick);
            let frame = rig.update(&pair, tick).expect("frame");
            let intent = LookIntent::from_frame(&frame);
            assert_eq!(DampedSync::bypass(&intent), intent, "bypass ≡ raw");
            let smoothed = sync.sync(&intent);
            assert!(smoothed.eye_m.is_finite() && smoothed.look_m.is_finite());
        }
        assert!(sync.smoothed().is_some(), "sync latched");
    }

    #[test]
    fn gp20_smoothed_converges_and_replays() {
        let target = LookIntent {
            eye_m: Vec3::new(1.0, 4.0, 6.0),
            look_m: Vec3::ZERO,
        };
        let start = LookIntent {
            eye_m: Vec3::new(-8.0, 0.5, 2.0),
            look_m: Vec3::new(0.0, 1.0, 0.0),
        };
        // Step response: snap to `start`, then chase `target`.
        let run = || {
            let mut sync = DampedSync::new(100).expect("sync builds");
            sync.sync(&start);
            let mut trail = Vec::with_capacity(96);
            for _ in 0..96 {
                trail.push(sync.sync(&target));
            }
            trail
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "smoothing replays exactly");
        let gap0 = (first[0].eye_m - target.eye_m).len();
        assert!(gap0 > 1.0, "step must start far, gap {gap0}");
        // Exponential approach: a third of the way in, the gap is a
        // fraction of the start, and the tail converges.
        let gap32 = (first[32].eye_m - target.eye_m).len();
        assert!(
            gap32 < gap0 * 0.1,
            "smoothing must close in, {gap32} vs {gap0}"
        );
        let gap_last = (first[95].eye_m - target.eye_m).len();
        assert!(
            gap_last < 0.05,
            "96 steps at gain 100 must converge, gap {gap_last}"
        );
        let look_last = (first[95].look_m - target.look_m).len();
        assert!(look_last < 0.05, "look must converge too, gap {look_last}");
        // Full gain snaps every call, bit-exact.
        let mut snap = DampedSync::new(1000).expect("snap builds");
        snap.sync(&start);
        let other = LookIntent {
            eye_m: Vec3::new(-3.0, 2.0, 9.0),
            look_m: Vec3::new(1.0, 1.0, 1.0),
        };
        assert_eq!(snap.sync(&other), other, "gain 1000 snaps");
        // Bad gains refuse with the row name.
        assert_eq!(
            DampedSync::new(0),
            Err(CameraError::BadParam("damping_gain"))
        );
        assert_eq!(
            DampedSync::new(1001),
            Err(CameraError::BadParam("damping_gain"))
        );
    }

    // Trauma curve: jab vs super separation plus restore.
    #[test]
    fn gp21_jab_is_at_most_quarter_super() {
        assert_eq!(TRAUMA_JAB_PERMILLE, 180);
        assert_eq!(TRAUMA_SUPER_PERMILLE, 1000);
        let jab = trauma_gain(TRAUMA_JAB_PERMILLE);
        let super_gain = trauma_gain(TRAUMA_SUPER_PERMILLE);
        assert!((jab - 0.0324).abs() < 1e-6, "jab gain {jab}");
        assert!((super_gain - 1.0).abs() < 1e-6, "super gain {super_gain}");
        assert!(
            jab <= 0.25 * super_gain,
            "curve must separate jab from super"
        );
        // Measured displacements over real ticks keep the same ratio.
        let params = DirectorParams::default();
        let mut jab_peak = 0.0f32;
        let mut super_peak = 0.0f32;
        for tick in 0..600u32 {
            jab_peak =
                jab_peak.max(shake_stage(TRAUMA_JAB_PERMILLE, tick, params.max_shake_m).len());
            super_peak =
                super_peak.max(shake_stage(TRAUMA_SUPER_PERMILLE, tick, params.max_shake_m).len());
        }
        assert!(jab_peak > 0.0 && super_peak > 0.0, "shake must move");
        assert!(
            jab_peak <= 0.25 * super_peak,
            "measured jab {jab_peak} vs super {super_peak}"
        );
        assert_eq!(
            shake_displacement_m(&CameraFrame {
                tick: 0,
                eye_m: Vec3::ZERO,
                look_m: Vec3::ZERO,
                shake_m: Vec3::ZERO,
                lock: None,
                trauma_permille: 0,
            }),
            0.0
        );
    }

    #[test]
    fn gp21_restore_holds_every_tick() {
        // Shake never steers: a trauma-fed rig picks the same lock as a calm
        // one on every tick, and the unshaken eye restores the pose eye.
        let params = DirectorParams::default();
        let mut shaken = StagedRig::new(0, params).expect("rig builds");
        let mut calm = StagedRig::new(0, params).expect("rig builds");
        for tick in 0..600u32 {
            if tick.is_multiple_of(5) {
                shaken.add_trauma(180);
            }
            let pair = scripted_pair(tick);
            let loud = shaken.update(&pair, tick).expect("loud frame");
            let quiet = calm.update(&pair, tick).expect("calm frame");
            assert_eq!(loud.lock, quiet.lock, "shake steered lock at tick {tick}");
            assert_eq!(loud.look_m, quiet.look_m, "shake moved look at tick {tick}");
            let restored = unshaken_eye(&loud);
            let calm_eye = unshaken_eye(&quiet);
            assert!(
                (restored - calm_eye).len() <= 1e-5,
                "restore drift at tick {tick}"
            );
        }
    }

    // Framing fit: two-to-four-fighter orbits plus wall pins, every tick
    // framed with margin over 600 ticks, pair shape matching the frozen
    // director, refusals naming the culprit, sim bytes untouched, 0 allocs.
    use std::f32::consts::{FRAC_PI_2, TAU};

    /// Even phase spread for a two-fighter bout (opposite corners).
    const GROUP_PHASES_2: [f32; 2] = [0.0, TAU * 0.5];
    /// Even phase spread for a three-fighter bout.
    const GROUP_PHASES_3: [f32; 3] = [0.0, TAU / 3.0, TAU * 2.0 / 3.0];
    /// Even phase spread for a four-fighter bout.
    const GROUP_PHASES_4: [f32; 4] = [0.0, FRAC_PI_2, TAU * 0.5, TAU * 1.5 / 2.0];

    /// Scripted bout orbit: radius 5 m at 1 m height, phases spread evenly so
    /// the spread breathes as the bout turns. Writes the first `phases.len()`
    /// slots and returns the fighter count. Pure scratch math, no heap.
    fn scripted_group_into(tick_f: f32, phases: &[f32], out: &mut [FighterView; 4]) -> usize {
        let count = phases.len().min(out.len());
        for (index, slot) in out.iter_mut().enumerate().take(count) {
            let angle = tick_f * 0.02 + phases[index];
            slot.id = u8::try_from(index).expect("scripted bout holds at most four fighters");
            slot.pos_m = Vec3::new(5.0 * angle.cos(), 1.0, 5.0 * angle.sin());
            slot.alive = true;
        }
        count
    }

    /// Scripted wall pin: four corners pinned near opposite walls (`x` near
    /// `+/-9` m, `z` breathing near `+/-6` m), the bout spread the director
    /// pair framing cannot see. Writes all four slots. Pure, no heap.
    fn scripted_wall_pin_into(tick_f: f32, out: &mut [FighterView; 4]) -> usize {
        const CORNERS: [[f32; 2]; 4] = [[9.0, 6.0], [-9.0, 6.0], [9.0, -6.0], [-9.0, -6.0]];
        for (index, slot) in out.iter_mut().enumerate().take(CORNERS.len()) {
            let wobble = tick_f * 0.03 + GROUP_PHASES_4[index];
            slot.id = u8::try_from(index).expect("wall pin holds four fighters");
            slot.pos_m = Vec3::new(
                CORNERS[index][0] + 1.5 * wobble.cos(),
                1.0,
                CORNERS[index][1] + 1.5 * wobble.sin(),
            );
            slot.alive = true;
        }
        CORNERS.len()
    }

    /// Pose as a presented frame with no trauma, so the shared frustum oracle
    /// below checks exactly what the raster view would consume.
    fn frame_of(pose: &CameraPose) -> CameraFrame {
        CameraFrame {
            tick: 0,
            eye_m: pose.eye_m,
            look_m: pose.look_m,
            shake_m: Vec3::ZERO,
            lock: None,
            trauma_permille: 0,
        }
    }

    #[test]
    fn framing_fit_matches_pair_shape_within_one_step() {
        let params = DirectorParams::default();
        for tick in 0..600u32 {
            let pair = scripted_pair(tick);
            let fit = frame_group(&pair, &params).expect("pair fits");
            let again = frame_group(&pair, &params).expect("pair fits");
            assert_eq!(fit, again, "same slices give the same pose at tick {tick}");
            let flipped = [pair[1], pair[0]];
            assert_eq!(
                frame_group(&flipped, &params).expect("flipped fits"),
                fit,
                "shuffled slices frame identically at tick {tick}"
            );
            let frozen =
                frame_pair(pair[0].pos_m, Some(pair[1].pos_m), &params).expect("pair frames");
            assert!(
                (fit.look_m - frozen.look_m).len() <= 1.0e-5,
                "look drifts at tick {tick}"
            );
            assert!(
                (fit.eye_m - frozen.eye_m).len() <= 1.0e-3,
                "eye drifts at tick {tick}"
            );
            assert_framed(&frame_of(&fit), &pair[0], &params, tick);
            assert_framed(&frame_of(&fit), &pair[1], &params, tick);
        }
    }

    /// One bout shape over 600 ticks: every fighter strictly inside the cone
    /// (positive slack is the margin working); range is checked only where the
    /// rails allow it (a wall-wide bout outruns the lock rail while staying in
    /// frame, and the raster consumes the cone, not the rail).
    /// Returns the framed count plus the worst slack met.
    fn run_bout_shape(phases: &[f32], wall_pin: bool, check_range: bool) -> (u32, f32) {
        let params = DirectorParams::default();
        let cos_limit = (params.fov_y_rad * 0.5).cos();
        let max_sq = params.max_dist_m * params.max_dist_m;
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        let mut framed = 0u32;
        let mut worst_slack = f32::INFINITY;
        let mut tick_f = 0.0f32;
        for tick in 0..600u32 {
            let count = if wall_pin {
                scripted_wall_pin_into(tick_f, &mut slot)
            } else {
                scripted_group_into(tick_f, phases, &mut slot)
            };
            let fit = frame_group(&slot[..count], &params).expect("bout fits");
            assert_eq!(
                frame_group(&slot[..count], &params).expect("bout fits"),
                fit,
                "same slices give the same pose at tick {tick}"
            );
            let frame = frame_of(&fit);
            let fwd = (frame.look_m - frame.eye_m)
                .normalize()
                .expect("non-degenerate view");
            for fighter in slot.iter().take(count) {
                let delta = fighter.pos_m - frame.eye_m;
                let dist_sq = delta.len_sq();
                if check_range {
                    assert!(dist_sq <= max_sq, "fighter {} in range", fighter.id);
                }
                let slack = delta.dot(fwd) / dist_sq.sqrt() - cos_limit;
                assert!(slack > 0.0, "fighter {} inside the margin", fighter.id);
                worst_slack = worst_slack.min(slack);
            }
            framed += 1;
            tick_f += 1.0;
        }
        (framed, worst_slack)
    }

    #[test]
    fn framing_fit_orbits_keep_two_three_four_framed() {
        let (two, slack2) = run_bout_shape(&GROUP_PHASES_2, false, true);
        let (three, slack3) = run_bout_shape(&GROUP_PHASES_3, false, true);
        let (four, slack4) = run_bout_shape(&GROUP_PHASES_4, false, true);
        assert_eq!((two, three, four), (600, 600, 600));
        assert!(
            slack2 > 0.0 && slack3 > 0.0 && slack4 > 0.0,
            "margin holds every tick ({slack2}, {slack3}, {slack4})"
        );
    }

    #[test]
    fn framing_fit_wall_pin_spreads_back_into_view() {
        let params = DirectorParams::default();
        let (framed, slack) = run_bout_shape(&GROUP_PHASES_4, true, false);
        assert_eq!(framed, 600);
        assert!(slack > 0.0, "pinned bout keeps its margin ({slack})");
        // The look point stays inside the bout instead of chasing one wall.
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        scripted_wall_pin_into(0.0, &mut slot);
        let fit = frame_group(&slot, &params).expect("pinned bout fits");
        assert!(
            fit.look_m.x.abs() < 9.0 && fit.look_m.z.abs() < 6.0,
            "look stays between the walls, got {:?}",
            fit.look_m
        );
        // The spread pulls the eye back: a wall-wide bout frames farther out
        // than any lone corner, instead of clipping at the frame edge.
        let solo = frame_pair(slot[0].pos_m, None, &params).expect("solo frames");
        let group_reach = (fit.eye_m - fit.look_m).len();
        let solo_reach = (solo.eye_m - solo.look_m).len();
        assert!(
            group_reach > solo_reach,
            "spread must zoom out ({group_reach} vs {solo_reach})"
        );
    }

    #[test]
    fn framing_fit_refusals_name_the_culprit() {
        let params = DirectorParams::default();
        let solo = scripted_pair(0);
        for len in [0usize, 1usize, 5usize] {
            let mut wide = [solo[0]; 5];
            wide[1] = solo[1];
            assert_eq!(
                frame_group(&wide[..len.min(5)], &params),
                Err(CameraError::BadParam("fighters")),
                "count {len} must fail"
            );
        }
        let mut bad_id = scripted_pair(0);
        bad_id[1] = FighterView {
            id: 7,
            pos_m: Vec3::new(f32::NAN, 1.0, 0.0),
            alive: false,
        };
        assert_eq!(
            frame_group(&bad_id, &params),
            Err(CameraError::BadPosition(7)),
            "NaN names its fighter even when dead"
        );
        let bad_tuning = DirectorParams {
            base_dist_m: 0.0,
            ..params
        };
        assert_eq!(
            frame_group(&solo, &bad_tuning),
            Err(CameraError::BadParam("base_dist_m"))
        );
        // The fallen stay in frame: dead fighters are framed, never dropped.
        let mut with_fallen = scripted_pair(3);
        with_fallen[1].alive = false;
        let fit = frame_group(&with_fallen, &params).expect("fallen still framed");
        assert_framed(&frame_of(&fit), &with_fallen[1], &params, 3);
    }

    #[test]
    fn framing_fit_leaves_sim_bytes_untouched() {
        fn run_with_fit() -> (u64, Vec<CameraPose>) {
            let text = fixture_file_text();
            let rules = FixtureRules::parse(&text).expect("fixture rules parse");
            let params = DirectorParams::default();
            let mut mode = FixtureMode::boot(&rules);
            let mut slot = [FighterView {
                id: 0,
                pos_m: Vec3::ZERO,
                alive: true,
            }; 4];
            let mut poses = Vec::new();
            let mut step = 0u32;
            let mut step_f = 0.0f32;
            while !mode.is_done() {
                let phases: &[f32] = match step % 3 {
                    0 => &GROUP_PHASES_2,
                    1 => &GROUP_PHASES_3,
                    _ => &GROUP_PHASES_4,
                };
                let count = scripted_group_into(step_f, phases, &mut slot);
                poses.push(frame_group(&slot[..count], &params).expect("bout fits"));
                mode.step().expect("tick steps");
                step += 1;
                step_f += 1.0;
            }
            let playback = mode.run().expect("mode finishes");
            (playback.tape.digest(), poses)
        }
        let text = fixture_file_text();
        let rules = FixtureRules::parse(&text).expect("fixture rules parse");
        let plain = FixtureMode::boot(&rules).run().expect("plain runs");
        let (first_digest, first_poses) = run_with_fit();
        let (second_digest, second_poses) = run_with_fit();
        assert_eq!(first_digest, plain.tape.digest(), "fit steers no sim byte");
        assert_eq!(second_digest, first_digest, "sim bytes replay exactly");
        assert_eq!(first_poses, second_poses, "same bouts give the same poses");
        assert_eq!(first_poses.len(), 120, "fit ran every fixture tick");
    }

    #[test]
    fn framing_fit_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let params = DirectorParams::default();
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        scripted_group_into(0.0, &GROUP_PHASES_4, &mut slot);
        frame_group(&slot, &params).expect("warm-up fit");
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            let mut tick_f = 1.0f32;
            for _ in 1..600u32 {
                let count = scripted_group_into(tick_f, &GROUP_PHASES_3, &mut slot);
                let pose = frame_group(&slot[..count], &params).expect("bout fits");
                folded ^= u64::from(pose.eye_m.x.to_bits());
                folded ^= u64::from(pose.look_m.z.to_bits());
                tick_f += 1.0;
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state framing fit must not allocate"
        );
        assert!(folded != 0, "sanity: poses advanced during the window");
    }

    // Follow-camera acceptance: the director frames the whole bout (group fit)
    // with zero unframed over a 120-tick four-fighter run, a clamped rail
    // reports unit arrows instead of silence, refusals name the culprit, and
    // steady state allocates nothing.
    const AR3_NO_ARROW: OffScreenArrow = OffScreenArrow {
        id: u8::MAX,
        dir_x: 0.0,
        dir_y: 0.0,
    };

    /// Drive the director over 120 scripted four-fighter ticks, asserting the
    /// acceptance oracle every tick: zero unframed, zero arrows. Returns the
    /// frame bytes for the replay gate.
    fn run_ar3_director_120(wall_pin: bool) -> Vec<[u8; FRAME_BYTES]> {
        let params = DirectorParams::default();
        let mut director = Director::new(0, params).expect("director builds");
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        let mut arrows = [AR3_NO_ARROW; 4];
        let mut bytes = Vec::with_capacity(120);
        let mut tick_f = 0.0f32;
        for tick in 0..120u32 {
            if wall_pin {
                scripted_wall_pin_into(tick_f, &mut slot);
            } else {
                scripted_group_into(tick_f, &GROUP_PHASES_4, &mut slot);
            }
            let frame = director.update(&slot, tick).expect("bout frames");
            assert_eq!(
                count_unframed(&frame, &slot, &params).expect("oracle runs"),
                0,
                "all four framed at tick {tick}"
            );
            assert_eq!(
                off_screen_arrows(&frame, &slot, &params, &mut arrows).expect("arrows run"),
                0,
                "no arrows at tick {tick}"
            );
            bytes.push(frame.to_bytes());
            tick_f += 1.0;
        }
        bytes
    }

    #[test]
    fn ar3_director_keeps_four_framed_120_ticks() {
        let orbit = run_ar3_director_120(false);
        let pinned = run_ar3_director_120(true);
        assert_eq!(orbit.len(), 120);
        assert_eq!(pinned.len(), 120);
        assert!(orbit.first() != orbit.last(), "camera tracked the orbit");
        assert!(pinned.first() != pinned.last(), "camera tracked the pin");
        assert_eq!(
            run_ar3_director_120(false),
            orbit,
            "orbit bytes replay exactly"
        );
        assert_eq!(
            run_ar3_director_120(true),
            pinned,
            "pin bytes replay exactly"
        );
    }

    #[test]
    fn ar3_clamped_bout_reports_arrows_not_silence() {
        let params = DirectorParams {
            max_dist_m: 6.0,
            base_dist_m: 4.0,
            ..DirectorParams::default()
        };
        params.validate().expect("clamped table is valid");
        let mut director = Director::new(0, params).expect("director builds");
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        scripted_wall_pin_into(0.0, &mut slot);
        let frame = director.update(&slot, 0).expect("clamped bout frames");
        let missing = count_unframed(&frame, &slot, &params).expect("oracle runs");
        assert!(missing > 0, "clamped rails cannot fit the pin");
        let mut arrows = [AR3_NO_ARROW; 4];
        assert_eq!(
            off_screen_arrows(&frame, &slot, &params, &mut arrows).expect("arrows run"),
            missing,
            "one arrow per unframed fighter"
        );
        for (index, arrow) in arrows.iter().enumerate().take(missing) {
            assert_eq!(arrow.id, slot[index].id, "arrows follow slice order");
            let len = (arrow.dir_x * arrow.dir_x + arrow.dir_y * arrow.dir_y).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-5,
                "arrow {} is unit ({}, {})",
                arrow.id,
                arrow.dir_x,
                arrow.dir_y
            );
        }
        assert_eq!(
            run_ar3_director_120(false).len(),
            120,
            "clamped rails never touch the default fit"
        );
    }

    #[test]
    fn ar3_arrows_point_off_screen_with_unit_directions() {
        let params = DirectorParams::default();
        let eye = Vec3::new(0.0, 4.5, 10.0);
        let look = Vec3::ZERO;
        let frame = CameraFrame {
            tick: 0,
            eye_m: eye,
            look_m: look,
            shake_m: Vec3::ZERO,
            lock: None,
            trauma_permille: 0,
        };
        let centered = FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        };
        assert_eq!(
            fighter_framing(&frame, &centered, &params),
            Ok(None),
            "the look point is framed"
        );
        assert_eq!(
            fighter_framing(
                &frame,
                &FighterView {
                    id: 1,
                    pos_m: eye,
                    alive: true,
                },
                &params
            ),
            Ok(None),
            "the camera sits inside the fighter"
        );
        let right_out = FighterView {
            id: 2,
            pos_m: Vec3::new(50.0, 4.5, 5.0),
            alive: true,
        };
        let arrow = fighter_framing(&frame, &right_out, &params)
            .expect("oracle runs")
            .expect("far side is out");
        assert_eq!(arrow.id, 2, "arrows carry the fighter id");
        assert!(
            arrow.dir_x > 0.9,
            "side arrow points right ({})",
            arrow.dir_x
        );
        assert!(
            arrow.dir_y.abs() < 0.5,
            "side arrow stays level ({})",
            arrow.dir_y
        );
        let behind = FighterView {
            id: 3,
            pos_m: Vec3::new(0.0, 4.5, 15.0),
            alive: true,
        };
        let back_arrow = fighter_framing(&frame, &behind, &params)
            .expect("oracle runs")
            .expect("behind the eye is out");
        let back_len =
            (back_arrow.dir_x * back_arrow.dir_x + back_arrow.dir_y * back_arrow.dir_y).sqrt();
        assert!((back_len - 1.0).abs() < 1e-5, "behind arrow is unit");
        let squad = [centered, right_out, behind];
        assert_eq!(
            count_unframed(&frame, &squad, &params).expect("oracle runs"),
            2,
            "only the centered fighter is framed"
        );
        let mut arrows = [AR3_NO_ARROW; 3];
        assert_eq!(
            off_screen_arrows(&frame, &squad, &params, &mut arrows).expect("arrows run"),
            2
        );
        assert_eq!(
            [arrows[0].id, arrows[1].id],
            [2, 3],
            "arrows follow slice order"
        );
        assert_eq!(arrows[2], AR3_NO_ARROW, "framed slots stay untouched");
    }

    #[test]
    fn ar3_framing_refusals_name_the_culprit() {
        let params = DirectorParams::default();
        let pair = scripted_pair(0);
        let mut director = Director::new(0, params).expect("director builds");
        let frame = director.update(&pair, 0).expect("pair frames");
        let bad_tuning = DirectorParams {
            fov_y_rad: 0.0,
            ..params
        };
        assert_eq!(
            fighter_framing(&frame, &pair[0], &bad_tuning),
            Err(CameraError::BadParam("fov_y_rad"))
        );
        assert_eq!(
            count_unframed(&frame, &pair, &bad_tuning),
            Err(CameraError::BadParam("fov_y_rad"))
        );
        let nan = FighterView {
            id: 7,
            pos_m: Vec3::new(f32::NAN, 1.0, 0.0),
            alive: false,
        };
        assert_eq!(
            fighter_framing(&frame, &nan, &params),
            Err(CameraError::BadPosition(7)),
            "NaN names its fighter even when dead"
        );
        assert_eq!(
            count_unframed(&frame, &[pair[0], nan], &params),
            Err(CameraError::BadPosition(7))
        );
        let mut one = [AR3_NO_ARROW; 1];
        assert_eq!(
            off_screen_arrows(&frame, &pair, &params, &mut one),
            Err(CameraError::BadParam("arrows")),
            "a short buffer fails before any work"
        );
        assert_eq!(one, [AR3_NO_ARROW], "a refused fill writes nothing");
        let mut no_eye = frame;
        no_eye.eye_m.x = f32::NAN;
        assert_eq!(
            fighter_framing(&no_eye, &pair[0], &params),
            Err(CameraError::BadEye)
        );
        let mut no_look = frame;
        no_look.look_m.z = f32::INFINITY;
        assert_eq!(
            fighter_framing(&no_look, &pair[0], &params),
            Err(CameraError::BadForward)
        );
    }

    #[test]
    fn ar3_framing_oracle_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let params = DirectorParams::default();
        let mut slot = [FighterView {
            id: 0,
            pos_m: Vec3::ZERO,
            alive: true,
        }; 4];
        scripted_group_into(0.0, &GROUP_PHASES_4, &mut slot);
        let mut director = Director::new(0, params).expect("director builds");
        director.update(&slot, 0).expect("warm-up frame");
        let mut arrows = [AR3_NO_ARROW; 4];
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            let mut tick_f = 1.0f32;
            for tick in 1..600u32 {
                scripted_group_into(tick_f, &GROUP_PHASES_4, &mut slot);
                let frame = director.update(&slot, tick).expect("bout frames");
                folded ^=
                    u64::try_from(count_unframed(&frame, &slot, &params).expect("oracle runs"))
                        .expect("counts fit in u64");
                folded ^= u64::try_from(
                    off_screen_arrows(&frame, &slot, &params, &mut arrows).expect("arrows run"),
                )
                .expect("counts fit in u64");
                if let Some(arrow) =
                    fighter_framing(&frame, &slot[0], &params).expect("status runs")
                {
                    folded ^= u64::from(arrow.dir_x.to_bits());
                }
                folded ^= u64::from(frame.eye_m.x.to_bits());
                tick_f += 1.0;
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state framing oracle must not allocate"
        );
        assert!(folded != 0, "sanity: frames advanced during the window");
    }
}
