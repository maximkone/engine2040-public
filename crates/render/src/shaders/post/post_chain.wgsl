
@group(2) @binding(5) var post_bloom: texture_2d<f32>;
@group(2) @binding(6) var<uniform> post_chain: vec4<f32>;
@group(2) @binding(7) var<uniform> post_grade: vec4<f32>;
fn aces_fitted(x: f32) -> f32 {
    let v = max(x, 0.0);
    return clamp((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14), 0.0, 1.0);
}
fn post_chain_apply(hdr: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let bloom = textureSampleLevel(post_bloom, post_sampler, uv, 0.0).rgb * post_chain.x;
    let exposed = (hdr + bloom) * post_chain.y;
    let w = max(aces_fitted(post_chain.z), 1e-4);
    let toned = vec3<f32>(aces_fitted(exposed.x), aces_fitted(exposed.y), aces_fitted(exposed.z)) / w;
    let mid = vec3<f32>(0.18);
    let con = (toned - mid) * post_grade.x + mid;
    let luma = dot(con, vec3<f32>(0.2126, 0.7152, 0.0722));
    return max(mix(vec3<f32>(luma), con, post_grade.y) * post_grade.z + post_grade.w, vec3<f32>(0.0));
}
