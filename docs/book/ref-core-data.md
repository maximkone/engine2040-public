# Reference: core, ecs, geo, spatial, scene, world, save, observe, ui, audio, app

Details only. Signatures are copied from source. Units: meters, radians, seconds, fixed 60 Hz ticks,
millimetres (`_mm`), centimetres (`_cm`), permille, bytes.

Read first: `docs/HASH-LAW.md` and `docs/ARCHITECTURE.md` (`core` is std-only at the bottom; `save` never
imports game or arena; `geo` is cook-time only). `crates/core` has no `ids` module: identifiers live in
`ecs::Entity`, `scene::ObjectId`, `input::FighterId`.

## 0. Determinism classes in this group

| Class | Crates | Rule |
|---|---|---|
| Hashed, integer or `Fixed` | `core` (`fixed`, `hash`, `rng`, `counter_rng`, `tick`), `ecs` (`archetype`, `schedule`), `save` (envelope bytes), `world` (bake bytes), `scene` (text) | Same input gives same bits. FNV-1a 64 is a regression checksum, not an integrity proof. |
| Hashed, float on the input edge | `spatial` (f32 in, integer cm inside; FNV report hash), `ui` (`state_hash` over ints) | Floats are converted to integers before any fold. |
| Cook-time only, float allowed | `geo` | Pure function of mesh and options. Never in the tick graph. |
| Presentation, never hashed | `audio`, `observe` (diagnostics), `app` (extraction, wall-clock startup) | Output does not feed simulation state. |

Test command for every crate: `node tools/gh-check.mjs cargo test -p <crate> --lib`.

## 1. `engine-core` (`crates/core`)

Std-only foundation: fixed tick, seeded streams, hashes, fixed-point math.

### 1.1 Fixed-point and deterministic math

| Item | Signature and behaviour |
|---|---|
| `Fixed(pub i64)` (Copy, Ord, Hash) | Q47.16: raw counts of 1/65536. `SCALE` 65536, `FRAC` 16. Constants `ZERO`, `ONE`, `HALF`, `NEG_ONE`, `MAX` (about 1.4e14), `MIN`, `PI`, `TAU`, `FRAC_PI_2`, `FRAC_PI_4`. |
| `from_bits(i64)`, `to_bits()`, `from_int(i64)`, `to_int()`, `to_le_bytes()`, `from_le_bytes([u8;8])` | Canonical bytes for hashing. |
| `+ - *` and `wrapping_*` | Wrap on overflow. `wrapping_mul` is `(a * b) >> 16` in i128, rounding toward negative infinity. |
| `/` (`wrapping_div`) | Panics on zero divisor. Use `checked_div` on untrusted input. |
| `checked_add/sub/mul/div`, `saturating_add/sub`, `abs`, `min`, `max`, `clamp(lo, hi)`, `lerp(o, t)` | `checked_*` return `Option<Self>`. |
| `sqrt(self) -> Self` | Integer square root of raw * SCALE. Non-positive gives zero. |
| `sin`, `cos`, `sin_cos`, `tan` | 64-segment quarter-wave table, linear interpolation, any input range. `tan` saturates to `MAX`/`MIN` at zero cosine. |
| `Fixed::atan2(y, x) -> Self` | 16-step CORDIC, result in `-PI..=PI`. `(0, 0)` gives zero. |
| `det_math::det_sin`, `det_cos`, `det_sin_cos`, `det_exp`, `det_atan2za` | Polynomial and series in Q32 on `Fixed`, no tables, no heap. `det_exp` uses 16 terms and saturates outward. |
| `quat::quat_mul(a: [f32;4], b: [f32;4]) -> [f32;4]`; `quat_mul_fixed([Fixed;4], [Fixed;4]) -> [Fixed;4]` | One product for every crate. Float version is presentation. |
| `vec3::cross3`, `vec3::dist2` | `f32` helpers. |

Example, adapted from `arith_exact` (`crates/core/src/fixed.rs`):

```rust
assert_eq!(Fixed::from_int(3) / Fixed::from_int(2), Fixed::from_bits(98304));
assert!(Fixed::from_int(1).checked_div(Fixed::ZERO).is_none());
```

### 1.2 Hashes (`hash.rs`, `digest.rs`, `adler.rs`)

| Item | Behaviour |
|---|---|
| `FNV64_OFFSET`, `FNV64_PRIME`, `GOLDEN_GAMMA` | FNV-1a 64 constants; golden gamma `0x9e37_79b9_7f4a_7c15`. |
| `Fnv64::new()`, `with_state(u64)`; `write_u8/u16/u32/u64/i32/i64/f32/bool/str`; `write(&[u8])`; `finish() -> u64` | Streaming FNV-1a 64. `write_f32` folds `to_bits`. |
| `hash_bytes(&[u8]) -> u64`; `hash_u64s(&[u64]) -> u64`; `hash_hex(&[u8]) -> String`; `hex16(u64) -> String` | |
| `mix64(u64) -> u64` (bijection); `splitmix64(u64) -> u64` = `mix64(z + GOLDEN_GAMMA)` | Seed expansion; used by `arena::seed`, `world`, `audio`. |
| `digest::TwinFold::{new, push_a_u32, push_b_u32, finish}` | Two-lane digest over `u32` words. |
| `adler::adler32(&[u8]) -> u32` | Checksum for test fixtures. |

### 1.3 Random streams (`rng.rs`, `counter_rng.rs`)

| Item | Behaviour |
|---|---|
| `Rng::new(seed: u64)` | Stores `seed \| 1`. Seeds `2k` and `2k+1` alias on purpose. TRAP in source: removing `\| 1` changes every legacy replay and `GOLDEN_600`. |
| `from_state(u64)`, `state()`, `next_u64()` (xorshift 13/7/17), `next_u32()` (high 32 bits), `next_range(lo, hi)` (modulo, slightly biased; returns `lo` when `hi <= lo`) | Raw state for snapshots. |
| `fork(stream: u64) -> Self`, `fork_named(&str) -> Self`, `digest(&[u64])`, `hash_state(&[u64])` | |
| `CounterRng::new(seed)`, `key()`, `sample(counter: u64) -> u64`, `fill(&mut [u64], base)`, `fork`, `fork_named`, `for_island`, `for_chunk` | Counter-based: adjacent seeds do not alias. Use for new fixtures. |
| `CounterCursor::new(seed)`, `from_rng(CounterRng, base)`, `rng()`, `position()`, `next_u64()`, `next_u32()`, `next_range(lo, hi)` | Sequential view of a `CounterRng`. |

Example, adapted from `first_value_golden` (`crates/core/src/rng.rs`): `assert_eq!(Rng::new(2040).next_u64(), 2189151099318);`

Test: `node tools/gh-check.mjs cargo test -p engine-core --lib` (`chunkings_1_2_4_8_identical`, `same_seed_twice_identical` in `counter_rng.rs`).

### 1.4 Tick, time, config, buffers

| Item | Signature and behaviour |
|---|---|
| `TICK_HZ` (u64) = 60; `DT` (f32) = 1/60; `LEGACY_DT`; `MAX_STEPS` = 5; `MAX_FRAME_DT` = 0.1 (legacy names kept) | `tick.rs`. |
| `Tick(pub u64)`; `TickState::new(seed)`, `step(step_hash) -> u64`, `hash()`; `fold_hash(acc, tick, step_hash) -> u64`; `run_ticks(ticks, seed, step) -> Vec<u64>` | Per-tick chain hash. |
| `LegacyFixedAcc::{new(dt), fixed(), push(frame_dt: f32) -> u32, alpha()}`; alias `FixedAcc` | Frame-to-step accumulator, capped at `MAX_STEPS` (anti-spiral). |
| `time::IntAcc::{new, tick, buffered_ns, buffered_thirds, alpha, push_ns(u64) -> Result<u32, TimeError>, push_duration(Duration), push_secs(f32)}`; `quantize_wall_secs(f32) -> Result<u64, TimeError>` | Integer wall clock to steps. `STEP_NS` 16 666 666, `STEP_THIRDS`, `MAX_FRAME_NS` 100 000 000, `NS_PER_SEC`. Wall clock is never a sim input. |
| `config::Config::{new(seed), try_new(seed, tick_hz) -> Result<Self, ConfigError>, validate(), dt()}`; `Budgets` | `ConfigError` refuses bad tick rates. |
| `checked::{saturating_i64_to_i32, saturating_usize_to_u32, u32_from_u64 (const), decimal_u32(&str) -> Option<u32>}` | Narrowing without panics. |
| `arena::SliceArena::{new, with_capacity_words, reserve_words, reset, used_words, capacity_words, remaining_words, alloc(len) -> Result<&mut [u32], ArenaError>, push_slice(&[u32])}`; `grow_zeroed` | Bump arena of `u32` words. `ArenaError` when full. |
| `seal_key::{parse_seal_key(&str) -> Result<Vec<u8>, SealKeyError>, seal_key_from_env(&str), decode_seal_hex, parse_hex4}`; `MIN_SEAL_KEY_BYTES` 32 | Key material for sealed files (`save`). Never logged. |
| `alloc_probe::CountingAlloc`; crate consts | Test global allocator (`measure` counts heap calls on one thread). Consts: `FIXED_DT` (f32), `TICK_HZ_U32`, `STUCK_WINDOW_TICKS` = 3 x 60. |

Tests: `fixed_60hz_consts`, `acc_cap5_anti_spiral` (tick.rs); `arith_exact` (fixed.rs); `stream_same_seed` (rng.rs).

## 2. `engine-ecs` (`crates/ecs`)

Two worlds with one checkout rule (`WORLD_FORK_DECISION`). The legacy `World` (`lib.rs`) stays
byte-stable for the frozen proof path. `archetype::World` plus `PhaseArena` is the tick path for new code.

| Item | Signature and behaviour |
|---|---|
| `Entity { id: u32, gen: u32 }` | Generational. Stale handles fail closed in every accessor. |
| `Position { x, y }`, `Velocity` | Component records. |
| `ComponentId(u8)`; `Registry::{new, register::<T>(name) -> Option<ComponentId>, bit::<T>()}` | Column bits. |
| `archetype::World::{new, tick(), advance(), spawn(Option<Position>, Option<Velocity>) -> Entity, despawn(e) -> bool, kill(e) -> bool, flush(), flush_with(&mut PhaseArena) -> Result<(), ArenaError>, count(), pending(), reserve(n)}` | `kill` and `spawn` are deferred until `flush`. |
| `get_pos`, `set_pos`, `get_vel`, `set_vel`, `add_vel`, `remove_pos`, `remove_vel`, `has_pos`, `has_vel` | Return `Option`/`bool`; stale entities give `None`/`false`. |
| `add_marker`, `remove_marker`, `has_marker`, `marker_count`, `added_marker_since`, `changed_marker_since` | Zero-size markers. |
| `for_each_p(f)`, `for_each_pv(f)`, `for_each_pv_changed(since: Tick, f)`, `changed_pos_since(e, last_run, this_run)` | Iteration in storage order. |
| `check_change_ticks()`, `check_change_ticks_if_due()` | Clamps stamps older than the max change age. |
| `Schedule<W>::{new, add_system(phase, name, f), add_system_with_access(phase, name, reads, writes, f), add_order(before, after), set_shuffle_seed(seed), validate(), ambiguities(), run(&mut W, dt: f32), run_arena_tick(&mut ArchetypeWorld, &mut PhaseArena, dt: f32) -> Result<u64, ArenaError>, last_run(name)}` | Phase order, declared edges, then seed shuffle of free systems. The returned `u64` is the tick checkpoint. |
| `Phase::{Startup, PreUpdate, Update, PostUpdate}`, `Phase::ORDER` | Startup runs once. |
| `PhaseArena::{new, with_capacity(entity_slots, byte_len), reserve, begin_phase, clear, entities(), bytes(), try_push_entity, try_extend_entities, try_extend_bytes}` | Caller-sized per-phase scratch. Zero heap after setup. `ArenaError::Overflow` names the shortfall. Phases already run keep their effects. |
| `JobPool::new(workers: usize) -> Result<Self, PoolError>`; `scope(f)`; `par_chunks_map(&[T], ChunkSize, f) -> Vec<R>`; `jobs::{par_for_each_mut, par_map_default, split_ranges(len, chunk)}` | Parallel helpers. Results keep input order; thread count never changes a reduction. |
| `SpineSchedule<W>::{new, register, len, build() -> Result<Schedule<W>, SpineScheduleError>}`; `SpineSystem` | Named systems for the spine step (feature `spine` in `arena`). |

Example, adapted from `spawn_query_counts` (`crates/ecs/src/archetype.rs`):

```rust
let mut w = World::new();
w.spawn(Some(Position { x: 0.0, y: 0.0 }), Some(Velocity { x: 1.0, y: 0.0 }));
w.spawn(Some(Position { x: 1.0, y: 1.0 }), None);
assert_eq!(w.count(), 2);
let mut moving = 0; w.for_each_pv(|_, _, _| moving += 1);
assert_eq!(moving, 1);
```

Tests: `node tools/gh-check.mjs cargo test -p engine-ecs --lib`, plus `--test arena_schedule` and
`--test restore_resimulate_reverse`. Named: `generation_rejects_stale_handles`, `swap_remove_remaps`,
`fixed_order_within_phase`.

## 3. `engine-geo` (`crates/geo`)

Cook-time mesh tools (decision D4). Never in the fixed tick. Results are pure functions of the input
mesh and options: no clock, no thread count, no seed the caller did not pass.

| Item | Signature and behaviour |
|---|---|
| `GeoMesh::from_positions(Vec<[f32;3]>, Vec<u32>) -> Self` | Triangle soup with indices. |
| `validate() -> Result<(), GeoError>`; `triangle_count()`; `weld_map()`; `edge_report() -> EdgeReport`; `open_edge_count()`; `volume() -> f64`; `bounds() -> ([f32;3],[f32;3])`; `digest() -> u64`; `welded()`; `placed(scale, quarter_turns_y: u32, offset)` | `GeoError` for bad meshes. `EdgeReport` counts open and non-manifold edges. |
| `ops::{weld_by_distance(&GeoMesh, epsilon), mirror(&GeoMesh, Axis), array(&GeoMesh, count: u32, offset), flip, recompute_normals}` | Always return new meshes. |
| `primitives::{cuboid(size), cuboid_faces, uv_sphere(radius, rings, segments), cylinder(radius, height, segments), torus(major, minor, major_segments, minor_segments), l_prism(size, arm, height)}` | Test shapes and kit parts. |
| `boolean::boolean(&GeoMesh, &GeoMesh, BoolOp) -> Result<GeoMesh, GeoError>` (feature `boolean`, manifold-rust) | Union, difference, intersection. |
| `lod::{simplify, chain, seam_vertices}`; `LodOptions`, `LodResult` (feature `lod`, meshopt) | Seam-locked simplification. |
| `collision::{single_hull(&GeoMesh, max_hull_vertices) -> Result<GeoMesh, GeoError>, decompose(&GeoMesh, &HullOptions) -> Result<Vec<GeoMesh>, GeoError>, digest(&[GeoMesh]) -> u64}` (feature `collision`, parry3d) | Convex decomposition. |
| `uv::{unwrap(&GeoMesh, &UnwrapOptions) -> Result<Unwrap, GeoError>, overlapping_chart_pairs}` (feature `uv`, vendored xatlas via `cc`) | UV unwrap and chart packing. |
| `recipe::{Recipe, Op, BooleanKind, run(&Recipe) -> Result<RecipeResult, RecipeError>}` | Budgets: `MAX_OPS` 64 operations, 200 000 triangles, 32 name characters. |
| `bonemap` | Humanoid auto-map: 22 bones named from a rig's joint names. |

Example, adapted from `convex_shape_stays_one_part` (`crates/geo/src/collision.rs`):

```rust
let parts = decompose(&cuboid([1.0, 2.0, 3.0]), &HullOptions::default()).unwrap();
assert_eq!(parts.len(), 1);
assert!((parts[0].volume() - 6.0).abs() < 0.5);
```

Test: `cargo test -p engine-geo --lib --features boolean,collision,lod,uv`; kit: `tests/kit.rs` (feature `kit`).

## 4. `engine-spatial` (`crates/spatial`)

Deterministic scene reports. Same actors in any order give the same hash. Owns no sim state. Model-free.

| Item | Signature and behaviour |
|---|---|
| `Actor { id: u32, name: String, x, y, z: f32 (m), sx, sy, sz: f32 (m), parent: Option<u32>, physical: bool, vx, vz: f32 (m/s), yaw: f32 (rad), .. }` | Scene snapshot row. |
| `Finding { check: &'static str, a: u32, b: u32, pct: f32, severity: &'static str }` | `b == u32::MAX` means a single-actor finding. |
| `Report { findings: Vec<Finding>, hash: u64 }`; `verdict() -> &'static str`; `count(check) -> usize` | FNV-1a over canonical bytes. |
| `run(&[Actor]) -> Report` | The 14 offline checks (catalog minus the four nav checks). Never fails. |
| `run_checks(&[Actor], &[&str]) -> Result<Report, Vec<&'static str>>` | Unknown names return the legal list. Never an empty pass. |
| `run_spatial(&[Actor], &[&str], nav: Option<&navcheck::NavView>) -> Result<Report, Vec<&'static str>>` | Nav checks need a bake view. |
| `SPATIAL_CHECKS` (18 names), `NAV_CHECKS` (`navConnect`, `flowPath`, `accessFront`, `doorClear`), `OPT_IN_CHECKS` (`relationCycle`) | Opt-in names are legal in `run_spatial` but absent from `SPATIAL_CHECKS`. |
| Thresholds (private, `checks.rs`) | World bound 10 000 cm (red); out-of-bounds 9 900 cm (yellow); giant object 5 000 cm; floating epsilon 1 cm; support gap 5 cm; duplicate-name limit 10; sweep horizon 60 ticks; density cell 100 cm, max 8 per cell; name length 64. |
| `brief(&[Actor], &Graph, &Report, budget_bytes: usize) -> Brief`; `brief_text(&Brief)`; `brief_stats`; `estimate_tokens` | Bounded text for a model: whole rows only; the digest row is always kept. |
| `relation::{build, digest, rebuild_cached, Graph, Layer, Relation}` | Relation graph; cache keyed by content. |
| `diff::{diff_graph, dry_run_diff, describe, summarize, SemanticDiff}` | Semantic diff of two graphs. |
| `staged::{check_staged, actors_from_snapshot, staged_to_actors, full_check_timed}` | Staged scenes (pre-apply checks). |
| `place` (intent solver), `repair` (repair loop, seeded scenes), `sight`, `exposure`, `isovist`, `fps_visibility`, `afford`, `kdtree` (integer-cm 3D k-d tree), `index` (`overlap_pairs`, `sweep_pairs`, `nearest`), `tag_lift::lift_label`, `fuzz` | Supporting queries; `fuzz` is the seeded op-stream harness with first-divergence receipts. |

Example, adapted from `overlap_reroute_matches_legacy_on_flat_scene` (`crates/spatial/src/checks.rs`):

```rust
let scene = vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 0.5, 0.0),
    resting(2, "c", 5.0, 5.0), resting(3, "d", 5.4, 5.0), resting(4, "e", 40.0, 40.0)];
assert_eq!(reported_overlap(&scene).len(), 2);   // a/b and c/d overlap
```

Test: `node tools/gh-check.mjs cargo test -p engine-spatial --lib`. Named: `layer_assignment_follows_bounds_nesting`,
`supports_pair_contrasts_with_moved_crate` (`relation.rs`), `overlap_reroute_is_input_order_invariant` (`checks.rs`).

## 5. `engine-scene` (`crates/scene`)

Persistent identity and versioned text. `ObjectId` is stable across reimports; `content_digest` versions bytes.
Never use one for the other.

| Item | Signature and behaviour |
|---|---|
| `ObjectId` (UUID wrapper): `new()`, `NIL`, `as_uuid()`, `is_nil()`, `parse(&str) -> Result<Self, ParseIdError>`; `content_digest(&[u8]) -> String` | Persistent identity; digest versions bytes. |
| `Transform::{new(position, rotation, scale), identity(), is_finite(), to_matrix() -> [[f32;4];4]}` | Position m, rotation quaternion `[x,y,z,w]`, scale. Non-finite is refused by `Hierarchy`. |
| `Hierarchy::{new, len, is_empty, contains, insert(id, parent, Transform), set_local(id, Transform), local(id), parent(id), reparent(id, new_parent), remove(id), world(id), children(parent), roots(), propagation_order(), world_all(), validate()}` | Cycle rejection, deterministic propagation order. `HierarchyError` on bad links. |
| `codec::Scene { version: u32, nodes: Vec<SceneNode> }`; `SceneNode { id, parent, transform }`; `Scene::encode(&Hierarchy)`; `decode(&self) -> Result<Hierarchy, CodecError>`; `migrate()`; `digest() -> String`; `pin(revision: u64) -> Result<ScenePin, PinError>` | JSON text via serde. Nodes sorted by id: same tree, same text. `CURRENT_VERSION` 1; `MIN_SUPPORTED_VERSION` 1. Newer or older versions fail with `CodecError::UnsupportedVersion`. |
| `component::ComponentKind` (8 kinds: `Mesh`, `Collider`, `Material`, `Light`, `Spawner`, `Destructible`, `Anatomy`, `Rig`); `ALL_KINDS`; `tag()`, `from_tag(u8)`, `name()`, `from_name(&str)`, `name_list()` | |
| `Component::{kind(), validate() -> Result<(), ComponentError>, to_bytes(), from_bytes(&[u8]), to_json(), from_json(kind, &str), to_versioned(), from_versioned(..), describe()}` | Bounds: `MAX_ASSET_NAME_LEN` 64, `MAX_CHUNKS` 4096, `MAX_LIMBS` 32, `MAX_EXTENT_M` 100 000. |
| `register_builtin(&mut SchemaRegistry) -> Result<(), SchemaError>` | Schema metadata for inspectors and AI tools. |
| `pin::ScenePin`, `prefab`, `provenance` | Run pins for pre-start gates; prefab overrides; serializable provenance records. |

Example, adapted from `roundtrip_preserves_ids_links_and_poses` (`crates/scene/src/codec.rs`):

```rust
let text = serde_json::to_string(&Scene::encode(&tree)).unwrap();
let back = serde_json::from_str::<Scene>(&text).unwrap().decode().unwrap();
assert_eq!(back.world(child), tree.world(child));
```

Test: `node tools/gh-check.mjs cargo test -p engine-scene --lib`. Named: `encoding_is_deterministic_across_insertion_orders`,
`insert_rejects_bad_links`, `world_pose_composes_down_the_chain`.

## 6. `engine-world` (`crates/world`)

Deterministic bake: seed and size give versioned bytes. Std-only. Pathfinding lives in `engine-nav`; `world`
only emits cells and heights.

| Item | Signature and behaviour |
|---|---|
| `MAX_BAKE_SIZE` = 256 (texels, per side) | Checked before any allocation. |
| `terrain::bake_terrain(seed: u64, size: u16) -> Result<Vec<u8>, BakeErr>` | Bytes: `MAGIC` `b"W204"`, version 1, seed LE (8), size LE (2), then `size * size` height bytes. |
| `parse_terrain(&[u8]) -> Option<Terrain>`; `Terrain { seed, size, heights: Vec<u8>, holes: Vec<bool>, splat: Vec<[u8;3]> }` | `None` on bad bytes. |
| `blocked(&Terrain, limit: u8) -> Vec<bool>`; `terrain_hash(&[u8]) -> u64`; `bake_hash(seed, size) -> Result<u64, BakeErr>` | |
| `ground::{bake_ground(seed, size) -> Result<GroundBake, BakeErr>, bake_ground_bytes, parse_ground(&[u8]) -> Option<GroundBake>, blocked_tiles(&GroundBake)}`; `GroundProp` | Ground tiles and props. |
| `scatter::poisson_disk(...)` with `ScatterErr` | Poisson-disc placement. |
| `cells::CellCoord::new(x: i32, z: i32) -> Result<Self, CellError>` (bounded by `MAX_COORD` 512) | |
| `cells::CellBudget::new(bytes: u64, jobs: u32, frame_bytes: u64) -> Result<Self, CellError>` | Per-frame budget. |
| `cells::bake_cell(seed, CellCoord, size: u16) -> Result<Vec<u8>, CellError>`; `parse_cell(&[u8]) -> Result<CellData, CellError>`; `seam_joins(&CellData, &CellData) -> Result<(), CellError>` | Cell magic `b"WCD1"`, version 1; sizes `CELL_MIN_SIZE` 4 to `CELL_MAX_SIZE` 64; `CELL_HALO` 8; `SEAM_TOL` 72; `SLOPE_LIMIT` 64. |
| `cells::Cells::{new(seed, CellBudget), stream(...), publish(frame_bytes) -> Vec<CellEvent>, cancel(&CellCoord), evict(&CellCoord), repair(...), route(...), sim_hash(), used_bytes(), pending_count(), live_count()}` | Streaming with a byte cap (`CELL_MAX_BYTES` 64 MiB), pending cap 1024, admitted cap 4096. Publishes at frame boundaries only. |

Example, adapted from `bake_sha_x2` (`crates/world/tests/terrain_bake.rs`):

```rust
let a = bake_terrain(0x12345678, 32).unwrap();
let b = bake_terrain(0x12345678, 32).unwrap();
assert_eq!(a, b);
assert_ne!(a, bake_terrain(0x12345679, 32).unwrap());
```

Test: `node tools/gh-check.mjs cargo test -p engine-world --lib`, plus `--test terrain_bake` and `--test cell_streaming`.

## 7. `engine-save` (`crates/save`)

Versioned envelope, 16 slots, atomic write. Storage only: the game owns its progression payload bytes; `save`
never imports game or arena code. Integrity is CRC32 (not cryptographic).

| Item | Value or signature |
|---|---|
| `MAGIC` `b"E204"`; `VERSION` 1; `SLOTS` 16; `SUPPORTED_VERSIONS` `[1]` | |
| `MAX_PAYLOAD_BYTES` 1 048 576; `MAX_TOTAL_BYTES` = 29 + payload | Envelope is 29 bytes: magic 4, version 1, seed 8, tick 8, length 4, CRC32 4. |
| `seal(version: u8, seed: u64, tick: u64, payload: &[u8]) -> Result<Vec<u8>, SaveErr>` | Hashes before seal, seals after. |
| `open(&[u8]) -> Result<Envelope, SaveErr>`; `verify(&[u8]) -> Result<(u8, u64, u64, usize), SaveErr>`; `migrate(&Envelope)`; `migrate_owned(Envelope)` | `Envelope { version, seed, tick, payload }`. |
| `write_atomic(dir: &Path, slot: u8, bytes: &[u8]) -> io::Result<()>`; `save_generation(dir, slot, seed, tick, payload) -> io::Result<()>`; `slot_name(slot) -> Result<String, SaveErr>` | Temp file then rename. Previous generation kept for fallback. |
| `fallback::load_slot(dir, slot, tick, &mut RingBuffer) -> Result<Loaded, FallbackErr>`; `LoadSource`; `has_slot(dir, slot)`; `prev_name(slot)` | Falls back to the previous generation and reports through `observe`. |
| `SaveErr`: `BadMagic`, `BadVersion`, `Truncated`, `CrcMismatch`, `BadSlot`, `TooLarge` | |
| `persist::{seal_spine(seed, tick, &[u8]), open_spine(&[u8]) -> Result<PersistedSpine, PersistErr>, migrate_spine}`; `PERSIST_HEADER_LEN` 21 | Spine bytes. |
| `rollback::{RollbackRing::new(cap), push(tick, &[u8]), get(tick), rollback_to(tick), latest_tick()}`; `encode_frame`, `decode_frame`; `MAX_ROLLBACK_FRAMES` 128; `MAX_SPINE_BYTES` 65 536 | Ring of spine frames. `RollbackErr` on overflow or unknown tick. |

Example, adapted from `seal_open_roundtrip_x2` (`crates/save/src/lib.rs`):

```rust
let a = seal(1, 2040, 600, b"payload").expect("valid payload seals");
assert_eq!(a, seal(1, 2040, 600, b"payload").unwrap());   // deterministic bytes
let env = open(&a).expect("roundtrip opens");
assert_eq!((env.seed, env.tick), (2040, 600));
```

Test: `node tools/gh-check.mjs cargo test -p engine-save --lib` (`seal_open_roundtrip_x2`, `atomic_roundtrip_tmp`).
CRC and round-trip tests are not crash-durability evidence (`ARCHITECTURE.md`).

## 8. `engine-observe` (`crates/observe`)

Typed diagnostic events. Consumers: agents, the MCP `log.digest` and `log.query` verbs, crash bundles.
Events never enter a simulation hash. Simulation state must never be derived from them.

| Item | Signature and behaviour |
|---|---|
| `EventCode(u32)`: 18 codes, `SIM_TICK` 1, `PHYSICS_BODY_NONFINITE` 2, `BRAIN_FIGHTER_STUCK` 3, `NET_CLIENT_DESYNC` 4, `RENDER_DEVICE_LOST` 5, `RENDER_PIPELINE_FAILED` 6, `ASSETS_MISSING` 7, `PROVIDER_TIMEOUT` 8, `PROVIDER_MALFORMED` 9, `COMMANDS_STALE` 10, `EDITOR_CRASH` 11, `SAVE_CORRUPT` 12, `RENDER_FRAME_OVER_BUDGET` 13, `HEAL_APPLIED` 14, `HEAL_FAILED` 15, `FPS_POSE` 16, `FPS_HIT` 17, `FPS_FRAME_SPLIT` 18 | `raw()`, `name() -> Option<&'static str>`, `from_raw(u32) -> Result<Self, ObserveError>`; free `name(u32)`. Unknown codes fail. |
| `Event::new(code, tick: u64, &str) -> Self`; `try_new(raw: u32, tick, &str) -> Result<Self, ObserveError>` | `MAX_MESSAGE_LEN` 256. |
| `RingBuffer::new(cap)`; `push(Event)`; `iter()`; `len()`; `capacity()`; `overflow_count() -> u64`; `query(Option<EventCode>) -> Vec<&Event>`; `push_pose`, `push_hit`, `push_split` | Full ring evicts the oldest and counts it. Zero capacity counts every push as overflow. |
| `digest(&[Event], budget_bytes: usize) -> String` | Sorted by count descending, then code ascending: input order never changes the text. |
| `event_queue::EventQueue::new(queue_cap, tape_cap)`; `send(band: u8, Event) -> Result<u64, QueueError>`; `send_auto`; `begin_flush`; `pop_dispatch`; `drain_into_ring`; `tape_digest()`; `HandlerSet::{register, unregister, matching}` | Bands: `BAND_AUTHORITY` 0, `BAND_SERVICE` 1, `BAND_PRESENTATION` 2. |
| `heal` (bounded MAPE-K auto-heal registry), `supervisor::Supervisor` (reconnect with backoff, tick-derived), `fps` (`FpsPose`, `FpsHit`, `FpsFrameSplit` with `MAX_FPS_POSES` 240, `MAX_FPS_HITS` 64, `MAX_FPS_SPLITS` 120, `FPS_COORD_BOUND_MM` 1 000 000), `rerun_sink` | `fps` bounds use integer mm and ddeg. |

Example, adapted from `overflow_counter_increments_on_over_cap_push` (`crates/observe/src/lib.rs`):

```rust
let mut buf = RingBuffer::new(2);
buf.push(Event::new(EventCode::SIM_TICK, 1, "a"));
buf.push(Event::new(EventCode::SIM_TICK, 2, "b"));
buf.push(Event::new(EventCode::SIM_TICK, 3, "c"));   // evicts "a"
assert_eq!(buf.overflow_count(), 1);
```

Test: `node tools/gh-check.mjs cargo test -p engine-observe --lib`.

## 9. `engine-ui` (`crates/ui`)

HUD data lane. The renderer draws from snapshots. Every update is a pure function on an owned struct.
Permille for health and stamina, sRGB `[u8;3]` for palette colours, fixed steps for the kill feed.

| Item | Signature and behaviour |
|---|---|
| `Hud::new(tick: u64, hp: u32, max_hp: u32, ammo: u32, score: u64) -> Self` | Fields: `fighters: Vec<FighterHud>` (insertion order), `feed: KillFeed`, `map: MinimapLite`. |
| `upsert_fighter(FighterHud) -> Result<(), HudError>`; `fighter(id) -> Option<FighterHud>`; `set_hp(id, hp_permille: u16)`; `set_stamina(id, u16)`; `chip(shown: u32) -> u32` | Errors: `UnknownFighter(u8)`, `TooManyFighters(usize)`. |
| `push_kill(KillEntry)`; `upsert_dot(id, x_m: f32, z_m: f32)`; `state_hash() -> u64` | `state_hash` is FNV-1a 64 over integers (gallery and accessibility gate). |
| `KillFeed::new(cap) -> Result<Self, HudError>` (`DEFAULT_FEED_CAP` 8, `MAX_FEED_CAP` 64); `push`, `oldest`, `latest`, `iter`, `clear` | Oldest-first. `FeedCapOutOfRange(usize)`. |
| `MinimapLite::new(cap) -> Result<Self, HudError>` (`DEFAULT_MAP_CAP` 8, `MAX_MAP_CAP` 16); `upsert(id, x_m, z_m)`; `remove(id)`; `quantize(value_m: f32) -> i16` | `CELLS_PER_METER` 16, `MAP_HALF_METERS` 64. `MapFull`, `NonFiniteCoordinate`. |
| `contrast_ratio(a: [u8;3], b: [u8;3]) -> f32`; `MIN_CONTRAST` 3.0; `Palette` | `LowContrast { ratio_permille }` refuses low-contrast palettes. |
| `hp_pips(hp_permille: u16) -> u8` (`MAX_HP_PIPS` 10); `DuelOverlay`, `OverlayRow`, `OVERLAY_VERSION` 1, `MAX_OVERLAY_ROWS` 16, `OVERLAY_MAX_BYTES` | Overlay wire form, versioned. |
| `Juice` (boss bar, combo, trauma 0..100); `FLASH_TICKS` 30 | Presentation counters with fixed-point integers. |
| Modules | `menus` (main and pause state machine), `settings`, `lobby_ui` (LAN host list, ready, start), `text` (IME commit), `locale`, `access` (accessibility tree), `tutorial`, `play_hud`. |

`HudError` variants: `UnknownFighter(u8)`, `TooManyFighters(usize)`, `FeedCapOutOfRange(usize)`,
`MapCapOutOfRange(usize)`, `MapFull`, `NonFiniteCoordinate`, `LowContrast { ratio_permille: u32 }`.

Example, adapted from `hud_hash_x2` (`crates/ui/src/lib.rs`):

```rust
let a = Hud::new(7, 80, 100, 30, 1500);
let b = Hud::new(7, 80, 100, 30, 1500);
assert_eq!(a.state_hash(), b.state_hash());
```

Test: `node tools/gh-check.mjs cargo test -p engine-ui --lib` (`chip_lags_damage`, `hud_hash_x2`) and `--test ime_commit`.

## 10. `engine-audio` (`crates/audio`)

Offline synth, cue bank, headless voice mixer, spatial mix. Presentation only; the crate sits outside the
simulation. The device path (cpal) is behind feature `device`.

| Item | Signature and behaviour |
|---|---|
| `Pcm::new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Self`; `frames()`; `is_empty()`; `mono_at(frame)`; `stereo_at(frame)` | |
| `CueBank::new(Vec<Cue>) -> Result<Self, CueError>`; `build(sample_rate, load: impl FnMut(&'static str) -> Option<Pcm>) -> Result<(Self, LoadReport), CueError>`; `placeholder(sample_rate) -> Result<Self, CueError>`; `id(&str) -> Option<CueId>`; `cue(CueId)`; `names()` | Named cues. `CueId(pub u16)`. Max `u16::MAX` cues. |
| `feed(capacity, Arc<CueBank>) -> (FeedTx, FeedRx)` | Lock-free ring from the game thread to the audio thread. `FEED_CAPACITY`. |
| `FeedTx::send(Command) -> Result<(), FeedError>`; `play(tick, cue: &str, gain: f32, pos_cm: Option<[i32;3]>)`; `listener(tick, pos_cm, yaw_deg)`; `stop(tick, cue)`; `tension(tick, Tension)`; `stinger(tick, name)`; `stats()`; `is_closed()` | `FeedError::{Full, Closed, UnknownCue}`. Full ring counts drops; order survives the wrap. |
| `AudioEngine::with_feed(bank, capacity, sample_rate, seed) -> (FeedTx, Self)`; `drain() -> usize` (at most `DRAIN_LIMIT` per call); `mixer()`, `mixer_mut()`, `music_state()`, `applied()` | Drains on the audio callback. No lock, no allocation. |
| Constants | `MAX_VOICES` 64; `MAX_GAIN` 2.0; `GAIN_MASTER_DB` 0, `GAIN_MUSIC_DB` -12, `GAIN_SFX_DB` -3, `GAIN_DIALOGUE_DB` -6; `DUCK_*`, `STEAL_FADE_MS` 8. |
| `spatial::{SpatialMixer, MixCmd, Pos2, attenuation, falloff_gain, pan_for, NEAR_CM}`; `combat_cues`, `crowd::{Crowd, FightCue, Reaction}`, `music::{Tension, TensionTracker}`, `hrtf`, `reverb`, `occlude` | Positional mix, crowd and fight reactions, HRIR pair, tails, wall muffle. |
| `heal::{DeviceHeal, DeviceState, HealReport}`; `DEVICE_FAULT_CODE` | Device loss reconnects through `observe::supervisor`; the silent mixer keeps rendering. |

Example, adapted from `full_ring_refuses_counts_and_recovers_in_order` (`crates/audio/src/feed.rs`):

```rust
let (mut tx, mut rx) = feed(4, bank());
for t in 0..4 { tx.send(play(t)).expect("room"); }
assert_eq!(tx.send(play(99)), Err(FeedError::Full));
assert_eq!(tx.stats().dropped_full, 1);
assert_eq!(rx.pop().map(|c| c.tick()), Some(0));
```

Test: `node tools/gh-check.mjs cargo test -p engine-audio --lib`; device path: `--features device` (Windows).

## 11. `engine-app` (`crates/app`)

Composition lifecycle: builder, registration, phase order, extraction. Startup failures leave the app
unstarted and retryable, never half-started.

| Item | Signature and behaviour |
|---|---|
| `AppBuilder::new()`; `with_config(AppConfig)`; `scene_doc(Scene)`; `with_platform(impl Platform)`; `register(name, SystemKind, Phase, FnMut(&mut FrameCtx)) -> Result<Self, AppError>`; `build() -> Result<App, AppError>`; `startup() -> Result<App, AppError>` | Wrong phase or duplicate names refuse at registration. |
| `App::{startup(&mut self) -> Result<StartupReceipt, AppError>, run_frame(frame_dt: f32) -> Result<FrameReceipt, AppError>, shutdown() -> Result<ShutdownReceipt, AppError>, load_scene(&Scene), spawn_worker(name, cost_ms), frame(), sim_ticks(), journal(), app_hash(), last_extraction(), system_count(), is_running()}` | `frame_dt` must be finite and non-negative. Calls after `shutdown` refuse. |
| `SystemKind::{Setup, Poll, MatchStep, Extract, Present}` | Homes: Startup, PreUpdate, Update, PostUpdate (Extract before Present). |
| `AppConfig { seed: u64, tick_hz: u64, quality: QualityTier (Low, Medium, High), worker_deadline_ms: u64, journal_capacity: usize }`; `new(seed)`; `validate() -> Result<(), AppError>`; `core()`; `dt()` | `MIN_JOURNAL` 16, `MAX_JOURNAL` 4096, `MAX_DEADLINE_MS` 30 000. |
| `AppError` (variants per step, e.g. `NotStarted { what }`, `AlreadyShutdown { what }`); `hint() -> &'static str` | Each error carries a recovery hint. |
| `Journal::new(cap)`; `record(step, String) -> Result<(), AppError>`; `seal()`; `is_sealed()`; `entries()`; `steps()` | Bounded, sealable startup record. `JournalEntry`. |
| `Platform` trait: `name()`, `overlay_available()`, `unlock_achievement(id) -> Result<bool, PlatformError>`, `record_stat(name, value: i64) -> Result<(), PlatformError>`; `LobbyId(pub u64)`; `WorkshopId(pub u64)`; `PlatformError` | Platform services sit behind a trait. `MAX_CLOUD_BYTES` 1 048 576, `MAX_CLOUD_SLOTS` 8, `MAX_LOBBY_USERS` 64. |
| `live_proof::{LiveProof::of(..), attach(now_ms), heartbeat(now_ms), live_at(now_ms)}`; `Lease::take(name, holder, requested_ms)`; `clamp_lease`; `PROOF_TTL_MS` 60 000; `HEARTBEAT_MS` 2000; `LEASE_MIN_MS` 1000; `LEASE_MAX_MS` 60 000 | Lease handling for live proofs. |
| `snapshot::Extraction` | Snapshot published to render, audio and UI each frame. |

Example, adapted from `startup_then_frames_run_phases_in_plan_order` (`crates/app/src/tests.rs`):

```rust
let mut app = standard_builder(&log).register("init", SystemKind::Setup,
    Phase::Startup, logged(&log, "init")).unwrap().startup().unwrap();
assert_eq!(*log.borrow(), vec!["init"]);
app.run_frame(1.0 / 60.0).unwrap();
assert_eq!(*log.borrow(), vec!["init", "poll", "sim", "extract"]);
```

Test: `node tools/gh-check.mjs cargo test -p engine-app --lib` (`post_update_extract_runs_before_present_despite_reverse_registration`,
`duplicate_registration_refuses_and_keeps_the_first`, `kit_shape_matches_rc4_fixture`).

## Where the code is

- `crates/core/src/fixed.rs`, `det_math.rs`, `quat.rs`, `vec3.rs`: `Fixed`, deterministic trig and exp, quaternion product, vectors.
- `crates/core/src/hash.rs`, `digest.rs`, `adler.rs`, `rng.rs`, `counter_rng.rs`, `tick.rs`, `time.rs`, `config.rs`, `checked.rs`, `arena.rs`, `seal_key.rs`, `alloc_probe.rs`: hashes, streams, tick and clock, config, narrowing, slice arena, seal keys, probe.
- `crates/ecs/src/archetype.rs`, `schedule.rs`, `arena.rs`, `pool.rs`, `jobs.rs`, `lib.rs`, `spine.rs`, `spine_schedule.rs`: entities and archetype world, phases, phase arena, job pool, legacy `World`, spine systems.
- `crates/geo/src/mesh.rs`, `ops.rs`, `primitives.rs`, `bonemap.rs`, `recipe.rs`, `bake.rs`: `GeoMesh`, operators, shapes, bone map, recipes.
- `crates/geo/src/boolean.rs`, `lod.rs`, `collision.rs`, `uv.rs`: feature-gated donors (manifold, meshopt, parry3d, xatlas).
- `crates/spatial/src/lib.rs`, `checks.rs`: `Actor`, `Finding`, `Report`, `run`, `run_checks`, `run_spatial`, catalog.
- `crates/spatial/src/brief.rs`, `relation.rs`, `diff.rs`, `staged.rs`, `place.rs`, `repair.rs`, and the query modules (`sight`, `exposure`, `isovist`, `fps_visibility`, `afford`, `navcheck`, `index`, `kdtree`, `sample`, `scale`, `tag_lift`, `fuzz`).
- `crates/scene/src/codec.rs`, `ids.rs`, `transform.rs`: `Scene` codec, `ObjectId`, `Transform`, `Hierarchy`.
- `crates/scene/src/component.rs`, `schema.rs`, `pin.rs`, `prefab.rs`, `provenance.rs`: component kinds, schema registry, pins, prefabs, provenance.
- `crates/world/src/terrain.rs`, `ground/`, `scatter.rs`, `cells.rs`, `roster.rs`: terrain and ground bakes, Poisson placement, cell streaming, seeded spawn rosters.
- `crates/save/src/lib.rs`, `fallback.rs`, `persist.rs`, `rollback.rs`: envelope and slots, previous-generation fallback, spine payload, spine ring.
- `crates/observe/src/lib.rs`, `event_queue.rs`, `heal.rs`, `supervisor.rs`, `fps.rs`, `rerun_sink.rs`: events, banded queue, heal, supervisor, FPS records.
- `crates/ui/src/lib.rs`, `menus.rs`, `settings.rs`, `lobby_ui.rs`, `text.rs`, `locale.rs`, `access.rs`, `tutorial.rs`, `play_hud.rs`: HUD data, screens, accessibility.
- `crates/audio/src/feed.rs`, `engine.rs`, `cue.rs`, `bank.rs`, `mix.rs`, `spatial.rs`, `hrtf.rs`, `reverb.rs`, `occlude.rs`, `crowd.rs`, `music.rs`, `combat_cues.rs`, `heal.rs`, `device.rs`: feed, engine, cues, mixer, spatial, tails, crowd, music, device.
- `crates/app/src/app.rs`, `config.rs`, `error.rs`, `journal.rs`, `platform.rs`, `live_proof.rs`, `snapshot.rs`: `AppBuilder`, `App`, `SystemKind`, config, errors, journal, platform trait, leases, extraction.
- `docs/HASH-LAW.md`, `docs/ARCHITECTURE.md`: determinism contract and layering.
