---
name: flow-shell-checks
description: "Use when a builder adds a title pause result or rematch shell around the arena match and a reviewer checks flow states and tick accounting."
version: 0.1.0
author: skillworks
---

# Flow shell checks

Eight checks for the window shell around one match. Name the crate per finding (game, ui). PASS needs a test plus the unchanged golden hash, never a promise.

1. **One driver.** Ask: does one Flow in crate game own Title Match Paused Result and drive menus?
   Flag: state split across windowed and bin with no single owner; two places advance the match.
2. **Objectives as graph.** Ask: are objectives a sequence tree or graph in crate game, not one per chunk?
   Flag: level id equals objective id so a chunk rename rewrites progress and saves.
3. **Success and failure.** Ask: does each objective state its success and its failure move in crate game?
   Flag: a win with no next step or a loss that strands the player with no rematch.
4. **Loose chunks.** Ask: can chunks and objectives change apart in crate game with maps by id?
   Flag: a chunk edit forces a flow edit; one table holds mesh refs and win rules.
5. **Pause stops ticks.** Ask: does pause halt the accumulator in crate game so 60 paused frames add zero ticks?
   Flag: pause hides the window while ticks keep stepping; the replay hash drifts.
6. **Result rematches.** Ask: does Result lead to the next match with prior state cleared in crate game?
   Flag: old scores or timers leak into the next bout; no fresh-seed path exists.
7. **No settings creep.** Ask: does this shell add only flow in crate game and leave settings to ui rows?
   Flag: a settings screen smuggled into the flow diff; parked ui work returns by stealth.
8. **Hash proof.** Ask: is there a flow_cycle test in crate game plus golden x2 unchanged?
   Flag: manual clicks with no test; a pause-in-match hash is never compared.

## Do / Don't
- Do: pause the accumulator at the frame edge and keep the tick count still. Don't: keep stepping hidden ticks and call it a pause.

source: Gregory, Game Engine Architecture (2nd ed.), ch. 14.1.3