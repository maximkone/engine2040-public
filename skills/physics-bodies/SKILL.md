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

## 4. Where to add a thing

| Thing | Authority side (test first) | Picture side |
|---|---|---|
| Break (wall, crate, prop) | `crates/destruction/src/bonded.rs` emits `BreakEvent`; `fracture.rs` and `shards3d.rs` make chunks | `crates/game/src/break_look.rs` calls `PhysFx::spawn_debris(shards, impulse)` or `spawn_debris_of(material, ..)` |
| Limb or cut | cut tables `crates/combat/src/cut.rs`, limb state `limbloss.rs`, 16-byte event `crates/arena/src/limbs.rs` | game calls `PhysFx::spawn_limb(part, shape, transform, velocity)` or `spawn_limb_of(Material, ..)` on the event |
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

`crates/physics/src/cloth/mod.rs` exists (RZ7). It is CPU position-Verlet cloth for capes and banners: `Cloth`, `ClothParams`, `ClothTier`, `Cloth::pin_top_row`, `Cloth::step`, `Cloth::positions`. Jakobsen tension-only links, tethers, one-way capsule colliders, sleeps when still, tearing off unless `tear_strain` is set. It is presentation only: floats, never hashed, never read by the sim. `cloth/mod.rs` says the fighter look and arena banners consume it. A grep of `crates/` on 2026-10-17 found no caller outside `cloth.rs` itself: verify with Grep before you call it wired.

An earlier element-constraint module (`ClothWorld`, `step_cloth`, `MAX_CONSTRAINTS`) was cut on 2026-10-16 and is gone: `git log --follow -- crates/physics/src/cloth.rs`. `docs/plan/body-and-break.md` still sketches it.

The checks below are a design target for the next soft-body or strain-based module. They do not describe today's `cloth/mod.rs`. PASS needs element-constraint tests plus x2-equal bytes with `cargo test -p engine-physics --lib` green. A lively capture alone is not proof.

1. **One constraint per element.** Each triangle or tet carries one element constraint solved per pass. Flag: per-edge soup where element shear is invisible to the solver.
2. **Stretch split from shear.** Diagonal stretch and off-diagonal shear get separate stiffness fractions. Rest stretch defaults to one. Flag: one k for both.
3. **Rest inverse baked once.** Build the per-element rest inverse at init and store it. Refuse degenerate rest shapes. Flag: inverse recomputed per tick, or a zero-area rest triangle that smuggles NaN.
4. **Bending keeps its rest angle.** Store the rest dihedral angle at init. Scale stiffness by iteration count, cap at one, solve after the element pass. Flag: raw k per iteration, so more iterations means stiffer.
5. **Volume and area held.** Tets carry a volume constraint, tris an area one, with analytic gradients that sum to zero, inside one fixed constraint cap. Flag: squeezed elements invert or vanish.
6. **Zeroed scratch, fixed order.** Clear the correction accumulator each solve. Run constraints in index order for a fixed iteration count. Flag: corrections leaking across ticks, or hash-order merges.
7. **Damp along the correction, pin the anchors.** Pinned particles hold their position and are never integrated. Flag: energy gained per tick, or anchors integrated then snapped back.
8. **Collide last, draw from a map.** Colliders run after the internal passes (push out, kill velocity). Surface verts come from a barycentric map built once. Flag: collide-then-solve re-penetration, or a per-frame nearest-element search.

Do: bake rest state at init, collide after the solve, prove x2 equal plus frame ms. Don't: pile per-edge distances, share one stiffness, ship "looks soft" with no numbers.

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
