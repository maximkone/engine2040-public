// LF5c: the live temporal resolve. Reads this frame's HDR, reprojects last
// frame's resolved HDR by the motion target, clamps the history to the 3x3
// YCoCg neighbourhood of the current texel, then blends ten percent current.
// The motion target is uv units, y down, written un-jittered; the camera
// jitter stays inside the clamp box (under half a pixel).

struct TaaU {
    // Target size in pixels: the current frame's own size.
    size: vec2<u32>,
    // History weight (0.9: ten percent of the current frame).
    blend: f32,
    // Velocity reject, pixels: faster history follows the current frame.
    threshold: f32,
    // 1.0 when the history holds a resolved frame, 0.0 after a cut or reset.
    valid: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};

@group(0) @binding(0) var current_tex: texture_2d<f32>;
@group(0) @binding(1) var history_tex: texture_2d<f32>;
@group(0) @binding(2) var history_sampler: sampler;
@group(0) @binding(3) var motion_tex: texture_2d<f32>;
@group(0) @binding(4) var motion_sampler: sampler;
@group(0) @binding(5) var<uniform> frame: TaaU;

struct VsOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VsOut {
    // One triangle larger than the target: no vertex buffer, no edge seam.
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return VsOut(vec4<f32>(x, y, 0.0, 1.0));
}

fn to_ycocg(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * rgb.r + 0.5 * rgb.g + 0.25 * rgb.b,
        0.5 * rgb.r - 0.5 * rgb.b,
        -0.25 * rgb.r + 0.5 * rgb.g - 0.25 * rgb.b,
    );
}

fn from_ycocg(ycocg: vec3<f32>) -> vec3<f32> {
    let y = ycocg.x;
    let co = ycocg.y;
    let cg = ycocg.z;
    return vec3<f32>(y + co - cg, y + cg, y - co - cg);
}

@fragment
fn fs(input: VsOut) -> @location(0) vec4<f32> {
    // `position` is the y-down pixel center; nearest loads keep the current
    // frame exact and the motion sample on its own texel.
    let size = vec2<i32>(frame.size);
    let pixel = vec2<i32>(floor(input.position.xy));
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(frame.size);
    let current = textureLoad(current_tex, pixel, 0);
    var low = vec3<f32>(1.0e30);
    var high = vec3<f32>(-1.0e30);
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            let at = clamp(pixel + vec2<i32>(dx, dy), vec2<i32>(0), size - vec2<i32>(1));
            let value = to_ycocg(textureLoad(current_tex, at, 0).rgb);
            low = min(low, value);
            high = max(high, value);
        }
    }
    let motion = textureSampleLevel(motion_tex, motion_sampler, uv, 0.0).rg;
    let moved_px = length(motion * vec2<f32>(frame.size));
    let history = textureSampleLevel(history_tex, history_sampler, uv - motion, 0.0);
    let clamped = from_ycocg(clamp(to_ycocg(history.rgb), low, high));
    // A cut, a resize or a rejected velocity follows the current frame.
    let weight = select(frame.blend, 0.0, moved_px > frame.threshold) * frame.valid;
    return vec4<f32>(mix(current.rgb, clamped, weight), current.a);
}
