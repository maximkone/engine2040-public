# The agent fleet: one lead, one level of helpers, gates that judge (owner 2026-10-22)

Owner, 2026-10-22: "can subagents spawn subagents, minimax spawns 5, each spawns 5 dumb coders, a
decider in between?"; "switch to 1"; "make all subagents commit"; "deny hard". Research:
`docs/research/finish-line-2026-10-22/04-arxiv-agents.md` and `00-verdict.md`. Rows: AGT01-AGT19 in
`docs/SPRINT-31-research.md`.

## The shape

- One lead per session. It reads the board, splits rows by the files they touch, and gives each
  helper one row whose files no other helper holds.
- One level of helpers, 3-4 per wave. A helper sends no helpers (`experimental.subagent_depth` 1
  in `opencode.jsonc`, 16b5b2842). Nested teams were not measured to pay on coding, and setups
  without one central check spread more errors (2512.08296: up to -70% on sequential work, +80.8%
  on work that splits). Our rows share one golden hash and shared interfaces, so most are the
  sequential case.
- Gates judge, not a reviewer model: tests and clippy on GitHub (`tools/gh-check.mjs`), the
  rust-quality ratchet, workspace-check, and the AGENTS.md rules as one script (AGT04). The strong
  model reads the gate result and the diff, never the chat.
- Every helper lands its own proven step at once (AGENTS.md "Commit as you go"); a dead helper's
  work is adopted from `node tools/leftovers.mjs` or, with AGT03, from a saved patch.

## What broke on 2026-10-22, and the guard for each

| What happened | Guard |
|---|---|
| A failed merge reset the shared tree three times and every session's uncommitted files left it | the LFS phantom PNG renormalized (30f67cb6a); commit as you go; recovery steps in memory and below |
| An agent's whole-file write wiped another's uncommitted hunks | AGENTS.md: edit hunks, never rewrite a file another AI has dirty |
| Stash, reset, checkout of "." and clean ran in the shared tree | denied in `opencode.jsonc` (3fe775ebb, b0372cfc9) and in the local `.claude/settings.json` |
| Worktrees appeared (an Empire recovery lane, an OpenCode quiet copy) | `git worktree add` denied; one tree, `master` |
| Helpers nested without a check | depth 1 |
| Generated art sat uncommitted for 15 hours | `tools/leftovers.mjs`; generated content commits in the step that made it |

Recovery when work vanishes: the newest `refs/auto-backup/master` snapshot is a merge whose second
parent is the base; a failed merge leaves its saved copy as an unreachable "WIP on master" commit
(`git fsck --no-reflogs --unreachable`). Restore only files that are clean now and unchanged
upstream since that base, with `git restore --source=<commit> --worktree -- <paths>`.

## Open measures

- AGT19 measures the worker-count curve here (rows landed per hour and reds per landing at 2, 4, 8
  and 16 helpers); no paper measures it for coding.
- The free-model quota, not the agent count, is today's ceiling: the OpenCode log shows "Rate
  limit exceeded" while 20+ agents run.
