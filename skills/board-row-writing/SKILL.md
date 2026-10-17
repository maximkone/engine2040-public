---
name: board-row-writing
description: "Use when you add or edit a board row in docs/SPRINT-*.md or a plan's section 7: the one-line row format with Diff, Tier, Size, Owns, Accept, F2P, P2P and needs, so tools/board-slice.mjs and workspace-check accept it."
---

# Board row writing

The board is `docs/SPRINT.md` (index) plus shards `docs/SPRINT-NN-*.md`. A row's spec is its plan in `docs/plan/` (`node tools/plan-atlas.mjs --row <ID>`). Read rows by slice, never whole: `node tools/board-slice.mjs <ID>`, `--ready`, `--focus`, `--find <text>`, `--next`.

## The shape (one bullet per row)
```
- [ ] RZ8 physics: a material table for friction, bounce, density and sound — what and why in 1-3 sentences with the real file names. Diff: regular. Tier A. Size M. Owns `crates/physics/src/material.rs`, the audio cue hunk. Accept: `node tools/gh-check.mjs cargo test -p engine-physics --lib material` prints N passed; the number or file a second reader can check. F2P: what fails today. P2P: what must keep passing (golden `d57b5c8ae3096614`). needs: RZ1, WEB2.
```
- Head must match `- [ ] ID area: title` (or `[x]`). ID is capitals plus a digit, optional trailing letter (`WEB4`, `RZ5`, `GP34b`). IDs are unique across all shards and `docs/archive/SPRINT-done-*.md`: number after the highest in your prefix (prefix table in `docs/plan/README.md`).
- `area` is one short word: the Areas table in `docs/SPRINT.md` lists the usual ones (render, physics, game, assets, net, proof, ...).
- Fields, in this order: `Diff:` regular | good | hard (routing: regular = any coder, good = Sonnet class, hard = Opus class: GPU, concurrency, determinism, subtle math). `Tier:` A alpha, B v1, C VISION bar. `Size:` S, M, L. `Owns` = the exact paths the row may edit, in backticks. `Accept:`, `F2P:`, `P2P:`, `needs:`.
- Size: S is one agent session (up to about 150 changed lines in SPRINT-22), M two to four (up to about 400), L five to ten. Bigger than L: split. For L list the landings in parentheses: `Size L (landings: world and caps; ragdoll seeding; debris).`
- A row is one long line (SPRINT-22 style) or a head line plus continuation lines indented two spaces. A line with no indent ends the row.

## Accept that a second reader can run
- Name the command and the number: `cargo test -p engine-game --features app --lib ragdoll_look` plus what it must show (settles within 3 s, 6 cuts stay under the cap), or a printed line, or a file that must exist. Prefer `node tools/gh-check.mjs cargo test ...` (skill `github-check`).
- A visual claim needs a capture you opened; a perf claim needs a measured number and where (runner or "(reference GPU)" for the RTX 3050).
- Always include the golden: `golden unchanged` or `golden d57b5c8ae3096614 x2`.
- `node tools/plan-atlas.mjs --plan <name>` lists an Accept that has no number.

## F2P and P2P (every NEW row needs both, or it is not ready)
- F2P = fails before the change, passes after. Name the test or the missing thing: `F2P: no wasm row in the matrix today.`
- P2P = passes before and still passes after: `P2P: the three native rows still match.`

## needs
- Last in the row, ends at the FIRST period: `needs: WEB2, WEB5.` or `needs: none.` Text after the period is not parsed, so notes go before `needs:` or after the period.
- Every id must exist on the board (or in the same plan). A ticked row cannot need an open row (workspace-check fails it).
- A row is ready when open, every need ticked, and its text has no `BLOCKED:` or `LATER:`. `LATER:` parks it; deleting that word unparks it.

## Landing and ticking
- After the Accept passed and the commit exists: `- [x] ID ... Landed <sha7>.` (skill `commit-by-path`). Partial: keep `[ ]` and add `Landed so far (<date>): <sha7> (<what>). Remaining: ...`.
- A second identical failure ends the row: add `BLOCKED:` and one row to `docs/BLOCKERS.md`; never mark it green.

## Check
- `node tools/workspace-check.mjs` (ids unique, needs resolve) and `node tools/board-slice.mjs <ID>` (the row prints, under 8 KB).
- New plan file: one line in `docs/plan/README.md` in the same commit (`node --test tools/plan-index.test.mjs`).

## Pitfalls
- A period inside the `needs:` list (`needs: RZ1. Also EG45`) hides the rest; use commas.
- Reusing an id that lives only in an archive file: it counts as taken.
- Writing a row for work that exists: run `git grep` for the function first; a ticked row or an existing named function means NOOP.
- Rows that name a file in backticks make that path "owned" for slicing; do not backtick paths you only mention.
