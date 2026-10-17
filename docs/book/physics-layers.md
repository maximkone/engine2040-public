# Physics layers: authority and physical

Heavy, real-looking physics, the hash law and a browser tab can all hold at once because the engine keeps two physics layers apart. Source: `docs/plan/web-and-realism.md` section 2, `docs/HASH-LAW.md` and the docs at the top of `crates/physfx/src/lib.rs`.

## The two layers

| | Authority layer | Physical layer |
|---|---|---|
| Crates | `engine-arena`, `engine-combat`, `engine-destruction`, `engine-physics` (balance) | `engine-physfx` on top of Rapier |
| Numbers | integers and fixed point | floats, meters, seconds, Y up |
| Hashed | yes, in the tick hash | never |
| Decides | who hit whom, whether it cut, which limb is gone, who is floored, which bond broke | how a ragdoll tumbles, where debris and severed limbs land, how it looks and feels heavy |
| Same on every PC | yes | no: it may differ by a few centimeters |

The rules that keep it honest:

- The sim never reads a float from the physical layer. The physical layer cannot call back into the sim.
- `engine-arena`, `engine-combat` and `engine-proof` must never depend on `engine-physfx`. The test `sim_crates_never_name_physfx` and the dependency edges in `tools/workspace-check.mjs` hold that line.
- When a physical outcome has to matter for the rules (a boulder that crushes, a collapse that traps), it goes through an authority row first (a break event, a damage volume). `physfx` only paints it. This keeps rollback and web multiplayer honest.
- Every spawn is checked. A NaN, an infinity or an out of range value is refused with a typed `Error` and leaves the world exactly as it was.

## What may differ between machines

Measured:

- The authority layer is the same on four targets: the golden tape and 20 corpus blocks are equal on linux-x86_64, windows-x86_64 and linux-aarch64 (mirror run 39032745322) and on wasm32 (probe run 39036560799).
- Rapier built with `enhanced-determinism` (pure-Rust libm, no hardware transcendentals) gave the same ragdoll digest `5d7aedd4c6095b7c` on native and on wasm, scalar and simd128. That is one scene on one V8; it does not make Rapier an authority engine on every CPU.

So the verdict (who fell, what broke) never differs. Transforms from `PhysFx` agree on one machine for the same inputs and may drift by centimeters across machines, which is allowed because nothing there is authority. Keep relaxed-simd off.

## Budgets: caps per tier

One table, `Tier::caps` in `crates/physfx/src/caps.rs`. When a spawn does not fit, the oldest body (a whole ragdoll counts as one unit) is recycled first. Contacts over the cap shed the oldest limbs and debris, never a ragdoll. The values are the first cut; the `ladder` tests tune them from measured cost. See [Quality tiers](quality-tiers.md) for the table.

## Use it

```rust
use engine_physfx::{PhysFx, Tier};

let mut fx = PhysFx::new(Tier::Web);
fx.add_static_box([20.0, 0.5, 20.0], [0.0, -0.5, 0.0])?; // the floor
// authority said fighter 3 is floored: hand over its animated pose, speed and the blow
fx.spawn_ragdoll(3, &rig, &pose, [1.0, 0.0, 0.0], [0.0, 0.0, 400.0])?;
fx.step(); // once per sim tick, 60 Hz
let placed = fx.transforms(alpha)?; // blend the last two ticks for the render
println!("{}", fx.stats().receipt_line());
```

The tests that prove each rule are in `crates/physfx/src/tests.rs`. Run them on a GitHub runner:

```sh
node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib the_same_inputs_give_identical_transforms_twice
```

## Add a break (a wall, a crate)

1. Authority first. A break is a hashed event from `engine-destruction` (`BondedCollection`, `BreakEvent`). Author and test the fracture through the door: the `break.fracture` verb takes `size_m`, `hit`, `cells` (2 to 64), `radius_m` and `seed` and returns the pieces, the break events and a digest. Same request, same bytes.
2. Physical second. Hand the shards of the broken piece to `PhysFx::spawn_debris` (stone) or `spawn_debris_of` (any `Material`) with an impulse. The pieces sleep when they settle and the oldest are recycled at the cap.
3. Prove the authority side with the break corpus: `cargo test -p engine-proof break_corpus` twice gives one digest.

```sh
node tools/gh-check.mjs cargo test -p engine-proof break_corpus
```

## Add a limb

1. Authority first. A cut and a limb loss are decided in `crates/combat/src/cut.rs` and `limbloss.rs` and tracked in `crates/arena/src/limbs.rs`. The cut event is 16 bytes and hashed. A limb exists in the rules only when that event says so.
2. Physical second. On that event, call `PhysFx::spawn_limb` (flesh) or `spawn_limb_of` with a `Material`, a `BoneShape`, the transform of the joint and a velocity. `part` names the limb for the read-back.
3. Check the budget: the scene of 8 ragdolls, 200 debris bodies and 6 limbs must stay under the tier's step cost.

```sh
node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib budget_eight_ragdolls_two_hundred_debris_six_limbs
```

## What is not built yet

The window game still runs the kinematic match; ragdolls, flying limbs and debris are not drawn there. The rows that wire `physfx` into the game (RZ2 ragdoll on knockout, RZ3 limbs that come off, RZ4 walls and crates that break) are open on the board (`docs/SPRINT-22-web-realism.md`). Today the layer exists as a library with tests.
