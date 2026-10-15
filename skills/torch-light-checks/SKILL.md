---
name: torch-light-checks
description: Use when a builder wires torch point lights in crates/render and a reviewer checks the forward shade before it lands.
---

# Torch light checks

Live torches light the forward frame, never the tick. Nine checks. Each names the file to open. PASS needs capped assignment in order plus on-vs-off pixels, never a hash move.

1. **Finite inputs.** Ask: does `TorchLight::validate` in `crates/render/src/shadow_cluster/torch_cluster.rs` refuse non-finite pos, non-positive `range_m`, negative color or intensity with `LightError::Invalid`? Flag: bad light reaching assignment. (render)
2. **Known sphere.** Ask: does each light carry `range_m` as its cutoff so outside geos skip shade, with `ClusterConfig::validate` in the same file bounding `width_px`, `height_px` and `depth_slices`? Flag: world-wide light with no cutoff. (render)
3. **Falloff to zero.** Ask: does the forward shade in `crates/render/src/shaders/forward.wgsl` scale by one falloff with zero at cutoff, clamped past edge and capped near center by a size, wired via `LIGHT_UNIFORM_BYTES` in `crates/render/src/v1_gpu.rs`? Flag: endless tail or blow-up near center. (render)
4. **Brightness apart from reach.** Ask: does `intensity` change output without touching `range_m` in `TorchLight`, so a brighter torch needs no bigger sphere? Flag: reach growing with brightness. (render)
5. **Tile order stable.** Ask: does assignment in `crates/render/src/shadow_cluster/torch_cluster.rs` walk cell-major sorted with `CLUSTER_TILE_PX` 16 in `crates/render/src/shadow_cluster.rs`, clamping to `max_per_cluster` with overflow counted? Flag: hash-order walk or silent drop. (render)
6. **Budget degrades.** Ask: does over `MAX_TORCH_LIGHTS` 256 in `crates/render/src/shadow_cluster.rs` clamp with `LightError::BudgetExceeded` and a receipt, while bad input still fails? Flag: fail on many torches or quiet clamp. (render)
7. **Ambient base apart.** Ask: does `sh_uniform_bytes_into` in `crates/render/src/ibl.rs` pack 9 coeffs into `FORWARD_IBL_SH_UNIFORM_BYTES` 144, with AO scaling ambient only in `forward.wgsl`? Flag: torches carrying the base or AO darkening direct. (render)
8. **Tangents handed.** Ask: does `TangentMode` in `crates/assets/src/import_repair.rs` keep or recompute via `orthogonalize_tangent`, with `fallback_tangents` in `crates/assets/src/mesh_import.rs` and `validate_mesh` in `crates/assets/src/cooked.rs` demanding finite `w` of 1 or -1? Flag: torch sliding on bad frames. (assets)
9. **Never in hash.** Ask: do torch bytes stay out of `state_hash` per `docs/HASH-LAW.md` like `shadow_cluster.rs` keeps wall clock in receipts only, with golden `d57b5c8ae3096614` unchanged x2? Flag: light bytes in tick or replay. (render)

## Do / Don't
- Do: bound each torch by range, ease falloff to zero, assign per tile in fixed order, keep ambient and tangents clean, and prove on-vs-off pixels plus hash unchanged.
- Don't: light the world with one cutoff-free falloff, walk lights in hash order, mix reach with brightness, or let torch bytes reach the tick.

source: Lengyel, Foundations of Game Engine Development Vol 2, ch. 8.1.1
