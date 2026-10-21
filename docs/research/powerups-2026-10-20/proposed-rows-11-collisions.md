# Proposed rows: collisions, cloth and layers (CLX) — 2026-10-21

Lane: N1 collisions (`docs/research/powerups-2026-10-20/11-collisions-cloth-layers.md`).
One row per find. The lead merges the board; nothing here edits `docs/SPRINT*.md`,
`docs/FOCUS.md` or `docs/plan/finish-line.md`.

Every row names the donor it was read from, the exact file inside that donor, the licence read
live, and the date it was read. `OBSERVED` means read in the tree or in the donor file on that
date; `DERIVED` means reasoned from what was read.

---

## CLX1 — the cloth collider carries a layer, and the layer table decides the pairs

**What it is.** A cloth collider today is a bare capsule: it pushes the cloth aside and the
cloth pushes nothing back, but it cannot say which body it is, so the cloth meets everything it
is handed. CLX1 gives every collider a group bit and an accept mask and asks the one layer table
whether the pair may meet at all.

- **Donor**: `bulletphysics/bullet3` — `src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h`
  (the `CollisionFilterGroups` enum, `m_collisionFilterGroup`, `m_collisionFilterMask` and the
  two-direction test in `needsBroadphaseCollision`).
  URL: https://raw.githubusercontent.com/bulletphysics/bullet3/master/src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h
  Licence: zlib (the licence text sits in the file header; read live 2026-10-20).
- **Closes**: PX28 (one-way cloth-vs-collider coupling: the coupling needs the layer, not one
  torso capsule), and the group half of PX9 (the same table serves the ragdoll groups).
- **Owned paths**: `crates/physics/src/layers.rs`, `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — prints
  `cloth_gap the layer table decides which colliders the cloth meets` green: cloth meets a
  fighter, meets the world, never meets debris, never meets a sensor.
- **Kill line**: the cloth must not lose the wearer's own body: if the pair rule ever refuses
  the fighter layer, the cape sinks into the torso and the row dies.
- **Cost**: S (done in this lane: `ClothCollider::with_layer`, `ClothCollider::mask`,
  `layers::Layers`, `layers::collides`).

## CLX2 — the swept particle-vs-collider test (no tunnelling for fast cloth)

**What it is.** The only speed guard a cloth sheet had was a 30 m/s cap on the Verlet step: at
that speed a particle travels 0.25 m per substep, five times a 5 cm shield, so a gust could put a
cape straight through a body. CLX2 sweeps the particle's path against the collider and stops it
at the time of impact, using the hashed tick's own Q16 detector.

- **Donor**: the engine's own `crates/physics/src/query.rs` `swept_cast` (PX20, Q16 `Fixed`,
  time-of-impact then id) — read 2026-10-20. Idea check: arXiv 2403.19272 v4,
  "Efficient GPU Cloth Simulation with Non-distance Barriers and Subspace Reuse",
  https://arxiv.org/abs/2403.19272 (abstract) and https://arxiv.org/html/2403.19272v4 (full text),
  read live 2026-10-20: a non-distance barrier lets most of the traditional CCD go and keeps a
  cheap partial pass. Licence: research paper, ideas only, no bytes.
- **Closes**: PX10 (per-body CCD enable: the cloth gets the same continuous answer the bones
  already have) and the stability half of PX27.
- **Owned paths**: `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — prints
  `cloth_gap gust crossed=0 pen_max_m=0.0000 swept_stops=6265` (the swept test really fires) and
  `cloth_gap a thirty metre gust never crosses a collider` green.
- **Kill line**: one crossed collider in the 30 m/s gust case, or the swept test never firing
  (a test that passes because nothing moves).
- **Cost**: S (done in this lane: `Cloth::sweep_colliders`, `Cloth::cap_speed`).

## CLX3 — the intercollision particle set (self-collision), the Bullet cluster shape

**What it is.** PX29's self-collision code was deleted in the RZ7 rewrite of the cloth module
(`331dca1b`), so a folded curtain today rests self-penetrated. CLX3 brings it back as an
*intercollision particle set*: the caller names the particles that collide with each other, a
spatial hash keeps the pass off the quadratic path, and the linked neighbours (structure plus
shear) are skipped so the links never fight the push.

- **Donor**: `bulletphysics/bullet3` — `src/BulletSoftBody/btSoftBody.h`, `struct Cluster`
  (`m_collide`, `m_maxSelfCollisionImpulse = 100.f`, `m_selfCollisionImpulseFactor = 0.01f`,
  `m_clusterIndex`) and `fCollision::{CL_SELF, CL_SS, VF_SS}`.
  URL: https://raw.githubusercontent.com/bulletphysics/bullet3/master/src/BulletSoftBody/btSoftBody.h
  Licence: zlib (header text read live 2026-10-20).
- **Old shape read from git history (read-only, nothing checked out)**: `git log -S` found
  `e5c85f19` "PX29 builder-body-r3: folded-curtain self-collision (cloth_self 2/2, lib 101/0,
  quality 99A)" (2026-10-06) and `d6526e26` "474 PX27: cloth_gap baseline tape (1/1, digest x2,
  quality 99 A)" (2026-10-05). The old shape reused: the intercollision set over the
  structural-plus-shear links, the pair set that skips the 1-ring, `MAX_SELF_PASSES = 4` with an
  early-out, the alternating sweep direction, the pinned/free share split, and the coincident
  escape along the index-sum axis. The old code was integer fixed-point on a `ClothWorld` that
  no longer exists; the shape was re-implemented in the f32 Verlet sheet.
- **Closes**: PX29 (self-collision; re-open it — the code it accepted is gone).
- **Owned paths**: `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — prints
  `cloth_gap fold start_gap_m=0.0000 close_pairs=0` and `cloth_gap a folded curtain never
  self-penetrates` green (zero close pairs at every tick of the 60-tick fold).
- **Kill line**: one close pair left after the sweep, or the pass costs more than the 0.5 ms
  cloth budget at 8 fighters.
- **Cost**: S (done in this lane).

### Why the Bullet cluster shape beats the Rapier soft body here

Both are Apache-2.0/zlib-clean and both are "already in reach", but they are not the same job.
Rapier 0.36 ships a full soft-body solver (`rapier3d-0.36.0/src/dynamics/soft_body/`, read live
2026-10-20 in the vendored cargo registry: `soft_body_builder_settings.rs` has `self_contacts`,
`particle_radius`, `surface_collider` and `tear_strain`; `soft_body_cluster.rs` has
`SoftBodyCluster`; `soft_body_coloring.rs` has the greedy colour sweep). Taking Rapier means a
second cloth path: a Rapier soft body wants a mesh and a mass table, our cape is a procedural
6x10 grid, and the two would need one asset path each. Bullet's `btSoftBody.h` gives us the part
we actually need — the *shape* of an intercollision set (a named group of particles with its own
collision flag and its own impulse cap) — as a rule we write in our own f32 code, inside the one
solver we already have. One solver, one asset path, no new float world. Rapier stays the right
donor for a future volumetric soft body in `crates/physfx` (flesh jiggle, a muscle), never for
the cape.

## CLX4 — the per-particle Max Distance (a gradient, not one global clamp)

**What it is.** The tether is one number per particle (the reach from its nearest pin) solved as
a hard clamp, so a sheet cannot have a painted limit. CLX4 gives every particle its own Max
Distance, measured from its rest position (Unity and Unreal semantics), so a gradient across the
sheet is one data row and not a second solver. Zero keeps the tether reach, so PX35's bar is
unchanged.

- **Donor**: Unity 6000.5 `Cloth` documentation (Max Distance and Surface Penetration painted
  with Select/Paint/Gradient),
  https://docs.unity.com/en-us/engine/6000.5/manual/physics-section/physics-overview/class-cloth,
  read live 2026-10-20, PROPRIETARY (docs only, no bytes). Cross-check: Unreal 5.8 Clothing
  Tool, https://dev.epicgames.com/documentation/en-us/unreal-engine/clothing-tool-in-unreal-engine
  (Paint/Gradient/Smooth/Fill, "a value of 0 would mean the skinned vertex could not move and a
  value of 100 would allow the skinned vertex to move one meter"), read live 2026-10-20,
  PROPRIETARY (docs only, no bytes).
- **Closes**: PX35 (the gradient half; the tethers themselves already landed with 6f871bb5).
- **Owned paths**: `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — the
  `cloth_gap` suite green with `Cloth::set_max_distance_gradient` in use, and PX35's own test
  `a_32x32_sheet_settles_with_every_link_within_a_quarter_of_rest_and_replays_exactly` still
  green (the default limit is the tether reach, so the bar does not move).
- **Kill line**: the per-particle limit must not change the default sheet: if a sheet with no
  gradient set stretches past the PX35 bar, the row dies.
- **Cost**: S (done in this lane).

## CLX5 — the layer table gets CLOTH and SOFT, and becomes group plus accept mask

**What it is.** The table has five bits (fighter, prop, debris, sensor, world) and one symmetric
"a blocks b" row each. CLX5 adds the two soft bits and changes the table to the Bullet proxy
shape: every collider carries a group bit and an accept mask, and a pair collides only when each
side accepts the other. The old one-way read (`blocks`) stays, so the arena kit zone and the
door verbs keep working unchanged.

- **Donor**: `bulletphysics/bullet3` — `src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h`
  (zlib, read live 2026-10-20), same file as CLX1. Second shape for the same row: Godot 4.4
  `SoftBody3D` (`collision_layer` = the layers it is in, `collision_mask` = the layers it scans),
  https://docs.godotengine.org/en/4.4/classes/class_softbody3d.html, read live 2026-10-20,
  MIT (docs).
- **Closes**: **no existing row** — propose a new row "the collision layer table" and give it
  this id; it also feeds G11 and the ragdoll groups (PX9) and the per-body CCD switch (PX10).
- **Owned paths**: `crates/physics/src/layers.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib layers` — green:
  `the table is seven distinct single bits`, `blocking is symmetric for every pair`, `the pair
  rule is bullets and agrees with the table`, `a custom mask narrows what a collider meets`.
- **Kill line**: the table must keep the old answers for every old pair: if `blocks` changes for
  fighter/prop/debris/sensor/world, the arena kit zone (`crates/arena/src/kit_zone.rs:123`) and
  the door verbs break and the row dies.
- **Cost**: S (done in this lane).

### The exact layer table shape (for the physfx lane to build later)

The float world gets the same table, in the same bits, with the same rule. Nothing here is
float-specific: the table is bit math.

**The bits** (one group bit per collider, `u32`):

| word | bit | value |
|---|---|---|
| fighter | `FIGHTER` | `1 << 0` |
| prop | `PROP` | `1 << 1` |
| debris | `DEBRIS` | `1 << 2` |
| sensor | `SENSOR` | `1 << 3` |
| world | `WORLD` | `1 << 4` |
| cloth | `CLOTH` | `1 << 5` |
| soft | `SOFT` | `1 << 6` |

**The mask rule** (the Bullet proxy pair rule, read live from `btBroadphaseProxy.h`, zlib, on
2026-10-20). A collider has a `group` (exactly one bit) and a `mask` (the groups it accepts).
A pair collides only when **both** directions pass:

```
collides(a, b) = (a.group & b.mask) != 0 && (b.group & a.mask) != 0
```

The table's accept mask per group (symmetric: `a` lists `b` exactly when `b` lists `a`):

```
fighter: FIGHTER | PROP | WORLD | CLOTH | SOFT
prop:    FIGHTER | PROP | DEBRIS | WORLD | CLOTH | SOFT
debris:  PROP | DEBRIS | WORLD
sensor:  0
world:   FIGHTER | PROP | DEBRIS | CLOTH | SOFT
cloth:   FIGHTER | PROP | WORLD | CLOTH
soft:    FIGHTER | PROP | WORLD | SOFT
```

Read the table in Rust as `engine_physics::layers` (the crate is the owner; the float world
imports it, it does not copy it):

```rust
use engine_physics::layers::{self, Layers};

let pair = Layers::new(layers::CLOTH).expect("cloth group");   // group + default mask
let other = Layers::new(layers::FIGHTER).expect("fighter group");
let meet = layers::collides(&pair, &other);                    // the pair rule
```

**The Rapier mapping** (the float world, `crates/physfx`). Rapier's `InteractionGroups` is the
same pair, already used by `crates/physics/src/joint_limits.rs:229`:

```rust
use rapier3d::group::{Group, InteractionGroups, InteractionTestMode};

fn rapier_groups(layers: &Layers) -> InteractionGroups {
    // The membership half is the group bit, the filter half is the accept
    // mask. `And` is the pair rule: both sides must accept each other.
    InteractionGroups::new(
        Group::from_bits_retain(layers.group),
        Group::from_bits_retain(layers.mask),
        InteractionTestMode::And,
    )
}
```

`InteractionTestMode::And` is the whole rule: Rapier already asks both directions. The float
world therefore needs no new concept, only the same table: put `group` in the membership half and
`mask` in the filter half, and debris-vs-debris, debris-vs-ragdoll and ragdoll-vs-debris stop
colliding by default (today `crates/physfx` has no group table at all: `git grep collision_groups
crates/physfx` is empty, so every body meets every body with Rapier's default groups).

**The float-world defaults to land later** (for the arXiv-sim lane): debris meets prop, debris
and world; ragdoll bones meet fighter, ragdoll bones and world; a soft body meets fighter, prop,
soft and world; cloth meets fighter, prop, world and cloth; cloth never meets debris.

## CLX6 — hair and rope meet colliders

**What it is.** `Strand::step` took no arguments at all: a hair lock and a rope had no collider
input, no world collision and no production caller. CLX6 gives the strand the same one-way
colliders the sheet has (the lock off the skull, the rope off the post) in its own Q16 math.

- **Donor**: `vrm-c/vrm-specification` — `specification/VRMC_springBone-1.0/README.md` ("You can
  set the collision between the end (sphere) of each section of SpringBone and the collision
  detection node (collider: sphere / capsule)", `colliderGroups`, `hitRadius`, and the reference
  sphere and capsule pseudocode), https://raw.githubusercontent.com/vrm-c/vrm-specification/master/specification/VRMC_springBone-1.0/README.md,
  read live 2026-10-20. Licence: **unverified** — no LICENSE at the repo root today (`LICENSE`
  and `LICENSE.md` both 404 on 2026-10-20), so this row is ideas only and no bytes.
- **Closes**: PX35 (the hair and rope half) and **no existing row** for hair collision — propose
  a new row "hair and rope collision".
- **Owned paths**: `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — prints
  `cloth_gap hair_lock nodes=16` and `cloth_gap a hair lock never enters a capsule` green.
- **Kill line**: a strand node inside the capsule after the run, or a strand tick that allocates.
- **Cost**: S (done in this lane).

## CLX7 — the cloth_gap baseline tape (the fixture, the numbers)

**What it is.** The D-CLOTH-1 denominator fixture (50 seeded drapes over capsule colliders plus a
folded-curtain set) was measured by a tape whose module the RZ7 rewrite deleted, so the numbers
that gated PX28 and PX29 are gone. CLX7 re-measures them in the f32 sheet and posts the receipt.

- **Donor**: the old tape in git history (read-only): `d6526e26` "474 PX27: cloth_gap baseline
  tape (1/1, digest x2, quality 99 A)" (the constants `GAP_SEED = 2040`, `GAP_DRAPES = 50`,
  the per-drape `Rng::fork(i)` streams, `gap_capsule`, `gap_drape`, `gap_digest`,
  `gap_folded_curtain`, `gap_self_gap`, and the receipt line
  `cloth_gap pen_max_raw=... drape=... self_gap_raw=... close_pairs=... ms=... alloc_calls=...
  digest_a=... digest_b=...`). Licence: our own code.
- **Closes**: PX27 (the baseline tape).
- **Owned paths**: `crates/physics/src/cloth.rs`.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` — the
  receipt lines print: `cloth_gap drapes=50 pen_max_m=0.0000 drape=30`, `cloth_gap fold
  start_gap_m=0.0000 close_pairs=0`, `cloth_gap alloc_calls=0`, `cloth_gap eight_fighters
  us_per_tick=203.7`, `cloth_gap sim_hash_on=89c5d0c905b72905 sim_hash_off=89c5d0c905b72905`.
- **Kill line**: the tape must not measure itself into the hash: any change in the sim hash
  caused by the measurement kills the row.
- **Cost**: S (done in this lane).

## CLX8 — ragdoll grandparent contacts and the 1 cm bar (PX9)

**What it is.** PX9's groups keep a bone away from its parent and its children, but not from its
grandparent, so the 50-fall bar still misses: the worst overlap is 4.53 cm (elbow_r/ankle_r) at
four solver iterations and the test stays ignored. CLX8 clears the grandparent in the group
filter and raises the solver iterations, then re-measures.

- **Donor**: `dimforge/rapier` 0.36.0 — `rapier3d-0.36.0/src/dynamics/soft_body/soft_body_coloring.rs`
  ("Greedy coloring of a soft body's elements: two elements sharing a particle never get the
  same color"), read live 2026-10-20 in the vendored cargo registry, Apache-2.0 — the idea is
  the coloured sweep the contact solver wants, and the same file's `soft_body_crossing_tests.rs`
  is the crossing test a bone pair wants. Second donor: `jrouwe/Jolt` (MIT, already mined) for
  the layer pair filter shape.
- **Closes**: PX9 (ragdoll vs ragdoll separation and the 1 cm bar).
- **Owned paths**: `crates/physics/src/joint_limits.rs`, `crates/physics/src/ragdoll.rs`.
- **Accept**: `cargo test -p engine-physics --lib ragdoll` green with the 50-fall test **not
  ignored**: it prints `worst bone overlap 0.0098 m` (under 1 cm) and the two-ragdoll separation
  tape still passes byte-equal twice.
- **Kill line**: the 1 cm bar stays open after the grandparent filter plus six solver iterations:
  the row dies and the contact model moves to the Jolt donor.
- **Cost**: M (not built in this lane: the lane owns cloth and layers).

## CLX9 — a per-body CCD switch for cloth and debris (PX10)

**What it is.** PX10's per-body CCD exists for the ragdoll bones (`ragdoll.rs:582`,
`powered.rs:453`, the one policy in `lod.rs:172`), but the float world has no per-body switch for
debris and the cloth has no continuous answer of its own beyond CLX2's sweep. CLX9 gives every
body in the float world the same switch.

- **Donor**: `dimforge/rapier` 0.36.0 — `rapier3d-0.36.0/src/dynamics/soft_body/soft_body_builder/soft_body_builder_settings.rs`
  (`additional_solver_iterations`, `additional_pgs_iterations`), read live 2026-10-20 in the
  vendored cargo registry, Apache-2.0.
- **Closes**: PX10 (per-body CCD enable for everything that is not a bone).
- **Owned paths**: `crates/physfx/src/fx.rs`, `crates/physfx/src/caps.rs` (physfx lane; the
  physics lane already has the bone half).
- **Accept**: `cargo test -p engine-physfx --lib ccd` green: a debris shard at 40 m/s never
  passes a 5 cm wall in 1,000 seeded runs, and a shard with the switch off does (the control).
- **Kill line**: the switch costs more than 0.1 ms per tick for 200 debris bodies.
- **Cost**: S.

## CLX10 — projectile vs world: the SegmentCast adapter over swept_cast (UW2)

**What it is.** `crates/combat/src/projectile.rs` steps rounds in Q16 with gravity and drag
tables and a spread fork from (tick, shooter, shot), but the live game hands it a `NullCaster`
("no scene cast"), so a round flies and hits nothing. UW2 wants the round to meet the world.

- **Donor**: the engine's own `crates/physics/src/query.rs` `swept_cast` (Q16, TOI then id,
  PX20) — read 2026-10-20; the composer adapter over `SegmentCast` is the UW2 plan's own shape.
- **Closes**: UW2 (projectile flight through a real cast).
- **Owned paths**: `crates/combat/src/projectile.rs`, `crates/arena/src/match_wire.rs` (combat
  and arena lanes; the physics lane only owns the detector).
- **Accept**: `cargo test -p engine-combat --lib projectile` green: 1,000-shot range hashes equal
  twice, the drop is within 2% of the table curve, and a round fired at a wall stops on the wall
  with the material id of the wall.
- **Kill line**: the adapter costs more than 0.3 ms for 2,048 rounds, or the range hashes move.
- **Cost**: M.

## CLX11 — bullet holes, chips, scorch and impacts (UC8, UW36)

**What it is.** `crates/render/src/damage.rs` does not exist and `games/urban/src/` holds only
`boot.rs`, so a bullet leaves no hole and an impact leaves no dust. The collision layer has to
hand the render lane the hit it already knows.

- **Donor**: the engine's own hit events (the `Cut`/`Break` event family in the plan) — no donor
  code; the rule is that the presentation reads the event and never feeds back.
- **Closes**: UC8 (surface damage) and UW36 (impacts and look).
- **Owned paths**: `crates/render/src/damage.rs`, `games/urban/src/impacts.rs` (render and game
  lanes; not the physics lane).
- **Accept**: `cargo test -p engine-render --lib damage` green: 2,000 holes in view inside the P6
  GPU budget, the golden hash unchanged, and the first bullet hole lands on the material the cast
  named.
- **Kill line**: the decal pass needs a pixel read or a second scene copy, or the golden moves.
- **Cost**: M.

## CLX12 — the physical gate receipt: the hash equal with the physical layer on and off (G11)

**What it is.** The finish-line gate G11 asks for a receipt: 8 ragdolls, 200 debris bodies and 6
limbs inside the frame budget, and the sim hash equal with the physical layer on and off. The
hash half is a rule every soft addition has to keep, so it wants its own row and its own proof.

- **Donor**: the engine's own `crates/physics/src/rapier.rs` snapshot/restore discipline (PH1,
  PX17) — read 2026-10-20; the float world reads authority events and never writes back.
- **Closes**: G11 (the physical gate; the receipt line).
- **Owned paths**: `crates/proof` (the receipt), `crates/physfx` (the float world).
- **Accept**: `cargo test -p engine-proof --lib golden_hash_untouched` green with the physical
  layer on and off: the golden `d57b5c8ae3096614` is byte-equal twice in both cases, and the
  receipt line reads `physfx: ragdolls=8 debris=200 limbs=6 ms=...`.
- **Kill line**: the golden moves when the physical layer is on, or the receipt cannot be
  printed in one run.
- **Cost**: S.

---

## What stays open, and who owns it

| row | what is open | why it stays open | the row that should own it |
|---|---|---|---|
| PX9 | the 1 cm bar: the worst bone overlap is 4.53 cm (elbow_r/ankle_r) at four solver iterations and the 50-fall test stays ignored | the group filter clears the parent and the children but not the grandparent, and four solver iterations cannot carry the chain; this lane does not touch the ragdoll | CLX8 (proposed above) |
| PX10 | per-body CCD for everything that is not a bone: debris and the float world have no per-body switch | the bone half landed (`ragdoll.rs:582`, `powered.rs:453`); the float world half is the physfx lane's file | CLX9 (proposed above) |
| UW2 | a round flies and hits nothing: the live game hands the projectile a `NullCaster` ("no scene cast", `crates/combat/src/projectile.rs:4`) | the flight is there; the cast adapter over `swept_cast` is a combat and arena change | CLX10 (proposed above) |
| UC8 | `crates/render/src/damage.rs` does not exist: no bullet holes, chips or scorch | the render lane owns the file; the physics lane only has to hand over the hit | CLX11 (proposed above) |
| UW36 | `games/urban/src/` holds only `boot.rs`: no muzzle flash, dust, splinters, casings or blood decals | the game lane owns the file | CLX11 (proposed above) |

Nothing in this list is a gap in the cloth layer any more: CLX1 to CLX7 are built and proven in
`crates/physics`.

---

## Proof (2026-10-21, the GitHub runner, `node tools/gh-check.mjs`)

`cargo test -p engine-physics --lib cloth_gap` — **8 passed, 0 failed** (run 39510345817). The
receipt lines the suite prints:

```
cloth_gap drapes=50 pen_max_m=0.0000 drape=30
cloth_gap fold start_gap_m=0.0000 close_pairs=0
cloth_gap gust crossed=0 pen_max_m=0.0000 swept_stops=6265
cloth_gap eight_fighters us_per_tick=203.7 (bar 500 us in release)
cloth_gap alloc_calls=0
cloth_gap sim_hash_on=89c5d0c905b72905 sim_hash_off=89c5d0c905b72905
cloth_gap hair_lock nodes=16
```

The same command in release (`cargo test -p engine-physics --release --lib cloth_gap`, run
39511329886) is 8 passed with the 8-fighter bar at **203.7 us per tick** (bar 500 us). The whole
crate is green: `cargo test -p engine-physics --lib` = **196 passed, 0 failed, 4 ignored**, and
`cargo clippy -p engine-physics --all-targets -- -D warnings` is clean.

**The golden-hash leg is green as of 2026-10-21.** It was blocked for a while by a foreign red,
not by this work: `crates/audio/src/heal.rs` (another lane's work at the time) added
`HealError::UnverifiedRecovery`, `HealError::ScopeMismatch` and `HealError::UnknownHeal`, and the
`engine-net` match over `HealError` was not exhaustive (`error[E0004]: non-exhaustive patterns`).
That red is gone, and `cargo test -p engine-proof --lib golden_hash_untouched` now reads
`test a9::tests::golden_hash_untouched ... ok` - `test result: ok. 1 passed; 0 failed; 0 ignored;
167 filtered out` - so the golden `d57b5c8ae3096614` (`crates/proof/src/lib.rs:60`,
`GOLDEN_600`) did not move. Nothing in this lane touches `engine-net`, `engine-audio` or
`engine-proof`. The second proof that the cloth layer never reaches the hash stays inside the
green suite: `cloth_gap the sim hash is equal with the cloth layer on and off` marches the same
rigid bodies 240 ticks with and without a cape stepping every tick and reads the same
`Space::state_hash` (89c5d0c905b72905 both ways) - the G11 rule, proven at the crate level.

## What the shipped shape actually is (for the next builder)

- **`Cloth::step(&[ClothCollider])` order** (one substep): integrate, `cap_speed`, tear, link
  passes, tethers, Max Distance, the swept collider test, the one-way push-out, then the
  intercollision pass last (the PX29 shape: the tick rests separated).
- **The swept test** uses the hashed tick's own Q16 detector, `crates/physics/src/query.rs`
  `swept_cast` (PX20): a particle whose path crosses a shell stops at the time of impact. A
  particle that starts on the shell and moves in is held in place instead, because the push-out
  would send it out on the far side.
- **The intercollision pass**: an intercollision particle set (a bitset over particle ids), a
  preallocated spatial hash (the cell edge is the separation), the linked neighbours (structure
  plus shear) skipped so the links never fight the push, four passes with the sweep direction
  alternating, and the separation = half the smaller rest spacing (Unity's rule: the
  inter-collision distance stays smaller than the smallest distance between two particles).
- **The layers**: `crates/physics/src/layers.rs` is now the group + accept-mask table; the old
  one-way `blocks` read stays, so `crates/arena/src/kit_zone.rs` and the door verbs are untouched.


## Addendum 2026-10-21: the cloth file is split, and every cast has a name

The CLX rows above landed as one 3030-line `crates/physics/src/cloth.rs`. This pass splits that
file and gives every number crossing a name. Nothing about the behaviour moves: the eight
`cloth_gap` tests, the whole crate and the sim hash all read the same as before the split.

### The new file layout

`crates/physics/src/cloth.rs` is gone. `crates/physics/src/cloth/` holds seven files, and the
biggest production file is now `mod.rs` at 788 lines (the rule is 1500):

| file | lines | what lives there |
| --- | --- | --- |
| `mod.rs` | 788 | the sheet: `Cloth`, `ClothParams`, `ClothTier`, `ClothKnobs`, `ClothGrid`, `ClothError`, the private constants, and the step (`step`, `substep`, `solve_links`, the tethers, the pins) |
| `collide.rs` | 259 | `ClothCollider`, the one-way push-out, the swept test (`sweep_colliders`), `cap_speed`, `touched`, `cloth_meets` |
| `intercollision.rs` | 251 | the intercollision set, the spatial hash (`build_hash`, `hash_shape`), the separation pass (`resolve_self`, `self_fix_pass`) |
| `max_distance.rs` | 54 | the Max Distance limit pass (`solve_max_distance`) |
| `strand.rs` | 432 | `Strand`, `StrandParams`, `StrandError`, the Q16 hair and rope |
| `convert.rs` | 47 | the number crossings, one named function per direction |
| `tests.rs` | 1281 | the cloth tests: the old `#[cfg(test)] mod tests` body, moved as it was |

Public names are unchanged, so no caller changes: `cloth/mod.rs` re-exports `ClothCollider`,
`Strand`, `StrandParams`, `StrandError` and `MAX_STRAND_NODES`, and `Cloth` keeps its path. The
constants stay private to `cloth/mod.rs` and the child modules reach them through `super::`; a
cross-module item is `pub(in crate::cloth)`, so nothing new became public outside the crate.

### The conversions, and what each one replaces

| helper | direction | what it replaces |
| --- | --- | --- |
| `to_fixed(f32) -> Fixed` | metres to Q16 fixed | the inline `(v * 65_536.0).round() as i64` of the swept cast setup |
| `to_fixed_point([f32; 3]) -> [Fixed; 3]` | a point to Q16 fixed | the same cast, three times over |
| `from_fixed(Fixed) -> f32` | Q16 fixed to metres | `bits.0 as f32 / 65_536.0` |
| `cell_index(f32) -> usize` | a coordinate to a hash cell index | `(v as usize)` |
| `index(u32) -> usize` | a hash count or cursor to an index | `(n as usize)` |
| `cell_count(f32, f32) -> usize` | an extent to a number of cells | `((e / cell).ceil() as usize)` |

Each one keeps the old number exactly, which is why the sim hash and the tape are unchanged:

- `to_fixed` rounds half away from zero in f32 before it narrows, so the fixed value is bit for
  bit what the inline multiply-and-round gave; NaN becomes zero and a value past `i64` saturates.
- `from_fixed` divides by a power of two, so it is exact inside the world limit.
- `cell_index` truncates toward zero and puts a negative or NaN coordinate in cell zero, the way
  the bare cast did.
- `cell_count` rounds up, and a negative or NaN extent is zero cells.

The local `q16`, `q16_point` and `metres` helpers are gone. The hash axis is written out twice
(`MAX_HASH_AXIS` and `MAX_HASH_AXIS_F32`) so the cell arithmetic needs no cast, and the saturating
narrowing comes from `engine_core::checked`.

### The proof

- `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth_gap` - `test result: ok. 8
  passed; 0 failed; 0 ignored; 192 filtered out` (the PC: GitHub was unreachable that minute, and
  the tool falls back to the PC by design)
- `node tools/gh-check.mjs cargo test -p engine-physics --lib` - `test result: ok. 196 passed; 0
  failed; 4 ignored` (runner run 39528318581)
- `node tools/gh-check.mjs cargo clippy -p engine-physics --all-targets -- -D warnings` - clean, no
  warnings
- `node tools/rust-quality.mjs --crate physics --owned crates/physics/src/cloth --owned
  crates/physics/src/layers.rs` - `RESULT PASS: score 100 (A), 0 ratchet regressions over 1
  crate(s)`; the same command with `--tighten` - `TIGHTEN engine-physics dup_fns 1 -> 0` (the
  split killed the last duplicated function body in the crate) and `RESULT tightened 1 entry`
- the release tape, same values as before the split: `cloth_gap drapes=50 pen_max_m=0.0000
  drape=30`, `cloth_gap fold start_gap_m=0.0000 close_pairs=0`, `cloth_gap gust crossed=0
  pen_max_m=0.0000 swept_stops=6265`, `cloth_gap alloc_calls=0`, `cloth_gap
  sim_hash_on=89c5d0c905b72905 sim_hash_off=89c5d0c905b72905`, `cloth_gap hair_lock nodes=16`
- `cargo test -p engine-proof --lib golden_hash_untouched` - `test a9::tests::golden_hash_untouched
  ... ok`; `test result: ok. 1 passed; 0 failed; 0 ignored; 167 filtered out` - the golden
  `d57b5c8ae3096614` did not move

### The one number that moves with the machine, not with the code

The `cloth_gap eight_fighters` tape measures wall-clock time, so it reads the machine, not the
cloth. On an idle PC the pre-split code read `us_per_tick=203.7` (bar 500). While this pass ran
the PC sat at 100% CPU on other lanes' processes, and the budget test read 492.4 (passing) in the
first release run and then 522.2, 607.2, 750.7 and 828.1 in the next ones. To show the split is
not the cause, the pre-split code was rebuilt beside it in a scratch worktree - the 3030-line
monolith with the inline casts put back - and the two were measured in the same minutes, back to
back:

| run | split (this pass) | monolith (pre-split) |
| --- | --- | --- |
| round 1 | 704.7 us | 735.9 us |
| round 2 | 700.1 us | 702.8 us |

Same load, same code path, the same numbers inside the noise: the split and the named
conversions cost nothing. The 500 us bar is a wall-clock budget and the PC was saturated; on an
idle machine the same tape reads 203.7 us, as it did before the split.
