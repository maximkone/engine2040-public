//! occl_cpu: software Hi-Z for the tiers with no depth prepass and no GPU
//! readback (the web/GL2 potato path). Occluder triangles are rasterized into
//! a small max-depth buffer beside the frustum cull, the buffer is max-reduced
//! down to one texel, and a node's AABB is tested against the four parent
//! texels that cover it. A box behind every occluder at its screen rect is
//! hidden; anything else stays, so the pass can only remove draws the frustum
//! cull kept.
//!
//! Shape (ideas from Godot `HZBuffer`: `resize`, `update_mips`,
//! `CHECK_OFFSET`): level 0 is [`OCCL_W`] by [`OCCL_H`] view depths, one per
//! texel, zero where no occluder covers; each later level is the maximum of
//! its four children, so [`OCCL_MIPS`] levels reach one texel. Depth is the
//! view-space distance along the camera forward, in meters, so a larger number
//! is farther away.
//!
//! Units: meters, radians. Determinism: `+ - * /` and `sqrt` only, fixed pixel
//! order, no clock and no hash map, so the same camera and the same triangles
//! give the same bytes twice. No GPU call is made or needed.

use core::fmt;

use crate::draw::{dot, len, scale, sub, Camera};

/// Level 0 width in texels: 256x144 is a sixteenth of a 1080p frame.
pub const OCCL_W: u32 = 256;
/// Level 0 height in texels.
pub const OCCL_H: u32 = 144;
/// Levels down to one texel: 256x144 halves eight times (144 -> 72, 36, 18, 9,
/// 4, 2, 1) and the last level is 1x1.
pub const OCCL_MIPS: usize = 9;

/// Why a Hi-Z buffer could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcclError {
    /// The camera is unusable for a projection.
    BadCamera,
}

impl fmt::Display for OcclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadCamera => write!(f, "the camera cannot be projected"),
        }
    }
}

impl std::error::Error for OcclError {}

/// The camera a buffer was built for: eye, basis and half-frame tangents.
#[derive(Clone, Copy, Debug)]
struct View {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    tan_half_w: f32,
    tan_half_h: f32,
    near_m: f32,
}

/// One software Hi-Z buffer plus the view it was rasterized from.
#[derive(Clone, Debug)]
pub struct OcclusionBuffer {
    view: View,
    /// Depth per texel, level 0 first; level `l` is `mip_dims(l)` wide.
    levels: Vec<Vec<f32>>,
}

/// One axis halved for the next mip, never below one texel.
fn halve(value: u32) -> u32 {
    (value / 2).max(1)
}

/// The size of mip `level`; `None` past [`OCCL_MIPS`].
#[must_use]
fn dims_at(level: usize) -> Option<(u32, u32)> {
    if level >= OCCL_MIPS {
        return None;
    }
    let mut w = OCCL_W;
    let mut h = OCCL_H;
    for _ in 0..level {
        w = halve(w);
        h = halve(h);
    }
    Some((w, h))
}

/// A texel count as `f32` for the projection math (a viewport fits a `u16`).
fn texels(count: u32) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// An axis as a texel index: floored, never negative. The caller clamps `v`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "v is clamped to 0..=texels and truncation is the floor"
)]
fn texel(v: f32) -> u32 {
    v as u32
}

/// Row-major index of one texel; `0` where the product cannot fit a `usize`.
fn index_of(x: u32, y: u32, width: u32) -> usize {
    usize::try_from(y.saturating_mul(width).saturating_add(x)).unwrap_or(0)
}

/// Cross product of two 3-vectors.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The view for a camera, or `None` when its basis or optics is degenerate.
fn view_of(camera: &Camera) -> Option<View> {
    let forward = camera.forward;
    let up = camera.up;
    let f_len = len(forward);
    let u_len = len(up);
    if !(f_len > 1e-6) || !(u_len > 1e-6) {
        return None;
    }
    let forward = scale(forward, 1.0 / f_len);
    let right = cross(forward, up);
    let r_len = len(right);
    if !(r_len > 1e-6) {
        return None;
    }
    let right = scale(right, 1.0 / r_len);
    let up = cross(right, forward);
    let tan_half_h = (camera.fov_y_rad * 0.5).tan();
    if !tan_half_h.is_finite() || tan_half_h <= 0.0 {
        return None;
    }
    if !camera.aspect.is_finite() || camera.aspect <= 0.0 {
        return None;
    }
    if !(camera.near_m > 0.0) || !(camera.far_m > camera.near_m) {
        return None;
    }
    Some(View {
        eye: camera.pos_m,
        right,
        up,
        forward,
        tan_half_w: tan_half_h * camera.aspect,
        tan_half_h,
        near_m: camera.near_m,
    })
}

impl View {
    /// Screen position and view depth of one world point; `None` at or behind
    /// the near plane, which the rasterizer skips rather than clips.
    fn project(&self, point: [f32; 3]) -> Option<([f32; 2], f32)> {
        let offset = sub(point, self.eye);
        let z = dot(offset, self.forward);
        if !(z > self.near_m) {
            return None;
        }
        let x = dot(offset, self.right) / (z * self.tan_half_w);
        let y = dot(offset, self.up) / (z * self.tan_half_h);
        let px = (x + 1.0) * 0.5 * texels(OCCL_W);
        let py = (1.0 - y) * 0.5 * texels(OCCL_H);
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        Some(([px, py], z))
    }
}

impl OcclusionBuffer {
    /// A buffer for this camera with every texel empty (nothing occluded).
    ///
    /// # Errors
    ///
    /// Returns [`OcclError::BadCamera`] when the camera's basis, field of view,
    /// aspect, near or far cannot be projected.
    pub fn new(camera: &Camera) -> Result<Self, OcclError> {
        let view = view_of(camera).ok_or(OcclError::BadCamera)?;
        let mut levels = Vec::with_capacity(OCCL_MIPS);
        for level in 0..OCCL_MIPS {
            let (w, h) = dims_at(level).ok_or(OcclError::BadCamera)?;
            levels.push(vec![0.0; usize::try_from(w * h).unwrap_or(0)]);
        }
        Ok(Self { view, levels })
    }

    /// Level 0 size in texels.
    #[must_use]
    pub fn dims(&self) -> (u32, u32) {
        (OCCL_W, OCCL_H)
    }

    /// Levels in the chain, level 0 included; the last is one texel.
    #[must_use]
    pub fn mip_count(&self) -> usize {
        self.levels.len()
    }

    /// Size of mip `level`, `None` past the chain.
    #[must_use]
    pub fn mip_dims(&self, level: usize) -> Option<(u32, u32)> {
        dims_at(level)
    }

    /// Depth kept at `(x, y)` of `level`; `None` outside the mip.
    #[must_use]
    pub fn depth_at(&self, level: usize, x: u32, y: u32) -> Option<f32> {
        let (w, _) = dims_at(level)?;
        let level_data = self.levels.get(level)?;
        level_data.get(index_of(x, y, w)).copied()
    }

    /// Empty every texel: nothing is occluded.
    pub fn clear(&mut self) {
        for level in &mut self.levels {
            level.fill(0.0);
        }
    }

    /// Rasterize one occluder triangle into level 0, keeping the farthest
    /// depth per texel. A triangle with a vertex at or behind the near plane is
    /// skipped whole: the pass is an optimisation and never clips.
    pub fn add_triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let (Some((pa, za)), Some((pb, zb)), Some((pc, zc))) = (
            self.view.project(a),
            self.view.project(b),
            self.view.project(c),
        ) else {
            return;
        };
        let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let (w, h) = (OCCL_W, OCCL_H);
        let min_x = pa[0].min(pb[0]).min(pc[0]).floor().max(0.0);
        let max_x = pa[0].max(pb[0]).max(pc[0]).ceil().min(texels(w) - 1.0);
        let min_y = pa[1].min(pb[1]).min(pc[1]).floor().max(0.0);
        let max_y = pa[1].max(pb[1]).max(pc[1]).ceil().min(texels(h) - 1.0);
        if max_x < min_x || max_y < min_y {
            return;
        }
        // Depth interpolates as 1/z, which is linear in screen space.
        let (wa, wb, wc) = (1.0 / za, 1.0 / zb, 1.0 / zc);
        let (x0, x1, y0, y1) = (texel(min_x), texel(max_x), texel(min_y), texel(max_y));
        for y in y0..=y1 {
            let py = texels(y) + 0.5;
            for x in x0..=x1 {
                let px = texels(x) + 0.5;
                let w0 = ((pb[0] - px) * (pc[1] - py) - (pc[0] - px) * (pb[1] - py)) / area;
                let w1 = ((pc[0] - px) * (pa[1] - py) - (pa[0] - px) * (pc[1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let depth = w0 * wa + w1 * wb + w2 * wc;
                    if depth > 0.0 {
                        let z = 1.0 / depth;
                        if let Some(slot) = self.levels[0].get_mut(index_of(x, y, w)) {
                            if z > *slot {
                                *slot = z;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rasterize one quad as two triangles, in corner order.
    pub fn add_quad(&mut self, corners: &[[f32; 3]; 4]) {
        self.add_triangle(corners[0], corners[1], corners[2]);
        self.add_triangle(corners[0], corners[2], corners[3]);
    }

    /// Max-reduce every level from the one below it. Call once per frame after
    /// the occluders are in. Writes in place: the levels are allocated at
    /// [`OcclusionBuffer::new`], so a frame allocates nothing.
    pub fn update_mips(&mut self) {
        for level in 1..self.levels.len() {
            let (Some((w, h)), Some((src_w, src_h))) = (dims_at(level), dims_at(level - 1)) else {
                return;
            };
            let (below, this) = self.levels.split_at_mut(level);
            let src = &below[level - 1];
            let dst = &mut this[0];
            for y in 0..h {
                for x in 0..w {
                    let mut deepest = 0.0f32;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = x * 2 + dx;
                            let sy = y * 2 + dy;
                            if sx >= src_w || sy >= src_h {
                                continue;
                            }
                            if let Some(depth) = src.get(index_of(sx, sy, src_w)) {
                                if *depth > deepest {
                                    deepest = *depth;
                                }
                            }
                        }
                    }
                    if let Some(slot) = dst.get_mut(index_of(x, y, w)) {
                        *slot = deepest;
                    }
                }
            }
        }
    }

    /// True when the axis-aligned box `lo..hi` sits behind every occluder over
    /// its screen rect, so the draw can be skipped. Conservative: a box that
    /// touches the frame edge, reaches the near plane or leaves the buffer
    /// stays visible.
    #[must_use]
    pub fn is_hidden_aabb(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        let mut min_px = f32::INFINITY;
        let mut min_py = f32::INFINITY;
        let mut max_px = f32::NEG_INFINITY;
        let mut max_py = f32::NEG_INFINITY;
        let mut near_z = f32::INFINITY;
        for corner in 0..8 {
            let point = [
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let Some((screen, z)) = self.view.project(point) else {
                return false;
            };
            min_px = min_px.min(screen[0]);
            max_px = max_px.max(screen[0]);
            min_py = min_py.min(screen[1]);
            max_py = max_py.max(screen[1]);
            near_z = near_z.min(z);
        }
        if min_px < 0.0 || min_py < 0.0 || max_px >= texels(OCCL_W) || max_py >= texels(OCCL_H) {
            return false;
        }
        // Coarsen until the rect covers at most two texels each way, so the
        // test reads the four parent texels of one child.
        let mut level = 0;
        while level + 1 < self.levels.len() {
            let (lw, lh) = dims_at(level + 1).unwrap_or((1, 1));
            let span_x = (max_px - min_px) / (texels(OCCL_W) / texels(lw));
            let span_y = (max_py - min_py) / (texels(OCCL_H) / texels(lh));
            if span_x <= 2.0 && span_y <= 2.0 {
                level += 1;
            } else {
                break;
            }
        }
        let (lw, lh) = dims_at(level).unwrap_or((1, 1));
        let sx = texels(OCCL_W) / texels(lw);
        let sy = texels(OCCL_H) / texels(lh);
        let x0 = texel((min_px / sx).floor().max(0.0));
        let x1 = texel((max_px / sx).floor().min(texels(lw) - 1.0));
        let y0 = texel((min_py / sy).floor().max(0.0));
        let y1 = texel((max_py / sy).floor().min(texels(lh) - 1.0));
        let mut deepest = 0.0f32;
        for x in x0..=x1 {
            for y in y0..=y1 {
                if let Some(depth) = self.depth_at(level, x, y) {
                    if depth > deepest {
                        deepest = depth;
                    }
                }
            }
        }
        deepest > 0.0 && near_z > deepest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{
        cull_cpu_flags_into, frustum_planes_packed, visible_count, DrawList, LodTable, SceneNode,
        SceneSnapshot,
    };

    /// The potato-tier camera: at the origin, looking down -Z, 60 degrees down
    /// the frame, 16:9.
    fn camera() -> Camera {
        Camera {
            pos_m: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad: core::f32::consts::FRAC_PI_3,
            aspect: 16.0 / 9.0,
            near_m: 0.1,
            far_m: 200.0,
        }
    }

    /// A wall plane 30 m ahead, wide enough to fill the frame.
    fn wall() -> [[f32; 3]; 4] {
        [
            [-40.0, -30.0, -30.0],
            [40.0, -30.0, -30.0],
            [40.0, 30.0, -30.0],
            [-40.0, 30.0, -30.0],
        ]
    }

    /// The 12 fighters of the wall scene: 8 behind the wall, 4 in front of it.
    fn fighters() -> Vec<([f32; 3], bool)> {
        let mut out = Vec::new();
        for x in [-20.0f32, -14.0, -8.0, -2.0, 2.0, 8.0, 14.0, 20.0] {
            out.push(([x, 0.0, -40.0], true));
        }
        for x in [-6.0f32, -2.0, 2.0, 6.0] {
            out.push(([x, 0.0, -10.0], false));
        }
        out
    }

    /// The AABB of one fighter: a 0.8 m box round the body.
    fn aabb(center: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let r = 0.4;
        (
            [center[0] - r, center[1] - r, center[2] - r],
            [center[0] + r, center[1] + r, center[2] + r],
        )
    }

    #[test]
    fn occl_cpu_builds_nine_max_depth_mips_down_to_one_texel() {
        let buffer = OcclusionBuffer::new(&camera()).expect("the camera projects");
        assert_eq!(buffer.dims(), (256, 144), "a sixteenth of 1080p");
        assert_eq!(buffer.mip_count(), 9, "256x144 halves eight times");
        let mut expected = [(256u32, 144u32); 9];
        for level in 1..9 {
            let (w, h) = expected[level - 1];
            expected[level] = (halve(w), halve(h));
        }
        for (level, want) in expected.iter().enumerate() {
            let (w, h) = buffer.mip_dims(level).expect("a level of the chain");
            assert_eq!((w, h), *want, "level {level}");
        }
        assert_eq!(expected[8], (1, 1), "the chain ends at one texel");
        assert!(buffer.mip_dims(9).is_none(), "no level past the chain");

        // The coarse mips carry the deepest child, not the nearest.
        let mut painted = OcclusionBuffer::new(&camera()).expect("the camera projects");
        painted.add_quad(&wall());
        painted.update_mips();
        assert!(
            painted.depth_at(0, 128, 72).unwrap_or(0.0) > 29.0,
            "the wall is 30 m out"
        );
        for level in 1..9 {
            assert!(
                painted.depth_at(level, 0, 0).unwrap_or(0.0) > 29.0,
                "level {level} holds the wall's depth"
            );
        }
    }

    #[test]
    fn occl_cpu_verdicts_match_the_gpu_flag_rule_on_a_wall() {
        let camera = camera();
        let planes = frustum_planes_packed(&camera).expect("the camera projects");
        let fighters = fighters();
        let spheres: Vec<[f32; 4]> = fighters
            .iter()
            .map(|(center, _)| [center[0], center[1], center[2], 0.4])
            .collect();
        let mut flags = vec![0u8; spheres.len()];
        let written = cull_cpu_flags_into(&spheres, &planes, &mut flags);
        assert_eq!(written, flags.len());
        assert_eq!(
            visible_count(&flags),
            u32::try_from(flags.len()).unwrap_or(0),
            "the frustum alone keeps every fighter: the wall is the only occluder"
        );

        let mut buffer = OcclusionBuffer::new(&camera).expect("the camera projects");
        buffer.add_quad(&wall());
        buffer.update_mips();

        let mut hidden = 0;
        for (index, (center, behind)) in fighters.iter().enumerate() {
            let (lo, hi) = aabb(*center);
            let occluded = buffer.is_hidden_aabb(lo, hi);
            // The GPU flag rule on this scene: a fighter is drawn when the
            // frustum keeps it and its near depth is nearer than the wall.
            assert_eq!(
                occluded, *behind,
                "fighter {index} at z {}: the Hi-Z verdict must equal the flag rule",
                center[2]
            );
            if occluded {
                hidden += 1;
            }
        }
        assert_eq!(hidden, 8, "the wall hides the 8 fighters behind it");
    }

    #[test]
    fn occl_cpu_keeps_every_draw_when_nothing_is_rasterized() {
        let mut buffer = OcclusionBuffer::new(&camera()).expect("the camera projects");
        buffer.update_mips();
        for (center, _) in fighters() {
            let (lo, hi) = aabb(center);
            assert!(!buffer.is_hidden_aabb(lo, hi), "nothing occludes yet");
        }
        buffer.add_quad(&wall());
        buffer.update_mips();
        buffer.clear();
        for (center, _) in fighters() {
            let (lo, hi) = aabb(center);
            assert!(
                !buffer.is_hidden_aabb(lo, hi),
                "a cleared buffer hides nothing"
            );
        }
    }

    #[test]
    fn occl_cpu_rasterizes_the_same_wall_twice() {
        let mut a = OcclusionBuffer::new(&camera()).expect("the camera projects");
        let mut b = OcclusionBuffer::new(&camera()).expect("the camera projects");
        a.add_quad(&wall());
        b.add_triangle(wall()[0], wall()[1], wall()[2]);
        b.add_triangle(wall()[0], wall()[2], wall()[3]);
        a.update_mips();
        b.update_mips();
        for level in 0..a.mip_count() {
            let (w, h) = a.mip_dims(level).expect("a level");
            for x in 0..w {
                for y in 0..h {
                    assert_eq!(
                        a.depth_at(level, x, y),
                        b.depth_at(level, x, y),
                        "level {level} texel ({x}, {y}) is the same twice"
                    );
                }
            }
        }
    }

    #[test]
    fn occl_cpu_draw_list_drops_the_hidden_fighters() {
        let camera = camera();
        let lod = LodTable {
            id: 7,
            bands_m: vec![5.0, 20.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        };
        let mut snapshot = SceneSnapshot::default();
        for (index, (center, _)) in fighters().iter().enumerate() {
            snapshot.nodes.push(SceneNode {
                id: u64::try_from(index).unwrap_or(0) + 1,
                mesh: 3,
                center_m: *center,
                radius_m: 0.4,
                lod_levels: 3,
                lod_table: lod.id,
                prev_lod: 0,
            });
        }
        let mut buffer = OcclusionBuffer::new(&camera).expect("the camera projects");
        buffer.add_quad(&wall());
        buffer.update_mips();
        let mut listed = DrawList::new();
        let all = listed
            .build(&snapshot, &camera, &lod)
            .expect("the scene builds");
        let kept = all.len();
        let mut occluded = DrawList::new();
        let lit = occluded
            .build_occl(&snapshot, &camera, &lod, Some(&buffer))
            .expect("the scene builds");
        assert_eq!(kept, 12, "the frustum keeps all 12");
        assert_eq!(lit.len(), 4, "the wall hides the 8 behind it");
    }
}
