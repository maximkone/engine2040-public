# Spatial reasoning for MCP: report and verdict (2026-10-18)

Read this first. Details: 01-ours.md (our audit), 02-forge.md (C:\forge), 03-flax.md (flax-mcp, flaxmcp-plugin).
34 board rows were added to docs/SPRINT-07-accepted-plan-rows.md: SPR1-10 (ours), SPF1-12 (Forge), SPX1-12 (Flax).

## Verdict: improve, do not train yet

- The geometry core is good (crates/spatial, about 27k lines, integer cm, deterministic, gold tests pass).
- What an AI agent gets through the door is weak: every scene node is a 1 m cube called prop_NNNN, no positions or sizes in the brief, reachable is always true (every actor sits in nav polygon 0), full object tables cost about 50-90k tokens per call at 2,000 objects, placement failures say "failed" without naming the blocker.
- About 3k lines are finished and have no caller (sense_text, isovist, exposure, kdtree, tag_lift, scale, fps_visibility). Landing them costs less than writing anything new.
- No number anywhere comes from a real model. The harness gold agent echoes gold; the 14/50 -> 50/50 figures are node simulations. Forge's own measure: a free model passed 3 of 8 scene tasks at 12.9k-47k tokens each, and failures pointed at missing tools, not at weak reasoning.
- Training does not pay yet. The gaps are labels, schema text, error messages and missing algorithms. A learned part fits only fuzzy name grounding (SP33, SP67), and only if recall on real asset names stays under 90 percent. Measure first with SPR4 (50 seeded questions on cassettes).

## Order to land (value per cost)

1. Make answers usable: SPR1 (handle and place for every object), SPR3 (self-describing scene.query), SPR8 (compact answers), SPX8/SPF10 (brief hygiene and robust extent), SPR6 (say why a placement failed).
2. Make answers true: SPR2 (check left/right, which may be inverted: the door returns objects that have X on their left; the test only compares with itself), AD20 (real reachability), SPF5 (giant object relative to the layout), SPX2 and SPF8 (stop false red findings), SPF1 (known-good scenes as a gate with negative controls).
3. Add the verbs agents ask for: SPR7+SPX6 (sees and why hidden), SPF4+SPX11 (clearance and measure), SPF6+SPX5 (slide and travel), SPF3 (facing-relative wants), SPR5 (dry-run and footprint-aware placement), SPR9 (rooms), SPR10+SPF2+SPF11 (camera frame fit and coverage), SPX12 (egress).
4. Teach weak models: SPX4 (scene.next names the one next call), SPX3 (spatial findings returned on the write), SPF12 (read-repeat guard).
5. Prove it: SPR4 (real-model A/B on cassettes), SPF7 (collateral and budget bars on the AD25 tasks).

## Lanes (rows that touch scene_query.rs and scene_tools.rs collide: one agent at a time)

- Lane A, endpoint door: SPR1, SPR3, SPR8, SPR6, SPX3, SPX4, SPF12, SPX9.
- Lane B, crates/spatial algorithms: SPR2, SPR7+SPX6, SPF4+SPX11, SPF6+SPX5, SPF3, SPR9, SPX2, SPF8, SPF5.
- Lane C, eval and content: SPF1, SPR4, SPF7, AD20, SPF2+SPR10+SPF11.

## Caveats

- The audit read code and board rows only; no live door call, no model call.
- Forge numbers are one run, one model, one scene. Neither Forge nor Flax records a token figure for a spatial tool; costs in 03-flax.md are schema caps.
- The `left_of` inversion is derived by hand and may be a misread of the intended convention. SPR2 settles it.
- Ideas skipped on purpose: Forge's 33-mode view math, its 17-viewpoint scorer (no occlusion), scene-SQL, contrastive bench pairs; Flax's terrain_raycast and query_actors_in_radius (listed in the README, never implemented), and the retired measure_* tools.
