
// The tone map is the shared chunk: `tonemap.wgsl` is prepended at compile
// time (`post/gpu.rs` `POST_CHAIN_WGSL`), so `aces_tonemap` is the one
// hue-keeping curve of the crate. The per-channel copy that used to live
// here shifted a torch flame's hue by +32.3 degrees.
@group(2) @binding(4) var post_sampler: sampler; // CQ23: the chunk samples bloom, so it names its sampler
@group(2) @binding(5) var post_bloom: texture_2d<f32>;
@group(2) @binding(6) var<uniform> post_chain: vec4<f32>;
@group(2) @binding(7) var<uniform> post_grade: vec4<f32>;
fn post_chain_apply(hdr: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let bloom = textureSampleLevel(post_bloom, post_sampler, uv, 0.0).rgb * post_chain.x;
    let exposed = (hdr + bloom) * post_chain.y;
    let toned = aces_tonemap(exposed, 1.0, post_chain.z);
    let mid = vec3<f32>(0.18);
    let con = (toned - mid) * post_grade.x + mid;
    let luma = dot(con, vec3<f32>(0.2126, 0.7152, 0.0722));
    return max(mix(vec3<f32>(luma), con, post_grade.y) * post_grade.z + post_grade.w, vec3<f32>(0.0));
}
