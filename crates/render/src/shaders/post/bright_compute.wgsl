
struct BrightU {
    params: vec4<f32>,
    dims: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: BrightU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
// Kino soft-knee weight: `params.x` is the threshold, `params.y` the knee
// fraction. Absolute knee = threshold * fraction + 1e-5; the curve triple is
// (threshold - knee, 2 knee, 0.25 / knee). Mirrors `kino_prefilter_weight`.
fn bloom_w(luma: f32) -> f32 {
    let threshold = u.params.x;
    let knee = threshold * u.params.y + 0.00001;
    let c0 = threshold - knee;
    let c1 = knee * 2.0;
    let c2 = 0.25 / knee;
    if (luma <= c0) {
        return 0.0;
    }
    if (luma >= c0 + c1) {
        return 1.0;
    }
    let rq = clamp(luma - c0, 0.0, c1);
    let soft = c2 * rq * rq;
    let over = luma - threshold;
    return max(soft, over) / max(luma, 0.00001);
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let p = src[idx].rgb;
    let luma = dot(p, vec3<f32>(0.2126, 0.7152, 0.0722));
    dst[idx] = vec4<f32>(p * bloom_w(luma), 1.0);
}
