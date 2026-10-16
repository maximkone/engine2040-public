
struct DownU {
    dims: vec4<u32>,
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
    var n = 0.0;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let sx = gid.x * 2u + dx;
            let sy = gid.y * 2u + dy;
            if (sx < sw && sy < sh) {
                acc += src[sy * sw + sx].rgb;
                n += 1.0;
            }
        }
    }
    dst[gid.y * dw + gid.x] = vec4<f32>(acc / max(n, 1.0), 1.0);
}
