---
name: opencode-loop
description: "Use when you launch helpers, write shell, edit files or repeat a failed call in a sprint turn or a helper packet: foreground batches (packet: <name>), the RESULT line, pwsh-only shell, permission-safe calls and compaction-safe edits."
---

# Run the OpenCode loop right

## Foreground batches

- Helpers run in the foreground: send the whole batch as ONE message of Task calls, each prompt exactly `packet: <name>`. The message returns when the last helper ends.
- Never `background: true`. Keep packets about the same size: a batch waits for its slowest helper.
- Aborting the parent cancels the batch with no result. Its packets go back to the ready queue.
- Every todo written with `todowrite` needs `status` and `priority`.

## The RESULT line

- Every helper reply ends with one line: `RESULT: DONE|PARTIAL|BLOCKED|NOOP - <what changed> | proof: <command result, file or URL>`.
- The orchestrator judges from that line and the diff. No RESULT line means the packet did not finish.

## pwsh shell

- The shell is `pwsh`, never bash. Do not use `grep`, `head`, `tail`, `wc`, `cd` or `2>/dev/null`. Use `Select-String`, `Select-Object -First N`, `Measure-Object -Line` and `2>$null`.
- Never `cd`: commands run in the repo root already.
- Every command of a compound call is permission-checked. `git grep x | Select-Object -First 20` is refused unless both sides are allowed.

## Permissions

- Never ask a question and never wait on a prompt. Decide from the packet and report.
- Never make a worktree or a branch: lanes are retired. Edit only the packet's OWNED paths in `C:\engine2040` on master. Do not run `git add`, `commit`, `stash`, `checkout` or `restore` unless the packet names it.
- No owned paths in the packet: report `RESULT: BLOCKED - no-owned-paths <id>`, no retry.

## Compaction-safe edits

- Re-read every file right before you edit it. Match whitespace exactly. One Edit per call. Never retry an Edit from remembered text.
- Never repeat a failed call unchanged. Fix it, retry once, then stop the step with BLOCKED and the error text.
- Never guess a path. Glob or `git grep` first. A 404 means stop, not rephrase.
- Keep outputs small: narrow reads, `Select-Object -First`, one file per call.
- On a Grep overflow, narrow the Grep tool to one folder plus one `include` glob. Never a repo-wide grep.
