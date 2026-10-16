---
name: feel-fps-checklist
description: "Use when a builder or judge touches hit pause, shake, squash, flash or slow motion in crates combat, arena, render, game or ui and must prove the hit lands without breaking determinism or budget."
version: 0.1.0
author: miner
---

# Feel fps checklist

Eight proof checks for feel work. Name the crate in each finding. PASS needs an on/off pair plus three gates: frozen sim with live view, pure injected-time tests, and a release bench against the frame budget.

1. **Layer pair (game).** Ask: is there an off/on pair showing pause-off feels soft, pause-only feels empty, all-on lands?
   Flag: single all-on clip with no isolated comparison.
2. **Frozen sim, live view (arena).** Ask: while frozen does sim dt read zero yet the view still trembles from the live clock?
   Flag: view locked solid or sim time advancing during the freeze.
3. **Fixed step carry (core).** Ask: does the loop accrue real dt, run whole fixed steps, and carry the leftover slice forward?
   Flag: raw frame dt into sim or a dropped remainder.
4. **Visible numbers (ui).** Ask: does the HUD print live load plus remaining freeze so the pause is measured?
   Flag: eye-only verdict with no numbers on screen.
5. **Pure tests (combat).** Ask: do feel tests inject dt and avoid window, canvas, wall clock, and timer mocks?
   Flag: wall clock, random call, or canvas in the tested path.
6. **Release bench (perf).** Ask: are update and impact costs measured in release with calls-per-frame at real counts?
   Flag: debug-only number or no per-frame fit count.
7. **Decay table (perf).** Ask: does the measured decay table show halved load falling to about a quarter move?
   Flag: linear falloff or a table missing from the diff.
8. **Slow stretch (render).** Ask: under slow motion does the tremble stretch calm and cinematic with no decay while frozen?
   Flag: layers fading during freeze or shake unchanged by slow factor.

## Do / Don't
- Do: demand off/on pairs plus frozen-sim proof, pure tests, and bench numbers before PASS. Don't: pass a feel diff on a single all-on video with no numbers.

source: m2-md/game-feel-hitstop-juice v1.0.0, ch. Demo/Test/Benchmark
