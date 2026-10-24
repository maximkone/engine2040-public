// fsr1_upscale.wgsl: the spatial upscale of the live frame (AMD FSR 1.0 shape).
//
// Two fragment passes over a fullscreen triangle, both reading display-referred bytes the blit
// wrote (the tone map and grade already ran, so the filter works in a perceptual space, as the
// donor asks):
//   fs_easu   edge-adaptive upscale from the render size to the output size (12 taps, a gradient
//             direction from the four bilinear corners, an anisotropic kernel, a 2x2-style dering
//             clamp against the four nearest texels).
//   fs_rcas   contrast-adaptive sharpen at the output size: the 4-neighbour lobe the CPU
//             `fsr_rcas_sharpen` states, clamped to the neighbour min and max.
// `*_hw_srgb` entries are for a `*Srgb` target, which encodes on write: they decode the byte
// values to linear first, so the hardware encode returns the same bytes.
//
// `crates/render/src/post/fsr1.rs` is the CPU statement of fs_easu, tap for tap and in the same
// order of float operations; the device test in `upscale_gpu.rs` holds the two together.
//
// PORT(GPUOpen-Effects/FidelityFX-FSR@a21ffb8:ffx-fsr/ffx_fsr1.h FsrEasuF): MIT, rewritten for
// WGSL, whole texels read with textureLoad instead of per-channel gathers, no pasted bytes.

struct UpU {
    // xy: source texels per output pixel. zw: 0.5 * scale - 0.5, the donor's con0.
    con0: vec4<f32>,
    // x: RCAS sharpness, 0 (identity) to 1.
    params: vec4<f32>,
};
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> up: UpU;

struct VOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return out;
}

// The donor's integer-subtract reciprocal and reciprocal square root (wrapping u32 math).
fn rcp_lo(a: f32) -> f32 {
    return bitcast<f32>(0x7ef07ebbu - bitcast<u32>(a));
}
fn rsq_lo(a: f32) -> f32 {
    return bitcast<f32>(0x5f347d74u - (bitcast<u32>(a) >> 1u));
}
fn sat(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}
// Luma times two: b/2 + (r/2 + g).
fn luma2(c: vec3<f32>) -> f32 {
    return c.z * 0.5 + (c.x * 0.5 + c.y);
}
// One texel, clamp-to-edge.
fn texel(p: vec2<i32>) -> vec3<f32> {
    let d = vec2<i32>(textureDimensions(src_tex));
    return textureLoad(src_tex, clamp(p, vec2<i32>(0), d - vec2<i32>(1)), 0).rgb;
}

// One bilinear corner's gradient and edge length: (dir.x * w, dir.y * w, lenX * w, lenY * w).
fn easu_set(w: f32, la: f32, lb: f32, lc: f32, ld: f32, le: f32) -> vec4<f32> {
    let len_x = max(abs(ld - lc), abs(lc - lb));
    let dir_x = ld - lb;
    var lx = sat(abs(dir_x) * rcp_lo(len_x));
    lx = lx * lx;
    let len_y = max(abs(le - lc), abs(lc - la));
    let dir_y = le - la;
    var ly = sat(abs(dir_y) * rcp_lo(len_y));
    ly = ly * ly;
    return vec4<f32>(dir_x * w, dir_y * w, lx * w, ly * w);
}

// One tap's weight: rotate the offset into the edge frame, stretch it, then the lanczos2-like
// polynomial (a negative lobe whose size follows the window).
fn easu_weight(off: vec2<f32>, dir: vec2<f32>, len2: vec2<f32>, lob: f32, clp: f32) -> f32 {
    let vx = (off.x * dir.x + off.y * dir.y) * len2.x;
    let vy = (off.x * (-dir.y) + off.y * dir.x) * len2.y;
    let d2 = min(vx * vx + vy * vy, clp);
    var wb = 0.4 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb = wb * wb;
    wa = wa * wa;
    wb = 1.5625 * wb - 0.5625;
    return wb * wa;
}

// The upscaled colour of output pixel `ip`.
fn easu(ip: vec2<f32>) -> vec3<f32> {
    let pos = ip * up.con0.xy + up.con0.zw;
    let fp = floor(pos);
    let pp = pos - fp;
    let base = vec2<i32>(fp);
    //     b c
    //   e f g h
    //   i j k l
    //     n o
    let b = texel(base + vec2<i32>(0, -1));
    let c = texel(base + vec2<i32>(1, -1));
    let e = texel(base + vec2<i32>(-1, 0));
    let f = texel(base + vec2<i32>(0, 0));
    let g = texel(base + vec2<i32>(1, 0));
    let h = texel(base + vec2<i32>(2, 0));
    let i = texel(base + vec2<i32>(-1, 1));
    let j = texel(base + vec2<i32>(0, 1));
    let k = texel(base + vec2<i32>(1, 1));
    let l = texel(base + vec2<i32>(2, 1));
    let n = texel(base + vec2<i32>(0, 2));
    let o = texel(base + vec2<i32>(1, 2));
    let lbb = luma2(b);
    let lc = luma2(c);
    let le = luma2(e);
    let lf = luma2(f);
    let lg = luma2(g);
    let lh = luma2(h);
    let li = luma2(i);
    let lj = luma2(j);
    let lk = luma2(k);
    let ll = luma2(l);
    let ln = luma2(n);
    let lo_ = luma2(o);
    // Gradient direction and edge amount, weighted by the four bilinear corners.
    let s1 = easu_set((1.0 - pp.x) * (1.0 - pp.y), lbb, le, lf, lg, lj);
    let s2 = easu_set(pp.x * (1.0 - pp.y), lc, lf, lg, lh, lk);
    let s3 = easu_set((1.0 - pp.x) * pp.y, lf, li, lj, lk, ln);
    let s4 = easu_set(pp.x * pp.y, lg, lj, lk, ll, lo_);
    var dir = s1.xy + s2.xy + s3.xy + s4.xy;
    var len = s1.z + s1.w + s2.z + s2.w + s3.z + s3.w + s4.z + s4.w;
    // Normalize with the approximate rsqrt; a near-zero gradient points along x.
    var dir_r = dir.x * dir.x + dir.y * dir.y;
    let zro = dir_r < 0.000030517578125;
    dir_r = rsq_lo(dir_r);
    if (zro) {
        dir_r = 1.0;
        dir.x = 1.0;
    }
    dir = dir * dir_r;
    len = len * 0.5;
    len = len * len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) * rcp_lo(max(abs(dir.x), abs(dir.y)));
    let len2 = vec2<f32>(1.0 + (stretch - 1.0) * len, 1.0 + (-0.5) * len);
    let lob = 0.5 + (-0.29) * len;
    let clp = rcp_lo(lob);
    // Weighted sum over the twelve taps, in the CPU's order.
    var wt = easu_weight(vec2<f32>(0.0, -1.0) - pp, dir, len2, lob, clp);
    var acc = b * wt;
    var acc_w = wt;
    wt = easu_weight(vec2<f32>(1.0, -1.0) - pp, dir, len2, lob, clp);
    acc = acc + c * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(-1.0, 0.0) - pp, dir, len2, lob, clp);
    acc = acc + e * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(0.0, 0.0) - pp, dir, len2, lob, clp);
    acc = acc + f * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(1.0, 0.0) - pp, dir, len2, lob, clp);
    acc = acc + g * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(2.0, 0.0) - pp, dir, len2, lob, clp);
    acc = acc + h * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(-1.0, 1.0) - pp, dir, len2, lob, clp);
    acc = acc + i * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(0.0, 1.0) - pp, dir, len2, lob, clp);
    acc = acc + j * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(1.0, 1.0) - pp, dir, len2, lob, clp);
    acc = acc + k * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(2.0, 1.0) - pp, dir, len2, lob, clp);
    acc = acc + l * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(0.0, 2.0) - pp, dir, len2, lob, clp);
    acc = acc + n * wt;
    acc_w = acc_w + wt;
    wt = easu_weight(vec2<f32>(1.0, 2.0) - pp, dir, len2, lob, clp);
    acc = acc + o * wt;
    acc_w = acc_w + wt;
    // Dering against the four nearest texels f g j k.
    let near_lo = min(min(min(f, g), j), k);
    let near_hi = max(max(max(f, g), j), k);
    let inv = 1.0 / acc_w;
    return min(near_hi, max(near_lo, acc * inv));
}

// The sharpened colour of output pixel `ip`: the unsharp lobe against the four neighbours,
// held inside their min and max so no halo escapes the local range.
fn rcas(ip: vec2<f32>) -> vec3<f32> {
    let p = vec2<i32>(ip);
    let e = texel(p);
    let b = texel(p + vec2<i32>(0, -1));
    let d = texel(p + vec2<i32>(-1, 0));
    let f = texel(p + vec2<i32>(1, 0));
    let h = texel(p + vec2<i32>(0, 1));
    let mn = min(min(min(b, d), f), h);
    let mx = max(max(max(b, d), f), h);
    let blur = (b + d + f + h) * 0.25;
    return clamp(e + (e - blur) * up.params.x, mn, mx);
}

// sRGB EOTF for a `*Srgb` target, which encodes on write.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = v / 12.92;
    let hi = pow((v + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, v > vec3<f32>(0.04045));
}

@fragment
fn fs_easu(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(easu(floor(in.pos.xy)), 1.0);
}

@fragment
fn fs_easu_hw_srgb(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(easu(floor(in.pos.xy))), 1.0);
}

@fragment
fn fs_rcas(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(rcas(floor(in.pos.xy)), 1.0);
}

@fragment
fn fs_rcas_hw_srgb(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(rcas(floor(in.pos.xy))), 1.0);
}
