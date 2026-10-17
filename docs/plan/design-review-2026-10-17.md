# Design review 2026-10-17: what in the design hurts a heavy realistic game on Windows and in a browser

Owner order 2026-10-17: "improve design wise, make it smarter". Read-only review by one Sonnet session; it edits no code and no board. Question: if a game author (or an AI) builds a heavy realistic game on Windows AND in a browser (WebGPU plus wasm) with multiplayer, destruction, ragdoll and dismemberment, what in the engine DESIGN will hurt them or slow us in the next month?

Method: read `docs/ARCHITECTURE.md`, `HASH-LAW.md`, `CRATE-MAP.md`, `plan/web-and-realism.md`, `plan/finish-line.md`, `AGENTS.md`, the edge table in `tools/workspace-check.mjs`, the crate manifests, then read the code named in each finding. Numbers come from commands run on 2026-10-17 on the shared tree (`wc`, `grep`, `node tools/wasm-gate.mjs`, two throwaway node scripts over the manifests). No cargo build was run. "Not checked" marks what I did not read.

## Size of the thing (measured)

- 38 crates plus the root package. Rust in `crates/*/src`: about 364,000 production lines and 263,000 lines of inline `#[cfg(test)]` modules; 38,000 more in `crates/*/tests/`. 8,050 `#[test]` fns. 1,015 tracked `.rs` files, 674,526 lines in all.
- Biggest crates, production lines: render 55.0k (plus 43.2k inline tests), endpoint 37.7k, game 31.0k, assets 22.3k, editor 21.6k, playtest 19.7k, net 18.8k, anim 17.2k.
- Pub surface (`pub fn/struct/enum/trait/const/type/mod` at line start): render 2,457 in 61 `pub mod`; endpoint 1,506; game 1,190 in 39 `pub mod`; assets 796; net 989.
- Cargo graph: no cycle, including optional edges (script `scratchpad/graph.mjs`, strongly connected components over all `[dependencies]` edges: none). Fan-in: core 31, arena 9, input 7, anim 7, observe 7, physics 6, combat 6, ai 6. The layering is better than the prose fears. The pain is elsewhere: the layers are not what the running game uses.
- Tick graph of `engine-arena` (default features): ai, anim, combat, core, destruction, ecs, input, nn, physics (9 crates). `engine-game` default closure: 19 crates. `engine-web` default closure: 14 crates, reached only through `engine-proof`.

## Ranked findings (payoff over cost, best first)

### 1. The network plays a different match than the window (DSN1, size M)

Evidence.
- `crates/net/src/session.rs:343`, `:461-462`, `:620`: `Server` and `Client` build `LiveMatch::new(&config)`.
- `crates/arena/src/live.rs:164-166`: `LiveMatch::new` is `with_rules(config, ArenaRules::V1)`, and `with_rules` is `with_rules_world(.., MatchWorld::empty())` (`:170-172`). So the net match is rules v1 on open sand.
- The window plays `App::with_world(config, kit_match_world_or_empty())?.into_duel()` (`crates/game/src/bin/arena.rs:937`, `crates/game/src/windowed.rs:1227`): rules v2 (balance, collision, stagger memory, limb pool) on the kit walls.
- `crates/game/src/lib.rs:578-582` says it outright: "LAN play keeps replaying v1 on both sides". `HASH-LAW.md:76-84`: v1 is the default of every runner, tape, snapshot.
- `ArenaConfig` carries neither rules nor world (`live.rs:164-191` takes them as separate args). `MatchSnapshot` (`crates/arena/src/lib.rs:565`) is not checked for rules or world identity.

Why it hurts. Every heavy feature the owner asked for (stagger, knockdown, limb loss, kit collision) lives in v2 plus the kit world. The multiplayer path cannot run any of it, so G8 (the duel on LAN and internet, one hash) would prove a game nobody plays. The lobby already compares a rules digest (`online.rs:474-482`, `lobby.rs:87-146`), but the sim is not built from that value. The longer this waits, the more v2 state (limbs, bonded walls) must be added to the wire later.

Smallest fix. One value `MatchSpec { config, rules, world_digest }` in `engine-arena` (a new `spec.rs`, about 120 lines). `LiveMatch::from_spec`. `net::Server::new` and `Client::new` take a `MatchSpec`; the lobby build identity hashes it (rules version plus world digest plus content digest, see DSN5). The default spec stays v1 plus empty world, byte-identical to today. Files: `crates/arena/src/spec.rs` (new), `crates/arena/src/live.rs`, `crates/net/src/session.rs`, `dedicated.rs`, `lobby.rs`, `crates/game/src/netgame.rs`.

Golden risk: none for `GOLDEN_600` (it never runs arena). v1 arena digests stay as they are because the default spec is v1. A new v2 net fixture gets its own version tag.

Board row:
- [ ] DSN1 net: the networked match is the played match — add `MatchSpec { config, rules, world_digest }` to `engine-arena`, build `LiveMatch` from it in `engine-net` `Server` and `Client` (today `LiveMatch::new` = rules v1 on an empty world: `crates/net/src/session.rs:343,461`), hash it into the lobby build identity, and make `engine-game`'s LAN host pass the same spec the window plays (`into_duel` plus kit world). Diff: good. Tier A. Size M. Owns `crates/arena/src/spec.rs`, `crates/arena/src/live.rs` (constructor only), `crates/net/src/{session,dedicated,lobby}.rs`, `crates/game/src/netgame.rs`. Accept: `cargo test -p engine-net --lib spec_` (a server and a client built from one v2 kit spec end a 600-tick bout on equal digests; a client with another rules version or world digest is refused in the lobby with both digests named); `cargo test -p engine-arena --lib spec_`; the default spec's checkpoints equal today's v1 bytes. F2P: net bouts run v1 on open sand whatever the window plays. P2P: `GOLDEN_600=d57b5c8ae3096614`, the HS2 matrix, v1 arena checkpoints, `cargo test -p engine-net --lib` unchanged. needs: none.

### 2. The arena match has no cross-architecture pin (DSN2, size S)

Evidence.
- `HASH-LAW.md:55-60`: "The multiplayer arena needs tier 3 for the arena simulation".
- `crates/proof/src/matrix.rs:1-11`: a matrix row is the legacy golden tape plus the BX28 cut corpus. `.github/workflows/golden-matrix.yml:70,75,84` runs `engine2040 --record`, `matrix_line`, `ragdoll_line`. `finish-line.md:17` (G6) lists the same two things.
- `grep` for a 16-hex-digit pin in `crates/arena/src`, `crates/game/src`, `crates/proof/src` finds none for an arena match (only `arena/src/live.rs`, `proof/src/lib.rs` and `proof/src/m1.rs` contain 16-digit hex literals; what they pin was not read). `crates/arena` has no `tests/` dir. Not checked: pins inside `tools/*.mjs` or receipts in docs.
- `HASH-LAW.md:83-84`: the golden fixture "never runs arena or combat".

Why it hurts. The one claim the multiplayer and the browser both rest on (rules v2 arena equal on x86-64, aarch64, wasm32) is unproven. The legacy 2D fixture is a poor stand-in: it does not touch `controller.rs` (115 f32 mentions, used by arena at 4 sites), the f32 pose, balance, collision, the limb pool or bonded walls. A libm-style or fused-multiply-add drift in those would only show up as a desync in the field.

Smallest fix. A second bin in `engine-proof`, feature `arena`: `arena_line` runs one fixed 8-slot v2 kit match for 1200 ticks with a scripted tape that forces a block, a parry, a knockdown, a cut and a wall break, and prints the checkpoint digest every 100 ticks. Add it as a fifth tape of `golden-matrix.yml` (the compare job already takes any number of rows) and as a wasm32-wasip1 leg. Files: `crates/proof/src/arena_matrix.rs`, `crates/proof/src/bin/arena_line.rs`, `golden-matrix.yml`, `tools/golden-matrix.mjs` only if a label is missing.

Golden risk: none (new fixture, own version). If it diverges the first bad tick is the finding.

Board row:
- [ ] DSN2 proof: pin the arena match across architectures — an `arena_line` bin in `engine-proof` (feature `arena`) plays one fixed 8-slot rules-v2 kit match of 1200 ticks with a tape that forces block, parry, knockdown, cut and a wall break, prints a checkpoint digest every 100 ticks, and joins the HS2 matrix and the wasm32-wasip1 leg as one more row. Diff: regular. Tier A. Size S. Owns `crates/proof/src/arena_matrix.rs`, `crates/proof/src/bin/arena_line.rs`, `.github/workflows/golden-matrix.yml`. Accept: a mirror run prints the new row on linux-x86_64, windows-x86_64, linux-aarch64 and wasm32-wasip1 with `MATCH`; a unit test flips one input and the digest moves; the report names the first divergent tick. F2P: no arena digest is compared across machines. P2P: golden `d57b5c8ae3096614`, golden tape `2aca9338670aa0cb`, 20 corpus blocks, ragdoll digest unchanged. needs: none (DSN1 makes it the net fixture later).

### 3. `game.toml` is a husk: no key reaches the game (DSN3, size M for slice 1)

Evidence.
- `src/manifest.rs:17-25`: 7 keys: `name targets quality physics_layers gore content seed_file`.
- `src/project.rs:322-330`: `content` is only checked to be a directory. `:417-427`: `run --headless` records and replays `engine_proof::record(Config{seed,ticks})`, the legacy 2D ECS fixture, and prints its receipt. `quality`, `physics_layers`, `gore` are parsed and printed in a summary only.
- `src/build.rs:152-170`: `build --target windows` is `cargo build --release -p engine-game --features app --bin arena` of the ENGINE checkout. The project's `content` roots and keys are not passed. `build --target web` runs `tools/web-build.mjs` the same way.
- `GameConfig` (`crates/game/src/lib.rs:157-166`) has four fields: seed, ticks, fighters, half_extent. Weapons, classes and AI rows are `include_str!`'d design rows (see DSN5). There is no `Game` trait, no plugin and no prelude (`grep "mod prelude" crates` finds none).
- To make a new arena game today an author must fork `engine-game` (39 `pub mod`, 16 engine deps) or hand-edit `content/design/*.rows` and rebuild the engine.

Why it hurts. AIS1 reads as done in the sense that `new/run/build` exist and their tests pass, but the owner's goal ("an AI sets it all up easy", same game on Windows and web) is not reachable. An AI that edits `game.toml` changes nothing; one that runs `run --headless` gets the golden fixture, which says nothing about its game.

Smallest fix, slice 1 (the spec reaches the sim). A `GameSpec` module in `engine-game` (not a new crate): rules (`v1|v2`), fighters, kit path, `physics_layers` to a `physfx::Tier`, `gore` to the combat gore level, `quality` to the single ladder of DSN4. `run --headless` builds a `MatchSpec` (DSN1), plays it with `LiveMatch`, prints the arena digest. `build` passes `--project <dir>` to the `arena` bin, which reads the spec and content roots. Slice 2: the bin reads the same spec on web. Files: `crates/game/src/spec.rs`, `src/project.rs`, `src/build.rs`, `crates/game/src/bin/arena.rs` (arg only).

Golden risk: none while the template defaults write `rules = "v2"` for new projects and old behaviour stays reachable as `v1`.

Board row:
- [ ] DSN3 author: `game.toml` reaches the game — a `GameSpec` in `engine-game` (rules, fighters, kit, `physics_layers` to the physfx tier, `gore`, `quality`, content roots) parsed from the manifest; `engine2040 run --headless` plays that spec on `LiveMatch` and prints the ARENA digest instead of the legacy 2D fixture (`src/project.rs:417-427`); `build` passes `--project <dir>` to the arena bin (`src/build.rs:152-170`). Diff: good. Tier A. Size M (slice 1; slice 2 is the web bin). Owns `crates/game/src/spec.rs`, `src/project.rs`, `src/manifest.rs`, `src/build.rs`, `docs/templates/**`. Accept: `cargo test -p engine2040 --test project_cli` (a template with `rules = "v2"`, 8 fighters and a changed kit prints a different arena digest than the default, twice equal; an unknown key still names key and line); `cargo test -p engine-game --lib spec_`. F2P: every manifest key except `seed_file` is ignored by `run` and `build`. P2P: golden `d57b5c8ae3096614`, the proof CLI default run, HS2 matrix unchanged. needs: DSN1.

### 4. Six quality ladders, none shared (DSN4, size S)

Evidence (enum, variants).
- `crates/render/src/manifest.rs:33` `QualityTier` Minimal/Low/Medium/High/Ultra (what `game.toml quality` parses into, `src/manifest.rs:9`).
- `crates/app/src/config.rs:12` `QualityTier` Low/Medium/High.
- `crates/render/src/quality.rs:11` `Quality` Potato/Comfort/High.
- `crates/render/src/tier.rs:21` `Tier` Base/Rt/Ultra (lighting).
- `crates/physfx/src/caps.rs:8` `Tier` Potato/Web/Comfort/High.
- `crates/anim/src/lod.rs:29` `Tier` (not read further).

Why it hurts. The owner's rule is "every game looks great and runs on a potato; web has its own tier" (potato-target, WEB8, RZ8). Nothing ties a render potato to a physfx potato, so the browser tier (WEB8) will be added a fifth time by hand, and `game.toml quality = "medium"` has no defined meaning for physics caps, shadow size or texture size.

Smallest fix. One `Tier { Potato, Web, Comfort, High }` in `engine-core` (std-only, 40 lines, plus `Ultra` stays render-only as an extra lighting flag). Each subsystem keeps its own table keyed by that enum (`Caps::of(tier)`, `Quality::of(tier)`, `LodTable::of(tier)`) and drops its own enum. `game.toml quality` maps to it. No behaviour change on the existing three tiers.

Golden risk: none (presentation only; core gains a type, not a hash input).

Board row:
- [ ] DSN4 quality: one tier ladder — `engine_core::tier::Tier { Potato, Web, Comfort, High }`; `engine-render` `Quality`/`QualityTier`, `engine-physfx` `Tier`, `engine-app` `QualityTier` and the `anim` LOD tier become tables keyed by it (one `of(tier)` fn each); `game.toml quality` parses to it. Diff: regular. Tier A. Size S. Owns `crates/core/src/tier.rs`, `crates/render/src/{quality,manifest,tier}.rs`, `crates/physfx/src/caps.rs`, `crates/app/src/config.rs`, `crates/anim/src/lod.rs`. Accept: `cargo test -p engine-core --lib tier`; a test in each crate asserts its table is ordered potato <= web <= comfort <= high on every cost knob; `rg "enum (QualityTier|Quality|Tier)" crates` lists only core and the render lighting `Rt/Ultra` flag. F2P: six enums, no shared order. P2P: golden unchanged; capture manifests keep their tier labels (`render/manifest.rs` strings pinned in a test). needs: none (WEB8 builds on it).

### 5. Content is baked into sim crates and found by walking up from the exe (DSN5, size M)

Evidence.
- Non-test `include_str!` of repo content in library crates: `crates/arena/src/kit.rs:17` (`content/arena/placement.json`), `crates/arena/src/zones.rs:173` (`arena-zones.rows`), `crates/game/src/kit_load.rs:16`, `crates/game/src/arena_kit.rs:33`. `include_*!("../../../...")` appears in 35 files under `crates/` (most are test fixtures; `ai`, `combat` embed `content/design/*.rows` in tests, the four above are production consts).
- `AssetSource` exists (`crates/assets/src/asset_source.rs:165`, landed in `bc6838ac`, WEB6 landing 1), but only `arena_kit.rs` and `fight_ring.rs` use it. `fighter_look.rs:935-945` still finds `content/fighters` by walking up from `current_exe()`; `windowed.rs:791` opens `content/audio`; `arena_kit.rs:572-582` the same walk for the kit root.
- `wasm-gate` (run now): `std::fs` hazards: game 21, assets 14, proof 15, save 6, render 7, net 2, audio 1 (66 of 231 hazards).
- `.gitignore` keeps `content/fighters/bundle/` and `content/arena/bundle/` untracked: a fresh clone has no cooked bundle until the cook runs.

Why it hurts. A second game cannot have its own arena without editing `engine-arena`. The same bytes are embedded in two crates (`arena` and `game` both embed `placement.json`; two copies of `arena-zones.rows` consts) and nothing proves they match. The sim result depends on content that the net handshake does not hash.

Smallest fix. (a) `MatchSpec.world_digest` (DSN1) hashes the placement and zone bytes; both crates load the default through one `engine_arena::kit::DEFAULT_*` const and the game's copy is deleted. (b) `fighter_look`, audio and `practice.rs` take `&dyn AssetSource`; the roots come from `GameSpec.content` (DSN3). (c) The embedded consts stay as the default source for tests and the headless proof. Files: `crates/arena/src/{kit,zones}.rs`, `crates/game/src/{kit_load,arena_kit,fighter_look,windowed,practice}.rs`.

Golden risk: none if the embedded default bytes are unchanged (assert digest equal to today's in a test).

Board row:
- [ ] DSN5 content: one source, one copy — delete the second embedded copy of `placement.json` and `arena-zones.rows` in `engine-game` (`kit_load.rs:16`, `arena_kit.rs:33`) in favour of `engine-arena`'s const, put `fighter_look`, the audio bank load and `practice` on `AssetSource` with roots from `GameSpec.content`, and hash the loaded placement and zones into `MatchSpec.world_digest`. Diff: regular. Tier A. Size M. Owns `crates/arena/src/{kit,zones}.rs`, `crates/game/src/{kit_load,arena_kit,fighter_look,windowed,practice}.rs`. Accept: `node tools/wasm-gate.mjs` shows game `fs` at or below 8 (from 21) with no new baseline raise; `cargo test -p engine-game --lib kit` (embedded default digest equals the pinned one; a project root with another placement changes `world_digest`); `rg 'include_str!\(".*placement.json' crates` hits once. F2P: two embedded copies; fighters and audio read `content/` by walking up from the exe. P2P: golden, matrix, v1 arena checkpoints unchanged. needs: DSN1 (digest), DSN3 (roots) for the last bullet only; the dedupe can land first.

### 6. A third physics lives inside the authority: `arena/sim` (DSN6, size S)

Evidence.
- `crates/arena/Cargo.toml`: `sim = ["engine-physics/rapier"]`. 56 `cfg(feature = "sim")` sites in arena: lib.rs 28, fork.rs 12, live.rs 11, sync.rs 4, parity.rs 1.
- Nobody ships it: `crates/game/Cargo.toml:12-17` says `app` does NOT enable it; `crates/net/Cargo.toml` comment says the sim track stays off; `LiveMatch::snapshot` exists only for `cfg(not(feature = "sim"))` (`crates/arena/src/live.rs:400`), so with `sim` on there is no rollback at all.
- `crates/physics/src` mixes the layers in one crate: float (`ragdoll.rs` 112 f32 mentions, `cloth.rs` 93, `kcc.rs` 47, `powered.rs` 45, `rapier.rs`, `rigid.rs`, `pd.rs`) beside integer (`limb.rs`, `stagger_memory.rs`, `layers.rs`, `controller2.rs`: zero f32). Only the cargo feature `rapier` separates them. `wasm-check.yml:57-58` still builds `engine-arena-sim`.
- Honest statement of the seam: authority is not "all integer". `engine_physics::controller` (115 f32/f64 mentions) and the f32 pose are authority (arena uses them at 4 sites); the integer parts are v2 add-ons. The hash law guarantees hold through `+ - * /` and `sqrt` only (`HASH-LAW.md:59-65`), which is fine, but `docs/plan/web-and-realism.md:30` ("integers") oversells it.

Why it hurts. Two authority tracks double the state space of every arena change (56 cfg sites to keep compiling), and a flag flip puts Rapier into the hashed path with no rollback. A future author reading `arena/sim` will think it is the way to get real rigid bodies, which is the opposite of the two-layer design.

Smallest fix. Delete the `sim` feature and its 56 sites (or, if the owner wants it parked, move it to `archive/` with a note). Add a workspace-check rule: `engine-arena`, `engine-net`, `engine-proof`, `engine-combat` may not enable `engine-physics/rapier` or name `engine-physfx`; today only physfx is guarded. Update the doc sentence about integers. Files: `crates/arena/Cargo.toml`, `crates/arena/src/{lib,fork,live,sync,parity}.rs`, `tools/workspace-check.mjs`, `.github/workflows/wasm-check.yml`, `docs/plan/web-and-realism.md`.

Golden risk: none for the golden; default-feature arena bytes unchanged (assert with the DSN2 digest before and after).

Board row:
- [ ] DSN6 physics: one authority track — remove the `engine-arena` `sim` feature and its 56 `cfg(feature = "sim")` sites (`lib.rs` 28, `fork.rs` 12, `live.rs` 11, `sync.rs` 4, `parity.rs` 1), drop the `engine-arena-sim` leg of `wasm-check.yml`, and make `tools/workspace-check.mjs` fail when `arena`, `net`, `proof` or `combat` enable `engine-physics/rapier` or list `engine-physfx`. Diff: regular. Tier A. Size S. Owns `crates/arena/Cargo.toml`, `crates/arena/src/*.rs` (cfg removal only), `tools/workspace-check.mjs`, `.github/workflows/wasm-check.yml`. Accept: `cargo test -p engine-arena --lib` green on the default features; the DSN2 digest (or `run_match_bytes` of the existing fixtures) identical before and after; `node --test tools/workspace-check.test.mjs` has a fixture where a manifest enabling `engine-physics/rapier` in arena fails. F2P: a `sim` leg nobody ships; no check on rapier reaching authority crates. P2P: golden `d57b5c8ae3096614`, matrix, v1 and v2 arena checkpoints unchanged. needs: DSN2 preferred first (gives the before/after pin).

### 7. The shipped frame loop is in a binary, with a tested twin beside it (DSN7, size L)

Evidence.
- `crates/game/src/bin/arena.rs`: 3,341 lines. `struct ArenaApp` (`:797`), `impl ArenaApp` (`:940-1873`, about 930 lines), `impl ApplicationHandler for ArenaApp` (`:1873`). It owns the fixed-tick accumulator, audio feed, presenter, kit, fighter cache, hot reload poll, capture. Plus `bin/arena_frame/mod.rs` 860 and `bin/arena_capture.rs` 1,265.
- `crates/game/src/windowed.rs:1222` `run_windowed` is a second session driver (lib, 3,086-line file) that builds its own `WindowShell` and bindings. Grep finds no non-doc caller of `run_windowed` outside `windowed.rs`.
- Three winit loops exist: `render/src/gpu.rs:331` (sprite demo), `editor/src/app.rs:1378`, `game/src/bin/arena.rs:1873`.
- `WEB5` (board) plans to "drive the forward PBR plus GPU skinning frame from `crates/web`" and split `window` into `gpu` and `window`; the winit coupling in render is small (`presenter.rs:1164 init_for_window`, `submit.rs:2159 create_surface`, `gpu.rs`: 3 files), and `Presenter::attach_surface` already takes a `wgpu::Surface<'static>` (`presenter.rs:1197-1215`). In-flight work in the tree (untracked `scope_ledger.rs`, `capture.rs`) is already on that.

Why it hurts. Web has no frame to reuse: the thing that turns a `MatchWorld` into draws, audio cues and HUD sits behind an `ApplicationHandler` in a bin. WEB5 would copy about 1,500 lines, and the copy and the original drift. Meanwhile the tests exercise `run_windowed`, not the loop that ships.

Smallest fix. `engine-game::session_loop::FrameDriver`: owns accumulator, `App`, cues, fighter look, kit stage, HUD; input as plain events (`Key/Pad/Pointer` enums), output a `DrawList` plus cue list plus receipt line; takes `&dyn AssetSource`. `arena.rs` becomes a winit shim (about 400 lines: window, events, present, args). `crates/web` becomes the second shim. Delete `run_windowed`, or make it call the driver. Land in three moves: extract the pure frame assembly (`arena_frame`) first, then the accumulator and cues, then the shell.

Golden risk: none (presentation); keep `smoke` receipts byte-equal.

Board row:
- [ ] DSN7 game: the frame loop is a library — move `ArenaApp`'s tick accumulator, cue feed, fighter look, kit stage and HUD assembly (`crates/game/src/bin/arena.rs:797-1873`, `bin/arena_frame/mod.rs`) into `engine-game::session_loop::FrameDriver` with plain input events and an `AssetSource`; `bin/arena.rs` keeps only winit; remove `windowed::run_windowed` (no caller outside its file) or route it through the driver. Diff: hard. Tier A. Size L (landings: arena_frame; accumulator and cues; shell; delete the twin). Owns `crates/game/src/session_loop.rs`, `crates/game/src/bin/arena.rs`, `crates/game/src/bin/arena_frame/mod.rs`, `crates/game/src/windowed.rs`. Accept: `bin/arena.rs` under 1,200 lines; a headless test drives `FrameDriver` for 600 ticks and its draw-list digest per tick equals the pre-change binary's `--smoke` receipt; `cargo test -p engine-game --lib session_loop`; WEB5 consumes the driver. F2P: the loop exists only inside the bin. P2P: golden, matrix, arena checkpoints, product-check frame metrics unchanged. needs: DSN5 (AssetSource everywhere) for the last landing; queue behind the render writer of the day.

### 8. The physical layer is clean but three seam gaps will bite RZ2 and RZ3 (DSN8, size M)

Evidence.
- Direction is correct: `physfx` depends on physics and anim only; `types.rs:1` says no arena/combat/anim type crosses; `break_look.rs` has `the_sim_hash_is_equal_with_the_look_on_and_off` (`:844`). Good, keep it.
- Gap a: only debris is fed. `spawn_debris_of` has one caller (`crates/game/src/break_look.rs:329`); `spawn_ragdoll` (`fx.rs:167`) and `spawn_limb` (`fx.rs:240`) have no caller in `crates/game`.
- Gap b: statics are boxes only (`fx.rs:138 add_static_box`). The authority collides capsule footprints (`crates/arena/src/kit_zone.rs`) and the cook now writes hull and triangle-mesh colliders (`crates/assets/src/collider_cook.rs`, `crates/physics/src/collider.rs`, in the working tree). A ragdoll can come to rest inside a pillar the authority respects.
- Gap c: no hand-back contract. Authority "holds the root" of a fallen fighter (`HASH-LAW.md:98-99`) while the Rapier ragdoll tumbles away; nothing says how the picture returns to the authority pose at get-up or what happens when they disagree by metres.
- Gap d: `PhysFx::transforms` returns a fresh `Vec<Placed>` every frame (`fx.rs:346`); the potato target counts allocations (`render` has zero-alloc probes).

Why it hurts. Without b and c the first ragdoll in the window will either clip through walls or pop at get-up, and the fix will tempt someone to feed physics results back into authority, which breaks the whole design.

Smallest fix. (b) `add_static_hull` and `add_static_mesh` fed by `engine_physics::collider::decode` so authority and picture read one cooked blob (E2CC). (c) `PhysFx::settled(slot) -> Option<Transform>` plus a game-side `Handback { slot, from_pose, blend_ticks }` that blends the picture to the authority pose over a fixed 20 ticks, with a counter `handback_gap_mm` in the `physfx:` receipt line (so a large gap is visible). (d) `transforms_into(&mut Vec<Placed>, alpha)`. A grep gate keeps `physfx` out of arena, combat and proof (exists).

Golden risk: none (floats, never hashed). Keep the on/off sim-hash test, extend it to ragdoll and limbs.

Board row:
- [ ] DSN8 physfx: close the seam gaps before RZ2/RZ3 — `add_static_hull`/`add_static_mesh` fed by the cooked E2CC collider blob (one blob for authority footprints and the picture), `PhysFx::settled(slot)` plus a game-side `Handback` blend over a fixed 20 ticks with `handback_gap_mm` in the `physfx:` receipt line, and `transforms_into(&mut Vec<Placed>, alpha)` with no per-frame allocation. Diff: good. Tier A. Size M. Owns `crates/physfx/src/{fx,types}.rs`, `crates/game/src/break_look.rs` (read-back call), a new `crates/game/src/handback.rs`. Accept: `cargo test -p engine-physfx --features rapier --lib static_hull` (a debris shard dropped on a cooked cube hull rests on it, not inside); `cargo test -p engine-game --lib handback` (a ragdoll thrown 5 m from its authority pose is back on it within 20 ticks, gap printed); `transforms_into` steady 120 frames reads zero allocs; the sim-hash on/off test covers ragdoll and a severed limb. F2P: statics are boxes, no hand-back, one Vec per frame. P2P: golden, matrix, arena checkpoints unchanged (physfx is never hashed). needs: RZ5 (collider cook) for the hull input; the receipt and `transforms_into` parts can land first.

### 9. Two dead rollback traits that look like the plug-in point (DSN9, size S)

Evidence.
- `crates/net/src/rollback.rs:298` `trait SimTick` and `crates/net/src/netcode.rs:99` `trait Sim`, both two-player (`[u32; 2]`), both `Clone`. Implementors: only test toys (`crates/net/tests/it/*.rs`, `rollback.rs:815`) and `delay.rs:74` `DelayProbe`. No caller outside `crates/net` (checked with `rg "netcode::|rollback::"`; the `save` hits are `save`'s own `rollback` module).
- The real path never uses them: `Server`/`Client` are written against `LiveMatch` (`session.rs:67`). Files: rollback.rs 1,291 lines, netcode.rs 863, delay.rs 181 (2,335 lines).

Why it hurts. It reads as "implement `SimTick` and get rollback", so an author or an AI will try it for an 8-player game and find two seats. It is also 2.3k lines every net change must keep compiling and every agent must read past.

Smallest fix. Delete `P2pSession`, `netcode::Sim` and `delay.rs`, keep `netcode`'s `KIND_CHANNEL` table (used by the encoder) and `rollback`'s frame log only where `session.rs` uses it. Or, if a second sim is wanted, make `Server<S: NetSim>`; I recommend delete: the arena is the engine's sim.

Golden risk: none.

Board row:
- [ ] DSN9 net: delete the two-player rollback toys — remove `rollback::{SimTick, P2pSession}`, `netcode::Sim` and `delay.rs` (no caller outside `crates/net`; the real path is `LiveMatch` in `session.rs`), keep what `session.rs` and the encoder use (`KIND_CHANNEL`, the frame log). Diff: regular. Tier A. Size S. Owns `crates/net/src/{rollback,netcode,delay,lib}.rs`, `crates/net/tests/it/*` that only exercise the toys. Accept: `cargo test -p engine-net --lib` and `--test it` green; `rg "SimTick|P2pSession" crates` empty; net production lines down by at least 1,500. F2P: two unused sim traits. P2P: golden, matrix, net session wire bytes (`WIRE_VERSION` fixtures) unchanged. needs: none.

### 10. Joining a match replays every input since tick 0 (DSN10, size M)

Evidence.
- `crates/net/src/session.rs:305-320` `SnapshotDelta { version, tick, hash, tape }`: "the complete authoritative input prefix", docs call it "full-snapshot-for-now". `Client::resimulate` (`:657-700`) restores from `truth.snapshot()` for the unacked suffix, but a late joiner or a rejoiner gets the whole tape from tick 0, cut into 1200-byte packets (`MAX_PACKET_BYTES`, no fragmenting).
- `MatchSnapshot` (`crates/arena/src/lib.rs:565`) exists for in-process rollback and has no byte codec (`grep "fn to_bytes|fn from_bytes|fn encode|fn decode" crates/arena/src` finds only `recorder.rs`).
- Wasm physics costs 1.5x to 1.8x native (`web-and-realism.md:23`) and the browser is the client most likely to join late.

Why it hurts. Late join costs O(match length) packets and O(match length) sim steps on the weakest client. With 8 fighters and rules v2 that is a visible stall; at 10 minutes it is 36,000 ticks to replay in wasm. It also blocks spectators and reconnect-after-drop on the web.

Smallest fix. `MatchSnapshot::to_bytes/from_bytes` in arena (canonical order, integer mm, versioned; includes limb pool and bonded state through `engine_destruction::snapshot`, which already has a codec: `limbs.rs:17`), a keyframe every 300 ticks on the server, and a `SnapshotDelta` variant carrying `state_bytes + tape_suffix`. The version tag is already the migration seam the comment names.

Golden risk: none; round-trip test: `from_bytes(to_bytes(s))` steps to the same digest as `s`.

Board row:
- [ ] DSN10 net: join from a keyframe — `MatchSnapshot::to_bytes/from_bytes` (versioned, canonical, includes the limb pool and bonded wall state through `engine_destruction::snapshot`), a server keyframe every 300 ticks, and a `SnapshotDelta` variant of `state_bytes + tape_suffix` cut into the existing 1200-byte packets; a late joiner or rejoiner restores from it instead of replaying from tick 0. Diff: good. Tier A. Size M. Owns `crates/arena/src/snapshot_wire.rs` (new), `crates/net/src/session.rs`, `crates/net/src/split.rs`. Accept: `cargo test -p engine-arena --lib snapshot_wire` (round trip; a bit flip is refused; restored match steps 600 ticks to the same digest); `cargo test -p engine-net --lib join_keyframe` (a client joining at tick 3,000 of a v2 8-slot bout converges after at most 300 stepped ticks and fewer than 40 packets). F2P: join cost grows with match length. P2P: golden, matrix, existing `WIRE_VERSION` fixtures decode unchanged. needs: DSN1.

### 11. Consumer-or-park gate and stale edges (DSN11, size S)

Evidence.
- Crates with no reverse dependency and no binary: `playtest` (19.7k production lines, 27k with tests), `nav` (5.0k), `distill` (6.2k; KEEP decision `ARCHITECTURE.md:80-91`), `engine-chaos` (96). `CRATE-MAP.md` "Used by" is `-` for all four. About 38k lines compiled and read by nobody. (`ARCHITECTURE.md` says SL1 deletes what stays unconsumed.)
- `tools/workspace-check.mjs:19-79` allow-list entries with no manifest edge (script `scratchpad/edges.mjs`): `engine-arena -> engine-provider` (it is a dev edge, ok), `engine-destruction -> engine-nav`, `engine-physics -> engine-ecs`, plus keys for packages that do not exist: `engine-viewer`, and root entries `engine-script`, `engine-ponytail`. An allowed-but-absent edge is a pre-approved cycle path.
- Prose drift: `ARCHITECTURE.md:4` says "35 crates plus the root (36 total)"; real: 38 plus the root. The layer diagram names `engine-app` as the composition layer, but only `editor` uses it (`CRATE-MAP.md`), see DSN13.
- Weight: 772 tracked `.md` (docs 651 files, `docs/archive` 393 files 15 MB), 33 `docs/SPRINT*.md` totalling 10,187 lines, `tools/` 78 scripts plus 108 test files, `skills/` 152 files, `from-design-studio/` 31 files, `team/` 8, `opencode.jsonc`, a local `.opencode` of 70 MB; the OpenCode loop is retired (memory `opencode-loop-retired`). Local `target/` is 63 GB (not in git). Tracked working-copy bytes: 209 MB, of which `content/arena` 110 MB.

Smallest fix. A gate in `workspace-check`: every crate without a reverse edge and without `[[bin]]` must be listed in a `PARKED` table with a date and a one-line reason, or the check fails; fail also on "allowed but absent" edges. Then prune: move archive docs older than 30 days out (`git rm`, history keeps them), delete the retired loop's leftovers the owner confirms. Playtest and nav need an owner decision: wire into `endpoint` (`play.*`) and arena crowd, or park.

Golden risk: none.

Board row:
- [ ] DSN11 hygiene: consumer-or-park — `tools/workspace-check.mjs` fails on (a) a crate with no reverse dependency and no `[[bin]]` that is not in a dated `PARKED` table (today `playtest`, `nav`, `distill`, `engine-chaos`), and (b) an allow-list edge or key with no manifest behind it (`destruction->nav`, `physics->ecs`, `engine-viewer`, root `engine-script`/`engine-ponytail`); fix the crate count in `docs/ARCHITECTURE.md:4`. Diff: regular. Tier A. Size S. Owns `tools/workspace-check.mjs`, `tools/workspace-check.test.mjs`, `docs/ARCHITECTURE.md` (counts only). Accept: `node --test tools/workspace-check.test.mjs` (two new fixtures fail, the real tree passes with the PARKED table); `node tools/workspace-check.mjs` PASS. F2P: unconsumed crates and unused allowed edges pass silently. P2P: none (no Rust change). needs: owner decision on playtest and nav (wire or park).

### 12. `engine-render` is one 98k-line unit and "one render writer at a time" is its symptom (DSN12, size L)

Evidence.
- 97,247 lines in 101 files (55.0k production), 61 `pub mod`, 2,457 pub items. `Presenter` is one struct of 242 lines (`crates/render/src/present/gpu_present/presenter.rs:54-296`); its passes are `impl Presenter` blocks spread over `live.rs` (3,011), `presenter.rs` (3,000), `submit.rs` (2,837), `sun_shadow.rs`, `gtao_live.rs`, `msaa.rs`, `capture.rs`. Nine render paths are dirty in the tree at this moment (`Cargo.toml`, `ibl.rs`, `gpu_present.rs`, `live.rs`, `submit.rs`, `sun_shadow.rs`, `sky.rs`, plus new `capture.rs` and `scope_ledger.rs`).
- `window` feature = wgpu + winit + pollster + the Windows-only `windows` crate + `v1` (`crates/render/Cargo.toml`); 336 `cfg(feature = "window")` sites; winit appears in 3 files only. `lib.rs` itself says most modules are std-only (draw list, graph, quality, heal, cache, pass_truth).
- A frame graph exists (`graph.rs`, `executor.rs`: transient aliasing planner, CPU executor) but the live encode is hand-sequenced in `submit.rs`; `executor.rs` header says it "refuses GPU submit ... until the bind groups land".
- `wasm-gate` today: render 112 of 231 hazards (54 clock, 50 blocking GPU waits, 7 fs, 1 thread).

Why it hurts. Every Look row (LF1-LF3, EG11-EG13, VR1-VR4) edits the same three files and the same struct, which is why `web-and-realism.md:59` has to queue them "one render writer at a time". With 8-10 agents that is the biggest throughput limit in the repo, and it also makes the web port (async init, deferred error scopes) edit the same hot files.

Smallest fix, in order of cost. (1) WEB5 landing 1 as already planned (`gpu` vs `window` feature; make `init_for_window` a thin shim over `attach_surface`). (2) Give each pass its own state struct and file, starting with the ones with the most churn (`sun_shadow`, `gtao_live`, `sky`, `ibl`): `trait Pass { fn prepare(&mut self, ctx: &mut FrameCtx); fn encode(&mut self, enc: &mut wgpu::CommandEncoder, ctx: &FrameCtx); }`; `Presenter` keeps the list and the order. (3) Later, a crate split: `engine-render` (std-only core: draw list, graph, quality, cache, heal, pass_truth) and `engine-gpu` (wgpu passes).

Golden risk: none (render never feeds the hash). Frame bytes must stay equal: use the existing pass-truth on/off pairs and a capture digest per landing.

Board row:
- [ ] DSN12 render: passes own their state — introduce `trait Pass { prepare, encode }` in `crates/render/src/present/gpu_present/pass.rs`, move the sun shadow and GTAO state out of `struct Presenter` (`presenter.rs:54-296`) into `SunShadowPass` and `GtaoPass` in their own files, `Presenter` only sequences; the next passes (sky, IBL, bloom) follow as one landing each. Diff: hard. Tier A. Size L (landings: trait plus sun shadow; GTAO; sky; IBL). Owns `crates/render/src/present/gpu_present/{pass,sun_shadow,gtao_live,presenter}.rs`. Accept: `cargo test -p engine-render --features window --lib` (GPU; runs on the PC) with the pass-truth on/off pairs and the offscreen capture digest of the reference scene byte-equal before and after each landing; `Presenter` struct under 160 lines after landing 2; steady-frame alloc probes unchanged. F2P: every pass edits one 242-line struct and three 3,000-line files. P2P: golden, matrix unchanged; frame bytes equal. needs: none; each landing waits for the render writer lock the board already names (this row is how that lock goes away). Do after WEB5 landing 1.

### 13. The documented composition layer is not the one the game uses (DSN13, size M)

Evidence.
- `ARCHITECTURE.md:13-25` draws `engine-app` as the composition root under every application. `CRATE-MAP.md:11`: `engine-app` is used by `editor` only (3,481 lines: AppBuilder, Platform trait with workshop/lobby stubs, its own `QualityTier`, journal, extraction). `engine-game` has no `engine-app` dependency (`crates/game/Cargo.toml`) and composes 16 crates by hand (`App` struct, `crates/game/src/lib.rs:565`).
- `engine-web` gets the authority through `engine-proof` (`crates/web/Cargo.toml`: `engine-proof.workspace = true`; `crates/web/src/golden.rs:14`), a fixture crate whose default closure is 12 crates and whose `m1` feature pulls `editor` and `render`.
- `engine-ai` depends on `engine-ecs` and `engine-physics` (`crates/ai/Cargo.toml`; `lib.rs:8-9` legacy `World`/`Space` patch seam, named as an exception in `ARCHITECTURE.md:52`), so arena's tick graph carries ecs and physics through ai.
- There is no prelude and no `Game` entry type; `engine_game` exports 39 modules.

Why it hurts. For the author there are two answers to "what do I build on": `engine-app` (editor) or `engine-game` (the product); web will add a third (`engine-web` + `proof`). Each new target copies the composition. An AI asked to "make a new arena game" has no single type to start from.

Smallest fix, decide first. Option A (recommended): retire `engine-app` as the composition root in the docs, keep it as the editor's play-in host, and make `engine_game::Game` (built from `GameSpec`, DSN3) the one entry: `Game::new(spec)`, `step(inputs)`, `view() -> DrawList`, `cues()`, `net_spec()`; `engine-game::prelude` re-exports the 12 types a game author touches. `engine-web` and `arena` bin both call `Game`. `engine-web` drops `engine-proof` (keep golden as a dev-only self-test). Move ai's legacy `patch` behind a `legacy-world` feature so arena stops carrying ecs and physics through ai. Option B: move game onto `engine-app`.

Golden risk: none (composition only).

Board row:
- [ ] DSN13 api: one entry type — `engine_game::Game` built from `GameSpec` (`new`, `step`, `view`, `cues`, `net_spec`) plus `engine_game::prelude` (the 12 types an author touches); `engine-web` and the `arena` bin both use it; `engine-web` drops its `engine-proof` dependency (golden self-test moves to a dev feature); `engine-ai`'s legacy `World`/`Space` patch moves behind a `legacy-world` feature; `ARCHITECTURE.md` section 1 names `Game` and the editor's `engine-app` honestly. Diff: good. Tier A. Size M. Owns `crates/game/src/{game,prelude}.rs`, `crates/web/Cargo.toml`, `crates/web/src/golden.rs`, `crates/ai/Cargo.toml` and its patch module, `docs/ARCHITECTURE.md` section 1, `tools/workspace-check.mjs` edges. Accept: `cargo tree -p engine-arena -e normal` no longer lists `engine-ecs` or `engine-physics` through ai (physics stays direct for controller2); `cargo tree -p engine-web --target wasm32-unknown-unknown` lacks `engine-proof` and `engine-editor`; `cargo test -p engine-game --lib game_` (a `Game` from the arena-duel template steps 600 ticks twice to one digest); the quick-start in `docs/templates/arena-duel/README.md` uses only `prelude`. F2P: no entry type; web reaches authority through the proof crate. P2P: golden, matrix, arena checkpoints, `cargo test -p engine-ai --lib` unchanged. needs: DSN3.

## Layer-by-layer notes (what I checked and did not find a problem in)

1. Layering: no cargo cycle with features on (graph script); `physfx` is imported by `game` only; the `physfx` grep gate exists (`physfx/src/lib.rs:12`). The real flaws are DSN6, DSN11 and DSN13. `endpoint` (instability 1.00) depends on `game`, which is fine for a door that binds play; not changed.
2. Two physics layers: direction right, seam gaps in DSN6 and DSN8.
3. Window/GPU split and wasm: the compiler is not the blocker (probe, `wasm-gate` PASS with 231 hazards under baseline). The blockers are DSN7 (frame loop in a bin), then the `block` and `clock` hazards in render (50 and 54) that the in-flight scope ledger and clock work is already shrinking; `wasm-gate.mjs` printed six IMPROVED lines today. Not checked: whether `pollster::block_on` sites (57 across render and game by grep) are test-only.
4. Author API: DSN3, DSN13.
5. Data and cooks: DSN5; `AssetSource` is the right seam and exists. Not checked: `dev_hotpatch.rs` and `reload.rs` beyond their headers (path notes plus a cook closure, applied at frame boundaries; looks sound).
6. Net: DSN1, DSN9, DSN10. The `Transport` trait is a sync poll (`quic.rs:187`), which suits wasm; WebTransport server exists behind the `webtransport` feature; no browser client transport exists yet (WEB9, WEB12). Not checked: lag compensation (cut on 2026-10-16).
7. Testing and proof: DSN2. Also: GPU tests run only on the owner's PC; the CI path for WEB5 is the SwiftShader flag set recorded in `web-and-realism.md`. 8,050 tests is a lot; the gap is a cross-layer one (arena + net + physfx + render in one deterministic scene), which DSN1 and DSN2 start to fill.
8. Size: DSN11, DSN12.

## Do these first (ten lines)

1. DSN2: add the arena match digest to the HS2 matrix and the wasm leg (S). It turns the tier-3 claim into a number and gives every later row a before/after pin.
2. DSN6: delete `arena/sim`, add the "no rapier in authority crates" check (S), using the DSN2 pin.
3. DSN1: `MatchSpec` so net plays the v2 kit match the window plays (M).
4. DSN9: delete the two-player rollback toys (S, can run beside DSN1 in `crates/net`, different files).
5. DSN4: one tier ladder in core (S), before WEB8 adds the fifth one.
6. DSN3 slice 1: make `game.toml` reach `LiveMatch` and print the arena digest (M, after DSN1).
7. DSN5: dedupe the embedded placement/zones copies, put fighters and audio on `AssetSource` (M).
8. DSN10: keyframe join over the wire (M, after DSN1; it is what a browser client needs).
9. DSN7: extract `FrameDriver` from the arena bin, first the pure `arena_frame` assembly (L, landings), then WEB5 uses it.
10. DSN8, DSN11, DSN12, DSN13 in that order as capacity allows; DSN12 landing 1 is WEB5 landing 1, do not duplicate it.

## Not checked

- Runtime cost: no profile, bench or GPU run was done. `physfx` budgets (8 ragdolls, 200 debris, 6 limbs) are unmeasured by me.
- `crates/endpoint` internals (61k lines, about 27 verb-name sites): whether packs duplicate each other was not read.
- `crates/editor`, `crates/audio` (web audio path), `crates/provider`, `crates/spatial`, `crates/world`, `crates/nn`: only manifests and headers.
- Whether `MatchSnapshot` carries the rules version and world identity (DSN1 assumes it does not; read `lib.rs:565-600` first).
- Whether `docs/book/door-reference.md`, `docs/templates/` and `src/{project,manifest,build}.rs` (untracked, another session's AIS1 work in the tree today) will change DSN3's evidence before it lands.
- CI contents beyond `golden-matrix.yml`, `wasm-check.yml` and the head of `ci.yml`.
