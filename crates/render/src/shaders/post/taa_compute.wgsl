// taa_compute.wgsl: history clip plus resolve for the post chain.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/taa.fragment.fx): history resolve shape, reimplemented, zero bytes vendored.
struct TaaU {
    dims: vec4<u32>,
    params: vec4<f32>,
    jitter: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: TaaU;
@group(0) @binding(1) var<storage, read> cur: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> hist: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> mot: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> dst: array<vec4<f32>>;
fn cur_at(x: i32, y: i32) -> vec3<f32> {
    let w = i32(u.dims.x);
    let h = i32(u.dims.y);
    let cx = u32(clamp(x, 0, w - 1));
    let cy = u32(clamp(y, 0, h - 1));
    return cur[cy * u.dims.x + cx].rgb;
}
fn hist_bilinear(uu: f32, vv: f32) -> vec3<f32> {
    let w = u.dims.x;
    let h = u.dims.y;
    let x = clamp(uu, 0.0, f32(w) - 1.0);
    let y = clamp(vv, 0.0, f32(h) - 1.0);
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, w - 1u);
    let y1 = min(y0 + 1u, h - 1u);
    let tx = clamp(x - f32(x0), 0.0, 1.0);
    let ty = clamp(y - f32(y0), 0.0, 1.0);
    let a = hist[y0 * w + x0].rgb;
    let b = hist[y0 * w + x1].rgb;
    let c = hist[y1 * w + x0].rgb;
    let d = hist[y1 * w + x1].rgb;
    return mix(mix(a, b, tx), mix(c, d, tx), ty);
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let mv = mot[idx].xy;
    let speed = length(mv);
    let blend = select(u.params.x, 0.0, speed > u.params.y);
    let hs = hist_bilinear(f32(gid.x) - mv.x + u.jitter.x, f32(gid.y) - mv.y + u.jitter.y);
    var kept = hs;
    if (u.params.z > 0.5) {
        var lo = cur[idx].rgb;
        var hi = cur[idx].rgb;
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let t = cur_at(i32(gid.x) + dx, i32(gid.y) + dy);
                lo = min(lo, t);
                hi = max(hi, t);
            }
        }
        kept = clamp(hs, lo, hi);
    }
    let c = cur[idx].rgb;
    dst[idx] = vec4<f32>(mix(c, kept, blend), 1.0);
}
