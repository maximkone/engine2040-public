# Parallel campaign 2026-10-20 — what three agent lanes can land

Status: prepared, not started. Owner decision: board shape and whether `LATER` rows stay frozen.

## The target is 309, not 695

`node tools/board-slice.mjs --ready --open` and the strict `rowReady` rule (open,
needs ticked, no `BLOCKED:`/`LATER:`, not an operator row) agree with
`docs/plan/2026-10-20-progress.md`: about 700 open rows, 309 startable now. The
rest are not landable by an agent today:

- 217 wait on another open row (chains: land the front row first).
- 154 carry `LATER:` (owner-parked; FOCUS says do not take them).
- 13 carry `BLOCKED:`; 1 is an operator row; 1 needs an id that is not on the board.

Before counting the 309, close or rewrite the rows that name work in the retired
center: GN4, GN5, GN7, GN8 all own or run `kaggle.mjs`, which the standalone-repo
decision removed on 2026-10-17. `docs/plan/2026-10-17-obsolete-rows.md` already
holds the same verdict for E4.

## Speed: GitHub yes, Kaggle no

- **GitHub** is the proof fleet: `node tools/gh-check.mjs cargo test ...` runs one
  plain cargo line on a runner against the working tree, up to 20 at once. It
  removes the local target lock from heavy tests and clippy. It does not make
  edits, reading or review faster, and every check snapshots the whole dirty
  tree, so another session's WIP is inside it.
- **Kaggle** is not a resource in this repo. The center pipeline is gone, there
  is no `kaggle.mjs`, and the Linux runners have no RTX 3050 or window for the
  GPU/window rows. Do not plan around it.

## Lanes

First owned crate decides the lane, so a row belongs to one lane only. Counts are
the 309 ready rows on 2026-10-20.

| Lane | Ready | Work |
|---|---|---|
| T triage | 98 | no crate in `Owns`: old E shard rows, focus rows with shader/doc paths, rows that need an `Owns` line before an agent can be held to it |
| R render | 31 | `crates/render`; 6 of the 12 focus rows are here, so this lane is the long pole |
| G game | 24 | `crates/game`; includes FC1, LF14, RZ4 |
| E endpoint | 22 | `crates/endpoint`, `spatial`, door verbs |
| A assets | 16 | `crates/assets`, content intake, WEB6 |
| N net | 12 | `crates/net`, rollback and browser policy |
| M anim+combat | 18 | `crates/anim`, `crates/combat` |
| P physics+proof | 13 | `crates/physics`, `crates/physfx`, `crates/proof` |
| rest | 75 | core, world, ui, ai, observe, audio, material, perf, editor, playtest, provider, nav, web, geo |

## Waves

1. **W0 now, clean tree:** E, A, N (48 rows whose owned crates have no dirty
   files from the other session). Start three lanes, five rows each, then judge.
   Leave GN7 out: it runs the retired `kaggle.mjs` and waits on the obsolete-row
   decision.
2. **W1 focus, once `crates/render` and `crates/game` are clean:** R takes FC1 LF2
   EG12 GS43 GS44 GS45; G takes LF14 AM24 RZ4; E/A take OPT27 WEB6. Twelve rows,
   the current FOCUS path.
3. **W2:** R 31, G 24, M 18, P 13. T runs beside them and removes rows instead of
   landing them: close obsolete, add missing `Owns`, park what needs a person.
4. **W3:** the rest, still three lanes.

Never run all lanes at once. One tree, no worktrees, one `target/` lock and one
`Cargo.lock`. The 2026-10-19 session ran three loops and about ten cargo
processes; timing rows had to wait for a quiet window. Three lanes plus the
GitHub proof fleet is the useful ceiling here.

## Per-row protocol

- Read `AGENTS.md` and the matching skill first; take one row's slice with
  `node tools/board-slice.mjs <ID>`.
- Edit only the paths in the row's `Owns`; never touch another session's dirty
  file, never `git add -A`, never a worktree.
- Prove locally with `cargo check -p <crate>`; send tests, clippy and multi-crate
  proofs to `node tools/gh-check.mjs`. A red in a file you do not own is its
  owner's; re-run when the tree clears, do not fix it.
- Commit by path, pull `--no-rebase --no-edit`, push. Tick the row only after its
  Accept passed, with `Landed <sha7>` at the end of the first line.
- Stop on `target/nonstop/STOP`, two `workspace-check` failures, a moved golden
  hash, or a model call failing three times.

## Throughput, honestly

The 2026-10-19 session landed or advanced about 30 rows in a day with three
loops, including judgment and re-runs. At 20-30 rows a day, 309 is roughly two
weeks of continuous running; at 10 a day it is a month. The 154 `LATER` rows
would add weeks and need an owner decision, so "all 695" is not a scheduling
target. The board also grows while it is worked; treat it as a backlog, not a
burn-down.

## Launch, after the owner says go

```
node tools/nonstop.mjs --ids ARC6,ARC9,ARC16,SP20,API6,SPR1,CT16,AB15,PB51,PB60,PB64,E16,E44,NS13,FU3,EG35,AD2,AD9,AD10,AD20,SP62,AX28 --max 5 --minutes 40
node tools/nonstop.mjs --ids AS5,CT10,CT11,CT13,E70,E73,NS17,NS20,GN6,FU6,AD15,GS37,QW6,AM26 --max 5 --minutes 40
node tools/nonstop.mjs --ids ARC5,MP12,MP16,E15,E40,E50,ON26,ON28,ON29,BX9,MP26,QN9 --max 5 --minutes 40
```

No `--retry`: the shared ledger already holds attempted ids. The three loops
write one ledger and one log per row under `target/nonstop/`.

## Owner decisions this plan waits on

- Archive `SPRINT-01`/`02`/`09` and the 2026-10-02 `res` rows, or keep them?
- Close the 32 obsolete rows plus GN4/GN5/GN7/GN8?
- Keep `LATER` frozen (recommended) or unfreeze a named cluster?
- Call a quiet window for the timing rows (FC1, OPT31, OPT33, RL5) before the
  loops start, or accept that their numbers wait.
