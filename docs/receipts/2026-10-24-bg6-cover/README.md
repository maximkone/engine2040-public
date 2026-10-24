# Cover hides a foe from the rules-v2 plan (2026-10-24)

Board row: `docs/SPRINT-35-book-gaps-4.md` BG6 (book gap, Worlds II.6
L7450-7456). The rules-v2 melee plan (`crates/arena/src/engage.rs`) ranked
foes by separation alone: an obstacle between two fighters changed nothing,
so a fighter standing behind the kit's `crate_a` was seen and fought as if
the crate were not there.

## What moved

| fixture | before | after |
|---|---|---|
| arena match, `Fixture::STANDARD` (rules v2, kit walls), live path | `605f6503a39082f4` | `3886d0f78453149c` |
| the same match through the spine step (feature `spine`) | `605f6503a39082f4` | `3886d0f78453149c` |

Both literals moved together in `arena_digest_is_pinned` and
`spine_arena_digest_is_pinned`, so the live path and the spine path still
fold one number.

## Why it moved

BG6: `MatchWorld` now keeps its canonical obstacles as cover geometry
(`CoverBox`, `crates/arena/src/world.rs`) and the rules-v2 plan tests every
foe pair's sight line against it (`crates/arena/src/engage.rs`,
`cover_table` + `sightings`). A foe whose line crosses an obstacle never
enters a `FoeView`, so the brain does not aim at it and does not press it.
The fixture plays on the kit walls, so the plan changes on the first tick
where a wall stands between two fighters, and every later tick of the match
follows.

Rules v1 never runs the plan, so the v1 golden cannot move.
`GOLDEN_600` (`d57b5c8ae3096614`) covers the legacy 2D fixture only and the
arena is outside its scope (`docs/HASH-LAW.md`).

## The measurement

- The pin moved because the plan changed: `cargo test -p engine-proof
  --features arena,spine --lib arena_` on the tree with BG6 and the old
  literal fails on the stale pin and prints the new one: left
  `4073172674498729116` (`0x3886d0f78453149c`), right
  `6944380216732058356` (`0x605f6503a39082f4`), run 40062814173. The live
  path and the spine path printed the same new number.
- After the move both pins pass together with the tape's own laws: the same
  line, 9 passed, 0 failed (live pin, spine pin, the tape repeats on one
  machine, follows one flipped input, follows the seed, plays rules v2 on
  the kit walls, and the crate row), run 40063085359.
- The row's own test: `cargo test -p engine-arena --lib` on the BG6 tree:
  321 passed, 0 failed, 7 ignored, including the new
  `a_foe_behind_the_crate_stays_out_of_the_view` (run 40063261212).
- The lint gate: `cargo clippy -p engine-arena --all-targets -- -D warnings`
  PASS on the same tree (run 40063269581).
- The band and the golden: `cargo test -p engine-playtest --lib sweep_noise`
  PASS, the P3 band stays 400-600 permille (run 40062564113), and
  `cargo test -p engine2040 --release --test proof_cli default_and_max_seed`
  PASS, so `GOLDEN_600` is still `d57b5c8ae3096614` (run 40062564118).
- `node tools/rust-quality.mjs --crate engine-arena` -> RESULT PASS, score
  100 (A), no ratchet regression, no new cast.
- The tree the receipt lands on: another seat's commit swept the staged BG6
  blobs into `57f3ccdbc` while they sat in the index, so the code reached
  master inside that commit and not in a commit of its own. The same lines
  were therefore run again on plain HEAD (`--only` a path with no dirty
  content, so the build is HEAD and nothing else): the arena digest line
  passes both pins (run 40063620835), the arena suite is 321 passed,
  0 failed, 7 ignored (run 40063789404), clippy is PASS (run 40063792398),
  the golden is PASS (run 40063795546) and the band guard is PASS (run
  40063799136). The anim seat's change in the same commit did not move the
  digest: the value above is HEAD's value.

## Call and limits

Called by the book-gap lane: BG6 is a booked row with its own accept line,
and the digest move is a rules-v2 change, so it needs a receipt like TC3's.

Limits a reader must know:

- The value above is the one for the tree this receipt lands on. The
  readable tell in flight on 2026-10-24 (the v2 light startup 15 -> 30
  ticks) moves the same digest again: whoever lands it must re-pin with
  BG6 in the tree and name both causes in the pin's comment.
- The tournament band does not move because the tournament runs the v1
  rules (`engine_arena::run_prefix`): the plan is never built there, so
  cover cannot reach the band. The band leg above is a guard, not a
  measurement of cover.
- The pin proves identity, not quality: it says the arena match folds the
  same number twice, not that the cover rule is good play. The row's own
  test carries the behaviour.

## What this receipt does not claim

No pixel was looked at: BG6 changes what a fighter knows, not how it is
drawn. No new hash lane was added: the cover is read from the world's
obstacles, which the world digest already covers, so a world with cover and
a world without it never compare equal.
