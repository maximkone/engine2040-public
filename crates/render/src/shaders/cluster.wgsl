// cluster.wgsl: tile/cluster assignment compute for the forward torch loop.
// Reimplemented shapes, zero donor bytes vendored.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/clusteredLight.js): cluster light loop shape.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/clusteredLightUtils.js): tile/slice index shape.
// PORT(playcanvas/engine@5531b4c0afaf39cba7e1701d300cb6c27f6bad30:src/scene/shader-lib/wgsl/chunks/lit/frag/clusteredLightShadows.js): clustered shadow read shape.
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
@group(0) @binding(1) var<storage, read> light_cells: array<u32>;
@group(0) @binding(2) var<storage, read_write> cell_counts: array<u32>;
fn cluster_cell_of(tile_x: u32, tile_y: u32, slice: u32) -> u32 {
    return (slice * cluster.tiles_y + tile_y) * cluster.tiles_x + tile_x;
}
@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell = gid.x;
    if (cell >= cluster.cells) {
        return;
    }
    cell_counts[cell] = light_cells[cell % 8u];
}
