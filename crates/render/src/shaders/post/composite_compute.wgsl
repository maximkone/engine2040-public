
struct PostU {
    dims: vec4<u32>,
    mips0: vec4<u32>,
    mips1: vec4<u32>,
    mips2: vec4<u32>,
    mips3: vec4<u32>,
    mips4: vec4<u32>,
    mips5: vec4<u32>,
    bloom: vec4<f32>,
    grade0: vec4<f32>,
    grade1: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: PostU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> mips: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> dst: array<vec4<f32>>;
fn sample_level(base: u32, mw: u32, mh: u32, uu: f32, vv: f32) -> vec3<f32> {
    let x = clamp(uu, 0.0, f32(mw) - 1.0);
    let y = clamp(vv, 0.0, f32(mh) - 1.0);
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, mw - 1u);
    let y1 = min(y0 + 1u, mh - 1u);
    let tx = clamp(x - f32(x0), 0.0, 1.0);
    let ty = clamp(y - f32(y0), 0.0, 1.0);
    let a = mips[base + y0 * mw + x0].rgb;
    let b = mips[base + y0 * mw + x1].rgb;
    let c = mips[base + y1 * mw + x0].rgb;
    let d = mips[base + y1 * mw + x1].rgb;
    return mix(mix(a, b, tx), mix(c, d, tx), ty);
}
fn aces_fit(x: f32) -> f32 {
    let v = max(x, 0.0);
    return clamp((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14), 0.0, 1.0);
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let hdr = src[idx].rgb;
    let count = u.dims.z;
    var acc = vec3<f32>(0.0);
    if (count > 0u) {
        let mw = u.mips0.x;
        let mh = u.mips0.y;
        acc += sample_level(u.mips0.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    if (count > 1u) {
        let mw = u.mips1.x;
        let mh = u.mips1.y;
        acc += sample_level(u.mips1.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    if (count > 2u) {
        let mw = u.mips2.x;
        let mh = u.mips2.y;
        acc += sample_level(u.mips2.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    if (count > 3u) {
        let mw = u.mips3.x;
        let mh = u.mips3.y;
        acc += sample_level(u.mips3.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    if (count > 4u) {
        let mw = u.mips4.x;
        let mh = u.mips4.y;
        acc += sample_level(u.mips4.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    if (count > 5u) {
        let mw = u.mips5.x;
        let mh = u.mips5.y;
        acc += sample_level(u.mips5.z, mw, mh, (f32(gid.x) + 0.5) * f32(mw) / f32(w) - 0.5, (f32(gid.y) + 0.5) * f32(mh) / f32(h) - 0.5);
    }
    let per = select(u.bloom.x / f32(max(count, 1u)), 0.0, count == 0u);
    let exposed = (hdr + acc * per) * u.bloom.y;
    let white = max(aces_fit(u.bloom.z), 1e-4);
    let toned = vec3<f32>(aces_fit(exposed.x), aces_fit(exposed.y), aces_fit(exposed.z)) / white;
    let mid = vec3<f32>(0.18);
    let con = (toned - mid) * u.bloom.w + mid;
    let luma = dot(con, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sat = clamp(u.grade0.x, 0.0, 4.0);
    let graded = mix(vec3<f32>(luma), con, sat)
        * vec3<f32>(u.grade1.x, u.grade1.y, u.grade1.z)
        + u.grade0.yzw;
    dst[idx] = vec4<f32>(max(graded, vec3<f32>(0.0)), 1.0);
}
