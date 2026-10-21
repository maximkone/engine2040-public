// viewmodel: the first-person helmet rim.
//
// The rim is drawn in its own depth range on its own depth target: the
// world never writes into it, so a wall five centimetres in front of the
// eye cannot cover the rim. The shading is the scene light over the
// vertex color, with a soft highlight off the iron; the CPU mirror in
// `viewmodel.rs` runs the same law.

struct ViewmodelParams {
    // The view-projection of the pass: the eye at the origin looking +z.
    view_proj: mat4x4<f32>,
    // The sun direction (xyz) and its intensity (w).
    light_dir: vec4<f32>,
    // The sun color (rgb); w is unused.
    light_color: vec4<f32>,
    // The ambient color (rgb); w is unused.
    ambient: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: ViewmodelParams;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) @interpolate(flat) roughness: f32,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) roughness: f32,
) -> VsOut {
    var out: VsOut;
    out.clip = params.view_proj * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    out.roughness = roughness;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(params.light_dir.xyz);
    let lambert = max(dot(n, l), 0.0);
    let radiance = params.light_color.rgb * params.light_dir.w;
    let view = vec3<f32>(0.0, 0.0, 1.0);
    let half_vec = normalize(l + view);
    let exponent = mix(64.0, 4.0, in.roughness);
    let spec = pow(max(dot(n, half_vec), 0.0), exponent) * (1.0 - in.roughness) * 0.3;
    let lit = in.color * (params.ambient.rgb + radiance * lambert) + vec3<f32>(spec);
    return vec4<f32>(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
