---
name: coder-overlap-guard
description: "Use when a keeper batch is about to start a second coder packet and the guard would hold it for paths overlap with a running coder in engine2040."
---

# Give each coder its own crate

Two coder packets on one crate hold. The second waits in ready/ and the round loses a coder. Check before dispatch, then take a disjoint crate or wait.

Cause (target/loop-keeper.log 2026-10-11T12:03:55Z): `packet 0-lf2-port-ibl-bind held by the seat guard (paths overlap with running coder fail-ratchet-dup-render-repair: crates/render/src/shaders/forward.wgsl; give each coder...)` plus `cut-spot-torch-shadow held ... crates/render/src/twin_shadow.rs` behind the same running render repair. Same crate counts even for different files (.opencode/seat-guard.mjs:627-632 `crateOf`).

Fix: disjoint crates before start. Never retry a held packet unchanged.

## Check first (pwsh, engine2040)

- Read running coders: `Get-Content target/queue/batch.md -TotalCount 40` plus `Get-Content target/queue/claims.txt -Tail 20` for `builder-r* crates/<crate>/...` lines. A running coder on `crates/render/...` blocks every new `crates/render/...` packet.
- Same crate is overlap: `crates/render/src/shaders/forward.wgsl` overlaps `crates/render/src/decals.rs` via `crateOf`. `crates/physics/...` vs `crates/render/...` plus `tools/...` do not overlap.
- Held means wait: the packet stays in `ready/`. Take the next READY row in another crate (`node tools/board-slice.mjs --ready`) or wait for the running coder to land.
- Claim before writing: `Add-Content -Path target/queue/claims.txt -Value "<run> | <UTC yyyy-mm-ddTHH:MMZ> | <item> <exact files>"`. Never `Set-Content`.

## When held

Report `RESULT: BLOCKED - coder overlap <packet> held behind <running> <crate>` with the guard text quoted. Never force a second coder into the same crate.
