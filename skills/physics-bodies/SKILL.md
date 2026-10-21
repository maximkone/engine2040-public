---
name: physics-bodies
description: "Use when you change physics, physfx, destruction, ragdoll, limb cuts, debris, a break, a material, Rapier or cloth: hashed authority layer versus float physfx layer, fixed dt, spawn order, rollback rebuild."
---

# Physics bodies

Two layers. One law. Hash rules live in `docs/HASH-LAW.md` (read it only when the hash moves).
Spec: `docs/plan/web-and-realism.md` section 2 and `docs/plan/body-and-break.md`. Rows: RZ1-RZ10 in `docs/SPRINT-22-web-realism.md`.

## 1. The split

- **Authority** (integers, hashed, same on every PC and in wasm): who hit whom, whether it cut, which limb is gone, who is floored, which bond broke. Crates `arena`, `combat`, `destruction`, `physics` (integer parts), `proof`. It decides every game outcome. It never reads a float from the other layer.
- **Physical** (floats, Rapier, NOT hashed): `crates/physfx`. It only has to look and feel heavy: ragdolls after a knockout, flying limbs, debris, props. It may differ by centimeters between machines. It takes plain events and gives back transforms. It cannot call into the sim.
- Rule of thumb: if a rule or a score depends on the outcome, it goes through an authority row first (a break event, a damage volume). physfx only paints it. A boulder that crushes is authority. The rubble it leaves is physfx.
- The look side runs live: `crates/game/src/ragdoll_look.rs` seeds `PhysFx::spawn_ragdoll` and reads `PhysFx::transforms`. `crates/game/src/break_look.rs` turns each `BreakEvent` into `spawn_debris_of`. Both sit behind the `app` feature of `engine-game`.

## 2. HASH-LAW in one minute

- Golden `d57b5c8ae3096614` (seed 2040, 600 ticks) moves only with an explicit receipt: old and new hash, reason, both run outputs, in the commit. Never re-pin it to go green.
- No `sin`, `cos`, `exp`, `powf`, `HashMap` order, wall clock or pointers in hashed state. Integers, `Fixed`, `+ - * /`, `sqrt` only.
- New authority state that must join the hash joins the arena fold with its own tag byte (see the `0xB1` and `0xB2` notes in `docs/HASH-LAW.md`), under a new fixture version. v1 stays frozen.
- A hash that moves and you do not know why: skill `determinism-debug`.

## 3. Determinism rules for crates/physics

1. **Fixed dt only.** `crates/physics/src/rapier.rs::FIXED_DT` re-exports `engine_core::FIXED_DT` (1/60). `World::step` refuses a dt that is not finite or not above zero with `Error::InvalidDt`.
2. **Spawn order, never a hash map.** Iterate the `order` field of `crates/physics/src/rapier.rs::World`; `World::state_hash` reads it. `tools/sim-clippy.toml` bans `HashMap` and `HashSet` in sim crates.
3. **Know what the rapier hash covers.** Dynamic bodies only: position, rotation, linear and angular velocity, each scaled by `HASH_SCALE` (1e6) and rounded (`rapier.rs::quantize`).
4. **Hash integer state only.** The rapier ragdoll is client-local and never hashed. Fold `crates/physics/src/balance.rs::BalanceState::state_hash`. Never fold `crates/physics/src/ragdoll.rs::Ragdoll::state_hash`.
5. **Rapier is opt-in and pinned.** Feature `rapier` in `crates/physics/Cargo.toml`. The root `Cargo.toml` entry `rapier3d` has `enhanced-determinism`, `default-features = false`, and no `simd` or `parallel`. Do not add them.
6. **Roll back by rebuilding the whole world**, not only the poses. Rapier keeps warm-start state a snapshot cannot hold. Use `World::island_snapshot` and `World::restore_island_snapshot`; they rebuild through the private `World::reset_to`. Pure island bytes: `crates/physics/src/islands.rs::island_snapshot`.
7. **Math.** `+ - * /`, `sqrt`, comparisons, integer math. `crates/physics/src/controller.rs::planar_len` is the allowed `sqrt`.
8. **Never read** wall-clock time, a render value or a hash-map order into any tick.

## 3b. Prove the laws (every new or changed sim step)

Example tests prove the cases you thought of. Laws prove the physics. `engine_core::laws` (dev-dependency `engine-core` with feature `test-util`) steps your state with your step closure and names the law, the tick and the state when it breaks:

```rust
use engine_core::laws;
laws::rest_stays_rest(parked, 600, |s| step(s, &idle)).unwrap();
laws::mirror(east, west, 300, |s, _| step(s, &input_for(s)), |a, b| a.pos[0] == -b.pos[0]).unwrap();
laws::energy_never_rises(thrown, 600, |s| step(s, &idle), energy, 1.0).unwrap();
laws::steady_rate(walker, 60, 600, |s| step(s, &walk(59)), |s| f64::from(s.pos[0]), 59.0 / 60.0, 1.0).unwrap();
```

Pick the laws that apply: rest stays rest (anything with ground or contacts), replay (all), mirror (all), energy never rises (bounce, springs, ropes, damping, drag), conserved (mass through breaks, momentum in pushes), steady rate (anything moved by a velocity). Slack comes from the rounding (1 mm, one unit squared), never from the failure. Worked example: the walker tests in `crates/physics/src/controller2.rs`.

The bugs these catch, all found here on 2026-10-21:
- `pos += vel / 60` in integer mm stalls every speed under 60 mm/s. Carry the remainder (`controller2.rs::travel`).
- A rule written for one sign only (`grade > max` with no `grade < -max`): a slope solid one way, hollow the other.
- `(x1 - x0).max(1)` on a span that may be negative: use the signed run, guard zero only.
- `>>` on a negative value floors, `/` truncates toward zero: only `/` is mirror-symmetric.
- Explicit Euler on a spring (both updates from the old state) gains energy every step. Update velocity first, then position.
- Damping or drag applied as `v - v * k * dt` reverses the velocity once `k * dt > 1`. Use `v / (1 + k * dt)`.

## 4. Where to add a thing

| Thing | Authority side (test first) | Picture side |
|---|---|---|
| Break (wall, crate, prop) | `crates/destruction/src/bonded.rs` emits `BreakEvent`; `fracture.rs` and `shards3d.rs` make chunks | `crates/game/src/break_look.rs` calls `PhysFx::spawn_debris(shards, impulse)` or `spawn_debris_of(material, ..)` |
| Limb loss | severing tables `crates/combat/src/cut.rs`, limb state `limbloss.rs`, 16-byte event `crates/arena/src/limbs.rs` | game calls `PhysFx::spawn_limb(part, shape, transform, velocity)` or `spawn_limb_of(Material, ..)` on the event |
| Ragdoll | balance and stagger state (integer) says floored | game seeds `PhysFx::spawn_ragdoll(slot, rig, pose, velocity, impulse)` once, then reads `PhysFx::transforms(alpha)` each frame (`ragdoll_look.rs`) |
| Material | one table: `crates/physics/src/material.rs::Material`, integer thousandths; `Material::props`, `sound`, `cue` | physfx (density, bounce, damping, grip), the authority walker (grip, step height) and the audio cue picker read it |
| Cap or tier | `crates/physfx/src/caps.rs` (`Tier::Potato`, `Web`, `Comfort`, `High`) | bodies, ragdolls, contacts, solver budget in one table |

- A new `Material` variant needs a row in every accessor and a sound class. A test fails when the Rapier constants drift from the `Flesh` and `Stone` rows: edit the table, not the constants.
- Caps: the oldest body is recycled first. Never add a spawn path that skips the caps. Print `Stats::receipt_line` (`crates/physfx/src/types.rs`) in the proof.
- Ragdoll handoff rules (powered bones, no LERP into a ragdoll pose): skill `anim-combat`.

## 5. The one dependency line

- `engine-physfx` depends on physics and anim only. `arena`, `combat` and `proof` must never name it. Test `sim_crates_never_name_physfx` (`crates/physfx/src/tests.rs`) greps them. `tools/workspace-check.mjs` holds the crate edges. `engine-game` may depend on it, optional, behind `app`.
- physfx reads no clock of its own (browsers have no `Instant`). Give it one with `PhysFx::with_clock`.
- physfx is behind its `rapier` feature so the headless graph stays std-only. Test with `--features rapier`.

## 6. Soft body and cloth

Soft-body and cloth checks are design work, not code: see `docs/plan/body-and-break.md`. The cloth that exists is `crates/physics/src/cloth/mod.rs` (presentation only: floats, never hashed).

## 7. Proof commands

Run through GitHub (skill `github-check`), one cargo line each:

- `node tools/gh-check.mjs cargo test -p engine-proof --lib golden_hash_untouched` passes and the golden stays `d57b5c8ae3096614`.
- `node tools/gh-check.mjs cargo test -p engine-physics --lib` 0 failed.
- `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib balance_hash` 0 failed. This proves the ragdoll stays out of the hash.
- `node tools/gh-check.mjs cargo test -p engine-physics --features rapier --lib snapshot` 0 failed. Rollback stays byte-equal.
- `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib sim_crates_never_name_physfx`.
- `node tools/workspace-check.mjs` exit 0, no sim determinism violation.
- `node tools/rust-quality.mjs` no mismatch of `crates/physics/clippy.toml` against `tools/sim-clippy.toml`.
- Sim code changed: `cargo run -p engine2040 --release` twice, same hash both times.

## 8. Pitfalls

- A refused spawn (NaN, infinity) must leave the world untouched. Validate in `crates/physfx/src/check.rs` before any change. Return a typed `Error`.
- Do not feed physfx results back into the sim, not even "just the final rest position". That is an authority row.
- Rapier cross-architecture equality is unproven until the golden matrix prints it (skill `determinism-debug`). Cosmetic use does not depend on it.
- Do not change `GOLDEN_600` or its fixture without a receipt in the same commit.

Sources: Game Engine Gems 3, ch. 12 (strain-based elements).
