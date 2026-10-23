// bloom_dual_down.wgsl: one 13-tap downsample step of the live dual-filter
// bloom, a source mip into the next smaller one. Destination texel (x, y)
// sits at source coordinate (2x + 0.5, 2y + 0.5); every tap is a bilinear
// sample there, the four corner groups take 1/32 per tap and the centre group
// 1/8, so the thirteen taps sum to one and a flat mip keeps its value.
//
// Owner: render lane. Caller: `present::gpu_present::bloom_dual` (one dispatch
// per mip below mip 0). CPU twin: `bloom_dual::oracle::downsample13`.
struct BloomU {
    src: vec4<u32>,
    dst: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: BloomU;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_smp: sampler;
@group(0) @binding(3) var dst_tex: texture_storage_2d<rgba16float, write>;
// Texel-centre coordinate of destination texel `d` in a source image of `s`
// texels: UV `(d + 0.5) / dst`, read back at `uv * s - 0.5`.
fn dest_to_src(d: u32, dst: u32, s: u32) -> f32 {
    return (f32(d) + 0.5) * (f32(s) / f32(max(dst, 1u))) - 0.5;
}
fn tap(x: u32, y: u32, off: vec2<f32>) -> vec3<f32> {
    let c = vec2<f32>(dest_to_src(x, u.dst.x, u.src.x), dest_to_src(y, u.dst.y, u.src.y)) + off;
    let uv = (c + vec2<f32>(0.5, 0.5)) / vec2<f32>(f32(u.src.x), f32(u.src.y));
    return textureSampleLevel(src_tex, src_smp, uv, 0.0).rgb;
}
@compute @workgroup_size(8, 8)
fn down_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u.dst.x || y >= u.dst.y) {
        return;
    }
    let a = tap(x, y, vec2<f32>(-2.0, -2.0));
    let b = tap(x, y, vec2<f32>(0.0, -2.0));
    let c = tap(x, y, vec2<f32>(2.0, -2.0));
    let d = tap(x, y, vec2<f32>(-1.0, -1.0));
    let e = tap(x, y, vec2<f32>(1.0, -1.0));
    let f = tap(x, y, vec2<f32>(-2.0, 0.0));
    let g = tap(x, y, vec2<f32>(0.0, 0.0));
    let h = tap(x, y, vec2<f32>(2.0, 0.0));
    let i = tap(x, y, vec2<f32>(-1.0, 1.0));
    let j = tap(x, y, vec2<f32>(1.0, 1.0));
    let k = tap(x, y, vec2<f32>(-2.0, 2.0));
    let l = tap(x, y, vec2<f32>(0.0, 2.0));
    let m = tap(x, y, vec2<f32>(2.0, 2.0));
    let corners = (a + b + f + g) + (b + c + g + h) + (f + g + k + l) + (g + h + l + m);
    let centre = d + e + i + j;
    let value = corners * (1.0 / 32.0) + centre * (1.0 / 8.0);
    textureStore(dst_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(value, 1.0));
}
