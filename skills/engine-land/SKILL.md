---
name: engine-land
description: Use when a land was REFUSED (see `land:` lines in target/loop-keeper.log) - how the lead lands one judged row by hand on master from the one tree (gate, tests, golden hash, ratchet, tick, receipt, commit, push, release claim). A review PASS normally lands itself.
---

# Engine land (from merger)

The keeper lands a review PASS itself (`node tools/land.mjs <name>`). Only for a REFUSED land,
the lead lands the row by hand, one at a time, in this order. Root `AGENTS.md`
always wins (stage only owned paths, build green, identical hash across two runs,
`node tools/rust-quality.mjs` PASS). Shell is pwsh; one command per call.

One tree: `C:\engine2040` on master (lanes retired, owner 2026-10-10). The builder's work is
already there as uncommitted edits to its OWNED paths. The judge's record in
`target/queue/done/` names the builder's `ROW`, `OWNED` and `CRATE` line and carries
`VERDICT: PASS`. Never land a row without that PASS. Stop at the first red step.

1. `git pull --no-rebase --no-edit origin master`.
2. `git status --short -- <owned paths>` (read each `??` new file), then
   `git diff -- <owned paths> | node tools/packet-gate.mjs --patch - --owned <path>` (one `--owned` per path).
   FAIL: stop, block the row.
3. The crate's tests: `cargo test -p <crate>`, the row's acceptance commands, the golden hash
   `d57b5c8ae3096614` twice when the tick is touched, `node tools/rust-quality.mjs` PASS.
   Red: block the row with the red command and its first error line; the builder fixes in
   place next wave. Abandoned work: save `git diff -- <owned paths>` to
   `target/abandoned/<row>-<date>.patch`, then `git restore --source=HEAD -- <owned paths>`
   (only the lead reverts).
4. Before the tick, run `node tools/drift-check.mjs`. Tick the row in `docs/SPRINT.md` (never untick another) and add its receipt under
   Receipts: commit, test counts, what stays unverified. Lock the row's gains with
   `node tools/rust-quality.mjs --tighten` (it only lowers improved counts). After any board edit, run `node tools/workspace-check.mjs`.
5. Stage only the owned paths plus `docs/SPRINT.md` and `tools/rust-quality.baseline.json`
   (`git add -- <paths>`); commit `<ID>: <outcome> (<tests>)`;
   pull again; `git push origin HEAD:master`. A refused push: pull once more, then push.
6. Release the claim: append (Add-Content, never rewrite) `# released <UTC yyyy-mm-ddTHH:MMZ>: <ID>` to `target/queue/claims.txt`.

Cargo needs: `Cargo.toml` / `Cargo.lock` edits the builder reports are yours to make.
You still cannot edit Rust; a red build is a builder one-off.
