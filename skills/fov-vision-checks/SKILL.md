---
name: fov-vision-checks
description: "Use when a builder or judge adds tile visibility, sight-gated pursuit, or explored-map memory in crates spatial, ai, game, or render, before the diff lands."
version: 0.1.0
author: skillworks
---

# FOV vision checks

Eight checks for viewer-based sight. Name the crate in each finding (spatial, ai, game, render, ecs, ui, perf, core). PASS needs a test or measured cost per touched item, never a promise.

1. **Opacity apart from blocking.** Ask: does vision blocking live in its own flag apart from movement blocking? Flag: one flag drives both, so a slit or window behaves wrong. Fits crates/spatial for tiles, physics for motion.
2. **Visible set per viewer.** Ask: does each viewer keep its own deduped point set plus a radius tuned per kind? Flag: one shared list or one radius for all kinds. Fits crates/spatial and ai.
3. **Dirty-gated recompute.** Ask: is the set rebuilt only when its owner moves, starting dirty and clearing after the run? Flag: full rebuild every tick for every viewer. Fits crates/ecs and game.
4. **Order after movement.** Ask: does the sight pass run after movement settles and before draw and chase, in each schedule that moves? Flag: draw or chase reads last turn's spots. Fits crates/game.
5. **Draw gated on sight.** Ask: are tiles, bodies, and hover tips shown only when the viewer set holds the point? Flag: hidden foes drawn or named by hover. Fits crates/render and ui.
6. **Chase gated on sight.** Ask: does pursuit start only when the target point sits inside the chaser set and stop when sight breaks? Flag: foes path through walls at all ranges. Fits crates/ai.
7. **Memory kept apart.** Ask: is explored state a separate bool array set from the viewer set on move and never cleared, seen drawn bright and recalled dim? Flag: recall stored inside the live set or wiped each move. Fits crates/spatial and render.
8. **Same input same set plus cost.** Ask: do the same origin, radius, and map yield the same set, with build cost measured at real viewer counts? Flag: shifting order leaks into sim state, or full-map scans with no number. Fits crates/perf and core.

## Do / Don't
- Do: recompute on move, gate chase and draw on set membership, and keep recall apart from sight. Don't: scan each tick, chase through walls, or fold recall into the live set.

source: Wolverson, Hands-on Rust, ch. 10
