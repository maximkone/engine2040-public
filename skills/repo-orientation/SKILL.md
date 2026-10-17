---
name: repo-orientation
description: "Use when you enter an unfamiliar crate, rank risk before a plan, or a root Grep dies (Ripgrep JSON record exceeded): intent-first map, git-history hotspots, crate owners, narrow chunked search."
---

# Repo orientation

Read-only. Never checkout, rebase or edit while you map. Scope the map to the row's focus area. Skip untouched crates.

## 1. Map before code (under 120 s)

Write each section in three lines or less. If two sections come back empty on a non-empty tree, or it takes over 120 s, stop and narrow the scope.

| heading | what to state |
|---|---|
| Intent | README, the row's plan slice, its spec. Summarize in three lines before you open any source file. |
| Scan | Count files through the crate map and modmap plus one board slice. Never read whole trees or whole docs. |
| Architecture | One row per crate touched: package, directory, job, engine dependencies. |
| Boundaries | Dependency direction per crate (`docs/ARCHITECTURE.md`). Name any new edge the row would add. |
| Determinism | The hash law (`docs/HASH-LAW.md`), the fixed tick, what the golden covers. Flag wall clock or map iteration order in hashed state. |
| Tests | The acceptance command per touched row: crate check, crate test, then the proof or capture gate. |
| Unknowns | List them with who can answer and what done looks like. Stop on ambiguity. Do not guess. |

## 2. Where things are

- Order: `docs/FOCUS.md` "## Now". Rows: `node tools/board-slice.mjs --ready`, `--focus`, `<ID>` (never read a shard whole).
- Plans: `docs/plan/README.md`. A row's plan: `node tools/plan-atlas.mjs --row <ID>`.
- Docs: `docs/README.md` (one screen), `docs/INDEX.md` (contracts). Crates: `docs/CRATE-MAP.md`.
- Files inside a crate: grep a crate name in `.opencode/modmap.md`. Never read it whole. Glob skips dot folders from root, so put `.opencode` in `path`.

## 3. Hotspots (after the map, never instead of it)

Rank risky files from git history before you plan. Plan tests around the top ones first.

1. Churn: `git log --format= --name-only -n 200` from the repo root. Group by path in pwsh (`Group-Object`), sort by count, keep the top 10.
2. Hotspot score: churn times recent fixes. A file touched often and fixed often outranks a file touched once long ago.
3. Momentum: count recent fix, revert and hotfix commits per top file (`git log --oneline -n 200 -- <file>` and look at the subjects).
4. Owner: map each top file to its crate in `docs/CRATE-MAP.md`. Name the crate beside the file.
5. Bus factor: `git shortlog -sn -- <file>` per top file. Flag a file when fewer than 30% of its authors are still active.
6. Write a short risk note: the top 3 hotspots and what your plan avoids touching in each.

## 4. Searching

`C:\engine2040\.ignore` hides `content/arena/src/*.gltf` (one-line files of 2 to 8 MB, they broke grep with `Ripgrep JSON record exceeded 65536 bytes`) and the archive docs (`docs/archive/`, `docs/SPRINT-ARCHIVE.md`). Read by path still works for those.
If a search still overflows or returns nothing useful:

- Never search from the root. Pass one folder (`crates`, `docs`, `tools`) and one include glob (`*.rs`, `*.md`, `*.mjs`) per call.
- Never scope to `content/arena/src/` alone. Scope `content/` only with an include and one subfolder.
- Make a broad regex an exact literal. Page long output with `Select-Object -First N`.
- Chunk folder by folder, merge the hits, report scope plus include plus chunk results. Never rerun the same wide search.
- Code over one line goes to a temp `.mjs`, not a multi-line `node -e`.
