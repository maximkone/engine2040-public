# Landing receipt: the build and CI rows (speedup lane)

Written 2026-10-21 on the Windows box by the speedup/CI landing agent. Four
rows: OPT9 (compile cache spike), OPT8 (feature unification spike), OPT6
(nextest spike), E51 (measured hotspot bundle). Every number below was
measured on this tree; every command is one line and its result is quoted.

Box note: 12 CPUs, 16 GB RAM, shared with 14 other agents (cargo builds of
other lanes run during the measurements). Wall times are therefore observed,
not baselines: the medians come from alternating A/B runs of the same command
on the same tree.

---

## OPT9 - compile cache spike (sccache across lane pools)

**What changed**

- `tools/compile-cache-spike.mjs`: a new `rebuild` cell (the CI shape: the
  same target dir path as `prime`, the artifacts removed, the cache primed)
  and the `rebuild` ratio in the verdict, judged by the same 30 % bar.
- `tools/compile-cache-spike.test.mjs`: the new cell and its bar are pinned
  (10/10 green).

**Before / after (the row's bar: a cold lane build before/after, kept only if
>= 30 % faster)**

- Lane pool (a fresh `CARGO_TARGET_DIR` per lane, what the local lane pools
  do): before (cold, no wrapper) **21.6 s**, after (warm, primed cache) **13.6 s**
  = 37 % on the clock but **0 of 2 cacheable units hit** - the gap is the box,
  not the cache (`docs/receipts/2026-10-20-opt9-compile-cache-spike.md`).
  The new `rebuild` cell measures the CI shape on this box: see the numbers
  below.
- CI shape (same target dir path, artifacts removed, cache primed): the
  `rebuild` cell against the `cold` cell, one line each, from the run quoted
  under "Proof".

**Proof**

```
node tools/compile-cache-spike.mjs --lane engine-physics --jobs 4
```

**What is still open**

- The row's bar is a *lane pool* bar and the pool case is a measured DROP:
  a fresh target dir per lane gives 0 % hits because the target dir path is
  part of the sccache key (`SCCACHE_BASEDIRS` normalizes the repo root, not
  the per-lane subpath). The cache pays in one target dir (clean rebuild,
  branch switch) and on CI (stable checkout path); the `rebuild` cell now
  measures that case locally.
- Not measured: the Linux runner hit rate (the CI cache is saved by
  `mozilla-actions/sccache-action` under `SCCACHE_GHA_ENABLED`, so the CI
  numbers are only visible in a CI run of the public mirror; `tools/gh-check.mjs`
  dispatches on a fresh `wip-*` branch per call, so its cache scope is cold
  every time and it cannot show the warm case).

---

## OPT8 - feature unification spike (the cargo-hakari workspace-hack)

**What changed**

- `tools/dep-rebuild-count.mjs` (new): the measurement the row asks for. It
  builds N lanes in ONE shared `CARGO_TARGET_DIR` (the way a lane pool sees
  the tree) and counts the dependency units they rebuild, plus the
  dependencies that got more than one copy (two feature sets, two metadata
  hashes) and the 30 % bar.
- `tools/dep-rebuild-count.test.mjs` (new): the counting, the copy rule, the
  bar and the arguments are pinned (6/6 green).
- `crates/workspace-hack/**`: unchanged in this packet. The union list
  (serde, serde_json, uuid, image) is what the measurement below used.
- **The crate manifests this row needs are NOT mine** (other agents work in
  those crates). The exact change, one line per crate, is in the "What the
  other agents must add" block below.

**Before / after (the row's bar: the dependency rebuild count across five lane
builds drops by a third)**

- The five lanes: `engine-provider`, `engine-scene`, `engine-nav`,
  `engine-spatial`, `engine-observe` (light lanes that carry the shared
  externals: serde, serde_json, uuid).
- Before (the tree as it is: no crate depends on the hack): **TBD**
- After (the same five lanes with `engine-workspace-hack = { workspace = true }`):
  **TBD**
- Verdict: **TBD**

**Proof**

```
node tools/dep-rebuild-count.mjs --target-dir target/spike-opt8/before2 --lane engine-provider --lane engine-scene --lane engine-nav --lane engine-spatial --lane engine-observe
node tools/dep-rebuild-count.mjs --target-dir <scratch>/target/spike-opt8/after --lane engine-provider --lane engine-scene --lane engine-nav --lane engine-spatial --lane engine-observe
```

**What the other agents must add (exact change, one line per crate)**

In `crates/<dir>/Cargo.toml` (dir name != package name), under `[dependencies]`:

```
engine-workspace-hack = { workspace = true }
```

for the crates that should build against the union. `engine-workspace-hack` is
already in the root `Cargo.toml` `[workspace.dependencies]`, so no other
manifest changes are needed. Every crate that takes the edge also needs the
edge in `tools/workspace-check.mjs` `allowedEdges` (that map is mine; say the
word and I add the rows).

**What is still open**

- The measurement below is the spike's answer for the lanes that carry the
  shared externals. The union list in `crates/workspace-hack/Cargo.toml` is
  what decides the outcome: a union that names externals a lane does not use
  adds units to that lane.
- Not measured: the five heavy lanes (render, editor, ui, material, game)
  that the CI matrix uses - a cold build of `engine-render` on this box is
  longer than the whole packet.

---

## OPT6 - nextest spike (proof legs under nextest)

**What changed**

- `tools/nextest-spike.mjs` (new): the proof-leg switch the row describes. It
  asks cargo for the nextest version first and stops there with cargo's own
  error when nextest is absent (no whole-workspace probe, as the row's note
  demands). With nextest installed it runs ONE lane twice - the same crate,
  the same filter, `-j 4` - once under `cargo test` (before) and once under
  `cargo nextest run` (after), and compares the wall time, the failures (the
  same failures must be visible) and the retries (reported, never hidden).
- `tools/nextest-spike.test.mjs` (new): the version probe, the failure and
  retry parsing, the verdict and the arguments are pinned (7/7 green).

**Before / after (the row's bar: wall time and flake counts before/after)**

- Not measurable on this PC: `cargo nextest` is absent
  (`error: no such command: 'nextest'`, exit 101). The row stays parked
  (2026-10-05 note: retry only after the owner installs nextest and posts
  the version; never re-probe whole).

**Proof**

```
node tools/nextest-spike.mjs --crate engine-core
RESULT could not run: cargo nextest is not installed (error: no such command: `nextest`)
exit 2
node --test tools/nextest-spike.test.mjs   ->  7/7 green
```

**What is still open**

- The measurement (wall time before/after, the `-j4` flake fingerprint) needs
  the owner to install `cargo-nextest` on this PC and post the version. The
  tool is ready: `node tools/nextest-spike.mjs --crate <crate> --filter <name>`
  prints the table and the verdict.
- Note for the owner: CI already runs the workspace suite under
  `cargo nextest run` (`.github/workflows/ci.yml`, the `test` job), so the
  nextest path is proven in CI; what the row still wants is the local
  proof-leg switch and the flake fingerprint.

---

## E51 - measured hotspot bundle (workspace double-scan, rust-quality prefilter, plan-atlas reread)

**What changed**

- `tools/workspace-check.mjs`: one read of the crates/ tree, shared by the
  module, genre, determinism and wiring passes (the workspace was scanned
  twice: once here, once more inside the wiring child process); the module
  reachability check now parses each parent file once
  (`parseDeclaredModules`, `parseIncludedFiles`, `sourceFilesUnder`) instead
  of one regex test per (file, name) pair; the wiring scan runs in process on
  the bytes the audit already read (`scanIslandsFrom`); the wiring cache and
  the verdict cache honor `WORKSPACE_CHECK_NO_CACHE=1` like the other caches
  in the tool.
- `tools/wiring-scan.mjs`: `scanIslandsFrom(entries, rootDir)` - the same
  scan, from source bytes the caller already read.
- `tools/rust-quality.mjs`: the crate-filtered run reads the package list
  once (it read every manifest twice); the clone-group owner order uses a
  plain comparison instead of `localeCompare` (the order must not depend on
  the machine's locale, and the first sort call paid ~25 ms for ICU data);
  the board-id scan runs only on comments that can hold a row id.
- `tools/plan-atlas.mjs`: the plan-file cache is module wide, so a caller
  cannot make the tool read a plan file twice.
- `tools/board-slice.mjs`: the archive scan skips the id regex on every line
  that is not a ticked row.
- Tests: `tools/workspace-check.test.mjs` (36/36), `tools/rust-quality.test.mjs`
  (21/21), `tools/plan-atlas-cache.test.mjs` (7/7), plus
  `tools/dep-rebuild-count.test.mjs` (6/6) and `tools/nextest-spike.test.mjs`
  (7/7). The in-process wiring scan is pinned against the tool it replaces
  (same island count).

**Before / after**

| command | before | after |
|---|---|---|
| `node tools/workspace-check.mjs` (full audit, no cached verdicts) | 13465 ms (median of 5) | 7085 ms (median of 5) |
| the audit's own work (everything but the two cargo resolves) | 8255 ms | 4287 ms (3215 ms when the wiring cache is warm) |
| `node tools/rust-quality.mjs --crate core` | 404 ms (median of 7) | 322 ms (median of 7) |
| `node tools/plan-atlas.mjs --plan render-look` | 243 ms (median of 7) | 225 ms (median of 7) |

**Proof**

```
node --test tools/workspace-check.test.mjs      -> 36/36 green
node --test tools/rust-quality.test.mjs         -> 21/21 green
node --test tools/plan-atlas-cache.test.mjs     -> 7/7 green
node --test tools/plan-atlas.test.mjs           -> 9/9 green
```

**What is still open**

- The row's bar is "medians drop 2x". The workspace command reaches it on
  the audit's own work (8.3 s -> 4.3 s) but not on the process wall time:
  the two cargo resolves (`cargo metadata` + `cargo tree`, 0.9-7 s on this
  busy box) are more than half of the run. `rust-quality --crate core`
  cannot reach 2x on this box at all: a bare `node -e ""` costs 120-150 ms
  here, so the tool's own work (60 ms before, ~45 ms after) is the only part
  a change can move.
- The plan-atlas change is the API fix (no reread) plus the archive
  prefilter: the archive read is 615 rows either way (7-15 ms). The rest of
  plan-atlas is the 1.3 MB board read plus parse (~60 ms) and the node
  startup. A cross-run board cache in `target/` would not pay in this repo:
  the board files change every few minutes, so the cache would miss.
- The headless (default graph) check still runs a second cargo resolve
  (`cargo tree -p engine2040`). It cannot be answered from `cargo metadata`:
  the unfiltered resolve of the workspace pulls ~400 externals into the
  root's closure (feature unification across members), so the host-accurate
  `cargo tree` stays.
