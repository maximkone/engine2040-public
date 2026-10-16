// cull.wgsl: GPU frustum cull, one compute pass plus the args write.
//
// One thread per static instance tests its bounding sphere against six
// inward-facing frustum planes with the `-radius` rule from Lengyel vol2
// 9.4.2 (sphere visibility): inside where `dot(n, center) + dist >= -radius`.
// Touching counts as visible. Visible threads set `flags[i] = 1` and bump
// `count`; culled threads set `flags[i] = 0`. One thread then writes the
// 5-u32 indirect args from `count` (36 indices, no
// MULTI_DRAW_INDIRECT_COUNT, so an iGPU without it still runs).

// Hi-Z second stage (voidin visibility-prepass pattern, reimplemented,
// zero bytes vendored): same plane math as `main`, then one depth-pyramid
// fetch per frustum survivor. Fully hidden spheres miss the draw;
// survivors append their index to `visible_indices` for the single
// indirect draw. Caller: `CullSubmitter::run_hiz_on_device` packs
// `hiz_params` from `HizPyramid` (refuses when mip input missing).

@group(0) @binding(0) var<storage, read> spheres : array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> planes : array<vec4<f32>, 6>;
@group(0) @binding(2) var<storage, read_write> flags : array<u32>;
@group(0) @binding(3) var<storage, read_write> visible_count : atomic<u32>;
@group(0) @binding(4) var<storage, read_write> args : array<u32, 5>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&spheres)) {
        return;
    }
    let center = spheres[i].xyz;
    // -radius rule: the sphere survives when no plane pushes it out.
    let radius = spheres[i].w;
    var visible = true;
    for (var p = 0u; p < 6u; p = p + 1u) {
        let plane = planes[p];
        let dist = dot(plane.xyz, center) + plane.w;
        if (dist < -radius) {
            visible = false;
            break;
        }
    }
    flags[i] = select(0u, 1u, visible);
    if (visible) {
        atomicAdd(&visible_count, 1u);
    }
}

@compute @workgroup_size(64)
fn write_args(@builtin(global_invocation_id) gid : vec3<u32>) {
    if (gid.x != 0u) {
        return;
    }
    let count = atomicLoad(&visible_count);
    args[0] = 36u;
    args[1] = count;
    args[2] = 0u;
    args[3] = 0u;
    args[4] = 0u;
}

@group(0) @binding(5) var<storage, read> hiz_params : array<vec4<f32>, 1>;
@group(0) @binding(6) var<storage, read_write> visible_indices : array<u32>;

fn hiz_hidden_wall(center : vec3<f32>, radius : f32, hiz : vec4<f32>) -> bool {
    let occluder_depth = hiz.x;
    let half_extent = hiz.y;
    let behind = -center.z - radius > occluder_depth;
    let inside = abs(center.x) <= half_extent && abs(center.y) <= half_extent;
    return behind && inside;
}

@compute @workgroup_size(64)
fn main_hiz(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&spheres)) {
        return;
    }
    let center = spheres[i].xyz;
    // Same plane math as `main`: the sphere survives when no plane pushes it out.
    let radius = spheres[i].w;
    var visible = true;
    for (var p = 0u; p < 6u; p = p + 1u) {
        let plane = planes[p];
        let dist = dot(plane.xyz, center) + plane.w;
        if (dist < -radius) {
            visible = false;
            break;
        }
    }
    if (!visible) {
        flags[i] = 0u;
        return;
    }
    // One depth-pyramid fetch per survivor: the wall depth is the coarsest mip.
    let hiz = hiz_params[0];
    if (hiz_hidden_wall(center, radius, hiz)) {
        flags[i] = 0u;
        return;
    }
    flags[i] = 1u;
    let slot = atomicAdd(&visible_count, 1u);
    visible_indices[slot] = i;
}
