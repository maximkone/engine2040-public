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
- CI shape (same target dir path, artifacts removed, cache primed): the new
  `rebuild` cell against the `cold` cell. The cell is landed and pinned, but
  its run did not finish inside this packet's time budget on this box (the
  cold cell alone ran 17 min while 24 cargo builds of other agents ran), so
  the CI-shape number is still open: run
  `node tools/compile-cache-spike.mjs --lane engine-provider --jobs 4` on a
  quiet box and read the `rebuild` row of the table (the report lands in
  `target/spike-opt9/<utc>/report.json`).

**Proof**

```
node --test tools/compile-cache-spike.test.mjs   ->  10/10 green
node tools/compile-cache-spike.mjs --lane engine-provider --jobs 4   ->  the cold cell did not finish on this loaded box
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
- Measured quantity: the dependency units each lane's build graph holds
  (`cargo tree -p <crate> -e normal,build --prefix none --format "{p}"`, no
  compilation), summed over the five lanes. This is the cheap proxy for the
  rebuild count; the rebuild count itself (the `Compiling` lines of five lane
  builds in one shared target dir) is what `tools/dep-rebuild-count.mjs`
  counts, and the cold build of that side did not finish inside this
  packet's time budget on this box (see "What is still open").
- **Before** (the tree as it is: no crate depends on the hack):
  provider 7, scene 19, nav 3, spatial 2, observe 2 = **33 dependency units**
  across the five lane graphs.
- **After** (the same five lanes with `engine-workspace-hack = { workspace =
  true }`, measured in a scratch copy of the workspace so the shared tree was
  not touched): provider 35, scene 42, nav 36, spatial 35, observe 35 =
  **183 dependency units**.
- Verdict: the count **rises 5.5x**, so the bar (a third less) is **not
  met** - with the union as it stands the spike is a **DROP**. The reason is
  the union list in `crates/workspace-hack/Cargo.toml` (serde with derive,
  serde_json, uuid with serde+v4, image with png+hdr+jpeg): every lane that
  takes the hack builds all four externals, even the lanes that use none of
  them (nav, spatial, observe: 2-3 units -> 35-36). The union has to be
  split per feature group and given only to the crates that use it.

**Proof**

```
node tools/dep-rebuild-count.mjs --target-dir target/spike-opt8/before2 --lane engine-provider --lane engine-scene --lane engine-nav --lane engine-spatial --lane engine-observe
(the cold build of the five lanes did not finish in 40 min on this box: 24 concurrent cargo builds from other agents)
cargo tree --manifest-path <tree>/Cargo.toml -p <crate> -e normal,build --prefix none --format "{p}"   ->  33 units before, 183 units after
node --test tools/dep-rebuild-count.test.mjs   ->  6/6 green
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

- The rebuild count itself (the `Compiling` lines of five lane builds in one
  shared target dir, before and after) is not measured: the cold build of
  the "before" side ran past 40 minutes on this box (24 concurrent cargo
  builds from the other agents) and was cut. The tool is ready and the
  direction is not in doubt (the union adds ~30 units to every lane that
  takes it), but the row's own number is still open; the next run should be
  on a quiet box, with `node tools/dep-rebuild-count.mjs --target-dir
  <fresh dir> --lane ...` on both sides.
- The five heavy lanes (render, editor, ui, material, game) that the CI
  matrix uses were not measured: a cold build of `engine-render` on this box
  is longer than the whole packet.
- The copies (the same crate built twice for two feature sets) are not in
  the graph count; they need the rebuild count to see them.

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
node --test tools/workspace-check.test.mjs tools/rust-quality.test.mjs tools/plan-atlas-cache.test.mjs tools/plan-atlas.test.mjs tools/compile-cache-spike.test.mjs tools/dep-rebuild-count.test.mjs tools/nextest-spike.test.mjs tools/wiring-scan.test.mjs tools/board-slice.test.mjs
  ->  148/148 green
node tools/workspace-check.mjs   ->  FAIL on crates/assets/src/proxy.rs (another agent's uncommitted file, outside my paths) and on the genre ledger; every other check passes
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

---

## Housekeeping for the lead

- The lead's auto-backup sweep (commit `dbbe6435`) picked up four throwaway
  measurement copies of mine: `tools/tmp-board-slice-before.mjs`,
  `tools/tmp-plan-atlas-before.mjs`, `tools/tmp-rust-quality-before.mjs`,
  `tools/tmp-ws-before.mjs`. They are deleted in the working tree; please
  commit the deletion by path (they are copies of the pre-change tools, kept
  only for the A/B measurement).
- My files (all under my ownership): `tools/workspace-check.mjs` +
  test, `tools/wiring-scan.mjs`, `tools/rust-quality.mjs` + test,
  `tools/plan-atlas.mjs`, `tools/plan-atlas-cache.test.mjs`,
  `tools/board-slice.mjs`, `tools/compile-cache-spike.mjs` + test,
  `tools/dep-rebuild-count.mjs` + test, `tools/nextest-spike.mjs` + test,
  and this receipt. I committed nothing and staged nothing.
