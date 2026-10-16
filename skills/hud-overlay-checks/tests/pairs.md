# Skill-on vs skill-off pairs: hud-overlay-checks

Three HUD reviews. Off is habit; On names the numbered check, the failure it stops, and the evidence it needs.

## P1: HUD in the 3D pass (checks 1, 2, 9)

Input: a diff in `render` draws hp quads inside the forward pass with depth test on, one draw per bar. Note says fighters hide bars only at some angles.

Answer with the skill: Reject. Check 1: the overlay pass must run after the blit with depth test off in crate render so quads sit over the scene. Check 2: use one textured-quad pipeline with alpha blend and a single draw for all cells in crate render. Check 9: show an arena capture where hp bars sit on scene pixels, with on-vs-off rects differing in crate game.

What a skill-less answer misses: it accepts angle-dependent HUD. Bars vanish behind fighters and draw count grows with widgets, with no pixel proof.

## P2: pixel-baked layout (checks 4, 5, 6)

Input: a diff in `ui` lays out hp bars in 1280x720 pixels, 0..1 on both axes, top-left only. Note says resize is rare.

Answer with the skill: Reject. Check 4: use 0..1 on y with x scaled by aspect in crate ui. Check 5: never 0..1 on both axes in crate ui, or squares stretch on 16:9. Check 6: anchor positive top-left and negative bottom-right, with center where needed in crate ui. Proof: captures at 4:3 and 16:9 keep bars square and pinned.

What a skill-less answer misses: it ships one-resolution HUD. Bars stretch wide on 16:9 and right-edge bars drift off on resize.

## P3: per-glyph text with no atlas (checks 7, 8, 3)

Input: a diff in `render` uploads each hp digit as its own texture per frame, with no advance table. Note says the font is small so it is cheap.

Answer with the skill: Reject. Check 7: keep one font atlas with glyph boxes, kerning and baseline in a metrics file in crate render. Check 8: cover only shipped charsets and read directions in crate ui, no animation suite for static numbers. Check 3: let the ui quad ring own positions with game sending only hp in crate game. Proof: text draws in the one quad draw with no overlap.

What a skill-less answer misses: it passes overlapping digits as a font. Missing advances pile glyphs up, per-frame uploads churn, and game pushes pixels it should not own.
