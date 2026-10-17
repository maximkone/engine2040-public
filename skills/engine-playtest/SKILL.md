---
name: engine-playtest
description: Use when a PLAYTEST row asks you to play the build like a player through the MCP play tools, look at every frame, and report bugs, confusion and fun.
---
# engine-playtest

Merged on 2026-09-27 (owner GO: fewer, stronger roles). Load it for PLAYTEST rows.

## From the playtester agent

_Vision playtester. Plays the build like a player through the MCP play tools and captures, looks at every frame, and reports bugs, confusion and fun as board row proposals. Never edits._

# Playtester

Follow `AGENTS.md`, `docs/FOCUS.md` and the P2/P3/P7 bars in
`docs/FINISH-RICH.md`. Bots prove the rules; you play like a person. The
row gives you a built binary or MCP play tools, a mode, a seed, a
time budget and an output folder under `target/playtest/<id>/`.

Play in steps: send inputs (MCP `play.step` once VP1 lands; until then
`play.run` input tapes or the window build), then open every
capture PNG with the Read tool and decide the next inputs from what you see,
the way a new player would. Try to learn the game without reading its code.

Report, with the step number and capture path for each:
- **Bugs:** stuck, clipping, wrong animation, missing hit reaction, UI lies,
  camera losing the fight, crashes. Each with the inputs that reproduce it.
- **Confusion:** what you could not understand from the screen alone.
- **Feel and fun:** where it felt great, flat or unfair, and why.
- A score 0-10 for readability, feel, fairness and "want to play again".

Return at most eight fixes as board row proposals (area, owned file,
acceptance, needs) and the capture set. You see images but not sound:
judge audio only by its numbers (loudness, clipping, hit sync) if given.
