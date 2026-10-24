// weather.wgsl: the half-res screen-space weather pass (SW142). Rain sheets,
// ripple rings pressed into the sand and the wet sheen, drawn as one
// screen-space refraction of the scene behind the soft layers.
//
// Owner: render. View only, never hashed. The CPU twin is
// `crates/render/src/weather.rs`: same hash, same constants, same ring, so the
// oracle's numbers hold here.
//
// Donor: godotshaders.com/shader/rain-drop-floor-effect (`hash12`, `hash22`,
// MAX_RADIUS 2, sin(31*d), offset_amount 0.05, effect_mix 0.55).

struct WeatherUniforms {
    // Frame size in pixels of this pass (half res, or a third on potato).
    resolution: vec2<f32>,
    // World xz at the pass's top-left corner, meters.
    frame_origin: vec2<f32>,
    // World meters per pass pixel.
    metres_per_px: f32,
    // Gauge 0..1: 0 leaves the frame byte-equal, 1 is full rain.
    gauge: f32,
    // Tick count: the drops are staggered on ticks, never on a wall clock.
    tick: f32,
    // 1.0 on the potato tier (the 7-tap kernel), 0.0 at half res (5x5).
    potato: f32,
};

@group(0) @binding(0) var<uniform> weather: WeatherUniforms;
@group(0) @binding(1) var scene_texture: texture_2d<f32>;
@group(0) @binding(2) var scene_sampler: sampler;

const RIPPLE_CELL_M: f32 = 0.5;
const RIPPLE_MAX_RADIUS: f32 = 2.0;
const RIPPLE_RING_HZ: f32 = 31.0;
const RIPPLE_OFFSET: f32 = 0.05;
const RIPPLE_EFFECT_MIX: f32 = 0.55;
const RIPPLE_TICKS: f32 = 120.0;
const RING_MIN_AGE: f32 = 0.02;
const WET_ALBEDO_SCALE: f32 = 0.75;
const WET_ROUGHNESS_DELTA: f32 = -0.3;

// Donor hash12: one cell in, one 0..1 word out.
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Donor hash22: the drop's own centre inside its cell.
fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// The ring's age in 0..1 for a cell at this tick.
fn drop_age(cell: vec2<f32>, tick: f32) -> f32 {
    let phase = RING_MIN_AGE + (1.0 - RING_MIN_AGE) * hash12(cell + vec2<f32>(0.5, 0.5) * 37.0);
    let cycle = fract(tick / RIPPLE_TICKS);
    return max(fract(cycle + phase), RING_MIN_AGE);
}

// The UV displacement one world point earns from its 5x5 ring neighbourhood.
// The potato tier walks 7 of the cells: the nearest 2x3 plus the centre.
fn ripple_offset(world: vec2<f32>) -> vec2<f32> {
    if (weather.gauge <= 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    let cell = floor(world / RIPPLE_CELL_M);
    var offset = vec2<f32>(0.0, 0.0);
    for (var dz = -2; dz <= 2; dz = dz + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            if (weather.potato > 0.5 && abs(dx) + abs(dz) > 2) {
                continue;
            }
            let here = cell + vec2<f32>(f32(dx), f32(dz));
            let age = drop_age(here, weather.tick);
            let radius = age * RIPPLE_MAX_RADIUS;
            let centre = (here + hash22(here)) * RIPPLE_CELL_M;
            let to_point = world - centre;
            let distance = length(to_point);
            if (distance >= radius || distance >= RIPPLE_MAX_RADIUS) {
                continue;
            }
            let ring = sin(RIPPLE_RING_HZ * distance) * (1.0 - age) * (1.0 - age);
            offset = offset + to_point * (RIPPLE_OFFSET * weather.gauge * RIPPLE_EFFECT_MIX * ring / (RIPPLE_CELL_M * 2.0));
        }
    }
    return offset;
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle: no vertex buffer, one draw.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, 1.0 - y);
    return out;
}

// Sample the scene through the ring's UV offset, then lay the wet sheen on.
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let world = weather.frame_origin + in.uv * weather.resolution * weather.metres_per_px;
    let offset = ripple_offset(world);
    let lens = clamp(weather.gauge, 0.0, 1.0);
    let sample_uv = clamp(in.uv + offset, vec2<f32>(0.0), vec2<f32>(1.0));
    let scene = textureSample(scene_texture, scene_sampler, sample_uv);
    // Wet sand darkens the ground albedo and loses roughness; the pass only
    // carries the colour half, the material half rides the tier knobs.
    let wet = mix(vec3<f32>(1.0), vec3<f32>(WET_ALBEDO_SCALE), lens);
    let sheen = vec3<f32>(WET_ROUGHNESS_DELTA, WET_ROUGHNESS_DELTA, WET_ROUGHNESS_DELTA);
    return vec4<f32>(scene.rgb * wet + sheen * lens * 0.5 * length(offset), scene.a);
}
