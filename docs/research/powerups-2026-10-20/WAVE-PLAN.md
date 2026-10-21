# Wave plan: powerups 2026-10-20 (lead's file)

Owner: the lead seat (Maxim's chat, 2026-10-20). Helpers write files; the lead merges and commits.
Rules for everyone: `RULES.md` in this folder.

## Lanes and what each one owns

| Lane | Output file | Owns code in | Session |
|---|---|---|---|
| G1 render + 2040 look | `01-render-look-steals.md` | crates/render (BUSY: GS44) | ses_ea5157248ffelKGjO8QP48JTEF |
| G2 physics, VFX, destruction | `02-physics-vfx-destruction-steals.md` | crates/physfx, crates/destruction | ses_ea5157244ffeg1vajrM6UXNX1U |
| G3 animation + feel | `03-anim-feel-steals.md` | crates/anim (BUSY: AM24) | ses_ea515723dffeuZ1GnYzQ7QT5kX |
| G4 AI-native / MCP | `04-ai-native-mcp-steals.md` | crates/endpoint, crates/spatial | ses_ea5157238ffeQFkCErtlL1jrHq |
| G5 web, perf, assets, shipping | `05-web-perf-shipping-steals.md` | crates/web, crates/net | ses_ea5157233ffel9NNl41AdI6h6V |
| A1 arXiv rendering | `06-arxiv-rendering.md` | crates/material | ses_ea5157221ffegB634FAmQ2B2m8 |
| A2 arXiv simulation | `07-arxiv-simulation.md` | crates/physics | ses_ea5157023ffeFhAocytO4SVtmY |
| A3 arXiv AI for games | `08-arxiv-ai-games.md` | crates/provider (BUSY: OPT27) | ses_ea515701fffeDCbK5Pl3nyoC6m |
| C1 code + context compaction | `09-code-compaction.md` | tools (BUSY: speedup) | ses_ea515701dffezvhihjP8Phm1Wn |
| W1 workspace underused | `10-workspace-underused-powerups.md` | none (proposal only) | ses_ea5157016ffe2BUZupfqEq0p4x |
| N1 collisions | `11-collisions-cloth-layers.md` | crates/physics, crates/physfx | ses_ea50e6835ffez4vXbyJjOqx2GS |
| N2 auto making | `12-auto-making-content.md` | crates/geo, crates/world | ses_ea50e6831ffeCNfwieWT2zHJ76 |
| N3 auto heal | `13-auto-heal-resilience.md` | crates/observe | ses_ea50e67dfffeC1qZhM6aO67UnP |
| N4 smart engine | `14-smart-ai-native-engine.md` | crates/ai, crates/spatial | ses_ea50e67daffexZj7EhhdksvNoQ |
| N5 layers above and below | `15-layered-architecture.md` | crates/nav, crates/spatial | ses_ea50e67d6ffetHnAJfdneWyZMD |

Landing agents (code only, one crate each, no commit):
GS44 crates/render | WEB6 crates/assets + crates/game | OPT27 crates/provider | AM24 crates/anim |
speedup tools/ + .github/workflows/ + root Cargo.toml + crates/workspace-hack.

## The follow-up pass (sent to each research lane after it reports)

1. `proposed-rows-<lane>.md` in this folder: one row per find, in this shape:
   id (lane prefix, e.g. PX-01), title, what it is, donor + URL + licence, which row it closes,
   owned paths, Accept (a command and its one-line result), kill line, cost.
   Nobody edits `docs/SPRINT*.md`; the lead merges, so no two agents touch the 598 KB board.
2. Code, only for S rows whose crate is free: tests first, `cargo check -p <crate>` to iterate,
   `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` to prove, golden `d57b5c8ae3096614` unmoved.
3. A receipt file `landing-<lane>.md`: paths changed, proof commands and their one-line results, what is open.
4. Never commit. The lead commits by path.

## Lead's consolidation, in order

1. Read the 15 lane files and the 5 receipts.
2. Write `00-INDEX.md` here: one table, every find, ranked, with the row it closes.
3. Merge the proposed rows into one file `proposed-rows-ALL.md`, then into the board (one commit, by path).
4. Review the landed code; commit by path in C:\engine2040 with a proof line in the body.
5. Report: what landed, what is open, what the next wave should hunt.

## Ground rules that keep 20 agents from colliding

- One crate, one owner. If a lane's crate is BUSY in the table above, it writes a proposal, not code.
- No agent edits `docs/SPRINT*.md`, `docs/FOCUS.md` or `docs/plan/finish-line.md`. The lead does.
- No agent edits a Cargo.toml of a crate it does not own.
- Nobody runs two cargo commands at once. "Blocking waiting for file lock" means another build holds `target/`: wait.
- Nobody touches `.git/index.lock`, never `git clean`, never deletes under `target/`.

## Commit rule, changed 2026-10-21 after a wipe

**Lanes commit their own paths AND push master, with the proof line in the commit body.**

The owner landed the same rule in `AGENTS.md` on 2026-10-21 ("we work locally"):
commit by path and push each small change as soon as it is proven; never leave
work uncommitted across a pull; pull with `--no-rebase --no-edit` and **no
`--autostash`**; when a pull refuses because an uncommitted file is also changed
upstream, wait for its owner to commit - never stash, commit or revert their work
to force the merge through; the `Empire-AutoPush` task is snapshot-only for
engine2040 (it holds the dirty tree on `refs/auto-backup/master` and never
commits, merges or pushes master). Sessions own their commits and their pushes.

Why the wave changed too: a session in the shared tree restored tracked files
under the wave's lanes at 2026-10-21 (no `git reset` in the reflog, so it was a
path restore, which leaves no trace). The smart lane's audio wiring was
uncommitted and gone; the plan trace came back at the untightened version. A path
restore is not revertible by git, and uncommitted work has no copy of it.

So: the lead still merges the board and still owns the docs of record, but a lane
that owns a crate commits that crate's paths itself, by path, with the proof
command and its one-line result in the body, and pushes master. The pre-commit
hook still refuses a commit whose rust-quality ratchet regresses; that refusal is
not to be bypassed with `--accept` on fresh code. If a push is rejected because
the remote moved, pull without autostash and push again - and if the pull refuses
because another lane has an uncommitted file, wait for that lane.


