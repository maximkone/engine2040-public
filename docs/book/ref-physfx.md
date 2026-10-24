# Reference: physfx (`engine-physfx`)

Crate `crates/physfx` (package `engine-physfx`). The float Rapier presentation layer. Its job is to
look and feel heavy. Nothing it computes is hashed, and no authority crate may name it.
Authority rules live in `docs/HASH-LAW.md`; the boundary is in `docs/ARCHITECTURE.md` section 1.

## 1. Build, features, dependencies

| Item | Value |
|---|---|
| Feature | `rapier` (off by default). Without it every module is compiled out and the crate is empty. |
| Feature wiring | `rapier = ["dep:engine-physics", "engine-physics/rapier", "dep:engine-anim", "dep:rapier3d"]` (`Cargo.toml:20`) |
| Safety | `#![forbid(unsafe_code)]` (`lib.rs:43`) |
| Test | `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib` |
| Consumers | `crates/game` only: `src/break_look.rs` (`PhysFx::new` at 265, `add_static_box` at 275-288, `spawn_debris_of` at 319, `transforms` at 343, `stats` at 382). Ragdoll, limb, blast and thrown spawns have no caller outside this crate's tests; the RZ2 and RZ3 rows are not wired. |
| Edge rule | `tools/workspace-check.mjs:52` lists `engine-physfx` depending on `engine-physics` and `engine-anim` only; `engine-arena`, `engine-combat` and `engine-proof` must not depend on it. |

## 2. Rules of the world (from `lib.rs:15-40`)

- Units: meters, seconds, Y-up. Rotations are unit quaternions `[x, y, z, w]`. Impulses are newton-seconds.
- `PhysFx::step` is one tick at the fixed 60 Hz (`engine_physics::rapier::FIXED_DT`). The caller steps once per sim tick.
- `PhysFx::transforms(alpha)` blends the last two ticks, so a 144 Hz window does not stutter.
- Caps come from one table (`Tier::caps`). Section 3.
- A spawn that does not fit recycles the **oldest** group (a whole ragdoll counts as one unit). Section 4.
- Contacts above the cap shed the oldest limbs and debris. Ragdolls are never shed.
- Sleeping bodies cost no solver work and read the same transform every frame.
- Bad input (NaN, infinity, out of range) is refused with a typed `Error` before anything changes.
- Determinism: the same inputs give the same transforms on one machine. Across machines they may differ by centimeters.
- No clock of its own. `PhysFx::with_clock` supplies one for `Stats::micros`.

## 3. Caps and tick bars (`caps.rs`)

| Tier | bodies | ragdolls | contacts | statics | solver_iterations | ccd_substeps | tick_bar_us |
|---|---|---|---|---|---|---|---|
| `Potato` | 96 | 2 | 256 | 64 | 2 | 0 | 400 |
| `Web` | 192 | 4 | 512 | 128 | 3 | 1 | 800 |
| `Comfort` | 384 | 8 | 1024 | 256 | 4 | 1 | 1500 |
| `High` | 768 | 16 | 2048 | 512 | 6 | 2 | 2000 |

`Tier::ALL` is weakest first. `Caps` is plain data (`Clone, Copy, PartialEq`) and can be built by the
caller. `Caps::validate` (crate-private) refuses any zero limit except `ccd_substeps`, which may be 0
(CCD off). `Tier::tick_bar_us` is the mean per-step cost bar used by the ladder soak. The bar is five
times the measured mean on a GitHub runner, as the doc comment at `caps.rs:84-88` says.

Rapier is configured as `world.integration_parameters.num_solver_iterations = caps.solver_iterations`
and `max_ccd_substeps = caps.ccd_substeps` (`fx.rs:111-116`).

## 4. Public API

### 4.1 `PhysFx` (`fx.rs`, `fx/blast.rs`)

| Method | Signature | Units / range | Error |
|---|---|---|---|
| `new` | `fn new(tier: Tier) -> Self` | | none |
| `with_caps` | `fn with_caps(caps: Caps) -> Result<Self, Error>` | | `BadCaps` |
| `with_clock` | `fn with_clock(self, clock: fn() -> u64) -> Self` (`#[must_use]`) | microseconds | none |
| `add_static_box` | `fn add_static_box(&mut self, half: [f32;3], pos: [f32;3]) -> Result<(), Error>` | half-extents `(0, 1000]` m; position within 1000 m; statics never recycled | `NonFinite(Extent)`, `OutOfRange(Extent)`, `OutOfRange(Position)`, `OverCap{need, cap}` |
| `spawn_ragdoll` | `fn spawn_ragdoll(&mut self, slot: u8, rig: &Rig, pose: &LocalPose, velocity: [f32;3], impulse: [f32;3]) -> Result<(), Error>` | velocity `<= 100` m/s per axis; impulse `<= 100000` N s; the impulse lands on bone 0 | `Anim`, `OutOfRange(Velocity/Impulse)`, `OverCap{need: bones, cap: bodies}` |
| `spawn_limb` / `spawn_limb_of` | `fn spawn_limb(&mut self, part: u32, shape: BoneShape, at: Transform, velocity: [f32;3]) -> Result<(), Error>`; `_of` adds `material: Material` first | radius and half-height `[0.01, 5]` m (half-height may be 0) | `OutOfRange(Extent)`, `OutOfRange(Position)`, `OutOfRange(Velocity)` |
| `spawn_debris` / `spawn_debris_of` | `fn spawn_debris(&mut self, shards: &[Shard], impulse: [f32;3]) -> Result<Batch, Error>` (stone); `fn spawn_debris_of(&mut self, material: Material, shards: &[Shard], impulse: [f32;3]) -> Result<Batch, Error>` | every shard checked; list cut to the body cap in caller order | `NoShards`, `OutOfRange(Impulse)` (impulse / mass above 100 m/s), per-shard `Extent`, `Position`, `Velocity` |
| `apply_blast` | `fn apply_blast(&mut self, blast: Blast) -> Result<usize, Error>` (`fx/blast.rs:20`) | section 4.3 | `OutOfRange(Position|Extent|Impulse)` |
| `spawn_thrown` | `fn spawn_thrown(&mut self, id: u32, shape: BoneShape, material: Material, flight: Flight) -> Result<(), Error>` (`fx/blast.rs:60`) | `flight.ticks` in `1..=1200`; launch velocity `<= 100` m/s | `NonFinite` or `OutOfRange(Position)` (`from`, `to`), `OutOfRange(Ticks)`, `OutOfRange(Velocity)`, shape `Extent` |
| `step` | `fn step(&mut self)` | one 60 Hz tick | none (no `Result`) |
| `transforms` | `fn transforms(&self, alpha: f32) -> Result<Vec<Placed>, Error>`; alpha clamped to `[0, 1]`; output in spawn order | | `NonFinite(Alpha)` |
| `stats` | `fn stats(&self) -> Stats` | counters as of the last step | none |

Determinism note for every spawn: inputs are checked, then the world changes in one pass; a refused
call leaves the world exactly as it was (`tests.rs:387`, `a_nan_impulse_is_refused_and_leaves_the_world_unchanged`).

Example (adapted from `budget_eight_ragdolls_two_hundred_debris_six_limbs`, `tests.rs:942`):

```rust
let mut fx = on_floor(PhysFx::new(Tier::Comfort));          // static floor, top at y = 0
for slot in 0..8u8 {
    fx.spawn_ragdoll(slot, &rig, &start, [0.0, 0.0, 1.0], [0.0, 0.0, 40.0])?;
}
fx.spawn_debris(&debris_field(200, 2.0), [0.0, 150.0, 0.0])?;
fx.step();
let placed = fx.transforms(1.0)?;   // Key::Bone, Key::Shard, ...
```

Test: `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib budget_eight`

### 4.2 Data types (`types.rs`, `rig.rs`)

| Type | Fields / behaviour | Notes |
|---|---|---|
| `Transform` | `{ pos: [f32;3], rot: [f32;4] }`; `IDENTITY`; `blend` (crate-private): lerp position, shortest-arc normalized lerp rotation | |
| `Key` | `Bone { slot: u8, bone: u16 }`, `Limb { part: u32 }`, `Shard { batch: u32, index: u32 }`, `Thrown { id: u32 }` | the caller maps a key to its mesh |
| `Placed` | `{ key: Key, at: Transform }` | one per live body |
| `Shard` | `{ at: Transform, half: [f32;3], velocity: [f32;3] }` | box half-extents in m |
| `Batch` | `{ id: u32, admitted: usize }` | `admitted` is the prefix of the list that fit; the rest were not spawned |
| `Blast` | `{ center_mm: [i32;3], radius_mm: u32, impulse_ns: u32 }` | integers, as the authority holds them |
| `Flight` | `{ from: [f32;3], to: [f32;3], ticks: u32 }` | launch and hit, m and ticks |
| `Rig` | `Rig::new(skeleton: Skeleton, shapes: Vec<BoneShape>) -> Result<Self, Error>`; `fn bones(&self) -> usize` | validates skeleton, one shape per bone (`Anim` length error), each shape |
| `Stats` | `{ bodies, awake, contacts, recycled: u64, steps: u64, micros: u64 }`; `receipt_line(&self) -> String` | `receipt_line` is `physfx: bodies=.. contacts=.. us=..` (`types.rs:155`) |

Re-exports: `Material` and `BoneShape` come from `engine_physics`; `Tier`, `Caps`, `Error`, `Field`.

### 4.3 Blast and thrown arcs (`fx/blast.rs`)

`apply_blast`: every body whose centre is strictly inside `radius_mm` gets an outward velocity gain
`peak * (1 - distance / reach)` with `peak = impulse_ns / 70` kg-equivalent (so a 70 kg body at the
centre gains `impulse_ns / 70` m/s whatever it weighs). A body at the exact centre (under 0.1 mm)
goes straight up. Each pushed body is woken. Returns how many were pushed.

Refusals (`check::blast`, `check.rs:94`): centre component over 1000 m (`OutOfRange(Position)`);
radius 0 or over 100 m (`OutOfRange(Extent)`); impulse over 7000 N s (`OutOfRange(Impulse)`), because
`impulse_ns / 70` must stay under the 100 m/s speed cap. Test: `blast_bodies_in_radius_gain_outward_speed_in_order_of_distance_none_outside_move` (`tests/blast.rs:61`).

`spawn_thrown`: the launch velocity is the drag-free arc from `flight.from` to `flight.to` in exactly
`flight.ticks` steps, corrected for the solver's substeps (`ballistic_velocity`, `fx/blast.rs:95`). The
capsule's long axis (local +Y) is aimed along the launch direction. Linear damping is zeroed after the spawn.
Test: `thrown_body_arrives_on_the_authority_hit_on_the_hit_tick_at_every_tier` (`tests/blast.rs:282`).

### 4.4 Interpolation (`fx.rs:346`, `types.rs:52`)

`transforms(alpha)` takes each body's `(prev, curr)` pair (stored at the last two ticks) and blends
them. A sleeping body is settled at `prev == curr`, so it reads the same at every alpha. Tests:
`transforms_blend_between_the_last_two_ticks` (`tests.rs:667`), `rotations_blend_along_the_short_arc` (`tests.rs:693`).

## 5. Caps, recycling and shedding

Bookkeeping (`fx.rs:67-87`): a slab of `Record`s (indices reused last-in first-out), a list of `Group`s in
spawn order, and a `GroupId` per group: `Ragdoll(slot)`, `Limb(part)`, `Thrown(id)`, `Shard`.

| Event | Rule | Counted in `Stats::recycled` |
|---|---|---|
| Respawn of the same id | `drop_group` removes the old group first (ragdoll slot, limb part, thrown id) | no (a replacement, not a recycle) |
| Ragdoll count at the ragdoll cap | the oldest ragdoll group is recycled (`spawn_ragdoll`) | yes |
| Bodies over the body cap | `make_room(need)` recycles `groups[0]` (oldest, of any kind) until `live + need <= bodies` | yes |
| Overspawn of one ragdoll | refused: `OverCap{need, cap}` before anything changes | no |
| Contacts over the contact cap | after each step, up to `SHED_PER_STEP = 8` oldest non-ragdoll groups are recycled (`shed_contacts`, `fx.rs:488`) | yes |

`Stats::contacts` is the count **before** shedding (`fx.rs:489-490`). Contacts counted are active
pairs with an awake dynamic side (`pair_is_live`). Tests: `body_cap_holds_under_ten_times_overspawn`,
`the_oldest_body_is_recycled_first`, `a_ragdoll_is_recycled_whole_never_in_part`,
`contacts_over_the_cap_shed_the_oldest_debris_and_never_a_ragdoll` (`tests.rs:212, 256, 290, 768`).

## 6. Errors (`error.rs`)

| Variant | Meaning | Field enum |
|---|---|---|
| `NonFinite(Field)` | NaN or infinity | `Field::{Position, Rotation, Velocity, Impulse, Extent, Alpha, Ticks}` |
| `OutOfRange(Field)` | finite but outside the bound | same |
| `BadCaps` | a cap limit is zero (`with_caps`) | |
| `OverCap { need, cap }` | one spawn alone exceeds the cap | |
| `NoShards` | `spawn_debris` got an empty list | |
| `Anim(AnimError)` | skeleton, pose or bone list failed the animation contract | |

Every variant leaves the world exactly as it was. Tests: `with_caps_refuses_a_zero_limit` (`tests.rs:189`),
`finite_but_absurd_input_is_refused_too` (`tests.rs:500`), `the_static_cap_and_bad_boxes_are_refused`
(`tests.rs:816`), `blast_refusals_name_the_input_and_leave_the_world_unchanged` (`tests/blast.rs:193`).

## 7. Seeding from authority events (the one live path)

`crates/game/src/break_look.rs` is the only caller of this crate today. The flow, per accepted sim tick:

1. The arena's bonded collection emits `BreakEvent::{BondBroken, ChunkFreed}` (`engine-destruction`).
2. `BreakLook::observe(tick, events)` reads them (`plan`, `break_look.rs:396`). A `BondBroken` gives
   an impulse `strain * mean mass` (the authority added `impulse / mean mass` to strain, so this runs it
   back). A `ChunkFreed` marks a standing piece as freed, once. The freed set is then sorted nearest the
   struck chunk first, ties by index (`break_look.rs:428`).
3. The freed pieces become `Shard`s at their chunk centres with zero velocity, half-extents from the
   prop (`debris`, `break_look.rs:446`).
4. A push is built from the blow: `kick = min(impulse / authority_mass, 12 m/s)`, `push = min(kick *
   kilograms, 90000 N s)` along a lean that points away from the struck chunk and up by 0.5
   (`PUSH_LIFT`).
5. `spawn_debris_of(material, &shards, push)` admits the first `min(len, body cap)` shards; the caller's
   order is the priority (nearest the blow first).
6. The batch is recorded per piece (`Piece::Flying` or `Piece::Gone`), and `fx.step()` runs only once
   something broke.
7. The renderer calls `BreakLook::frame(alpha)`, which reads `transforms(alpha)` (`break_look.rs:343`).

Nothing from physfx goes back to `observe`: the picture never feeds the authority (`break_look.rs:12-16`).
Test: `debris_count_stays_under_the_tier_cap` (`break_look.rs:644`) and
`the_intact_mesh_is_hidden_on_the_break_tick` (`break_look.rs:618`), in `crates/game`.

## 8. Rules the crate must keep

- **Never hashed.** Not in `state_hash`, not in a checkpoint, not in a replay tape. There is no `state_hash`
  in this crate, by design.
- **Never read by the sim.** `arena`, `combat` and `proof` must not name it. The test
  `sim_crates_never_name_physfx` (`tests.rs:915`) scans `crates/arena`, `crates/combat` and `crates/proof`
  for `engine_physfx` or `engine-physfx` in `.rs` and `Cargo.toml` files, and plants a leak to prove the
  scan works. Command: `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib sim_crates_never_name_physfx`.
- **No authority feedback.** A break, a blast or a hit is an input here, never an output.
- **No wall clock in the crate.** `Stats::micros` is zero unless the caller passes a clock.
- **Bounded cost.** The soak `ladder_soak_each_tier_holds_its_scene_for_600_ticks_under_its_bar`
  (`tests/ladder.rs:168`) holds each tier's scene for 600 ticks under `tick_bar_us`; the budget soak
  `budget_eight_ragdolls_two_hundred_debris_six_limbs` (`tests.rs:942`) prints mean and p95 and asserts a 4 ms
  mean in release.
- **No `unsafe`.**

## 9. Invariants

| Invariant | Rule | Proof |
|---|---|---|
| Caps hold | bodies never exceed `caps.bodies`; ragdolls never exceed `caps.ragdolls`; statics never exceed `caps.statics` | `body_cap_holds_under_ten_times_overspawn` (`tests.rs:212`), `the_ragdoll_cap_recycles_the_oldest_ragdoll_and_a_slot_respawn_replaces` (`tests.rs:314`), `the_static_cap_and_bad_boxes_are_refused` (`tests.rs:816`) |
| Same inputs, same transforms | one machine, bit-equal twice | `the_same_inputs_give_identical_transforms_twice` (`tests.rs:366`) |
| Thrown arc on the hit tick | within 1 cm at every tier | `thrown_body_arrives_on_the_authority_hit_on_the_hit_tick_at_every_tier` (`tests/blast.rs:282`) |
| Material rows | wood and stone bounce to different heights; sand and flesh do not bounce | `material_wood_and_stone_debris_bounce_to_different_heights` (`tests/materials.rs:36`) |
| Receipt line | fixed field order | `ladder_receipt_line_names_bodies_contacts_and_micros_in_order` (`tests/ladder.rs:75`) |
| Tier table | every tier valid and climbing | `every_tier_table_is_valid_and_climbs` (`tests.rs:172`) |

## 10. Known gaps (read from the code, not changed here)

- **Undocumented blast ceiling.** The `Blast` docs describe the impulse as the momentum a 70 kg body
  gains, with no upper bound. `check::blast` refuses anything above 7000 N s (`check.rs:94-106`, via the
  100 m/s speed cap). Callers must know this.
- **Solver substeps.** `ballistic_velocity` treats `caps.solver_iterations` as the substep count of the
  gravity-then-move integrator (`fx/blast.rs:92-101`). That is an assumption about Rapier's soft-step
  solver; the arc test checks it numerically at every tier, not from the Rapier source.
- **Unused rows.** The ragdoll, limb, blast and thrown paths have no caller outside this crate's tests.
  `break_look.rs` uses only `new`, `add_static_box`, `spawn_debris_of`, `transforms`, `stats` and `step`.
- **Static boxes.** `add_static_box` has no removal. `BreakLook::new` adds one floor plus one box per anchor
  (`break_look.rs:275-288`), and those count against `caps.statics` for the life of the world.

## 11. Ragdoll tone and the tow brace

Landed 2026-10-24 (`cb278da5b`). The joint motors live in `crates/physfx/src/fx/hit.rs`. The tow number lives in the game crate, `crates/game/src/ragdoll_look.rs`.

- **Resting tone.** `RAGDOLL_TONE` is 0.2 of the body's muscle (`hit.rs:39`). Every ragdoll's motors start at that tone (`spawn_drive`, `hit.rs:256`).
- **At the tone the motors hold no pose.** At rest each motor is a velocity motor aimed at zero, so it acts as joint friction (`drive`, `hit.rs:60`). A fallen body folds, settles and sleeps. It does not whip like a sack.
- **A blow braces the body.** A blow sets the strength to the full muscle. It falls back to the tone over `HIT_FADE_S` (0.3 s, `hit.rs:23`).
- **Holding a brace.** `PhysFx::brace_ragdoll(slot, strength)` (`hit.rs:217`) raises the brace to `strength`. The value is clamped to 0 to 1 and capped at the body's muscle. A lower value does nothing. Once the calls stop, the brace fades to the tone.
- **Reading a brace.** `PhysFx::ragdoll_strength(slot)` (`hit.rs:244`) returns the current strength, or `None` when the slot has no ragdoll.
- **Tow brace.** `TOW_BRACE` is 0.6 (`ragdoll_look.rs:100`). `FighterLook::observe_carries` (`ragdoll_look.rs:1393`) braces each towed body at that value (`ragdoll_look.rs:1435`). A towed fighter flies braced, and the rope still swings its limbs.
- **Getting up.** The module doc says a towed body may get up once its root is slower than `LANDED_MS` (1.2 m/s, `ragdoll_look.rs:102`).
- **Limp bodies.** A body file with zero muscle keeps zero tone (`hit.rs:33-38`, and the muscle cap in `brace_ragdoll`).
- **Blow path.** The FEEL9 blow reaction calls `PhysFx::hit_ragdoll` from `crates/game/src/ragdoll_look/hit.rs` (lines 244 and 339). That chain is separate from the tone.

**Limits.**

- `TOW_BRACE` is one constant for every towed body. It is not per body.
- Presentation only. Nothing here is hashed or read by the sim (section 8).
- Stale text: section 10 says the ragdoll path has no caller outside the crate's tests. That is out of date. `brace_ragdoll` and `hit_ragdoll` have game callers (see above).

**Tests.**

- `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib hit` runs the `tests::hit` module (`crates/physfx/src/tests/hit.rs`). It includes `strength_goes_from_one_to_the_tone_over_three_tenths_of_a_second` (line 79), `a_braced_neck_gives_way_less_than_a_limp_one` (line 143), `a_toned_body_keeps_its_shape_where_a_limp_one_folds` (line 204) and `a_held_brace_stays_up_while_asked_and_fades_to_the_tone_after` (line 217).
- `node tools/gh-check.mjs cargo test -p engine-game --features app --lib ragdoll_look` runs the look tests. `ragdoll_look_tow_lifts_a_body_along_the_track_then_it_lands_and_gets_up` is at `ragdoll_look.rs:2189`.

## 12. Where the code is

| File | Role |
|---|---|
| `crates/physfx/src/lib.rs` | rules of the world, re-exports, feature gate |
| `crates/physfx/src/caps.rs` | `Tier`, `Caps`, cap table, tick bars, `validate` |
| `crates/physfx/src/error.rs` | `Error`, `Field`, `Display` |
| `crates/physfx/src/types.rs` | `Transform`, `Key`, `Placed`, `Shard`, `Batch`, `Blast`, `Flight`, `Stats` |
| `crates/physfx/src/check.rs` | every input check: finiteness, bounds, speed and impulse caps, blast and flight rules |
| `crates/physfx/src/rig.rs` | `Rig`, the validated skeleton plus shapes |
| `crates/physfx/src/fx.rs` | `PhysFx`: spawns, recycling, shedding, step, transforms, stats |
| `crates/physfx/src/fx/blast.rs` | `apply_blast`, `spawn_thrown`, the ballistic velocity, the aim rotation |
| `crates/physfx/src/tests.rs` | caps, recycling, determinism, refusals, sleep, interpolation, budget, the sim-crate scan |
| `crates/physfx/src/tests/blast.rs` | blast falloff, sleep wake, arc at every tier, refusals |
| `crates/physfx/src/tests/ladder.rs` | tier ordering, receipt line, the 600-tick soak |
| `crates/physfx/src/tests/materials.rs` | material bounce, density, damping, refusals |
| `crates/game/src/break_look.rs` | the consumer: break events in, debris and transforms out |
| `crates/physfx/Cargo.toml` | `rapier` feature and dependencies |
