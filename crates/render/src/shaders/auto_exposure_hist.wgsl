// auto_exposure_hist.wgsl: the 64-bin log-luma histogram behind the meter.
//
// One thread per sampled texel on the CPU oracle's pattern (`hist_stride`:
// x % stride == 0 and y % stride == 0), one atomic add per sample. The bin
// edges are `post::auto_exposure`'s: AUTO_LOG_MIN_EV (-12) to
// AUTO_LOG_MAX_EV (+8) over 64 bins (0.3125 EV each), non-positive luma in
// bin 0, the top edge in bin 63. A plain u32 count per bin; the meter reads
// them back as f32 (exact below 2^24 samples).
//
// Caller (next step): a presenter compute dispatch after the forward pass,
// then `AutoExposure::tick_hist` over the readback. Mirrors the CPU oracle
// `crate::post::auto_exposure::histogram_log_luma`, so the same frame bins to
// the same counts on either side.

struct HistU {
    // Full frame size in texels.
    size: vec2<u32>,
    // Sample every `stride` texels in x and y (1 = every texel).
    stride: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> u: HistU;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> bins: array<atomic<u32>, 64>;

const LOG_MIN_EV: f32 = -12.0;
const LOG_MAX_EV: f32 = 8.0;
// Bin width in stops: 20 stops over 64 bins, exactly 0.3125. The CPU
// oracle divides by the same constant, so one sample lands in the same
// bin on both sides.
const BIN_WIDTH_EV: f32 = 0.3125;
const BIN_COUNT: f32 = 64.0;

@compute @workgroup_size(8, 8, 1)
fn hist_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let texel = id.xy * u.stride;
    if (texel.x >= u.size.x || texel.y >= u.size.y) {
        return;
    }
    let c = textureLoad(hdr, vec2<i32>(i32(texel.x), i32(texel.y)), 0);
    // Rec.709 luma, the same weights the CPU oracle reads.
    let luma = max(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0);
    var ev = LOG_MIN_EV;
    if (luma > 0.0) {
        ev = clamp(log2(luma), LOG_MIN_EV, LOG_MAX_EV);
    }
    let scaled = (ev - LOG_MIN_EV) / BIN_WIDTH_EV;
    let bin = u32(clamp(scaled, 0.0, BIN_COUNT - 1.0));
    atomicAdd(&bins[bin], 1u);
}
