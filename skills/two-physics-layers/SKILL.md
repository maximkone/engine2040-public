---
name: two-physics-layers
description: "Use when you add or change a break, a severed limb, a ragdoll, debris or a material in engine2040: which part is the hashed authority layer, which is the float physfx layer, and how HASH-LAW keeps them apart."
---

# Two physics layers

Spec: `docs/plan/web-and-realism.md` section 2. Rows: RZ1-RZ10 in `docs/SPRINT-22-web-realism.md`. Hash rules: `docs/HASH-LAW.md`.

## The split
- **Authority** (integers, hashed, same on every PC and in wasm): who hit whom, whether it cut, which limb is gone, who is floored, which bond broke. Crates `arena`, `combat`, `destruction`, `physics` (integer parts), `proof`. It decides every game outcome and never reads a float from the other layer.
- **Physical** (floats, Rapier, NOT hashed): `crates/physfx`. It only has to look and feel heavy: ragdolls after a knockout, flying limbs, debris, props. It may differ by centimeters between machines. It takes plain events and gives back transforms. It cannot call into the sim.
- Rule of thumb: if a rule or a score depends on the outcome, it goes through an authority row first (a break event, a damage volume); physfx only paints it. A boulder that crushes is authority, the rubble it leaves is physfx.

## HASH-LAW in one minute
- Golden `d57b5c8ae3096614` (seed 2040, 600 ticks) moves only with an explicit receipt (old and new hash, reason, both run outputs, in the commit).
- No `sin`, `cos`, `exp`, `powf`, `HashMap` order, wall clock or pointers in hashed state. Integers, `Fixed`, `+ - * /`, `sqrt` only.
- New authority state that must join the hash joins the arena fold with its own tag byte (see the `0xB1` and `0xB2` notes in `docs/HASH-LAW.md`), under a new fixture version. v1 stays frozen.
- Proof the ragdoll stays out of the hash: `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib balance_hash`.

## The one dependency line
- `engine-physfx` depends on physics and anim only. `arena`, `combat` and `proof` must never name it: test `sim_crates_never_name_physfx` in `crates/physfx/src/tests.rs` greps them, and `tools/workspace-check.mjs` holds the crate edges (`engine-game` may depend on `engine-physfx`, optional, behind the `app` feature).
- Run it: `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib sim_crates_never_name_physfx`.
- physfx reads no clock of its own (browsers have no `Instant`): give it one with `PhysFx::with_clock`.

## Where to add a thing
- **A break (wall, crate, prop).** Authority: bonded collections and fracture in `crates/destruction/src` (`bonded.rs` emits `BreakEvent`, `fracture.rs` and `shards3d.rs` make chunks). Picture: `crates/game/src/break_look.rs` turns each event into `PhysFx::spawn_debris(shards, impulse)`. Add the authority rule and a test first, then the look.
- **A limb or a cut.** Authority: cut tables in `crates/combat/src/cut.rs`, limb state in `limbloss.rs`, the 16-byte event in `crates/arena/src/limbs.rs`. Picture: the game calls `PhysFx::spawn_limb(part, shape, transform, velocity)` (or `spawn_limb_of(Material, ...)`) when the event arrives.
- **A ragdoll.** Authority says floored (balance and stagger state, integer). The game seeds `PhysFx::spawn_ragdoll(slot, rig, pose, velocity, impulse)` once from the event and reads `PhysFx::transforms(alpha)` each frame.
- **A material.** One table: `crates/physics/src/material.rs` (`Material`, integer thousandths). Three readers use it: physfx (density, bounce, damping, grip), the authority walker (grip, step height) and the audio cue picker (`Material::cue`). A new variant needs a row in every accessor and a sound class. A test fails when the Rapier constants drift from the `Flesh` and `Stone` rows, so edit the table, not the constants.
- **A cap or a tier.** `crates/physfx/src/caps.rs` (`Tier::Potato`, `Web`, `Comfort`, `High`): bodies, ragdolls, contacts, solver budget in one table. The oldest body is recycled first; never add a spawn path that skips the caps. Print `Stats::receipt_line` in the proof.

## Pitfalls
- A refused spawn (NaN, infinity) must leave the world untouched: validate in `check.rs` before any change, return a typed `Error`.
- Do not feed physfx results back into the sim, even "just the final rest position". That is an authority row.
- Rapier cross-architecture equality is a claim until the golden matrix prints it (skill `golden-matrix`); cosmetic use does not depend on it.
- physfx is behind the `rapier` feature so the headless graph stays std-only: test with `--features rapier`.
