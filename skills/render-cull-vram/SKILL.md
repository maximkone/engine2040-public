---
name: render-cull-vram
description: "Use when you add frustum, light, shadow, portal or occlusion culling, or VRAM math, a staging belt, streaming pool or eviction rule (crates render, spatial, assets, perf): bake bounds once, integer pool math, never in the hash."
---

# Culling and VRAM budgets

Both are presentation-only. Culling and VRAM answers never reach the tick or the hash (`docs/HASH-LAW.md`). A pass needs a test or capture, never a promise. Name the crate per finding. For pass-truth, ms and lighting rules see skills `render-pipeline-math` and `render-lighting`. For the GPU cull pass itself see `crates/render/src/gpu_cull.rs` (`GpuCuller::cull`, `check_inputs`, `CullProof::visible_ids`).

## 1. Culling: nine checks

Source: Lengyel Vol 2 ch. 9.3-9.9.

| # | Check | Flag | Crate |
|---|---|---|---|
| 1 | Bake tight volumes once. Sphere and box bounds are built at cook time from all vertices, for a small cross-section. Never rebuilt per frame. | per-frame min-max over the mesh, average-vertex centers, triangle meshes tested directly | spatial |
| 2 | Test in world space with near kept. Region planes face inward in world space. Test near plus four sides, far optional. | camera-space planes used raw, or near dropped so walls behind the camera survive | render |
| 3 | Skip and accept whole groups. A missed parent skips its children. A fully inside parent accepts them with no more tests. | a flat loop over every object, or retesting children of a fully inside parent | spatial |
| 4 | Cull spheres by center minus radius. Drop a sphere only when its center lies beyond its radius behind any one plane. | center-only tests that pop large objects, or exact corner math that costs more than its false positives | render |
| 5 | Cull boxes by projected extents. The per-plane radius is the summed absolute projections of the half-extents, with a fast axis-aligned path. | a sphere radius reused for long boxes, or a full vertex loop per plane | render |
| 6 | Cull lights before shading. Skip out-of-range points and out-of-frustum spot pyramids. Exclude unlit objects per light. | every light shades every object, or shadows dropped once the caster leaves the view | render |
| 7 | Build small shadow regions. The caster set is the hull of view plus light from silhouette edges and back faces, per cascade, clipped to ground. | the whole world drawn into each shadow map, or one giant region that never shrinks | render |
| 8 | Enter zones only through seen portals. Use front-facing visible portals with clipped extruded regions. Tag twice-seen objects. | every zone tested each frame, or overlap walls drawn twice | game |
| 9 | Hide only what is fully behind a proven blocker. An object is skipped only when its whole volume sits inside an occlusion region whose blocker tested visible. | partial overlap treated as hidden, or blockers used without their own visibility test | render |

Do: bake tight volumes, cull groups, then lights, then shadows, through portals, and hide only fully blocked objects. Don't: test every triangle every frame, or shade every light in every zone.

## 2. VRAM budget: eight checks

Pool math is pure integer math. Source: Game Engine Gems 1 ch. 11.

| # | Check | Flag | Crate |
|---|---|---|---|
| 1 | Pure integer math. Reservation, budget and usage use saturating integer ops, after `crates/assets/src/stream.rs::StreamQueue` (`used_bytes`, `remaining_bytes`). | float math, a panic on underflow, an OS query inside the pure fn | assets |
| 2 | Residency caps throttle. `crates/render/src/present/cache.rs::CpuCache::insert` refuses an over-capacity pool by evicting exactly one resident key and counting it (`evicted`, `last_evicted`). It never grows past the cap. | a CPU copy into the live pool, or an entry cleared before its fence | render |
| 3 | Chunks split and merge. The pool splits a free chunk on place and joins neighbours on free, aligned to the platform max. | fixed blocks for variable sizes, or neighbours left split so the pool shreds | assets |
| 4 | Moves stay ordered. Pool moves queue in insertion order: use finishes before move, move before reuse. See `CpuCache::insert`. | a CPU wait that tracks each texture use by hand | render |
| 5 | Defrag has a cap. Each pass is capped in bytes per frame and regioned. `crates/perf/src/budget.rs::check_time_ms` checks it. | a full-pool sweep with no cap that spikes on one hole at the start | perf |
| 6 | Eviction is counted once per frame. It compares resident and waiter order and keeps pinned L0. See `CpuCache::evicted` and `crates/assets/src/stream.rs::StreamEvent::Evicted`. | evict-on-miss with no score, L0 evicted, thrash with no guard | render |
| 7 | Headers apart from data. Fixed headers are pooled apart from variable data. Handles fail stale: `crates/assets/src/registry.rs::Registry::get`. | a bare slot index that silently reads the next occupant | assets |
| 8 | Never enters the hash. Budget, pressure and mip bias stay out of `state_hash`. `crates/perf/src/budget.rs::check_report` and `docs/HASH-LAW.md` prove it. | VRAM numbers steering the tick or moving the golden hash | perf |

Do: keep pure math saturating, queue moves in order, cap defrag, score eviction, and prove the golden hash unchanged. Don't: float the budget, copy on the CPU, sweep the pool uncapped, evict without a score, or feed VRAM into the tick.

## 3. Cut notes (recover with git)

- `crates/render/src/uploads.rs` (the staging belt: `StagingBelt::upload`, `OverCapacity`, `advance_frame`, `invalidate_all`) is cut. Check 2 now watches `CpuCache`. Recover the old shape: `git log --follow -- crates/render/src/uploads.rs`.
- `crates/render/src/stream.rs` (tile residency: `TileResidency`) is cut. Check 4 now watches `CpuCache`. Recover the old shape: `git log --follow -- crates/render/src/stream.rs`.
- The cache release on resize, surface loss and device loss is described in skill `wgpu-render` section 6 rule 6.

Sources: Lengyel, Foundations of Game Engine Development Vol 2 ch. 9.3-9.9; Game Engine Gems 1 ch. 11.
