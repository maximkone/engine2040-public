// fluids_present.wgsl: droplet billboards over the seeded fluid pool.
// Engine-owned WGSL, fixed math only, no wall clock.
struct FluidU { view_proj: mat4x4<f32>, right: vec3<f32>, _p0: f32, up: vec3<f32>, _p1: f32 };
struct FluidP { pos: vec3<f32>, size: f32, vel: vec3<f32>, age01: f32, tint: vec4<f32> };
@group(0) @binding(0) var<uniform> fu: FluidU;
@group(0) @binding(1) var<storage, read> drops: array<FluidP>;
struct FOut { @builtin(position) clip: vec4<f32>, @location(0) fade: f32, @location(1) col: vec4<f32> };
@vertex
fn fluid_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FOut {
    let d = drops[ii];
    var c = vec2<f32>(-0.5, -0.5);
    if (vi == 1u) { c = vec2<f32>(0.5, -0.5); }
    if (vi == 2u) { c = vec2<f32>(-0.5, 0.5); }
    if (vi == 3u) { c = vec2<f32>(0.5, 0.5); }
    let world = d.pos + fu.right * (c.x * d.size) + fu.up * (c.y * d.size);
    var out: FOut;
    out.clip = fu.view_proj * vec4<f32>(world, 1.0);
    let age01 = clamp(d.age01, 0.0, 1.0);
    out.fade = 1.0 - age01;
    out.col = vec4<f32>(d.tint.rgb, d.tint.a * out.fade);
    return out;
}
@fragment
fn fluid_fs(in: FOut) -> @location(0) vec4<f32> { return in.col; }
