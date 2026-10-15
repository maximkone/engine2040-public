---
name: capture-pixel-checks
description: "Use when a builder wires a window capture or a frame gate and when a reviewer checks that a pass verdict comes from decoded pixels in crates game, render or playtest."
---

# Capture pixel checks

A capture proves pixels, never counts. Name the crate per finding (game, render, playtest).

1. **Presented surface only.** Ask: does the capture copy the presented framebuffer from video memory to main memory in crate game? Flag: an offscreen CPU raster labelled as the window; a manifest trusted for the picture. (game)
2. **Dated names in one folder.** Ask: is each shot written to a fixed folder with a date and time stamp name in crate game? Flag: overwrote last good shots; two runs share one filename. (game)
3. **Decoded pixels for metrics.** Ask: are SSIM and color deltas run on decoded RGBA bytes in crate playtest? Flag: mean similarity over PNG bytes where one pixel looks huge and a wrong frame looks close. (playtest)
4. **Fighter floor per frame.** Ask: is there a fighter-colour pixel count above a floor on every checked frame in crate playtest? Flag: a gate that passes on file counts while the frame shows no fighter. (playtest)
5. **Blank fails.** Ask: does a blank set fail the gate and a real capture set pass it in crate playtest? Flag: a gate that cannot say no; a blank manifest with fighter counts still passes. (playtest)
6. **HUD and debug flags.** Ask: do capture options name HUD on or off plus debug lines on or off in crate render? Flag: a look verdict polluted by HUD quads or missing the debug overlay it claims. (render)
7. **Resolution stated.** Ask: is the capture resolution stated, with quad tiles only when high-res is claimed, in crate render? Flag: a tiled combine with a shifted projection; a resolution claim with no pixels behind it. (render)
8. **Motion needs sequence.** Ask: is a motion claim backed by a frame-rate sequence, not one still, in crate game? Flag: a frozen-frame pass sold as smooth play; a movie check with one screenshot. (game)
9. **Proof on real sets.** Ask: is there a test where blank frames fail and staged captures pass with node and cargo green in crate playtest? Flag: synthetic frames only; the window path never runs in any gate. (playtest)

## Do / Don't

- Do: copy the presented surface, decode to RGBA, count fighter pixels above a floor, and fail blank sets.
- Don't: trust a manifest count, compare PNG bytes, or label CPU raster as the window.

source: Gregory, Game Engine Architecture (2nd ed.), ch. 9.7
