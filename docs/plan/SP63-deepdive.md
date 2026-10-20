# SP63 deep-dive 2026-10-14 (planner thinker for the stuck gate)

Rows: SP63 render slice 1 (draw-id target, board `docs/SPRINT-18-gaps.md:1600`, needs EG2 ticked at `docs/SPRINT-18-gaps.md:143`), SP63b slice 2
(reduction + ring readback, board `docs/SPRINT-18-gaps.md:1609`, needs SP63), SP63c slice 3 (`arena pixels:`
receipt + product floors, board `docs/SPRINT-18-gaps.md:1618`, needs SP63b). Tier A, Size M each. All three still open (`[ ]`).
FOCUS Now Look row (`docs/FOCUS.md:7`).

## Symptom

Three second-identical failures in 48 h to 2026-10-14, zero rows ticked, work done but
unlanded. 2026-10-16 note: the `docs/BLOCKERS.md:35` / `:46` / `:53` cites below are stale — `:35` today is EG11 window build (`v1_skin.rs:769` E0308), `:46` is `provider-json-vs-audit`, `:53` is EG29 slice-1 foreign red (see `docs/BLOCKERS.md:35`, `:46`, `:53`).
2026-10-21 note: `docs/BLOCKERS.md` was rewritten on 2026-10-21 and 122 pre-standalone rows moved to `docs/archive/BLOCKERS-to-2026-10-17.md`, so every `docs/BLOCKERS.md:NN` line number below now points at a different row: the three named rows read `docs/archive/BLOCKERS-to-2026-10-17.md:32` (EG11 window build), `:43` (provider-json-vs-audit) and `:50` (EG29 slice-1 foreign red).
Current SP63 rows are `docs/BLOCKERS.md:28` (`loop-sp63-a2-repair` land REFUSED x10; now `docs/archive/BLOCKERS-to-2026-10-17.md:25`), `:51` (SP63 box-id closeout, `v1.rs:1844` no field tangents; now `:48`), and `:54` (SP63 id-saturates, foreign gate plus link-env; now `:51`). Original 2026-10-14 symptom was: SP63-a1 verify BLOCKED x2 on a foreign
compile error (`quality.rs:197`, owned by the running GS10 slice; SP63
code on disk unverified); SP63 id slice BLOCKED
x2 on pre-existing `frame.rs:461` targets-none after resize (id_target 4
passed 1 failed twice; slice hunks in `forward.wgsl`/`submit.rs` green,
packet-gate FAIL only on pre-existing long fns outside the slice);
`loop-sp63-a2-repair` land REFUSED x10 (ratchet
regresses staged render files, baseline not staged).

On disk today (2026-10-14 tree, `git status --short` re-read today): `draw_id.rs:1` (slice-1 CPU contract, `CLEAR` at
`draw_id.rs:14`, `id_of_draw` at `draw_id.rs:25`) and
`draw_id_reduce.rs:25` (slice-b `reduce_to_bodies`) are untracked new
files; the GPU side is wired (`frame.rs:12` `R32Uint` format plus
`frame.rs:14` `ID_CLEAR`, `frame.rs:257` id_texture field,
`submit.rs:419` id target create, `submit.rs:845` clear,
`forward.wgsl:332` flat `draw_id` in, `forward.wgsl:607` id out, and the
`forward.wgsl` id hunks are already committed); the resize fix sits in
staged-but-uncommitted hunks (`frame.rs:467` `ensure_targets` after
resize, tests at `frame.rs:439`, `:451`, `:490` plus submit tests at
`submit.rs:1740`, `:1789`); `quality.rs:192` shows only single asserts
now, no 13-tuple, but `quality.rs` itself is an untracked foreign file
(GS10 lane, still unlanded). Slice c is untouched: no `arena pixels:`
anywhere in `view_truth.rs` or `tools/product-check.mjs`, whose gate is
still the PNG blank/frozen floors (`product-check.mjs:74`).

## Root cause

Three conjuncts, none inside the row Accepts:

1. Foreign-file gate. SP63-a1 died on `quality.rs`, a file its packet
   never owned. Any crate-wide lib gate fails the slice for outside
   dirt. Same shape as the LF4 and EG11 rows.
2. Pre-existing target lifecycle. Resize drops targets (lazy rebuild),
   so the cold readback ran targets-none at old `frame.rs:461` (the
   `docs/BLOCKERS.md:46` failure line; 2026-10-21 note: that line number moved with the 2026-10-21 BLOCKERS rewrite, the row is now `docs/archive/BLOCKERS-to-2026-10-17.md:43`). The fix
   (`ensure_targets`, on disk staged at `frame.rs:467`, defined at
   `submit.rs:343`) lives in a foreign long-fn file, so the packet
   gate FAILs the slice for out-of-scope lines even with slice hunks
   green.
3. Land friction. Untracked CPU modules plus staged render hunks on a
   ~255-path dirty tree (counted today) cannot land whole: the ratchet
   baseline REFUSED x10 because `rust-quality.baseline.json` never rode
   the same commit. Resending SP63 whole repeats the refusal.

Scope note: slice 1 Accept is an offscreen fixture only (box id on its
pixels, clear elsewhere, x2 equal, 0 validation errors,
`docs/SPRINT-18-gaps.md:1600` lines 1605-1608); the failing test exercised the resize
path, which the Accept never promised.

## Different approach

Stop resending SP63 whole. One lane finishes in this order:

1. Land the two foreign fixes first, each its own <=200-line slice with
   the baseline riding the same commit: the `quality.rs` GS10 fix
   (untracked on disk today, still unlanded), then
   the `frame.rs:467` resize `ensure_targets` fix (staged but
   uncommitted) via
   `loop:sp63-frame-targets`. Neither touches SP63 scope.
2. Then one slice-1 packet owning only the unlanded id hunks:
   `frame.rs` id-target hunk, `submit.rs` id hunk, new `draw_id.rs`,
   `frame_id_tests.rs` (`forward.wgsl` id output is already committed,
   not re-owned). No resize test beyond the already-landed fix.
3. Slice b (`draw_id_reduce.rs` CPU plus the compute reduction and
   3-buffer ring mapped two frames later) only after SP63 ticks; slice c
   (`arena pixels:`, brief `px`/`mean_rgb`, product floors, PNG floors
   retired after a green week) only after SP63b. Never bundle b/c into
   slice-1 packets.
4. Land narrow on a clean tree; never resend land until the baseline is
   staged (the x10 REFUSED lesson).

Donor (pattern only, reimplemented, no vendored code): gfx-rs/wgpu
trunk @ `60036cdc0ad979c96ec8277df41ec35b098502f6` (verified live
2026-10-14 via api.github.com commits lookup: "Fix Cargo lint from
Rust 1.100 (#10576)", dated 2026-10-14) — `R32Uint` second attachment
plus copy-texture-to-buffer with deferred map, the shape slice 1/b
follows.

Done-when for this packet: this file names the board rows with their
`docs/SPRINT-18-gaps.md` lines, the tree status above matches
`git status --short`, and the proof command below is the judge's
verify (each exit 0). Done-when for the lane: the proof command green
x2, then SP63 ticks on its own Accept; SP63b only after SP63 ticks,
SP63c only after SP63b.

## Proof command

`cargo test -p engine-render --lib id_target` green x2 (5/5:
`id_target_cleared_to_max`, `id_target_stays_clear_on_offscreen`,
`id_target_reports_box_id`, `id_target_pipeline_has_two_targets`,
`id_target_forward_writes_box_id`), then `cargo test -p engine-render
--lib` green with its count pasted, validation errors 0, golden
`d57b5c8ae3096614` unchanged. F2P per board: no id target. P2P: render
lib green; golden unchanged.
