# Fix lane: dead code in engine-core, engine-scene, engine-ui, engine-save

Date: 2026-10-21. Lane: the fix lane for `crates/core`, `crates/scene`, `crates/ui`, `crates/save`.
Method: the lead's method in `00-CORRECTIONS.md` section C, with the lead's own tool
`tools/unwired-code-gate.mjs` as the module scan.

Golden `d57b5c8ae3096614` did not move. No commit, no push, no `git add` from this lane.

## 1. Every candidate in my four crates, and what I did with it

The module scan is the lead's own tool: `node tools/unwired-code-gate.mjs crates --json`.
It lists 11 modules in my four crates with no caller outside their own crate. Not one of them is
dead: six are reachable only through their own `#[cfg(test)]` module, so by the lead's rule they
are test-only and I say so instead of deleting them. Five have a live caller.

| file | lines | why it was dead (or why not) | the grep proof | removed |
|---|---|---|---|---|
| `crates/scene/Cargo.toml` | 2 | the `math` feature: no crate asks for it and it gates no code in engine-scene | `git grep -n "features = .*math" -- crates/` = 0 hits; `git grep -n 'cfg(feature = "math")' -- crates/` = 0 hits; `git grep -n "math" -- crates/scene/Cargo.toml` = 0 hits after the cut | **yes** |
| `crates/ui/src/text.rs` | 4 | `Atlas::ATLAS_W` and `Atlas::ATLAS_H`: no path reaches them, not even a test | `git grep -rn "ATLAS_W\|ATLAS_H" -- crates/` = 0 Rust hits (only the C macros in `crates/geo/vendor/xatlas`) | **yes** |
| `crates/core/src/det_math.rs` | 438 | test-only, not dead. Parked on RC8: the doc says the HASH-LAW sin/cos lift waits for a consumer and a matrix row | `git grep -n "det_math" -- crates/` = 2 hits, both the `pub mod` line in `crates/core/src/lib.rs:19` | no |
| `crates/core/src/det_hash.rs` | 328 | test-only, not dead. Parked on RC25 (EVIDENCE row for the Fx shape) | `git grep -n "det_hash" -- crates/` = 2 hits, both in `crates/core/src/lib.rs:18` and the re-export at `:46` | no |
| `crates/core/src/sim_verify.rs` | 548 | test-only, not dead. Parked on RC24 (EVIDENCE row for madsim). It is a sim-test harness for other crates | `git grep -n "sim_verify" -- crates/` = 1 hit, the `pub mod` line at `crates/core/src/lib.rs:37` | no |
| `crates/core/src/pump.rs` | 301 | test-only, not dead. Parked on RC22 (the off-tick job pump) | `git grep -n "pump" -- crates/core/src` = 1 hit, the `pub mod` line at `crates/core/src/lib.rs:28` | no |
| `crates/core/src/rng_wyhash.rs` | 342 | test-only, not dead. Parked on RC28. `Rng::wy` is a production fn, but only `crates/core/src/rng.rs`'s own test module calls it | `git grep -n "rng_wyhash" -- crates/` = 5 hits, all in `crates/core/src/lib.rs` and `crates/core/src/rng.rs` | no |
| `crates/ui/src/tutorial.rs` | 746 | test-only, not dead. Parked on UIX5 and UIX12. The UIX12 row names the future caller: a fresh-profile guide | `git grep -n "tutorial" -- crates/` = 1 hit, the `pub mod` line at `crates/ui/src/lib.rs:28` | no |
| `crates/save/src/fallback.rs` | 561 | live. `save_generation` calls `fallback::backup_current` in production (`crates/save/src/lib.rs:254`) | `git grep -n "fallback::" -- crates/save/src` = 3 hits, one of them production | no |
| `crates/scene/src/pin.rs` | 224 | live. `crates/endpoint/src/scene_tools.rs:45` uses it, and the endpoint `mcp` build ships that binary | `git grep -n "engine_scene::pin" -- crates/` = 2 hits, one of them the call | no |
| `crates/ui/src/access.rs` | 505 | live. `crates/endpoint/src/play/play_session.rs:1320` calls `hud_palettes`, and `crates/ui/src/lib.rs:1039` calls it too | `git grep -n "engine_ui::access" -- crates/` = 6 hits, 3 of them production calls | no |
| `crates/ui/src/play_hud.rs` | 1,200 | live. `crates/ui/src/lib.rs:1445` builds a `PlayHud` from the overlay | `git grep -n "play_hud::" -- crates/ui/src` = 4 hits, all in `crates/ui/src/lib.rs` | no |
| `crates/ui/src/settings.rs` | 1,722 | live. `crates/ui/src/menus.rs:18` imports it | `git grep -n "crate::settings" -- crates/ui/src` = 1 hit | no |

Removed: 6 lines in 2 files. The two constants were public, so
`tools/api-break.baseline.json` still lists `text::Atlas::ATLAS_W` and `text::Atlas::ATLAS_H`.
That file records the whole workspace, so the lead re-blesses it once after the deletion wave
(`node tools/api-break.mjs --write`). `tools/` belongs to the speedup lane, so I did not touch it.
`api-break` is not in CI (`ci.yml` runs `workspace-check` and `rust-quality` only).

## 2. Feature flags in my four crates

| flag | who enabled it | action |
|---|---|---|
| `engine-core/test-util` | 16 crates: ai, anim, arena, assets, audio, combat, destruction, ecs, game, net, nn, perf, physics, render, world | keep, live |
| `engine-scene/json` | `crates/editor/Cargo.toml:70` | keep, live |
| `engine-scene/math` | nobody | removed |
| `default`, `kit`, `all`, `tls`, `window` | not declared in my four crates | nothing to do |

`engine-scene/math` was the only dead flag of the seven the lead listed in section C.

## 3. A red suite in my crate, fixed

`cargo test -p engine-ui --lib` was red at HEAD: 159 passed, 2 failed.
The cause is outside my crates: `engine-input` raised the canonical fighter cap to 32
(`15049ef3`), and two lobby tests in `crates/ui/src/lobby_ui.rs` still mirrored the old 16.

| test | what was stale | fix |
|---|---|---|
| `lobby_scale_sixteen_virtualized_with_cost_and_projection` | two windows of 8 rows covered the old 16 slots; the units and bytes numbers were the 16-slot ones | gather every window in order, and scale from the real slot count |
| `a_thousand_ghosts_never_outgrow_the_list_or_lock_a_real_host_out` | the host cap and the eviction count were hardcoded to 16 | take the cap from `MAX_PARTICIPANTS` and count the evictions from it |

The production code in `lobby_ui.rs` was already cap-generic. The test names stay as they are:
`docs/SPRINT-07-accepted-plan-rows.md` pins them in the UIX8 and UIX11 rows. The name still says
"sixteen", which is now stale, but renaming it would make those doc lines false.

## 4. Why I did not delete the six test-only modules

Every one of them is accepted, ticked board work with a named future consumer:

- `det_math` (RC8), `det_hash` (RC25), `sim_verify` (RC24), `pump` (RC22), `rng_wyhash` (RC28):
  the rows are in `docs/SPRINT-07-accepted-plan-rows.md` and three of them carry an EVIDENCE row in
  `docs/EVIDENCE.md` (`det_hash` for the Fx shape, `sim_verify` for madsim, `rng_wyhash` for wyhash).
- `tutorial` (UIX5, UIX12): the UIX12 row names the caller it waits for.

The lead's rule says a module used only from a `#[cfg(test)]` mod is test-only, not dead.
The compaction lane's own list in `docs/compaction-baseline.md` section 5 has no row for
`engine-core` or `engine-ui` either, so no ticked row turns false because of this lane.
These six need a caller, not a grave: each one is a board row waiting to be wired.

## 5. Test-only public items inside live modules (not deleted, listed for the board)

These have no production caller but a test calls them, so they are test-only:

`core/src/arena.rs:129 remaining_words`, `core/src/counter_rng.rs:92 for_island`,
`core/src/counter_rng.rs:97 for_chunk`, `core/src/det_math.rs:138 det_sin_cos`,
`core/src/sim_verify.rs:321 percentile_ns`, `core/src/time.rs:88 buffered_thirds`,
`core/src/time.rs:131 push_duration`, `scene/src/component.rs:841 to_versioned`,
`scene/src/prefab.rs:37 MUSEUM_V1_JSON`, `scene/src/prefab.rs:450 set_components`,
`scene/src/prefab.rs:515 reinstantiate_with_overrides`, `ui/src/access.rs:70 contrast_table`,
`ui/src/lib.rs:47 HUD_TEXT_CONTRAST`, `ui/src/lib.rs:579 upsert_dot`, `ui/src/lib.rs:1050 set_palette`,
`ui/src/lib.rs:1103 row_area`, `ui/src/lib.rs:1213 set_poise`, `ui/src/lobby_ui.rs:376 take_notice`,
`ui/src/lobby_ui.rs:435 see_host`, `ui/src/lobby_ui.rs:624 join_refused`,
`ui/src/locale.rs:102 format_fighter_count`, `ui/src/locale.rs:155 pseudo_into`,
`ui/src/play_hud.rs:343 ammo_fraction_permille`, `ui/src/play_hud.rs:382 hit_flash_visible`,
`ui/src/settings.rs:505 from_saved`, `ui/src/settings.rs:590 undo_staged`,
`ui/src/settings.rs:629 undo_applied`, `ui/src/tutorial.rs:343 is_skipped`,
`save/src/fallback.rs:97 has_slot`, `save/src/fallback.rs:199 load_slot`,
`save/src/persist.rs:117 migrate_spine`, `save/src/rollback.rs:164 latest_tick`,
`save/src/rollback.rs:198 rollback_to`.

Two of them are worth a board row, because the path they guard never runs in production:
`save/src/fallback.rs:199 load_slot` (the corrupt-save fallback reader is never called, so a
corrupt current file never falls back to the `.prev` generation) and
`save/src/rollback.rs:198 rollback_to` (the rollback operation is never triggered).

## 6. The proof commands and their one-line results

| command | result |
|---|---|
| `node tools/gh-check.mjs cargo test -p engine-scene --lib` (before the cut) | PASS: 78 passed, 0 failed, 1 ignored (run 39504482431) |
| `node tools/gh-check.mjs cargo test -p engine-scene --lib` (after the cut) | PASS: 78 passed, 0 failed, 1 ignored (runs 39510030123 and 39518746352) |
| `node tools/gh-check.mjs cargo test -p engine-ui --lib` (before, at HEAD) | FAIL: 159 passed, 2 failed (run 39507889731) |
| `node tools/gh-check.mjs cargo test -p engine-ui --lib` (after the fix) | PASS: 161 passed, 0 failed (runs 39509384886 and 39514439064) |
| `node tools/gh-check.mjs cargo test -p engine-core --lib` | PASS: 152 passed, 0 failed, 4 ignored (run 39510312503) |
| `node tools/gh-check.mjs cargo test -p engine-save --lib` | FAIL to build: `engine-observe` has 2 errors in `crates/observe/src/supervisor.rs` (`QuarantineEntry` is private). That file is another lane's work in flight; this lane did not touch `crates/save` |
| `node tools/gh-check.mjs cargo clippy -p engine-ui --all-targets -- -D warnings` | PASS (run 39511200540) |
| `node tools/gh-check.mjs cargo clippy -p engine-scene --all-targets -- -D warnings` | PASS (run 39511566383) |
| `node tools/gh-check.mjs cargo clippy -p engine-core --all-targets -- -D warnings` | PASS (run 39506513183) |
| `node tools/unwired-code-gate.mjs crates --json` | 11 modules with no outside caller in my four crates, 0 of them dead (section 4) |
| `node tools/rust-quality.mjs` (from the repo root) | 12 ratchet regressions, none in engine-core, engine-scene, engine-ui or engine-save |
| `node tools/workspace-check.mjs` | FAIL on other crates only: engine-arena, engine-assets, engine-physfx and the genre ledger |
| `cargo run --locked --release -- --verify` | PASS: `ticks=600 final_hash=d57b5c8ae3096614 ... verified=true` (one run, 48 min of build). A second run could not build: `engine-physics` has 13 compile errors because the physics lane is mid-refactor (`crates/physics/src/cloth.rs` is deleted and `crates/physics/src/cloth/` is new in the tree). Not this lane's code |

Note on the golden: it did not move. The root package `engine2040` links `engine-ui` through
`engine-render` (`crates/render/Cargo.toml:44`), and the only change there is two constants that no
code reads plus a `#[cfg(test)]` module, which a release build never compiles. `engine-scene` is not
in the golden's chain at all: `engine-ecs` takes it only through the optional `registry` feature
(`crates/ecs/Cargo.toml:15`), and no crate enables that feature. The removed feature flag gated no
code, so no build changed.

One warning for the lead: the shared tree was checked out under this lane once (another lane's
tree-wide checkout), and the three edited files lost their changes. They were re-applied and the
proofs above were run again on the re-applied tree. The lead should commit these paths soon.
