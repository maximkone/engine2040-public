# WEB6 landing receipt: content by fetch with one load queue

Row: WEB6 (`docs/SPRINT-22-web-realism.md`) - "assets: content by fetch, not by
file - an `AssetSource` seam in `crates/assets` and `crates/game` (native:
files; web: a fetched bundle)".

Finish-line gap A2 ("Content by fetch with one load queue"): a `LoadQueue`
with priority, byte budget and cancel; the generational `AssetHandle`
(`crates/assets/src/handle.rs`, no caller before today) becomes the way the
game holds an asset, with unload.

Date: 2026-10-20/21. Owner of this slice: the landing agent for WEB6.
Paths owned: `crates/assets/**`, `crates/game/**` only.

## What changed

### `crates/assets` (the seam and the queue)

- `crates/assets/src/load_queue.rs` (new): `LoadQueue` over an `&dyn
  AssetSource`.
  - `enqueue(path, priority, bytes)` names content by logical path and hands
    back a generational handle before the bytes arrive; the same path twice
    is one handle; a request over the byte budget is refused with
    `LoadError::OverBudget` and stages nothing.
  - `pump(max_bytes)` reads the queue in priority order (higher priority
    first, then path order), admits at most `max_bytes` bytes per call, and
    reports every step: `Loaded`, `Deferred` (does not fit the budget yet),
    `Failed` (the source refused the path, by name), `Cancelled`,
    `Unloaded`.
  - `cancel(handle)` drops a queued load (its reservation goes back, the
    handle reads `Cancelled`), `unload(handle)` frees a resident asset (the
    handle goes stale, the bytes go back to the budget).
  - `payload`, `state`, `is_ready`, `failure_reason`, `resident_bytes`,
    `reserved_bytes`, `remaining_bytes`, `handles`.
  - Std-only: no clock, no RNG, no threads, no file system. The caller drives
    the pump, so the same code runs natively over files and in a browser over
    a fetched bundle.
- `crates/assets/src/asset_source/fetch.rs` (new): `FetchSource`, the fetch
  source. It asks a host fetcher for one file (the packed `.e2bundle` in the
  browser) and serves the logical paths inside it. The fetch happens once, on
  the first read; a refused fetch is reported by name and never cached.
  `FetchSource` is the same `AssetSource` the game already takes, so the game
  code does not change between the native and the browser path.
- `crates/assets/src/asset_source.rs`: the `fetch` module, the `FetchSource`
  re-export and the module doc.
- `crates/assets/src/handle.rs`: the handle store grew the two pieces the
  queue needs.
  - `AssetStore::stage(key)` stages a slot before its bytes arrive (the
    caller owns the key namespace: the queue keys by logical path, the
    importer keys by content; the two key spaces never share a slot).
  - `AssetStore::mark_loaded_owned(handle, bytes)` stores the payload without
    a second copy (`mark_loaded` now goes through it).
  - `AssetStore::mark_cancelled(handle, reason)` and the new
    `LoadState::Cancelled`, so a cancelled load is its own state and not a
    failure.
- `crates/assets/src/lib.rs`: the `load_queue` module and the
  `LoadError`/`LoadEvent`/`LoadQueue`/`FetchSource` exports.

### `crates/game` (the caller)

- `crates/game/src/kit_assets.rs` (new): `KitAssets`, the way the game holds
  assets. It stages every kit file through one queue (`src/<file>.gltf` by
  logical path), reads the bytes through the handle, and `unload()` frees
  every handle and gives the bytes back. `kit_load_priority(file)` puts the
  ground and the collision pieces (2) before the dressing (1), so the arena
  stands before the banner hangs.
- `crates/game/src/arena_kit.rs`: both legs of the kit import go through the
  queue.
  - The app leg (`import_kit_from(source)`) reads the 8 kit files through
    `KitAssets` instead of `source.read(...)`; the cooked map sets
    (`kit_pbr`) still read through the source.
  - The headless leg (`import_kit(root)`) is now `import_kit_from` over an
    `FsSource`, so the headless kit import has no `std::fs` of its own and
    the browser can hand it a fetched bundle. The `std::fs` call site in
    this file is gone (the wasm-gate counts it as an improvement).
  - `bound_diag` is `pub(crate)` now, so `kit_assets` can bound its reasons.
- `crates/game/src/lib.rs`: the `kit_assets` module.
- `crates/game/Cargo.toml`: `engine-assets` is a plain dependency now (the
  seam is the game's content path); the default features keep the headless
  graph std-only (no `gltf`, no `image`) and the `app` feature turns on
  `engine-assets/import` for the real glTF importer.

Not touched (the licence gate, the 7-gate checklist and the provenance rows
stay as they are): the fetch moves bytes that the cook already gated; no
import path was added, so nothing reaches the game without the same gate the
native path has.

## Proof (commands and their one-line results)

- `node tools/gh-check.mjs --budget 840 cargo test -p engine-assets --lib`
  -> `PASS on the runner: 218 passed; 0 failed; 2 ignored` (run 39516602832,
  the run after the re-apply; the same result in run 39501747641). The new
  tests: `load_queue::tests` 10, `asset_source::fetch::tests` 4,
  `handle::tests` +3.
- `node tools/gh-check.mjs --budget 840 cargo clippy -p engine-assets
  --all-targets -- -D warnings` -> `PASS on the runner` (run 39505296566).
- `node tools/gh-check.mjs --budget 840 cargo test -p engine-game --lib`
  -> `FAIL on the runner: could not compile engine-arena (lib) - missing
  field 'cuts' in initializer of 'Sim' at crates/arena/src/fork.rs:664`
  (runs 39502244620, 39503726477, 39507547809, 39508752089). That red is
  another agent's in-flight work in `crates/arena` (not a path of this
  slice), and it is red at `HEAD` too (the last arena commits flip-flop on
  it). The shared tree also has `crates/observe`, `crates/audio`,
  `crates/render` and `crates/game/src/class_sheet.rs` (another agent's
  untracked file) red, so the game proof ran in a scratch copy of the tree
  (`%TEMP%\opencode\web6-check`) where only those foreign reds are patched.
- `cargo test -p engine-game --lib -- --test-threads=1` (in that scratch
  copy) -> `469 passed; 7 failed; 6 ignored`. The same 7 fail with
  `HEAD`'s `crates/game` (466 passed; 7 failed): they are the
  fighter-cap fallout of commit 15049ef3 (a 17-fighter config is accepted
  now, the HUD expects 16 slots), not this slice. This slice adds the 3
  `kit_assets` tests and the kit import tests stay green.
- `cargo test -p engine-proof --lib golden_hash_untouched` (in the same
  scratch copy) -> `1 passed; 0 failed` - the golden `d57b5c8ae3096614`
  stands.
- `cargo clippy -p engine-game --all-targets --no-deps -- -D warnings` (in
  the same scratch copy) -> red on 3 foreign lints only (`camera_rig.rs:541`,
  `crash.rs:658`, `camera.rs:1451`); no lint in `kit_assets.rs` or
  `arena_kit.rs`.
- `node tools/wasm-gate.mjs` -> `[IMPROVED] crates/game/src/arena_kit.rs: fs
  1 -> 0` (the headless kit import has no `std::fs` any more). The gate
  itself is red on 11 foreign regressions in `crates/render`, `crates/ai`,
  `crates/endpoint`, `crates/input`, `crates/observe`, `crates/physics`,
  `crates/playtest` and `crates/web` (other agents' in-flight files); the
  new files `crates/assets/src/load_queue.rs`,
  `crates/assets/src/asset_source/fetch.rs` and
  `crates/game/src/kit_assets.rs` carry no hazard at all.
- `node tools/rust-quality.mjs --owned crates/assets,crates/game` -> the only
  FAIL is `engine-game: string_errors 0 -> 11`, and all 11 hits are in
  `crates/game/src/class_sheet.rs` (another agent's untracked file). The
  files of this slice carry no hit in any metric.
- `rustfmt --edition 2021 --check crates/assets/src/load_queue.rs
  crates/assets/src/asset_source/fetch.rs crates/assets/src/handle.rs
  crates/game/src/kit_assets.rs crates/game/src/arena_kit.rs` -> clean.
  `crates/assets/src/lib.rs` still fails the check on lines that are not
  mine (the module order of `anim_compress`, `morph_import`, `proxy` and the
  `pub use residency::{...}` wrap were already unformatted and belong to
  other agents' inserts); the lines added here are clean.
- `node tools/workspace-check.mjs` -> red on foreign items only (`engine-arena
  -> engine-nav` edge, `engine-anim/src/pose_c1_test.rs`, `engine-assets/
  src/proxy.rs` not declared, float math in anim, the genre ledger). The
  module graph of this slice is complete (every new file is declared).
- The golden `d57b5c8ae3096614` cannot move: this slice touches no sim
  crate, no tick and no hash input (`crates/assets` and `crates/game` are
  outside the fixed tick). The proof above re-ran it green.

## Shared-tree hazard (read this before committing)

While this slice worked, another agent's `git checkout`/`git restore` on the
shared tree wiped the tracked-file edits of this slice twice (the untracked
new files survived both times). The edits were re-applied and re-proved
after the last wipe (the assets run 39516602832 above is from the
re-applied tree). If the tracked edits are missing again, the patch is
`%TEMP%\opencode\web6-backup\web6-tracked.patch` and the new files are
beside it (`load_queue.rs`, `fetch.rs`, `kit_assets.rs`). Commit the slice
by path before the next checkout storm.

## Still open

1. The game proof on the shared tree: `cargo test -p engine-game --lib`
   cannot run there until `crates/arena` (and the other foreign reds) are
   fixed by their owners. The scratch copy above is the proof that this
   slice's game code is green; re-run the same line on the shared tree once
   the tree compiles.
2. The `app` leg (`cargo test -p engine-game --features app --lib`) was not
   run here: the window/device stack runs on the PC, not on the runner. The
   app-leg kit import changed (the files come from the queue), so that leg
   wants one run.
3. The cooked map sets (`kit_pbr::load_cooked_set`) still read through the
   source, not through the queue; they are decoded into GPU uploads, so the
   game holds no raw asset bytes there, but the "one queue" claim is not
   complete for them.
4. The row's other open items stay with their owners: the fetch loop with
   partial-prefix open, the fighter_bind/fighter_look loaders and the bundle
   step in `tools/web-build.mjs` (`tools/` is not a path of this slice).
5. The north-star proof line "headless `cargo tree -p engine-game -e normal`
   has no gltf/engine-assets/image" is now stale for `engine-assets`: the
   game needs the seam on the headless leg. The headless graph stays
   std-only (no `gltf`, no `image`), and `engine-assets` is a std-only
   crate on its default features.

