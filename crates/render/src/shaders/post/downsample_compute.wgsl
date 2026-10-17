
struct DownU {
    dims: vec4<u32>,
    // x = 1: Karis-weighted tap average (the first mip); 0: plain box.
    mode: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: DownU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let sw = u.dims.x;
    let sh = u.dims.y;
    let dw = u.dims.z;
    if (gid.x >= dw || gid.y >= u.dims.w) {
        return;
    }
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let sx = gid.x * 2u + dx;
            let sy = gid.y * 2u + dy;
            if (sx < sw && sy < sh) {
                let p = src[sy * sw + sx].rgb;
                // Karis: each tap weighs 1 / (luma + 1), so one hot texel
                // cannot own the first mip. Mirrors `downsample_karis`.
                var wt = 1.0;
                if (u.mode.x == 1u) {
                    wt = 1.0 / (dot(p, vec3<f32>(0.2126, 0.7152, 0.0722)) + 1.0);
                }
                acc += p * wt;
                wsum += wt;
            }
        }
    }
    let inv = select(0.0, 1.0 / wsum, wsum > 0.0);
    dst[gid.y * dw + gid.x] = vec4<f32>(acc * inv, 1.0);
}
