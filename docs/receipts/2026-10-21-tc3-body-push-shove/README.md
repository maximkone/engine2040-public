# TC3 body-push shove receipt (2026-10-21)

Board row TC3 in `docs/SPRINT-27-final-picture.md`. The row is armed by owner
decision 2026-10-21 (like AIN6): a moved v2 hash lands with a receipt under
`docs/receipts/`. This is that receipt.

## What moved

The arena collision pass now measures the push-apart displacement each fighter
took in one tick and feeds that fighter's `BalanceState` a shove scaled by the
push. Rules v2 only: under v1 the pass never runs, so no shove is fed.

| site | before | after |
|---|---|---|
| `crates/arena/src/collide.rs` collision pass | no balance slice | `PUSH_SHOVE_TABLE`, the push measurement, the shove into `BalanceState` |
| `crates/arena/src/lib.rs` `Sim::collide` | `collide_states(states, rules, half, fighters)` | `collide_states_shove(states, balance, rules, half, fighters)` |
| `crates/arena/src/spine_step.rs` `after_motion` | `collide_states(states, rules, half, fighters)` | `collide_states_shove(states, balance, rules, half, fighters)` |

The last two rows sit outside the paths the row owns (`collide.rs`, `live.rs`).
The shove has to land in the shared tick path: the live match and the spine
step must fold the same checkpoint hash, so a live-only shove would split the
two digests. `collide_states` keeps its old shape for the spear drag, which
carries no balance (a drag is not a body push).

## The table

`PUSH_SHOVE_TABLE` in `crates/arena/src/collide.rs` holds every number:

| push over (mm) | impulse (Q8) | what the fighter feels |
|---|---|---|
| 40 | 128 (`BALANCE_BLOCK_Q8`) | a light blow: the stance holds, the shove counter moves |
| 256 | 256 (`BALANCE_PARRY_Q8`) | a stagger |
| 384 | 384 (`BALANCE_HIT_Q8`) | a stumble |
| 640 | 640 (`BALANCE_FALL_Q8`) | a fall |

A push of 40 mm or less feeds nothing. The impulses are the arena's own EG34
balance steps (`crates/arena/src/lib.rs`). The physics tuning behind them
staggers at 256, stumbles at 384 and falls at 640
(`crates/physics/src/balance.rs`).

The table sits in `crates/arena/src/collide.rs`, the path the row owns, and
names the arena's balance constants. It does not sit beside `BalanceParams` in
`crates/physics/src/balance.rs`: that file belongs to the physics lane, and
the row owns only the two arena paths.

## The digest

| fixture | before | after |
|---|---|---|
| arena match, 1200 ticks, rules v2, kit walls, five targets | `2fbd56040fd091a7` | `9f162a2fe205f0ff` |

Both pins moved together in `crates/proof/src/arena_matrix.rs`:
`arena_digest_is_pinned` (the live path) and `spine_arena_digest_is_pinned`
(the spine path). The live path and the spine path fold the same shove, so the
two digests stay equal.

The golden does not move. `GOLDEN_600` covers the legacy 2D fixture only, and
the arena is not in its scope (`docs/HASH-LAW.md`), so
`d57b5c8ae3096614` stands.

## The checks

- `node tools/gh-check.mjs cargo test -p engine-arena --lib collide`: the
  crowd test, the shove table and the push measurement pass.
- `node tools/gh-check.mjs cargo test -p engine-arena --lib`: the whole arena
  crate passes.
- `node tools/gh-check.mjs cargo test -p engine-proof --features arena,spine --lib arena_`:
  the live and the spine digest match the new pin.
- `node tools/gh-check.mjs cargo clippy -p engine-arena --all-targets -- -D warnings`:
  clean.
- `node tools/gh-check.mjs --budget 840 cargo test -p engine2040 --release --test proof_cli default_and_max_seed`:
  the golden prints `d57b5c8ae3096614` twice.
- `rustfmt --edition 2021 --check crates/arena/src/collide.rs crates/arena/src/live.rs`:
  clean.
- `node tools/rust-quality.mjs --owned crates/arena/src/collide.rs --owned crates/arena/src/live.rs`:
  PASS, no ratchet regression.
- `node tools/workspace-check.mjs`: PASS.

The run ids are in the board row that ticks TC3.
