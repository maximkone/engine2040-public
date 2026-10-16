// cull.wgsl: GPU frustum cull, one compute pass plus the args write.
//
// One thread per static instance tests its bounding sphere against six
// inward-facing frustum planes with the `-radius` rule from Lengyel vol2
// 9.4.2 (sphere visibility): inside where `dot(n, center) + dist >= -radius`.
// Touching counts as visible. Visible threads set `flags[i] = 1` and bump
// `count`; culled threads set `flags[i] = 0`. One thread then writes the
// 5-u32 indirect args from `count` (36 indices, no
// MULTI_DRAW_INDIRECT_COUNT, so an iGPU without it still runs).

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
