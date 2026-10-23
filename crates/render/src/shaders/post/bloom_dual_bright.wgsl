// bloom_dual_bright.wgsl: mip 0 of the live dual-filter bloom, the bright
// extract at half the frame size. Each destination texel averages its 2x2
// source taps, each tap soft-knee extracted first and weighted by
// `1 / (luma + 1)` (Karis), so one hot texel cannot own the mip and a frame
// with no texel above the knee writes exactly zero.
//
// Owner: render lane. Caller: `present::gpu_present::bloom_dual` (one dispatch
// per frame, before the blit). The CPU twin is `bloom_dual::oracle::bright_half`
// (test builds); the knee curve is `post::bloom_weight`, mirrored below.
// The uniform is shared by the three bloom_dual shaders: source size,
// destination size, then the soft-knee pair.
struct BloomU {
    src: vec4<u32>,
    dst: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: BloomU;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var dst_tex: texture_storage_2d<rgba16float, write>;
// Kino soft-knee weight: quadratic under the foot, the line above threshold,
// one response (see `kino_prefilter_weight`).
fn bloom_soft_knee(luma: f32, threshold: f32, soft_knee: f32) -> f32 {
    let knee = threshold * soft_knee + 0.00001;
    let foot = threshold - knee;
    let span = knee * 2.0;
    if (luma <= foot) {
        return 0.0;
    }
    if (luma >= foot + span) {
        return 1.0;
    }
    let rq = clamp(luma - foot, 0.0, span);
    let quad = 0.25 / knee * rq * rq;
    return max(quad, luma - threshold) / max(luma, 0.00001);
}
@compute @workgroup_size(8, 8)
fn bright_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u.dst.x || y >= u.dst.y) {
        return;
    }
    let last = vec2<i32>(i32(u.src.x) - 1, i32(u.src.y) - 1);
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dy = 0u; dy < 2u; dy = dy + 1u) {
        for (var dx = 0u; dx < 2u; dx = dx + 1u) {
            let p = min(vec2<i32>(i32(x * 2u + dx), i32(y * 2u + dy)), last);
            let c = textureLoad(src_tex, p, 0).rgb;
            let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
            let weight = 1.0 / (max(luma, 0.0) + 1.0);
            acc = acc + c * bloom_soft_knee(luma, u.params.x, u.params.y) * weight;
            wsum = wsum + weight;
        }
    }
    let inv = select(0.0, 1.0 / wsum, wsum > 0.0);
    textureStore(dst_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(acc * inv, 1.0));
}
