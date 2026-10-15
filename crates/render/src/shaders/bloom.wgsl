// bloom.wgsl: the live render bloom chain between forward and the blit's
// one tonemap. One file, entry-point variants over a fullscreen triangle:
// `fs_extract` (soft-knee bright extract), `fs_blur` (one 2x2 box
// downsample step) and `fs_composite` (add-back of one blurred mip over the
// HDR base). Linear HDR in and out; this file darkens nothing and encodes
// nothing. Math mirrors `post.rs`: the soft knee matches `bloom_weight`
// (nothing below threshold minus knee, all above threshold plus knee,
// quadratic between), the blur matches `downsample_box` (ceil halves, edge
// texels average the taps inside), and the composite adds the even
// intensity share the `bloom_render_uniform` packs (the presenter records
// one composite per sampled mip so the sum equals the CPU add-back).
// Row convention matches `blit.wgsl`: clip +y is up while texture row 0 is
// the top row, so the vertex stage flips V.
struct BloomU {
    // x: knee center in linear HDR; y: soft-knee width; z: even intensity
    // share for this composite; w: sampled mip count (evidence only).
    params: vec4<f32>,
    // x/y: source size, z/w: destination size, all in pixels.
    dims: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: BloomU;
@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var bloom_smp: sampler;
struct BOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> BOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: BOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = vec2<f32>(p[vi].x * 0.5 + 0.5, 0.5 - p[vi].y * 0.5);
    return out;
}
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
@fragment
fn fs_extract(in: BOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let hdr = textureLoad(hdr_tex, px, 0);
    let luma = dot(hdr.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(hdr.rgb * bloom_w(luma), 1.0);
}
@fragment
fn fs_blur(in: BOut) -> @location(0) vec4<f32> {
    let src = vec2<i32>(u.dims.xy);
    let base = vec2<i32>(in.pos.xy) * vec2<i32>(2, 2);
    var acc = vec3<f32>(0.0);
    var n = 0u;
    for (var dy = 0; dy < 2; dy++) {
        for (var dx = 0; dx < 2; dx++) {
            let s = base + vec2<i32>(dx, dy);
            if (s.x < src.x && s.y < src.y) {
                acc += textureLoad(hdr_tex, s, 0).rgb;
                n += 1u;
            }
        }
    }
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(acc / vec3<f32>(f32(n)), 1.0);
}
@fragment
fn fs_composite(in: BOut) -> @location(0) vec4<f32> {
    let hdr = textureLoad(hdr_tex, vec2<i32>(in.pos.xy), 0);
    let bloom = textureSampleLevel(bloom_tex, bloom_smp, in.uv, 0.0).rgb * u.params.z;
    return vec4<f32>(hdr.rgb + bloom, hdr.a);
}
