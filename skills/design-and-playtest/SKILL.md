---
name: design-and-playtest
description: "Use when you design game data (moves, encounters, AI archetypes, modes, map flow), judge fight feel or fun, play the build through the MCP play tools, or score captures against a look reference: design as data, open every frame."
---

# Design, playtest and look review

You assess the game, not the code. In a review you do not edit code. You return a score, a pass or fail against the bar, and ranked fixes.

Read first: `AGENTS.md`, `docs/FOCUS.md` (its "EMPIRE" section), the P1/P2/P3/P7 bars in `docs/FINISH-RICH.md`, and `docs/VISUAL-TARGET.md` for look.

## Eyes rule (one rule for every section below)

- Open every capture PNG with the Read tool. Open its reference next to it for look work.
- Write one line on what is on screen: the region, the object, what is wrong or right ("left wall, shadow acne").
- Score from the pixels, never from logs or numbers alone.
- No capture means unverified. Say "unverified" and stop. Never score a shot you did not open.
- You see images, not sound. Rate audio only by numbers if they are given (loudness, clipping, hit sync).
- Read hit frames in order: anticipation, impact, hit-stop, reaction. Check silhouettes, UI and camera at each one.

## 1. Design (a DESIGN row)

You own data files named by the row: move tables (format: `docs/design/FORMAT.md`, `content/design/combat.rows`), encounter and AI archetype specs, modes, progression, map flow, and the `docs/FOCUS.md` sections the row names. Design as data the engine already loads. If the engine cannot express the design, return the missing capability as a board row (skill `commit-and-board`). Do not hack around it.

- Every design has a player goal, a counter for every strong option, a difficulty curve and a reason to play again.
- Rome at war: grounded, heavy, readable at a glance.
- AI game builders fail most on content and coherent design, not on code (GameCraft-Bench, arXiv 2606.17861).
- Prove it. Validate the data (combat rows: skill `anim-combat` section 3). Run the PQ2 tournament or fork matches on it. Note a seed for the playtest.
- Study how great games solve the same problem (GDC talks, design breakdowns) with websearch. Take ideas, never content.
- Report: changed data, validation and tournament numbers, what a playtest should check, open design questions.

## 2. Feel and fight review (FEEL row)

Run or read the named playtest tapes, tournament reports (PQ2), latency numbers (PQ1), move tables and captures. Check:

- Every hit has hit-stop, reaction, sound and camera trauma.
- Inputs feel immediate.
- No dominant move.
- The AI reads as human at each difficulty.
- The pacing director creates tension and relief.
- Traversal flows into combat.

Compare with the best third-person melee games through public talks and reviews (websearch), never their code.

Bars to score against (`docs/FINISH-RICH.md`):

| Bar | Numbers |
|---|---|
| P2 feel | input to first effect p95 <= 50 ms; foot slide < 1 cm from 0.5 to 7 m/s; no pops on interruption; every hit shows hit-stop, reaction, sound, camera trauma in the same tick window |
| P3 fight and AI | AI win rate 40-60% per difficulty over 200 fork matches; no single move above 35% of kills; zero stuck fighters; parry, riposte and clash reachable |

Return a score per bar item, pass or fail, and at most five ranked fixes. Each fix: area, owned file, acceptance, and the tape or capture that shows it. Feel proof checks for the code diff live in skill `anim-combat` section 4.

## 3. Playtest (a PLAYTEST row)

Bots prove the rules. You play like a person. The row gives you a built binary or the MCP play tools, a mode, a seed, a time budget and an output folder under `target/playtest/<id>/`.

1. Send inputs. Use MCP `play.step` (a bout held across calls) or `play.run` (a bounded headless match job; pass `scene_rev` from the scene you built or it is refused as `unpinned_run`, `empty_arena` or `stale_scene_rev`). Or use the window build. Run lines: skill `door-and-headless-runbook`.
2. Open every capture PNG (Eyes rule). Decide the next inputs from what you see, the way a new player would.
3. Learn the game from the screen. Do not read its code.

Report, with the step number and capture path for each:

- **Bugs:** stuck, clipping, wrong animation, missing hit reaction, UI lies, camera losing the fight, crashes. Each with the inputs that reproduce it.
- **Confusion:** what you could not understand from the screen alone.
- **Feel and fun:** where it felt great, flat or unfair, and why.
- **Scores 0-10:** readability, feel, fairness, "want to play again".

Return at most eight fixes as board row proposals (area, owned file, acceptance, needs) and the capture set. Bug report fields: skill `debug-triage`.

## 4. Look review and ART rows

- **ART row:** material layers, light rigs, sky and exposure presets, post and grading tables, quality-tier presets, as data or small tuning code in the owned files. Use physically plausible values (albedo ranges, EV100, light units). One preset home per concern. No one-off screenshot tricks.
- **Look review (P1 bar):** score captures against the reference set and the path-tracer reference: silhouette, material response, lighting, shadows, temporal stability, composition. Return a score per shot, pass or fail, and the three highest-value fixes with the file or preset that owns each.
- **Cost rule:** each shot names its per-pass GPU cost against the budget in `docs/VISUAL-TARGET.md`. A prettier shot that blows the frame budget fails.
- **FLIP:** a FLIP score against the reference is wanted once row PQ4 (reference comparison) lands. No FLIP tool was found in `tools/` or `crates/render` on 2026-10-17, so do not quote a FLIP number.
- Research the reference (talks, art breakdowns, CC0 references) with websearch. Steal structure, never text.
- Light and sky checks per feature: skill `render-lighting`. Capture file rules: skill `test-and-proof`.

Sources: GameCraft-Bench (arXiv 2606.17861); `docs/FINISH-RICH.md` bars P1, P2, P3, P7.
