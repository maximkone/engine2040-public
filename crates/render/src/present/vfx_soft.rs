//! vfx_soft: the soft-depth seam the blended sprite and ribbon passes share.
//!
//! Owner: render lane. The particle quads (`shaders/particle.wgsl`) and the sword
//! ribbon (`shaders/trail.wgsl`) draw in their own colour-only passes after the
//! forward pass, so the hardware depth test is not there to stop a ribbon cutting
//! through a body or a dust quad slicing into the floor. Both shaders read the
//! frame's stored depth instead (group 1, binding 0) and fade by the distance to
//! the surface behind them: a soft contact, never a hard line. One bind group per
//! target size holds the depth view; the uniforms of both passes end with the same
//! [`VfxFrame`] block (view-depth constants, the depth-valid flag, the light).
//! Units: meters, linear light. Determinism: fixed field order, no wall clock.
//! Errors: none here. Budget: one depth `textureLoad` per covered fragment.

/// Bind group index of the scene depth in both shaders.
pub const VFX_DEPTH_GROUP: u32 = 1;

/// Bytes of the [`VfxFrame`] block both uniforms end with: two depth constants,
/// the depth-valid word and its pad (16 bytes), then the light `vec4` (16 bytes).
pub const VFX_FRAME_BYTES: usize = 32;

/// Per-frame constants the soft-depth fade and the lit sprites read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VfxFrame {
    /// `(m22, m32)` of the standard `0..1` perspective: view depth is
    /// `m32 / (z_ndc + m22)`. See [`vfx_zparams`].
    pub zparams: [f32; 2],
    /// True when the depth bind holds this frame's forward depth. False (the 4x
    /// path keeps no 1x depth) reads every pixel as open air: no fade.
    pub depth_valid: bool,
    /// Linear light a lit sprite multiplies its colour by (rgb; `w` unused).
    pub light: [f32; 4],
}

impl Default for VfxFrame {
    fn default() -> Self {
        Self {
            zparams: vfx_zparams(0.1, 100.0),
            depth_valid: false,
            light: [1.0, 1.0, 1.0, 0.0],
        }
    }
}

/// View-depth constants for a standard (near maps to 0, far to 1) perspective:
/// `z_ndc = m32 / d - m22` for view depth `d`, so `d = m32 / (z_ndc + m22)`.
/// A degenerate range falls back to the default arena range. Pure.
#[must_use]
pub fn vfx_zparams(near_m: f32, far_m: f32) -> [f32; 2] {
    let ok = near_m.is_finite() && far_m.is_finite() && near_m > 0.0 && far_m > near_m * 1.001;
    let (near, far) = if ok { (near_m, far_m) } else { (0.1, 100.0) };
    let span = near - far;
    [far / span, near * far / span]
}

/// Pack the block both uniforms end with. Deterministic: fixed field order.
pub fn vfx_pack_frame(frame: &VfxFrame, out: &mut [u8]) {
    debug_assert_eq!(out.len(), VFX_FRAME_BYTES);
    let words: [[u8; 4]; 8] = [
        frame.zparams[0].to_le_bytes(),
        frame.zparams[1].to_le_bytes(),
        u32::from(frame.depth_valid).to_le_bytes(),
        0f32.to_le_bytes(),
        frame.light[0].to_le_bytes(),
        frame.light[1].to_le_bytes(),
        frame.light[2].to_le_bytes(),
        frame.light[3].to_le_bytes(),
    ];
    for (slot, word) in words.iter().enumerate() {
        out[slot * 4..slot * 4 + 4].copy_from_slice(word);
    }
}

/// True when every number of the block is finite; a bad block is refused before
/// it reaches a uniform.
#[must_use]
pub fn vfx_frame_finite(frame: &VfxFrame) -> bool {
    frame
        .zparams
        .iter()
        .chain(frame.light.iter())
        .all(|v| v.is_finite())
}

/// The light a lit sprite multiplies its colour by, the way the forward pass
/// lights a white diffuse face: the sun's radiance (`light_color`, already
/// multiplied by its intensity) over pi for the diffuse lobe, scaled by how much
/// of it a camera-facing quad catches, plus the sky's mean radiance (the ambient
/// the forward pass adds as albedo times radiance). Pure; non-finite input reads
/// as white.
#[must_use]
pub fn vfx_light(sun_rgb: [f32; 3], ambient_rgb: [f32; 3]) -> [f32; 4] {
    const SUN_CATCH: f32 = 0.55;
    let mut out = [0.0f32; 4];
    for (slot, (sun, amb)) in out.iter_mut().zip(sun_rgb.iter().zip(ambient_rgb.iter())) {
        let lit = sun * SUN_CATCH / std::f32::consts::PI + amb;
        *slot = if lit.is_finite() { lit.max(0.0) } else { 1.0 };
    }
    out
}

/// Bind-group layout of the depth the two shaders read: one non-filterable
/// `texture_depth_2d` in the fragment stage. Fixed shape, never rebuilt.
pub fn vfx_depth_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("vfx-depth-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

/// The depth bind group over `depth_view` (a `DEPTH_FORMAT` view created with
/// `TEXTURE_BINDING`). Built once per target size by the presenter.
pub fn vfx_depth_bind(device: &wgpu::Device, depth_view: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vfx-depth-bind"),
        layout: &vfx_depth_layout(device),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(depth_view),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// View depth a standard `0..1` perspective stores for `d`, written the
    /// way the matrix does it, then read back through the shader's formula.
    fn round_trip(near: f32, far: f32, d: f32) -> f32 {
        let [m22, m32] = vfx_zparams(near, far);
        let z_ndc = m32 / d - m22;
        m32 / (z_ndc + m22)
    }

    #[test]
    fn zparams_map_the_depth_range_onto_zero_to_one() {
        let (near, far) = (0.1_f32, 100.0_f32);
        let [m22, m32] = vfx_zparams(near, far);
        let z_at = |d: f32| m32 / d - m22;
        assert!(z_at(near).abs() < 1e-5, "near maps to 0");
        assert!((z_at(far) - 1.0).abs() < 1e-5, "far maps to 1");
        assert!(z_at(1.0) < z_at(2.0), "deeper is a larger depth value");
        for d in [0.3, 1.0, 4.0, 12.0, 60.0] {
            let back = round_trip(near, far, d);
            assert!((back - d).abs() < d * 1e-3, "{d} came back {back}");
        }
    }

    #[test]
    fn zparams_refuse_a_degenerate_range() {
        let fallback = vfx_zparams(0.1, 100.0);
        for (near, far) in [
            (0.0, 10.0),
            (5.0, 5.0),
            (f32::NAN, 10.0),
            (1.0, f32::INFINITY),
        ] {
            assert_eq!(vfx_zparams(near, far), fallback, "{near} {far}");
        }
    }

    #[test]
    fn the_frame_block_packs_in_shader_order() {
        let frame = VfxFrame {
            zparams: [-1.5, -0.25],
            depth_valid: true,
            light: [0.5, 0.6, 0.7, 0.0],
        };
        let mut bytes = [0u8; VFX_FRAME_BYTES];
        vfx_pack_frame(&frame, &mut bytes);
        let f = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        let u = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!((f(0), f(4)), (-1.5, -0.25));
        assert_eq!(u(8), 1, "the depth-valid word");
        assert_eq!((f(16), f(20), f(24)), (0.5, 0.6, 0.7));
        let mut again = [0u8; VFX_FRAME_BYTES];
        vfx_pack_frame(&frame, &mut again);
        assert_eq!(bytes, again, "the same block packs the same bytes");
        assert!(vfx_frame_finite(&frame));
        let bad = VfxFrame {
            light: [f32::NAN, 0.0, 0.0, 0.0],
            ..frame
        };
        assert!(!vfx_frame_finite(&bad));
    }

    #[test]
    fn the_light_follows_the_sun_and_never_goes_negative() {
        let day = vfx_light([3.0, 2.8, 2.4], [0.3, 0.3, 0.3]);
        let dusk = vfx_light([0.6, 0.3, 0.2], [0.1, 0.1, 0.2]);
        assert!(
            day[0] > dusk[0] && day[1] > dusk[1],
            "a brighter sun lights more"
        );
        assert!(day[0] > day[2], "a warm sun keeps its cast");
        let bad = vfx_light([f32::NAN, 1.0, 1.0], [0.1, 0.1, 0.1]);
        assert_eq!(bad[0], 1.0, "a bad number reads as white");
        let dark = vfx_light([-1.0, -1.0, -1.0], [0.0, 0.0, 0.0]);
        assert!(dark.iter().all(|c| *c >= 0.0));
    }
}
