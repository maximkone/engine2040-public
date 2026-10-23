# Finish-line steals, wave 2026-10-23

Scope: the five gaps that `docs/FOCUS.md` item 10 and
`docs/plan/finish-line.md` "What we still lack" still leave open:
GPU crowd at 20k+ reacting agents, streamed maps in cells, rollback netcode
with save/load and desync detection, bonded runtime destruction, and wgpu 30
code we can port (cull, virtual geometry, clustered light, TAA, bloom).

Method: every row was read on the donor's default branch through
`raw.githubusercontent.com` at the pinned SHA (the same bytes as the clone I
listed paths from). Licences were read from the repo's own licence file at the
same SHA; a Cargo `license` field is quoted where that is the only statement.
No bytes are vendored; ported code is rewritten in our own words at the site
named in the table.

## What we already hold (do not re-steal)

- Crowd VAT and octahedral impostor: `crates/render/src/shaders/crowd_vat.wgsl`
  ports `Mefodei/Render-Crowd-Of-Animated-Characters` and
  `wojtekpil/Godot-Octahedral-Impostors`; 50k seats, LOD bands 18/45 m,
  `CROWD_INST_BUDGET_MS = 0.5`, 20k no-alloc spike.
- Fyrox tile occlusion cell memory: card 6 of
  `docs/research/steals-2026-10-18/01-render.md` (take the cell memory, not the
  readback). HiZ already lives in `crates/render/src/shaders/cull.wgsl:61-96`.
- GGRS frame advantage and SyncTest shape: `crates/net/src/delay.rs`
  (`ClockPacer`, 30-frame window) and `crates/net/src/played.rs::sync_test`
  (distance 7, three tapes).
- Meshlet cluster cook and cone cull: `crates/assets/src/meshlet.rs` (64v/124t
  greedy growth) and `meshlet_cull.rs` (conservative sphere plus cone). Both
  have no draw caller.
- Froxel light assign, motion blur, volumetric fog, DOF, SSR: read from
  filament, bevy and godot in `docs/research/steals-2026-10-23/render-gpu.md`.
- Blast elastic/fatal bond bands: `crates/destruction/src/bond_grade.rs` and
  `bonded.rs` (`ELASTIC_BAND`, `ELASTIC_CHIP`).

## The donors

| Area | Donor | Licence | Pinned rev | File (read live) | What we take | What we change |
|---|---|---|---|---|---|---|
| 1 crowd draw | BabylonJS/Babylon.js | Apache-2.0 | `c9e77a1189e1cc2ae762ec4b88ce9085d2803bcc` | `packages/dev/core/src/ShadersWGSL/ShadersInclude/bakedVertexAnimation.fx`, `packages/dev/core/src/BakedVertexAnimation/bakedVertexAnimationManager.ts` | Per-instance `vec4 (startFrame, endFrame, offset, speed)` (`setAnimationParameters`, bound as `bakedVertexAnimationSettingsInstanced`) and the loop correction `frameCorrection = select(1.0, 0.0, time < 1.0)` | `crowd_vat.wgsl` has one shared `vat.frames` and a 3-bit `variant` in word 3; add an 8-row clip table `(start, end)` and carry offset+speed in word 4. The frame stays `(tick + phase) % frames` on integers, so the golden never sees this |
| 1 crowd audio | overte-org/overte | Apache-2.0 (`LICENSE`, same SHA) | `9820f3ae7bc94ae15ad7d472d524a401146a4373` | `assignment-client/src/audio/AudioMixerWorker.cpp` (`prepareMix`, `shouldBeInactive`, `shouldBeSkipped`, `approximateVolume`, the `nth_element` throttle), `assignment-client/src/audio/AudioMixerClientData.h` (`Streams { active, inactive, skipped }`) | Three-state voice sets, silence-driven `inactive`, and a per-listener top-N throttle on `trailingLoudness x distance gain`, with the tail flushed on the first silent frame | One crowd bed today (`crates/audio/src/bank.rs`) and one global reaction level (`crowd.rs`). Add a deterministic candidate list per sector and keep the top 8; ordering by integer gain then block id, no wall clock |
| 2 cells load/unload | ChousX/chunky-bevy | MIT OR Apache-2.0 | `c0d18a567c36c161060f4b2014835640dc6e5ab0` | `src/chunk_unloader.rs` (`ChunkUnloadLimit`, `unload_chunks_hybrid`, `ChunkUnloadReason`), `src/chunk_loader.rs` (`ChunkLoader`, `ChunkUnloadRadius`) | Separate load and unload radii (hysteresis, no edge flicker), an LRU cap as a second budget, a pinned set, and a reason on the unload event | `crates/world/src/cells.rs` already has byte/job budgets and an explicit `evict`; add `pin`, `unload_limit` on `CellBudget`, and `CellEvent::Evicted { reason }` so a drop can be told from a cancel |
| 2 cells occlusion | godotengine/godot | MIT | `c8256a8ecec068340753869e423dd478c84ef167` | `servers/rendering/renderer_scene_occlusion_cull.cpp` (`HZBuffer::resize`, `HZBuffer::update_mips`, the odd-size `CHECK_OFFSET` taps, `occlusion_jitter_enabled`) | Conservative max-depth mip pyramid (a covered block is never wrongly visible), correct odd-size edges, and the one-frame jitter switch for objects on the boundary | Our cells are ordered by a frustum only (`stream(coord, size, priority)`). Feed a per-cell coverage ratio from the existing HiZ into that `priority`, keep the readback off the frame, and hold a cell that contains the camera |
| 3 rollback save/load | gschup/ggrs | MIT OR Apache-2.0 | `25bd9983dee3ecb0677be03b9e2e23d2bd6d29cb` | `src/sync_layer.rs` (`GameStateCell::save(frame, data, checksum)`, `SavedStates::get_cell`, `load_frame`'s `assert_eq!(cell.frame, frame_to_load)`) | The checksum lives inside the save cell, the ring is exactly `max_prediction + 1`, and a load of a cell that is not the requested frame refuses | `crates/net/src/rollback.rs` keeps `checksum_log` beside the tape. Move the per-frame hash into the `FrameLog` row and refuse a misaligned load with the existing `RollbackError::History` (no wire change) |
| 3 rollback desync | cBournhonesque/lightyear | MIT OR Apache-2.0 | `ee50df6664338ff1eb4b3885c8525fa3c9030691` | `crates/deterministic/deterministic_replication/src/checksum.rs` (`ChecksumHistory`, `PendingP2PChecksums::record_local`, `record_remote`, `clean`, the order-independent XOR fold) | Pair a local and a remote checksum by tick before comparing, keep 30 ticks, and fold per entity so iteration order cannot false-flag | `crates/net/src/desync.rs` compares one peer hash at one frame; add the pairing window and keep the report fields as they are |
| 4 destruction | bulletphysics/bullet3 | zlib | `d3afe76132fc76b0f9b2359cbaef47a3766e7fbb` | `examples/FractureDemo/btFractureDynamicsWorld.cpp` (`glueCallback`), `examples/FractureDemo/btFractureBody.cpp` (`recomputeConnectivity`, `shiftTransform`) | Coalesce the resting pieces of one island into one body, keep their impulses as linear velocity x mass, shift to the principal axis, then rebuild bonds from a contact test with a 0.05 margin | `crates/destruction/src/bonded.rs` keeps every shard as its own body; add `coalesce()` for all-asleep sibling sets at a tick boundary, hashed in id order. Rules v2 only, the v1 golden scene keeps its own path |
| 5 wgpu 30 cull | bevyengine/bevy | MIT OR Apache-2.0 (wgpu 30 in `crates/bevy_render/Cargo.toml`) | `84e2d2cc5adaeff7547e44f434a0d0d7a20345b8` | `crates/bevy_pbr/src/meshlet/cull_clusters.wesl`, `cull_shared.wesl` (`should_occlusion_cull_aabb`, `lod_error_is_imperceptible`), `from_mesh.rs` (`TARGET_MESHLETS_PER_GROUP = 8`, `SIMPLIFICATION_FAILURE_PERCENTAGE = 0.60`, locked borders, `group_error = max(...)`) | Two-pass cluster cull (error plus frustum, then HZB with children pushed to pass two), the 64 px software/hardware raster split, and LOD groups of 8 with borders locked | `crates/render/src/meshlet.rs` is a sketch and `gpu_cull.rs` has no caller. Wire the cooked clusters from `crates/assets/src/meshlet.rs` into the two-pass cull and one indirect draw. WebGPU has no mesh shaders, so the software-raster lane is the web leg |

Bloom, TAA, GTAO and froxel light are not in the table: `post/taa.rs`,
`post/chain.rs`, `gtao.rs` and the earlier froxel steal already hold them. A
second bevy file, `crates/bevy_pbr/src/render/gpu_preprocess.rs`, is only a
fallback if the meshlet path cannot own the indirect draw.

## Top 3 by what they move in our tree

### 1. Two-pass cluster cull with an indirect draw (bevy meshlet)

- Home: `crates/render/src/meshlet.rs` (the draw path) plus
  `crates/render/src/gpu_cull.rs` (the pass), clusters from
  `crates/assets/src/meshlet.rs`.
- Number it moves: the arena kit's 11 draws become 1 indirect draw per
  material, and the clusters submitted drop to at most 40% at the default
  camera. `gpu_cull.rs` callers go 0 -> 1.
- Bounded repro: one lib test, 1280x720, arena kit plus 8 fighters, one fixed
  camera, read back the cull count and the indirect args; compare pixels with
  the existing mesh path on the same frame:
  `node tools/gh-check.mjs cargo test -p engine-render --features window --lib meshlet_two_pass`
  (GPU leg, so it runs on this PC).
- Kill: stop if the mean per-pixel difference against the mesh path is above
  1/255, if the 64 px software-raster lane cannot run on the web tier, or if
  the pass costs more than 0.4 ms at 1280x720 on the RTX 3050.
- Why first: it is the open half of must item B7 (cull, sort, instancing in
  the live draw loop) and it serves both the stands and the crowd.

### 2. A clip tuple per crowd seat (Babylon baked vertex animation)

- Home: `crates/render/src/crowd_inst.rs` (`pack_instance_full`, word 3/4),
  `crates/render/src/shaders/crowd_vat.wgsl` (`vat_frame_of`), clip table from
  `crates/assets/src/vat.rs`.
- Number it moves: distinct crowd clips 1 -> 4 (idle, cheer, gasp, flinch) over
  the same single atlas, at 20k seats, with the stage still inside
  `CROWD_INST_BUDGET_MS = 0.5`.
- Bounded repro: extend the existing `vat_crowd_spike` to 20k seats, 600 ticks,
  assert four different frames at one tick for four clip ids and 0 allocations;
  one capture pair (one clip vs four).
- Kill: stop if the stage passes 0.5 ms, if the atlas grows past 2x its bytes,
  or if the capture pair shows a visible pop at the loop seam.
- Why second: the arena is the first proof and its crowd must read as reacting,
  not as one bobbing mass; this is presentation only and cannot move the hash.

### 3. Cell priority from visibility (godot HZBuffer rule + chunky unload policy)

- Home: `crates/world/src/cells.rs` (`stream(coord, size, priority)`,
  `CellBudget`, `CellEvent`).
- Number it moves: in a 5x5 walk past one wall, live cells 25 -> at most 16 and
  the high-water bytes drop by half, while the existing nine-cell walk test
  keeps its 5% memory bar and zero hitches.
- Bounded repro: a camera path behind a wall, 600 frames, assert the evicted
  set and the priority order are equal on two runs, and that every cell that
  drew pixels was live.
- Kill: stop if any cell that draws pixels is evicted or never streamed within
  two frames of the camera entering it, or if the coverage query costs more
  than 0.2 ms per frame on the CPU.
- Why third: the Roman chapters carry dig and height cells; the loader today
  orders by distance only, so a courtyard cell behind a wall costs the same as
  the one in front of the camera.

Not ranked: rollback (row E23 waits on the owner decision and gates G7/G8),
destruction coalescing (rules v2, after the golden's v1 scene is frozen), crowd
voice thinning (cheap, but it lands after the clip tuple gives it something to
pick).

## Ideas only (no code)

- `OpenMW` (GPL-3.0) cell store and preload: interior/exterior cell split with
  a door teleport. Idea only.
- `Veloren` (GPL-3.0) chunk mesh budget: per-frame meshing quota. Idea only.
- `ioquake3` (GPL-2.0) 32-channel sound allocator: distance and priority slot
  reuse. Idea only; the Overte row is the permissive version.
- Blender cell fracture (GPL): cook-time only, already covered by
  `crates/destruction/src/fracture.rs`.

## Could not verify

- No donor was built or run here. Every number above is our constant or a
  target, not a measured donor result.
- Babylon's TypeScript through the WGSL include was read at the pinned SHA;
  the half-float path was not exercised, and `thinInstanceMesh` was listed but
  not read.
- Overte: only `AudioMixerWorker.cpp` to the end of `prepareMix` and
  `AudioMixerClientData.h` were read; `AudioMixer.cpp` (942 lines) was read to
  about line 340, and the HRTF renderer was not opened.
- Godot: `HZBuffer` was read whole (5 functions). The occluder raster and the
  query side live outside that file and were not read; the cell-coverage reuse
  is our construction, not the donor's.
- GGRS: `sync_layer.rs` was read to line 250 of 668, so sparse saving and the
  P2P checksum interval were only seen by name. The 0.13.0 crate was not run.
- Lightyear: `checksum.rs` was read to line 250 of 669; the receive plugin and
  the message shape were not read.
- Bullet: `glueCallback` to the end of the island loop, `btFractureBody.cpp`
  whole; `btFractureDynamicsWorld.cpp` past line 250 (the `processBreak` side)
  was not read.
- Bevy: the meshlet cull shader and `from_mesh.rs` were read to line 250 of
  1099; the resolve and material-shade nodes were listed only. Whether the
  meshlet path draws the arena kit within the web tier's limits is unmeasured.
- The chunky-bevy repo is small and unproven; its LRU policy is the steal, not
  its quality.
