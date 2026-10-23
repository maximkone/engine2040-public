// bloom_dual_up.wgsl: one 3x3 tent upsample step of the live dual-filter
// bloom. It resamples the next smaller ladder mip onto this mip's grid with
// weights 1 2 1 / 2 4 2 / 1 2 1 over 16 (each tap a bilinear sample), then
// halves the sum with this mip's own down value:
// `up[i] = (down[i] + tent(up[i+1])) / 2`.
// The half blend keeps a flat frame's halo at the frame's own value while a
// lone glint still spreads over every rung of the ladder.
//
// Owner: render lane. Caller: `present::gpu_present::bloom_dual` (one dispatch
// per rung, smallest first; the blit samples rung 0). CPU twin:
// `bloom_dual::oracle::tent_upsample` plus the half blend in `dual_ladder`.
struct BloomU {
    src: vec4<u32>,
    dst: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: BloomU;
@group(0) @binding(1) var base_tex: texture_2d<f32>;
@group(0) @binding(2) var prev_tex: texture_2d<f32>;
@group(0) @binding(3) var prev_smp: sampler;
@group(0) @binding(4) var dst_tex: texture_storage_2d<rgba16float, write>;
// Texel-centre coordinate of destination texel `d` in a source image of `s`
// texels: UV `(d + 0.5) / dst`, read back at `uv * s - 0.5`.
fn dest_to_src(d: u32, dst: u32, s: u32) -> f32 {
    return (f32(d) + 0.5) * (f32(s) / f32(max(dst, 1u))) - 0.5;
}
fn tap(x: u32, y: u32, off: vec2<f32>) -> vec3<f32> {
    let c = vec2<f32>(dest_to_src(x, u.dst.x, u.src.x), dest_to_src(y, u.dst.y, u.src.y)) + off;
    let uv = (c + vec2<f32>(0.5, 0.5)) / vec2<f32>(f32(u.src.x), f32(u.src.y));
    return textureSampleLevel(prev_tex, prev_smp, uv, 0.0).rgb;
}
@compute @workgroup_size(8, 8)
fn up_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u.dst.x || y >= u.dst.y) {
        return;
    }
    var tent = vec3<f32>(0.0);
    for (var oy = 0; oy < 3; oy = oy + 1) {
        for (var ox = 0; ox < 3; ox = ox + 1) {
            let wx = select(1, 2, ox == 1);
            let wy = select(1, 2, oy == 1);
            let off = vec2<f32>(f32(ox - 1), f32(oy - 1));
            tent = tent + tap(x, y, off) * f32(wx * wy);
        }
    }
    let base = textureLoad(base_tex, vec2<i32>(i32(x), i32(y)), 0).rgb;
    let value = 0.5 * (base + tent * (1.0 / 16.0));
    textureStore(dst_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(value, 1.0));
}
