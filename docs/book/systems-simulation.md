# Simulation systems: tick, fight, move, think

The authoritative game lives on a fixed 60 Hz tick. Same complete input
gives the same hash (`docs/HASH-LAW.md`). Presentation, IO, GPU, and
model calls never run inside the tick — they observe snapshots or submit
validated actions at tick boundaries. Full signatures live in rustdoc,
never copied here: `cargo doc -p <crate> --no-deps`.

## The tick and the match

- `engine-core` owns seeds, tick order, and hashes (std-only). No IO,
  no GPU, no wall clock in hashed state.
- `engine-ecs` owns entity lifetime, storage, and queries over the tick.
- `engine-arena` owns the headless deterministic match: fighters,
  checkpoints, forks on worker threads, and the decision recorder.
  The multiplayer and playtest spine runs through here.
- `engine-input` owns fighter IDs, bindings, dead zones, and rebinding.
  The scripted human in the staged fight builds `FighterInput` values
  (`tick`, `fighter`, `mx`, `mz`, `facing`, `buttons`).

Run the determinism golden (proof CLI, seed 2040, 600 ticks):

```powershell
cargo run --locked -- --verify
```

Expected: `final_hash=d57b5c8ae3096614` with `verified=true` — the
legacy golden from `CONTRIBUTING.md` and `tools/proof.mjs`. Corrupted
tapes fail naming the first divergent tick. Run the same fight twice
from the [quick start](quick-start.md) and the staged-fight
`final_hash` repeats exactly.

## Combat, physics, locomotion

- `engine-combat` owns the table-driven fight step (`fight.rs`) plus
  versioned replacement move tables (`tables.rs`: `MoveTableSet`,
  `validate_tables`, `TABLES_VERSION_V1 = 1`). Damage is permille,
  timing is ticks. V1 ships the CB1 row; new sets must pass validation
  before the arena swaps them at a recorded authority tick. Gore events
  are presentation-only.
- `engine-physics` owns the Rapier adapter with hash and
  snapshot/restore, the character controller, deterministic ragdoll,
  and integer-only vehicle math.
- `engine-anim` owns pose sampling, blending, root-motion extraction,
  the locomotion state machine, motion nets, motion matching v2, and
  traversal (vault, mantle, climb — probe fixtures pick the action,
  warped roots land within 1 cm, replay is exact).
- `engine-ai` owns the integer-utility gladiator policy and the net
  policy with scripted fallback. Curves are versioned (`HR4`).
- `engine-nav` owns pathfinding (single A* owner). `engine-world`
  emits cells; see the [presentation page](systems-presentation.md).
- `engine-destruction` owns pre-fractured collections with bonds and
  anchors plus the CSG fracture bake. The arena calls it: a sword hit
  in the match breaks a crate along the blade plane, with identical
  break sets twice.

Check the combat tables headlessly (verified this wave):

```powershell
cargo test --locked -p engine-combat --lib tables
```

Expected: `21 passed` (validation rejects bad rows, text copies
match compiled tables, old-version replays use V1, replacements
change outcomes). To tune a weapon, see
[Add a weapon](recipes-add-weapon.md) — it reuses this gate.

## Net and rollback

- `engine-net` owns the UDP LAN session, snapshot deltas, prediction,
  resimulation, and interpolation. `engine-game` runs the loopback LAN
  session with AI fill; the dedicated headless server adds discovery,
  reconnect with slot reclaim, and AI takeover.

What this page does not cover: pixels and frames live on the
[presentation page](systems-presentation.md); content cooking lives on
the [content page](systems-content.md); the MCP door and verifiers
live on the [AI door](ai-door.md), [verifiers](verifiers.md), and
[intelligence](systems-intelligence.md) pages.

## Reference

- `src/cli.rs` — the proof CLI (`--verify`, tapes, receipts)
- `crates/arena/` — match, checkpoints, forks
- `crates/combat/src/fight.rs`, `crates/combat/src/tables.rs` — step and move tables
- `crates/physics/`, `crates/anim/`, `crates/ai/`, `crates/nav/`, `crates/net/`, `crates/input/`, `crates/destruction/`
- `docs/HASH-LAW.md`, `docs/ARCHITECTURE.md`
