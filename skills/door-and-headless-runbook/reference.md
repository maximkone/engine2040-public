# Door and headless runbook — reference

Deep detail moved out of `SKILL.md`: the MCP door walkthrough, the arena receipt lines and view laws, the scene report, scoring, and the night GPU hazard. Read the part you need; `SKILL.md` holds the run lines.

## Stale copy check (run first)

`.opencode/bin/e2040-mcp.exe` is a copy from `tools/mcp-install.mjs` and is gitignored. On 2026-10-21 it was dated 2026-09-26 and served 20 tools with no `api.load`, `api.call`, `game.build`, `level.build`, `scene.next` or `scene.pick`. It also sent 64-bit hashes as JSON numbers, so a JS client rounds the approval and every `tx.apply` fails with `forged_approval`. Before you build anything, check that the tool list from `rpc.discover` shows `api.load`, `game.build` and `level.build`. If it shows 20 tools, report that the copy is stale and stop. Current source sends those hashes as decimal strings (`crates/endpoint/src/mcp_dto.rs`, `u64_text`).

## Build a small game through the door (verified 2026-10-21, `target\release\e2040-mcp.exe`)

The door places props from a fixed catalog. It does not build a game. Catalog kinds (`crates/endpoint/src/build_level.rs::KINDS`): `altar banner barrel bench brazier crate pillar post stand torch`. There is no player, enemy, goal or floor kind; walls come from the room shell. A brief is one directive per line: `prop <kind> <name>`, `want <name> <relation> <target> [<lo>,<hi>]?` (relations `next_to facing inside on_top_of supports`; targets `wall:north|south|east|west`, `object:<name>` or `point:<x_cm>,<z_cm>`), or `spawn <name>`. English fails with `level_refused: ... unknown directive`.

Edits do not survive a restart. Run the whole sequence in one process:

1. `scene.read` `{"limit":64,"cursor":0}`. Note `revision` R.
2. `level.build` `{"brief":"prop crate c1\nspawn c1","seed":2040,"expected_revision":R,"idempotency_key":8}`. Three props with two `want` lines were refused as `spatial_rejected` (`overlap`) and the message named no pair. Start with one prop and add one at a time.
3. `scene.read` again. Its `revision` (R1) is the one to pin. Poses from `scene.read` are metres; `scene.brief` and `scene.query` report centimetres.
4. `play.run` `{"action":"start","ticks":120,"fighters":2,"half_extent_m":10,"seed":2040,"scene_rev":R1}`, then `{"action":"poll","job_id":<id>}`. Without `scene_rev`: `unpinned_run`. At revision 0: `empty_arena`.
5. `play.step` `{"seed":2040,"upto_tick":30,"width":480,"height":270}` returns an image. The default frame is 96x64, too small to read. `play.step` takes no `scene_rev`; the frame checked was the default two-disc duel, so do not claim your props are in it until you have looked.
6. `tx.undo` `{}` reverts step 2 as a new revision.

Other verbs, verified:

- `game.build` `{"brief":"prop crate c1\nprop barrel b1\nspawn c1\nwant b1 next_to object:c1","seed":2040}` returned `verdict: pass` and `package_bytes` only. The package is not returned, so there is nothing to open or play. Read-only.
- `sim.build_encounter` `{"brief":"one enemy guards the goal","seed":2040}` returns `solved`, `plan_id`, `rung` and no geometry.
- No save verb exists. The project folder held no files after edits, and a fresh process starts at revision 0.

## Receipt lines, in print order

```
arena smoke: ticks=600 frames=550 alpha=0.606 hud_cells=63 ... fight_draws=16
arena sim: ticks=600 hash=<16 hex>
arena frame-split: sim=..ms present=..ms fighters=..ms ... interval_ms p50=13.0 p99=1382.9 ...
arena framing: frames=550 fighters=4 seen=4 empty=0 fewest_in_view=4 tinted_share=0.00
arena view: frames=550 sim_ticks=0.9..599.6 bodies=2200 offscreen_share=0.01 nonfinite=0 error_parts=0 sunk_cm_max=14.2 idle_float_cm_max=3.7 pops=180 pop_acts=attack:35/idle:38/run:107 pop_cm_max=107.6 eye_in_body=0 eye_below_ground=0 closest_cm=798 cover_max=0.02 foot_slide_cm=0.1 planted_feet=65 ragdolls=3
arena break: breaks=15 debris=2 awake=0
physfx: bodies=2 contacts=0 us=0
```

A real 4-fighter, 600-tick log of 2026-10-17; yours will differ. The `arena sim` hash is the frozen sim hash for that tick count: pausing or a different frame rate does not move it.

### arena view (`crates/game/src/view_truth.rs`)

Every drawn body as numbers from the CPU skin. Units: centimeters in the line, sim ticks for time.

| field | reads as | red when |
|---|---|---|
| `frames`, `sim_ticks=a..b` | frames folded and the tick span shown | fight must start by tick 30 and span at least 60 ticks |
| `bodies`, `offscreen_share` | bodies drawn in total; share with no on-screen rect | share high means the camera lost them |
| `nonfinite`, `error_parts` | NaN or infinite vertices; parts drawn with the skin error material | any above 0 |
| `sunk_cm_max` | deepest a body sank into the sand | above 3 (floor in `tools/product-bar.json`); target under 2 |
| `idle_float_cm_max` | most an idle body floated above the sand | trend only |
| `pops`, `pop_acts`, `pop_cm_max` | joints that missed their constant-velocity guess by more than 10 cm in one frame (a clip snap); by act | target 0; `run` pops come from the run clip phase |
| `eye_in_body`, `eye_below_ground` | frames with the camera inside a body or under the sand; in the first person (`--legionary`) also frames where the near plane cuts the wearer's own geometry, own geometry covers over 30 percent of the picture, or sits on the screen centre for more than 36 frames in a row (`arena pov:` line after this one: `near_cut_frames`, `cover_over_frames`, `cover_max`, `centre_run_max`) | any above 0 |
| `closest_cm`, `cover_max` | nearest body to the eye; biggest picture share (0..1) | trend only |
| `foot_slide_cm`, `planted_feet` | p95 slide of a planted foot off its spot, over this many foot-frames | above a few cm means a skating foot; 0 `planted_feet` means no proof |
| `ragdolls` | most bodies drawn from a ragdoll in one frame | 0 after a knockdown means the ragdoll is not wired |

Ragdolled bodies are not judged for pops. No `arena view:` line at all means the view truth did not fold (fighter look failed to bind).

### break and physfx

- `arena break: breaks=<n> debris=<n> awake=<n>`: breaks = authority `BreakEvent`s the window saw; debris = debris bodies alive; awake = bodies still moving. `awake=0` means the pile settled. `breaks=0` is normal when nothing hit the crate in a short run. No `arena break` line: the picture is off (`--no-break-look`) or it dropped itself and printed `arena notice: break picture dropped: <why>`.
- `physfx: bodies=<n> contacts=<n> us=<n>`: the float physics layer (debris, ragdolls) at its tier. It never feeds the hash.
- Both come from `crates/game/src/break_window.rs` and `crates/physfx/src/types.rs`.

### No window: the same view line

`FighterLook::view_run(seed, ticks, fighters)` folds a bout into one `arena view:` line with no GPU. Test: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib view_run` (runs on the PC). Pop and slide proofs on real bodies: `clip_switches_do_not_pop_in_a_real_duel` and `planted_feet_hold_their_spot_in_a_real_duel` in `crates/game/src/fighter_look_proofs.rs`. The door returns it as `view.laws`, and `view.scene` carries it beside the geometry.

### The spatial scene report and the render steer

The view line counts breaches; the scene report says WHERE things are. `view.scene` returns one JSON value plus a readable text plus a findings list for the live view: every fighter, prop and spear with world place, bounds, screen rect and cover, floor contact (grounded, support, ankle heights), which hand holds what (parts skinned rigidly to a palm), the bone a stuck spear's tip is nearest, what hides each thing from the camera (rays against the posed triangles) and by how much, sun lit and shaded shares, whether the sun shadow map holds the thing and where its shadow lands on the sand, the camera, the renderer's pass switches and the view laws. Findings are plain sentences (`fighter_1 is 62% hidden behind prop_2_pillar`, `fighter_0 is armed with gladius but no hand holds anything`, `the forward light and the sky sun point 11 degrees apart`).

- Windowless: `view.scene` with no seat plays the session's bout (empty tape) in `arena.exe --scene-report <seed> <ticks> <fighters> [--showcase] [--shot hero|wide|action]` and reports its last frame; the laws line equals `view.laws`'s. Build once: `cargo build --release -p engine-game --features app --bin arena`.
- Live: `seat.open` (give it many `frames`), `view.steer hold` to freeze the sim on its tick (an idle seat floods the socket with tick lines otherwise), then `view.scene`, `view.steer shot top|side|low|front|sun|close:N|hero|wide|action`, `view.steer pose`, `view.steer pass sun_shadow off`, `view.steer capture` (PNG under `target/scene-captures/`, then `node tools/frame-audit.mjs` reads it), `view.steer undo`, `view.steer resume`. The seat lines are `scene [pixels]` and `steer <op> ..`; the window answers `scene <json>` and `steer <json>` on its next frame (`crates/game/src/scene_desk.rs`, `agent_seat.rs`).
- No door: `arena.exe --showcase --frames 700 --exit --scene-at 400,600` prints `arena scene: <json>` and the text as `arena scene| ...` lines at those ticks. One window at a time: `node tools/arena-run.mjs -- <args>`.
- Code: `crates/game/src/scene_report.rs` (measure and emit), `fighter_look_scene.rs` (posed fighters, headless run), `bin/arena_frame/scene.rs` (window desk), `crates/render/src/present/gpu_present/pass_switch.rs` (pass registry), `crates/endpoint/src/packs/view/scene.rs` (verbs). Proof: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib -- scene_` and `cargo test -p engine-endpoint --features mcp --lib -- view::`.

## Score it

`tools/product-check.mjs` reads these lines (`parseView`, `parseReceipt`) against `tools/product-bar.json`; run it only with `--no-build` (it writes its report and captures, nothing else). `node --test tools/product-check.test.mjs` shows the parse rules. Report: the command, the exit code, the line you read (quoted), and the PNG you opened.

## Night GPU runs

A GPU run left alone overnight can strand the external display. The panel on the NVIDIA output goes `Off Line` after the next resume and the monitor shows its no-signal snow while the Intel panel comes back; no TDR is logged, so the Event log looks clean and the topology switch does nothing. Seen 2026-10-24: a GPU report written 06:53, the monitor re-arrived 06:57, firmware S3 resume 06:59, second screen `Off Line` from then on (`Win32_DesktopMonitor` availability 8).

Recover in this order: `Win+Ctrl+Shift+B`, then `Win+P` -> Extend; reseat the HDMI at the laptop; power-cycle the monitor and press its Source button; reboot if the output stays dark. Before a long GPU run: keep the lid open on AC, set the external display to never sleep, and prefer `--headless` for any row that does not need a window.
