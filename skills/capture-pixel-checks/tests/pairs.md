# Skill-on vs skill-off pairs: capture-pixel-checks

Three gate reviews. Off is habit; On names the check, the failure it stops, and the proof it asks for.

## P1: manifest-trusting frame gate (checks 4, 5, 9)

Input: a diff in `tools` keeps `gatePass = totalFighters > 0` from `manifest.json` plus a red-pixel count. A blank frame set with a manifest passes. Note says pixels only give red counts.

Answer with the skill: Reject. Check 4: count fighter-colour pixels above a floor on every checked frame in crate playtest. Check 5: a blank set must fail and a real capture set must pass in crate playtest. Check 9: add a test where blank frames fail and staged captures pass with node and cargo green in crate playtest.

What a skill-less answer misses: it calls producer counts a gate. A 0-fighter window stays green because the manifest says fighters exist.

## P2: PNG-byte golden compare (checks 3, 9)

Input: a diff in `playtest` scores captures with mean per-byte similarity over raw PNG bytes against golden files. One-pixel change scores large; a wrong image scores close. Goldens are not in the repo.

Answer with the skill: Reject. Check 3: run SSIM and color deltas on decoded RGBA bytes in crate playtest, never on PNG bytes. Check 9: prove it on real window sets where equal frames score SSIM 10000 with deltaE 0 and a 20 percent brightened region drops below 9800 in crate playtest.

What a skill-less answer misses: it approves compressed-byte math. Encoding noise reads as picture change and a wrong picture reads as close.

## P3: CPU raster sold as the window (checks 1, 6, 8)

Input: a diff in `game` captures 128x96 CPU dot-raster frames with staged boxes and exits 0, labelled as the arena smoke. HUD quads ride the receipt, never the draw list. A frozen frame is sold as smooth play.

Answer with the skill: Reject. Check 1: copy the presented framebuffer from video memory to main memory in crate game, or label the fallback as CPU raster. Check 6: name HUD on or off plus debug lines on or off in crate render. Check 8: back a motion claim with a frame-rate sequence in crate game, not one still.

What a skill-less answer misses: it ships a fallback as proof. The window path never runs, HUD never reaches the screen, and one still stands in for motion.
