//! SW5: the potato tier's torch sum at vertex rate.
//!
//! Consumer: [`crate::quality::knobs`] picks
//! [`TorchLighting::Vertex`](crate::quality::TorchLighting::Vertex) on the
//! potato row alone, and the potato pipeline pairs
//! `shaders/forward_torch_vertex.wgsl` with the shipped pass: `vs_torch` sums
//! the frame torches at each vertex, `fs_torch` adds the interpolated value
//! once per fragment instead of running the 8-light loop there.
//!
//! This module is the CPU twin the accept measures: one light function, two
//! sampling rates (per fragment, and at a card's four corners then
//! interpolated), so the error the tier accepts is a number rather than a hope.
//!
//! Units: meters for positions and ranges, linear RGB for radiance, one
//! dimensionless intensity scale. Determinism: pure `f32` math over the
//! caller's slots in slot order, no clock, no RNG, no map iteration.
//! Presentation only: nothing here feeds the tick or the hash.

use super::{TorchLight, TORCH_MAX};

/// Radiance one torch adds at `pos` under `normal` for `albedo`: the CPU twin
/// of the WGSL `torch_light`. Lambert diffuse, inverse-square falloff, and the
/// squared warm range window; a torch out of range, or one behind the surface,
/// adds zero.
///
/// The Burley and GGX terms stay folded to one: this measures the sampling
/// rate, which is what the vertex variant changes, on a wall lit head-on.
pub fn contribution(
    pos: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    torch: &TorchLight,
) -> [f32; 3] {
    let delta = [
        torch.pos[0] - pos[0],
        torch.pos[1] - pos[1],
        torch.pos[2] - pos[2],
    ];
    let dist = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if torch.range_m <= 0.0 || dist >= torch.range_m {
        return [0.0; 3];
    }
    let inv = 1.0 / dist.max(0.0001);
    let l = [delta[0] * inv, delta[1] * inv, delta[2] * inv];
    let n_len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if n_len <= 0.0 {
        return [0.0; 3];
    }
    let n = [normal[0] / n_len, normal[1] / n_len, normal[2] / n_len];
    let ndot_l = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).clamp(0.0, 1.0);
    if ndot_l <= 0.0 {
        return [0.0; 3];
    }
    let att = 1.0 / (1.0 + dist * dist);
    let edge = 1.0 - dist / torch.range_m;
    let k = ndot_l * att * edge * edge * torch.intensity;
    [
        albedo[0] * torch.color[0] * k,
        albedo[1] * torch.color[1] * k,
        albedo[2] * torch.color[2] * k,
    ]
}

/// Sum the live torches at one point, capped at [`TORCH_MAX`] like the uniform
/// block: the shipped per-fragment loop.
pub fn pixel_rate_sum(
    pos: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    torches: &[TorchLight],
) -> [f32; 3] {
    let mut sum = [0.0_f32; 3];
    for torch in torches.iter().take(TORCH_MAX) {
        let one = contribution(pos, normal, albedo, torch);
        sum[0] += one[0];
        sum[1] += one[1];
        sum[2] += one[2];
    }
    sum
}

/// The value a card carries into its fragments: the four corner sums
/// (`[lower-left, upper-left, lower-right, upper-right]`, as the vertex stage
/// writes them) interpolated bilinearly at `(u, v)` in `0..=1`. Perspective
/// correcting the same pair is the GPU's job; on a flat wall facing the camera
/// the affine pair is the answer.
pub fn vertex_rate_sum(corners: [[f32; 3]; 4], u: f32, v: f32) -> [f32; 3] {
    let u = u.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let mut out = [0.0_f32; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        let low = corners[0][channel] + (corners[2][channel] - corners[0][channel]) * u;
        let high = corners[1][channel] + (corners[3][channel] - corners[1][channel]) * u;
        *slot = low + (high - low) * v;
    }
    out
}

/// Largest absolute channel difference between the exact and the interpolated
/// sum, over `peak` (the card's brightest exact channel): the error the
/// vertex rate adds, as a fraction of the highlight it is measured against.
pub fn peak_relative_error(exact: [f32; 3], approx: [f32; 3], peak: f32) -> f32 {
    let mut worst = 0.0_f32;
    for channel in 0..3 {
        worst = worst.max((exact[channel] - approx[channel]).abs());
    }
    if peak <= 0.0 {
        return worst;
    }
    worst / peak
}

#[cfg(test)]
#[path = "torch_vertex_tests.rs"]
mod tests;
