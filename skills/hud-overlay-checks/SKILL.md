---
name: hud-overlay-checks
description: "Use when a builder draws a HUD, menu or text overlay in the arena window and a reviewer checks overlay order, coordinates and font atlases."
version: 0.1.0
author: skillworks
---

# Hud overlay checks

Nine checks for HUD quads over the arena frame. Name the crate per finding (render, game, ui). PASS needs a pixel test plus `cargo test -p engine-render --lib` green.

1. **Draw last, no depth.** Ask: does the overlay pass run after the blit with depth test off so quads sit over the scene? Flag: HUD quads in the 3D pass or depth-tested so fighters hide them. Fits crate render.
2. **One quad pipeline.** Ask: is there one textured-quad pipeline with alpha blend and a single draw for all cells? Flag: one pipeline per widget or one draw per quad that grows with HUD size. Fits crate render.
3. **Layout feeds quads.** Ask: does a dynamic quad ring fed by ui layout cells own positions, with game only sending hp and state? Flag: scene draws building HUD or game pushing pixels per frame. Fits crate game.
4. **Normalized y, aspect x.** Ask: are overlay coords 0..1 on y with x scaled by aspect (1.33 at 4:3, 1.78 at 16:9)? Flag: pixel coords baked for one resolution so layout breaks on resize. Fits crate ui.
5. **No square stretch.** Ask: is no layout 0..1 on both axes at once? Flag: a square bar that looks square in data but wide on a 16:9 screen. Fits crate ui.
6. **Anchored edges.** Ask: do positive offsets anchor top-left and negative anchor bottom-right, with center or element anchors where needed? Flag: right-edge bars that drift when aspect changes. Fits crate ui.
7. **Atlas plus metrics.** Ask: does one font atlas carry glyph boxes, kerning and baseline in a metrics file? Flag: per-glyph uploads or missing advance so text overlaps. Fits crate render.
8. **Text fits the game.** Ask: does text cover the shipped charsets and read directions, with only features the game uses? Flag: a full animation suite for static hp numbers. Fits crate ui.
9. **Pixel proof.** Ask: does the arena capture show hp bars where scene pixels sat, with on-vs-off rects differing? Flag: a receipt claim with no capture rect that moved. Fits crate game.

## Do / Don't
- Do: draw one blended quad ring after the blit in 0..1 coords and prove hp rects move pixels. Don't: depth-test HUD in the 3D pass or bake pixels for one window size.

source: Gregory, Game Engine Architecture (2nd ed.), ch. 10.4.4
