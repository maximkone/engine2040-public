// fog_vol: analytic volumetric march plus one cloud slice (device twin of
// the CPU reference in the parent module). Linear HDR in, linear factor out;
// the blit tone-maps once. One file, two entry variants sharing the helpers.

struct VolU {
    color: vec3<f32>,
    density: f32,
    falloff: f32,
    base: f32,
    max_dist: f32,
    _pad0: f32,
    eye: vec3<f32>,
    steps: u32,
    sun_dir: vec3<f32>,
    sun_boost: f32,
    cloud_alt: f32,
    cloud_coverage: f32,
    cloud_density: f32,
    exposure: f32,
    size: vec2<u32>,
    _pad1: vec2<u32>,
};

@group(0) @binding(0) var<uniform> vol: VolU;
@group(0) @binding(1) var<storage, read> depth: array<f32>;
@group(0) @binding(2) var<storage, read> heights: array<f32>;
@group(0) @binding(3) var<storage, read> dirs: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> factor: array<f32>;
@group(0) @binding(5) var<storage, read> sky_dirs: array<vec4<f32>>;

fn vol_hash(x: u32, z: u32) -> f32 {
    var h = x * 0x85ebca6bu + z * 0xc2b2ae35u;
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 16u);
    return f32(0x3f800000u | (h >> 9u)) - 1.0;
}

fn vol_noise(x: f32, z: f32) -> f32 {
    let ix = floor(x);
    let iz = floor(z);
    let fx = x - ix;
    let fz = z - iz;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let a = vol_hash(bitcast<u32>(ix), bitcast<u32>(iz));
    let b = vol_hash(bitcast<u32>(ix + 1.0), bitcast<u32>(iz));
    let c = vol_hash(bitcast<u32>(ix), bitcast<u32>(iz + 1.0));
    let d = vol_hash(bitcast<u32>(ix + 1.0), bitcast<u32>(iz + 1.0));
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    return ab + (cd - ab) * sz;
}

fn vol_fbm(x: f32, z: f32) -> f32 {
    let base = vol_noise(x, z);
    let mid = vol_noise(x * 2.03 + 13.7, z * 2.03 + 7.1);
    let fine = vol_noise(x * 4.07 + 47.3, z * 4.07 + 29.7);
    return clamp(base * 0.55 + mid * 0.28 + fine * 0.17, 0.0, 1.0);
}

fn vol_cover(n: f32, coverage: f32) -> f32 {
    return clamp((n - (1.0 - coverage)) / max(coverage, 0.0001), 0.0, 1.0);
}

fn vol_slab(frag_h: f32, dist: f32) -> f32 {
    let e = clamp(-((frag_h - vol.base) / vol.falloff), -8.0, 8.0);
    let tau = min(vol.density * dist * exp(e), 80.0);
    return 1.0 - exp(-tau);
}

fn vol_glow(view_dir: vec3<f32>) -> f32 {
    let cos_t = clamp(dot(view_dir, normalize(vol.sun_dir)), -1.0, 1.0);
    let g = 0.76;
    let denom = max(pow(1.0 + g * g - 2.0 * g * cos_t, 1.5), 0.000001);
    let phase = 0.1193662 * (1.0 - g * g) * (1.0 + cos_t * cos_t) / ((2.0 + g * g) * denom);
    let flat_d = max(pow(1.0 + g * g, 1.5), 0.000001);
    let head_d = max(pow((1.0 - g) * (1.0 - g), 1.5), 0.000001);
    let p0 = 0.1193662 * (1.0 - g * g) / ((2.0 + g * g) * flat_d);
    let p1 = 0.1193662 * (1.0 - g * g) * 2.0 / ((2.0 + g * g) * head_d);
    return clamp((phase - p0) / max(p1 - p0, 0.000001), 0.0, 1.0);
}

fn vol_cloud_od(eye: vec3<f32>, dir: vec3<f32>, dist: f32) -> f32 {
    if (vol.cloud_coverage <= 0.0 || vol.cloud_density <= 0.0) { return 0.0; }
    if (abs(dir.y) <= 0.00001) { return 0.0; }
    let hit_t = (vol.cloud_alt - eye.y) / dir.y;
    if (hit_t < 0.0 || hit_t > dist) { return 0.0; }
    let n = vol_fbm((eye.x + dir.x * hit_t) / 60.0, (eye.z + dir.z * hit_t) / 60.0);
    return vol.cloud_density * vol_cover(n, vol.cloud_coverage);
}

@compute @workgroup_size(8, 8)
fn vol_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= vol.size.x || y >= vol.size.y) { return; }
    let i = y * vol.size.x + x;
    let raw = dirs[i].xyz;
    if (dot(raw, raw) < 0.000000000001) {
        factor[i] = 0.0;
        return;
    }
    let dir = normalize(raw);
    let dist = min(depth[i], vol.max_dist);
    let step_len = dist / f32(max(vol.steps, 1u));
    var od = 0.0;
    for (var s = 0u; s < vol.steps; s = s + 1u) {
        let t = (f32(s) + 0.5) * step_len;
        let hy = vol.eye.y + dir.y * t;
        let e = clamp(-((hy - vol.base) / vol.falloff), -8.0, 8.0);
        od = od + vol.density * step_len * exp(e);
    }
    let glow = vol_glow(dir);
    let tau = min((od + vol_cloud_od(vol.eye, dir, dist)) * (1.0 + vol.sun_boost * glow), 80.0);
    // Exposure rides the color, never the factor; the +0.0 keeps it live.
    factor[i] = 1.0 - exp(-tau) + vol.exposure * 0.0;
}

@compute @workgroup_size(8, 8)
fn vol_sky_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= vol.size.x || y >= vol.size.y) { return; }
    let i = y * vol.size.x + x;
    let raw = sky_dirs[i].xyz;
    if (dot(raw, raw) < 0.000000000001) {
        factor[i] = 0.0;
        return;
    }
    let dir = normalize(raw);
    let p = vol.eye + dir * vol.max_dist;
    factor[i] = vol_slab(p.y, vol.max_dist) + vol.exposure * 0.0;
}
