STEAL CARD: dust-engine/dust (BLOCK 10, position 50, GitHub topic game-engine?l=rust&o=desc&s=updated). Read live 2026-10-18, read-only. No bytes vendored.
Donor head read: d00dc6a9b62197e43eb1ccf0cc65ab9e187f3432 (branch main).

What it is: a Rust research engine doing hardware ray tracing of sparse voxel scenes, built on Bevy ECS and Vulkan ray tracing (pumicite; Bevy patched to a dust fork). Crate map: vdb (hierarchical VDB-style voxel spatial index with bit-packed nodes and pooling), vox (voxel geometry/materials/palettes, .vox loading, hit-group shaders), pbr (ray-tracing renderer: pipeline, SBT, camera, sky atmosphere, HZB, screen trace, tonemap, upscaler), plus app, character, devtools, physics, profiler. Bazel + Slang shaders. 132 stars. Nightly Rust; needs VK_KHR_ray_tracing_pipeline.

Licence: MPL-2.0 (read live 2026-10-18: LICENSE.txt = "Mozilla Public License Version 2.0"; GitHub-detected SPDX: MPL-2.0). File-level copyleft: port only into our own files, keep the notice if a file is copied.

Engine2040 anchors it lands on:
- crates/render (culling/VRS/upscale): render-cull-vram (frustum/light/shadow/portal/occlusion culling), crates/render/src/upscale_easu.rs:1-23 (EASU 720p->1080p + RCAS finish, already shipped).
- crates/render/src/uploads.rs + the staging belt (integer pool math, no per-frame alloc).
- crates/assets (cooked content) and crates/spatial (index).
- engine-core (integer-only owners).

=== STEALS (ranked by value per hour) ===

1. RB closest-depth hierarchical Z buffer, rebuilt every frame from the depth G-buffer | dust-engine/dust@d00dc6a:crates/pbr/src/hzb.rs:1-2 (one compute dispatch for the whole mip chain, hzb_build.slang), :21-25 (MAX_MIPS 12, TILE_SIZE 64), :48-53 (mip-0 extent = half the power-of-two-rounded render extent), :97-106 (num_mips = 32 - leading_zeros, asserted <= MAX_MIPS) | MPL-2.0 | RENDER (occlusion culling) | lands crates/render (new occlusion pass) | M
   Idea: keep only the closest depth per tile, then a full mip pyramid where each level stores the max depth of its children; a bounding box is culled when its nearest depth is behind the level's stored depth. Built in one dispatch per frame (10-12 levels, 64-texel mip-0 tiles), so it tracks any camera.
   Maps: engine2040 culls by frustum/light/shadow. Steal an HZB to cull whole cooked mesh/prop bounds against the previous frame's depth before drawing; the integer extent math (next_power_of_two/2, mip count from leading_zeros) is a clean, cast-free sizing rule that fits render-cull-vram.
   Kill rule: kill if the on-vs-off pixel diff is empty on the arena scene (a cull that removes nothing to save), or if the build cost exceeds the draw cost it saves at 1080p.

2. Pool with a u32 freelist and refcounted shared subtrees | dust-engine/dust@d00dc6a:crates/vdb/src/pool.rs:90-116 (layout, freelist head, top, count, capacity, `refcounts: IntMap<u32,u32>` — a slot absent from the map has exactly one parent edge) and :142-160 (LIFO recycling; doc example frees 1,2 then allocs 2,1) | MPL-2.0 | RENDER/ASSETS (pool math) | lands crates/render/src/uploads.rs + crates/assets pools | M
   Idea: fixed-layout slots in one buffer, u32 handles, a LIFO freelist for O(1) alloc/free, and a refcount map only for slots referenced by more than one parent so shared subtrees (a repeated mesh/instance) are freed once. Allocation/free never touch the map, so the hot path is integer-only.
   Maps: engine2040 pools instance/mesh/uniform slots behind handles. Steal the "refcount only shared slots" rule and the u32 freelist to keep budget math integer and allocation-free on the hot path (render-cull-vram: integer pool math, bake bounds once).
   Kill rule: kill if a shared slot can be mutated by one parent (then refcount sharing is wrong); or if the freelist makes a VRAM budget non-deterministic across runs.

3. FrozenStorage capture: pin one backing allocation for readers while the live pool keeps growing/moving | dust-engine/dust@d00dc6a:crates/vdb/src/pool.rs:4-13 (capture returns a handle that keeps the *current* allocation alive independently of future resize) and :44-56 (OwnedBuffer: holds ptr+layout, deallocs on drop; the old allocation lives until the capture drops) | MPL-2.0 | RENDER (staging/readback) | lands crates/render staging belt | S-M
   Idea: resizing a pool copies into a new allocation and drops the old one, unless a capture pinned it; the capture is read-only and refuses to grow (pool.rs:29-42). So a GPU read or a snapshot can read the bytes it started with while the live pool reallocates underneath.
   Maps: engine2040 readbacks and uploads must not race a growing pool. Steal the explicit "pin the allocation, not the pool" handle for readback/staging so a read sees a fixed buffer while the owner keeps appending.
   Kill rule: kill if it needs a lock on the append path (then a double-buffer ring is cheaper); kill if any capture is held across a sim tick.

4. Bit-packed leaf occupancy at a fixed 4x4x4 so a leaf mask is exactly 64 bits, over a const-generic tree shape | dust-engine/dust@d00dc6a:crates/vdb/src/node/leaf.rs:13-26 (4x4x4 = 64 voxels, occupancy [usize; ...] + one value handle; "Size: 3 u32") and crates/vdb/src/lib.rs:25-40 (ConstUVec3 as a const generic param; glam conversion in a const fn) | MPL-2.0 | ASSETS/RENDER (voxel data) | lands crates/assets cooked content + crates/spatial | L
   Idea: choose the leaf size so its occupancy mask is exactly one machine word (64 bits), keep the shape in the type (const generics) so the compiler sizes every node, and treat occupancy as a bitslice (Lsb0) with one value per set bit.
   Maps: engine2040 content is cooked to fixed integer tables. Steal the "size the leaf so the mask is one word" rule and const-generic shape for any voxel/block data or a broad-phase occupancy grid, so iteration is `count_ones`/`IterOnes` and storage is declared, not computed.
   Kill rule: kill if the arena's content is not a regular grid (a leaf mask is wrong), or if it needs the nightly const-generic features dust pins (engine2040 builds on stable-ish Rust).

=== NOT WORTH STEALING (already have, or wrong shape) ===
- FSR EASU/RCAS upscaling (crates/pbr/src/upscaler/mod.rs:1-10 and crates/pbr/src/upscaler/{dlss,metalfx}.rs): engine2040 already ships crates/render/src/upscale_easu.rs (EASU 720->1080 + RCAS). The only new bit is the "pick one backend once at startup, plain blit fallback" enum (mod.rs:57-66).
- Hardware ray tracing, Slang shaders and Bazel: engine2040 is wgpu/WGSL with a cargo tree; not portable.

DONE 4 steals, 7 donor files read.
