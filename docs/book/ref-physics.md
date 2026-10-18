# Reference: physics (`engine-physics`)

Crate `crates/physics` (package `engine-physics`). Read `docs/HASH-LAW.md` first. Units are meters,
seconds, Y-up and unit quaternions `[x, y, z, w]`, unless a name says `_mm` (integer millimeters),
`_q8` (impulse, 256 = unity) or `_q16` (`65536` = 1.0).

## 1. Features and modules

| Item | Value |
|---|---|
| Default build | std-only. |
| Feature `rapier` | adds `rapier3d` and `engine-anim`; gates `controller`, `powered`, `ragdoll`, `ragdoll_tape`, `clinch`, `rapier`, and the servo half of `balance`. |
| rapier3d | `default-features = false`, `["dim3", "f32", "std", "enhanced-determinism"]` (root `Cargo.toml:193`). No SIMD, no parallel. |
| Test (default) | `node tools/gh-check.mjs cargo test -p engine-physics --lib` |
| Test (rapier) | `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib` |

| Module | Gate | Number type | Hashed | Authority consumer |
|---|---|---|---|---|
| `lib` (`Space`), `rigid`, `overlap` | none | `f32` | yes (`rigid`), as f32 bits | `engine-proof` legacy golden (`crates/proof/src/lib.rs:13`) |
| `controller2` | none | `i32` mm, `i64` math | yes | `engine-arena` `collide.rs`, `lib.rs:1804` (spawn and separation only) |
| `controller` | `rapier` | `f32` over a Rapier mirror | yes, quantized | `engine-arena` `lib.rs:1549` (locomotion), `lib.rs:1643`, `sync.rs:376` (feature `sim`) |
| `kcc` | none | `f32` | partly (section 10) | none under `crates/` |
| `balance`, `stagger_memory` | none; servo under `rapier` | integer mm, Q8 | yes | `engine-arena` `lib.rs:269-271` (fold tags `0xB1`, `0xB2`) |
| `limb` | none | integer mm, mm/s, micro-rad | yes (FNV over snapshot) | `engine-arena` `limbs.rs`, tag `0xB3` |
| `material` | none | integer thousandths | no (table) | `engine-physfx` |
| `collider`, `query`, `layers` | none | `f32` in, `f64` math | no | `engine-arena` `kit_zone.rs`; `engine-endpoint` `packs/physics.rs` |
| `cloth` | none | `f32` | never | none under `crates/` |
| `clinch` | `rapier` | `f32` | never (client side) | `ragdoll_tape` clinch leg |
| `articulated`, `door`, `stance` | none | integer (Q8 mm, decidegrees, mm) | yes once wired | none yet |
| `islands` | none | `f32` words as bytes | via `rapier::World` | rapier snapshot |
| `rapier`, `ragdoll`, `powered`, `pd`, `ragdoll_tape` | `rapier` | `f32`, quantized hash | yes, except `ragdoll_tape` (digest only) | `engine-proof` `ragdoll_matrix.rs`; `engine-physfx` (`insert_joints`, `world_transforms`) |

## 2. Integer vs float: what is hashed

A float enters an authority hash only through `quantize` (`round(v * 1e6)` to `i64`, `rapier.rs:458`),
as raw f32 bits where the module says so, or not at all.

| Hash | Folds | Encoding |
|---|---|---|
| `rigid::state_hash`, `Space::state_hash` | x, y, vx, vy per body, index order | `f32::to_bits`, LE FNV-1a 64 |
| `rapier::World::state_hash` | pos, rot, linvel, angvel per dynamic body, spawn order | `quantize`, `write_i64` |
| `ragdoll` / `powered` `state_hash` | bone transforms, velocities, solver counters | `quantize` |
| `controller::Controller::state_hash` | Rapier mirror hash, then `vel_y`, `grounded`, `coyote_ms`, `buffer_ms`, `buffered_vel` | mirror plus quantized floats and integer ms |
| `controller2::state_hash` | id, `pos_mm`, `vel_mm_s`, `grounded`, `ground_platform`, timers | `i32` lanes |
| `kcc::state_hash` | pos, vel, `coyote_t`, `buffer_t`, `grounded` | f32 bits |
| `BalanceState::state_hash` | `stagger_q8`, centre of mass as **integer mm** (`com_mm`), stagger and gait phase with countdowns, tick, shove and fall counters, get-up flag | `Fnv64` |
| `StaggerMemory::state_hash` | pain, fatigue, level code, injury per bone, tick, hit count | `Fnv64` |
| `LimbPool::state_hash` | FNV over `snapshot()` bytes | integer only |
| `Cloth` | nothing: no hash, snapshot or restore | n/a |

HASH-LAW tier-3 consequences the code follows: the authority modules (`controller2`, `balance`,
`stagger_memory`, `limb`) use `+ - * /`, comparisons, `isqrt` and `round` only. The centre of
mass is hashed as integer mm so FMA or summation order on another CPU cannot split it
(`balance.rs:731-735`). Ragdoll state is never folded into the balance hash
(`balance_hash_equal_with_ragdoll_on_and_off`, `ragdoll.rs:1553`).

## 3. Rigid bodies (`lib.rs`, `rigid.rs`, `overlap.rs`)

| Item | Signature | Units / range | Determinism | Error |
|---|---|---|---|---|
| `Body` | `pub struct Body { pub x, y, vx, vy, hx, hy: f32 }`; `Body::new(x, y, vx, vy, hx, hy)` | m, m/s; ground at `y == hy` | plain data | none |
| `Space::step` | `fn step(&mut self, dt: f32)` | `dt` clamped to 0.25 s; `dt <= 0` or non-finite is a no-op | index order | silent |
| `Space::overlap` / `add` / `state_hash` | `fn overlap(&self, x, y, hx, hy: f32) -> Vec<usize>` (AABB, touching counts); `fn add(&mut self, b: Body) -> usize`; `fn state_hash(&self) -> u64` | | hashed | none |
| `rigid::step_body` / `integrate` / `snapshot` / `restore` | `fn step_body(b: &mut Body, dt: f32)`; `fn integrate(&mut [Body], dt: f32)` (`GRAVITY = 9.8`); `fn snapshot(&[Body]) -> Vec<Body>`; `fn restore(&mut [Body], &[Body])` | | same ops in both | silent |

Example (from `ground_clamp_holds_and_hash_stable`, `crates/physics/src/lib.rs:108`):

```rust
let mut s = Space::new();
s.add(Body { x: 0.0, y: 5.0, vx: 1.0, vy: 0.0, hx: 0.5, hy: 0.5 });
for _ in 0..600 { s.step(1.0 / 60.0); }
assert_eq!(s.bodies[0].y, s.bodies[0].hy); // resting on the y = 0 plane
```

Test: `node tools/gh-check.mjs cargo test -p engine-physics --lib ground_clamp`

## 4. Character controllers

### 4.1 `controller2` (integer, authority)

Walkers move in integer mm at 60 ticks/s (`TICKS_PER_S = 60`); std-only; no allocation on the tick.

| Item | Signature / value | Error |
|---|---|---|
| `Config` | `{ step_height_mm, snap_mm, max_slope_grade_q, accel_ground_mm_s2, accel_air_mm_s2, max_wish_mm_s, jump_v_mm_s: i32 }`, `Default` (500, 400, 1000 = 45 deg, 60000, 20000, 8000, 8000) | none |
| `Block::new` | `fn new(min_mm: [i32;3], max_mm: [i32;3]) -> Block` (orders lanes) | none |
| `StepWorld<'a>` | `{ blocks: &[Block], ramps: &[Ramp], platforms: &[Platform] }`, caller order | none |
| `Walker` / `Walker::spawn` | `{ id, pos_mm, vel_mm_s, grounded, ground_platform, coyote_ticks, buffer_ticks }`; `fn spawn(id: u32, pos_mm: [i32;3]) -> Walker` | none |
| `TickInput` | `{ wish_mm_s: [i32;2], jump_pressed: bool }`, `TickInput::idle()`; wish clamped to `MAX_WISH_MM_S = 8000` | clamped, never refused |
| `step_walker` | `fn step_walker(walker: &mut Walker, input: &TickInput, world: &StepWorld<'_>, cfg: &Config) -> Resolved` | never panics |
| `Resolved` | `{ displacement_mm: [i32;2], vertical_mm, grounded, wall_normal: Option<[i32;2]>, stepped, slipped, rode_platform: Option<u32> }` | |
| `separate_pair` | `fn separate_pair(a: &mut Walker, b: &mut Walker)`: pushes to `2 * RADIUS_MM`, canonical id order | none |
| `apply_shove` | `fn apply_shove(walker: &mut Walker, dir_mm: [i32;2], strength_mm_s: i32)`; strength capped at 8000 mm/s, `<= 0` no-op | silent |
| `classify` | `fn classify(height_mm, depth_mm, gap_mm, speed_mm_s: i32, has_hold: bool) -> Result<LedgeAction, ProbeError>` | `ProbeError::{BadProbe(&str), NoHold, GapTooWide{gap_mm,max_mm}}` |
| `sense_ahead` | `fn sense_ahead(walker: &Walker, dx_mm, dz_mm: i32, world: &StepWorld<'_>, cfg: &Config) -> AheadSense` | range 5000 mm, stride 100 mm | none |
| `duration_ticks` / `state_hash` | `fn duration_ticks(LedgeAction) -> u32` (Hurdle 20, Vault 30, Mantle 40, Climb 60, Drop 25); `fn state_hash(&Walker) -> u64` | | |

Example (from `push_and_shove_separate`, `controller2.rs:1059`):

```rust
let mut a = Walker::spawn(1, [0, 0, 0]);
let mut b = Walker::spawn(2, [200, 0, 0]);
separate_pair(&mut a, &mut b);
assert!(b.pos_mm[0] - a.pos_mm[0] >= 2 * RADIUS_MM);
```

Test: `node tools/gh-check.mjs cargo test -p engine-physics --lib controller2::`

### 4.2 `controller` (Rapier mirror, `f32`, feature `rapier`)

A capsule over `rapier::World`; the Rapier world only mirrors statics and the resolved centre, and
`move_and_slide` never steps Rapier. Constants: `MAX_SLOPE_GRADE 1.0`, `STEP_HEIGHT 0.5`,
`GROUND_SNAP_DISTANCE 0.4`, `MAX_WISH_PER_TICK 1.0`, `MAX_FALL_SPEED -20.0`, `SKIN 0.02`, `COYOTE_MS 150`, `BUFFER_MS 150`.

| Item | Signature | Error |
|---|---|---|
| `Controller::spawn` | `fn spawn(pos: [f32;3], radius: f32, height: f32) -> Result<Self, Error>` (`height` is the cylinder part) | `rapier::Error` |
| `add_static_box` / `add_ramp` | `fn add_static_box(&mut self, half_extents: [f32;3], pos: [f32;3]) -> Result<(), Error>`; `fn add_ramp(&mut self, x0, x1, y0, y1, z_half, z_center: f32) -> Result<(), Error>` (needs `x1 > x0`, `z_half > 0`) | `InvalidHalfExtents`, `NonFinitePosition` |
| `move_and_slide` / `move_and_slide_with` | `fn move_and_slide(&mut self, wish_dp: [f32;2], dt: f32) -> Resolved`; `fn move_and_slide_with(&mut self, wish_dp, dt, shape: &MoveShape) -> Resolved` | never fails; `dt <= 0` gives no motion |
| `jump` | `fn jump(&mut self, vel: f32)`: up-only; buffered `BUFFER_MS` when airborne | NaN or down ignored |
| `snapshot` / `restore` | `fn snapshot(&self) -> ControllerSnapshot`; `fn restore(&mut self, ControllerSnapshot) -> Result<(), Error>` | `SnapshotMismatch` |
| `state_hash`, readers | `position`, `grounded`, `velocity_y`, `coyote_ms`, `buffer_ms` | |

`MoveShape::legacy()` is the frozen arena tuning: slide on, autostep `{ max_height: 0.5,
min_width: 0.0 }`, snap `Absolute(0.4)`, grade 1.0, skin 0.02. `MoveShape::default()` has autostep
off and snap `Relative(0.2)`. `Resolved.sliding_down_slope` equals `slipped` by construction
(`controller.rs:410`).

Example (adapted from `autostep_snap_grades`, `controller.rs:1494`; `HALF`, `RADIUS`, `HEIGHT` are its constants):

```rust
let mut c = Controller::spawn([-1.0, HALF, 0.0], RADIUS, HEIGHT)?;
c.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])?; // floor
c.add_static_box([1.0, 0.15, 3.0], [3.0, 0.15, 0.0])?; // 0.3 m ledge
for _ in 0..40 { c.move_and_slide_with([0.15, 0.0], FIXED_DT, &MoveShape::legacy()); }
```

Test: `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib controller::`

### 4.3 Tread support and autostep (`controller.rs`, private)

| Function | Line | Role |
|---|---|---|
| `horizontal_pass` | 415 | X then Z slide. A blocked axis asks `classify`; `Hit::Step(top)` is taken only with autostep set, the capsule grounded before the move, and `tread_holds` true. |
| `tread_holds(x, z, top, axis, sign, min_width, max_step)` | 522 | the step leaves `min_width` of free tread. |
| `tread_supported(x, z, feet, top, max_step) -> bool` | 551 | the ground probe under `(x, z)` reaches `top - EPS`. Only then does the capsule lift (`stepped = true`). |
| `classify` | 714 | returns `Hit::{Free, Step(f32), Wall}` (`controller.rs:185-191`). |
| `ground_height` | 807 | highest ground under the footprint within reach. |

Per `controller.rs:548-550`, the full-radius footprint meets a ledge before the half-radius probe does,
so the capsule keeps walking at its height until the tread can carry it. Test: `--lib controller::`
(`autostep_snap_grades`, `steep_face_must_never_autostep`).

### 4.4 `kcc` (float, std-only, no crate consumer)

`kcc::step(cfg: &KccConfig, w: &World, s: &mut KccState, inp: &KccInput, dt: f32)`: sweep and slide,
coyote and buffer timers, slope gate, deck carry (`apply_deck`). `KccConfig::default()`: radius 0.35,
slope 45 deg, gravity 22, jump 8. No `crates/` file imports it. Test: `--lib kcc::`.

## 5. Balance, stagger memory, limbs

### 5.1 `balance` (integer, authority)

Consumer: the arena hit pipeline (`observe`, then `apply_memory_hit` per landed blow). Constants:
`STAGGER_TICKS 30`, `STUMBLE_TICKS 45`, `FALL_PRONE_TICKS 30`, `CATCH_TICKS 15`,
`RECOVERY_BUDGET_TICKS 60`, `STUMBLE_ABOVE_STAGGER_Q8 128`, `FALL_ABOVE_STAGGER_Q8 384`,
`RESPONSE_Q8 = [128, 256, 384, 512, 640]` (steady, stagger, stumble, stumble, fall),
`DEFAULT_STAGGER_Q8 256`, `MAX_IMPULSE_Q8 4096`, `MAX_BONE_POS_M 1000.0`, `MAX_BONE_MASS_KG 1e6`.

| Item | Signature | Error |
|---|---|---|
| `BalanceParams::new` | `fn new(stagger_q8: u16) -> Result<Self, BalanceError>`, range `1..=4096` | `BadThreshold(v)` |
| `BalanceState::new` | `fn new(params: BalanceParams) -> Self` | |
| `observe` | `fn observe(&mut self, positions: &[[f32;3]], masses: &[f32]) -> Result<[f32;3], BalanceError>`; kg in `(0, 1e6]` | `EmptyBones`, `LengthMismatch{expected,got}`, `BadPosition(i)`, `BadMass(i)` |
| `apply_shove` | `fn apply_shove(&mut self, impulse_q8: u16) -> Result<BalanceVerdict, BalanceError>` | `BadImpulse(v)` (over 4096) |
| `apply_impact` | `fn apply_impact(&mut self, impulse_q8: u16) -> Result<GaitVerdict, BalanceError>` | `BadImpulse` |
| `apply_memory_hit` | `fn apply_memory_hit(&mut self, memory: &mut StaggerMemory, impulse_q8: u16, bone: usize) -> Result<(GaitVerdict, StaggerLevel), BalanceError>` | `BadImpulse`, `BadBone{bone,bones}`; nothing mutates on refusal |
| `respond` | `fn respond(&mut self, impulse_q8: u16) -> Result<u32, BalanceError>`: ticks to a legal stance, at most 60 for a fresh state | `BadImpulse` |
| `step`, `stance_legal`, `capture_bias`, `state_hash` | `fn step(&mut self)`; `fn stance_legal(&self) -> bool`; `fn capture_bias(&self, impulse_q8) -> Result<CaptureBias, BalanceError>`; `fn state_hash(&self) -> u64` | `BadImpulse` |
| `strike` (rapier) | `fn strike(&mut self, ragdoll: &mut PoweredRagdoll, strike: &HitStrike, mask: &MaskTable, tick: u64) -> Result<BalanceVerdict, ServoError>` (`balance.rs:763`) | `ServoError::{Balance, Ragdoll}`; nothing mutates on refusal |
| `strike_gait`, `strike_gait_biased`, `catch`, `rise` (rapier) | `balance.rs:791, 831, 851, 878` | `ServoError`; `rise` refuses with `GetupNotDue` or `WrongPhase` |

`BalanceError` variants: `EmptyBones`, `LengthMismatch`, `BadPosition`, `BadMass`, `BadImpulse`,
`BadThreshold`, `BadBone`, `BadMask{expected,got}`, `WrongPhase{want,got}`, `GetupNotDue`.
Free functions: `center_of_mass`, `com_mm`, `recovery_ticks`, `response_matrix`, `capture_error_mm`,
`servo_bias_targets`.

Example (from `balance_stagger_threshold_orders_five_strengths`, `balance.rs:1095`):

```rust
let mut state = BalanceState::new(BalanceParams::default());
state.observe(&positions, &masses)?;
assert_eq!(state.apply_shove(0)?, BalanceVerdict::Steady);
assert_eq!(state.apply_shove(256)?, BalanceVerdict::Staggered); // unity staggers
```

Tests: `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib balance::`; hash
proof: `--lib balance_hash`.

### 5.2 `stagger_memory` (integer)

| Item | Signature / rule | Error |
|---|---|---|
| `StaggerMemory::new` / `record_hit` | `fn new() -> Self`; `fn record_hit(&mut self, impulse_q8: u16, bone: usize) -> Result<StaggerLevel, BalanceError>`: pain += impulse (cap 4096), fatigue += 24 (cap 256), injury[bone] += impulse (cap 1024) | `BadImpulse`, `BadBone`; nothing changes |
| `step` / `level` | pain -8 per tick, fatigue -1 every 4 ticks, injury -4 per tick; `Flinch` below 512, `Stagger` from 512, `Ragdoll` from 896 | |
| `servo_strength_q8` / `injury_q8` / `limp_mm` | `256 - fatigue / 2`; `injury_q8` and `limp_mm` (injury / 64 mm) return `Result<u16, BalanceError>` | `BadBone` at bone 8 or more |

Tests: `--lib stagger_memory`; arena proof `node tools/gh-check.mjs cargo test -p engine-arena --lib stagger_hash`.

### 5.3 `limb` (integer, authority)

A severed limb as a free body. Units: `origin_mm` mm, velocity mm/s, angle micro-rad, spin
milli-rad/s, mass grams, impulse milli-newton-seconds, 60 Hz ticks. Constants: `MAX_LIMBS 256`,
`GRAVITY_MMPS_PER_TICK 163`, `MAX_SPEED_MMPS 40000`, `MAX_VOLUME_CM3 100000`,
`MAX_DENSITY_KG_M3 20000`, `MAX_RADIUS_MM 500`, `MAX_POS_MM 200000`, `SLEEP_TICKS 30`.

| Item | Signature | Error |
|---|---|---|
| `LimbSpawn` | `{ swing: u32, victim: u8, joint: u8, origin_mm, parent_vel_mmps, impulse_mns: [i32;3], volume_cm3: u32, density_kg_m3: u32, radius_mm: u32 }`; `validate`, `mass_g` (at least 1 g) | `BadVolume`, `BadSpawn` |
| `LimbPool::new` | `fn new(cap: usize) -> Result<Self, LimbError>` | `BadCap(cap)` for 0 or above 256 |
| `LimbPool::spawn` | `fn spawn(&mut self, spec: &LimbSpawn, debris_live: usize) -> Result<SpawnReport, LimbError>`: evicts the oldest sleeping limb, else the oldest; `debris_to_free` counts debris to drop | `LimbError` |
| `LimbPool::spawn_batch` | `fn spawn_batch(&mut self, batch: &mut [LimbSpawn], debris_live: usize) -> Result<Vec<SpawnReport>, LimbError>`: canonical (swing, victim, joint) order | first refusal stops |
| `LimbPool::step` | `fn step(&mut self)`: gravity, move, floor at `y = radius`, quarter bounce, 7/8 friction, sleep after 30 still ticks | |
| `snapshot` / `restore` / `state_hash` | `fn snapshot(&self) -> Vec<u8>`; `fn restore(bytes: &[u8]) -> Result<Self, LimbError>`; `fn state_hash(&self) -> u64` | `BadSnapshot` |

`SpawnReport { id, recoil: Recoil { victim, joint, impulse_mns }, evicted_limbs, debris_to_free }`.
Example (from `a_thousand_seeded_cuts_hash_twice_and_survive_snapshot_restore`, `limb.rs:627`):

```rust
let mut pool = LimbPool::new(MAX_LIMBS)?;
pool.spawn(&cut, 40)?;                 // 40 debris bodies already live
for _ in 0..3 { pool.step(); }
let bytes = pool.snapshot();
assert_eq!(LimbPool::restore(&bytes)?.snapshot(), bytes);
```

Test: `node tools/gh-check.mjs cargo test -p engine-physics --lib limb::`

## 6. Materials, colliders, queries, layers, cloth

### 6.1 `material` (integer table)

`Material::{Wood, Stone, Flesh, Metal, Sand, Cloth}`; `Material::ALL`; `Material::props(self) -> Props`
(`const fn`, exhaustive match). `Props { density_kg_m3: u16, friction_pm, restitution_pm,
linear_damping_pm, angular_damping_pm: u16 }`; accessors `density()`, `friction()`, `restitution()`,
`linear_damping()`, `angular_damping()` (thousandths divided by 1000).

| Material | kg/m^3 | friction | restitution | lin damp | ang damp |
|---|---|---|---|---|---|
| Wood | 1200 | 600 | 450 | 30 | 50 |
| Stone | 2000 | 800 | 200 | 20 | 50 |
| Flesh | 1000 | 900 | 0 | 50 | 50 |
| Metal | 7800 | 400 | 300 | 10 | 20 |
| Sand | 1600 | 1000 | 0 | 200 | 300 |
| Cloth | 250 | 700 | 0 | 400 | 500 |

No state, no errors. Test pins Flesh and Stone to the constants in use: `material_table_flesh_and_stone_are_the_constants_in_use_today` (`material.rs:231`); command `--lib material_table`.

### 6.2 `collider` (cooked static shapes)

`pub fn decode(bytes: &[u8]) -> Result<Vec<Collider>, ColliderError>`. Wire v1, little-endian:
`"E2CC" | u16 version (1) | u16 count`; per collider `u8 name_len | name | u32 layer | u8 kind`
(kind 0 hull: `u16 points | points x 3 f32 | u16 faces | faces x 3 u16`; kind 1 mesh: `u32 verts |
verts x 3 f32 | u32 indices | indices x u32`). Limits: 256 colliders, 64-byte names, 64 hull points,
65536 mesh vertices, 20000 mesh triangles. A defective blob yields no shapes at all.

| Item | Signature | Error |
|---|---|---|
| `Collider` / `Shape` | `{ id: String, layer: u32, shape: Shape }`; `enum Shape { Hull { points, faces }, Mesh { vertices, indices } }` | |
| `Shape::triangles` / `bounds` | `fn triangles(&self) -> Triangles<'_>` (bad index skips the triangle); `fn bounds(&self) -> Option<([f32;3],[f32;3])>` | |
| `ColliderError` | `Short{want}`, `BadMagic`, `BadVersion(u16)`, `TooMany`, `BadName`, `DuplicateName`, `BadLayer(u32)` (one layer bit), `BadKind(u8)`, `BadShape(&str)`, `NonFinite`, `Trailing` | |

Writer: `engine-assets` `collider_cook`. Test: `the_cooks_golden_bytes_decode_to_the_unit_cube` (`collider.rs:477`).

### 6.3 `query` (casts)

| Item | Signature | Units / range | Error |
|---|---|---|---|
| `ray_cast` | `fn ray_cast(colliders: &[Collider], origin: [f32;3], dir: [f32;3], max_distance: f32, scope: u32) -> Result<Option<Hit>, CastError>` | `dir` of any length; `max_distance > 0` | `CastError` |
| `sphere_cast` | `fn sphere_cast(colliders: &[Collider], origin, dir, radius: f32, max_distance: f32, scope: u32) -> Result<Option<Hit>, CastError>` | `radius >= 0` | `CastError` |
| `Hit` | `{ collider: usize, distance: f32, point, normal: [f32;3] }` | distance along the unit dir | |
| `CastError` | `NonFinite`, `ZeroDirection`, `BadDistance`, `BadRadius`, `Unscoped` | | |

A miss is `Ok(None)`. Determinism: `f64` math, stored order, ties to the earlier collider.
Example: section-level test `a_ray_hits_the_near_face_with_its_point_and_normal` (`query.rs:393`):

```rust
let world = [cube("COL_cube", layers::WORLD)];
let hit = ray_cast(&world, [-3.0, 0.5, 0.5], [2.0, 0.0, 0.0], 10.0, layers::WORLD)?.expect("a hit");
assert_eq!(hit.collider, 0);
assert!((hit.distance - 3.0).abs() < 1e-5);   // point [0, 0.5, 0.5], normal [-1, 0, 0]
```

Test: `--lib query::`

### 6.4 `layers`

Bits: `FIGHTER = 1`, `PROP = 2`, `DEBRIS = 4`, `SENSOR = 8`, `WORLD = 16`. Blocking rows
(`layers.rs:31-37`): fighter blocks fighter|prop|world; prop blocks fighter|prop|debris|world; debris
blocks prop|debris|world; sensor blocks nothing; world blocks fighter|prop|debris. `blocks(a, b)` is
symmetric; `scope` refuses an empty scope (`Empty`) or unknown words (`Unknown`). Test: `--lib layers::`.

### 6.5 `cloth` (presentation only)

CPU Verlet in floats, fixed 1/60 s tick, never hashed, never read by the sim.

| Item | Signature / rule | Error |
|---|---|---|
| `Cloth::new` | `fn new(grid: ClothGrid, size: [f32;2], origin: [f32;3], params: ClothParams) -> Result<Self, ClothError>`; size 0.01 to 20 m | `BadGrid`, `BadParam(&str)` |
| `step` / `pin_top_row` / `pin` / `unpin` | `fn step(&mut self, colliders: &[ClothCollider]) -> bool` (`false` and no work while asleep and untouched); `fn pin_top_row(&mut self, points: &[[f32;3]])`; `fn pin(&mut self, particle: usize, to: [f32;3])`; `fn unpin(&mut self, particle: usize)` | `BadPins{expected,got}`, `BadIndex` |
| `set_wind` / `set_params` / `reset` / `wake` | wind within +-60 m/s per axis | `BadParam` |
| `ClothParams::for_tier` / `validate` | `const fn for_tier(ClothTier) -> Self` (gravity 9.81, drag 1.5, no wind, no tearing); `fn validate(&self)`: gravity 0 to 40, drag 0 to 30, stiffness 0.05 to 1, iterations 1 to 16, substeps 1 to 8, tear strain 0.05 to 4 | `BadParam(name)` |
| `ClothCollider::sphere` / `capsule` | `fn sphere(center, radius: f32)`; `fn capsule(a, b: [f32;3], radius: f32)`; one-way | `ClothError` |
| `ClothTier::knobs` | Potato cape 4x6, banner 6x4, 1 substep, 3 iterations; Web 5x8 / 8x5, 1, 4; Comfort 6x10 / 10x6, 2, 4; High 8x14 / 14x8, 2, 5 | |

Example (from `a_capsule_keeps_the_cape_off_the_body`, `cloth.rs:1268`, with the `cape` helper at
`cloth.rs:850`, `CAPE_SIZE = [0.5, 1.0]` and `SHOULDERS = [0.0, 1.6, 0.0]` at `cloth.rs:839-840`):

```rust
let params = ClothParams::for_tier(ClothTier::Comfort);
let mut cape = Cloth::new(ClothTier::Comfort.knobs().cape, [0.5, 1.0], [0.0, 1.6, 0.0], params)?;
let body = ClothCollider::capsule([0.25, 0.7, -0.1], [0.25, 1.5, -0.1], 0.2)?;
for _ in 0..120 { cape.step(&[body]); }
```

Test: `--lib cloth::`. No consumer under `crates/` (grep for `Cloth::new`).

## 7. Rapier world, ragdolls, PD drive, tape (feature `rapier`)

| Item | Signature / value | Error |
|---|---|---|
| `rapier::GRAVITY_Y`, `HASH_SCALE`, `FIXED_DT` | `-9.81`; `1_000_000.0`; re-export of `engine_core::FIXED_DT` | |
| `World::new` / `add_static_box` | `fn new() -> Self`; `fn add_static_box(&mut self, half_extents: [f32;3], pos: [f32;3]) -> Result<usize, Error>` | `InvalidHalfExtents`, `NonFinitePosition` |
| `World::spawn_capsule` | `fn spawn_capsule(&mut self, pos: [f32;3], radius: f32, height: f32) -> Result<usize, Error>` (radius in `(0, 1000]`, height in `[0, 1000]`) | `InvalidRadius`, `InvalidHeight` |
| `World::step` | `fn step(&mut self, dt: f32) -> Result<(), Error>` | `InvalidDt` |
| `World::snapshot` / `restore` | `Snapshot { bodies: Vec<BodyState> }`; `fn restore(&mut self, Snapshot) -> Result<(), Error>` | `SnapshotMismatch` |
| `World::island_snapshot` | `fn island_snapshot(&mut self, coupled: &[(usize, usize)]) -> Vec<u8>`: **takes `&mut self`** and rebuilds the Rapier world around the same states as a sync point (`rapier.rs:368-375`) | none |
| `World::restore_island_snapshot` | `fn restore_island_snapshot(&mut self, bytes: &[u8]) -> Result<(), Error>`; atomic | `SnapshotBytes(SnapshotError)` |
| `Ragdoll::spawn` | `fn spawn(skeleton: &Skeleton, pose: &LocalPose, capsule_dims: &[BoneShape]) -> Result<Self, Error>` | `Anim(AnimError)` (length, pose), `InvalidRadius`, `InvalidHeight` |
| `Ragdoll::set_animated` / `blend_to_ragdoll` | `fn set_animated(&mut self, pose: &LocalPose) -> Result<(), Error>`; `fn blend_to_ragdoll(&mut self, amount: f32)`: 0 animated, 1 simulated | `BadBlend` |
| `Ragdoll::step` / `apply_hit` | `fn step(&mut self, dt: f32) -> Result<(), Error>`; `fn apply_hit(&mut self, bone: usize, impulse: [f32;3], tick: u64)` | `InvalidDt`, `NonFiniteImpulse`, `BadBone(usize)` |
| `Ragdoll::set_coupling` | `CouplingParams`: substeps at most 8 (`COUPLING_MAX_SUBSTEPS`) | `BadSubsteps`, `BadCouplingBound` |
| `PoweredRagdoll::spawn` | `fn spawn(skeleton, pose, capsule_dims: &[BoneShape], drives: &[DriveParams]) -> Result<Self, RagdollError>`; starts `Animated` | `BadDrive(usize)`, `BadBone`, `Anim` |
| `set_mask` / `set_drive` / `start_getup` / `step` | `set_mask(&[bool])`; `set_drive(bone, DriveParams)`; `start_getup() -> Result<(), RagdollError>` (done within `GETUP_BUDGET_TICKS = 60`); `step(dt)` | `BadMask`, `NotRagdolled`, `InvalidDt` |
| `pd::pd_axis` | `fn pd_axis(error, vel, kp, kd, dt, max: f32) -> f32`; `pd_raw`, `clamp_vector`, `dt_valid`; `PD_DT_FLOOR = 1e-6` | never fails; P only when `dt` is invalid |
| `ragdoll_tape::run_tape` | `fn run_tape(config: &TapeConfig) -> Result<TapeReport, TapeError>` | `Ragdoll`, `NoFalls`, `NoSteps` |
| `TapeConfig::new` / `baseline` | `fn new(seed: u64, falls: usize, fall_steps: u32, getup_steps: u32, clinch_steps: u32) -> Result<Self, TapeError>`; seed `TAPE_SEED = 2040` | |

Joint swing limits (`ragdoll::joint_depths`): depth 1 gets `SWING_ROOT = 0.9` rad, each deeper joint
loses `SWING_STEP = 0.15` rad, floor `SWING_MIN = 0.2`.

Example (from `hash_twice_identical`, `rapier.rs:537`):

```rust
let mut w = World::new();
w.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])?;
w.spawn_capsule([0.0, 4.0, 0.0], 0.4, 1.0)?;
for _ in 0..120 { w.step(FIXED_DT)?; }
```

Tests: `--features rapier --lib rapier::` (`hash_twice_identical`, `hash_matches_shared_fnv_bytes`), `--lib ragdoll::`, `--lib powered::`, and `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --test ragdoll_tape`.

## 8. Errors

Every refusal is typed and changes nothing: `rapier::Error`, `ragdoll::Error`, `powered::RagdollError`,
`balance::{BalanceError, ServoError}`, `limb::LimbError`, `cloth::ClothError`, `collider::ColliderError`,
`query::CastError`, `layers::LayerError`, `controller2::ProbeError`, `ragdoll_tape::TapeError`.

## 9. Invariants

| Invariant | Value / rule | Proof |
|---|---|---|
| Legacy golden `GOLDEN_600` | `d57b5c8ae3096614` (seed 2040, 600 ticks), legacy 2D path (`Space`, `rigid::state_hash`); must not move | `node tools/gh-check.mjs cargo test -p engine-proof --lib golden_hash_untouched` (`crates/proof/src/a9.rs:724`) |
| Rapier ragdoll digest | `GOLDEN_RAGDOLL = 0x8097_ccaf_b674_0acb`: one 17-bone fall, 300 steps (`crates/proof/src/ragdoll_matrix.rs:34`) | `node tools/gh-check.mjs cargo test -p engine-proof --lib the_standard_fall_keeps_its_four_machine_digest` |
| Balance hash | knockdown at ticks 40, 70, 100 hashes equal twice; ragdoll on or off gives the same hash (`balance.rs:2073`, `ragdoll.rs:1553`) | `--features rapier --lib balance_hash` |
| Tier-3 math, order, step | no `sin`, `cos`, `exp`, `powf` in sim glue; index order; 60 Hz; `enhanced-determinism`, no SIMD or parallel | `node tools/workspace-check.mjs`; root `Cargo.toml:193` |
| Restore | `controller2`, `Controller`, `LimbPool`, `World` restores replay bit-identical | `replay_exact_x2_with_snapshot` (`controller2.rs:1082`) |

## 10. Known gaps (read from the code, not changed here)

- `kcc::state_hash` (`kcc.rs:212-224`) omits `last_ride_mm` and `deck_delta_mm`. `apply_deck`
  (`kcc.rs:426-439`) reads `last_ride_mm` next tick, so states that differ only there hash the same.
  No crate consumes `kcc` yet.
- `rapier::World::add_static_box` says it returns "the static index" (`rapier.rs:219`), but
  `insert_static` returns `self.order.len()` (`rapier.rs:250`), the dynamic body count.
- `controller2` says "arena locomotion walks these walkers" (`controller2.rs:4`). Live arena locomotion
  calls `controller::Controller::move_and_slide` (`crates/arena/src/lib.rs:1549`). Arena imports only
  `Walker::spawn` and `separate_pair`; `step_walker` and `controller2::state_hash` have no caller outside
  this crate.
- `Controller::state_hash` reaches the arena checkpoint only with feature `sim` (`sync.rs:374-377`).

## 11. Where the code is

| File | Role |
|---|---|
| `lib.rs`, `rigid.rs`, `overlap.rs` | module list, `Space`; `Body`, integrate, hash; AABB overlap |
| `controller2.rs`, `controller.rs`, `kcc.rs` | integer walker; Rapier capsule with autostep and tread gates (rapier); float KCC (no consumer) |
| `balance.rs`, `stagger_memory.rs`, `limb.rs` | balance and gait (servo under rapier); pain, fatigue, injury; severed-limb pool |
| `material.rs`, `collider.rs`, `query.rs`, `layers.rs`, `cloth.rs` | material table; `E2CC` reader; casts; layer table; presentation cloth |
| `clinch.rs`, `articulated.rs`, `door.rs`, `stance.rs` | ragdoll parting pass (rapier); winch, rope and load graph; door states and outcome table; stance capsules, lean and weapon raise |
| `rapier.rs`, `ragdoll.rs`, `powered.rs`, `pd.rs`, `ragdoll_tape.rs`, `islands.rs` | Rapier world, ragdoll, powered ragdoll, PD law, tape, island snapshot (all rapier except `pd` and `islands`) |
| `tests/`, `tests/it/` | integration legs: `balance_matrix`, `stagger_memory`, `ragdoll_tape`, `pd_drive`, `oracle` (50-pose servo, contact and limit report), `trace_snapshot` |
| `Cargo.toml` | `rapier` feature |
