---
name: causal-triage-plan
description: "Use when one wave drops three or more linked errors: draw the causal map and the delegation plan before any fix starts."
version: 0.1.0
author: skillworks
---

# Causal triage plan

Kin: delegate-context-pack packs one dispatch, crash-stack-triage maps one stack, systematic-debug gates one bug; this skill orders many errors by cause then delegates before any fix.

1. **Cluster by boundary.** Group errors by time, subsystem, recent-change surface. One cluster per cause, never per message text.
2. **Split primary vs cascade.** A primary explains the rest. A cascade clears when its primary lands. Cite evidence past same text: file line, log, rebuild.
3. **Order by impact x info gain.** Fix what blocks most first. Break ties by what one fast test answers first.
4. **Delegate with zero overlap.** One cluster gets one owner with disjoint paths. Cascades stay held until the primary lands.
5. **Attach one disproof test.** Each unsure cluster carries one fast test that kills it when wrong. Mark guesses inferred, never confirmed, unless the parent asks.
6. **Close with one narrative.** Return the order fixed, proofs rerun, owners collision-free, unknowns closed.

## Checks

1. Every error sits in exactly one cluster by time, subsystem, change surface.
2. Each cluster is marked primary or cascade with evidence beyond message text.
3. Order is primary-first by impact x info gain.
4. Each cluster has one owner with disjoint paths and zero overlap.
5. Each unsure cluster carries one fast disproof test.
6. Card ends with one narrative plus reintegration checklist; inferred never confirmed.

## Test note

Build three cards from past waves, then score primary-first order and overlap.
Adopt only on 3/3 mapped, primary-first in 2 or more, zero overlapping owners.
Replay on the next real wave before dispatch; hold adopt when order breaks.

## Spike (ADOPT 3/3, 2026-10-14, record only)

| wave | primary | cascades | evidence | disproof test | delegation | primary-first |
|---|---|---|---|---|---|---|
| VS1-20261013 | E0432 missing kit_asset_files at arena_kit.rs:29 | game lib red, staged plus arena rebuild FAILED, mcp packs BLOCKED 0/0, GP34 released | file line plus circular import and call plus rebuild log plus proof log | define fn in one home, drop circular call, cargo check -p engine-game | one repair packet owns both files | yes |
| RENDER-LINK-20261013 | render lib-link linker break | aa-scale-fixture HOLD, submit-camera HOLD, submit-sort HOLD | same red crate, render smoke blocked | rebuild render smoke after link fix | link to render owner, rest held | yes |
| EG13-MSAA-20261013 | msaa-invalidate setup PARTIAL, window build broke twice | aa-shadow-shimmer HOLD, shadow-settle HOLD | same pixels as slice-3 | land slice-3 then recapture same pixels | slice to render owner, rest held | yes |

Score: 3/3 mapped, primary-first 3/3, 0 overlapping owners. Kill missed. ADOPT.
Records: vs1-escalation-20261013.md, proof-mcp-packs-20261013.md, run-arena-20261013T1632Z.md, held-triage-20261013.md, wave-score-5-20261013.md.

source: method ported in own words, zero donor bytes; donor VoltAgent/awesome-codex-subagents@16f880a categories/09-meta-orchestration/error-coordinator.toml (MIT)
