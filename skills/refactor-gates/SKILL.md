---
name: refactor-gates
description: "Use when planning a cross-crate refactor before any edit: draw blast radius, stage approved batches with per-batch CI and rollback, HALT until approval."
version: 0.1.0
author: skillworks
---

# Refactor gates

Kin: engine-proof repairs owned MAINT paths and audits read-only JAN findings, delegate-context-pack packs one dispatch, causal-triage-plan orders many live errors; this skill gates one planned cross-crate refactor — blast radius, staged batches, per-batch CI, rollback line — before any edit starts.

1. **Boundary map with exclusions.** List crates in play. List out of scope: generated files, vendor drops, lockfiles, local cargo config.
2. **Fixed weights.** Rank each item: security flaws, then breaking bugs, then architecture, then performance, then style. Style stays last.
3. **Blast radius per batch.** Each batch names its crates and files. No touched crate missing. Exclusions restated.
4. **Staged batches with CI and rollback.** Each batch carries one per-batch CI check and one rollback line. Land in approval order only.
5. **Diff preview before approval.** Show a short before/after sketch per batch. No approval without it.
6. **Split high risk.** One high-risk batch gets one gate. Never share one approval across two high-risk batches.
7. **HALT state.** The card ends in HALT until a human approves the next batch. Never work around a blocker; use the written fallback.

## Checks

1. Boundary map lists crates in play plus exclusions.
2. Every item ranked by fixed weights, style last.
3. Each batch names full blast radius, no touched crate missing.
4. Each batch has one per-batch CI check and one rollback line.
5. Each batch shows a diff preview before approval.
6. No approval covers two high-risk batches; card ends in HALT.

## Test note

Build one card per past cross-crate refactor from the record, then score blast radius, batches, and rank.
Adopt only on 3/3 mapped, rollback right in 2 or more, zero shared high-risk approvals.
Replay on the next real refactor before dispatch; hold adopt when a batch skips its gate.

## Spike (ADOPT 3/3, 2026-10-14, record only)

| refactor | blast radius + exclusions | staged batches (CI + rollback) | rank | rollback ok |
|---|---|---|---|---|
| 717d837c night land | net, perf, playtest, spatial; excl .cargo/*, Cargo.lock, target/ | b1 net quic (cargo test -p engine-net; revert quic.rs) b2 perf baselines (cargo test -p engine-perf; restore heap_peak.txt) b3 playtest+spatial (cargo test -p engine-playtest; revert tournament.rs) | breaking fix > arch > perf > style | yes |
| c7e6c665 night land | ai, arena, game; excl target/, Cargo.lock | b1 ai gates (cargo test -p engine-ai; revert behavior.rs) b2 arena rules (cargo test -p engine-arena; revert lib.rs) b3 game window (cargo test -p engine-game; revert windowed.rs) | breaking fix > arch > perf > style | yes |
| ddb423b6 night land | combat, endpoint; excl target/, Cargo.lock | b1 combat rows (cargo test -p engine-combat; revert classes.rs) b2 endpoint packs (cargo test -p engine-endpoint; revert packs/net.rs) | breaking fix > arch > style | yes |

Score: 3/3 mapped, rollback right 3/3, 0 shared high-risk approvals. Kill missed. ADOPT.

source: method ported in own words, zero donor bytes; donor VoltAgent/awesome-codex-subagents@16f880a categories/09-meta-orchestration/codebase-orchestrator.toml (MIT)
