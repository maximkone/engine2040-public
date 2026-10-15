
@group(2) @binding(0) var post_current: texture_2d<f32>;
@group(2) @binding(1) var post_history: texture_2d<f32>;
@group(2) @binding(2) var post_motion: texture_2d<f32>;
@group(2) @binding(3) var post_sampler: sampler;
@group(2) @binding(4) var<uniform> post_taa: vec4<f32>;
fn taa_resolve(uv: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let px = (uv - 0.5 / size) * size;
    let mv = textureSampleLevel(post_motion, post_sampler, uv, 0.0).xy;
    let huv = (px - mv + 0.5 / size) / size;
    let hist = textureSampleLevel(post_history, post_sampler, huv, 0.0).rgb;
    let cur = textureSampleLevel(post_current, post_sampler, uv, 0.0).rgb;
    var lo = cur;
    var hi = cur;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let t = textureSampleLevel(post_current, post_sampler, uv + vec2<f32>(dx, dy) / size, 0.0).rgb;
            lo = min(lo, t);
            hi = max(hi, t);
        }
    }
    let clamped = clamp(hist, lo, hi);
    let speed = length(mv);
    let blend = select(post_taa.x, 0.0, speed > post_taa.y);
    return mix(cur, clamped, blend);
}
