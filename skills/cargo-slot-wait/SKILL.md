---
name: cargo-slot-wait
description: "Use when a cargo build waits for busy slots or the file lock in engine2040: wait, do non-cargo work, or end PARTIAL instead of retrying."
---

# Wait for a cargo slot, do not retry

Slots full means wait. Retrying the same cargo line waits again. Second identical failure ends the step.

Cause (target/loop-keeper.log, 25 hits in last 300 lines 2026-10-11): `cargo wait: 3 of 3 slots busy (...)` (11x) plus `cargo slot after 20 s` (14x). Full wait ends with `cargo slots full: 3 of 3 busy for X min. Do non-cargo work now and run this command later, or end RESULT: PARTIAL - waiting for a cargo slot.` Sibling: `Blocking waiting for file lock` is another build holding `target/`, not a hang; wait, keep keeper timeout (AGENTS.md).

Fix: wait once, do non-cargo work, then run once. Never loop the same cargo line.

## Wait first (pwsh, engine2040)

- See `cargo wait: 3 of 3 slots busy`: stop, wait. Sleep once: `Start-Sleep -Seconds 60`, then read `target/queue/batch.md` for running builds. Never retry in a tight loop.
- Do non-cargo work while waiting: `Get-Content target/queue/claims.txt -Tail 20`, `node tools/board-slice.mjs --ready`, read `docs/INDEX.md` or owned `crates/core/src/lib.rs`. Proof: notes plus no new wait line.
- Coder or judge with owned crate paths only: after the wait, run one crate once: `cargo check -p engine-core`, then `cargo test -p engine-core --lib <filter>`; example `cargo test -p engine-spatial --lib place_on_top`. Never workspace-wide builds. Proof: scoped test output.
- Write, run and research seats never build: file a packet under `target/queue/ready/` naming `crates/render/src/sky.rs` and its test. Proof: ready packet plus queue-cap check.
- `Blocking waiting for file lock` means wait too: another build holds `target/`. Wait, keep keeper timeout. Never clean, stash, or delete `target/` to force it.
- Full wait means stop: report `RESULT: PARTIAL - waiting for a cargo slot` with the guard text quoted. Never retry unchanged.

## When held

Quote the wait line in the report. Take non-cargo work, one scoped run, or nothing. Never touch `C:/empire/center/skills-shared`.
