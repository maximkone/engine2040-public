# Physics, destruction and ragdoll: five steals from real donor source (2026-10-23)

Question this note answers: which five mechanics in Jolt, Rapier, Avian, Box2D v3 and
Blast are worth porting into `crates/physics`, `crates/destruction` and `crates/combat`,
and what does engine2040 already have? Every donor claim has a pinned 40-hex sha and a
quoted line. Every local claim has a `file:line`. Nothing here was built, run or tested.

## Method

Commit shas came from `git ls-remote <repo> HEAD` on 2026-10-23, then each file was
fetched at that sha and read line by line. `Jondolf/avian` now redirects to
`avianphysics/avian`, and the crate root moved from `crates/avian3d/` to `src/`.

| Repo | Pinned sha | Files read |
|---|---|---|
| jrouwe/JoltPhysics | `6f8bc72fd5fc6a58a52a7fbfe49ca7b5170b4990` | `Jolt/Physics/PhysicsSettings.h` (settings and defaults), `Jolt/Physics/Character/CharacterVirtual.cpp` (2018 L: slopes, stairs, stick-to-floor), `README.md` |
| dimforge/rapier | `475caa3ff4646fd9a796b67d14116256f4962bc7` | `src/control/character_controller.rs` (1497 L: autostep, snap, collision impulses) |
| avianphysics/avian | `a693f2b7cec0955770fabfa45a8e0fae355c449f` | `src/dynamics/ccd/mod.rs` (1202 L: swept vs speculative CCD and its caveats), `src/dynamics/solver/islands/sleeping.rs` (639 L) |
| erincatto/box2d | `a292912c10a6865cba2c250b765696c92e9996b8` | `src/contact_solver.c` (20 KB solve/restitution/warm start), `src/solver.c` (sleep, CCD trigger), `src/island.c`, `src/solver_set.c` (island sleep move) |
| NVIDIAGameWorks/Blast | `636e7eb43b8d41e44c40eb1b4799bfd027f17ba7` | `source/sdk/extensions/stress/NvBlastExtStressSolver.cpp` (1806 L: stress solve, damage, fracture commands) |

Read only, shape taken, no bytes vendored (AGENTS rule 4). LADDER lines are left to the
parent: this session was told not to edit `docs/research/LADDER.md`.

## engine2040 today

| Capability | Where | Honest state |
|---|---|---|
| Rigid body step (2D) | `crates/physics/src/rigid.rs:1` | Real, semi-implicit Euler plus a ground clamp at `y == hy`. This is the legacy `Space` the frozen golden runs (`docs/HASH-LAW.md:20-29`) |
| Contact graph topology | `crates/physics/src/islands.rs:279` | Real union-find, canonical order, snapshot bytes. No solver: it only groups units |
| Island sleep and wake | `crates/physics/src/islands.rs:561` (`update_sleep`), called at `crates/physics/src/lod.rs:412` | Real and already donor-shaped: island peak velocity wakes the whole island, `sleep_steps` hysteresis, timers per body |
| Character controller | `crates/physics/src/controller2.rs:28-34` | Real: 500 mm steps, 400 mm snap, 45 deg slope cap, downhill slip, coyote/buffer, ledges. Ahead of Rapier's docs, level with Jolt's |
| Swept time-of-impact | `crates/physics/src/query.rs:565` (`swept_cast`) | Real integer Q16 sweep, nearest hit first, ties by id |
| Cloth | `crates/physics/src/cloth/mod.rs:25-33` | Real: swept colliders, self-collision set, per-particle max distance |
| Rope and winch graph | `crates/physics/src/articulated.rs:15-19` | Real but position-only ("The solve is position based"), 48 sweeps (`:34`), no velocity pass. `:3` "Consumer: none yet" |
| Severed limbs (hashed) | `crates/physics/src/limb.rs:323-376` | Gravity, move, floor at `y = radius`, quarter bounce, friction, sleep. The step takes no world (`pub fn step(&mut self)`, `:325`): no wall, no pillar, no sweep |
| Bonded destruction | `crates/destruction/src/bonded.rs:57-63`, `:364-407`, `:779` | Real: per-bond strain that accumulates (`:392`), one limit, load-capacity collapse, support flood. A hit at or below `mass * threshold` returns before any strain (`:378`) |
| Voronoi fracture | `crates/destruction/src/fracture.rs:1` | Real, cached, integer sites. Not a gap |
| Ragdoll (float) | `crates/physfx/src/fx.rs:548-553` | Rapier, client-local, never hashed. Soft CCD (`set_soft_ccd_prediction`) and `max_ccd_substeps` per tier |
| Fighter push | `crates/physics/src/controller2.rs:884` (`separate_pair`) | Real but mass-free: overlap is split in half (`overlap / 2`), and `Walker` has no mass field (`:253-273`) |
| CCD policy | `crates/physics/src/lod.rs:172` (`ccd_for`) | Speed and `AgentKind` only, no body size |

## The steals, ranked

### 1. Speculative contact in the hashed limb step

Donor: Box2D v3, `src/contact_solver.c:335-341` —
https://github.com/erincatto/box2d/blob/a292912c10a6865cba2c250b765696c92e9996b8/src/contact_solver.c#L335-L341

```c
if ( s > 0.0f )
{
    // speculative bias is positive
    velocityBias = s * inv_h;
```

Same mechanic, two more donors: Jolt `Jolt/Physics/PhysicsSettings.h:47-50`
("Radius around objects inside which speculative contact points will be detected") and
Avian `src/dynamics/ccd/mod.rs:143-147`
("only predicts contacts up to a small, fixed `contact_tolerance`").

What it buys: a contact at positive separation is solved as a constraint, so a body may
never move through a thin slab between two ticks; no time-of-impact pass, no substeps.
The rule is one line of arithmetic on integer millimetres.

Local gap: `crates/physics/src/limb.rs:343` clamps only against `floor_um`. A severed limb
spawned inside a pillar or thrown at the round wall passes through it and lands on the
floor. The crate already owns the tool the fix needs: `query::swept_cast`
(`crates/physics/src/query.rs:565`), used by cloth at `cloth/mod.rs:29` and not by limbs.

Layer: **authority** (integers, hashed, `LimbPool::state_hash`). Not physfx: `fx.rs:551`
already sets Rapier soft-CCD on the tiers that pay for it.

Size: small. One new `step(&mut self, world)` signature plus a per-limb sweep against the
arena's pillar footprints and the round wall.

Risk: low. Needs the arena to hand the pool its static geometry: the pool is stepped with no
world today at `crates/arena/src/lib.rs:1701` (and `:4999`), so the change is one argument
threaded from the arena's pillar and wall tables.

Golden: cannot move `GOLDEN_600` (the legacy fixture never runs a limb,
`docs/HASH-LAW.md:20-29`). It can move the arena digest `9f162a2fe205f0ff`, so it needs an
arena receipt, like RZ3w's (`docs/HASH-LAW.md:154-158`).

### 2. Size-aware CCD trigger, not a speed threshold

Donor: Box2D v3, `src/solver.c:648-651` —
https://github.com/erincatto/box2d/blob/a292912c10a6865cba2c250b765696c92e9996b8/src/solver.c#L648-L651

```c
float maxMotion = b2MaxFloat( maxDeltaPosition, maxVelocity * timeStep );
if ( body->type == b2_dynamicBody && enableContinuous && maxMotion > safetyFactor * sim->minExtent )
```

Jolt states the same rule as a knob, `PhysicsSettings.h:55-56`: "Fraction of its inner
radius a body must move per step to enable casting for the LinearCast motion quality",
default `0.75`.

What it buys: CCD is decided by *travel over the shortest extent*, so a 6 mm shard at
0.6 m/s (10 mm per tick) casts while a 2 m crate at the same speed does not. A pure speed
gate misses the small-and-slow tunnelling case and pays for the large-and-fast one.

Local gap: `crates/physics/src/lod.rs:172` reads `kind`, `motion` and `speed_mps` only;
`AgentKind` (`lod.rs:67-74`) has no debris variant at all, so debris falls to a kind it
does not have. Config is `ccd_speed_mps` alone (`lod.rs:84-85`).

Layer: **policy for authority, consumed by physfx** (`rapier::RagdollParams::ccd_flags`,
`lod.rs:169-170`). The arithmetic is integer/float compares, no platform maths.

Size: small. One function, one config lane (`ccd_extent_q`), call sites already exist.

Risk: low, but it changes which bodies pay for CCD: a passing tier may lose frames. Needs
the tier receipts (`Caps`/`Tier::tick_bar_us`) before shipping.

Golden: no. `lod` is not in the legacy fixture. The 20k crowd digest `0x9627a1119822c2e0`
and the physfx soak are the receipts.

### 3. Mass-weighted push between walkers

Donor: Rapier, `src/control/character_controller.rs:970-974` —
https://github.com/dimforge/rapier/blob/475caa3ff4646fd9a796b67d14116256f4962bc7/src/control/character_controller.rs#L970-L977

```rust
let mass_ratio = body_mass * character_mass / (body_mass + character_mass);
body.apply_impulse_at_point(
    manifold.data.normal * delta_vel_per_contact.max(0.0) * mass_ratio,
```

What it buys: the two-body impulse factor. A 140 kg legionary in lorica moves a 60 kg
fighter, and barely moves a crate; the `.max(0.0)` keeps the contact push-only, never a
pull. For the Rome plan (`docs/FOCUS.md:18`, shield line) this is the difference between a
line that holds and eight walkers halving every overlap.

Local gap: `crates/physics/src/controller2.rs:884` splits overlap in half for both bodies
(`let step = (overlap / 2).max(1);`), and `Walker` (`:253-273`) has no mass. The arena's
shove is a distance table, `crates/arena/src/collide.rs:80` (`PUSH_SHOVE_TABLE`), not a
mass ratio.

Layer: **authority**: integer grams, `inv_mass_q16` per walker, split as
`share_a = overlap * w_a / (w_a + w_b)` with integer rounding that sums back to `overlap`
(so the pair still reaches a fixed point and `resolve`'s fixed-point stop stays valid,
`collide.rs:104-113`).

Size: medium. New field on `Walker` (a snapshot/hash change), one split function, all
callers of `Walker::spawn` updated, plus the shove table's intent re-expressed.

Risk: medium. Every `Walker` snapshot and digest is v2, so this is a rules-version change:
new tag bytes or a v3 rules lane, never a silent re-pin of the current arena digest.

Golden: no (`GOLDEN_600` never collides). It moves the arena v2 digest; do it under a new
rules version, as EG20 and TC3 did (`docs/HASH-LAW.md:102-108`, `:193-200`).

### 4. Bond health with elastic and fatal bands (damage below the break threshold)

Donor: Blast, `source/sdk/extensions/stress/NvBlastExtStressSolver.cpp:1595-1622` —
https://github.com/NVIDIAGameWorks/Blast/blob/636e7eb43b8d41e44c40eb1b4799bfd027f17ba7/source/sdk/extensions/stress/NvBlastExtStressSolver.cpp#L1595-L1622

```c
const float excessStress = stressTension - m_settings.tensionElasticLimit;
...
// bond health/area is reduced by excess pressure to approximate micro bonds in the material breaking
const float bondDamage = bondHealth * stressMultiplier;
```

What it buys: three separate stress modes (compression, tension, shear), each with an
elastic limit (no damage) and a fatal limit (full damage), and a per-bond health that
falls with the *excess over elastic*, not with a binary threshold. A wall hit five times
with a light ram cracks and then fails, instead of being invulnerable until one big hit.

Local gap: `crates/destruction/src/bonded.rs:378` returns before any strain when
`impulse <= mass * threshold`, so a whole series of light blows is exactly zero damage.
Above it, strain accumulates (`:392`) against one limit (`:393`): no shear axis, no
elastic/fatal band, no per-bond health. `fail_overloaded` (`:779`) is a load capacity, a
different mechanism.

Layer: **authority** (integer permille damage, `u16` health per bond, canonical bytes in
`bonded/snapshot.rs`).

Size: medium. `Bond` (`:57-63`) gains `health_pm`; `apply_impact` gains the excess-over-
elastic rule; snapshot and hash gain two lanes; the bake path (`bond_grade.rs`) must seed
health.

Risk: medium. It changes the break curve of every bonded prop, so every existing crate
test with a fixed impulse count (`bonded.rs:2191`, `:2262`) must be re-derived, and the
arena digest moves.

Golden: no (`GOLDEN_600` has no destruction). Arena digest moves; needs a rules lane and a
before/after table of one crate's hit count, like the grade calibration test.

### 5. Velocity iterations beside the position solve in the winch graph

Donor: Jolt, `Jolt/Physics/PhysicsSettings.h:82-87` —
https://github.com/jrouwe/JoltPhysics/blob/6f8bc72fd5fc6a58a52a7fbfe49ca7b5170b4990/Jolt/Physics/PhysicsSettings.h#L82-L87

```c
/// Number of solver velocity iterations to run
/// Note that this needs to be >= 2 in order for friction to work (friction is applied using the non-penetration impulse from the previous iteration)
uint		mNumVelocitySteps = 10;
/// Number of solver position iterations to run
uint		mNumPositionSteps = 2;
```

What it buys: the standard split. Velocity passes kill closing speed along the constraint
(no energy injected by a pure projection), then a few position passes remove the residual
error. Jolt ships a 10:2 ratio and two position steps; engine2040 ships zero velocity
passes and 48 position sweeps.

Local gap: `crates/physics/src/articulated.rs:15-19` says it plainly ("The solve is
position based"), and `ITERATIONS = 48` (`:34`) is all position. A load dropping onto a
taut winch rope is moved back but keeps its closing velocity, so the next tick projects
again: the classic rope spring.

Layer: **authority** (integer Q8/Q30, `Graph::hash`). No float.

Size: small inside the module (one relative-velocity lane per link plus a 2-4 pass loop),
but the ordering must stay canonical (links by id pair, `:8-11`).

Risk: low technically, high on value: `articulated.rs:3` says "Consumer: none yet". There
is no portcullis or chariot on the board, so this is a fix waiting for a caller. Wire a
consumer first (`wiring-a-feature`).

Golden: no. Nothing hashes this graph today.

## What not to steal

- **Character controller (Jolt `CharacterVirtual.cpp`, Rapier `character_controller.rs`).**
  engine2040 already has step, snap, slope, slip, coyote, buffer, ledges, moving platforms
  (`controller2.rs:28-34`, tests at `:1195`, `:1485`). Jolt's `StickToFloor` is only called
  when the character went supported-to-air and is not rising
  (`CharacterVirtual.cpp:1818-1825`); engine2040's snap lives in the non-rising branch
  (`controller2.rs:774`, `:815`). Nothing to add.
- **Island sleep and wake.** Already donor-shaped: `islands::update_sleep` reads an island
  peak velocity that "includes sleepers: a kick on zeroed velocity is the wake signal"
  (`crates/physics/src/islands.rs:561`) and is called at `lod.rs:412`. Box2D's
  "any single body in an island can keep it awake" (`src/solver.c:682-689`) is the same
  rule. Its extra piece, splitting an island before sleeping after a joint removal
  (`src/solver_set.c:186-190`), is unnecessary here because islands are rebuilt every tick
  from pairs (`lod.rs:384-385`).
- **Voronoï fracture and CSG.** Real and cached (`fracture.rs:1`, `csg.rs:1`). Blast's
  chunk graph would replace working code.
- **Swept/non-linear CCD as a mode selector** (Avian `ccd/mod.rs:32-35`, `SweptCcd`).
  physfx is a paint layer: soft CCD (`fx.rs:551`) plus `max_ccd_substeps` is enough, and
  Avian itself lists the cost: "a large speculative margin can also lead to excessive
  collision tests and false positives" (`ccd/mod.rs:149-151`). The authority needs the
  cheap speculative rule (steal 1), not a second CCD mode.
- **Jolt's "the simulation runs deterministically" line** (`README.md:30`) is a pointer to
  a website section with limits, not a recipe. Our tier is fixed by `docs/HASH-LAW.md:70-91`
  (Rapier `enhanced-determinism`, no SIMD/parallel, integer authority). Do not port a
  claim without its limits.

## Could not verify

- Rapier's `contact_skin`/`soft_ccd_prediction` source was not read; physfx already calls
  `set_soft_ccd_prediction` (`fx.rs:552`), and that is the only claim made about it.
- Jolt's contact solver internals (`ContactConstraintManager.cpp`) were not read. The
  speculative-contact claim rests on `PhysicsSettings.h`'s own doc comment plus Box2D's
  implementation, not on Jolt's code.
- The Blast stress solver's graph solve (`m_graphProcessor->solve`, `:1575`) was not read;
  only the damage rule around it. The claim is about the damage curve, not the FEM.
- Jolt's website "Deterministic Simulation" page was not fetched (off-repo).
- Avian's 2D/3D split and its schedule were not read; `src/dynamics/ccd/mod.rs` and
  `islands/sleeping.rs` were.
- No test, bench, capture or cargo line was run. "Reading code is not implemented": all five
  steals are cards, not rows.

## Cards

Shaped like the LADDER head (`docs/research/LADDER.md:17-18`). Not added there by this
session; the parent folds them.

- CARD | P4 | a hashed limb stops at walls and pillars by speculative contact, not only at the floor | erincatto/box2d@a292912c `src/contact_solver.c:335-341`, jrouwe/JoltPhysics@6f8bc72f `Jolt/Physics/PhysicsSettings.h:47-50` | not run: arena v2 bout with 20 seeded cuts, limb-vs-pillar penetration count today -> after, wall hugs, limb digest x2 equal, arena digest receipt | ROW proposal: PH1, `limb.rs` step takes the step world
- CARD | P3 | CCD triggers on travel over shortest extent, so a shard casts and a crate does not | erincatto/box2d@a292912c `src/solver.c:648-651`, jrouwe/JoltPhysics@6f8bc72f `Jolt/Physics/PhysicsSettings.h:55-56` | not run: 8 ragdolls plus 200 debris soak, current speed gate -> extent gate; tunnel count, CCD body count, physfx tick us per tier | ROW proposal: PH2, `lod::ccd_for` plus `CcdConfig`
- CARD | P4 | a shield line holds because a push splits by mass, not in half | dimforge/rapier@475caa3f `src/control/character_controller.rs:970-977` | not run: 8 walkers in a 3-wide corridor, one 140 kg pushing 60 kg, equal split -> mass split; closing distance after 60 ticks, fixed-point pass count, arena digest under a new rules lane | ROW proposal: PH3, `Walker.mass_g`, v3 rules
- CARD | P4 | light blows crack a wall over time: per-bond health with elastic and fatal bands | NVIDIAGameWorks/Blast@636e7eb4 `source/sdk/extensions/stress/NvBlastExtStressSolver.cpp:1595-1622` | not run: one crate, five sub-threshold rams then one heavy, intact -> broken at hit n; break count, piece count, digest x2 | ROW proposal: PH4, `Bond.health_pm`
- CARD | P3 | a winch rope stops the load by killing closing speed, then removes the error | jrouwe/JoltPhysics@6f8bc72f `Jolt/Physics/PhysicsSettings.h:82-87` | not run: 120 kg load onto a 4 m taut rope at 3 m/s, 48 position sweeps -> 10 velocity plus 2 position; peak stretch mm, rebound height, 600-tick digest x2, solve us | ROW proposal after a consumer exists (portcullis or chariot), `articulated.rs`
