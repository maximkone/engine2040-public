---
name: physics-determinism
description: "Use when changing a crates/physics solver (rigid, kcc, ragdoll, rapier, islands, balance) in a way that could move the golden hash, the rapier state or rollback bytes: gives the rules and the proof commands."
---

# Physics determinism in engine-physics

Physics steps at one fixed dt and hashes only integer or quantized state.
The one danger: a change that looks right on one machine but moves a hash, reorders bodies, or leaves the rapier build without its determinism settings.

## Rules

1. Step with the fixed dt only. Do: `crates/physics/src/rapier.rs::FIXED_DT` (1/60). `World::step` rejects a dt that is not finite or not above zero with `Error::InvalidDt`.
2. Iterate bodies in spawn order, never through a hash map. Do: the `order` field of `crates/physics/src/rapier.rs::World`, which `World::state_hash` reads. `tools/sim-clippy.toml` bans `HashMap` and `HashSet` in sim crates.
3. Know what the rapier hash covers. It folds dynamic bodies only: position, rotation, linear and angular velocity, each scaled by `HASH_SCALE` and rounded. Do: `crates/physics/src/rapier.rs::quantize`, then the "Hash scope" section of `docs/HASH-LAW.md`.
4. Fold only integer state into an authoritative hash. The rapier ragdoll is client-local and never hashed. Do: `crates/physics/src/balance.rs::BalanceState::state_hash`. Never fold `crates/physics/src/ragdoll.rs::Ragdoll::state_hash`.
5. Keep rapier opt-in and pinned. Do: the `rapier` feature in `crates/physics/Cargo.toml`, the feature gates in `crates/physics/src/lib.rs`, and the `rapier3d` entry in the root `Cargo.toml`, which has `enhanced-determinism` and no simd or parallel feature.
6. Roll back by rebuilding the whole world, not only the poses. Rapier keeps warm-start state that a snapshot cannot hold. Do: `crates/physics/src/rapier.rs::World::island_snapshot` and `World::restore_island_snapshot`, which rebuild through the private `World::reset_to`.
7. Use only `+ - * /`, `sqrt`, comparisons and integer math. Do: `crates/physics/src/kcc.rs::V3::len` for the allowed `sqrt`. `tools/sim-clippy.toml` lists the banned `sin`, `cos`, `exp` and `powf` family.

## Check
- `cargo test -p engine-proof --lib golden_hash_untouched`: passes, and the golden stays `d57b5c8ae3096614`.
- `cargo test -p engine-physics --lib`: 0 failed.
- `cargo test -p engine-physics --features rapier --lib balance_hash`: 0 failed.
- `cargo test -p engine-physics --features rapier --lib snapshot`: 0 failed. Rollback stays byte-equal.
- `node tools/workspace-check.mjs`: exit 0, no sim determinism violation reported.
- `node tools/rust-quality.mjs`: no mismatch reported for `crates/physics/clippy.toml` against `tools/sim-clippy.toml`.

## Don't
- Don't change `GOLDEN_600` or its fixture without a receipt in the same commit (`docs/HASH-LAW.md`, "Golden changes and failures").
- Don't read wall-clock time, a render value or a hash-map order into any tick.
- Don't add the `simd` or `parallel` features to rapier3d, or fold the ragdoll pose into any hash.
