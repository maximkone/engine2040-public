# Reference: destruction (`engine-destruction`)

Crate `crates/destruction` (package `engine-destruction`). Std-only, depends on `engine-core` alone
(`crates/destruction/Cargo.toml`). No cargo features. Offline bake produces chunk data; the runtime
keeps bonds, strain and anchors, emits typed break events, and hashes the moving state. Read
`docs/HASH-LAW.md` (the arena checkpoint folds these events) before changing any hashed field.

Test: `node tools/gh-check.mjs cargo test -p engine-destruction --lib`

## 1. Pipeline

```text
offline:  PropMesh + CutPlane[] --csg::bake--> FractureChunk[] --to_bonded_desc--> BondedDesc
          PropMesh + FractureSpec --fracture_at / FractureCache::bake--> Fracture (to_desc)
          box (x, y, z, half, pieces, seed) --BondedCollection::crate_prop--> BondedCollection
runtime:  apply_impact / add_strain / sever_chunk  ->  step(dt)  ->  drain_events() -> Vec<BreakEvent>
consumers: arena fold (crates/arena/src/sync.rs:368-373), game/src/break_look.rs, endpoint packs/break_.rs
```

| Module | Role | Consumers today |
|---|---|---|
| `bonded` (+ `bonded/snapshot.rs`) | bonded collection, bond strain, anchors, load leg, break log | arena `lib.rs:1173, 1628, 1635`, `spine_step.rs:140, 461, 468`, `fork.rs:43`; endpoint `break_.rs:119-121`; game `break_look.rs` (test fixtures only, 571-578, 762) |
| `csg` | blade cuts, boolean brushes, offline chunk bake | none outside the crate's tests |
| `fracture` | Voronoi fracture at a hit, cook cache | endpoint `break_.rs:252` (`fracture_at`) |
| `bake`, `bake3d`, `shards`, `shards3d` | 2D and 3D box shards; `in_contact` for bonds | `lib.rs` root re-exports |
| `material` | 14-row Q16 material table | `crates/material` (`set.rs`, `lib.rs`) |
| `budget` (+ `budget/snapshot.rs`) | debris ledger, caps, sleep, LOD | production: none (see section 9) |
| `snapshot` | one canonical blob for collections plus ledger | arena `limbs.rs:123-175` |
| `crack`, `spread` | siege-wall cache plus crack spread on the bonded path | none outside the crate's tests |
| `structure` | load-path analysis, building fixtures, oracle wall | endpoint `break_.rs:119-121` (`building`), arena tests |
| `cluster`, `shape_match` | bond topology helpers; opt-in shape matching (never in the bonded tick) | none outside the crate |

## 2. Break events

`BreakEvent` (`bonded.rs:129`):

| Variant | Fields | Emitted when |
|---|---|---|
| `BondBroken` | `{ tick: u32, bond: u32, strain: f32 }` | a bond's strain passes its `breaking_strain` (`apply_impact`, `add_strain`, `sever_chunk`) |
| `ChunkFreed` | `{ tick: u32, chunk: u32 }` | a chunk loses its last path to an anchor (emitted by `refresh_support`, which runs after a snap and on every `step`) |

Order: within one `apply_impact` the snaps come in bond index order; then the freed chunks in index
order. `sever_chunk` snaps neighbours in ascending neighbour order, then frees (`bonded.rs:731-745`).
`event_bytes(&BreakEvent) -> [u8; 13]` (`bonded.rs:150`): tag byte (0 bond, 1 chunk), tick LE, id LE,
then strain bits for bonds or zeros for chunks. The arena folds these bytes (section 8).

## 3. Bonded collection (`bonded.rs`, runtime)

### 3.1 Descriptor and limits

| Item | Rule | Constant |
|---|---|---|
| `Chunk3` | `{ x, y, z, hx, hy, hz, mass: f32 }` (centre, half-extents, mass) | `bake3d.rs:7` |
| `Bond` | `{ a: u32, b: u32, breaking_strain: f32 }`; `a != b`; ends below the chunk count; strain finite in `[0, 1e6]` | `MAX_BOND_STRAIN = 1e6`, `DEFAULT_BOND_STRAIN = 1.0` |
| `BondedDesc` | `{ chunks, bonds, anchors, break_threshold }`; `fn validate(&self) -> Result<(), BreakError>` | at most 1024 chunks, 4096 bonds |
| threshold | finite, `(0, 1e6]` | `MAX_BREAK_THRESHOLD = 1e6` |
| anchors | indices below the chunk count; fixed in place, never freed or integrated | `NO_PARENT = u32::MAX` marks an anchor's load parent |

Chunk geometry checks (`check_chunks`) bound centres to 10 000 m, half-extents to `[1e-4, 1e4]` m,
mass to `[1e-6, 1e12]` (`bonded.rs:36-42`). `BondedDesc::validate` also rejects duplicate bond pairs.

### 3.2 Runtime API

| Method | Signature | Units / rule | Error |
|---|---|---|---|
| `new` | `fn new(desc: &BondedDesc) -> Result<Self, BreakError>` | chunks start supported and asleep; loads and parents computed at once | `BreakError` from `validate` |
| `crate_prop` | `fn crate_prop(x, y, z, half: f32, pieces: usize, seed: u64) -> Result<Self, BreakError>` | `pieces` in `1..=64`; contact bonds at `DEFAULT_BOND_STRAIN`; bottom layer anchored; threshold `BREAKING_THRESHOLD = 3.0` (`bake.rs:173`) | `EmptyChunks`, `TooManyChunks`, `BadChunk(0)` for a bad box |
| `apply_impact` | `fn apply_impact(&mut self, chunk: usize, impulse: f32, tick: u32) -> Result<(), BreakError>` | impulse `<= mass * threshold` is a no-op; above it every intact bond on the chunk gains `impulse / mean mass` strain | `BadChunk`, `BadImpulse` (negative or non-finite) |
| `add_strain` | `fn add_strain(&mut self, bond: usize, amount: f32, tick: u32) -> Result<(), BreakError>` | strain on a broken bond is a no-op | `BadBond`, `BadStrain` |
| `sever_chunk` | `fn sever_chunk(&mut self, chunk: usize, tick: u32) -> Result<u32, BreakError>` | snaps every intact bond of the chunk (strain set to limit + 1); returns how many | `BadChunk` |
| `step` | `fn step(&mut self, dt: f32)` | `dt` in `(0, 1]`, else ignored; `tick += 1`; refresh support; capacity rule; integrate freed chunks under `g = 9.8`, ground clamp at `y = hy` | silent |
| `drain_events` / `drain_into` | `fn drain_events(&mut self) -> Vec<BreakEvent>`; `fn drain_into(&mut self, out: &mut Vec<BreakEvent>)` | emission order; draining never moves the hash | none |
| `state_hash` | `fn state_hash(&self) -> u64` (section 6) | | |
| readers | `len`, `tick`, `break_threshold`, `chunk_mass(i) -> Option<f32>`, `chunks`, `bonds`, `velocities`, `anchors`, `freed_chunks`, `broken_bonds`, `is_anchored`, `is_bond_broken`, `strain_of`, `bond_count`, `is_supported` | | |
| load leg | `weakest_link() -> Option<u32>`; `predict_removal(chunk)`, `predict_collapse(chunk) -> Result<Vec<u32>, BreakError>`; `load_ratio_q16(chunk) -> Option<u64>`, `max_load_ratio_q16() -> u64`; `capacity_q16() -> u64`, `set_capacity_q16(&mut self, u64) -> u64` (0 = rule off); `is_overloaded(chunk) -> bool` | Q16 (`LOAD_Q16_ONE = 65536`) | `BadChunk` on the `Result` predictions only |

Example (from `sword_hit_breaks_baked_chunks_through_dx1`, `csg.rs:1154`, and `impact_script_replays_identical_break_sets`, `bonded.rs:1112`):

```rust
let mut field = BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7)?;
let top = /* index of the highest chunk, lowest index on a tie */;
let impulse = field.chunk_mass(top).expect("mass") * field.break_threshold() + 1.0;
field.apply_impact(top, impulse, 3)?;
for _ in 0..30 { field.step(1.0 / 60.0); }
let events = field.drain_events();   // BondBroken rows, then ChunkFreed rows
```

Test: `node tools/gh-check.mjs cargo test -p engine-destruction --lib bonded::`. The predicted set of a
sever is tested in `sever_chunk_frees_exactly_the_predicted_set` (`bonded.rs:1617`).

### 3.3 Support, load and collapse

`refresh_support` floods from the anchors along intact bonds. A chunk not reached is dynamic and frees
on that step. Each supported chunk gets a parent in the layer above it and a Q16 load: its own mass plus
everything hanging below it in the tree (doc at `bonded.rs:218-221`). `fail_overloaded` runs when a
capacity is set: one wave per tick, time-sliced (`capacity_rule_fails_one_wave_per_tick_and_matches_the_prediction`,
`bonded.rs:1666`; `capacity_waves_are_time_sliced`, `bonded.rs:1752`).

## 4. Cuts and baked chunks (`csg.rs`, `fracture.rs`)

### 4.1 Blade cuts and booleans (`csg.rs`)

| Item | Signature | Units / range | Error |
|---|---|---|---|
| `CutPlane` | `{ normal: [f32;3], dist: f32 }` (`normal . p = dist`) | normal need not be unit; `|normal| > 1e-9` | `DegeneratePlane(i)` |
| `PropMesh` | `{ positions: Vec<[f32;3]>, indices: Vec<u32> }` | 1 to 65536 vertices, whole triangles, coords within 10 000 m | `EmptyMesh`, `BadVertex`, `BadIndex`, `BadTopology`, `TooLarge` |
| `blade_plane` | `fn blade_plane(point: &[f32;3], blade_dir: &[f32;3], swing_dir: &[f32;3]) -> Result<CutPlane, CsgError>` | normal = `blade x swing`, normalized | `BadBlade` (non-finite or parallel) |
| `knife` | `fn knife(mesh: &PropMesh, plane: &CutPlane) -> Result<(PropMesh, PropMesh), CsgError>` | both sides must be non-empty | `DegenerateCut(0)` |
| `brush_subtract` / `brush_intersect` | `fn brush_subtract(mesh: &PropMesh, brush: &[CutPlane]) -> Result<PropMesh, CsgError>` (and `brush_intersect`) | up to 64 planes (`MAX_CUT_PLANES`) | `CsgError` (`TooManyPlanes`, `EmptyResult` when the boolean removes everything) |
| `shell_union` | `fn shell_union(a: &PropMesh, b: &PropMesh) -> Result<PropMesh, CsgError>` | concatenates disjoint shells | `CsgError` (size bounds) |
| `bake` | `fn bake(mesh: &PropMesh, cuts: &[CutPlane]) -> Result<Vec<FractureChunk>, CsgError>` | at most 1024 chunks and 262144 live triangles | `NoCuts`, `TooManyChunks(n)`, `DegenerateCut(i)` |
| `to_bonded_desc` | `fn to_bonded_desc(chunks: &[FractureChunk], breaking_strain: f32, break_threshold: f32) -> Result<BondedDesc, BreakError>` | contact bonds; bottom layer anchored within `1e-3` m | `BreakError` |
| `csg_bake_bytes` / `csg_bake_digest` | `fn csg_bake_bytes(&[FractureChunk]) -> Vec<u8>`; `fn csg_bake_digest(&[FractureChunk]) -> u64` | the bake's pinned bytes | none |

`FractureChunk { name: String, mesh: PropMesh, min, max: [f32;3] }`; `to_chunk3()` gives the box with
slabs under `MIN_SLAB_M = 1e-4` m inflated to that size. `frac_name(i)` gives `FRAC_0000`-style names.

Example (adapted from `sword_hit_breaks_baked_chunks_through_dx1`, `csg.rs:1154`, with the blade
`(point, blade_dir, swing_dir) = ([0,0,0], [1,0,0], [0,0,1])`, which gives normal `(0, -1, 0)`):

```rust
let plane = blade_plane(&[0.0, 0.0, 0.0], &[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0])?;
let chunks = bake(&mesh, &[plane])?;
let desc = to_bonded_desc(&chunks, 1.0, BREAKING_THRESHOLD)?;
let mut field = BondedCollection::new(&desc)?;
```

Test: `blade_cut_splits_cube_identical_twice` (`csg.rs:1023`), `two_perpendicular_cuts_make_four_chunks` (`csg.rs:1050`),
`degenerate_inputs_refused` (`csg.rs:1194`). Command: `--lib csg::`.

### 4.2 Voronoi fracture at a hit (`fracture.rs`)

| Item | Signature / rule | Error |
|---|---|---|
| `FractureSpec` | `{ cells: u32, radius_m: f32, seed: u64 }`; `validate`; cells `2..=64`; radius `(0, 1000]` m | `BadCells`, `BadRadius` |
| `fracture_at` | `fn fracture_at(mesh: &PropMesh, hit: [f32;3], spec: &FractureSpec) -> Result<Fracture, FractureError>` | `BadHit` (outside the shell box), `NoSplit`, `Csg`, `Bonded` |
| `Fracture` | `{ chunks: Vec<FractureChunk>, sites: Vec<[f32;3]>, hit_chunk: u32 }`; `to_desc(breaking_strain, break_threshold)`; `break_events(tick, breaking_strain, break_threshold) -> Result<Vec<BreakEvent>, FractureError>`; `bytes()`; `digest()` | |
| `FractureCache::bake` | `fn bake(mesh: &PropMesh, spec: &FractureSpec, grid: u32) -> Result<Self, FractureError>`; grid `1..=8` per axis | `BadGrid` |
| `cut` / `cold` / `key_of` | `fn cut(&self, hit) -> Result<&Fracture, FractureError>` (table read, no clip); `fn cold(&self, mesh, key: CacheKey) -> Result<Fracture, FractureError>` (clip, offline); `fn key_of(&self, hit) -> Result<CacheKey, FractureError>` | `BadHit` |
| `crack_strike` | `fn crack_strike(...)` on `FractureCache`, used by `CrackWall::strike` | `CrackError` |

`break_events` sends the hit through a bonded collection built from the pieces: `sever_chunk(hit_chunk)`,
then `drain_events` (`fracture.rs:149-160`). Seeded sites: `scatter_sites_into` (`fracture.rs:189`),
with `NEAR_EIGHTHS = 5` of 8 sites near the hit. Example and test: `break_events_follow_the_dx1_shape_and_replay`
(`fracture.rs:1019`), `cache_hit_cut_is_byte_equal_to_the_cold_cut` (`fracture.rs:1044`). Command: `--lib fracture::`.

### 4.3 Dismemberment data

This crate has no dismember type. The severed-limb data lives elsewhere: combat `Cut` events
(`crates/combat/src/gore.rs` emits `GoreKind::DismemberLite`, `gore.rs:46`) become physics
`LimbSpawn` rows in arena `spawn_limbs` (`crates/arena/src/limbs.rs:82-97`, with `limb_of`). The
`ChunkFreed` path (bonded chunk to debris `ShardSeed`) is exercised in arena tests only (`limbs.rs:411-427`,
inside the `#[cfg(test)]` module at line 180).

## 5. Material table (`material.rs`)

`MaterialId` has 14 variants (`MATERIAL_COUNT = 14`). `MaterialRow` fields: `toughness_q16` (1 to
`64 << 16`, the graded bond's strain limit), `brittleness_q16` (0 dents, `65536` shatters), `shard:
ShardRule`, `sound`, `dust`, `spark`, `decal` (presentation only), `burn_q16` (0 never burns, cap `16 << 16`).

| Item | Signature | Error |
|---|---|---|
| `MaterialTable::siege_kit` | `pub const fn siege_kit() -> Self` (version `MATERIAL_VERSION_V1 = 1`) | |
| `validate` | `fn validate(self) -> Result<(), MaterialError>`; free `validate(table)` | `BadToughness{material,value}`, `BadBrittleness`, `BadBurn`, `BadVersion`, `UnknownVersion` (first failure in material order) |
| `digest` | `fn digest(self) -> u64` (`digest_rows`: version then rows, FNV) | |
| `breaking_strain_q16` / `is_combustible` / `row_of` | `fn breaking_strain_q16(self, id: MaterialId) -> u32`; `fn is_combustible(self, id) -> bool`; `fn row_of(self, id) -> MaterialRow` | |
| byte checks | `fn validate_bond_materials(ids: &[u8]) -> Result<(), MaterialError>`; `fn validate_chunk_materials(ids: &[u8])` | `BadBondMaterial{index,value}`, `BadChunkMaterial{index,value}`, `UnknownMaterial(u8)` |

Toughness runs from glass `0.5` to iron `32.0`; thatch and cloth burn fastest; stone and metal never
burn; glass shatters fully while iron dents (`material.rs:505-511`). `BondedDesc` has no material field:
the parallel byte arrays are checked with `validate_bond_materials` before use. Test:
`breaking_strain_orders_glass_below_timber_below_stone_below_iron` (`material.rs:1046`); `--lib material::`.
The material crate builds its record set from this table (`crates/material/src/set.rs:14-15, 220`).

## 6. Debris ledger (`budget.rs`)

Caps and sleep, integer mm and mm per tick. `DebrisBudget { per_event_cap: u32 (1..=64), max_shards: u32 (1..=2048),
sleep_ticks: u32 (1..=3600), lod_distance_mm: i32 (1..=10_000_000), sleep_speed_mm_per_tick: i32 (0..=100_000) }`.
`arena_default_budget()` is the 8-fighter soak: 16 per event, 1024 shards, 30 sleep ticks, LOD past 4 m, sleep speed 1.

| Item | Signature | Error |
|---|---|---|
| `DebrisBudget::validate` | `fn validate(&self) -> Result<(), BudgetError>` | `BadPerEventCap`, `BadMaxShards`, `BadSleepTicks`, `BadLodDistance`, `BadSleepSpeed` |
| `DebrisLedger::new` | `fn new(cfg: &DebrisBudget) -> Result<Self, BudgetError>` | as above |
| `admit` | `fn admit(&mut self, tick: u32, seeds: &[ShardSeed]) -> Result<AdmitReport, BudgetError>`: per-event cap keeps the lowest ids; the ledger cap refuses newcomers | seed and duplicate refusals (`budget.rs:136`) |
| `kick` | `fn kick(&mut self, id: u32, vel_mm_per_tick: [i32;3]) -> Result<(), BudgetError>` | |
| `step` | `fn step(&mut self, camera_mm: [i32;3])`: integrates awake shards, sleeps islands still for `sleep_ticks`, flips LOD past the distance | silent |
| readers | `len`, `tick`, `admitted_total`, `dropped_total`, `lod_swaps`, `sleeping_count`, `low_lod_count`, `sleep_ratio_q16`, `shard_at(i) -> Option<ShardView>`, `is_sleeping(id)`, `island_sleeping(island)` | |
| `state_hash` | `fn state_hash(&self) -> u64` (FNV over the canonical word stream) | |

Shards are kept in `(island, id)` order, so shuffled admit order hashes equal
(`shuffled_admit_orders_hash_equal`, `budget.rs:860`). Soak: `soak_8_fighters_300_ticks_holds_caps_and_hashes_twice`
(`budget.rs:892`). Command: `--lib budget::`.

## 7. Snapshots and rollback (`snapshot.rs`, `bonded/snapshot.rs`, `budget/snapshot.rs`)

| Item | Signature | Notes |
|---|---|---|
| `snapshot::capture` | `fn capture(collections: &[&BondedCollection], ledger: &DebrisLedger) -> Vec<u8>` | length-prefixed parts, canonical order |
| `snapshot::restore` | `fn restore(collections: &mut [&mut BondedCollection], ledger: &mut DebrisLedger, bytes: &[u8]) -> Result<(), SnapshotError>` | atomic; at most 64 collections (`MAX_COLLECTIONS`) |
| `snapshot::digest` | `fn digest(bytes: &[u8]) -> u64` | FNV over the bytes |
| `BondedCollection::snapshot_bytes` / `restore_bytes` | `fn snapshot_bytes(&self) -> Vec<u8>`; `fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), BreakError>` | `BONDED_SNAPSHOT_V1 = 1`; bit-exact f32; the pending break log is part of the state |
| `DebrisLedger::snapshot_bytes` / `restore_bytes` / `from_snapshot_bytes` | `fn snapshot_bytes(&self) -> Vec<u8>`; `fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), BudgetError>`; `fn from_snapshot_bytes(bytes: &[u8]) -> Result<Self, BudgetError>` | `LEDGER_SNAPSHOT_V1 = 1`; refusals change nothing |

Arena's production wrapper (`crates/arena/src/limbs.rs:123-175`) concatenates these bytes with the
physics `LimbPool` snapshot. Test: `alloc_probe_sees_a_real_allocation` and friends in `bonded.rs:1442-1452`
hold the no-allocation claim for the steady refresh; the snapshot tests live beside each module.

## 8. Where the events and hashes go (arena)

| Fold | Where | What |
|---|---|---|
| destruction state | `crates/arena/src/sync.rs:368` | `sim.destruction.state_hash()` (floats as bits, index order) |
| break rows | `sync.rs:369-373` | the **last tick's** `BreakEvent` rows, each through `break_bytes` (= `event_bytes`) |
| driving calls | `crates/arena/src/lib.rs:1628, 1635`; `spine_step.rs:461, 468` | `apply_impact` per hit, `drain_events` once per tick |

Because these bytes reach the arena checkpoint, changing `state_hash`, `event_bytes` or the impulse rule moves
arena checkpoints. `GOLDEN_600` does not run arena or combat (HASH-LAW lines 14-18, 82-84), so it does not move.

## 9. Invariants and proofs

| Invariant | Rule | Proof |
|---|---|---|
| Same input, same events | impact script replays the same break set | `impact_script_replays_identical_break_sets` (`bonded.rs:1112`) |
| Bake digests are stable | two bakes, same bytes and digest | `sword_hit_breaks_baked_chunks_through_dx1` (`csg.rs:1154`, asserts `csg_bake_digest` twice) |
| Predicted set equals the sever | `predict_removal` matches `sever_chunk` | `sever_chunk_frees_exactly_the_predicted_set` (`bonded.rs:1617`) |
| Sub-threshold hits change nothing | impulse at or below `mass * threshold` | `sub_threshold_impact_changes_nothing` (`bonded.rs:1180`) |
| Hashing does not move on drain | draining the log leaves the hash | `drain_into_matches_drain_events_and_appends` (`bonded.rs:1392`) |
| Root API | 2D bake deterministic and sized | `root_bake_is_deterministic_and_sized` (`lib.rs:90`), `root_siege_kit_validates_and_digests_stably` (`lib.rs:111`) |

## 10. Known gaps (read from the code, not changed here)

- `spread.rs:3-4` says the arena apply path calls it, but the sentence is garbled ("calls in (no intra-crate
  caller yet)"). No caller exists outside the crate. `crack.rs:3` says the same for `CrackWall`.
- `budget.rs:3` says "the arena match admits bonded-collection break chunks per event". The only `admit`
  calls are in arena test code (`limbs.rs:430`, inside the test module). Production code never admits.
- No dismember type here; see section 4.3.
- `csg::knife` reports `DegenerateCut(0)` regardless of the plane (`csg.rs:401`). The index is a constant,
  not a position in a list.
- `shape_match` and `cluster` have no caller under `crates/` (grep).

## 11. Where the code is

| File | Role |
|---|---|
| `lib.rs` | module list, re-exports, root API tests |
| `bonded.rs`, `bonded/snapshot.rs` | runtime collection, events, load leg, sever, snapshot bytes |
| `csg.rs` | blade planes, knife, booleans, offline bake, bonded conversion |
| `fracture.rs` | Voronoi sites, `Fracture`, `FractureCache` |
| `bake.rs`, `bake3d.rs`, `shards.rs`, `shards3d.rs` | 2D and 3D box shards; `Chunk3`; `in_contact` |
| `material.rs` | 14-row table, validators, digest |
| `budget.rs`, `budget/snapshot.rs` | debris ledger, caps, sleep, LOD, ledger bytes |
| `snapshot.rs` | world snapshot: collections plus ledger |
| `crack.rs`, `spread.rs`, `cursor.rs` | siege-wall strike, Q16 crack spread, bounded byte reader |
| `structure.rs` | load analysis, `building`, `oracle_wall`, digests |
| `cluster.rs`, `shape_match.rs` | bond topology; opt-in shape matching |
| `Cargo.toml` | `engine-core` only |
