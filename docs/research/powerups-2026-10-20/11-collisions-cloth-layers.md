# 11 — collisions, cloth and layers (powerups wave, 2026-10-20)

Lane question: the engine's collisions are the weakest physical layer, cloth has none. Write the
whole collision story and the donor list to build it.

## The answer

The story is not "cloth has no collision". Today `crates/physics/src/cloth.rs` takes colliders:
`Cloth::step(&[ClothCollider])` (OBSERVED, cloth.rs:596) and `ClothCollider` is a one-way capsule or
sphere (OBSERVED, cloth.rs:251-311). **The brief is wrong and so is `docs/research/COMPARE.md`
D-CLOTH-1**, whose line "collision UNKNOWN (no collider input in cloth.rs; only anchors)" was frozen
on 2026-09-30 against the *old* cloth.rs; the RZ7 rewrite (331dca1b, named in the PX27/PX28/PX29
tick audits) brought colliders back. The real story is narrower and worse:

- **Cloth vs characters: 1 collider, 1 body.** The only production caller is
  `crates/game/src/fighter_look.rs:373-413`: one torso capsule for the wearer's own cape, then
  `sheet.step(&[body])` (OBSERVED). No other fighter's capsule, no world geometry, no weapon, no
  debris, no ragdoll bone. Two fighters walk through each other's capes.
- **Cloth self-collision: gone.** `build_links` (OBSERVED, cloth.rs:1226-1253) builds structure plus
  two shear diagonals and nothing else. There is no particle-vs-particle test, no spatial hash, no
  intercollision particle set, anywhere in the file. PX29 was ticked on 2026-10-05 ("cloth_self 2/2")
  against code the RZ7 rewrite later deleted; the tick audit says so in plain words: "the named
  self-collision tests no longer exist" (OBSERVED, `docs/archive/SPRINT-done-2026-10-10.md:1094-1101`).
  **PX29's substance is open again.**
- **Hair and rope: zero collision, and no caller.** `Strand::step(&mut self)` takes no arguments at
  all (OBSERVED, cloth.rs:1095) — no collider input exists. The physfx rope
  (`crates/physfx/src/tether.rs`) has only a flat floor clamp `FLOOR_M` 0.02 m (OBSERVED, tether.rs:22,
  214). `Strand` has no production caller: `git grep` finds it only inside cloth.rs's own tests
  (OBSERVED).
- **Pinning is a clamp, not a Max Distance gradient.** `Tether { node, pin, max }` (OBSERVED,
  cloth.rs:326-330) ties each free particle to its *nearest pin* with `max` = flat rest distance,
  solved as a hard positional clamp (OBSERVED, `solve_tethers`, cloth.rs:751-765). Unity's Max
  Distance is a per-particle value you can paint; ours is one scalar per particle with no gradient
  and no authoring path (see donor row 4).
- **No CCD for cloth.** The only guard is `MAX_SPEED` 30 m/s (OBSERVED, cloth.rs:37, applied at
  681-684). DERIVED: at 30 m/s and a 1/120 s substep a particle moves 0.25 m, five times a 5 cm
  shield, so a fast gust can push a particle straight through a collider.
- **Layers: 5 bits, no cloth.** `crates/physics/src/layers.rs` owns FIGHTER, PROP, DEBRIS, SENSOR,
  WORLD and a symmetric "blocks" row per layer; `blocks(a,b) = blocks_mask(a) & b != 0` (OBSERVED).
  There is **no CLOTH bit and no SOFT bit**, and `crates/physfx` has **no group table at all**
  (`git grep collision_groups crates/physfx` → 0 hits, OBSERVED), so in the float world every body
  meets every body with Rapier's default groups.
- **Ragdoll vs ragdoll: filtered, not solved.** `joint_limits::bone_groups` (OBSERVED,
  joint_limits.rs:229-249) gives each bone its own membership bit and clears its parent and children
  from its filter, so bone *i* of one ragdoll meets bone *i* of another. `clinch.rs` then parts
  fighters apart (OBSERVED, lib.rs:9-11). **PX9's own 1 cm bar is still open**: the 50-fall test stays
  ignored with a 4.53 cm worst overlap (elbow_r/ankle_r) at four solver iterations (OBSERVED,
  `docs/SPRINT.md` PX9 row). Grandparent pairs are not filtered — that is the named open cause.
- **CCD per body exists but is not per-layer.** `lod::ccd_for` is the one policy (OBSERVED,
  lod.rs:172-186): bones always on, weapons and projectiles above 20 m/s, soldiers and duelists never.
  `ragdoll.rs:582-593` and `powered.rs:453-470` flip a single bone and carry the flag in the
  snapshot. PX10's ask (per-body CCD enable) is therefore *mostly* landed; what is missing is a
  per-body switch for cloth particles and for debris.
- **Swept casts exist and are in the hash.** `query::swept_cast` (Q16 `Fixed`, TOI-sorted, id-tiebroken)
  landed with PX20 (OBSERVED, query.rs:546) and is already used inside the hashed tick by the spear
  hit test (`crates/arena/src/spear.rs:499`, OBSERVED). This is the detector any new continuous
  collision work must use.
- **Projectile flight exists; projectile collision does not.** `crates/combat/src/projectile.rs` steps
  rounds in Q16 with gravity and drag tables and a spread fork from (tick, shooter, shot) (OBSERVED,
  projectile.rs:1-30), but the live game uses the local `NullCaster` — "no scene cast" (OBSERVED,
  projectile.rs:4). A round flies and hits nothing.
- **Bullet holes and decals are not built.** `crates/render/src/damage.rs` does not exist and
  `games/urban/src/` holds only `boot.rs` (OBSERVED by glob). UC8 and UW36 have no code.
- **Nav dirty tiles after a break already work.** `destruction::bond_grade::break_record` returns
  sorted unique `dirty_tiles` (OBSERVED, bond_grade.rs:283-352), `nav::TileRegistry::invalidate`
  marks and `rebake` bumps exactly those (OBSERVED, nav/src/tiles.rs:448-503), and
  `nav::links::rebake_links` names only the links on dirty tiles (OBSERVED, nav/src/links.rs:108).
  That part of the destruction story is closed; DX13's snapshot bytes also landed
  (`crates/destruction/src/bonded/snapshot.rs`, OBSERVED).

### The architectural answer: which collisions belong in the hashed tick

The hashed tick is integer/Q16 and one hash per tick (golden `d57b5c8ae3096614`). The soft layer is
float and never hashed: `crates/physfx` is a float Rapier world, `crates/physics/src/cloth.rs` is f32
Verlet and says so in its own first lines ("Presentation only: floats, never hashed, never read by
the simulation, so a cloth bug can look wrong but can never desync a replay", OBSERVED, cloth.rs:5-6).

So the split is a rule, not a preference:

1. **In the hashed tick (integer, stable order, allocation-free):** anything that can change a match
   outcome — a projectile hitting a fighter, a weapon sweep cutting a limb, a ragdoll contact that
   decides a fall, a break that frees a chunk and a nav tile. The only detector allowed is a
   closed-form integer test: `query::swept_cast` (Q16), the combat sweep (integer mm), the layer
   table (bit math). **Determinism cost:** every narrowphase must be exact integer arithmetic with a
   total order (TOI then id) and must land in the hash, so a contact changes the golden. A float
   narrowphase here is a golden-splitting bug. CPU cost is the budget: 2,048 casts ≤ 1 ms per tick
   (the UW2 plan text, OBSERVED).
2. **In the float layer (physfx, or cloth.rs):** anything that only moves a picture — capes,
   banners, hair, ragdoll sprawl, debris tumble, the blast. **Determinism cost: zero.** A float bug
   looks wrong and cannot desync a replay. The cost is a *visual* one only, plus the rule that the
   float layer may never feed back: a cape must not move the fighter's capsule, and the float world
   must never be read by the sim.
3. **The rule that keeps the two apart:** a soft contact that matters for gameplay (a net that
   blocks a charge) is not a soft solve. It is an integer event — the same shape as the authority
   `BreakEvent` family — produced by the hash-layer cast and consumed by the sim.

### Layers as data, today

`crates/physics/src/layers.rs` is the right shape already: one bit per layer, one symmetric "blocks"
row per layer, a test that symmetry holds for every pair (OBSERVED, layers.rs:116-181). It is missing
three things: a CLOTH bit, a SOFT bit, and the ability to say "this collider is on group G and
accepts mask M" instead of only "layer L blocks layer K". Bullet's per-proxy
`m_collisionFilterGroup` + `m_collisionFilterMask` pair (donor row 3) is the shape that scales,
and Rapier's `InteractionGroups` (already used by `joint_limits`) is the same idea in the float
world, where we currently have nothing.

## SPEC — build this

**Solver choice.** One cloth solver, not two. Keep `crates/physics/src/cloth.rs` f32 Verlet as the
sheet solver (it already passes PX35's 25 % + 64 raw stretch bar and the zero-allocation step test,
OBSERVED in the PX35 row) and add three passes to it. Do **not** add a Rapier soft body for the cape:
a cape here is a procedural 6×10 grid, not an authored skinned cloth mesh, so a Rapier soft body
would need a second cloth path, a second asset path, and a second float world for the same picture.
If a *real* soft body (a volumetric banner, a muscle) is ever wanted, it belongs in `crates/physfx`
where Rapier 0.36 already ships one (donor row 1) and the hash can never see it.

**Data layout** (all in `crates/physics/src/cloth.rs`, all float, all preallocated):

- `ClothCollider` gains `layer: u32` (a bit from `layers.rs`) and `max_distance: f32` (surface
  penetration). `push_out` stays one-way: cloth pushes nothing back (OBSERVED, cloth.rs:249-250).
- `Cloth` gains `self_set: Vec<u64>` — a bitset over particle ids: the **intercollision particle
  set**. Only particles in the set collide with each other. This is the exact Unity semantics
  (donor row 4) and the exact Bullet `Cluster::m_collide` semantics (donor row 2).
- `Cloth` gains `max_distance: Vec<f32>` — one value per particle, defaulting to the tether reach.
  `solve_tethers` becomes a gradient solve (pull the particle back by a fixed fraction of the
  overshoot, not a hard clamp), so a per-particle Max Distance is expressible and a gradient across
  the sheet is a data row, not a code path.
- `Cloth` gains a spatial hash for the self-collision pass: cell size = the inter-collision
  distance, buckets in a preallocated `Vec<u32>` with a count array, rebuilt each substep. The
  inter-collision distance must be **smaller than the smallest particle spacing** or the sheet
  jitters (donor row 4, exact wording).
- A swept particle-vs-collider test: after the Verlet move, test the segment from the previous
  position to the new one against each collider and, on a crossing, place the particle at the time of
  impact. This is the small continuous-collision fix that closes the tunnelling gap; the fast-cloth
  literature says a full CCD pass is the expensive part and most of it can be replaced by a cheap
  barrier (donor row 5).
- `crates/physics/src/layers.rs` gains `CLOTH` and `SOFT` bits and changes `blocks` from
  "symmetric row table" to "group + mask per collider, symmetric by construction". Keep the existing
  symmetry test and add a test that cloth never blocks debris and that the mask is the exact inverse
  of the group.
- `crates/physfx` gets the same table applied as Rapier `InteractionGroups` (the mechanism
  `joint_limits.rs` already uses), so debris stops colliding with debris by default and ragdoll
  bones stop colliding with the debris they made.

**Test fixture** (the PX27 fixture the board already names, extended):

- 50 seeded cape drapes over capsule colliders, seed pinned, 60 Hz fixed dt, wind script.
- A folded-curtain intercollision set: one sheet folded on itself, particles in the set, assert
  zero self-penetration.
- A fast-gust case: 30 m/s wind, assert no particle crosses a collider in one substep.
- An 8-fighter case: 8 capes, assert the cloth budget.
- A hair case: a `Strand` over a capsule, assert the strand never enters it.

**Accept line** (one line, so a builder can post it):

> `cargo test -p engine-physics --lib cloth_gap` green: max penetration ≤ 1 cm over the 50 drapes,
> zero self-penetration on the folded-curtain set, no particle crosses a collider in the fast-gust
> case, cloth ≤ 0.5 ms for 8 fighters, zero heap allocations in `step`, golden
> `d57b5c8ae3096614` unchanged x2, **and the sim hash equal with the cloth layer on and off** (the
> G11 rule that the float layer never reaches the hash).

**What this closes and what it does not.**

- Closes: **PX28** (one-way coupling — needs the full collider set and the layer, not one torso
  capsule), **PX29** (self-collision — re-open; the code is gone), **PX27** (the baseline tape — the
  `cloth_gap` filter is gone with the old file; the numbers must be posted again), and the
  Max-Distance part of **PX35** (the tether is a clamp, not a gradient).
- Does **not** close: **PX9** (the 4.53 cm ragdoll overlap needs grandparent contacts or a stiffer
  solver, not cloth), **PX10** (per-body CCD is landed for bones; a cloth/debris switch is new
  work), **PX12** (DAT coupling is a ragdoll seam), **PX16/PX17** (snapshot bytes; cloth never
  checkpoints by the presentation rule), **PX20** (already landed, reuse it).
- **RZ1/RZ2/RZ3** are the float layer that this spec's cloth work must not pollute: the hash must
  stay equal with the physical layer on and off (the G11 gate, OBSERVED,
  `docs/plan/finish-line.md:22`).
- **UW2** needs a `SegmentCast` adapter over `query::swept_cast` plus a composer adapter, because
  the live game uses `NullCaster` — that is a hashed-tick row, not a cloth row.
- **UC8** and **UW36** have no file at all: `crates/render/src/damage.rs` and
  `games/urban/src/impacts.rs` must be created; they are render/game rows that consume a hit event
  this layer would produce.
- **DX13** is archived and its snapshot hunks landed (`crates/destruction/src/bonded/snapshot.rs`).
  Nothing to do.

## Donor table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Rapier 0.36 soft bodies: a cloth/soft solver with rigid contacts, self-contacts, clusters, tearing and deterministic graph colouring — already a dependency, never used | `dimforge/rapier` 0.36.0, Apache-2.0 (licence read live in the vendored `rapier3d-0.36.0/Cargo.toml`, 2026-10-20); crates.io + local cargo registry | Apache-2.0 | PX29 (re-open), PX28, RZ1/RZ9, G11 | `rapier3d-0.36.0/src/dynamics/soft_body/soft_body_builder/soft_body_builder_settings.rs` (`self_contacts`, `particle_radius`, `surface_collider`, `pinned_particles`, `tear_strain`, `cell_model`), `.../soft_body_cluster.rs` (`SoftBodyCluster` = a particle set backed by a proxy rigid body = our intercollision particle set), `.../soft_body_crossing_tests.rs`, `.../soft_body_coloring.rs` (deterministic colour sweep, same idea as LADDER line 144) | M | a Rapier soft cape cannot hold PX35's 25 % + 64 raw stretch bar, or costs over 0.5 ms at 8 fighters, or the float world leaks into the hash | PORT (into `crates/physfx` only, never the tick) |
| 2 | Bullet soft body **clusters** as intercollision particle sets, with a per-cluster self-collision switch and an impulse cap | `bulletphysics/bullet3`, `src/BulletSoftBody/btSoftBody.h` (read live 2026-10-20): `struct Cluster { m_collide, m_maxSelfCollisionImpulse = 100.f, m_selfCollisionImpulseFactor = 0.01f, m_clusterIndex }`, `fCollision::CL_SELF` (cluster self collision), `CL_SS` (cluster vs cluster), `VF_SS` (vertex vs face soft-soft) | zlib (licence text in the file header, read live 2026-10-20) | PX29, PX28 | `src/BulletSoftBody/btSoftBody.h` — the *idea* only: a per-set `collide` flag, a per-set impulse cap, and vertex-vs-face (not vertex-vs-vertex) soft contact. Write our own f32 pass; port no bytes | S | the self-collision pass costs over 0.5 ms for a 60-particle cape, or the impulse cap cannot keep the sheet from jittering | PORT (idea; our own code) |
| 3 | Bullet per-proxy **filter group + mask** — the collision layer matrix as data | `bulletphysics/bullet3`, `src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h` (read live 2026-10-20): `enum CollisionFilterGroups { DefaultFilter=1, StaticFilter=2, KinematicFilter=4, DebrisFilter=8, SensorTrigger=16, CharacterFilter=32, AllFilter=-1 }`, plus `m_collisionFilterGroup` and `m_collisionFilterMask` per proxy | zlib (licence text in the file header, read live 2026-10-20) | PX28, PX29, PX10, G11, RZ1 | `src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h` — replace `layers.rs`'s symmetric "blocks" table with a group bit + an accept mask per collider; add `CLOTH` and `SOFT` bits; apply the same pair in `crates/physfx` as Rapier `InteractionGroups` | S | the layer table change breaks the existing symmetry test or the kit-zone fixture (`crates/arena/src/kit_zone.rs:123`) | PORT (idea; our own code) |
| 4 | Unity Cloth — the written spec for intercollision particle sets, Inter-Collision Distance/Stiffness, Max Distance gradients, Use Continuous Collision, virtual particles | `docs.unity.com/en-us/engine/6000.5/manual/physics-section/physics-overview/class-cloth` (read live 2026-10-20): "Inter-Collision Distance — the diameter of a sphere around each particle … should be smaller than the smallest distance between two particles"; "use self collision indices to reduce the number of particles that collide with each other"; Max Distance and Surface Penetration painted with Select/Paint/Gradient; "Use Continuous Collision"; "Use Virtual Particles" (one per triangle); Solver Frequency; Sleep Threshold; cloth reacts to bodies and never pushes back | PROPRIETARY (Unity docs) | PX27, PX28, PX29, PX35 | No file to port (docs only). Use it as the acceptance text: our `self_set` bitset = their intercollision particle set; our `max_distance` row = their Max Distance; our swept particle test = their Use Continuous Collision | S | the spec needs behaviour Unity does not document (it does not say how the sets are built) | RESEARCH-ONLY (use as the spec text) |
| 5 | Non-distance barrier cloth: a barrier that replaces most of the expensive CCD with a cheap partial pass | arXiv 2403.19272 v4, "Efficient GPU Cloth Simulation with Non-distance Barriers and Subspace Reuse" — abstract page https://arxiv.org/abs/2403.19272 read live 2026-10-20; full text https://arxiv.org/html/2403.19272v4 read live 2026-10-20: the non-distance barrier "allows a new way to integrate collision resolution", and it "replace[s] a large portion of traditional CCDs with the partial CCD procedure, which is much less expensive" | arXiv preprint (research only) | PX29, PX12 | No file. Use it to justify the cheap swept-particle test instead of a full CCD pass, and to justify a *distance-independent* self-collision rule (a particle is separated when it is inside another, not when it is within epsilon) | S | the swept-particle test still lets a particle cross a collider under the fast-gust case | RESEARCH-ONLY |
| 6 | I-Cloth: incremental continuous collision detection with spatial hashing for cloth | `min-tang.github.io/ICloth/` (page live 2026-10-20; abstract read): "incremental continuous collision detection algorithm that keeps track of deforming vertices and combine it with spatial hashing" | Research site (no licence read) | PX29 | No file. Use the two ideas: reuse the previous position to bound the swept test (incremental CCD), and hash the particles so the self-collision pass is not O(n²) | S | the hash rebuild per substep allocates, or the sheet cannot be made to stop jittering | RESEARCH-ONLY |
| 7 | PSCC: parallel self-collision culling with spatial hashing and normal cones | `min-tang.github.io/home/PSCC/` (page live 2026-10-20): spatial hashing + normal cone culling, 6–8× speedup over prior GPU self-collision | Research site (no licence read) | PX29 | No file. Use the *normal cone* idea on the CPU: skip particle pairs whose separation direction is not near the sheet normal (folded layers) — the cheap reject that keeps a folded curtain cheap | S | the normal-cone reject misses a real self-penetration in the folded-curtain fixture | RESEARCH-ONLY |
| 8 | Godot SoftBody3D — per-object layer and mask, pin list, and a collision exception list | `docs.godotengine.org/en/4.4/classes/class_softbody3d.html` (read live 2026-10-20): `collision_layer` (layers it is in), `collision_mask` (layers it scans), `parent_collision_ignore`, `set_point_pinned`, `add_collision_exception_with`; the page itself warns "many known bugs … not recommended … for things that can affect gameplay" | MIT (Godot docs) | PX28, PX29, G11 | No file (docs). Use it as a second layer-matrix shape: a *scans* mask plus an exception list, which is what a cape needs (ignore its own fighter, ignore debris) | S | the exception list has no place in our integer layer table | RESEARCH-ONLY |
| 9 | Unreal Chaos cloth: a cloth asset per skeletal-mesh section, a Physics Asset for the character's collision, and painted Max-Distance-style weights (Brush/Gradient/Smooth/Fill, 0 = cannot move, 100 = may move one metre) | `dev.epicgames.com/documentation/en-us/unreal-engine/clothing-tool-in-unreal-engine` (read live 2026-10-20) | PROPRIETARY (Epic docs) | PX35, PX27, QL3 | No file. Use the *weight scale* idea for our per-particle `max_distance` row: 0 = pinned, 100 = one metre of give, interpolated across the sheet | S | the paint weights need an editor panel, which decision D4 forbids | RESEARCH-ONLY |
| 10 | VRM 1.0 spring-bone collider groups: a chain's end sphere against named collider groups, with `hitRadius` and a written sphere/capsule pseudocode | `vrm-c/vrm-specification`, `specification/VRMC_springBone-1.0/README.md` (read live 2026-10-20): "You can set the collision between the end (sphere) of each section of SpringBone and the collision detection node (collider: sphere / capsule)"; `colliderGroups`; `hitRadius`; a reference sphere and capsule collider implementation in pseudocode | **unverified**: no LICENSE at the repo root today (`LICENSE` and `LICENSE.md` both 404 on 2026-10-20) | PX35 (hair and rope), no row for hair — propose a new row "PX-cloth-hair collision" | No file. Use the pseudocode shape for `Strand`: give `Strand::step` a collider list and a hit radius so a hair lock meets the skull | S | the licence stays unreadable, or `Strand` stays without a production caller | RESEARCH-ONLY (licence unverified, no port) |
| 11 | Müller's XPBD cloth: the donor everyone cites for "cloth with colliders" | `matthias-research/pages`, `tenMinutePhysics/14-cloth.html` (read live 2026-10-20): MIT header; XPBD stretching + bending, `numSubsteps: 15`, and a mouse *grabber* (a pinned particle) — **no sphere collider, no particle-particle collision, no self-collision** (grep for hash/collide/sphere over the page: 0 hits beyond `computeBoundingSphere` and the grabber) | MIT | none — this is a correction of `docs/research/steals-2026-10-11/anim-feel-steals.md` row 12 | Nothing to port. Our `cloth.rs` already has more (colliders, tethers, sleep, tiers, zero-alloc) | S | — | REJECT (it is not a collision donor; our sheet is already ahead of it) |
| 12 | NVIDIA FleX / PhysX cloth (the reference for authored cloth self-collision) | NVIDIA FleX and PhysX SDKs — proprietary, no source to read | PROPRIETARY | none | Nothing | L | — | REJECT (proprietary; the same ground is covered by rows 1-7) |

## Rejections

- **`jrouwe/Jolt`** (MIT) — in the RULES "already mined" list. Jolt's `ObjectLayerPairFilterTable`
  is the same shape as donor row 3 and adds nothing we do not already get from Bullet's
  group/mask pair plus Rapier's `InteractionGroups`. Not re-mined.
- **`dimforge/rapier` as a whole** — in the "already mined" list; row 1 is a CORRECTION, not a new
  find: the tree holds an **empty** `SoftBodySet` (`crates/physics/src/rapier.rs:633-657`,
  `ragdoll.rs:323-512`, `powered.rs:256-369`, OBSERVED) and the soft-body module it points at has
  `self_contacts`, `particle_radius`, `surface_collider` and `SoftBodyCluster`, none of which we use.
- **`box3d-rust`** — already mined; a 2D solver, no cloth.
- **`InteractiveComputerGraphics/PositionBasedDynamics`** (MIT) — already a LADDER CARD (line 220)
  for presentation-only flesh jiggle. Its cloth demo has colliders, but we do not need an XPBD
  rewrite: our f32 Verlet sheet already passes PX35's stretch bar and the zero-alloc test.
- **`ManevilleF/bevy_silk`** (MIT) — already in `docs/research/steals-2026-10-11/engine-landscape.md`
  (pinning by vertex id / colour / predicate). It is a pinning donor, not a collision donor.
- **`Unity-Technologies/UnityCsReference`** — `Runtime/Export/Cloth/ClothStructure.cs` **404s today**
  (2026-10-20), and the GitHub contents API returns 403 from this machine, so no exact Unity source
  file could be named. The Unity *docs* (row 4) carry the same semantics and were read live.
- **Provot 1995, "Deformation of Rods and Cloth"** (the classic collision-image self-collision
  method) — `https://www.graphics.cornell.edu/~srm/publications/Provot-1995.pdf` **404s today**
  (it serves an unrelated page), so no claim is made from it.
- **Müller's hair paper** (VRIPHYS 2006) — `matthias-research.github.io/pages/publications/papers/2006-VRPHYS-hair.pdf`
  **404s today**. The VRM spec (row 10) carries the same collider idea and was read live.
- **IEEE paywalled papers** — "A fast self-collision detection method for cloth animation based on
  constrained particle-based model" (ieeexplore 4414542) and the 2024 IEEE Access remeshing paper:
  abstract pages only, no open text to port. Not mined further.
- **Deep neural cloth collision detection** (doi 10.1155/2024/7889278) — a learned detector; the
  engine has no training path and no model pipeline for it.
- **Bullet `btSoftBody` full port** — Bullet's soft body is a whole solver (clusters, SDF, sparse
  SDF, plastic deformation). We want three ideas from it (rows 2 and 3), not 10,000 lines of C++.
- **Unreal/Unity/Godot as code donors** — comparison only by the rules; no port.
- **`crates/destruction/src/**` and `crates/nav/src/**`** — re-read today, no collision gap found:
  break → dirty tiles → tile registry → link rebake already works (bond_grade.rs:283-352,
  nav/src/tiles.rs:448-503, nav/src/links.rs:108). Not a donor question; reported here so nobody
  re-mines it.

## Top 3

1. **Rapier 0.36 soft bodies** — https://github.com/dimforge/rapier (Apache-2.0; the vendored
   `rapier3d-0.36.0` in the cargo registry, read 2026-10-20). Closes **PX29** (self-collision and
   intercollision particle sets, via `SoftBodyCluster`) and **PX28** (cloth-vs-rigid, via
   `surface_collider` / `particle_radius` / `self_contacts`), and gives the float layer a real soft
   body for **RZ1/RZ9/G11** without touching the hash. Cost **M**, one week.
2. **Bullet `btSoftBody.h` clusters** — https://github.com/bulletphysics/bullet3 (zlib, read live
   2026-10-20). Closes **PX29**'s intercollision particle set on our own f32 sheet: a per-cluster
   `m_collide` flag, a per-cluster self-collision impulse cap, and vertex-vs-face contact instead of
   the O(n²) vertex-vs-vertex test. Cost **S**, a day.
3. **Bullet `btBroadphaseProxy.h` filter group + mask** — same repo and licence. Closes the layer
   data gap behind **PX28/PX29/PX10/G11**: one group bit and one accept mask per collider, plus the
   `CLOTH` and `SOFT` bits `crates/physics/src/layers.rs` lacks, applied to `crates/physfx` as
   Rapier `InteractionGroups`. Cost **S**, a day.
