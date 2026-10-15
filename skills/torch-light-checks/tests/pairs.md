# Skill-on vs skill-off pairs: torch-light-checks
Three torch reviews. Off is general habit; On names the checklist item and the proof. Pass = On cites the item, the failure, and the test.
## P1: cutoff-free falloff lights the world (items 2, 3, 4)
Input: `TorchLight` in `crates/render/src/shadow_cluster/torch_cluster.rs` uses pure inverse square with no `range_m` cutoff, so a gate torch lights the whole arena and pixels near center blow up; `intensity` also grows `range_m`.
With skill: Item 2: each light carries `range_m` as cutoff so outside geos skip. Item 3: forward shade in `crates/render/src/shaders/forward.wgsl` eases to zero at cutoff via `LIGHT_UNIFORM_BYTES` in `crates/render/src/v1_gpu.rs`, capped near center. Item 4: brightness changes without touching reach. Evidence: masked on-vs-off differs, far field unchanged, no NaN.
Without skill misses: the world-wide cost and the center blow-up; approves because one close-up looks bright.
## P2: hash-order tiles drop torches quietly (items 1, 5, 6)
Input: assignment in `torch_cluster.rs` walks a hash map, keeps non-finite pos, and fails the frame when torches pass `MAX_TORCH_LIGHTS` 256 in `crates/render/src/shadow_cluster.rs`; `ClusterConfig` with 0 width passes.
With skill: Item 1: `TorchLight::validate` refuses bad input with `LightError::Invalid`. Item 5: walk cell-major sorted with `CLUSTER_TILE_PX` 16, clamp to `max_per_cluster` with overflow counted. Item 6: over 256 clamps with `BudgetExceeded` plus receipt, x2 equal. Evidence: 256-light drain stable twice, overflow reported.
Without skill misses: the order drift and the hard fail; calls the hash walk faster and misses replay divergence.
## P3: ambient mixed, tangents bad, bytes in hash (items 7, 8, 9)
Input: torches carry the base light, AO darkens direct in `forward.wgsl`, tangents from `crates/assets/src/mesh_import.rs` have `w` 0, and torch bytes fold into `state_hash` moving `d57b5c8ae3096614`.
With skill: Item 7: `sh_uniform_bytes_into` in `crates/render/src/ibl.rs` packs 144-byte SH base, AO scales ambient only. Item 8: `TangentMode` in `crates/assets/src/import_repair.rs` via `orthogonalize_tangent` plus `validate_mesh` in `crates/assets/src/cooked.rs` demands `w` 1 or -1. Item 9: torch stays presentation-only per `docs/HASH-LAW.md`, golden unchanged x2.
Without skill misses: the AO-on-direct error, the sliding highlight, and the hash move; ships because the frame looks warm.
