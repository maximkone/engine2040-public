### Final picture: auto-fix, compact code, touch and POV (owner 2026-10-20)

Boarded 2026-10-21 from the owner's ask: "what we steal ... to make it auto heal auto fix easy af",
"make it compacter and code better af", "all have collision in scenes by default", "crowds real
touching, leaning on ppl", "center of gravity", "pov helmet view realistic". Plans:
`docs/plan/final-picture.md` (AF, CX) and `docs/plan/touch-and-pov.md` (TC, POV). Runtime
self-heal rows already on the board (AD14, E75) and the compaction rows the powerups lead boards
(CMP-01..CMP-17) are not repeated here. Every row names its consumer and a number.

- [ ] AF1 tools: one command applies the toolchain's mechanical fixes to the paths an agent owns. `node tools/fix.mjs -p <crate> --only <path>...` snapshots every tracked and dirty file of the crate, runs `cargo fix --allow-dirty --lib -p <crate>`, then `cargo clippy --fix --allow-dirty --lib -p <crate>`, then `rustfmt --edition 2021` on the `--only` files, and restores byte-for-byte every file outside `--only` that a fixer touched (other sessions share the tree). Prints `fix: <n> owned files changed, <m> foreign files restored`. Donor: rustfix (the cargo fix path) and the clippy usage page, read 2026-10-20. Tier A. Size S. Owns `tools/fix.mjs`, `tools/fix.test.mjs`.
  Accept: `node --test tools/fix.test.mjs` passes: a fixture where the fixer edits one owned and one foreign file ends with the owned edit kept and the foreign file byte-equal to its snapshot; a run on a crate with a seeded unused import in an owned file removes it and prints `fix: 1 owned files changed, 0 foreign files restored`.
  F2P: no fix tool; `tools/fix.mjs` absent.
  P2P: `node tools/workspace-check.mjs` PASS.
  needs: none.
- [ ] AF2 tools: every rust-quality FAIL line names its fix. Each rule id that `tools/rust-quality.mjs` reports gets a one-sentence `fix:` tail naming the exact edit (for example `#[allow(x)]` -> `#[expect(x, reason = "...")]`, a lossy `as` cast -> the `cast_safe` helper), so an agent fixes a red gate without reading the rule source. Donor: ast-grep YAML `fix:` field (MIT), read 2026-10-20; no ast-grep install. Tier A. Size S. Owns `tools/rust-quality.mjs`, `tools/rust-quality.test.mjs`.
  Accept: `node --test tools/rust-quality.test.mjs` passes with one new test that runs every rule id on its seeded fixture and asserts each FAIL line ends with `fix: ` plus non-empty text; `node tools/rust-quality.mjs` still prints PASS on the tree.
  F2P: FAIL lines carry no fix text.
  P2P: rust-quality PASS, ratchet unchanged.
  needs: none.
- [ ] AF3 skills: the fix-red protocol. A new skill `skills/fix-red/SKILL.md` (Agentless shape: reproduce, localize, patch, validate) for a red test or gate: the exact `tools/gh-check.mjs` line to reproduce, localize from the panic `file:line`, `node tools/affected.mjs` and `git log -L`, apply `tools/fix.mjs` first, write the smallest patch, validate the failing test plus its crate's lib tests, never commit a patch whose test was not seen red first; a second identical failure becomes one `docs/BLOCKERS.md` row. Under 80 lines, listed in `skills/SKILLS.md`. Donor: OpenAutoCoder/Agentless (MIT), arXiv 2407.01489, read 2026-10-20. Tier A. Size S. Owns `skills/fix-red/SKILL.md`, `skills/SKILLS.md`.
  Accept: `node --test tools/skills-index.test.mjs` passes with the new skill listed; the skill names only tools that exist (`node tools/workspace-check.mjs` PASS).
  F2P: no skill for a red test beyond `debug-triage`'s triage steps.
  P2P: workspace-check PASS.
  needs: AF1.
- [ ] AF4 assets: content rules with a fix. `crates/assets/src/validate.rs` holds `Rule { id, severity, check, fix }` over a cooked mesh: `tangents.missing` (fix: `import_repair::recompute_tangents`), `tri.degenerate` (fix: drop zero-area triangles), `normal.not_unit` (fix: renormalize), `float.nonfinite` (no fix, error), `collider.missing` (fix: TC2's hull). `validate(&mesh)` returns hits in rule order; `fix(&mut mesh, id)` applies one rule and re-checks it. Donors: Unreal EditorValidatorBase, Godot configuration warnings, glTF-Validator (Apache-2.0), read 2026-10-20; none ship a fix callback, which is the part we add. Tier A. Size M. Owns `crates/assets/src/validate.rs`, `crates/assets/src/lib.rs`.
  Accept: `cargo test -p engine-assets --lib validate` passes: a seeded mesh with each defect reports exactly its rule ids, each fixable rule's fix clears its hit and no other, a clean CT7 body reports none, and two runs give identical hits.
  F2P: no rule registry; `validate.rs` absent.
  P2P: `cargo test -p engine-assets --lib import_repair` green; golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] AF5 tools: mutation testing names the missing tests in the sim core. Run `.tools/bin/cargo-mutants` (MIT, installed 2026-10-21 by owner decision) on `engine-core` and `engine-combat` with `--in-diff` off and a 20-minute cap per crate, write the surviving mutants to `target/reports/mutants-<crate>.md`, then add one test per surviving mutant in `crates/core/src/` until the core report lists none (combat's list is the next row's input). Donor: sourcefrog/cargo-mutants, read 2026-10-20. Tier B. Size M. Owns `crates/core/src/`, `docs/plan/final-picture.md`.
  Accept: a second `cargo mutants -p engine-core` run prints 0 missed mutants (or every survivor has a one-line reason in the report as equivalent); `cargo test -p engine-core` green; golden d57b5c8ae3096614 x2.
  F2P: no mutation report exists; the first run lists the missed mutants.
  P2P: golden x2; `node tools/rust-quality.mjs` PASS.
  needs: none.
- [ ] AF6 tools: unused dependencies go in one run per crate. `.tools/bin/cargo-shear` (MIT, installed 2026-10-21) runs report-only from `tools/workspace-check.mjs` (a WARN line with the count), then `cargo shear --fix` removes the unused entries crate by crate, each crate proved by `cargo check -p <crate> --all-targets` on GitHub. Feeds CMP-09 (duplicate copies). Donor: Boshen/cargo-shear, read 2026-10-20. Tier A. Size S. Owns `tools/workspace-check.mjs`, `crates/*/Cargo.toml`.
  Accept: workspace-check prints `WARN unused deps (report-only): 0`; every touched crate checks green on the runner; `Cargo.lock` package count printed before and after.
  F2P: no unused-dependency scan runs.
  P2P: workspace-check PASS; golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] CX1 game: the cape layer leaves `fighter_look.rs`. Move `Capes`, `CapeJoints`, `CapePose`, the cape constants, `cape_row_into`, `cape_normal`, `cape_mesh` and the `cape_*` methods into `crates/game/src/fighter_look_cape.rs` as a `#[path]` child module, the way `fighter_look_shoulder.rs` and `fighter_look_cam.rs` left. No behaviour change. Tier A. Size S. Owns `crates/game/src/fighter_look.rs`, `crates/game/src/fighter_look_cape.rs`.
  Accept: `fighter_look.rs` is at least 400 lines shorter; `cargo test -p engine-game --features app --lib cape` passes with the same test count as before the move; `node tools/rust-quality.mjs` PASS.
  F2P: the cape layer lives inside the 6,000-line file.
  P2P: golden d57b5c8ae3096614 x2; `capes_never_reach_the_authority_checkpoints` green.
  needs: TC1.
- [ ] CX2 tools: a file over 3,000 lines cannot grow. `tools/rust-quality.mjs` records the line count of every `crates/*/src` file over 3,000 lines in its baseline and FAILs when one grows; a file under 3,000 that crosses it FAILs too. Shrinking lowers the baseline. Today 13 files are over 3,000 (`docs/research/powerups-2026-10-20/09-code-compaction.md` top-20 table). Tier A. Size S. Owns `tools/rust-quality.mjs`, `tools/rust-quality.test.mjs`, `tools/rust-quality.baseline.json`.
  Accept: `node --test tools/rust-quality.test.mjs` passes with a test that grows a seeded 3,001-line file by one line and sees FAIL with the path and both counts; the tree run prints PASS.
  F2P: a 6,000-line file may grow without a signal.
  P2P: rust-quality PASS.
  needs: AF2.
- [ ] TC1 game: a cape meets every body near it, not only its wearer's torso. The cape steps against its wearer's torso capsule plus all ragdoll bone capsules from the one bone table (`ragdoll_look::bone_capsules`, radius plus 2 cm skin) and the same set for every posed rival within 1.6 m. Presentation only; the cloth is never hashed. Tier A. Size S. Owns `crates/game/src/fighter_look.rs`, `crates/game/src/ragdoll_look.rs`.
  Accept: `cargo test -p engine-game --features app --lib cape` passes, including `a_rival_pressed_behind_pushes_the_cape_instead_of_passing_through` (a rival 0.35 m behind: no free cape particle more than 2 cm inside its torso shell) and `capes_never_reach_the_authority_checkpoints`.
  F2P: the cape stepped against one collider (its wearer's torso), so legs and rivals passed through it.
  P2P: golden d57b5c8ae3096614 x2; `capes_pin_to_the_shoulders_and_trail_a_running_fighter` green.
  needs: none.
- [ ] TC2 assets: collision by default for meshes. `collider_cook` cooks a convex hull from the render mesh of every node that has no `COL_` child, unless the node name starts with `NOCOL_`; the hull takes the PROP layer. The receipt prints `colliders: <tagged> tagged, <auto> auto, <opted> opted out`. Tier A. Size S. Owns `crates/assets/src/collider_cook.rs`, `crates/assets/src/mesh_cook.rs`.
  Accept: `cargo test -p engine-assets --lib collider_cook` passes: a fixture with one plain mesh cooks one hull, a `NOCOL_` mesh cooks none, a mesh with a `COL_` child cooks only the tagged shape, and two cooks give identical bytes.
  F2P: an untagged mesh cooks no collider.
  P2P: `kit_world_holds_11_colliders` green (the kit is tagged); golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] TC3 arena: a body push is a shove. In rules v2, a fighter whose push-apart displacement from `collide.rs` exceeds 40 mm in one tick feeds `BalanceState` a shove scaled by the push (a light blow), so a packed crowd staggers and a hard shove can drop a fighter. The numbers sit in one table beside `BalanceParams`. Armed by owner decision 2026-10-21 (like AIN6): a moved v2 hash lands with a receipt under `docs/receipts/`. Donor: capture point, arXiv 1612.08034; Helbing PRE 51:4282 for the compression term (ideas). Tier A. Size M. Owns `crates/arena/src/collide.rs`, `crates/arena/src/live.rs`.
  Accept: `cargo test -p engine-arena --lib collide` passes with a test where 8 fighters spawned on one point under v2 give at least one `Stumbling` gait inside 30 ticks and two runs give the same balance hashes; a v1 run of the same spawn shows no shove.
  F2P: pushes never reach `BalanceState`.
  P2P: golden d57b5c8ae3096614 x2 (v1 never runs the pass); `push_apart_digest_twice_equal_and_tagged` green.
  needs: none.
- [ ] TC4 nav: ORCA steers AI bodies before they touch. `crates/nav/src/orca.rs` (integer RVO2 port, 0 callers today) gets its first caller: in rules v2, AI-driven slots inside the 15 m ring take their new planar velocity from `orca::step` before `collide.rs` runs. Tier A. Size M. Owns `crates/arena/src/live.rs`, `crates/nav/src/orca.rs`.
  Accept: `cargo test -p engine-arena --lib orca` passes: 8 AI slots crossing through each other in v2 end with total push-apart displacement at least 50% lower than the same run without ORCA, and two runs hash the same; `node tools/unwired-code-gate.mjs` no longer lists `orca`.
  F2P: `orca.rs` has 0 callers.
  P2P: golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] TC5 nav: crowds that touch. `crates/nav/src/crowd_contact.rs`: integer position-based contact for 64 to 512 discs, one projection pass per tick after ORCA (disc vs disc, disc vs wall), each agent's pressure in newtons-equivalent as an integer, and a crush threshold that hands TC3 a shove. Donor: Weiss et al., Computers and Graphics 78 (2019), DOI 10.1016/j.cag.2018.10.008 (paper, ideas only), read 2026-10-20. Tier B. Size M. Owns `crates/nav/src/crowd_contact.rs`, `crates/nav/src/lib.rs`.
  Accept: `cargo test -p engine-nav --lib crowd_contact` passes: 256 discs pushed through a 2 m gate keep every overlap under 5 mm after the pass, pressure peaks at the gate, the step allocates nothing warm, 512 discs step under 1 ms on the build PC (bench line printed), two runs give identical bytes.
  F2P: no crowd contact module.
  P2P: golden d57b5c8ae3096614 x2.
  needs: TC4.
- [ ] TC6 physfx: ragdolls, limbs and debris use the layer table. Every Rapier collider in `crates/physfx` carries the group and mask from `engine_physics::layers` as `InteractionGroups`, so a falling body lands on a crate, debris hits bodies, and debris never meets cloth. Donor: Rapier `src/geometry/interaction_groups.rs` (Apache-2.0), read 2026-10-20. Tier A. Size S. Owns `crates/physfx/src/`.
  Accept: `cargo test -p engine-physfx --lib layers` passes: a ragdoll dropped on a crate rests on it, a debris piece and a cloth-layer probe never generate a contact pair, and the existing physfx tests stay green.
  F2P: physfx has no group table (0 `InteractionGroups` hits).
  P2P: golden d57b5c8ae3096614 x2 (physfx is never hashed).
  needs: none.
- [ ] TC7 game: lean and brace look. When TC3 or TC5 reports pressure on a slot, the look tilts the upper body away from the push and puts the near hand on the pusher's shoulder with two-bone IK; it eases out over 0.25 s. Presentation only. Donor: Overgrowth `Data/Scripts/aschar.as` tilt and target tilt (Apache-2.0), read 2026-10-20. Tier B. Size M. Owns `crates/game/src/fighter_look.rs`.
  Accept: `cargo test -p engine-game --features app --lib lean` passes: a slot pushed from its left tilts its torso joint right by 4 to 15 degrees within 10 ticks and returns under 1 degree 30 ticks after the push stops; a window capture pair (pushed vs free) is opened.
  F2P: pressure does not change the pose.
  P2P: `capes_never_reach_the_authority_checkpoints` green; golden x2.
  needs: TC3.
- [ ] POV1 render: the bodycam lens. One post pass applies barrel distortion (k1), chromatic aberration (RGB shift), vignette and film grain from a `LensProfile` table with presets eye, helmet and chest; eye is identity and the default. A CPU oracle and a naga gate prove the WGSL like GS43. Donor: three.js `examples/jsm/shaders/RGBShiftShader.js`, `VignetteShader.js`, `FilmShader.js` (MIT), read 2026-10-20. Tier A. Size M. Owns `crates/render/src/lens.rs`, `crates/render/src/shaders/post/lens.wgsl`, `crates/render/src/lib.rs`.
  Accept: `cargo test -p engine-render --lib lens` passes: the eye profile leaves a test frame byte-equal, the helmet profile moves a corner texel inward by the oracle's barrel amount and splits R and B by the CA amount, grain is seeded by frame index so two runs match; the naga gate validates the shader; a helmet capture beside an eye capture is opened.
  F2P: no lens pass.
  P2P: golden d57b5c8ae3096614 x2; `--lib post` green.
  needs: none.
- [ ] POV2 game: the helmet mount. The first-person camera rides a halflife spring attached to the head joint (helmet) or the torso joint (chest) while the aim ray stays on the FV1 eye, so the view shakes and the aim does not. Halflives come from the `touch-and-pov.md` table (helmet 0.06 s, chest 0.10 s). Reuse the fight camera's halflife spring, no new spring type. Tier A. Size S. Owns `crates/game/src/fps_view.rs`.
  Accept: `cargo test -p engine-game --lib fps_view` passes with a test where a scripted head bob of 3 cm moves the camera by more than 1 cm and the aim ray by 0, chest moves more than helmet, and the same tape gives the same camera bytes twice.
  F2P: camera and aim are one pose.
  P2P: golden x2.
  needs: FV3.
- [ ] POV3 game: viewmodel weight. The held weapon lags the look by a spring (`sway_halflife_s` 0.12), breathes (`breath_amp_deg` 0.15, zero while breath is held) and sways with grip; prone sways less than crouch, crouch less than stand. Donor: Arma 3 OPREP weapon sway post (ideas), GodotSpringDamper (MIT) read 2026-10-20. Tier B. Size S. Owns `crates/game/src/fps_view.rs`.
  Accept: `cargo test -p engine-game --lib viewmodel` passes: a 90-degree flick leaves the weapon behind the look and settles inside 0.5 s; breath hold drops breath amplitude to 0; stance order holds; replay twice equal.
  F2P: the weapon is glued to the view.
  P2P: golden x2.
  needs: POV2.
- [ ] POV4 render: second cameras to textures. A throttled render-to-texture camera (default 15 Hz, 512 px) that the game points at an optiwand tip under a door or a teammate's helmet, drawn as a picture-in-picture quad; a budget line prints its GPU ms. Donor: Godot SubViewport docs (ideas), read 2026-10-20. Tier B. Size M. Owns `crates/render/src/present/rtt.rs`, `crates/render/src/present.rs`.
  Accept: `cargo test -p engine-render --features window --lib rtt` passes on the PC: the second camera renders a known scene into its texture, skips frames at the set rate, and its GPU ms prints; a capture with the PiP quad is opened.
  F2P: one camera per frame.
  P2P: golden x2; `validation_errors=0` on the arena run.
  needs: POV1.
- [ ] POV5 game: flashbang and suppression. A flash cue whites out the view and decays over 2 to 4 s by distance and facing; the audio mix gets a low-pass and a 4 kHz ring for the same time; near misses blur the screen edges. Presentation only. Tier B. Size S. Owns `crates/game/src/fps_view.rs`, `crates/game/src/audio_mix.rs`.
  Accept: `cargo test -p engine-game --lib flash` passes: a flash facing the eye at 2 m holds full white at least 1 s and is gone by 4 s, a flash behind the eye is weaker, the ring and low-pass ramp out on the same curve, replay twice equal.
  F2P: no flash response.
  P2P: golden x2.
  needs: POV1.
- [ ] POV6 game: comfort options. A FOV restrictor that narrows the view up to 10% during fast turns, and sliders for shake, bob, CA, grain and FOV (defaults low), stored in the settings file. Donor: Fernandes and Feiner, IEEE VR 2016 (paper). Tier B. Size S. Owns `crates/game/src/fps_view.rs`, `crates/ui/src/settings.rs`.
  Accept: `cargo test -p engine-game --lib comfort` passes: a 180 deg/s turn narrows the FOV by 5 to 10% and it recovers within 0.5 s; each slider at 0 removes its effect; settings round-trip byte-equal.
  F2P: no comfort controls.
  P2P: golden x2.
  needs: POV2.
- [ ] POV7 proof: the helmet-cam tape. A scripted 600-tick first-person run (walk, turn, flash, optiwand peek) records eye and camera bytes with lens on and off, writes `target/reports/pov-<stamp>.md` with the digests and four captures, and runs twice equal. Extends FV6. Tier B. Size S. Owns `crates/game/src/fps_view.rs`, `crates/game/tests/it/`.
  Accept: `cargo test -p engine-game --test it pov_tape` passes twice with identical digests; the report lists four opened captures.
  F2P: no POV tape.
  P2P: golden x2.
  needs: FV6, POV1, POV2, POV4, POV5.

#### Build CPU (owner 2026-10-21: "optimize our cpu usage af ... 27 cargo, 20 rustc")

Plan: `docs/plan/build-cpu.md`. One pool of compile slots for every cargo on the PC, then the cheap levers. sccache stays dropped (OPT9 receipt).

- [x] OPT37 tools: every cargo our tools start joins the slot pool. New `tools/jobserver.mjs` exports `poolEnv()`, which returns `{ CARGO_MAKEFLAGS: "-j --jobserver-auth=engine2040-jobserver" }` when `pwsh -NoProfile -File tools/jobserver.ps1 status` exits 0, and `{}` otherwise (and on non-Windows). `tools/gh-check.mjs` (`runLocal`) and `tools/proof.mjs` (`cargo`) merge it into the child env. Tier A. Size S. Owns `tools/jobserver.mjs`, `tools/jobserver.test.mjs`, `tools/jobserver.ps1`, `tools/gh-check.mjs`, `tools/proof.mjs`. Landed 6bfb92ed.
  Accept: `node --test tools/jobserver.test.mjs` passes: with a 1-slot holder under a test name the env names it, with none the env is empty, and a cargo check of a 7-dependency scratch crate under the 1-slot pool never shows more than 2 of its rustc at once (polled every 300 ms); `node tools/workspace-check.mjs` PASS.
  F2P: each cargo starts its own 12-job pool; four live builds ask for 48 jobs on 12 cores.
  P2P: with no holder running every tool behaves as before; golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] OPT38 proof: the local golden verify builds on `perf-test` (thin LTO, 16 units), not fat-LTO `release`. `tools/proof.mjs` runs `cargo run -p engine2040 --locked --profile perf-test` for `--verify`, `--replay` and the backend check; CI's release-proof leg keeps `--release`. If the keeper's thin-LTO `product` profile (`docs/research/build-cpu-2026-10-21/CARD.md`) lands first, use it instead so the golden shares that target dir. Tier A. Size S. Owns `tools/proof.mjs`.
  Accept: `node tools/gh-check.mjs cargo test -p engine2040 --locked --profile perf-test --test proof_cli default_and_max_seed` and the same line with `--release` pass twice each (1 test each; it asserts `final_hash` `d57b5c8ae3096614` from the binary built in that profile); `node --test tools/proof.test.mjs` passes; the receipt line gives the two build times.
  F2P: every local golden check pays the fat-LTO, one-unit relink in `target/release`.
  P2P: golden d57b5c8ae3096614 x2 on both profiles.
  needs: none.
- [ ] OPT39 build: dependencies build without debug info in dev. Add `debug = false` to `[profile.dev.package."*"]` in root `Cargo.toml` (workspace crates keep line tables). Tier B. Size S. Owns `Cargo.toml`, `docs/SMOOTH-BUILD.md`.
  Accept: two cold `cargo build -p engine-game --timings` runs before and two after in their own target dirs, and the receipt line gives the four wall times and `target/` sizes; the row stays only if the after-mean is at least 10% faster or 15% smaller; `cargo test -p engine-core --lib` passes.
  F2P: dependencies carry line tables nobody reads.
  P2P: workspace-crate backtraces keep file:line; golden x2.
  needs: none.
- [x] OPT40 tools: builds run below the desktop. `tools/gh-check.mjs` and `tools/proof.mjs` call `os.setPriority(os.constants.priority.PRIORITY_BELOW_NORMAL)` on their own process before they spawn cargo, so cargo and every rustc under it inherit below-normal on Windows. Tier B. Size S. Owns `tools/jobserver.mjs`, `tools/jobserver.test.mjs`, `tools/gh-check.mjs`, `tools/proof.mjs`. Landed 6bfb92ed.
  Accept: `node --test tools/jobserver.test.mjs` passes with a test where a process that called `lowerPriority()` spawns a child and the child reads its own `os.getPriority()` as below normal (10).
  F2P: a build at 100% CPU takes the same priority as the game window and the editor.
  P2P: build results unchanged.
  needs: none.
- [ ] OPT41 tools: spikes wait for a free slot. `tools/compile-cache-spike.mjs` and every `tools/*spike*.mjs` refuse to start (exit 2, one line) when `tools/jobserver.ps1 status` reports 0 free slots, unless `--force`. Tier C. Size S. Owns `tools/compile-cache-spike.mjs`, `tools/jobserver.mjs`.
  Accept: `node --test tools/jobserver.test.mjs` passes with a test that holds every slot of a test pool and sees the spike refuse.
  F2P: an old spike dir compiles `syn` at opt-level 3 next to live work.
  P2P: with no pool or free slots, spikes run as before.
  needs: OPT37.
- [x] OPT43 tools: gh-check reuses every result by what the line builds. A result is keyed by the line, the runner and its inputs (the `-p` crates with their dependency closure, root manifests, lock, `.cargo/config.toml`, toolchain, `content/`, `fixtures/`, paths the crates' code names outside them); the same line on unchanged inputs answers from `target/gh-check/results/` (pass 24 h, fail 30 min, `--fresh` reruns), a line another call is running is joined, and the same local line already building is waited for. Tier A. Size M. Owns `tools/gh-check.mjs`, `tools/gh-check.test.mjs`. Landed 137b6c2d.
  Accept: `node --test tools/gh-check.test.mjs` passes (the key ignores edits outside the closure and changes on edits inside it, a kept pass answers without a build, a fail expires at 30 min, a vacuous pass is never kept); a live third identical call answers from the kept result.
  F2P: every call builds again, and two AIs asking the same line build it twice.
  P2P: gh-check's route tests pass.
  needs: none.
- [x] OPT44 tools: feature builds go to a Windows runner, not the PC. Lines that need Windows or a window|app|ui|device feature only to build (check, clippy, `--no-run`, tests that open no GPU or window, `cfg(windows)` tests) run on `windows-latest` through check.yml's `os` input; only GPU, window and sound-device tests stay on the PC. ci.yml's window-compile job saves the `win-check` cache and builds the engine-game app tests for those runs to restore. Tier A. Size S. Owns `tools/gh-check.mjs`, `.github/workflows/check.yml`, `.github/workflows/ci.yml`, `skills/github-check/SKILL.md`, `AGENTS.md`. Landed 137b6c2d.
  Accept: `node --test tools/gh-check.test.mjs` passes (route sends an app clippy line to windows-latest and a gpu filter to the PC); a live `cargo check -p engine-audio --features device` runs on the Windows runner.
  F2P: every app, window, ui and device line compiled on the PC.
  P2P: Linux lines dispatch without the `os` input.
  needs: none.
- [ ] OPT42 build: nightly front-end threads and Cranelift for dev, spike only. LATER: owner go (needs a nightly toolchain on the PC). In `target/spike-nightly`, time a cold and a touch rebuild of `engine-game` with `-Zthreads=8`, and with `codegen-backend = "cranelift"`, against stable dev. Tier C. Size S. Owns `docs/receipts/`.
  Accept: a receipt with six wall times (3 setups x 2 runs) and the run spread.
  F2P: no nightly numbers.
  P2P: the default toolchain and profiles unchanged.
  needs: none.
