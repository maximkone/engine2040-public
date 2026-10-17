# land-gate pairs: with vs without the skill (3 cases)

## Case 1: patch outside owned paths

Without: `git diff -- skills/SKILL.md` piped with `--owned crates/core/src/time.rs` is skipped; the run goes straight to `node tools/land.mjs <packet>` and risks landing a foreign file.
With: gate runs first and prints `RESULT FAIL: 1 changed path(s), 2 finding(s)` with `FAIL skills/SKILLS.md: outside owned paths`, exit 1. Landing stops. Proof: gate FAIL receipt above.
Winner: with the skill.

## Case 2: DONE text with no RESULT proof

Without: the reply in `target/queue/done/builder-fixer-r161.md` carries no `RESULT:` line, but it is read as landable and sent to land.
With: `node tools/packet-gate.mjs --result target/queue/done/builder-fixer-r161.md` prints `FAIL no RESULT line` and `RESULT FAIL: 1 finding(s)`, exit 1; then `node tools/land.mjs builder-fixer-r161 --dry` prints `REFUSED builder-fixer-r161: the run ended with no RESULT line, not DONE, PARTIAL or NOOP`, exit 1. Landing stops. Proof: audit FAIL plus land REFUSED receipts above.
Winner: with the skill.

## Case 3: non-landing role sent to land

Without: a runner record is landed directly; the commit attempt wastes a cycle before the refusal.
With: `node tools/land.mjs runner-proof-r304 --dry` prints `REFUSED runner-proof-r304: role runner lands nothing`, exit 1; `node tools/land.mjs builder-integrator-r72 --dry` prints `REFUSED builder-integrator-r72: no review record for builder-integrator-r72 in done/`, exit 1. Nothing is committed. Proof: two REFUSED dry-run receipts above.
Winner: with the skill.

Score: with the skill wins 3 of 3.
