# EG8 deep-dive 2026-10-12 (planner thinker for the stuck gate)

Row: EG8 render - a pass is live only if pixels change (GX6 truth gate).
Needs EG2 is ticked. Row still open.

## Symptom

15 helpers in 72 h (13 builder, 2 judge), still open, no planner in 48 h.
Five landings in about 18 h, all in one file, `crates/proof/src/gpu_live.rs`:
a39b9c07, 42888443, 6ea9df22, d8b80809, d37590bb.

## Root cause

Every packet scoped itself to the proof crate only ("No render, game, sim"),
with synthetic 2x2 fixtures and no device. The oracle pieces are done and
green: `frame_diff_count`, `count_masked_ignored`, `live_by_fraction`,
`pass_verdict`, `resolve_pass_live`, `truth_table_markdown`,
`pass_truth_report` (15 `live_` tests green x2).

The row Accept needs the live leg: render the arena frame with each look
pass on and off, set each pass flag from the masked diff, write
`target/reports/pass-truth-<date>.md`, gate run x2 with the same table,
forward LIVE, sky and decals PROBE. That leg lives in the render crate and
the live scene, outside every packet's claimed paths. So writers keep
landing safe slices and the row never ticks. Latest packet state
(`target/queue/done/0-eg8-port-pass-truth.md`) says it plainly: resolver
plus writer landed, and the live capture leg is still missing.

## Different approach

Stop slicing proof helpers. They are done. One writer packet takes the
single live leg:

1. Toggle harness in the render crate: render the arena frame once per
   look pass, pass on and pass off, reusing the RA2 pass order.
2. Feed each on/off capture pair to the landed `resolve_pass_live` with
   the row epsilon and floor. Set each pass flag from its verdict.
3. Emit `target/reports/pass-truth-<date>.md` through the landed
   `pass_truth_report`. Sky and decals stay PROBE while their masked diff
   is at or under epsilon.
4. Run the gate twice. Same table both times.

If the pinned machine has no adapter for the capture step, end BLOCKED
with the exact failing line. Do not slice another pure helper.

## Proof command

Gate run x2 gives the same table; `cargo test -p engine-proof --lib live_`
green twice; `cargo test -p engine-render --lib` green; golden
`d57b5c8ae3096614` unchanged.
