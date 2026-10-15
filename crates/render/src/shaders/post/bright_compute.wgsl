
struct BrightU {
    params: vec4<f32>,
    dims: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: BrightU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
fn bloom_w(luma: f32) -> f32 {
    let threshold = u.params.x;
    let knee = u.params.y;
    if (luma <= threshold - knee) {
        return 0.0;
    }
    if (knee <= 0.0 || luma >= threshold + knee) {
        return 1.0;
    }
    let t = (luma - (threshold - knee)) / (2.0 * knee);
    return t * t;
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
