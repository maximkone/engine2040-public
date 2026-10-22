/// TOON1: MMD Standard toon shading (MIT donor: togechiyo/MMD_modoki)
/// Shadow band = one flat colour from a toon texel (no filtering)
/// Boundary = selfMask * occlusionMask (hard painted light, no continuous ramp)
/// Donor: https://github.com/togechiyo/MMD_modoki/blob/main/src/scene/shaders/builtin-toon/full_shadow.wgsl

/// @group 0 @binding 2
var<uniform> toon_ramp: texture_2d<f32>;

/// @group 0 @binding 3
var<uniform> toon_sampler: sampler;

/// TOON1 BRDF: hard-banded shadow with MMD Standard boundary
/// Returns vec3: base colour * shadow factor + ambient
fn toon_brdf(
    base_color: vec3f,
    light_diffuse: vec3f,
    light_ambient: vec3f,
    self_mask: f32,
    occlusion_mask: f32,
    n_dot_l: f32,
) -> vec3f {
    // The MMD Standard toon rule: shadow band is one texel
    // Sample the toon ramp at clamped N·L with point sampling
    let toon_uv = vec2f(clamp(n_dot_l, 0.0, 1.0), 0.5);
    let toon_texel = textureSample(toon_ramp, toon_sampler, toon_uv).rgb;
    
    // The light/dark boundary is selfMask * occlusionMask
    let boundary = self_mask * occlusion_mask;
    
    // Shadow factor: if N·L > boundary, full light; else, toon texel
    let shadow_factor = select(toon_texel, vec3f(1.0), n_dot_l > boundary);
    
    // Final colour: base * shadow factor + ambient
    return base_color * shadow_factor + light_ambient;
}