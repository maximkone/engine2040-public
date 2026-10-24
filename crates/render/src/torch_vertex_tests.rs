//! SW5 accept proofs: the vertex-rate torch sum holds within one percent of the
//! per-fragment loop on a potato-tier card, and the knob picks it on the potato
//! row alone with a shipped shader that carries both entries.

use super::*;
use crate::quality::{presentation, Quality, TorchLighting};
use crate::torch_light::TorchLight;

/// The torch frame the test uses: the eight arena torches in front of one wall,
/// warm, four meters of reach, three to one and a half meters away.
fn torches() -> Vec<TorchLight> {
    let xs = [-1.6_f32, -0.8, 0.0, 0.8, 1.6, -0.4, 0.4, 0.0];
    xs.iter()
        .enumerate()
        .map(|(i, x)| {
            let at = i as f32;
            TorchLight {
                pos: [*x, 1.2 + 0.1 * at.sin(), 0.96 + 0.15 * (2.0 * at).cos()],
                color: [1.0, 0.62, 0.28],
                range_m: 4.0,
                intensity: 7.0,
            }
        })
        .collect()
}

/// The wall: albedo and the surface normal, flat toward the camera.
const ALBEDO: [f32; 3] = [0.5, 0.45, 0.4];
const WALL_N: [f32; 3] = [0.0, 0.0, 1.0];

/// Peak-relative error of the vertex rate over one square card of `edge` meters
/// centered on the wall, sampled on an 11x11 grid, plus the two evaluation
/// counts (vertex rate, fragment rate).
fn card_error(edge: f32) -> (f32, u32, u32) {
    let torch_list = torches();
    let half = edge / 2.0;
    let corners = [
        [-half, -half, 0.0],
        [-half, half, 0.0],
        [half, -half, 0.0],
        [half, half, 0.0],
    ];
    let corner_sums = corners.map(|at| pixel_rate_sum(at, WALL_N, ALBEDO, &torch_list));
    let mut peak = 0.0_f32;
    let mut worst = 0.0_f32;
    let mut samples = 0_u32;
    for i in 0..=10 {
        for j in 0..=10 {
            let u = i as f32 / 10.0;
            let v = j as f32 / 10.0;
            let at = [-half + u * edge, -half + v * edge, 0.0];
            let exact = pixel_rate_sum(at, WALL_N, ALBEDO, &torch_list);
            for channel in 0..3 {
                peak = peak.max(exact[channel]);
            }
            let approx = vertex_rate_sum(corner_sums, u, v);
            worst = worst.max(peak_relative_error(exact, approx, 1.0));
            samples += 1;
        }
    }
    let per_evaluation = torch_list.len() as u32;
    (worst / peak, per_evaluation * 4, per_evaluation * samples)
}

/// SW5 F2P: on a potato-tier card (15 cm, a hand-sized prop) the four-corner sum
/// interpolated across the card keeps the highlight within one percent of the
/// per-fragment loop, for 32 light evaluations instead of one per fragment.
#[test]
fn torch_vertex_sum_matches_the_pixel_loop_within_one_percent() {
    let (error, vertex_evaluations, fragment_evaluations) = card_error(0.15);
    assert!(
        error < 0.01,
        "vertex-rate torch sum is {error} of the highlight, over one percent"
    );
    assert_eq!(vertex_evaluations, 32, "8 lights at 4 vertices per card");
    assert_eq!(
        fragment_evaluations, 968,
        "the per-fragment loop pays 8 lights at 121 fragments"
    );
    // The claim is bounded by card size, not universal: a one-meter card shows
    // the interpolation error the potato tier accepts on a big wall quad.
    let (big, _, _) = card_error(1.0);
    assert!(big > 0.01, "a big card should show the limit, got {big}");
}

/// SW5: the knob picks the vertex rate on the potato row alone, and the shipped
/// shader carries the two entries the pipeline pairs. The pixel path in
/// `forward.wgsl` keeps its per-fragment loop, so no tier loses it.
#[test]
fn torch_vertex_knob_picks_the_potato_row_and_the_shader_carries_it() {
    assert_eq!(
        presentation(Quality::Potato).torch_lighting,
        TorchLighting::Vertex,
        "the potato row pays the vertex rate"
    );
    for tier in Quality::ALL.into_iter().filter(|t| *t != Quality::Potato) {
        assert_eq!(
            presentation(tier).torch_lighting,
            TorchLighting::Pixel,
            "{tier:?} keeps the per-fragment loop"
        );
    }
    assert_ne!(TorchLighting::Pixel.name(), TorchLighting::Vertex.name());

    let vertex = include_str!("shaders/forward_torch_vertex.wgsl");
    assert!(
        vertex.contains("fn vs_torch(") && vertex.contains("@location(7) torch_sum: vec3<f32>"),
        "the vertex entry writes the interpolated torch sum"
    );
    assert!(
        vertex.contains("fn fs_torch(") && vertex.contains("in.torch_sum"),
        "the fragment entry adds the interpolated sum"
    );
    assert!(
        !vertex.contains("torch_frame_sum("),
        "the vertex variant must not run the per-fragment loop"
    );
    let pixel = include_str!("shaders/forward.wgsl");
    assert!(
        pixel.contains("+ torch_frame_sum(in.world_pos, mapped_n, albedo)"),
        "the pixel path keeps the per-fragment loop"
    );
}
