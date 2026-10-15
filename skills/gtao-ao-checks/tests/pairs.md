# Skill-on vs skill-off pairs: gtao-ao-checks

Three review situations. "Off" is what a reviewer says from general habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the evidence it asks for.

## P1: constant ao_tex in the blit (items 5, 8)

Situation: a diff wires GTAO but `crates/render/src/shaders/blit.wgsl:49` still reads 1.0 everywhere. Feet and wall bases look the same on vs off. Commit says "AO live".

Off: "The pass runs and the shader compiles. LGTM."

On: "Reject. Item 5: `ao_tex` is stuck at 1.0, so the blit multiplies by a constant and nothing darkens; also check it darkens ambient only, not direct sun. Item 8: open sand must stay within 2% on vs off while creases darken. Evidence needed: on-vs-off captures with feet darker, plus the same depth giving the same AO twice."

## P2: probes-only GTAO (items 6, 7)

Situation: `crates/render/src/gtao.rs::GtaoAo` runs under `--look-probes` in `crates/game/src/bin/arena.rs:86` but live frames never dispatch it. Probe captures look good; the windowed match shows no AO and no live ms.

Off: "Probes prove the math. Ship it; live wiring is a follow-up."

On: "Not yet. Item 6: AO must run on live frames, not only under probes; live `ao_tex` staying constant is the flag. Item 7: give live ms against `GTAO_BUDGET_MS` 0.8 ms. Evidence needed: a live run with `ao_tex` varying plus ms at or under budget."

## P3: full-res AO over budget (items 2, 4, 7)

Situation: a diff runs GTAO at full res with a huge radius in `crates/render/src/gtao.rs`. The frame looks flat, ms doubles, and `GtaoConfig` limits in `GTAO_RADIUS_MAX_PX` and `GTAO_FALLOFF_MAX_M` are ignored.

Off: "More taps is better quality. The flat look is the art direction."

On: "Rework. Item 2: build at half res by `GTAO_HALF_DIV` and `GTAO_HALF_PASS` then upsample; full-res with no ms is the flag. Item 4: keep `radius_px` and `falloff_m` inside their max caps or the frame flattens. Item 7: show live ms at or under 0.8 ms. Evidence needed: half-res on-vs-off pixels plus ms before and after."
