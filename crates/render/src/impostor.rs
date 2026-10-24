//! impostor: octahedral impostor math, the CPU half of far props, buildings
//! and crowds drawn as one camera-facing quad.
//!
//! Owner: render lane. Units: meters for distance and radius, pixels for
//! screen size, unit direction vectors in world space (y up) from the
//! object toward the camera. Determinism: pure `f32` arithmetic in a fixed
//! order, no allocation, no clock, so the same inputs give the same frames
//! and weights twice. Errors: typed [`ImpostorError`]; nothing panics.
//! Budget: O(1) per call.
//!
//! The idea (Amplify Impostors, Godot Octahedral Impostors): bake the mesh
//! from an N by N grid of view directions laid out on the octahedral map of
//! the sphere (albedo plus normal, optionally depth), then draw it far away
//! as one quad that samples the three nearest baked views and blends them.
//! This module owns the numbers both halves share:
//!
//! - [`octa_encode`] / [`octa_decode`]: direction <-> atlas UV, round-trips.
//! - [`frame_direction`] and [`bake_basis`]: where the baker puts the camera
//!   for frame `i`, and the right/up axes the baker and the runtime quad
//!   must both use so the sprite is never rotated against its source.
//! - [`pick_frames`]: the 3 nearest frames and barycentric weights.
//! - [`AtlasLayout::fit`]: frame resolution for a frame count under a byte
//!   budget and an edge cap (the potato knob).
//! - [`projected_diameter_px`], [`switch_distance_m`], [`use_impostor`]:
//!   the screen-space LOD switch with hysteresis.
//!
//! `shaders/impostor.wgsl` mirrors the sampling half in WGSL. The bake and
//! draw passes under `present/` are the open rest (see the board row).
//! PORT(wojtekpil/Godot-Octahedral-Impostors: grid of octahedral frames,
//! three-frame blend; reimplemented, zero bytes vendored).

use std::fmt;

/// Most frames per atlas side: 32 by 32 is 1024 views, far past any budget.
pub const MAX_FRAMES_SIDE: u32 = 32;
/// Smallest frame edge in pixels the layout will pick.
pub const MIN_FRAME_PX: u32 = 16;
/// Largest frame edge in pixels the layout will pick.
pub const MAX_FRAME_PX: u32 = 256;
/// Bytes per atlas texel for albedo RGBA8 plus normal RGBA8.
pub const TEXEL_BYTES_ALBEDO_NORMAL: u32 = 8;
/// Bytes per atlas texel with a 16-bit depth layer added.
pub const TEXEL_BYTES_WITH_DEPTH: u32 = 10;

/// Impostor math failures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImpostorError {
    /// A direction was not finite or had zero length.
    BadDirection,
    /// Frames per side outside `2..=MAX_FRAMES_SIDE`; value carried.
    BadFrames(u32),
    /// A numeric input was non-finite or out of range; name carried.
    BadInput(&'static str),
    /// Even the smallest frame size does not fit; bytes needed and given.
    BudgetTooSmall {
        /// Bytes the smallest layout needs.
        need: u64,
        /// Bytes the caller allowed.
        got: u64,
    },
}

impl fmt::Display for ImpostorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDirection => write!(f, "impostor direction is not finite or is zero"),
            Self::BadFrames(n) => {
                write!(
                    f,
                    "impostor frames per side {n} outside 2..={MAX_FRAMES_SIDE}"
                )
            }
            Self::BadInput(what) => write!(f, "impostor input {what} is out of range"),
            Self::BudgetTooSmall { need, got } => {
                write!(
                    f,
                    "impostor atlas needs {need} bytes at the smallest frame, budget is {got}"
                )
            }
        }
    }
}

impl std::error::Error for ImpostorError {}

/// Pixel or grid count as `f32`; every caller stays under 65536 (the cap
/// is exact in `f32`), larger values saturate.
fn px(v: u32) -> f32 {
    f32::from(u16::try_from(v).unwrap_or(u16::MAX))
}

/// Grid cell of a non-negative coordinate: how many whole cells fit below
/// it, at most `MAX_FRAMES_SIDE`.
fn cell_of(v: f32) -> u32 {
    let whole = (0..MAX_FRAMES_SIDE).take_while(|c| px(*c + 1) <= v).count();
    u32::try_from(whole).unwrap_or(0)
}

fn sgn(v: f32) -> f32 {
    if v < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn normalized(dir: [f32; 3]) -> Result<[f32; 3], ImpostorError> {
    if !dir.iter().all(|c| c.is_finite()) {
        return Err(ImpostorError::BadDirection);
    }
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len <= 1e-12 || !len.is_finite() {
        return Err(ImpostorError::BadDirection);
    }
    Ok([dir[0] / len, dir[1] / len, dir[2] / len])
}

/// Direction (any length) to atlas UV in `0..=1` on the full-sphere
/// octahedral map, +y at the center and -y at the four corners.
///
/// # Errors
/// [`ImpostorError::BadDirection`] for a non-finite or zero direction.
pub fn octa_encode(dir: [f32; 3]) -> Result<[f32; 2], ImpostorError> {
    let d = normalized(dir)?;
    let s = d[0].abs() + d[1].abs() + d[2].abs();
    let mut ox = d[0] / s;
    let mut oz = d[2] / s;
    if d[1] < 0.0 {
        let (px, pz) = (ox, oz);
        ox = (1.0 - pz.abs()) * sgn(px);
        oz = (1.0 - px.abs()) * sgn(pz);
    }
    Ok([
        (ox * 0.5 + 0.5).clamp(0.0, 1.0),
        (oz * 0.5 + 0.5).clamp(0.0, 1.0),
    ])
}

/// Atlas UV in `0..=1` back to a unit direction; inverse of [`octa_encode`].
/// UVs outside the square clamp to it.
#[must_use]
pub fn octa_decode(uv: [f32; 2]) -> [f32; 3] {
    let u = if uv[0].is_finite() {
        uv[0].clamp(0.0, 1.0)
    } else {
        0.5
    };
    let v = if uv[1].is_finite() {
        uv[1].clamp(0.0, 1.0)
    } else {
        0.5
    };
    let mut ox = u * 2.0 - 1.0;
    let mut oz = v * 2.0 - 1.0;
    let y = 1.0 - ox.abs() - oz.abs();
    if y < 0.0 {
        let (px, pz) = (ox, oz);
        ox = (1.0 - pz.abs()) * sgn(px);
        oz = (1.0 - px.abs()) * sgn(pz);
    }
    let len = (ox * ox + y * y + oz * oz).sqrt();
    if len <= 1e-12 {
        return [0.0, 1.0, 0.0];
    }
    [ox / len, y / len, oz / len]
}

/// UV of grid frame `index` (row-major, `y * n + x`) on an `n` by `n`
/// grid; frames sit on grid points so the map edge is baked too.
fn frame_uv(n: u32, index: u32) -> [f32; 2] {
    let span = px(n - 1);
    [px(index % n) / span, px(index / n) / span]
}

/// World direction (object toward baker camera) of frame `index`.
///
/// # Errors
/// [`ImpostorError::BadFrames`] when `n` is out of range;
/// [`ImpostorError::BadInput`] when `index >= n * n`.
pub fn frame_direction(n: u32, index: u32) -> Result<[f32; 3], ImpostorError> {
    check_frames(n)?;
    if index >= n * n {
        return Err(ImpostorError::BadInput("frame index"));
    }
    Ok(octa_decode(frame_uv(n, index)))
}

fn check_frames(n: u32) -> Result<(), ImpostorError> {
    if (2..=MAX_FRAMES_SIDE).contains(&n) {
        Ok(())
    } else {
        Err(ImpostorError::BadFrames(n))
    }
}

/// Right and up axes of a camera that sits along `to_camera` and looks back
/// at the object, world up +y. The baker renders frame `i` with
/// `bake_basis(frame_direction(i))` and the runtime quad is spanned by the
/// same function on the live view direction, so source and sprite agree. At
/// the poles (direction parallel to +y or -y) up falls back to -z.
///
/// # Errors
/// [`ImpostorError::BadDirection`] for a non-finite or zero direction.
pub fn bake_basis(to_camera: [f32; 3]) -> Result<([f32; 3], [f32; 3]), ImpostorError> {
    let z = normalized(to_camera)?;
    let mut up_ref = [0.0, 1.0, 0.0];
    if z[0].abs() < 1e-4 && z[2].abs() < 1e-4 {
        up_ref = [0.0, 0.0, -1.0];
    }
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let x = normalized(cross(up_ref, z))?;
    let y = cross(z, x);
    Ok((x, y))
}

/// The three frames a view blends and their weights.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameBlend {
    /// Row-major frame indices (`y * n + x`), heaviest weight first.
    pub frames: [u32; 3],
    /// Weights, non-negative, summing to 1.
    pub weights: [f32; 3],
}

/// The 3 nearest frames of an `n` by `n` octahedral grid for a view
/// direction (object toward camera) and their barycentric weights. The grid
/// cell holding the view splits along its diagonal into two triangles; the
/// view's triangle gives the frames. Weights sum to 1, an exact frame
/// direction gives that frame weight 1, and weights are continuous across
/// cell edges and diagonals, so the blend never pops.
///
/// # Errors
/// [`ImpostorError::BadFrames`] for `n` out of range;
/// [`ImpostorError::BadDirection`] for a non-finite or zero direction.
pub fn pick_frames(dir: [f32; 3], n: u32) -> Result<FrameBlend, ImpostorError> {
    check_frames(n)?;
    let uv = octa_encode(dir)?;
    let span = px(n - 1);
    let (gx, gy) = (uv[0] * span, uv[1] * span);
    let last = n - 2;
    let ix = cell_of(gx).min(last);
    let iy = cell_of(gy).min(last);
    let fx = (gx - px(ix)).clamp(0.0, 1.0);
    let fy = (gy - px(iy)).clamp(0.0, 1.0);
    let at = |x: u32, y: u32| y * n + x;
    let (frames, w) = if fx + fy <= 1.0 {
        let w0 = 1.0 - fx - fy;
        ([at(ix, iy), at(ix + 1, iy), at(ix, iy + 1)], [w0, fx, fy])
    } else {
        let w0 = fx + fy - 1.0;
        let w1 = 1.0 - fx;
        (
            [at(ix + 1, iy + 1), at(ix, iy + 1), at(ix + 1, iy)],
            [w0, w1, 1.0 - w0 - w1],
        )
    };
    let mut pairs = [
        (frames[0], w[0].max(0.0)),
        (frames[1], w[1].max(0.0)),
        (frames[2], w[2].max(0.0)),
    ];
    // Heaviest first; ties keep the lower frame index for a stable order.
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let sum = pairs[0].1 + pairs[1].1 + pairs[2].1;
    Ok(FrameBlend {
        frames: [pairs[0].0, pairs[1].0, pairs[2].0],
        weights: [pairs[0].1 / sum, pairs[1].1 / sum, pairs[2].1 / sum],
    })
}

/// Atlas shape for a frame count under a byte budget.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AtlasLayout {
    /// Frames along one side; the atlas holds `frames_per_side` squared.
    pub frames_per_side: u32,
    /// Edge of one frame in pixels (a power of two).
    pub frame_px: u32,
    /// Edge of the whole atlas in pixels.
    pub atlas_px: u32,
    /// Bytes of all layers at `texel_bytes` per texel.
    pub bytes: u64,
}

impl AtlasLayout {
    /// Largest power-of-two frame edge in `MIN_FRAME_PX..=MAX_FRAME_PX`
    /// such that the atlas edge stays within `max_edge_px` and all layers
    /// (`texel_bytes` per texel) stay within `budget_bytes`.
    ///
    /// # Errors
    /// [`ImpostorError::BadFrames`] for `frames_per_side` out of range;
    /// [`ImpostorError::BadInput`] for zero `texel_bytes`;
    /// [`ImpostorError::BudgetTooSmall`] when `MIN_FRAME_PX` already misses
    /// the budget or the edge cap.
    pub fn fit(
        frames_per_side: u32,
        texel_bytes: u32,
        budget_bytes: u64,
        max_edge_px: u32,
    ) -> Result<Self, ImpostorError> {
        check_frames(frames_per_side)?;
        if texel_bytes == 0 {
            return Err(ImpostorError::BadInput("texel_bytes"));
        }
        let build = |frame_px: u32| {
            let atlas_px = frames_per_side * frame_px;
            Self {
                frames_per_side,
                frame_px,
                atlas_px,
                bytes: u64::from(atlas_px) * u64::from(atlas_px) * u64::from(texel_bytes),
            }
        };
        let fits = |l: &Self| l.bytes <= budget_bytes && l.atlas_px <= max_edge_px;
        let smallest = build(MIN_FRAME_PX);
        if !fits(&smallest) {
            return Err(ImpostorError::BudgetTooSmall {
                need: smallest.bytes,
                got: budget_bytes,
            });
        }
        let mut best = smallest;
        let mut px = MIN_FRAME_PX * 2;
        while px <= MAX_FRAME_PX {
            let l = build(px);
            if !fits(&l) {
                break;
            }
            best = l;
            px *= 2;
        }
        Ok(best)
    }

    /// Pixel rectangle `[x, y, w, h]` of frame `index`, or `None` past the
    /// last frame.
    #[must_use]
    pub fn frame_rect(&self, index: u32) -> Option<[u32; 4]> {
        let n = self.frames_per_side;
        if index >= n * n {
            return None;
        }
        Some([
            (index % n) * self.frame_px,
            (index / n) * self.frame_px,
            self.frame_px,
            self.frame_px,
        ])
    }

    /// UV rectangle `[u0, v0, u1, v1]` of frame `index` in the atlas.
    #[must_use]
    pub fn frame_uv_rect(&self, index: u32) -> Option<[f32; 4]> {
        let [x, y, w, h] = self.frame_rect(index)?;
        let a = px(self.atlas_px);
        Some([px(x) / a, px(y) / a, px(x + w) / a, px(y + h) / a])
    }
}

fn check_screen(radius_m: f32, screen_h: u32, tan_half_fov_y: f32) -> Result<(), ImpostorError> {
    if !radius_m.is_finite() || radius_m < 0.0 {
        return Err(ImpostorError::BadInput("radius_m"));
    }
    if screen_h == 0 || screen_h > u32::from(u16::MAX) {
        return Err(ImpostorError::BadInput("screen_h"));
    }
    if !tan_half_fov_y.is_finite() || tan_half_fov_y <= 0.0 {
        return Err(ImpostorError::BadInput("tan_half_fov_y"));
    }
    Ok(())
}

/// Height in pixels of a bounding sphere of `radius_m` seen from `dist_m`.
///
/// # Errors
/// [`ImpostorError::BadInput`] for a bad radius, distance, screen height or
/// field of view.
pub fn projected_diameter_px(
    radius_m: f32,
    dist_m: f32,
    screen_h: u32,
    tan_half_fov_y: f32,
) -> Result<f32, ImpostorError> {
    check_screen(radius_m, screen_h, tan_half_fov_y)?;
    if !dist_m.is_finite() || dist_m <= 0.0 {
        return Err(ImpostorError::BadInput("dist_m"));
    }
    Ok(radius_m * px(screen_h) / (dist_m * tan_half_fov_y))
}

/// Distance in meters at which the sphere is `switch_px` pixels tall: the
/// inverse of [`projected_diameter_px`]. Past it the impostor takes over.
///
/// # Errors
/// [`ImpostorError::BadInput`] for a bad radius, screen height, field of
/// view or a non-positive `switch_px`.
pub fn switch_distance_m(
    radius_m: f32,
    screen_h: u32,
    tan_half_fov_y: f32,
    switch_px: f32,
) -> Result<f32, ImpostorError> {
    check_screen(radius_m, screen_h, tan_half_fov_y)?;
    if !switch_px.is_finite() || switch_px <= 0.0 {
        return Err(ImpostorError::BadInput("switch_px"));
    }
    Ok(radius_m * px(screen_h) / (switch_px * tan_half_fov_y))
}

/// Switch size that keeps one baked texel on one screen pixel: the object
/// turns into an impostor once it is no taller than its frame.
#[must_use]
pub fn default_switch_px(layout: &AtlasLayout) -> f32 {
    px(layout.frame_px)
}

/// Impostor or mesh with hysteresis: impostor once the projected size falls
/// below `switch_px - hyst_px`, mesh again above `switch_px + hyst_px`, and
/// `prev_impostor` holds inside the band so a shaking camera never flickers.
/// A non-finite size reads as far away.
#[must_use]
pub fn use_impostor(prev_impostor: bool, projected_px: f32, switch_px: f32, hyst_px: f32) -> bool {
    if !projected_px.is_finite() {
        return true;
    }
    let hyst = if hyst_px.is_finite() {
        hyst_px.max(0.0)
    } else {
        0.0
    };
    if projected_px < switch_px - hyst {
        true
    } else if projected_px > switch_px + hyst {
        false
    } else {
        prev_impostor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fibonacci sphere: `count` near-uniform unit directions.
    fn sphere(count: u32) -> Vec<[f32; 3]> {
        let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        (0..count)
            .map(|i| {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / count as f32;
                let r = (1.0 - y * y).max(0.0).sqrt();
                let a = golden * i as f32;
                [r * a.cos(), y, r * a.sin()]
            })
            .collect()
    }

    fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    #[test]
    fn octa_round_trips_every_direction() {
        let mut dirs = sphere(4000);
        dirs.extend([
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ]);
        for d in dirs {
            let uv = octa_encode(d).expect("finite");
            assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
            let back = octa_decode(uv);
            assert!(dist(d, back) < 2e-5, "{d:?} -> {uv:?} -> {back:?}");
        }
    }

    #[test]
    fn octa_poles_and_scale_independence() {
        assert_eq!(octa_encode([0.0, 3.0, 0.0]).unwrap(), [0.5, 0.5]);
        let corner = octa_encode([0.0, -1.0, 0.0]).unwrap();
        assert!(
            corner == [0.0, 0.0]
                || corner == [1.0, 0.0]
                || corner == [0.0, 1.0]
                || corner == [1.0, 1.0]
        );
        assert_eq!(
            octa_encode([1.0, 2.0, 3.0]).unwrap(),
            octa_encode([10.0, 20.0, 30.0]).unwrap()
        );
    }

    #[test]
    fn bad_directions_are_typed_errors() {
        assert_eq!(octa_encode([0.0; 3]), Err(ImpostorError::BadDirection));
        assert_eq!(
            octa_encode([f32::NAN, 0.0, 1.0]),
            Err(ImpostorError::BadDirection)
        );
        assert_eq!(
            pick_frames([f32::INFINITY, 0.0, 0.0], 4),
            Err(ImpostorError::BadDirection)
        );
        assert_eq!(
            pick_frames([0.0, 1.0, 0.0], 1),
            Err(ImpostorError::BadFrames(1))
        );
        assert_eq!(
            pick_frames([0.0, 1.0, 0.0], 33),
            Err(ImpostorError::BadFrames(33))
        );
        assert_eq!(
            frame_direction(4, 16),
            Err(ImpostorError::BadInput("frame index"))
        );
        assert_eq!(octa_decode([f32::NAN, 7.0]), octa_decode([0.5, 1.0]));
    }

    #[test]
    fn weights_sum_to_one_and_frames_are_in_range() {
        for n in [2u32, 3, 5, 8, 12, 32] {
            for d in sphere(1500) {
                let b = pick_frames(d, n).unwrap();
                let sum: f32 = b.weights.iter().sum();
                assert!((sum - 1.0).abs() < 1e-6, "n={n} sum={sum}");
                assert!(b.weights.iter().all(|w| (0.0..=1.0).contains(w)));
                assert!(b.frames.iter().all(|f| *f < n * n));
                assert!(b.weights[0] >= b.weights[1] && b.weights[1] >= b.weights[2]);
            }
        }
    }

    #[test]
    fn an_exact_frame_direction_gives_that_frame_weight_one() {
        for n in [2u32, 4, 8, 12] {
            for i in 0..n * n {
                let d = frame_direction(n, i).unwrap();
                let b = pick_frames(d, n).unwrap();
                // The four map corners are the same -y view, so any of
                // them is an exact answer there.
                let want = frame_direction(n, b.frames[0]).unwrap();
                assert!(dist(d, want) < 1e-4, "n={n} i={i}");
                assert!(b.weights[0] > 0.9999, "n={n} i={i} w={:?}", b.weights);
            }
        }
    }

    #[test]
    fn the_three_frames_enclose_the_view_in_uv() {
        let n = 6;
        for d in sphere(500) {
            let b = pick_frames(d, n).unwrap();
            let uv = octa_encode(d).unwrap();
            let mut mix = [0.0f32; 2];
            for k in 0..3 {
                let f = frame_uv(n, b.frames[k]);
                mix[0] += f[0] * b.weights[k];
                mix[1] += f[1] * b.weights[k];
            }
            assert!((mix[0] - uv[0]).abs() < 1e-5 && (mix[1] - uv[1]).abs() < 1e-5);
        }
    }

    #[test]
    fn blend_is_continuous_so_the_view_never_pops() {
        let n = 8;
        let weights_of = |b: FrameBlend, f: u32| {
            b.frames
                .iter()
                .zip(b.weights)
                .find(|(x, _)| **x == f)
                .map_or(0.0, |(_, w)| w)
        };
        for d in sphere(300) {
            let b0 = pick_frames(d, n).unwrap();
            let e = 1e-4;
            let b1 = pick_frames([d[0] + e, d[1], d[2] - e], n).unwrap();
            for f in b0.frames.iter().chain(b1.frames.iter()) {
                let dw = (weights_of(b0, *f) - weights_of(b1, *f)).abs();
                assert!(dw < 2e-3, "frame {f} jumped by {dw}");
            }
        }
    }

    #[test]
    fn bake_basis_is_orthonormal_and_keeps_up_up() {
        for d in sphere(400)
            .into_iter()
            .chain([[0.0, 1.0, 0.0], [0.0, -1.0, 0.0]])
        {
            let (x, y) = bake_basis(d).unwrap();
            let z = normalized(d).unwrap();
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            assert!((dot(x, x) - 1.0).abs() < 1e-5 && (dot(y, y) - 1.0).abs() < 1e-5);
            assert!(dot(x, y).abs() < 1e-5 && dot(x, z).abs() < 1e-5 && dot(y, z).abs() < 1e-5);
            if z[1].abs() < 0.99 {
                assert!(y[1] > 0.0, "sprite up must lean toward world up for {d:?}");
            }
        }
    }

    #[test]
    fn atlas_fit_obeys_budget_and_edge_cap() {
        let l = AtlasLayout::fit(8, TEXEL_BYTES_ALBEDO_NORMAL, 4 << 20, 2048).unwrap();
        assert!(l.bytes <= 4 << 20 && l.atlas_px <= 2048);
        assert_eq!(l.atlas_px, l.frames_per_side * l.frame_px);
        assert!(l.frame_px.is_power_of_two());
        // One size up would break the budget (the fit is the largest).
        let up = u64::from(l.atlas_px * 2).pow(2) * u64::from(TEXEL_BYTES_ALBEDO_NORMAL);
        assert!(up > 4 << 20 || l.frame_px == MAX_FRAME_PX || l.atlas_px * 2 > 2048);
        // Potato floor: 8 by 8 frames in 2 MiB is 64 px frames, 512 px atlas.
        let p = AtlasLayout::fit(8, TEXEL_BYTES_ALBEDO_NORMAL, 2 << 20, 1024).unwrap();
        assert_eq!((p.frame_px, p.atlas_px, p.bytes), (64, 512, 2 << 20));
        // A depth layer costs more, so the same budget cannot grow.
        let d = AtlasLayout::fit(8, TEXEL_BYTES_WITH_DEPTH, 2 << 20, 1024).unwrap();
        assert!(d.frame_px <= p.frame_px && d.bytes <= 2 << 20);
        // The edge cap alone bounds it.
        let c = AtlasLayout::fit(4, TEXEL_BYTES_ALBEDO_NORMAL, u64::MAX, 256).unwrap();
        assert_eq!((c.frame_px, c.atlas_px), (64, 256));
    }

    #[test]
    fn atlas_fit_refuses_what_cannot_fit() {
        assert!(matches!(
            AtlasLayout::fit(16, TEXEL_BYTES_ALBEDO_NORMAL, 1000, 4096),
            Err(ImpostorError::BudgetTooSmall { got: 1000, .. })
        ));
        assert!(matches!(
            AtlasLayout::fit(32, 8, u64::MAX, 256),
            Err(ImpostorError::BudgetTooSmall { .. })
        ));
        assert_eq!(
            AtlasLayout::fit(1, 8, 1 << 20, 512),
            Err(ImpostorError::BadFrames(1))
        );
        assert_eq!(
            AtlasLayout::fit(4, 0, 1 << 20, 512),
            Err(ImpostorError::BadInput("texel_bytes"))
        );
    }

    #[test]
    fn frame_rects_tile_the_atlas_exactly() {
        let l = AtlasLayout::fit(4, 8, 1 << 20, 1024).unwrap();
        let mut covered = vec![0u8; (l.atlas_px * l.atlas_px) as usize];
        for i in 0..16 {
            let [x, y, w, h] = l.frame_rect(i).unwrap();
            for yy in y..y + h {
                for xx in x..x + w {
                    covered[(yy * l.atlas_px + xx) as usize] += 1;
                }
            }
            let uv = l.frame_uv_rect(i).unwrap();
            assert!(uv[0] < uv[2] && uv[1] < uv[3] && uv[2] <= 1.0 && uv[3] <= 1.0);
        }
        assert!(covered.iter().all(|c| *c == 1));
        assert_eq!(l.frame_rect(16), None);
        assert_eq!(l.frame_uv_rect(16), None);
    }

    #[test]
    fn switch_distance_inverts_projected_size() {
        let (r, h, t) = (1.5_f32, 720_u32, 0.4142_f32);
        for px in [16.0_f32, 64.0, 200.0] {
            let d = switch_distance_m(r, h, t, px).unwrap();
            let back = projected_diameter_px(r, d, h, t).unwrap();
            assert!((back - px).abs() < px * 1e-5, "{px} -> {d} -> {back}");
        }
        // Twice as far reads half as tall; a bigger object switches later.
        let near = projected_diameter_px(r, 10.0, h, t).unwrap();
        let far = projected_diameter_px(r, 20.0, h, t).unwrap();
        assert!((near / far - 2.0).abs() < 1e-5);
        assert!(
            switch_distance_m(3.0, h, t, 64.0).unwrap()
                > switch_distance_m(1.5, h, t, 64.0).unwrap()
        );
    }

    #[test]
    fn screen_inputs_are_checked() {
        assert!(projected_diameter_px(-1.0, 10.0, 720, 0.4).is_err());
        assert!(projected_diameter_px(1.0, 0.0, 720, 0.4).is_err());
        assert!(projected_diameter_px(1.0, 10.0, 0, 0.4).is_err());
        assert!(projected_diameter_px(1.0, 10.0, 720, 0.0).is_err());
        assert!(projected_diameter_px(1.0, f32::NAN, 720, 0.4).is_err());
        assert!(switch_distance_m(1.0, 720, 0.4, 0.0).is_err());
    }

    #[test]
    fn switch_has_hysteresis_and_defaults_to_one_texel_per_pixel() {
        let l = AtlasLayout::fit(8, 8, 2 << 20, 1024).unwrap();
        let sw = default_switch_px(&l);
        assert_eq!(sw, 64.0);
        assert!(use_impostor(false, 50.0, sw, 4.0));
        assert!(!use_impostor(true, 80.0, sw, 4.0));
        // Inside the band the previous answer holds both ways.
        assert!(use_impostor(true, 65.0, sw, 4.0));
        assert!(!use_impostor(false, 65.0, sw, 4.0));
        assert!(use_impostor(false, f32::NAN, sw, 4.0));
        // A walk in and out switches once each, not on every wobble.
        let mut on = false;
        let mut flips = 0;
        for px in [
            90.0, 70.0, 66.0, 62.0, 60.0, 62.0, 66.0, 63.0, 60.0, 59.0, 66.0, 70.0, 72.0,
        ] {
            let next = use_impostor(on, px, sw, 4.0);
            flips += u32::from(next != on);
            on = next;
        }
        assert_eq!(flips, 2);
    }

    #[test]
    fn wgsl_mirror_holds_the_same_math() {
        let src = include_str!("shaders/impostor.wgsl");
        for needle in [
            "fn octa_encode",
            "fn octa_decode",
            "fn impostor_pick",
            "fn impostor_basis",
            "fn vs_main",
            "fn fs_main",
            "fx + fy <= 1.0",
            "span",
        ] {
            assert!(src.contains(needle), "impostor.wgsl lost `{needle}`");
        }
    }
}
