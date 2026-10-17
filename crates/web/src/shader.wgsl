// One square, six vertices, no vertex buffer. `Square::to_bytes` in anim.rs writes the block.
struct Square {
    centre: vec2<f32>,
    half: vec2<f32>,
    rgba: vec4<f32>,
};

@group(0) @binding(0) var<uniform> square: Square;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[i];
    return vec4<f32>(square.centre + c * square.half, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return square.rgba;
}
