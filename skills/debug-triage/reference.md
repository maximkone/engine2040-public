# Debug triage — reference

Moved out of `SKILL.md`: the bug report fields.

## Writing a bug report

A `docs/BLOCKERS.md` row or a bug row on the board carries these fields. Write facts, not opinions or fixes.

| field | rule |
|---|---|
| Title | one line: platform, area, symptom. Not "broken". |
| Row | the open board row id, if one exists |
| Failing command | the exact command with crate and filter. Not "run tests". |
| Build | build hash, date, OS and device. Not "latest". |
| Expected / actual | one line each |
| Steps | numbered, one action each, with preconditions |
| Proof | a capture or log plus repro rate x/5 (not 1/1) |
| Severity | by player impact. Leave priority to the owner. |

Do not write "must fix" in the report.
