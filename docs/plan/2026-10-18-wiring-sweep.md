# Wiring sweep — 2026-10-18

Read-only sweep of the crates changed today (`endpoint`, `playtest`, `render`,
`game`, `net`, `assets`, `web`): the pub items nothing outside their own file
or tests calls, ranked by what breaks while they stay unwired. Nothing here is
wired and no code changed; each entry names the smallest real caller that would
close it.

Method and evidence:

- `node tools/workspace-check.mjs` (report-only wiring line): `islands: 5852`.
  `tools/wiring-scan.baseline.json` pins `total: 5317`, so the official number
  has risen ~535 since the pin (report-only, no gate fails).
- The 7 crates carry 2910 of those islands (endpoint 620, render 1015, game 471,
  playtest 276, net 267, assets 221, web 40). 1825 are `fn`/`struct`/`enum`/
  `trait`; the rest are `const` (mostly table rows, low value).
- Value filter: an item that the name-scan calls an island but whose *only*
  mention is its own definition line is a dead entry point (nothing in
  production, not even its own file, touches it). Recomputed over the 7 crates:
  445 such items. Items a sibling in the same file calls are helpers, not gaps,
  and are excluded (see "Not worth wiring").
- Caveat on the official count: `wiring-scan.mjs` cuts a file's production text
  at the first literal `#[cfg(test)]`. A doc comment that *contains* that string
  truncates the whole file early and inflates its island count — e.g.
  `crates/endpoint/src/tool_capabilities.rs:12` (a doc comment) hides its own
  `check_args_ops` call site at `:791`, so `guards.rs:264` is listed as an
  island though `check_shape` really calls it. Recomputed with a cut only at a
  real `#[cfg(test)] mod` opener, the total is 5853 (distribution shifts), and
  every entry below was re-checked by `Grep` for a production caller.
- `node tools/ticked-rows-audit.mjs` runs (report-only): ~40 ticked rows name a
  path, crate or test filter that is gone (mostly from the 2026-10-16 rot and
  backup commits, plus `tests/` → `tests/it/` moves). Unrelated to this sweep
  but the same "row claims something the tree does not have" disease.

## Ranked: the 20 most valuable unwired pub items

Order is by what the gap costs: a dead subsystem first, a lost resilience path
or gate next, a dead readout last. "Caller" is the smallest real place that
would use it today.

1. **`crates/assets/src/handle.rs:473` `AssetStore::request_load`**
   (+ `mark_loaded:490`, `mark_failed:526`, `ready_handles:365`,
   `remaining_assets:264`, `is_loaded:80`) — the generational-handle streaming
   store, driven only by `handle.rs` tests. The module header already names the
   gap: "planned, no caller yet (verified 2026-10-16) … Planned caller:
   `crates/game/src/arena_kit.rs`". Whole load-state machine (NotLoaded →
   Loading → Loaded/Failed) is dead, so nothing stages or waits on a kit asset.
   Caller: `crates/game/src/arena_kit.rs` at kit import — one handle per kit
   file, then read the ready payloads at init.

2. **`crates/endpoint/src/transport.rs:37` `token_ok`** (+ `check_caps:47`,
   `timeout_for:52`, and `validate:24`) — the MCP door's token auth, payload
   caps and per-kind timeouts. Only `crates/endpoint/tests/it/{session_chaos,
   redteam,lib_wiring_vectors}.rs` call them; the live door never does
   (`is_loopback` is the one member in use, from the provider crate). So a real
   frame reaching the server is neither shape-checked, token-checked, capped nor
   timed. Caller: `McpServer::call_tool` at `crates/endpoint/src/mcp.rs:1271`
   (admit the frame with `validate` + `token_ok` + `check_caps` before dispatch).

3. **`crates/render/src/uploads.rs:96` `StagingBelt`** (+ `live_uploads:134`,
   `invalidate_all:236`) — the bounded in-flight staging belt; the harness only
   builds it in `uploads.rs` tests. The presenter writes every buffer with a
   bare `queue.write_buffer` and has no belt, no retire-after-submit discipline
   and no invalidation on surface/device loss. The module header names the hook:
   "invalidate_all is the AH2 heal hook (resize, surface-loss, device-loss)".
   Caller: `Presenter::resize` (`presenter.rs:1417`) and
   `reconfigure_surface` (`present/gpu_present/submit.rs:1472`) for
   `invalidate_all`, and the present path for issue/advance.

4. **`crates/render/src/heal.rs:321` `RenderHeal::hold_last_good`** (+ the whole
   `RenderHeal` type, `report_device_lost:358`, `report_present_error:382`) —
   device-loss/surface-loss recovery: backoff, quality fallback, last-good
   generation hold. `crates/render/src/present/surface.rs:252,272` even name
   `heal::RenderHeal` as the consumer of its faults, yet nothing constructs a
   `RenderHeal` outside `heal.rs` tests: the renderer degrades with no healing.
   Caller: the windowed frame loop `crates/game/src/windowed.rs` (feed surface
   events, hold the last-good frame while recovering).

5. **`crates/net/src/replay.rs:743` `Theater::step_forward`** (+ `step_back:753`,
   `seek_keyframe:783`, `is_playing:722`) — play/pause/frame-step over a
   recorded tape. `HeadlessServer::theater()` (`net/src/dedicated.rs:579`)
   documents "play, pause and frame-step", but no operator surface drives it:
   the net bins never touch `Theater`. Caller: `crates/net/src/bin/net_serve.rs`
   at match end (or a replay viewer) — step/seek the theater the server builds.

6. **`crates/playtest/src/highlights.rs:381` `cut_reel`** (+
   `tournament_tapes:439`, `write_reel:483`) — the SHOW3 highlight reel: rank
   tournament moments and cut a shot list inside a tick budget. Header names the
   consumer ("devlog and trailer cuts") and SHOW1 capture replays the reel;
   nothing calls the cutter, so no reel is ever produced. Caller: a door verb in
   `crates/endpoint/src/packs/gym.rs` (a `gym.reel` verb), or the
   `crates/game/src/bin/arena.rs` capture path.

7. **`crates/render/src/present/gpu_present/presenter.rs:1852` `evict_texture`**
   (+ `evict_material_set:1970`, `material_set_keys:1977`) — manual VRAM drops
   for a texture or a material set. Only resident-insert capacity eviction runs;
   the explicit eviction API is test-only, so the texture cap is the only
   pressure valve and per-frame texture residency can grow to it with no rule.
   Caller: `Presenter::upload_texture` (`presenter.rs:1729`) when
   `texture_capacity()` (`:2144`) is reached — evict by the same policy the
   material layer names.

8. **`crates/game/src/modes.rs:340` `ModeManager`** (+ `try_swap:434`,
   `apply_pending:438`) — staged display-mode/fullscreen swap with last-good
   fallback. `crates/game/src/bin/arena.rs:48` imports `DisplayMode`,
   `SwapRequest`, `rank_modes` but never builds a `ModeManager`, so the window
   can never change mode through the checked path. Caller:
   `crates/game/src/bin/arena.rs` fullscreen/resize handler — `stage` the swap,
   `apply_pending` at a safe point.

9. **`crates/net/src/rollback.rs:431` `Rollback::verify_full_window`** (+
   `enable_sparse_saving:417`, `checksum_at:496`, `checksum_log:501`) — the
   desync window check over the rollback history. Used only by `rollback.rs`
   tests; the live session advances without ever verifying the window it keeps.
   Caller: `Rollback::advance_frame` (`rollback.rs:626`) — the natural moment to
   compare checkpoints (or a `net_serve` verify mode).

10. **`crates/assets/src/gltf.rs:181` `validate_gltf`** (+ the shape validators
    at `:215/231/248/269`) — the glTF/GLB gate (magic, framing, version, joint
    and weight shapes). Only the `assets` integration tests call it; the intake
    path parses with the `gltf` crate without this check. Caller:
    `crates/assets/src/import.rs:220` `import` (its `.glb`/`.gltf` branch, next
    to the existing `gate_results`) or `mesh_import::import_gltf_bytes`
    (`mesh_import.rs:172`).

11. **`crates/endpoint/src/discover.rs:1129` `resolve_tool`** (+
    `suggest_tool:1120`) — exact tool-name resolution with a fuzzy suggestion
    (`scene.red` → `scene.read`), "one round trip saved". The island metric hid
    it behind the file's own `suggest_tool` caller; still nothing outside
    `discover.rs` uses either. Caller: the unknown-tool arm of
    `McpServer::call_tool` (`crates/endpoint/src/mcp.rs:1271`) before it refuses.

12. **`crates/endpoint/src/guards.rs:228` `check_dry_run_id`** — the `rcpt_`
    prefix/shape/length gate for dry-run receipt ids in the write path. Its
    siblings `check_args_ops` and `check_policy` are called from
    `tool_capabilities.rs`; this one is not, so a malformed dry-run id is not
    refused at the door. Caller: `check_shape` (`tool_capabilities.rs:790`) or
    the `tx.preview`/`tx.apply` arg check beside it.

13. **`crates/endpoint/src/insight/index.rs:682` `Index::refresh_from_dir`**
    (+ `rebuild_one:619`, `add_asset:650`, `add_scene_ref:666`) — incremental
    insight-index rebuild from a changed file. Only `index.rs` tests drive it, so
    the API/asset/scene index never refreshes from disk after boot. Caller: the
    door's post-write step in `crates/endpoint/src/mcp.rs` (or `discover.rs`)
    after a `tx.apply` lands a file on disk.

14. **`crates/playtest/src/eval/scorecard.rs:327` `run_scorecard`** (+
    `to_markdown:159`, `from_markdown:223`) — the per-task eval scorecard and its
    markdown round trip. No binary or pack runs a scorecard, so eval scores are
    only ever asserted inside the crate's tests. Caller: a door verb in
    `crates/endpoint/src/packs/gym.rs`, or the `engine-playtest` eval driver
    `crates/playtest/src/eval.rs`.

15. **`crates/game/src/lib.rs:1051` `match_winner`** (+ `close_round:1037`,
    `reset_for_next_round:1056`, `load_progress:1681`) — the round/match-finish
    flow. `rounds.rs` tests read `match_winner`; the app never does, so a match
    can run past its end with no winner surfaced. Caller: the app's per-tick
    round step in `crates/game/src/lib.rs` / `crates/game/src/windowed.rs`.

16. **`crates/render/src/present/gpu_present/quality_apply.rs:36`
    `Presenter::quality_knobs`** (+ `shadow_map_edge:42`, `particle_slots:48`,
    `decal_slots:54`) — the readback of what `apply_quality` actually stored.
    `apply_quality` runs (web `arena.rs:142,540`), but nothing outside tests
    reads the applied values, so the printed tier table is built from
    `quality::knobs(tier)` (`crates/web/src/arena.rs:799` `tier_facts`) and can
    disagree with the live frame. Caller: `tier_facts` in
    `crates/web/src/arena.rs:799` — read `presenter.quality_knobs()` and the
    three pool/shadow getters.

17. **`crates/render/src/present/look.rs:328`
    `hold_clip_for_establishing_view`** (+ `clip_hold::hold_camera`,
    `hold_frame`, `hold_motion`) — the WIN1/ED1 hero motion-hold clip for the
    twin establishing view. `present/look.rs` is the only caller of
    `clip_hold`, and nothing calls it, so the preset clip is never played.
    Caller: the twin establishing-view capture in
    `crates/render/src/present/gpu_present/capture.rs` (or
    `crates/game/src/staged_fight_frames.rs`).

18. **`crates/render/src/baked_gi.rs:669` `gpu::upload_lightmap`** (+
    `upload_lightmap` into the presenter, `probe_seal_bytes_into:438`) — upload
    a baked-GI lightmap to the GPU. Only `baked_gi.rs` tests call it, so a baked
    lightmap is computed but never resident. Caller:
    `Presenter` baked-GI init in
    `crates/render/src/present/gpu_present/presenter.rs`.

19. **`crates/game/src/fps_view.rs:40` `apply_mouse`** (+ the sibling handle
    path around `eye_for_fighter:208`, `camera_for_eye:263`) — first-person
    mouse look feeding the eye pose. `render_eye` is wired, but the look-in
    path is not, so FV rows that claim a look channel are dead. Caller: the
    game input tick in `crates/game/src/lib.rs` (or
    `crates/game/src/camera_feed.rs`) where per-tick input is applied.

20. **`crates/web/src/input.rs:268` `InputState::pointer_locked`** — the
    pointer-lock state the `browser.rs` `pointerlockchange` listener already
    maintains (`browser.rs:316` `set_pointer_lock`). Nothing reads it, so the
    page cannot suppress look (or show a "click to look" hint) when the pointer
    is free. Caller: the `mousemove` listener in
    `crates/web/src/browser.rs:300` — guard the look delta on `pointer_locked()`.

## Next ten (same class, lower cost to leave)

- `crates/web/src/netquery.rs:151` `NetQuery` (+ `parse_join:119`) — join
  `?server=`/`?token=` URL query; nothing parses it, so a page cannot join from
  a link. Caller: `crates/web/src/arena.rs` startup.
- `crates/net/src/webtransport/cert.rs:191` `cert_pem` (+ `key_pem:202`,
  `leaf_der:161`) — the WebTransport certificate is never served. Caller: the
  webtransport server init in `crates/net/src/`.
- `crates/endpoint/src/plan_goals.rs:369` `plan_encounter` (+
  `plan_staged_encounter:420`) — encounter planner for goal-directed sessions.
  Caller: `crates/endpoint/src/packs/play.rs`.
- `crates/endpoint/src/sandbox/manifest.rs:70` `manifest_digest` (+
  `sandbox/scan.rs:115` `scan_digest`) — the sandbox receipts' digests are never
  computed. Caller: `sandbox::check_before_run` (`sandbox.rs:40`).
- `crates/endpoint/src/guards.rs:287` `check_policy` is called, but
  `crates/endpoint/src/packs/web.rs:296` `disallow_web` / `:302`
  `set_web_runner` are not — the web pack's runner gate is dead. Caller:
  `crates/endpoint/src/packs/web.rs` pack build.
- `crates/render/src/shadow_cascade.rs:349` `spot_shadow_pcf` (+
  `cube_shadow_pcf:391`, `cascade_blend_weight:425`) — PCF helpers the live
  shadow pass does not use. Caller: the shadow pass in
  `crates/render/src/present/gpu_present/`.
- `crates/game/src/ladder.rs:362` `rung_beat` (+ `rank_name:280`,
  `is_beat_chart_clean:400`) — ladder progress/rank readout. Caller:
  `crates/game/src/bin/arena.rs` result screen.
- `crates/net/src/quic.rs:290` `send_state` (+ `unrouted_refusals:170`) — QUIC
  link state/refusal counters never sampled. Caller: `net_serve` status line.
- `crates/playtest/src/frame_check.rs:777` `check_pose_sequence` (+
  `frozen_gate:651`, `capture_png:887`) — pose-sequence QA gate. Caller:
  `crates/endpoint/src/packs/verify.rs`.
- `crates/render/src/gtao.rs:84` `set_gtao_live_half_enabled` (+
  `gtao_pass_plan:1170`) — the AO half-res toggle is never flipped at runtime.
  Caller: `Presenter::apply_quality` (`quality_apply.rs:21`).

## Not worth wiring (island, but nothing is missing)

- `crates/render/src/cache.rs:69` `toolchain_hash` — flagged by the diff gate
  today (VR5), but it *is* called, in-file by `PipelineKey::new` (`cache.rs:176`);
  its value reaches the cache key through that. A same-file helper is not a gap.
  Same shape: `assets/src/bc.rs:161` `encoded_len`, `render/src/cache.rs:549`
  `diagnostic_count`, `endpoint/src/door_dispatch.rs:41` `audit_len` and the
  other `audit_*` test accessors.
- `crates/playtest/src/bench_mirror.rs:70` `run_bench_mirror` — the module header
  says its consumer *is* the test (`cargo test -p engine-playtest --lib
  bench_mirror --nocapture` prints the table). Test-driven by design.
- `crates/assets/src/handle.rs` `is_loaded`/`ready_handles` are listed in entry 1
  as one subsystem; if only the whole store is wired, wire it together.

## What to do next

This is a report, not a board slice. The cheapest first cut is entry 2
(door gates) plus entry 12, then entry 1 (the documented `arena_kit` caller),
then entry 8 (display modes) which the deleted-code triage already ordered. A
follow-up could fix `wiring-scan.mjs`'s `#[cfg(test)]` cut so the official
island count stops over-reporting and the baseline can be re-pinned honestly.
