# GS30 deep-dive 2026-10-14 (planner thinker for the stuck row)

Row: GS30 spatial isovist (board `docs/SPRINT-18-steals.md:324`).
Tier A, Size M. Still open (`[ ]`).

## Symptom

Code is landed on master but the row never ticked. Commits
`cf8bd3c9` (types plus area), `977fa283` (row fixtures),
`9b9a52d8` (coder slice), `b16e3986` (clippy). The module is
tracked: `crates/spatial/src/isovist.rs:1`. Five tests cover the
row Accept. The board box is still `[ ]`.

## Root cause

Three conjuncts, none of them a code defect in the fixtures:

1. Spec versus implementation. The row prescribes a rotational
   sweep with an active-edge heap. The code casts three integer
   rays per endpoint (`crates/spatial/src/isovist.rs:243`) with
   `K = 1024` offsets over a quadratic crossing split
   (`crates/spatial/src/isovist.rs:139`). Fixtures pass, but no
   budget binds the substitution at arena wall counts. The row
   cannot tick on fixtures alone while the substitution is
   unproven.
2. Verify environment. `docs/BLOCKERS.md:34` records engine-spatial
   lib tests link-red x2 (rust-lld undefined llvm symbols, same
   fingerprint as GS27). Any crate-wide P2P gate fails the slice
   for the toolchain, not the code. Re-sending GS30 whole repeats
   that wall.
3. Island risk plus overlap. `isovist` and `visible_area_cm2`
   (`crates/spatial/src/sight.rs:488`) have zero production
   callers outside tests. That is the shape that abandoned GS31
   (`docs/BLOCKERS.md:43`). And `docs/plan/forge-adoption.md:698`
   puts GS30 near SP65 `view_sees`: one home per job. Meanwhile
   `sight.rs` is modified inside a ~150-file dirty tree, so a
   whole-row resend also risks a foreign-file gate.

## Different approach

Stop resending GS30 whole. One test-only slice, then tick:

1. GS38 (new, Size S, needs none, owns the test hunk of
   `crates/spatial/src/isovist.rs` only, never `sight.rs`):
   lock the budget and the edge semantics. Accept: an
   arena-scale wall set (at least 200 walls) sweeps under 2 ms
   p95 with 0 allocs past setup, x2 equal; open ground reports
   0; origin on an endpoint is documented and tested; the four
   row fixtures stay green. F2P: no budget number exists. P2P:
   `cargo test -p engine-spatial --lib isovist` green x2 (exit
   0), then the full spatial lib green with its count pasted
   (exit 0), golden `d57b5c8ae3096614` unchanged.
2. Then tick GS30 on its own Accept (square corners at
   `crates/spatial/src/isovist.rs:312`, pillar shadow at `:322`,
   room area at `:345`, crossing stability at `:371`, x2 at
   `:316`). No new module, no rewrite to the heap.
3. Scope boundary (Row changes below, no duplicate row): GS30
   owns the 2D polygon plus `area_cm2` readability only.
   `view_pick` and `view_sees` stay SP65 in
   `crates/game/src/view_truth.rs`
   (`docs/SPRINT-18-gaps.md:1636`). Different homes, both stand.
4. If the lib gate links red again, end BLOCKED with the exact
   linker line and receipt. Do not resend code. Code needs no
   change; the toolchain or a quiet window does.

## Proof command

Judge verify for GS38 and the GS30 tick (each exit 0):

`cargo test -p engine-spatial --lib isovist` green x2;
`cargo test -p engine-spatial --lib` green with its count
pasted; `node tools/workspace-check.mjs` PASS after the board
move; golden `d57b5c8ae3096614` unchanged. F2P per board: no
isovist fn. P2P: spatial lib green; golden unchanged.

Done-when for this packet: this file exists and names the
verify above. Done-when for GS38: the proof command, exit 0.

Donor (pattern only, reimplemented integer-cm std-only, no
vendored code): kratocz/godot-visibility-polygon (CC0) trunk
@ `d87e4c27cb36b9ad004b8b8dffa7f9efb65663a6` (via
api.github.com commits listing; Asset Library download commit
`3666007881b6348142ddec7424ddc47bc8b0d5db`). Upstream ports
byronknoll/visibility-polygon-js (public domain). Take: split
crossings first, then sweep. Deliberate substitution: our
3-rays-per-endpoint integer fan replaces the donor O(N log N)
active-edge heap; GS38 proves the bound instead of rewriting.
