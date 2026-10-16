# pairs.md — claim-paths-guard on-off test (3 cases)

Each case shows the WITHOUT answer (no skill) and the WITH answer
(skill applied). Winner named per case.

## Case 1: sibling skill path (miner-books-r15 pattern)

Situation: claim allows `skills/msaa-resolve-checks/SKILL.md` plus its
`tests/pairs.md`. Task wants `skills/sky-bake-checks/tests`.

- WITHOUT: Write `skills/sky-bake-checks/tests` at once. Guard:
  `blocked bash: out of packet paths: skills/sky-bake-checks/tests;
  allowed: skills/msaa-resolve-checks/SKILL.md, ...`. Retry same path:
  blocked again, step ends.
- WITH: `Get-Content target/queue/claims.txt -Tail 20` first, see the
  allow-list names only msaa paths, stop. Either claim sky-bake under
  own run with `Add-Content` then write, or park the item. No block.
- Winner: WITH (no guard hit, one claim, then a clean write).

## Case 2: sibling test file (smith-tools-r14 pattern)

Situation: claim allows `tools/q1-check.test.mjs`,
`tools/bins-quiet.test.mjs`, `tools/bins.test.mjs`. Task wants
`tools/quiet.test.mjs`.

- WITHOUT: Write `tools/quiet.test.mjs` directly. Guard:
  `blocked write: out of packet paths: tools/quiet.test.mjs; allowed:
  tools/q1-check.test.mjs, tools/bins-quiet.test.mjs, tools/bins.test.mjs`.
  Work stalls on a path never owned.
- WITH: read claims tail, confirm quiet is not allowed, append
  `Add-Content -Path target/queue/claims.txt -Value "<run> | <UTC> |
  quiet-no-test tools/quiet.test.mjs"`, then write. Guard passes.
- Winner: WITH (claim first, write lands).

## Case 3: wholly unclaimed path (miner-books-r15 pattern)

Situation: no claim line and no `paths:` line own
`skills/spatial-support-checks/SKILL.md`.

- WITHOUT: Write at once. Guard: `blocked write: no packet paths:
  skills/spatial-support-checks/SKILL.md is not claimed`. Retry
  unchanged: second identical failure ends the step, nothing written.
- WITH: stop at the first block text, quote it, append the claim for
  the exact file, retry once under the new claim. Write lands. Report
  `RESULT: DONE` with the claim line as proof.
- Winner: WITH (one diagnosed retry, then green).

Score: WITH wins 3 of 3. Install.
