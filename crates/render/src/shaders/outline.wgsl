/// TOON1: Inverted hull outline with constant pixel width (MIT donor: togechiyo/MMD_modoki)
/// Outline = view normal / viewport × clip.w (constant width in pixels)
/// Donor: https://github.com/togechiyo/MMD_modoki/blob/main/src/scene/shaders/builtin-toon/full_shadow.wgsl

@group(0) @binding(4)
var<uniform> outline_color: vec4f;

@group(0) @binding(5)
var<uniform> outline_width: f32;

/// Inverted hull outline: constant pixel width at any distance
/// Uses view normal divided by viewport, times clip.w
fn outline_fragment(
    position: vec4f,
    view_normal: vec3f,
    viewport_size: vec2f,
    clip_w: f32,
) -> vec4f {
    // Constant pixel width: view normal / viewport × clip.w
    let screen_space_normal = abs(view_normal.xy) / viewport_size * clip_w;
    
    // Outline thickness based on screen-space normal
    let outline_thickness = max(screen_space_normal.x, screen_space_normal.y);
    
    // Apply outline if thickness exceeds threshold
    let outline_factor = step(outline_width, outline_thickness);
    
    // Return outline color or transparent
    return outline_color * outline_factor;
}