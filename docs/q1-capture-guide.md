# Q1 capture guide - 0-PNG to 3-PNG path

This guide closes the 0-PNG gap.
No GPU is needed to read it or run checks.

## Current counts (2026-10-09, read-only)

- target/q1 top-level fighter PNGs: 0 / 3.
- target/q1/captures/: 4 PNGs.
- target/q1/2026-10-05/orbit/: 30 PNGs.
- Q1 total: 34 / 34 staged plus orbit stills present.
- RA8 files in target/q1/: 0, need 1 or more. Manifest missing.

## Repeatable path

1. Keep the staged scene fixed. Same tape gives same PNGs.
2. Capture V1 hero, V2 arena-day, V3 fight stills.
3. Save 3 PNGs as target/q1 top-level.
4. Keep captures and orbit as is. Do not rename frames.
5. Run the checks below. Fix gaps, then re-run.

## Tool commands (read-only, no build)

- `node tools/ra8-check.mjs` - top PNG count plus RA8 presence.
- `node tools/q1-check.mjs` - synth 4 plus orbit 30 plus manifests.
- `node tools/fighter-count.mjs --json` - box vs fighter pixels.
- `node tools/showcase-gate.mjs` - newest set fresh plus fighters over 0.

## Done when

- PNG is 3 or more in target/q1 top-level.
- `ra8-check` shows `png_count: 3 (expected 3)`.
- RA8 manifest present (1 or more ra8 files).
- `showcase-gate` passes with fighters over 0.

## If stuck

- Write `skipped: plus why`. Second same failure ends the step.
