// lod_dither.wgsl: LOD dither cross-fade, no pop, no alpha sort.
//
// One 4x4 Bayer matrix shared with `lod_fade.rs`: a pixel survives when its
// cell value is below the per-draw keep threshold, so a fading LOD dissolves
// instead of popping. The forward pass binds `lod_keep` per fragment next
// slice; today this file is the compiled mirror of the CPU oracle.

// 4x4 Bayer cells in y-major order, values 0..=15, one permutation.
var<private> bayer4_table: array<u32, 16> = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);

// Cell value at a pixel, wrapping every 4 texels.
fn bayer4_at(x: u32, y: u32) -> u32 {
    return bayer4_table[(y & 3u) * 4u + (x & 3u)];
}

// True when the pixel survives at this fade share: 255 keeps all,
// 0 keeps none, matching `lod_fade::keep` on the CPU.
fn lod_keep(fade: u32, x: u32, y: u32) -> bool {
    let threshold = (fade + 15u) / 16u;
    return bayer4_at(x, y) < threshold;
}
