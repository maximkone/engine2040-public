# Cleanup report — 2026-10-18

Autonomous, conservative pass over `C:\engine2040` (master) at HEAD `67ae5139`.
Rule followed: delete only clearly stale *private* `target/` scratch that no rule
protects and no build could be using; keep `deps/`; keep anything younger than 3
hours; if a name is uncertain, leave it and list it. Nothing outside `target/`
was touched. No other session's file was removed.

## 1. Git state

- `git status --short` — empty. Working tree clean, nothing uncommitted.
- `git status` — `On branch master`, up to date with `origin/master`.
- `git stash list` — 5 stashes, left in place (not mine, not touched):

  | stash | subject | date |
  |---|---|---|
  | 0 | `janitor-r43-f2p` | 2026-10-16 |
  | 1 | `hard-merge: local edits to six overlap files` | 2026-10-14 |
  | 2 | `spill: unclaimed build_level split circa 03:40Z, superseded by judged lane patch` | 2026-10-10 |
  | 3 | `px7 land revert ratchet-red` | 2026-10-08 |
  | 4 | `render-dirt-2026-10-06-foreign-v1-test-gate` | 2026-10-06 |

No uncommitted files to attribute to an owner: there are none.

## 2. Branches and remotes

- `git branch -a` — one local branch: `master` (tracks `origin/master` at `67ae5139`).
- `git remote -v` — one remote: `origin` → `https://github.com/maximkone/engine2040.git`.
- Remote-tracking refs present:
  - `refs/remotes/origin/master` → `67ae5139`
  - `refs/remotes/origin/HEAD` → `origin/master`
  - `refs/remotes/origin-tags/lucid-hopper-mcam05` → `5cbc82b6` (2026-09-24)

**Stray ref (left, reported):** `origin-tags/lucid-hopper-mcam05` belongs to a
remote named `origin-tags` that is **not** in `git config` (only `origin` is).
It is a stale remote-tracking ref from a removed remote. Left in place — not
deleted per instructions. Clear it by hand with
`git update-ref -d refs/remotes/origin-tags/lucid-hopper-mcam05` if wanted.

No local stray branches.

## 3. `target/` sizes (with hidden files)

| dir | size | last write | verdict |
|---|---|---|---|
| `target/debug` | 45.8 GB | 2026-10-18 17:36 | kept — workspace build cache, rule-protected (`clean.mjs` needs `--include-debug`) |
| `target/release` | 1.45 GB | 2026-10-18 18:01 | kept — release cache, live |
| `target/reports` | 1.43 GB | 2026-10-17 16:48 | kept — live pass-truth / product-check output |
| `target/autopush-check` | 1.15 GB | 2026-10-18 21:00 | kept — **live**, written minutes before this pass |
| `target/public-export` | 574 MB | 2026-10-18 14:19 | kept — clone of the public remote, referenced by `public-export.mjs` |
| `target/spot-clone` | 498 MB | 2026-10-16 05:07 | **DELETED** (see below) |
| `target/captures` | 302 MB | 2026-10-18 18:09 | kept — captures, live |
| `target/gh-check` | 195 MB | 2026-10-18 21:02 | kept — active check cache (`base-*.tree`, `wip-*.index`); no `jobs/` dir exists |
| `target/clips` | 159 MB | 2026-10-18 03:07 | kept — clip captures, recent |
| `target/pilot-gx13` | 38 MB | 2026-10-18 00:37 | left — captures, purpose uncertain |
| `target/rz4w-cap` | 33 MB | 2026-10-17 21:44 | left — captures, purpose uncertain |
| `target/books` | 32 MB | 2026-10-10 21:08 | left — `books/index.json` cache, referenced by `tools/books.mjs` |
| `target/queue` | 18 MB | 2026-10-17 13:43 | kept — live packet queue; `target/abandoned` is here in spirit, see note |
| other lane/scratch dirs (`c1-*`, `c3-*`, `mp6-*`, `rl21-*`, `shake-watch-*`, …) | ≤ 15 MB each | 2026-10-10…15 | left — small, some named after rows; uncertain |

Total `target/` ≈ **50.5 GB**.

## 4. Deleted — exact paths

All under `C:\engine2040\target\`:

1. `C:\engine2040\target\spot-clone` — a **standalone** clone (its own `.git`
   directory at commit `5a24295a`, clean), last written 2026-10-16 05:07,
   measured 498.3 MB on disk. Not a linked worktree (`git worktree list` shows
   only `C:/engine2040`); no `.git\worktrees` entry; **not referenced anywhere**
   in the repo (`grep spot-clone` → no matches). No build could be using it: no
   `cargo`/`rustc` process running. Safe to remove; re-clone if needed.
2. `C:\engine2040\target\web-run` — 14 × `base-*.tree` cache files (~560 bytes
   total), all older than 3 hours. Scratch cache for `tools/web-run.mjs`.
3. `C:\engine2040\target\tmp` — 10 scratch files (`skill-cites*.mjs`, `brute.mjs`,
   …), ~14 KB, all older than 3 hours. Declared trash by `tools/clean.mjs`
   (`Trash (auto): target/tmp, …`).

## 5. Disk space (C:)

| | bytes | GB |
|---|---|---|
| free before (delete-time) | 72,798,257,152 | 67.80 |
| free after (delete-time) | 73,002,606,592 | 67.99 |
| **freed** | **204,349,440** | **0.19** |

Free measured again after: 67.99 GB. The whole `target/` tree is busy (many live
`node` processes, `autopush-check` written minutes before), so this delta is
approximate. `spot-clone` measured 498.3 MB on disk but the free-space delta is
0.19 GB — NTFS dedup/compression, plus other sessions writing during the pass.

## 6. Left, and why

- `target/debug`, `target/release` — rule-protected build caches; live.
- `target/autopush-check`, `target/gh-check`, `target/public-export`,
  `target/captures`, `target/clips`, `target/reports`, `target/queue` — live
  output or cache referenced by tools (`clean.mjs`, `gh-check.mjs`,
  `public-export.mjs`, `proof.mjs`, `product-check.mjs`).
- `target/abandoned` — holds ABANDONED patches referenced in `docs/BLOCKERS.md`
  (`mp26-resim-ledger`, `eg18-acts-extract`, `cut-audio-occlude`, `r133-latency`,
  `p6-cassette`, …). Do **not** delete.
- `target/rot`, `target/evidence`, `target/perf`, `target/q1` — referenced by
  tools/docs (`rot` scans, `part-score.mjs`, `proof.mjs`, `q1-check.mjs`).
- `target/pilot-gx13`, `target/rz4w-cap`, `target/books` — captures / tool
  cache; purpose uncertain, so left and listed.
- All `target/*` lane dirs and `target/locks`, `target/sprint.lock` — small or
  meaningful; `sprint.lock` is never auto-touched by rule.

## 7. Suspicious, left for a human

- `refs/remotes/origin-tags/lucid-hopper-mcam05` — orphaned remote-tracking ref
  (remote `origin-tags` no longer configured), 2026-09-24. See §2.
- `target/spot-clone` was a full repo clone sitting in `target/` for ~2 days,
  unreferenced. Likely a leftover manual "spot check" clone; watch for the
  pattern returning.
- 5 stashes dating back to 2026-10-06; none mine, none touched.
