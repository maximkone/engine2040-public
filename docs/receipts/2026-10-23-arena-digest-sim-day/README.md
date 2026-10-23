# Arena digest: the sim day since TC3 (2026-10-23)

Board history: `crates/proof/src/arena_matrix.rs` pins the rules-v2 arena
match digest, and `docs/receipts/2026-10-21-tc3-body-push-shove/README.md`
records the last move (`2fbd56040fd091a7` -> `9f162a2fe205f0ff`,
commit `4e53de91b`). This receipt records the next move.

## What moved

| fixture | before | after |
|---|---|---|
| arena match, `Fixture::STANDARD` (rules v2, kit walls), live path | `9f162a2fe205f0ff` | `605f6503a39082f4` |
| the same match through the spine step (feature `spine`) | `9f162a2fe205f0ff` | `605f6503a39082f4` |

Both literals moved together in `arena_digest_is_pinned` and
`spine_arena_digest_is_pinned`, so the live path and the spine path still fold
one number.

## Why it moved

`git log 4e53de91b..HEAD -- crates/{arena,combat,anim,physics,physfx,ai,core}/src`
counts 54 commits. The ones whose own message declares a behaviour change
(most sim-path commits are docs, tests, fmt or structure) are:

- FEEL2: `5270904d4` (hit impulse from weapon mass and blade travel) and
  `3a75ead54` (a blow moves the body by its momentum).
- FEEL4 `a409668fa`: a plate deflects instead of cutting (rules v2).
- FEEL6 `615637595`: a clash throws both blades back the same ticks.
- FEEL10 `d64dc8071`: bodies in contact trade momentum, not only position.
- `de9e71257` core: `Fixed` products round to nearest (sim arithmetic).
- `1843106d0` combat: a weapon weighs what it is; `b2e5de07b` the shield line.
- `2afcc9de9` ragdoll joint table; `59dbfaf70`, `061269529`, `c3a667e46`
  physics laws; `504c49d2b` swept casts; `42b023844` sub-millimetre travel.
- `584fd0bed` a spear swing comes to rest; `c6e838abd` the manuballista bolt;
  `d35cca7f1` the contubernium falls in.

Each landed with its own row proof; none is a silent change. TC4's ORCA
(`b0fde676e`) is opt-in (`Sim::set_steering`), so it does not arm in
`v2_free_for_all` and does not contribute to this value.

## The measurement

At HEAD `1662cc66d`:

- `cargo test -p engine-proof --lib --features arena arena_digest` fails on the
  stale literal and prints the new one: left `6944380216732058356`
  (`0x605f6503a39082f4`), right `11463396286662045951`
  (`0x9f162a2fe205f0ff`), run 39866796008.
- The tape's own laws pass on the same tree: `arena_tape_repeats_on_one_machine`,
  `arena_tape_follows_one_flipped_input`, `arena_tape_follows_the_seed` and
  `arena_tape_plays_rules_v2_on_the_kit_walls` — 5 passed, 0 failed,
  run 39867824812. The tape is deterministic, sensitive to one flipped input
  and to the seed.

After the move, both pins pass together: `cargo test -p engine-proof --lib
--features arena,spine arena_digest` — 2 passed (live and spine), 206 filtered,
run 39868149558.

## Call and limits

Called by the overnight lead under the owner's full-autonomy instruction
(2026-10-22): master carried a red identity gate that fails the full-suite
proof, and the move is loud, not silent. Reverting is one literal.

Limit a reader must know: no bisect to the first moving commit was possible in
the shared tree (worktrees are banned, and a separate clone would rebuild
several minutes per step), so the cause is attributed at the commit-message
level, not by checkout. The pin proves identity, not quality; the FEEL, physics
and combat rows carry their own behaviour proofs.

The golden does not move: `GOLDEN_600` (`d57b5c8ae3096614`) covers the legacy
2D fixture only and the arena is outside its scope (`docs/HASH-LAW.md`).
