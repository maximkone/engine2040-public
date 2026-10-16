---
name: wave8-random
description: "Use when a staged-fight replay, capture set, or perf run looks wrong in engine2040 and the seat must decide in order whether it is a replay/desync fault, a capture-path fault, or perf noise before filing any bug packet."
---

# Replay-capture-perf triage

A wrong-looking run is three suspects in a fixed order: replay first,
capture path second, perf noise last. Name the crate per finding (game,
render, playtest, perf harness).

1. **Replay before pixels.** Ask: does the same fixture hash the same twice
   before any capture is read? Flag: a look verdict on a diverged replay;
   pixels from two different trajectories compared as one. (game)
2. **First divergent tick, not the last frame.** Ask: is the earliest tick
   where hash or checkpoint differs named? Flag: a bug packet filed on
   frame 600 when the split started at tick 40. (game)
3. **Capture path is the presented surface.** Ask: does the shot copy the
   presented framebuffer to dated files, decoded to RGBA for metrics?
   Flag: CPU raster labelled as the window; PNG-byte similarity standing
   in for pixels. (render, playtest)
4. **Blank fails, staged passes.** Ask: does the gate fail a blank set and
   pass the staged set on this tree? Flag: a gate that cannot say no;
   fighter counts from a manifest with no fighter on screen. (playtest)
5. **Perf is paired and exclusive.** Ask: are numbers paired runs on one
   idle machine with p50/p95/p99, not one wall-clock sample beside a
   build? Flag: a regression claimed from a loaded box; a speedup with no
   baseline. (perf harness)
6. **Noise stays open.** Ask: is a borderline delta (inside run-to-run
   noise) left open, never ticked green or red? Flag: a row landed on
   overlapping distributions. (perf harness)
7. **One variable per rerun.** Ask: does each rerun change exactly one of
   fixture, build, capture flags, or machine load? Flag: a new binary plus
   new flags plus a busy GPU sold as one result. (game)
8. **Packet names the layer.** Ask: does the bug packet name replay,
   capture, or perf with the failing file:line and the receipt? Flag: a
   packet that blames all three and proves none. (game, playtest)

## Do / Don't

- Do: hash twice, bisect to the first divergent tick, then read decoded
  presented pixels, then take paired perf numbers.
- Don't: compare captures across diverged replays, trust manifest counts,
  or land a verdict inside the noise band.

## Skill-on cases (load this skill)

- On: a pilot reports fighters missing at t600 but the manifest lists
  counts; triage order says re-hash, then check the capture path.
- On: two perf CSVs disagree by 3% on a shared box; the skill keeps the
  row open and orders an exclusive paired rerun.

## Skill-off cases (do not load)

- Off: the golden hash already moved with a receipt naming a sim change;
  that is desync-debug, not triage.
- Off: the task is to report paired numbers on a known-good fixture;
  that is engine-bench, no fault suspected.

source: superpowers systematic-debugging + GGPO dev guide + GEA ch. 9.7 triage order, adapted to the engine2040 staged-fight loop
