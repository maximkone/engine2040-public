// ssao.wgsl: screen-space ambient occlusion plus combine for the forward frame.
// Ambient occlusion scales the ambient term only, never the final color.
// Reimplemented shapes, zero donor bytes vendored.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/ssao2.fragment.fx): occlusion sample shape.
// PORT(BabylonJS/Babylon.js@136feacc96b011d98f26413edef252e098c846fd:packages/dev/core/src/ShadersWGSL/ssaoCombine.fragment.fx): combine shape.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/aoDiffuseOcc.js): diffuse occlusion shape.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/aoSpecOcc.js): specular occlusion shape.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/blurVSM.js): blur shape.
struct SsaoU {
    dims: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: SsaoU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
fn ao_scale_ambient(ambient: vec3<f32>, occlusion: f32) -> vec3<f32> {
    return ambient * clamp(occlusion, 0.0, 1.0);
}
fn vsm_blur3(a: f32, b: f32, c: f32) -> f32 {
    return (a + b * 2.0 + c) * 0.25;
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let depth = clamp(src[idx].w, 0.0, 1.0);
    let occlusion = vsm_blur3(depth, src[idx].x, depth);
    let ambient = src[idx].rgb;
    dst[idx] = vec4<f32>(ao_scale_ambient(ambient, occlusion), 1.0);
}
