//! Tests for [`super::gpu`]: the real passes on a real device. The mask draws a
//! target over a depth prefill, the edge chain Sobels it and the composite lands
//! in the readback target. The proof the row asks for is on-vs-off pixels: a
//! target behind the berm reads the dim warm edge, and the same frame with no
//! selection draws nothing.

use super::{depth_for, EdgeGlobals, OutlineFrame, OutlineGpu, EDGE_WGSL, MASK_WGSL};
use crate::outline::{OutlineTarget, OutlineView};

/// The test frame: 128x72 runs the chain at 64x36, and 64 four-byte depth
/// texels are exactly one 256-byte row, so the prefill upload needs no padding.
const FRAME_W: u32 = 128;
const FRAME_H: u32 = 72;

/// The camera: eye at the ring edge looking along +x at the berm, the arena's
/// 60 degree vertical field of view.
fn view() -> OutlineView {
    OutlineView::new(
        [0.0, 1.5, 0.0],
        [10.0, 1.5, 0.0],
        60.0_f32.to_radians(),
        128.0 / 72.0,
        0.1,
        200.0,
    )
    .expect("the arena view has a projection")
}

/// A device and queue, or a routed skip when the machine has no adapter.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    crate::test_gpu::hold_gpu_slot();
    let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
    let probe = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|error| {
        crate::test_gpu::NoAdapter(format!("no wgpu adapter on this machine: {error}"))
    });
    let adapter = crate::test_gpu::require_or_skip("outline_gpu", probe)?;
    let pair = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("sw150-outline"),
        ..Default::default()
    }))
    .expect("a device follows from an adapter");
    Some(pair)
}

/// A depth prefill at one distance for every texel: the berm standing between
/// the eye and the target. In metres, so the test names the geometry rather than
/// a depth value.
fn prefill(device: &wgpu::Device, queue: &wgpu::Queue, z_m: f32) -> wgpu::Texture {
    let [width, height] = [FRAME_W / 2, FRAME_H / 2];
    let depth = depth_for(z_m, 0.1, 200.0);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sw150-test-prefill"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut bytes = Vec::with_capacity((width * height) as usize * 4);
    for _ in 0..width * height {
        bytes.extend_from_slice(&depth.to_le_bytes());
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Run one frame of the chain and read the composite back.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chain: &OutlineGpu,
    z_m: f32,
    targets: &[OutlineTarget],
) -> Vec<[u8; 4]> {
    let depth = prefill(device, queue, z_m);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sw150-test-frame"),
    });
    let camera = view();
    chain.encode(
        device,
        queue,
        &mut enc,
        &OutlineFrame {
            view: &camera,
            prefill: &depth_view,
            prefill_size: [FRAME_W / 2, FRAME_H / 2],
            targets,
        },
    );
    queue.submit(Some(enc.finish()));
    chain.read_out(device, queue)
}

/// The brightest texel of a readback, `[r, g, b]`.
fn peak(pixels: &[[u8; 4]]) -> [u8; 3] {
    let mut best = [0u8; 3];
    for p in pixels {
        if p[0] >= best[0] {
            best = [p[0], p[1], p[2]];
        }
    }
    best
}

/// F2P on the device: the target 1 m behind the berm reads the dim warm edge,
/// and the same frame with no selection draws nothing at all.
#[test]
fn outline_gpu_hidden_target_reads_the_dim_warm_edge() {
    let Some((device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let chain = OutlineGpu::new(&device, crate::quality::Quality::High, FRAME_W, FRAME_H)
        .expect("the high tier draws the outline");
    assert_eq!(chain.mask_size(), [64, 36], "the chain runs at half res");
    chain.upload_mesh(&queue);
    let behind = [OutlineTarget {
        id: 7,
        center: [11.5, 1.5, 0.0],
        radius_m: 0.7,
    }];
    let covered = render(&device, &queue, &chain, 10.0, &behind);
    let hot = peak(&covered);
    assert!(hot[0] > 8, "the covered target still draws, got {hot:?}");
    assert!(
        hot[0] >= hot[1] && hot[1] >= hot[2] && hot[0] > hot[2],
        "the hidden band is warm (r >= g >= b), got {hot:?}"
    );
    assert!(
        hot[0] <= 120,
        "the hidden band is dim, not white, got {hot:?}"
    );
    // Off: the same frame, nothing selected, no draw.
    let off = OutlineGpu::new(&device, crate::quality::Quality::High, FRAME_W, FRAME_H)
        .expect("the high tier draws the outline");
    off.upload_mesh(&queue);
    let empty = render(&device, &queue, &off, 10.0, &[]);
    assert!(
        empty.iter().all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "no selection draws nothing, got {:?}",
        peak(&empty)
    );
    // On-vs-off: the pass is what put the band there.
    assert_ne!(peak(&covered), peak(&empty));
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "no validation error from the chain: {err:?}");
}

/// The same target in front of the berm reads the visible white on the device.
#[test]
fn outline_gpu_open_target_reads_the_visible_white() {
    let Some((device, queue)) = device() else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let chain = OutlineGpu::new(&device, crate::quality::Quality::Comfort, FRAME_W, FRAME_H)
        .expect("the comfort tier draws the outline");
    chain.upload_mesh(&queue);
    // The prefill sits 20 m away, so the 11.5 m target stands in front of it.
    let open = [OutlineTarget {
        id: 3,
        center: [11.5, 1.5, 0.0],
        radius_m: 0.7,
    }];
    let pixels = render(&device, &queue, &chain, 20.0, &open);
    let hot = peak(&pixels);
    assert!(hot[0] > 120, "the open target draws bright, got {hot:?}");
    let spread = hot[0].abs_diff(hot[1]).max(hot[1].abs_diff(hot[2]));
    assert!(spread <= 8, "the visible band is white, got {hot:?}");
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "no validation error from the chain: {err:?}");
}

/// The potato floor allocates nothing: the chain refuses to build.
#[test]
fn outline_gpu_potato_builds_no_chain() {
    let Some((device, _queue)) = device() else {
        return;
    };
    assert!(
        OutlineGpu::new(&device, crate::quality::Quality::Potato, FRAME_W, FRAME_H).is_none(),
        "the potato tier draws no outline"
    );
    assert!(
        OutlineGpu::new(&device, crate::quality::Quality::High, 0, FRAME_H).is_none(),
        "a zero-area frame draws no outline"
    );
}

/// CQ23's gate over the two shipped files, in the run that proves the pipelines:
/// a type error here fails the same run as a broken pipeline build.
#[cfg(feature = "shader-validate")]
#[test]
fn outline_shaders_validate() {
    for (name, source) in [
        ("outline_mask.wgsl", MASK_WGSL),
        ("outline_edge.wgsl", EDGE_WGSL),
    ] {
        crate::shader_gate::validate_source(name, source)
            .unwrap_or_else(|e| panic!("{name} fails naga: {e}"));
    }
}
