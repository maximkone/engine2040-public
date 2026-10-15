// particles_hanabi.wgsl: one indirect billboard dispatch for N effects.
struct HanabiU { view_proj: mat4x4<f32>, right: vec3<f32>, _p0: f32, up: vec3<f32>, _p1: f32 };
struct HanabiP { pos: vec3<f32>, size: f32, age01: f32, effect: u32, tint_a: vec4<f32>, tint_b: vec4<f32> };
@group(0) @binding(0) var<uniform> hu: HanabiU;
@group(0) @binding(1) var<storage, read> particle: array<HanabiP>;
fn gradient(a: vec4<f32>, b: vec4<f32>, age01: f32) -> vec4<f32> { return mix(a, b, clamp(age01, 0.0, 1.0)); }
struct HOut { @builtin(position) clip: vec4<f32>, @location(0) col: vec4<f32> };
@vertex fn hanabi_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> HOut {
    let p = particle[ii]; var c = vec2<f32>(-0.5, -0.5);
    if (vi == 1u) { c = vec2<f32>(0.5, -0.5); } if (vi == 2u) { c = vec2<f32>(-0.5, 0.5); } if (vi == 3u) { c = vec2<f32>(0.5, 0.5); }
    let world = p.pos + hu.right * (c.x * p.size) + hu.up * (c.y * p.size);
    var out: HOut; out.clip = hu.view_proj * vec4<f32>(world, 1.0);
    out.col = gradient(p.tint_a, p.tint_b, p.age01); return out;
}
@fragment fn hanabi_fs(in: HOut) -> @location(0) vec4<f32> { return in.col; }
