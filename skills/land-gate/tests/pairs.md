# land-gate pairs: with vs without the skill (3 cases)

## Case 1: patch outside owned paths

Without: `git diff -- skills/SKILL.md` piped with `--owned crates/core/src/time.rs` is skipped; the run goes straight to `node tools/land.mjs <packet>` and risks landing a foreign file.
With: gate runs first and prints `RESULT FAIL: 1 changed path(s), 2 finding(s)` with `FAIL skills/SKILLS.md: outside owned paths`, exit 1. Landing stops. Proof: gate FAIL receipt above.
Winner: with the skill.

## Case 2: DONE text with no RESULT proof

Without: the keeper line `DONE - checked wall quantize in time.rs` in `target/queue/done/0-cq10-time-checked-20261014.md` is read as landable and sent to land.
With: `node tools/packet-gate.mjs --result target/queue/done/0-cq10-time-checked-20261014.md` prints `FAIL no RESULT line` and `RESULT FAIL: 1 finding(s)`, exit 1; then `node tools/land.mjs 0-cq10-time-checked-20261014 --dry` prints `REFUSED 0-cq10-time-checked-20261014: the run ended with no RESULT line, not DONE, PARTIAL or NOOP`, exit 1. Landing stops. Proof: audit FAIL plus land REFUSED receipts above.
Winner: with the skill.

## Case 3: non-landing role sent to land

Without: a runner record is landed directly; the commit attempt wastes a cycle before the refusal.
With: `node tools/land.mjs 0-arena-shake-watch-20261014 --dry` prints `REFUSED 0-arena-shake-watch-20261014: role runner lands nothing`, exit 1; `node tools/land.mjs 0-dp1-loopback-knobs --dry` prints `REFUSED 0-dp1-loopback-knobs: no review record for 0-dp1-loopback-knobs in done/`, exit 1. Nothing is committed. Proof: two REFUSED dry-run receipts above.
Winner: with the skill.

Score: with the skill wins 3 of 3.
