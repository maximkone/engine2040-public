// lut_present.wgsl: the last pass of the frame when a look table is bound
// (E11). It reads the graded display frame the blit wrote, decodes the
// display value back to the linear display scale `Lut3d::apply` works in,
// samples the table once per pixel, and writes the display frame again.
//
// Placement: after the blit's one tone map plus grade (and after the HUD and
// debug lines that ride the same view) and before the display write, so a
// look sits on graded display values exactly as the CPU `run_chain_look`
// does. A frame with no table never records this pass: the presenter keeps
// the blit's own bytes.
//
// Coordinates: the LUT fetch is the CPU sample, not a naive fetch. The grid
// is `color * (size - 1)`, the texture unit is `(grid + 0.5) / size`, and the
// upload is blue fastest, so the texture's x axis carries blue and its z axis
// red. `Lut3d::sample_texel` states the same blend on the CPU, and the
// device test in `lut_upload` compares the two within 2/255.
//
// Two fragment entries, like the blit: `fs` writes the sRGB bytes itself (a
// plain RGBA8 target, the LDR twin); `fs_hw_srgb` leaves the encode to a
// `*Srgb` surface view, which would otherwise encode twice and wash the
// frame out.
//
// Color transfer: the source texture holds byte-encoded display values (the
// blit's `linear_to_srgb_term`), so the pass decodes with the matching EOTF,
// applies the table on linear display values, and encodes back with the same
// 0.0031308 / 0.04045 pair. Textbook sRGB transfer (found2 5.3); no donor
// bytes.

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var lut_tex: texture_3d<f32>;
@group(0) @binding(2) var lut_smp: sampler;

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Same fullscreen triangle as the blit, same V flip: presented row 0 holds
// the source's top row, so the pass keeps the frame the right way up.
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> LOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: LOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = vec2<f32>(p[vi].x * 0.5 + 0.5, 0.5 - p[vi].y * 0.5);
    return out;
}

// sRGB EOTF: byte-encoded display value to linear display value.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = v / 12.92;
    let hi = pow((v + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, v > vec3<f32>(0.04045));
}

// The one trilinear table fetch: domain 0..=1 onto texel centers, blue
// fastest. `Lut3d::apply` is the CPU statement of this same blend.
fn lut_fetch(display: vec3<f32>) -> vec3<f32> {
    let dims = vec3<f32>(textureDimensions(lut_tex));
    let grid = clamp(display, vec3<f32>(0.0), vec3<f32>(1.0)) * (dims - vec3<f32>(1.0));
    let uv = (grid + vec3<f32>(0.5)) / dims;
    return textureSampleLevel(lut_tex, lut_smp, vec3<f32>(uv.b, uv.g, uv.r), 0.0).rgb;
}

// One display texel through the table, in linear display values.
fn looked(src: vec3<f32>) -> vec3<f32> {
    return lut_fetch(srgb_to_linear(src));
}

fn source_texel(in: LOut) -> vec4<f32> {
    let px = vec2<i32>(i32(in.pos.x), i32(in.pos.y));
    return textureLoad(src_tex, px, 0);
}

@fragment
fn fs(in: LOut) -> @location(0) vec4<f32> {
    let texel = source_texel(in);
    let seen = looked(texel.rgb);
    // The table may hold values above 1.0 (a linear-table cook); the byte
    // target clamps them, exactly as the display write would.
    let lo = seen * 12.92;
    let hi = 1.055 * pow(max(seen, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    let bytes = select(hi, lo, seen <= vec3<f32>(0.0031308));
    return vec4<f32>(clamp(bytes, vec3<f32>(0.0), vec3<f32>(1.0)), texel.a);
}

@fragment
fn fs_hw_srgb(in: LOut) -> @location(0) vec4<f32> {
    let texel = source_texel(in);
    return vec4<f32>(looked(texel.rgb), texel.a);
}
