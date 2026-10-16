//! face: GPU morph weights plus jaw, eye and brow rig over baked visemes.
//!
//! Owner: `engine-anim`; consumers: the skin pass reads [`MorphUniform`]
//! bytes as a storage buffer, audio starts [`bake_line`] clips on event
//! ticks. Units: unitless weights in `0..=1`, seconds, meters on the CPU
//! oracle. Determinism: `+ - * /` and min/max only, fixed lane order, no
//! clock; same clip plus time plus rig replays bit-exact. Errors:
//! [`FaceError`]. Budget: per-frame sample plus rig is fifty-two lanes of
//! lerp and clamps with no allocation; shade cost rides [`morph_cost_us`].

use crate::clip::clamp01;
use crate::lipsync::{self, BakeError};
use crate::viseme::{self, VisemeClip};
use std::fmt;

/// Morph lane count; equals the viseme lane count by construction.
pub const MAX_MORPHS: usize = viseme::N;
/// Largest morph count the GPU path uploads; matches [`MAX_MORPHS`] so a
/// baked line never needs lane surgery before upload.
pub const MAX_GPU_MORPHS: usize = MAX_MORPHS;
/// Uniform image size in bytes: one little-endian `f32` per lane.
pub const UNIFORM_BYTES: usize = MAX_MORPHS * 4;
/// Weight floor that counts as active for sparse dispatch and cost.
pub const ACTIVE_MIN: f32 = 0.004;
/// Tier-B head vertex count the posted cost is evaluated at.
pub const TIER_B_HEAD_VERTS: u16 = 8000;
/// Vertex-shader cost model: base microseconds per head per frame.
pub const MORPH_BASE_US: f32 = 1.5;
/// Vertex-shader cost model: microseconds per vertex per active morph.
pub const MORPH_PER_VERT_MORPH_US: f32 = 0.0015;
/// Posted frame budget for the full morph shade of one head.
pub const MORPH_BUDGET_US: f32 = 700.0;

/// Brow-down left lane in [`viseme::NAMES`] order.
pub const BROW_DOWN_LEFT: usize = 0;
/// Brow-down right lane in [`viseme::NAMES`] order.
pub const BROW_DOWN_RIGHT: usize = 1;
/// Brow-inner-up lane in [`viseme::NAMES`] order.
pub const BROW_INNER_UP: usize = 2;
/// Brow-outer-up left lane in [`viseme::NAMES`] order.
pub const BROW_OUTER_UP_LEFT: usize = 3;
/// Brow-outer-up right lane in [`viseme::NAMES`] order.
pub const BROW_OUTER_UP_RIGHT: usize = 4;
/// Eye-blink left lane in [`viseme::NAMES`] order.
pub const EYE_BLINK_LEFT: usize = 8;
/// Eye-blink right lane in [`viseme::NAMES`] order.
pub const EYE_BLINK_RIGHT: usize = 9;
/// Eye-look-down left lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_DOWN_LEFT: usize = 10;
/// Eye-look-down right lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_DOWN_RIGHT: usize = 11;
/// Eye-look-in left lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_IN_LEFT: usize = 12;
/// Eye-look-in right lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_IN_RIGHT: usize = 13;
/// Eye-look-out left lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_OUT_LEFT: usize = 14;
/// Eye-look-out right lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_OUT_RIGHT: usize = 15;
/// Eye-look-up left lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_UP_LEFT: usize = 16;
/// Eye-look-up right lane in [`viseme::NAMES`] order.
pub const EYE_LOOK_UP_RIGHT: usize = 17;
/// Jaw-open lane in [`viseme::NAMES`] order.
pub const JAW_OPEN: usize = 25;

/// Face failures: every variant names the lane or sizes at fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaceError {
    /// Weight slice length differs from [`MAX_MORPHS`]; expected + got.
    LengthMismatch {
        /// Expected lane count.
        expected: usize,
        /// Slice length found.
        got: usize,
    },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Rig field out of range; field name carried.
    BadRig(&'static str),
    /// Upload weight outside `0..=1`; lane carried.
    BadWeight {
        /// Morph lane index.
        lane: usize,
    },
}

impl FaceError {
    /// Stable message for logs and receipts.
    pub fn as_str(self) -> &'static str {
        match self {
            FaceError::LengthMismatch { .. } => "length-mismatch",
            FaceError::NonFinite(_) => "non-finite",
            FaceError::BadRig(_) => "bad-rig",
            FaceError::BadWeight { .. } => "bad-weight",
        }
    }
}

impl fmt::Display for FaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            FaceError::LengthMismatch { expected, got } => {
                write!(f, "face wants {expected} lanes, found {got}")
            }
            FaceError::NonFinite(lane) => write!(f, "face {lane} is not finite"),
            FaceError::BadRig(field) => write!(f, "face rig field {field} is out of range"),
            FaceError::BadWeight { lane } => write!(f, "face weight {lane} is outside 0..=1"),
        }
    }
}

impl std::error::Error for FaceError {}

/// Procedural jaw, eye and brow rig layered over a sampled viseme.
///
/// Neutral is identity: jaw scale one, everything else zero. Look axes run
/// `-1..=1` (right/up positive); brow runs `-1..=1` (lift positive, knit
/// negative); jaw scale runs `0..=2`, jaw boost and blinks `0..=1`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FaceRig {
    /// Jaw-open multiplier; zero mutes speech jaw, two doubles it.
    pub jaw_scale: f32,
    /// Flat jaw-open add after the scale; keeps whisper lines readable.
    pub jaw_boost: f32,
    /// Left blink override; takes the max with the viseme lane.
    pub blink_left: f32,
    /// Right blink override; takes the max with the viseme lane.
    pub blink_right: f32,
    /// Gaze left/right; right positive.
    pub look_x: f32,
    /// Gaze up/down; up positive.
    pub look_y: f32,
    /// Brow lift (positive) or knit (negative).
    pub brow: f32,
}

impl FaceRig {
    /// Identity rig: sampled visemes pass through untouched.
    pub fn neutral() -> Self {
        Self {
            jaw_scale: 1.0,
            jaw_boost: 0.0,
            blink_left: 0.0,
            blink_right: 0.0,
            look_x: 0.0,
            look_y: 0.0,
            brow: 0.0,
        }
    }

    /// Reject non-finite fields and out-of-range controls loudly.
    pub fn validate(&self) -> Result<(), FaceError> {
        if !self.jaw_scale.is_finite()
            || !self.jaw_boost.is_finite()
            || !self.blink_left.is_finite()
            || !self.blink_right.is_finite()
            || !self.look_x.is_finite()
            || !self.look_y.is_finite()
            || !self.brow.is_finite()
        {
            return Err(FaceError::NonFinite("face.rig"));
        }
        if !(0.0..=2.0).contains(&self.jaw_scale) {
            return Err(FaceError::BadRig("jaw_scale"));
        }
        if !(0.0..=1.0).contains(&self.jaw_boost) {
            return Err(FaceError::BadRig("jaw_boost"));
        }
        if !(0.0..=1.0).contains(&self.blink_left) {
            return Err(FaceError::BadRig("blink_left"));
        }
        if !(0.0..=1.0).contains(&self.blink_right) {
            return Err(FaceError::BadRig("blink_right"));
        }
        if !(-1.0..=1.0).contains(&self.look_x) {
            return Err(FaceError::BadRig("look_x"));
        }
        if !(-1.0..=1.0).contains(&self.look_y) {
            return Err(FaceError::BadRig("look_y"));
        }
        if !(-1.0..=1.0).contains(&self.brow) {
            return Err(FaceError::BadRig("brow"));
        }
        Ok(())
    }
}

/// Sample one face pose at `t` seconds into `out`.
///
/// Thin slice wrapper over [`viseme::sample`]: endpoints exact, empty or
/// ragged clips and `NaN` times yield zeros. Reuses the caller buffer, so
/// steady-state frames allocate nothing.
pub fn sample_face(clip: &VisemeClip, t: f32, out: &mut [f32]) -> Result<(), FaceError> {
    if out.len() != MAX_MORPHS {
        return Err(FaceError::LengthMismatch {
            expected: MAX_MORPHS,
            got: out.len(),
        });
    }
    let sampled = viseme::sample(clip, t);
    out.copy_from_slice(&sampled);
    Ok(())
}

fn apply_jaw(weights: &mut [f32], rig: &FaceRig) {
    weights[JAW_OPEN] = clamp01(weights[JAW_OPEN] * rig.jaw_scale + rig.jaw_boost);
    weights[EYE_BLINK_LEFT] = clamp01(weights[EYE_BLINK_LEFT].max(rig.blink_left));
    weights[EYE_BLINK_RIGHT] = clamp01(weights[EYE_BLINK_RIGHT].max(rig.blink_right));
}

fn apply_look(weights: &mut [f32], rig: &FaceRig) {
    let up = rig.look_y.max(0.0);
    let down = (-rig.look_y).max(0.0);
    let right = rig.look_x.max(0.0);
    let left = (-rig.look_x).max(0.0);
    weights[EYE_LOOK_UP_LEFT] = clamp01(weights[EYE_LOOK_UP_LEFT].max(up));
    weights[EYE_LOOK_UP_RIGHT] = clamp01(weights[EYE_LOOK_UP_RIGHT].max(up));
    weights[EYE_LOOK_DOWN_LEFT] = clamp01(weights[EYE_LOOK_DOWN_LEFT].max(down));
    weights[EYE_LOOK_DOWN_RIGHT] = clamp01(weights[EYE_LOOK_DOWN_RIGHT].max(down));
    weights[EYE_LOOK_OUT_LEFT] = clamp01(weights[EYE_LOOK_OUT_LEFT].max(left));
    weights[EYE_LOOK_IN_LEFT] = clamp01(weights[EYE_LOOK_IN_LEFT].max(right));
    weights[EYE_LOOK_OUT_RIGHT] = clamp01(weights[EYE_LOOK_OUT_RIGHT].max(right));
    weights[EYE_LOOK_IN_RIGHT] = clamp01(weights[EYE_LOOK_IN_RIGHT].max(left));
}

fn apply_brow(weights: &mut [f32], rig: &FaceRig) {
    if rig.brow >= 0.0 {
        let lift = rig.brow;
        let relax = 1.0 - lift;
        weights[BROW_INNER_UP] = clamp01(weights[BROW_INNER_UP].max(lift));
        weights[BROW_OUTER_UP_LEFT] = clamp01(weights[BROW_OUTER_UP_LEFT].max(lift));
        weights[BROW_OUTER_UP_RIGHT] = clamp01(weights[BROW_OUTER_UP_RIGHT].max(lift));
        weights[BROW_DOWN_LEFT] = clamp01(weights[BROW_DOWN_LEFT] * relax);
        weights[BROW_DOWN_RIGHT] = clamp01(weights[BROW_DOWN_RIGHT] * relax);
    } else {
        let knit = -rig.brow;
        let relax = 1.0 - knit;
        weights[BROW_DOWN_LEFT] = clamp01(weights[BROW_DOWN_LEFT].max(knit));
        weights[BROW_DOWN_RIGHT] = clamp01(weights[BROW_DOWN_RIGHT].max(knit));
        weights[BROW_INNER_UP] = clamp01(weights[BROW_INNER_UP] * relax);
        weights[BROW_OUTER_UP_LEFT] = clamp01(weights[BROW_OUTER_UP_LEFT] * relax);
        weights[BROW_OUTER_UP_RIGHT] = clamp01(weights[BROW_OUTER_UP_RIGHT] * relax);
    }
}

/// Layer `rig` over sampled `weights` in place.
///
/// Jaw scale plus boost reshape speech jaw, blinks and gaze take the max
/// with the viseme lanes so baked blinks are never erased, and brow lift
/// relaxes the knit lanes (and vice versa). Validates the rig and every
/// lane first; invalid input errors, never half-applies.
pub fn apply_rig(weights: &mut [f32], rig: &FaceRig) -> Result<(), FaceError> {
    if weights.len() != MAX_MORPHS {
        return Err(FaceError::LengthMismatch {
            expected: MAX_MORPHS,
            got: weights.len(),
        });
    }
    rig.validate()?;
    if weights.iter().any(|v| !v.is_finite()) {
        return Err(FaceError::NonFinite("face.weights"));
    }
    apply_jaw(weights, rig);
    apply_look(weights, rig);
    apply_brow(weights, rig);
    Ok(())
}

/// Voice line with a baked face clip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineId {
    /// Arena announcer callout.
    Announcer,
    /// Taunt line.
    Taunt,
}

impl LineId {
    /// Baked phoneme sheet for the line; authoring data, never inferred.
    pub fn tsv(self) -> &'static str {
        match self {
            LineId::Announcer => ANNOUNCER_TSV,
            LineId::Taunt => TAUNT_TSV,
        }
    }
}

/// Announcer phoneme sheet: open vowels plus a neutral tail.
pub const ANNOUNCER_TSV: &str = "0.00 A\n0.20 B\n0.40 C\n0.60 D\n0.80 E\n1.00 X\n";
/// Taunt phoneme sheet: fricatives and a grin plus a neutral tail.
pub const TAUNT_TSV: &str = "0.00 F\n0.16 G\n0.32 H\n0.48 E\n0.64 B\n0.80 X\n";

/// Bake a voice line offline to a viseme clip.
///
/// Parse plus bake through the lipsync owner at grid `dt`; runtime audio
/// inference stays refused there. Line starts ride the event-track ticks;
/// this call only cooks the data those ticks play.
pub fn bake_line(line: LineId, dt: f32) -> Result<VisemeClip, BakeError> {
    lipsync::bake_tsv(line.tsv(), dt)
}

/// Validated GPU morph weights: one `0..=1` lane per morph target.
///
/// The skin pass uploads [`MorphUniform::write_bytes`] verbatim as a
/// storage buffer in [`viseme::NAMES`] order; bit `i` of
/// [`MorphUniform::active_mask`] carries lane `i` for sparse dispatch.
#[derive(Clone, PartialEq, Debug)]
pub struct MorphUniform {
    weights: [f32; MAX_MORPHS],
}

impl MorphUniform {
    /// Validate `weights` (`0..=1`, finite, [`MAX_MORPHS`] lanes) and pack.
    pub fn upload(weights: &[f32]) -> Result<Self, FaceError> {
        if weights.len() != MAX_MORPHS {
            return Err(FaceError::LengthMismatch {
                expected: MAX_MORPHS,
                got: weights.len(),
            });
        }
        for (lane, w) in weights.iter().enumerate() {
            if !w.is_finite() {
                return Err(FaceError::NonFinite("face.weights"));
            }
            if *w < 0.0 || *w > 1.0 {
                return Err(FaceError::BadWeight { lane });
            }
        }
        let mut packed = [0.0f32; MAX_MORPHS];
        packed.copy_from_slice(weights);
        Ok(Self { weights: packed })
    }

    /// Packed weights in [`viseme::NAMES`] order.
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// Sparse bitmask: bit `i` set while lane `i` exceeds [`ACTIVE_MIN`].
    pub fn active_mask(&self) -> u64 {
        let mut mask = 0u64;
        for (lane, w) in self.weights.iter().enumerate() {
            if *w > ACTIVE_MIN {
                mask |= 1u64 << lane;
            }
        }
        mask
    }

    /// Count of lanes above [`ACTIVE_MIN`]; the cost-model input.
    pub fn active_count(&self) -> u32 {
        self.active_mask().count_ones()
    }

    /// Write the little-endian uniform image into `out` (no allocation).
    ///
    /// `out` must hold exactly [`UNIFORM_BYTES`] bytes; lane `i` lands at
    /// bytes `i * 4..i * 4 + 4` so the GPU reads lane order directly.
    pub fn write_bytes(&self, out: &mut [u8]) -> Result<(), FaceError> {
        if out.len() != UNIFORM_BYTES {
            return Err(FaceError::LengthMismatch {
                expected: UNIFORM_BYTES,
                got: out.len(),
            });
        }
        for (lane, w) in self.weights.iter().enumerate() {
            out[lane * 4..lane * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        Ok(())
    }
}

/// Posted shade cost in microseconds for one head.
///
/// Analytic vertex-shader model: a base dispatch cost plus one
/// multiply-add per vertex per active morph. Compare against
/// [`MORPH_BUDGET_US`] with [`morph_within_budget`].
pub fn morph_cost_us(vert_count: u16, active_morphs: u8) -> f32 {
    MORPH_BASE_US + f32::from(vert_count) * f32::from(active_morphs) * MORPH_PER_VERT_MORPH_US
}

/// True while [`morph_cost_us`] fits [`MORPH_BUDGET_US`].
pub fn morph_within_budget(vert_count: u16, active_morphs: u8) -> bool {
    morph_cost_us(vert_count, active_morphs) <= MORPH_BUDGET_US
}

/// CPU morph oracle for one vertex: `base + sum(w * delta)`.
///
/// Reference for the GPU blend; tests and captures compare against it.
/// Lengths must agree; every lane must be finite. Out-of-range weights
/// extrapolate past the sculpt instead of clamping, by design.
pub fn morph_vertex(
    base: [f32; 3],
    deltas: &[[f32; 3]],
    weights: &[f32],
    out: &mut [f32; 3],
) -> Result<(), FaceError> {
    if deltas.len() != weights.len() {
        return Err(FaceError::LengthMismatch {
            expected: weights.len(),
            got: deltas.len(),
        });
    }
    if base.iter().any(|v| !v.is_finite()) {
        return Err(FaceError::NonFinite("face.morph"));
    }
    for d in deltas {
        if d.iter().any(|v| !v.is_finite()) {
            return Err(FaceError::NonFinite("face.morph"));
        }
    }
    for w in weights {
        if !w.is_finite() {
            return Err(FaceError::NonFinite("face.morph"));
        }
    }
    let mut blended = base;
    for (d, w) in deltas.iter().zip(weights.iter()) {
        blended[0] += w * d[0];
        blended[1] += w * d[1];
        blended[2] += w * d[2];
    }
    *out = blended;
    Ok(())
}

/// Peak active-lane count over every baked key of `clip`.
///
/// Bake-time QA input for [`morph_cost_us`]: the worst key decides the
/// posted cost, so a talkative line cannot blow the budget mid-word.
pub fn peak_active_count(clip: &VisemeClip) -> u8 {
    let mut peak = 0u8;
    for keys in &clip.weights {
        let mut live = 0u8;
        for w in keys {
            if *w > ACTIVE_MIN {
                live = live.saturating_add(1);
            }
        }
        if live > peak {
            peak = live;
        }
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lipsync::DEFAULT_DT;
    use engine_core::alloc_probe::measure;

    fn baked(line: LineId) -> VisemeClip {
        bake_line(line, DEFAULT_DT).unwrap()
    }

    fn two_key() -> VisemeClip {
        VisemeClip {
            times: vec![0.0, 1.0],
            weights: vec![
                viseme::prototype(viseme::Shape::A),
                viseme::prototype(viseme::Shape::B),
            ],
        }
    }

    #[test]
    fn lines_bake_replay_and_differ() {
        let first = baked(LineId::Announcer);
        let second = baked(LineId::Announcer);
        assert_eq!(lipsync::bake_hash(&first), lipsync::bake_hash(&second));
        assert_eq!(first.times.len(), second.times.len());
        let taunt = baked(LineId::Taunt);
        assert_ne!(
            lipsync::bake_hash(&first),
            lipsync::bake_hash(&taunt),
            "announcer and taunt must not share a bake"
        );
        assert_ne!(first.weights[0], taunt.weights[0]);
        assert_eq!(bake_line(LineId::Announcer, 0.0), Err(BakeError::BadDt));
    }

    #[test]
    fn baked_lines_end_neutral_with_readable_jaw() {
        let announcer = baked(LineId::Announcer);
        let mut far = [0.0f32; MAX_MORPHS];
        sample_face(&announcer, 99.0, &mut far).unwrap();
        assert_eq!(far, [0.0f32; MAX_MORPHS]);
        let mut jaw = 0.0f32;
        for keys in &announcer.weights {
            if keys[JAW_OPEN] > jaw {
                jaw = keys[JAW_OPEN];
            }
        }
        assert!(jaw > 0.5, "announcer peak jaw {jaw}");
        let taunt = baked(LineId::Taunt);
        let mut tjaw = 0.0f32;
        for keys in &taunt.weights {
            if keys[JAW_OPEN] > tjaw {
                tjaw = keys[JAW_OPEN];
            }
        }
        assert!(tjaw > 0.0, "taunt must move the jaw");
        assert!(
            tjaw < jaw,
            "taunt peak {tjaw} must sit below announcer {jaw}"
        );
    }

    #[test]
    fn sample_endpoints_empty_and_bad_slices() {
        let clip = two_key();
        let mut out = [0.0f32; MAX_MORPHS];
        sample_face(&clip, -1.0, &mut out).unwrap();
        assert_eq!(out, viseme::prototype(viseme::Shape::A));
        sample_face(&clip, 99.0, &mut out).unwrap();
        assert_eq!(out, viseme::prototype(viseme::Shape::B));
        sample_face(&clip, f32::NAN, &mut out).unwrap();
        assert_eq!(out, [0.0f32; MAX_MORPHS]);
        sample_face(&VisemeClip::default(), 0.25, &mut out).unwrap();
        assert_eq!(out, [0.0f32; MAX_MORPHS]);
        let mut short = [0.0f32; 3];
        assert_eq!(
            sample_face(&clip, 0.0, &mut short),
            Err(FaceError::LengthMismatch {
                expected: MAX_MORPHS,
                got: 3
            })
        );
        assert_eq!(
            FaceError::LengthMismatch {
                expected: 1,
                got: 2
            }
            .as_str(),
            "length-mismatch"
        );
    }

    #[test]
    fn neutral_rig_is_identity() {
        let clip = two_key();
        let mut plain = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.3, &mut plain).unwrap();
        let mut rigged = plain;
        apply_rig(&mut rigged, &FaceRig::neutral()).unwrap();
        assert_eq!(rigged, plain);
    }

    #[test]
    fn rig_drives_jaw_blink_look_brow() {
        let clip = two_key();
        let mut jaw = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.0, &mut jaw).unwrap();
        assert!(jaw[JAW_OPEN] > 0.5);
        let mute = FaceRig {
            jaw_scale: 0.0,
            ..FaceRig::neutral()
        };
        apply_rig(&mut jaw, &mute).unwrap();
        assert_eq!(jaw[JAW_OPEN], 0.0);

        let mut blink = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.0, &mut blink).unwrap();
        let shut = FaceRig {
            blink_left: 1.0,
            blink_right: 1.0,
            ..FaceRig::neutral()
        };
        apply_rig(&mut blink, &shut).unwrap();
        assert_eq!(blink[EYE_BLINK_LEFT], 1.0);
        assert_eq!(blink[EYE_BLINK_RIGHT], 1.0);

        let mut gaze = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.0, &mut gaze).unwrap();
        let right = FaceRig {
            look_x: 1.0,
            look_y: 1.0,
            ..FaceRig::neutral()
        };
        apply_rig(&mut gaze, &right).unwrap();
        assert_eq!(gaze[EYE_LOOK_OUT_RIGHT], 1.0);
        assert_eq!(gaze[EYE_LOOK_IN_LEFT], 1.0);
        assert_eq!(gaze[EYE_LOOK_UP_LEFT], 1.0);
        assert_eq!(gaze[EYE_LOOK_UP_RIGHT], 1.0);

        let mut lift = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.0, &mut lift).unwrap();
        let raised = FaceRig {
            brow: 1.0,
            ..FaceRig::neutral()
        };
        apply_rig(&mut lift, &raised).unwrap();
        assert_eq!(lift[BROW_INNER_UP], 1.0);
        assert_eq!(lift[BROW_OUTER_UP_LEFT], 1.0);
        assert_eq!(lift[BROW_DOWN_LEFT], 0.0);
        let mut knit = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.0, &mut knit).unwrap();
        let knit_rig = FaceRig {
            brow: -1.0,
            ..FaceRig::neutral()
        };
        apply_rig(&mut knit, &knit_rig).unwrap();
        assert_eq!(knit[BROW_DOWN_LEFT], 1.0);
        assert_eq!(knit[BROW_DOWN_RIGHT], 1.0);
    }

    #[test]
    fn rig_rejects_bad_input() {
        let mut weights = viseme::prototype(viseme::Shape::A);
        let mut bad = FaceRig::neutral();
        bad.jaw_scale = f32::NAN;
        assert_eq!(
            apply_rig(&mut weights, &bad),
            Err(FaceError::NonFinite("face.rig"))
        );
        bad = FaceRig::neutral();
        bad.jaw_scale = 3.0;
        assert_eq!(
            apply_rig(&mut weights, &bad),
            Err(FaceError::BadRig("jaw_scale"))
        );
        bad = FaceRig::neutral();
        bad.look_x = 2.0;
        assert_eq!(
            apply_rig(&mut weights, &bad),
            Err(FaceError::BadRig("look_x"))
        );
        bad = FaceRig::neutral();
        bad.brow = -2.0;
        assert_eq!(
            apply_rig(&mut weights, &bad),
            Err(FaceError::BadRig("brow"))
        );
        weights[JAW_OPEN] = f32::INFINITY;
        assert_eq!(
            apply_rig(&mut weights, &FaceRig::neutral()),
            Err(FaceError::NonFinite("face.weights"))
        );
        let mut short = [0.0f32; 4];
        assert_eq!(
            apply_rig(&mut short, &FaceRig::neutral()),
            Err(FaceError::LengthMismatch {
                expected: MAX_MORPHS,
                got: 4
            })
        );
        assert_eq!(
            FaceError::BadWeight { lane: 3 }.to_string(),
            "face weight 3 is outside 0..=1"
        );
    }

    #[test]
    fn uniform_bytes_roundtrip_mask_and_rejections() {
        let proto = viseme::prototype(viseme::Shape::A);
        let uniform = MorphUniform::upload(&proto).unwrap();
        assert_eq!(uniform.weights(), &proto);
        let mut image = [0u8; UNIFORM_BYTES];
        uniform.write_bytes(&mut image).unwrap();
        for (lane, chunk) in image.chunks_exact(4).enumerate() {
            let mut raw = [0u8; 4];
            raw.copy_from_slice(chunk);
            assert_eq!(f32::from_le_bytes(raw), proto[lane], "lane {lane}");
        }
        let mut want = 0u32;
        for w in proto {
            if w > ACTIVE_MIN {
                want += 1;
            }
        }
        assert!(want >= 3, "shape A must light several lanes");
        assert_eq!(uniform.active_count(), want);
        assert_ne!(uniform.active_mask() & (1u64 << JAW_OPEN), 0);
        assert_eq!(uniform.active_mask() & (1u64 << EYE_BLINK_LEFT), 0);

        let mut hot = proto;
        hot[EYE_BLINK_LEFT] = 1.5;
        assert_eq!(
            MorphUniform::upload(&hot),
            Err(FaceError::BadWeight {
                lane: EYE_BLINK_LEFT
            })
        );
        hot[EYE_BLINK_LEFT] = f32::NAN;
        assert_eq!(
            MorphUniform::upload(&hot),
            Err(FaceError::NonFinite("face.weights"))
        );
        assert_eq!(
            MorphUniform::upload(&hot[..4]),
            Err(FaceError::LengthMismatch {
                expected: MAX_MORPHS,
                got: 4
            })
        );
        let mut short = [0u8; 8];
        assert_eq!(
            uniform.write_bytes(&mut short),
            Err(FaceError::LengthMismatch {
                expected: UNIFORM_BYTES,
                got: 8
            })
        );
    }

    #[test]
    fn morph_vertex_matches_hand_blend_and_refuses() {
        let base = [1.0f32, 2.0, 3.0];
        let deltas = [[0.5f32, -1.0, 0.0], [0.0, 0.25, 2.0]];
        let mut out = [0.0f32; 3];
        morph_vertex(base, &deltas, &[1.0, 0.5], &mut out).unwrap();
        assert_eq!(out, [1.5, 1.125, 4.0]);
        morph_vertex(base, &deltas, &[0.0, 0.0], &mut out).unwrap();
        assert_eq!(out, base);
        assert_eq!(
            morph_vertex(base, &deltas, &[1.0], &mut out),
            Err(FaceError::LengthMismatch {
                expected: 1,
                got: 2
            })
        );
        let mut poison = deltas;
        poison[1][2] = f32::NAN;
        assert_eq!(
            morph_vertex(base, &poison, &[1.0, 0.5], &mut out),
            Err(FaceError::NonFinite("face.morph"))
        );
        assert_eq!(
            morph_vertex([f32::INFINITY, 0.0, 0.0], &deltas, &[1.0, 0.5], &mut out),
            Err(FaceError::NonFinite("face.morph"))
        );
    }

    #[test]
    fn posted_cost_covers_full_and_spoken_load() {
        assert!(morph_within_budget(
            TIER_B_HEAD_VERTS,
            u8::try_from(MAX_GPU_MORPHS).unwrap()
        ));
        let announcer = baked(LineId::Announcer);
        let peak = peak_active_count(&announcer);
        assert!(peak >= 3, "speech must light several lanes, got {peak}");
        assert!(peak <= u8::try_from(MAX_GPU_MORPHS).unwrap());
        assert!(
            morph_within_budget(TIER_B_HEAD_VERTS, peak),
            "peak {peak} over budget"
        );
        assert_eq!(peak_active_count(&VisemeClip::default()), 0);
        assert_eq!(
            morph_cost_us(0, 0),
            MORPH_BASE_US,
            "idle heads pay dispatch only"
        );
        assert!(morph_cost_us(8000, 10) > morph_cost_us(8000, 2));
        assert!(morph_cost_us(8000, 2) > morph_cost_us(1000, 2));
        assert_eq!(FaceError::BadRig("brow").as_str(), "bad-rig");
    }

    #[test]
    fn replay_is_bit_identical() {
        let clip = baked(LineId::Taunt);
        let rig = FaceRig {
            look_x: 0.5,
            brow: 0.25,
            ..FaceRig::neutral()
        };
        let mut first = [0.0f32; MAX_MORPHS];
        let mut second = [0.0f32; MAX_MORPHS];
        sample_face(&clip, 0.37, &mut first).unwrap();
        sample_face(&clip, 0.37, &mut second).unwrap();
        assert_eq!(first, second);
        apply_rig(&mut first, &rig).unwrap();
        apply_rig(&mut second, &rig).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            FaceError::NonFinite("face.weights").to_string(),
            "face face.weights is not finite"
        );
    }

    #[test]
    fn lane_indices_match_tooling_names() {
        for (lane, name) in [
            (BROW_DOWN_LEFT, "browDownLeft"),
            (BROW_DOWN_RIGHT, "browDownRight"),
            (BROW_INNER_UP, "browInnerUp"),
            (BROW_OUTER_UP_LEFT, "browOuterUpLeft"),
            (BROW_OUTER_UP_RIGHT, "browOuterUpRight"),
            (EYE_BLINK_LEFT, "eyeBlinkLeft"),
            (EYE_BLINK_RIGHT, "eyeBlinkRight"),
            (EYE_LOOK_DOWN_LEFT, "eyeLookDownLeft"),
            (EYE_LOOK_DOWN_RIGHT, "eyeLookDownRight"),
            (EYE_LOOK_IN_LEFT, "eyeLookInLeft"),
            (EYE_LOOK_IN_RIGHT, "eyeLookInRight"),
            (EYE_LOOK_OUT_LEFT, "eyeLookOutLeft"),
            (EYE_LOOK_OUT_RIGHT, "eyeLookOutRight"),
            (EYE_LOOK_UP_LEFT, "eyeLookUpLeft"),
            (EYE_LOOK_UP_RIGHT, "eyeLookUpRight"),
            (JAW_OPEN, "jawOpen"),
        ] {
            assert_eq!(viseme::index_of(name), Some(lane), "{name}");
        }
        assert_eq!(MAX_MORPHS, viseme::N);
        assert_eq!(MAX_GPU_MORPHS, MAX_MORPHS);
    }

    #[test]
    fn probe_sees_a_real_allocation() {
        let ((), heap) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(heap.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn steady_state_frame_allocates_nothing() {
        let clip = baked(LineId::Announcer);
        let uniform = MorphUniform::upload(&clip.weights[0]).unwrap();
        let mut weights = [0.0f32; MAX_MORPHS];
        let mut image = [0u8; UNIFORM_BYTES];
        let rig = FaceRig::neutral();
        sample_face(&clip, 0.1, &mut weights).unwrap();
        let ((), heap) = measure(|| {
            for t in &clip.times {
                sample_face(&clip, *t, &mut weights).unwrap();
                apply_rig(&mut weights, &rig).unwrap();
                std::hint::black_box(uniform.active_mask());
                std::hint::black_box(uniform.active_count());
                uniform.write_bytes(&mut image).unwrap();
            }
        });
        assert_eq!(heap.calls, 0, "face frame must not allocate");
    }
}
