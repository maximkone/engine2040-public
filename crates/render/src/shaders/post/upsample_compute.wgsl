
struct UpU {
    dims: vec4<u32>,
    factors: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: UpU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
fn src_bilinear(uu: f32, vv: f32) -> vec3<f32> {
    let sw = u.dims.x;
    let sh = u.dims.y;
    let x = clamp(uu, 0.0, f32(sw) - 1.0);
    let y = clamp(vv, 0.0, f32(sh) - 1.0);
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, sw - 1u);
    let y1 = min(y0 + 1u, sh - 1u);
    let tx = clamp(x - f32(x0), 0.0, 1.0);
    let ty = clamp(y - f32(y0), 0.0, 1.0);
    let a = src[y0 * sw + x0].rgb;
    let b = src[y0 * sw + x1].rgb;
    let c = src[y1 * sw + x0].rgb;
    let d = src[y1 * sw + x1].rgb;
    return mix(mix(a, b, tx), mix(c, d, tx), ty);
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dw = u.dims.z;
    let dh = u.dims.w;
    if (gid.x >= dw || gid.y >= dh) {
        return;
    }
    let fx = (f32(gid.x) + 0.5) * u.factors.x - 0.5;
    let fy = (f32(gid.y) + 0.5) * u.factors.y - 0.5;
    dst[gid.y * dw + gid.x] = vec4<f32>(src_bilinear(fx, fy), 1.0);
}
