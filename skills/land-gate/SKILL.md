---
name: land-gate
description: "Use when landing an engine2040 packet through tools/land.mjs and the run must gate the patch before any commit and refuse on FAIL"
---

# Land gate

Gate before land. Refuse on FAIL. Pwsh only. One command per call.

1. **Gate the patch first.** `git diff -- <owned> | node tools/packet-gate.mjs --patch - --owned <owned>` must print `RESULT PASS`. A FAIL stops the landing. Quote the FAIL line.
2. **Audit the reply.** `node tools/packet-gate.mjs --result <record>` must print `RESULT PASS`. A DONE needs a command or file proof and no FAIL text. `no RESULT line` stops the landing.
3. **Dry-run the landing.** `node tools/land.mjs <packet> --dry` prints `DRY <packet>: would commit` or `REFUSED <packet>: <why>`. A REFUSED stops the landing. Quote the why.
4. **Land only after PASS plus DRY.** Real `node tools/land.mjs <packet>` runs only when steps 1-3 all pass. Never land on a FAIL, a missing review, or a non-landing role.
5. **Prove the order.** Report the three lines in order: gate RESULT, audit RESULT, land DRY or REFUSED. A report with only the land line is unverified.

## Test note

- `git diff -- skills/SKILLS.md | node tools/packet-gate.mjs --patch - --owned skills/SKILLS.md` prints `RESULT PASS`.
- Same diff with `--owned crates/core/src/time.rs` prints `RESULT FAIL` with `outside owned paths`.
- `node tools/packet-gate.mjs --result target/queue/done/builder-fixer-r161.md` prints `RESULT FAIL` with `no RESULT line`.
- `node tools/land.mjs builder-fixer-r161 --dry` prints `REFUSED` with `no RESULT line`.
- `node tools/land.mjs runner-proof-r304 --dry` prints `REFUSED` with `role runner lands nothing`.
- `node tools/land.mjs builder-integrator-r72 --dry` prints `REFUSED` with `no review record for builder-integrator-r72 in done/`.

## Hands off

Depth and evidence stay in `engine-review`. Rust scoring stays in `engine-rust-quality`. Red-to-green loops stay in `gate-red-fix`. This skill owns only the gate-audit-dry order.
