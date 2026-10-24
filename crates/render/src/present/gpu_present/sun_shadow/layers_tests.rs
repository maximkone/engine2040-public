//! SW1: the sun map's cascade layers and the cadence that skips the far ones.
use super::*;
use crate::arena_day::ARENA_DAY_SUN_DIR;
use crate::shadow_cascade::CascadeCadence;

/// Sand points of a 100 m square, every 5 m, that land inside `vp`.
fn sand_points_held(vp: [[f32; 4]; 4]) -> usize {
    let steps = (-10i16..=10).map(|k| 5.0 * f32::from(k));
    steps
        .clone()
        .flat_map(|x| steps.clone().map(move |z| [x, 0.0, z]))
        .filter(|p| matches!(sun_uv_depth(vp, *p), Ok(Some(_))))
        .count()
}

#[test]
fn layer_zero_is_the_pit_map_and_each_far_layer_holds_more_sand() {
    let near = sun_layer_view_proj(ARENA_DAY_SUN_DIR, 0).expect("layer 0 builds");
    assert_eq!(
        near,
        sun_view_proj_for_dir(ARENA_DAY_SUN_DIR).expect("pit map builds"),
        "layer 0 is the live pit matrix bit for bit"
    );
    let held: Vec<usize> = (0..SUN_LAYERS)
        .map(|layer| {
            sand_points_held(sun_layer_view_proj(ARENA_DAY_SUN_DIR, layer).expect("layer builds"))
        })
        .collect();
    assert!(
        held[0] < held[1] && held[1] < held[2],
        "each far layer holds more of the bowl: {held:?}"
    );
    assert!(
        sun_layer_view_proj(ARENA_DAY_SUN_DIR, SUN_LAYERS).is_err(),
        "a fourth layer is refused"
    );
}

#[test]
fn cascade_two_draws_once_in_four() {
    // The potato row's periods: near every frame, mid every second, far every
    // fourth, all three drawn on the first frame (nothing drawn before it).
    let mut cadence = CascadeCadence::new([1, 2, 4]);
    let mut draws = [0u32; SUN_LAYERS];
    for frame in 0..4u64 {
        let plan = plan_sun_layers(&mut cadence, frame, ARENA_DAY_SUN_DIR, SUN_LAYERS)
            .expect("the arena sun plans");
        for (count, draw) in draws.iter_mut().zip(plan) {
            *count += u32::from(draw.is_some());
        }
    }
    assert_eq!(draws, [4, 2, 1], "periods [1, 2, 4] over four frames");
    // A frame capped at one layer plans only the near one.
    let mut capped = CascadeCadence::new([1, 1, 1]);
    let plan = plan_sun_layers(&mut capped, 0, ARENA_DAY_SUN_DIR, 1).expect("plans");
    assert!(plan[0].is_some() && plan[1].is_none() && plan[2].is_none());
    // A bad sun fails before the cadence moves.
    let before = cadence;
    assert!(plan_sun_layers(&mut cadence, 4, [0.0; 3], SUN_LAYERS).is_err());
    assert_eq!(
        cadence, before,
        "a refused frame leaves the cadence as it was"
    );
}

#[test]
fn sun_layers_cadence_keeps_the_uniform_of_a_skipped_cascade() {
    // The GPU-side uniforms as bytes, written only by a due layer: on every
    // frame a skipped layer's bytes are the ones it was drawn with.
    let mut cadence = CascadeCadence::new([1, 2, 4]);
    let mut store: [Option<[u8; 80]>; SUN_LAYERS] = [None; SUN_LAYERS];
    let mut skipped = 0u32;
    for frame in 0..8u64 {
        let plan = plan_sun_layers(&mut cadence, frame, ARENA_DAY_SUN_DIR, SUN_LAYERS)
            .expect("the arena sun plans");
        for (layer, draw) in plan.iter().enumerate() {
            match draw {
                Some((vp, bytes)) => {
                    assert_eq!(*bytes, sun_shadow_uniform_bytes(*vp, SUN_SHADOW_BIAS, 1.0));
                    store[layer] = Some(*bytes);
                }
                None => {
                    let kept = store[layer].expect("a skipped layer was drawn before");
                    let drawn = cadence.drawn_with(layer).expect("its matrix is kept");
                    assert_eq!(
                        kept,
                        sun_shadow_uniform_bytes(*drawn, SUN_SHADOW_BIAS, 1.0),
                        "frame {frame} layer {layer}: the skipped uniform is the drawn one"
                    );
                    skipped += 1;
                }
            }
        }
    }
    assert_eq!(skipped, 4 + 6, "mid skips 4 of 8 frames, far 6 of 8");
    // The sun moves on an off frame: every layer re-renders at once.
    let moved = [
        ARENA_DAY_SUN_DIR[0] + 0.05,
        ARENA_DAY_SUN_DIR[1],
        ARENA_DAY_SUN_DIR[2],
    ];
    let plan = plan_sun_layers(&mut cadence, 9, moved, SUN_LAYERS).expect("moved sun plans");
    assert!(plan.iter().all(Option::is_some), "a sun move redraws all");
}

/// Depths of the live `edge` corner of one layer of the live map.
fn read_layer(presenter: &crate::present::Presenter, layer: u32) -> Vec<u32> {
    let edge = presenter.sun_sub.edge();
    let padded = (edge * 4).div_ceil(256) * 256;
    let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sw1-layer-readback"),
        size: u64::from(padded) * u64::from(edge),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = presenter
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sw1-layer-copy"),
        });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: presenter.sun_sub.map(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(edge),
            },
        },
        wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: 1,
        },
    );
    presenter.queue.submit(Some(enc.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    presenter
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("readback poll holds");
    rx.recv()
        .expect("readback channel holds")
        .expect("readback maps");
    let (w, stride) = (
        usize::try_from(edge).expect("edge fits"),
        usize::try_from(padded).expect("stride fits"),
    );
    let mut bits = Vec::with_capacity(w * w);
    {
        let mapped = slice.get_mapped_range().expect("mapped range holds");
        for y in 0..w {
            for x in 0..w {
                let at = y * stride + x * 4;
                bits.push(u32::from_le_bytes([
                    mapped[at],
                    mapped[at + 1],
                    mapped[at + 2],
                    mapped[at + 3],
                ]));
            }
        }
    }
    buffer.unmap();
    bits
}

#[test]
fn the_live_map_keeps_a_skipped_layer_and_clears_a_due_one() {
    use crate::pass_truth::harness::fixtures::{init_or_skip, sun_shadow_scene, H, W};
    let Some(mut presenter) = init_or_skip("the_live_map_keeps_a_skipped_layer") else {
        return;
    };
    let list = sun_shadow_scene(&mut presenter, W, H);
    let empty = crate::present::DrawList {
        items: Vec::new(),
        ..list.clone()
    };
    presenter.sun_sub.resize(256);
    presenter.sun_sub.set_cascades(SUN_LAYERS, [1, 2, 4]);
    let record = |presenter: &crate::present::Presenter, list: &crate::present::DrawList| {
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sw1-sun-frame"),
            });
        let us = presenter.sun_sub.record(presenter, &mut enc, list, true);
        presenter.queue.submit(Some(enc.finish()));
        us
    };
    let scope = presenter
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    let clear = 1.0f32.to_bits();
    // Frame 0 draws the box and floor into all three layers.
    record(&presenter, &list);
    let far_drawn = read_layer(&presenter, 2);
    assert!(
        far_drawn.iter().any(|&d| d != clear),
        "the far layer holds the scene"
    );
    assert!(read_layer(&presenter, 0).iter().any(|&d| d != clear));
    // Frames 1 to 3 draw nothing: the near layer is due every frame and
    // clears, the far one is not due and keeps its depths byte for byte.
    for _ in 1..4 {
        record(&presenter, &empty);
    }
    assert!(
        read_layer(&presenter, 0).iter().all(|&d| d == clear),
        "the near layer re-rendered the empty frame"
    );
    let far_kept = read_layer(&presenter, 2);
    let differ = far_kept
        .iter()
        .zip(&far_drawn)
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differ, 0,
        "the skipped far layer kept its depths: {differ} differ"
    );
    // Frame 4 is the far layer's period: it re-renders the empty frame.
    record(&presenter, &empty);
    assert!(
        read_layer(&presenter, 2).iter().all(|&d| d == clear),
        "the far layer re-rendered on its period"
    );
    assert!(
        pollster::block_on(scope.pop()).is_none(),
        "0 validation errors"
    );
}
