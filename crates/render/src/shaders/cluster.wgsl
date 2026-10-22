// cluster.wgsl: the per-cell torch light masks the forward torch loop walks.
//
// One thread per cluster cell counts that cell's lit bits, so the fragment
// loop visits only the lights that reach its cell instead of every torch in
// the frame. The assignment rule (which light reaches which cell) lives once,
// on the CPU in `shadow_cluster::torch_cluster`, and
// `shadow_cluster::cluster_mask` packs it into the mask words this pass reads:
// the shader never re-derives a cell, so the two cannot drift.
//
// A mask word holds 32 lights, and a frame needs `ceil(max_lights / 32)` words
// per cell, in light order: word `g` bit `b` is light `g * 32 + b`. Eight
// words cover `MAX_TORCH_LIGHTS` (256). `LIGHT_GROUPS` is a pipeline override,
// not a default to trust: a group count that disagrees with the CPU table must
// fail at pipeline build, not shade the wrong cell.
//
// Reimplemented shapes, zero donor bytes vendored.
// PORT(google/filament@479ee3548410f21c038e50f1fe9335fe09eb433b:filament/src/Froxelizer.cpp): a light-group bitmask per froxel record, read back by the fragment instead of testing every light per pixel.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/clusteredLight.js): tile/slice index shape.

/// Words per cell. Must equal `shadow_cluster::cluster_mask::mask_groups`
/// of the frame's `max_lights`; supplied at pipeline build.
override LIGHT_GROUPS: u32 = 8u;

struct ClusterU {
    tiles_x: u32,
    tiles_y: u32,
    slices: u32,
    tile_px: u32,
    cells: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> cluster: ClusterU;
/// `cells * LIGHT_GROUPS` words, cell-major: word `cell * LIGHT_GROUPS + g`,
/// bit `b` set when light `g * 32 + b` reaches the cell.
@group(0) @binding(1) var<storage, read> cell_masks: array<u32>;
/// Lights per cell, written by `resolve`.
@group(0) @binding(2) var<storage, read_write> cell_counts: array<u32>;
/// Receipt words: `[0]` lit cells, `[1]` most lights in one cell.
@group(0) @binding(3) var<storage, read_write> cluster_stats: array<atomic<u32>>;

/// Cell index for tile `(tile_x, tile_y)` and depth slice `slice`,
/// tile-major, matching `ClusterPass::cell_index` on the CPU.
fn cell_of(tile_x: u32, tile_y: u32, slice: u32) -> u32 {
    return (slice * cluster.tiles_y + tile_y) * cluster.tiles_x + tile_x;
}

/// Mask word `group` of `cell`.
fn mask_word(cell: u32, group: u32) -> u32 {
    return cell_masks[cell * LIGHT_GROUPS + group];
}

/// True when light `light` reaches `cell`. Out-of-range lights never light.
fn light_lit(cell: u32, light: u32) -> bool {
    let group = light / 32u;
    if (group >= LIGHT_GROUPS) {
        return false;
    }
    return (mask_word(cell, group) & (1u << (light % 32u))) != 0u;
}

/// Lights that reach `cell`, counted across the groups.
fn lights_in(cell: u32) -> u32 {
    var total = 0u;
    for (var group = 0u; group < LIGHT_GROUPS; group = group + 1u) {
        total = total + countOneBits(mask_word(cell, group));
    }
    return total;
}

@compute @workgroup_size(64, 1, 1)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell = gid.x;
    if (cell >= cluster.cells) {
        return;
    }
    let count = lights_in(cell);
    cell_counts[cell] = count;
    if (count > 0u) {
        atomicAdd(&cluster_stats[0], 1u);
    }
    atomicMax(&cluster_stats[1], count);
}
