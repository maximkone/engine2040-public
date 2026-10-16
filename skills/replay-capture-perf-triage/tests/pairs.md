# Skill-on vs skill-off pairs: replay-capture-perf-triage

Failure: wave7 measured loop has no fresh frame table and no machine
score (perf CSVs stop 2026-10-11, staged replay unchecked, capture
path unverified). Triage order is replay first, capture second,
perf noise last.

## P1: fighters missing at t600, manifest lists counts (capture gap)

Situation: pilot reports fighters missing at frame 600 but the
manifest lists fighter counts.

Off (no skill): blames crates/render, files a render bug packet on
the last frame, trusts manifest counts as pixels.

On (with skill): re-hash the same fixture twice first (game); bisect
to the first divergent tick (split started tick 40, not frame 600);
no capture verdict until replay matches; then check the presented
surface to dated files decoded to RGBA (render, playtest).

Winner: On. Off files on the wrong layer; On stops the wrong verdict.

## P2: two perf CSVs disagree by 3% on a shared box (frame cost)

Situation: two perf runs disagree by 3% on a box running a build
beside the run, no machine score, no p50/p95/p99.

Off (no skill): claims a 3% regression and files a cut packet for
the slowest scenario.

On (with skill): numbers must be paired runs on one idle machine
with p50/p95/p99; a borderline delta inside run-to-run noise stays
open, never ticked; one variable per rerun; orders an exclusive
paired rerun on the same fixture and build.

Winner: On. Off lands inside the noise band; On keeps it open.

## P3: one bug packet blames all three layers (triage gap)

Situation: a run looks wrong and the draft packet blames replay,
capture, and perf together with no file:line and no receipt.

Off (no skill): files the vague packet; keeper cannot assign it.

On (with skill): names exactly one layer with the failing
file:line and the receipt (hash rerun, blank-vs-staged gate, or
paired perf paths); a packet that blames all three and proves none
is rejected before filing.

Winner: On. Off is unassignable; On is actionable.

Score: On wins 3 of 3.
