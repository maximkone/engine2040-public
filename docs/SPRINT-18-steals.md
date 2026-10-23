### Steals and potato tier (owner 2026-10-11, `docs/plan/potato.md`; research `docs/research/steals-2026-10-11/engine-landscape.md`)

Boarded 2026-10-11 from two research waves (74 scouts). Owner: "all games looking
good af visually yet run on potato" (weak GPU, weak RAM, weak CPU). The 3050 / 16 GB
box is the comfortable tier; the floor is an iGPU or a 2-4 GB card with 4-8 GB RAM.
Checked against the board and `.opencode/modmap.md` first: the engine already has the
graph, dyn-res, baked GI, decals, steam, crash, director, EQS and the editor gizmo.
These rows are only what was missing, plus the gates that stop finished modules from
staying unwired. Numbers marked "proposal" are starting values, not owner decisions.
Every row is one crate, small, with a named donor and a test that fails before.

**Wiring gates (a landed module must be reachable)**

- [x] GS1 tools: wiring scan — find `pub` items with no production user. For every Landed ceaca3d2.
  `crates/*/src/**/*.rs`, list each `pub fn|struct|enum|trait|const` and count word
  hits in other files' non-test code (not `tests/`, not inside `#[cfg(test)]`) and
  in other crates' `src`; items with zero are "islands". rustc `dead_code` is per
  crate, so cross-crate islands pass it (donor idea: cpg314/cargo-workspace-unused-pub,
  1419 candidates down to 43). Diff: regular. Tier A. Size M. Owns
  `tools/wiring-scan.mjs`, `tools/wiring-scan.test.mjs`, `tools/fixtures/wiring/**`.
  Accept: a fixture of three pub fns (one used by another crate's src, one used
  only by a test, one used nowhere) reports exactly the last two; `--json` is
  byte-equal x2; on the repo it prints the ten crates with most islands and writes
  `target/reports/wiring-scan.json`; the first run writes `tools/wiring-baseline.json`
  and `--baseline` exits 1 when any crate's island count rises. F2P: no
  wiring-scan tool. P2P: `node --test tools/wiring-scan.test.mjs`;
  `node tools/workspace-check.mjs` PASS. Landed so far (2026-10-12 check): ec5d0f45 (scanner and fixtures). Progress 2026-10-23: every Accept part checks green — 18 tests (`node --test tools/wiring-scan.test.mjs`: `--json` byte-equal x2, top ten by count, a baseline rise detected, the committed pin), the report written to `target/reports/wiring-scan.json`, the pin refreshed to `total: 6934` over 41 crates, and a root that is not a directory now exits 2 instead of passing on zero islands (HEAD exited 0 on `tools/wiring-baseline.json`, which is what `docs/plan/potato.md` had been reading as a green gate). Recheck later on 2026-10-23, after the night's landings: `--check` reads 6984 against that pin, and the rise is committed code rather than a stale number - perf +14, nav +11, game +9, endpoint +9, world +3, render +3, web +3, proof +2, assets +2, destruction +2, anim +2, arena +1, material +1. Two lanes were mid-edit when it ran (nav's platoon and twelve endpoint files), so the pin stays at 6934: the seat that lands one of those crates refreshes it in its own commit, the way the tool's header asks.
- [x] GS2 tools: wiring coverage, nightly and report only — which functions does the Progress 2026-10-17: tool and 15 tests green (node --test tools/wiring-cov.test.mjs, NOOP exit 2 without llvm-cov, canned fixture grouped list x2, fake-cargo plan); f1456ff. Left: the real run, `cargo llvm-cov` is not installed on this PC so target/reports/wiring-cov.json is still the canned fixture.
  real arena never run? `cargo llvm-cov run` on the arena binary for a short
  scripted run (same flags as FC1: `--scene arena --fighters 8 --frames 300 --exit`),
  then list zero-hit functions per crate for render, game, anim, audio, ai (donor:
  taiki-e/cargo-llvm-cov, `--json`; run the bin, not `cargo test`, because test-only
  callers must not count). It is a heavy instrumented build, so it never runs in the
  40-seat loop. Diff: regular. Tier A. Size M. Owns `tools/wiring-cov.mjs`,
  `tools/wiring-cov.test.mjs`. Accept: when `cargo llvm-cov --version` fails the tool
  prints the install line and exits 2 (NOOP); a canned llvm-cov JSON fixture gives
  the same grouped zero-hit list x2 and groups by crate; the real run writes
  `target/reports/wiring-cov.json`. F2P: no such report. P2P: `node --test
  tools/wiring-cov.test.mjs` green. Progress 2026-10-19: `cargo-llvm-cov` 0.9.1
  and `llvm-tools-preview` are installed; the real run still yields no usable data on
  this PC. `cargo llvm-cov run --no-report -p engine-game --features app --bin arena --
  --scene arena --fighters 8 --frames 300 --exit` (isolated
  `CARGO_LLVM_COV_TARGET_DIR=target/llvm-cov-target`) writes a 2.16 MB
  `engine2040-<pid>-.profraw` whose `llvm-profdata show --counts` reads 13,697
  functions with total count 0, and `cargo llvm-cov report --json` exports `files: 0`
  (the tool then groups 6,474 zero-hit fns, all zero). Two causes: normal-mode
  profraws stay 0 bytes because `crates/game/src/bin/arena.rs:690` leaves via
  `std::process::exit` (ExitProcess on Windows skips the LLVM writer, continuous mode
  `LLVM_PROFILE_FILE_NAME=engine2040-%p-%c.profraw` only makes an all-zero file), and
  the counter layout itself is unreadable on 1.97.1 MSVC: the raw route
  (`RUSTFLAGS=-Cinstrument-coverage`, `CARGO_INCREMENTAL=0`, isolated dir) on a small
  `engine-core` test binary gives a clean exit and a profraw whose
  `llvm-profdata show` reports max count 209,510,251 over 529 functions, the signature
  of 64-bit reads over 32-bit counters. The tool, its 15 tests and the `--from-json`
  route stay green; the all-zero export was discarded, not kept as the report. Left:
  run the three `cargoPlan` calls on Linux (the same line is the standard path there)
  or fix the arena exit path, then write `target/reports/wiring-cov.json`. Progress 2026-10-23:
  the exit path is fixed (`a5d149aa5`): `main` returns `ExitCode`, so Windows runs the CRT
  exit path and the profile flushes; the only `std::process::exit(1)` left is the wedged-run
  watchdog (`crates/game/src/bin/arena.rs:784`), which has no profile worth saving. So the
  first cause is closed and the row waits on the second (counter layout on 1.97.1 MSVC) or on
  the Linux `cargoPlan` route, run on a game crate with no other seat's edits in it. Landed 2026-10-23: the real run writes `target/reports/wiring-cov.json`. The blocker was the arena's own exit: `std::process::exit(code)` at the end of `main` ends the process without the CRT exit path, so an instrumented build wrote a 0-byte `.profraw` (measured both ways on this PC, 0 bytes for `process::exit` and 464 bytes for a returned `ExitCode`); `main` now returns `std::process::ExitCode` and the wedged-loop watchdog keeps its forced exit. `cargo-llvm-cov 0.9.1` on this PC then ran the whole plan (`llvm-cov clean --workspace`, `llvm-cov run -p engine-game --features app --bin arena -- --scene arena --fighters 8 --frames 300 --exit`, `llvm-cov report --json`) on the RTX 3050 and the report is real, not the old all-zero export: 8069 functions in the row's 5 crates, 6069 zero-hit, 2000 hit (engine-game 3017 total, engine-render 3217, engine-anim 732, engine-ai 552, engine-audio 551). The tool's own 15 tests stay green (`node --test tools/wiring-cov.test.mjs`, 15/15), so NOOP without llvm-cov and the canned fixture are unchanged. Clippy `-p engine-game --features app --bin arena -- -D warnings` PASS on the Windows runner (run 39861858337, 2026-10-23), after `crates/assets/src/pack_clip.rs` was made clippy-clean under `import`.
- [x] GS3 tools: pass floors — the product check already reads `name=Xms` per pass Landed d74e785f.
  (`tools/product-check.mjs` `passes`). Add `tools/pass-bar.json` listing each look
  pass (sky, ibl, shadow, gtao, bloom, taa, particles, decals) as `required` or
  `pending`; a `required` pass with no ms or 0 ms over the run files
  `bug:product-pass-<name>`; `pending` only prints. Each LF row flips its pass to
  `required` in its own Accept. Diff: regular. Tier A. Size S. Owns the pass hunk of
  `tools/product-check.mjs`, new `tools/pass-bar.json`, the product-check test.
  Accept: a canned log with `bloom` required and absent is red and names
  `bug:product-pass-bloom`; the same log with every required pass above 0 ms is
  green; `pending` entries never turn it red. F2P: a missing pass is ignored today.
  P2P: `node --test tools/product-check.test.mjs` green.
- [x] GS4 tools: fake-pass guard in `packet-gate` — a diff that edits or removes Landed 6d889933.
  existing assertion lines (`assert*!(`, expected-value literals under `tests/` or
  in `#[cfg(test)]`) while code outside tests also changes prints
  `FAKE-PASS-RISK: n assertion lines edited` and exits 1, unless the packet row
  carries a `TESTS:` mark (EvilGenie 2511.21654: edited tests are the first
  fake-pass signal). Diff: good. Tier A. Size S. Owns `tools/packet-gate.mjs` and
  its test. Accept: three canned diffs — adds a new test: pass; changes
  `assert_eq!(f(), 5)` to `6` and edits `f`: fail; the same with `TESTS:`: pass.
  F2P: the edited-assertion diff passes today. P2P: `node --test
  tools/packet-gate.test.mjs` green.
- [x] GS5 render: one pipeline at a time on DX12 — wgpu #10547 (open, no PR): the FXC Landed 1d01a41. cache_gate_dx12_serializes_eight_threads (peak 1), cache_gate_vulkan_allows_parallel_eight_threads, cache_gate_gl_allows_parallel_eight_threads, warm_keys_gated_routes_compiles_through_gate, async_submit_dx12_serializes_second_submit_refused_until_resolve green in engine-render lib 719 passed, run 39056588691.
  shader cache shares `ID3DBlob` copies across threads with a non-atomic refcount;
  3 of 221 runs hit use-after-free. Add `PipelineGate::enter(backend)` in
  `cache.rs`: on `Dx12` it serializes `create_*_pipeline` (max 1 in flight), on other
  backends it allows many. Diff: good. Tier A. Size S. Owns the creation hunks of
  `crates/render/src/cache.rs`. Accept: 8 threads creating through the gate record
  max in-flight 1 on `Dx12` and above 1 on `Vulkan`; the async compile path in
  `cache.rs` goes through the gate. F2P: no gate; the same test shows 8 in flight
  on `Dx12`. P2P: `cargo test -p engine-render --lib cache` green; golden unchanged.

**Potato tier (new gate G9: the same game on a weak PC)**

- [x] GS6 render: choose the adapter on purpose — `presenter.rs` takes Landed 1d01a41. adapter.rs tests (auto_picks_the_discrete_gpu, integrated_picks_the_uhd, name_fragment_picks_the_uhd, cpu_only_list_is_only_software_naming_it, backend_order_is_dx12_then_vulkan_then_gl, describe_prints_the_startup_line) green in engine-render lib 719 passed, run 39056588691; presenter.rs reads choice_from_env at 3 sites and holds no PowerPreference::HighPerformance.
  `PowerPreference::HighPerformance` at three call sites, and on an Optimus laptop
  wgpu #257 returned the Intel adapter with the NVIDIA card missing; Bevy #13113
  silently got the CPU "Basic Render Driver". New `crates/render/src/adapter.rs`:
  `AdapterChoice {Auto, Discrete, Integrated, Name(String)}` read from env
  `E2040_ADAPTER`; pure `choose(&[AdapterSummary], choice) -> Result<usize,
  AdapterError>` (never a `DeviceType::Cpu` adapter unless `E2040_ALLOW_SOFTWARE=1`;
  backend order Dx12, Vulkan, Gl); `describe()` prints
  `adapter: <name> <backend> <type> vendor=0x<id>`. Wire the three presenter sites.
  Diff: good. Tier A. Size M. Owns `crates/render/src/adapter.rs`, the request hunks of
  `crates/render/src/present/gpu_present/presenter.rs`. Accept: over a canned list
  [UHD integrated, RTX 3050 discrete, Basic Render Driver cpu]: Auto picks the RTX,
  Integrated picks the UHD, `Name("uhd")` picks the UHD, a list with only the cpu
  adapter is `Err(OnlySoftware)` naming it; `presenter.rs` has no
  `PowerPreference::HighPerformance` left. F2P: presenter hardcodes it. P2P: `cargo
  test -p engine-render --lib` green; golden unchanged.
- [x] GS7 game: pick the dGPU on hybrid laptops — export `NvOptimusEnablement = 1` and Progress 2026-10-17: the two statics exist (`NvOptimusEnablement`, `AmdPowerXpressRequestHighPerformance` in crates/game/src/crash.rs, 1d01a41) but there is no crates/game/build.rs /EXPORT link arg and `node tools/pe-exports.mjs target/release/arena.exe` prints nothing (exit 0), so neither name is exported yet. needs GS8 is landed. Landed c9115185.
  `AmdPowerXpressRequestHighPerformance = 1` from `arena.exe` (donor: GLFW
  `src/win32_init.c`; the exe, not a DLL, must export them). A `static` marked
  `#[no_mangle]` plus `crates/game/build.rs` printing
  `cargo:rustc-link-arg-bin=arena=/EXPORT:NvOptimusEnablement,DATA` (and the AMD
  one), only on `windows` + `msvc`. Diff: good. Tier A. Size S. Owns
  `crates/game/build.rs` and the statics hunk of `crates/game/src/bin/arena.rs`.
  Accept: after a release build `node tools/pe-exports.mjs target/release/arena.exe`
  lists both names. F2P: arena exports neither. P2P: `cargo check -p engine-game
  --features app` green. needs: GS8. Progress 2026-10-18: landed c9115185 (build.rs /EXPORT link args for the arena bin on windows-msvc, the two statics moved from crash.rs to bin/arena.rs). node tools/pe-exports.mjs target/debug/arena.exe built from this tree lists AmdPowerXpressRequestHighPerformance and NvOptimusEnablement; the same statics plus build.rs in a scratch package listed both on a release build. Left: the literal release arena.exe proof (a release build of the whole arena was still compiling on the loaded PC); tick it with node tools/pe-exports.mjs target/release/arena.exe.
  Proof 2026-10-23: `cargo build --release -p engine-game --features app --bin arena` finished on this PC (1m28s) and `node tools/pe-exports.mjs target/release/arena.exe` printed both names, in this order: `AmdPowerXpressRequestHighPerformance`, `NvOptimusEnablement`. The row's accept is met by that release binary from this tree; no source change was needed for the proof.
- [x] GS8 tools: read a PE export table — `tools/pe-exports.mjs <exe|dll>` parses the Landed 2746b129.
  export directory (about 80 lines, no dependency) and prints one name per line.
  Diff: regular. Tier A. Size S. Owns `tools/pe-exports.mjs`,
  `tools/pe-exports.test.mjs`. Accept: on Windows, `C:\Windows\System32\kernel32.dll`
  lists `CreateFileW`; a file that is not a PE exits 2 with a message; the test skips
  off Windows. F2P: no tool. P2P: `node --test tools/pe-exports.test.mjs` green.
- [ ] GS9 render: prove the weakest limits — env `E2040_LIMITS=downlevel` makes
  `request_device` use `wgpu::Limits::downlevel_defaults()` clamped by the adapter's
  own limits (4 storage buffers per stage, 16 KiB uniform binding, 2048 px textures).
  Pure `limits_for(mode, adapter_limits)`. A GPU test renders one frame under it and
  writes the passes that needed more to `target/reports/downlevel-gaps.md`. Donor:
  wgpu-types `DownlevelFlags`. Diff: good. Tier A. Size M. Owns the
  `request_device` hunk of `presenter.rs` and a test in `present/tests_gpu.rs`.
  Accept: `limits_for` takes the lower of both per field; the GPU test (skips without
  an adapter via `test_gpu.rs`) renders one frame with zero validation errors or
  lists the failing passes. F2P: `request_device` always asks for the adapter's full
  limits. P2P: `cargo test -p engine-render --lib` green. needs: GS6.
- [x] GS10 render: one quality table — new `crates/render/src/quality.rs`: Landed 1d01a41. quality_monotone_tiers, quality_units_nonempty, quality_cut_order_fixed, quality_hash_stable_x2 green in engine-render lib 719 passed, run 39056588691.
  `Quality {Potato, Comfort, High}` and `Knobs {render_scale_pct, shadow_cascades,
  shadow_map_px, gtao, bloom_half_res, taa, msaa, aniso, mip_bias_x100,
  stream_pool_mb, particle_cap, decal_cap, crowd_cap}` as consts, plus `CUT_ORDER`
  (what to drop first). Proposal for Potato from Unreal scalability level 0:
  67% scale, 1 cascade, 1024 px, gtao off, aniso 0, mip bias +1.0, pool 200 MB,
  particles 512, decals 64, crowd cards 1500. Comfort: 100%, 3 cascades, 2048, gtao
  half, aniso 4, pool 700 MB, particles 4096, decals 256, crowd 5000. Cut order:
  crowd, particles, aniso and motion blur, SSR, AO, shadow cascades, render scale.
  `PLF10` (game presets) reads it. Diff: regular. Tier A. Size S. Owns
  `crates/render/src/quality.rs` and its `lib.rs` `mod` line. Accept: every knob is
  monotone Potato <= Comfort <= High; each knob has a unit string; `CUT_ORDER` is
  fixed; the table hashes x2. F2P: no quality table. P2P: `cargo test -p
  engine-render --lib` green.
- [x] GS11 render: GPU to tier rules — new `crates/render/src/tier_rules.rs`, a rule
  table in the Chromium `gpu_control_list` shape: `vendor_id`, `device_type`, VRAM MB
  range, driver version `op`, result `Quality` plus named workarounds, first match
  wins, with `exceptions`. Defaults: integrated is Potato; discrete under 4 GB is
  Potato; discrete 4 GB or more is Comfort; a CPU adapter is `Refuse`. Diff: regular.
  Tier A. Size S. Owns `crates/render/src/tier_rules.rs`. Accept: UHD 630 (0x8086,
  integrated) gives Potato; RTX 3050 (0x10de, 6144 MB) gives Comfort; GTX 1050
  (2048 MB) gives Potato; an unknown discrete 8 GB gives Comfort; Basic Render Driver
  gives Refuse; x2 equal. F2P: no rules. P2P: `cargo test -p engine-render --lib`
  green. needs: GS10. Landed dc517ed. tier_rules_name_the_reference_gpus (UHD 630 Potato, RTX 3050 Comfort, GTX 1050 Potato, unknown 8 GB Comfort, Basic Render Driver Refuse), tier_rules_same_facts_give_the_same_rule_twice, tier_rules_vram_line_is_four_gb_and_unknown_is_not_trusted, tier_rules_driver_op_and_exception_steer_a_custom_table green in gh-check run 39101673996; engine-render --lib 763 passed run 39101797684. Caller: the adapter pick (presenter_pick, held by another session) reads decide next.
- [x] GS12 render: dynamic resolution control law — pure `crates/render/src/dynres_gov.rs`. Landed acdd233.
  `DynRes {budget_us, min_pct, max_pct, deadband_pct, cooldown_frames,
  panic_over_budget_frames}` and `step(&mut self, gpu_us) -> Option<u8>` (donor:
  Unreal `r.DynamicRes.*`: history window average, deadband, cooldown, panic drop
  after K consecutive over-budget frames that clears history, slow recovery). Integer
  only. `OPT20` and `aa_scale.rs` read it. Diff: regular. Tier A. Size S. Owns
  `crates/render/src/dynres_gov.rs`. Accept: steady at budget gives no change; a
  3x spike drops to the floor within K frames; after the spike it returns to 100 in
  at most 40 cooldown periods; x2 equal. F2P: no governor fn. P2P: `cargo test -p
  engine-render --lib` green.
- [x] GS13 perf: VRAM budget math — `crates/perf/src/vram.rs`, pure, from Dawn Landed 3513a7e7. Proof: cargo test -p engine-perf, run 39051586513.
  `UpdateVideoMemoryInfo`: `reservation = min(budget / 2, requested)`,
  `budget = (budget - reservation) * 95 / 100`, `usage = current - reservation`
  (saturating), plus `pressure_pct()`. The Windows `QueryVideoMemoryInfo` call that
  feeds it is a later orchestrator step (dependency feature). Diff: regular. Tier A.
  Size S. Owns `crates/perf/src/vram.rs` and its `lib.rs` line. Accept: budget 6 GiB,
  usage 2 GiB, requested 1 GiB gives reservation 1 GiB, budget 4.75 GiB, usage 1 GiB;
  usage above budget gives pressure over 100; x2 equal. F2P: no vram module. P2P:
  `cargo test -p engine-perf --lib` green.
- [x] GS14 perf: peak heap gate — extend `crates/perf/src/alloc.rs` with `peak_bytes` Landed 334b5700. peak_live_bytes in alloc_probe; runs 39051586513, 39051586814.
  if absent, and add `crates/perf/tests/it/heap_peak.rs`: run the 600-tick 8-fighter
  headless match and record peak live heap to `target/reports/heap-peak.csv`. The
  first run writes `crates/perf/baselines/heap_peak.txt`; later runs fail when peak
  grows more than 10%. Diff: regular. Tier A. Size S. Owns `crates/perf/src/alloc.rs`
  and `crates/perf/tests/it/heap_peak.rs`. Accept: peak is the same x2; a test that
  allocates 64 MiB extra trips the 10% rule. F2P: no peak number exists. P2P:
  `cargo test -p engine-perf` green; golden unchanged. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-perf --test it heap_peak::` 8 passed, run 39231141286.
- [x] GS15 assets: potato asset budget — new `crates/assets/src/budget.rs`: `Caps Landed 4c6d817.
  {max_tris, max_bones, max_influences, max_materials_per_skin, max_tex_px}` consts
  `POTATO` and `COMFORT`, and `check(&MeshStats, &Caps) -> Vec<BudgetFinding>` over a
  plain stats struct. Proposal (Unity desktop hero 1500-4000 tris, Unreal mobile 75
  bones and 4 influences, KayKit one shared atlas): fighter 4000 tris, crowd member
  1500, 75 bones per section, 4 influences, 1 material per skin, base colour 1024 px.
  Import wiring as gate 8 is a later row. Diff: regular. Tier A. Size S. Owns
  `crates/assets/src/budget.rs` and its `lib.rs` line. Accept: a 3,900-tri fighter
  passes and a 4,100-tri one reports `tris` with both numbers; a section with 76 bones
  reports `bones`; x2 equal. F2P: no budget module. P2P: `cargo test -p engine-assets
  --lib` green.
- [x] GS16 tools: the potato run — `node tools/potato-check.mjs` runs `arena.exe --scene Landed 178a0f5. node --test tools/potato-check.test.mjs 27 pass (canned arena plus fake tasklist exact potato line x2, NO_IGPU exit 2, --floor exit 1 on p50 over); real report target/reports/potato-2026-10-17.md (p50 7.969 ms, p99 13.163 ms, peak ws 1508 MB on the UHD). needs GS6 landed above.
  arena --fighters 8 --frames 600 --exit` with `E2040_ADAPTER=integrated` and
  `E2040_LIMITS=downlevel` on this PC's Intel UHD, samples the process working set
  every 250 ms with `tasklist`, reads the `arena frame-split` lines, and prints
  `potato: adapter=<name> p50_ms=.. p99_ms=.. peak_ws_mb=..`; writes
  `target/reports/potato-<date>.md`. `tools/potato-bar.json` holds proposal numbers
  (p50 50 ms, p99 80 ms, working set 3072 MB); without `--floor` it only prints
  PASS/FAIL per number. Diff: regular. Tier A. Size M. Owns `tools/potato-check.mjs`,
  `tools/potato-check.test.mjs`, `tools/potato-bar.json`. Accept: a canned arena
  output and a fake `tasklist` give the exact potato line x2; no integrated adapter
  prints `NO_IGPU` and exits 2; `--floor` exits 1 when p50 is over. F2P: no potato
  report. P2P: `node --test tools/potato-check.test.mjs` green. needs: GS6.
- [x] GS17 render: LOD dither cross-fade — no pop, no alpha sort (donor: keijiro Landed 0a4239e.
  CrossFadingLod). New `crates/render/src/lod_fade.rs`: `fade(dist, switch, band) ->
  u8`, `bayer4(x, y) -> u8`, `keep(fade, x, y) -> bool`; and `shaders/lod_dither.wgsl`
  with the same 4x4 matrix. Diff: regular. Tier A. Size S. Owns the two new files and
  the `lib.rs` line. Accept: over a 4x4 tile the kept share equals `ceil(fade / 16)`
  of 16 for every fade 0..=255; the WGSL file holds the same 16 values (string-compare
  test); x2 equal. F2P: no dither fade. P2P: `cargo test -p engine-render --lib`
  green.
- [x] GS18 render: GPU frustum cull slice 1, the cull shader — `shaders/cull.wgsl` Landed f737386. gpu_cull::tests::gpu_cull_matches_the_cpu_oracle_over_10k_spheres (GPU visible set equals cull_cpu over 10,000 seeded spheres incl. boundary cases) green on the RTX 3050 with draw.rs gpu_cull_cpu_boundary_keeps_and_culls, gpu_cull_cpu_10k_seeded_stable_x2, gpu_cull_indirect_args_are_5_u32 (engine-render --features window --lib, local PC run); no draw path uses it yet (GS18b).
  plus `crates/render/src/gpu_cull.rs`: one compute pass tests each static instance's - code cut 2026-10-16, recover via git log --follow -- crates/render/src/gpu_cull.rs
  bounding sphere against the six frustum planes and writes each draw's `instanceCount`
  (vkguide "compute culling": no count buffer, so no `MULTI_DRAW_INDIRECT_COUNT`, which an
  iGPU may lack). Donor: zeux/niagara `drawcull.comp.glsl` (about 140 lines: sphere vs
  frustum, `atomicAdd` slot). No draw path uses it yet. Diff: good. Tier A. Size M. Owns
  the two new files. Accept: over 10,000 seeded spheres incl. boundary cases the GPU
  visible set equals `cull_cpu` (read back in a test that skips without an adapter);
  indirect args are 5 u32; x2 equal. F2P: no GPU cull. P2P: `cargo test -p
  engine-render --lib` green; golden unchanged.
- [ ] GS18b render: GPU frustum cull slice 2, draws go indirect — the static instance
  draws in `crates/render/src/draw.rs` read GS18's indirect args
  (`draw_indexed_indirect`), the CPU cull stays the fallback (`E2040_CPU_CULL=1`).
  Diff: good. Tier A. Size M. Owns the cull hook in `crates/render/src/draw.rs`.
  Accept: the arena scene's frame draw count drops with GPU cull on; the same frame
  with `E2040_CPU_CULL=1` matches the CPU path; validation errors 0. F2P: draws are
  CPU-culled only. P2P: render lib green; golden unchanged. needs: GS18.
  Correction 2026-10-21: duplicate - PW05 owns this work now; PW05's row says it closes GS18b and gap B7 cut 5. This row is not ticked and is not built on its own; retire it when PW05 lands.
- [x] GS19 render: wound maps with a budget — donor naelstrof/SkinnedMeshDecals. New Landed 1d01a41. wound_drain_caps_per_frame_in_order (4,4,2), wound_evicts_oldest_map_over_cap, wound_stable_x2 green in engine-render lib 719 passed, run 39056588691; no draw caller yet (pure budget core).
  `crates/render/src/wound_map.rs`, pure: `WoundMaps::new(cap_bytes,
  paints_per_frame)`, `queue(char_id, uv_rect, kind)`, `drain(frame) -> Vec<Paint>`
  capped per frame, evict the oldest character's map when over the byte cap, a
  `DILATE_PX` const for seam fill. Order by (tick, char_id). Diff: regular. Tier A.
  Size M. Owns `crates/render/src/wound_map.rs`. Accept: 10 queued paints with a cap - code cut 2026-10-16, recover via git log --follow -- crates/render/src/wound_map.rs
  of 4 per frame drain 4, 4, 2 in order; over the byte cap the oldest map is evicted
  and reported; x2 equal. F2P: no wound map. P2P: `cargo test -p engine-render --lib`
  green.
- [x] GS20 assets: vertex animation textures for the crowd — new Landed 4c6d817.
  `crates/assets/src/vat.rs`: `bake_vat(frames: &[Vec<[f32; 3]>]) -> Vat` stores
  positions normalised in the clip bounding box as unorm16 RGBA (8 bytes per vertex
  per frame) with a header (verts, frames, bbox). Donors: Mefodei
  Render-Crowd-Of-Animated-Characters, luma.gl baked crowd. 5,000 verts x 30 frames is
  1.2 MB. Per-seat time offset and tint are instance data in a later shader row. Diff:
  regular. Tier A. Size M. Owns `crates/assets/src/vat.rs` and its `lib.rs` line.
  Accept: decode of any sample is within 1e-3 of the source; bytes = verts x frames x
  8 plus header; bake is byte-equal x2. F2P: no vat module. P2P: `cargo test -p
  engine-assets --lib` green.

**Content (the empty folder is the biggest "richer" gap)**

- [x] GS21 tools: `tools/fetch-content.mjs` — read a list file of `<source> <id> <res>` Landed 9dd2ea43.
  lines and download into `content/_incoming/<source>/<id>/`, each with a
  `<id>.license.json` sidecar (url, author, license, retrieved date). Sources: Poly
  Haven (`api.polyhaven.com/files/{id}`, send a User-Agent, never scrape pages),
  ambientCG (`ambientCG.com/api/v2/full_json`), plain URLs with md5. One request per
  second, skip files whose md5 matches. Diff: good. Tier A. Size M. Owns
  `tools/fetch-content.mjs`, `tools/fetch-content.test.mjs`. Accept: against a local
  http server fixture it downloads two files and verifies md5, writes the sidecars,
  and a second run downloads nothing; a wrong md5 deletes the file and exits 1;
  `--dry-run` prints total bytes and every license. F2P: no tool. P2P: `node --test
  tools/fetch-content.test.mjs` green.
- [x] GS22 content: fetch list v1 — `content/fetch.list` with 3 outdoor or Italian-style
  HDRIs (2k), 8 PBR sets (1k: sand, marble, travertine, brick, mosaic, bronze, leather,
  cloth) and 6 props, all CC0, chosen by querying the APIs. Progress 2026-10-22 (v1, 06124ea96): the list exists and its Accept passes live — `node tools/fetch-content.mjs --list content/fetch.list --dry-run` prints `total_bytes=218288 files=9`, nine `LICENSE CC0` lines and no duplicate ids. Landed: 3 Rome HDRIs at 2k (colosseum, borghese_gardens, castel_st_angelo_roof) and 6 PBR sets at 1k (aerial_sand, brick_4, blue_metal_plate, corrugated_iron, brown_leather, cotton_jersey), every row scouted live from the Poly Haven files API with its real md5. Owed for the tick (v2): marble and travertine (ambientCG serves the zips but its API no longer serves the licence or md5, and a url row needs the md5 before the licence column), mosaic (no CC0 mosaic on either API: ambientCG Mosaic002 answers 404, Poly Haven has no mosaic category), and the 6 props (Poly Haven models are per-file trees with no zip URL; ambientCG's model listing does not filter by type any more). P2P: `node --test tools/` is red tonight on the foreign board-cut fallout (finish.test.mjs names rows the 2026-10-22 cut moved off the shards), not on this row. Landed 0962defb7. v2: 42 rows, dry-run `total_bytes=38562005 files=42`, 42 CC0 rows, 42 unique ids, 0 rows without md5; real fetch 42 OK / 84 files with every md5 checked; `node --test "tools/*.test.mjs"` 1506 tests, 1491 pass, 0 fail, 15 skipped.
  Diff: regular. Tier A. Size
  S. Owns `content/fetch.list`. Accept: `node tools/fetch-content.mjs --dry-run
  content/fetch.list` prints at most 250 MB, every row CC0, no duplicate ids (needs
  network once). F2P: no list. P2P: `node --test tools/` green. needs: GS21.
- [x] GS23 content: fetch list v2, Roman and sound — scout-verified metadata only, open
  each page before use. Poly Pizza (CC-BY 3.0: Coliseum `4raJTBfhAfc`, Centurion
  `g2ckFsGszB`, legionnaire `czyveAaJ0hd`, Roman Arch `agiRDMN0VR`). Sketchfab CC-BY
  with `SKETCHFAB_TOKEN`, skipped silently without it: gladius `b123c4388a6f4307aff353c165c94f92`,
  `7261d334924f4a9b835ef4b15c1e16d0`, `eeae43c837484f1b8f98acc60a7b2597`; Colosseum
  `f9b800d747034c2d9908696c30b2338e`, `d9a8a96e16534bdab76be23c1f2c45fc`,
  `6459307c93e54394bed44b2583701dc6`, `39e39dba81354367af36a4b975dd06aa`. Freesound CC0
  crowd `264377`, `397434`, `511788`, `365132`, ambience `341684`, `341687`
  (`FREESOUND_TOKEN`, OAuth2). OpenGameArt StarNinjas sword clashes (CC0). CC-BY rows
  carry the required attribution text in the sidecar; no NC and no ND rows. Diff:
  regular. Tier A. Size S. Owns `content/fetch-roman.list`. Accept: `--dry-run` lists
  at least 12 rows, each CC0 or CC-BY, each CC-BY row with attribution text. F2P: no
  list. P2P: `node --test tools/` green. needs: GS21. Landed 22b7766ee. `content/fetch-roman.list`
  12 rows 24,945,995 bytes, 12 unique ids, every row CC-BY-3.0 (4, attribution text in the row
  and sidecar) or CC0 (8: 4 Freesound crowd/ambience previews + 2 OpenGameArt StarNinjas sword
  zips), no NC/ND row; `--dry-run` prints DRY total_bytes=24945995 files=12 and 4 ATTRIBUTION
  lines; real fetch to temp: 12 OK 24 files, all CC-BY sidecars carry the text; the tool now
  refuses a CC-BY row with no attribution and stays backwards compatible with the 42-row CC0
  list; `node --test tools/` green. Caveat: the Freesound rows fetch the preview mp3 URL (the
  licence text is on the page, not in the file).
- [x] GS24 tools: headless Blender cook — `tools/cook-char.mjs` runs `blender -b -P Landed f5bf4f7.
  tools/blender/cook_char.py -- <in> <out.glb>` (import BVH, FBX or glTF, export glTF),
  samples peak working set, prints `cook: peak_ws_mb=..`. Blender is heavy on RAM, so
  one process per clip. Diff: regular. Tier A. Size S. Owns `tools/cook-char.mjs`,
  `tools/blender/cook_char.py`, `tools/cook-char.test.mjs`. Accept: a stub `blender`
  on PATH receives exactly `-b -P <script> -- <in> <out>` and the runner reports the
  peak line and exits 0; no Blender on PATH exits 2 with the install hint. F2P: no
  runner. P2P: `node --test tools/cook-char.test.mjs` green.
- [x] GS25 tools: replay to video — `node tools/replay-to-mp4.mjs <tape> --out clip.mp4 Landed f6c0bce.
  [--max-ticks N]` runs the existing capture binary (`crates/game/src/bin/arena_capture.rs`;
  read its usage for the flags) to numbered PNGs, then `ffmpeg -framerate 60 -i
  frame_%06d.png`. It uses `h264_nvenc` when `ffmpeg -hide_banner -encoders` lists it
  (keeps the weak CPU free) and else `libx264 -preset veryfast -crf 18`. Donors: Crimson
  `replay render`, Godot `movie_writer_pngwav.cpp`. Diff: regular. Tier A. Size S. Owns
  `tools/replay-to-mp4.mjs`, `tools/replay-to-mp4.test.mjs`. Accept: with a stub capture
  and a stub `ffmpeg` the tool builds the exact argv for both encoder cases; no ffmpeg
  exits 2 with the install hint. F2P: no tool. P2P: `node --test
  tools/replay-to-mp4.test.mjs` green.

**Camera, AI, spatial, combat (small pure pieces with exact tests)**

- [x] GS26 game: camera arm that stops at pillars — new `crates/game/src/camera_arm.rs`: Landed 4c6d817.
  `arm_length(pivot, dir, max_len, radius, boxes: &[Aabb]) -> f32`, a sphere sweep via
  boxes inflated by the radius and a slab test, with a floor length (donor: Godot
  SpringArm3D). Presentation only, never hashed. `camera.rs` calls it in a later hunk.
  Diff: regular. Tier A. Size S. Owns `crates/game/src/camera_arm.rs` and its `lib.rs`
  line. Accept: radius 0.3 toward a box face 3.0 m away returns 2.7 m; no obstacle
  returns `max_len`; a pivot inside a box returns the floor; x2 equal. F2P: no arm
  fn. P2P: `cargo test -p engine-game --lib camera` green.
- [x] GS27 ai: reaction delay as data — new `crates/ai/src/reaction_delay.rs`: a ring Landed 4c6d817.
  `ObsRing<T, const N: usize>` returning the observation from `tick - d`, and
  `delay_ticks(skill_permille)` from 15 ticks (skill 0) to 4 ticks (skill 1000); a
  decision that is not ready repeats the last input (FightingICE 15-frame rule).
  Difficulty and personality become numbers and replay stays exact. Diff: regular.
  Tier A. Size S. Owns `crates/ai/src/reaction_delay.rs` and its `lib.rs` line.
  Accept: with obs pushed at ticks 0..30 and d = 9 the brain reads tick `t - 9`; d = 0
  reads now; `delay_ticks` is monotone; a not-ready decision returns the last input;
  x2 equal. F2P: no delay ring. P2P: `cargo test -p engine-ai --lib` green.
- [x] GS28 ai: hearing events — new `crates/ai/src/hearing.rs`: sound kinds as a table Landed 4c6d817.
  (proposal, scaled from yapb `sounds.cpp` Source units x 1.9 cm: gunfire and flashbang
  4000 cm for 120 ticks, blast 4000 cm for 180, door 2000 cm for 210, footstep 2400 cm
  for 90, quiet 1000 cm for 30); range holds for the first 40% of life then fades
  linearly; a louder noise replaces the current one. `heard_by(listener_cm, noise,
  tick) -> Option<u16>` integer strength. Diff: regular. Tier A. Size S. Owns
  `crates/ai/src/hearing.rs` and its `lib.rs` line. Accept: a gunshot at 3900 cm is
  heard at its start tick and not heard 130 ticks later; a footstep at 2500 cm is never
  heard; the fade is linear after tick 48 of 120; x2 equal. F2P: no hearing table.
  P2P: `cargo test -p engine-ai --lib` green.
- [x] GS29 ai: alert ladder — new `crates/ai/src/alert.rs`: `Alert {Unaware, Suspicious, Landed 4c6d817.
  Investigate, Track, Combat}`; inputs `Sense {sight: None|Partial|Confirmed, sound,
  door_closed}`; evidence expires after 300 ticks; Track heads for the fixed last-seen
  cell, never the live position (donor: esyfxy/godot-tactical-study awareness test; the
  numbers are one author's choices). Diff: regular. Tier A. Size S. Owns
  `crates/ai/src/alert.rs` and its `lib.rs` line. Accept: partial sight gives
  Suspicious; sound only gives Investigate; confirmed gives Combat; confirmed then lost
  gives Track and then Unaware after 300 ticks; a closed door blocks sight; x2 equal.
  F2P: no ladder. P2P: `cargo test -p engine-ai --lib` green.
- [x] GS30 spatial: isovist — new `crates/spatial/src/isovist.rs`: the visibility polygon Landed d10f8c58.
  from an integer-cm origin over wall segments by a rotational sweep with an active-edge
  heap (donor: kratocz/godot-visibility-polygon, CC0, itself from byronknoll
  visibility-polygon-js; split crossing segments first, since `break_intersections` is
  quadratic). Also `area_cm2()` as a readability score. Diff: good. Tier A. Size M.
  Owns `crates/spatial/src/isovist.rs` and its `lib.rs` line. Accept: from the centre of
  a 10 m square room the polygon is the four corners; a 1 m pillar adds shadow vertices
  behind it; a closed room has the area of the room; x2 equal. F2P: no isovist fn.
  P2P: `cargo test -p engine-spatial --lib` green. Scope: GS30 owns the 2D polygon
  plus `area_cm2` only; `view_pick`/`view_sees` stay SP65 in
  `crates/game/src/view_truth.rs`. needs: GS38.
- [x] GS38 spatial: isovist budget plus edge-semantics lock — the 3-rays-per-endpoint Landed f1cf426e.
  integer fan may stand; prove its bound, never rewrite the heap. Owns the test hunk
  of `crates/spatial/src/isovist.rs` only, never `sight.rs`. Accept: an arena-scale
  wall set (200+ walls) sweeps under 2 ms p95 with 0 allocs past setup, x2 equal;
  open ground reports 0; origin-on-endpoint documented and tested; the four GS30
  fixtures stay green. F2P: no budget number exists. P2P: `cargo test -p
  engine-spatial --lib isovist` green x2. needs: none.
- [x] GS31 combat: gladiator class rows — new `crates/combat/src/classes.rs` building Landed cea8a1cd.
  `Tables` rows for four classes from the Ludus Magnus table shape (read `fight.rs`
  `Tables` first): stamina cost and damage multiplier Attack 10 / 1.0, Heavy 20 / 1.5
  (20% stun), Defend 5 (halves damage), Special 30 / 2.0 with cooldown 3; class
  Retiarius 0.5x with an 80% net proc, Samnite 2.2x with 30% stun, Thraex 1.8x with a
  50% bleed, Murmillo 1.0x with shield cover. Integer permille. Diff: good. Tier A.
  Size M. Owns `crates/combat/src/classes.rs`. Accept: `validate_tables` passes for
  each; the 200-bout survey (`feel.rs` `BALANCE_BOUTS`) gives fighter-0 share between
  350 and 650 permille for every ordered pair; hash x2. F2P: no class rows. P2P:
  `cargo test -p engine-combat --lib` green; golden unchanged.
- [x] GS32 combat: telegraph floors report — `crates/combat/tests/it/telegraph_floor.rs` Landed 62dd9539.
  prints, for every move in `TABLES_V1`, signal + attack ms (`(startup + active) * 1000
  / 60`) against the Dark Souls 3 floors (340 ms, bait 240 ms, end pose 170 ms; Jerry
  Zhang, Anatomy of an Enemy Attack). Report only; `TELEGRAPH_STRICT=1` fails below
  340. Diff: regular. Tier A. Size S. Owns the new test file. Accept: the table is
  identical x2; strict mode fails while any heavy move is under 340 ms. F2P: no table.
  P2P: `cargo test -p engine-combat` green. needs: EG19. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-combat --test it telegraph_floor::` 8 passed, run 39231165909.
- [x] GS33 endpoint: `play.sweep` verb — seeded bots play N headless bouts and return one
  JSON envelope `{bouts, crashes, hash_divergences, stuck_ticks}` with evidence in
  `target/reports/sweep-<seed>.json` (donor: echoo19/hearth `probe_sweep`). Diff: good.
  Tier A. Size M. Owns `crates/endpoint/src/packs/sweep.rs` and one registration line.
  Accept: a 5-bout sweep with seed 7 returns the same envelope x2; an injected
  divergence reports `hash_divergences` 1; the verb is listed by `api.describe`. F2P:
  the verb is unknown. P2P: `cargo test -p engine-endpoint --lib` green; golden
  unchanged. needs: LF16. Landed ef52041f. play.sweep: sweep_five_bouts_seed_7_repeats_its_envelope, an_injected_divergence_reports_one_hash_divergence, a_refused_bout_is_a_counted_crash, the_verb_is_listed_and_writes_its_evidence green, router pin 65 (run 39109659487).

**Planner row (promotes the backlog)**

- [ ] PLN30 plan: promote the landscape ideas to rows — one cluster per run (A render,
  B motion and feel, C ai audio ui, D tools loop data, E content ship). Take at most 8
  ideas from `docs/plan/landscape-ideas.md`; for each read the donor file, check
  `.opencode/modmap.md` for an existing module, run `node tools/board-slice.mjs --find`
  for a duplicate, then write one row in board format (one crate, at most 200 changed
  lines, a named donor file, `F2P:` and `P2P:`). Append them to `docs/SPRINT-18-steals.md`
  as GS34 and up and delete the promoted lines. The orchestrator adds the new IDs to the
  FOCUS group 7 line. Tier A. Size S. Owns `docs/plan/landscape-ideas.md`,
  `docs/SPRINT-18-steals.md`. Accept: each promoted idea is one board row with both
  `F2P:` and `P2P:`; no duplicate of an existing row; `node tools/workspace-check.mjs`
  PASS; `node --test tools/plan-index.test.mjs` green. F2P: `landscape-ideas.md` holds
  every idea and the shard ends at GS33. P2P: `node tools/workspace-check.mjs` PASS.
  needs: none.

**Promoted from the landscape backlog (PLN30, cluster E ship and input)**

- [ ] GS39 game: borderless fullscreen toggle on F11 — the arena window opens windowed
  and has no way to fill the screen. Add `WindowMode { Windowed, Borderless }` with
  `toggled()` and `is_borderless()` to `crates/game/src/bin/arena_frame/mod.rs`; F11
  (pressed, no repeat) flips it and calls `window.set_fullscreen(...)` with `None` or
  `Some(Fullscreen::Borderless(None))`. Donor: winit `winit/examples/application.rs`
  `toggle_fullscreen`. Presentation only: the tape and the hash never see the mode.
  Diff: regular. Tier A. Size S. Owns the new type in `arena_frame/mod.rs` and the F11
  branch in `crates/game/src/bin/arena.rs`. Accept: `cargo test -p engine-game
  --features app --bin arena window_mode` x2: two toggles return to `Windowed`,
  `Borderless.is_borderless()` is true; golden `d57b5c8ae3096614` unchanged. F2P: no
  `WindowMode`. P2P: `cargo test -p engine-game --features app --bin arena` green.
  needs: none.
- [ ] GS40 render: battery cap on the quality tier — a laptop on battery draws full
  frames and the fan and heat show it. Add `crates/render/src/power.rs`: a Windows leaf
  `read_power()` beside `backend::read_vram` (`GetSystemPowerStatus`: `ACLineStatus`,
  `BatteryLifePercent`, `SystemStatusFlag` battery saver) returning `PowerReading::{Known
  { ac_online, percent, saver }, Unknown(reason)}`, and a pure `power_cap(&PowerReading,
  Quality) -> Quality` that steps one tier down (High to Comfort, Comfort to Potato,
  never `Web`) when `!ac_online && (percent <= 30 || saver)`. Donor: Microsoft
  `SYSTEM_POWER_STATUS` (`winbase.h`). Cold path, off the tick, no hash input. Diff:
  regular. Tier A. Size S. Owns `crates/render/src/power.rs` and the
  `Win32_System_Power` feature on the `windows` line of `crates/render/Cargo.toml`.
  Accept: `cargo test -p engine-render --lib power` x2: AC on at 100 percent keeps
  `High`; battery 25 percent gives `Comfort`; saver on at 80 percent gives `Comfort`;
  `Potato` stays `Potato`; `Unknown` keeps the tier; golden unchanged. F2P: no `power`
  module. P2P: `cargo test -p engine-render --lib quality` green. needs: none.
- [x] GS41 game: raw mouse turns the free camera - the arena window reads keys and Landed 51f7687ec.
  buttons only, and `fps_view::LookState::apply_mouse` has no mouse source. Add
  `MouseRaw` (new `crates/game/src/mouse_raw.rs`): `push(dx, dy)` sums winit
  `DeviceEvent::MouseMotion` deltas only while the cursor is grabbed and drops
  non-finite values; `take()` returns the frame sum clamped to 2,000 px per axis and
  zeroes it. The arena `device_event` pushes, the free camera (`C` key) grabs the cursor
  (`CursorGrabMode::Locked`, else `Confined`) and each frame feeds `take()` to
  `apply_mouse`. Donor: winit `winit/examples/application.rs` `device_event` and the
  `DeviceEvent::MouseMotion` docs (raw, unaccelerated). Presentation only, never hashed.
  Diff: regular. Tier A. Size S. Owns `crates/game/src/mouse_raw.rs`, its `lib.rs` line
  and the `device_event` hunk of `crates/game/src/bin/arena.rs`. Accept: `cargo test
  -p engine-game --lib mouse_raw` x2: two pushes of (3, -1) take (6, -2) and a second
  take gives (0, 0); a push while not grabbed, NaN or infinity changes nothing; a 9,000
  px spike takes 2,000; golden unchanged. F2P: no `mouse_raw` module. P2P: `cargo test
  -p engine-game --lib fps_view` green. needs: none.
  Progress 2026-10-23: `mouse_raw` 5 passed x2 (runs 39954193832, 39954853032); `fps_view` 16 (39954885446); clippy all-targets PASS (39954206438); golden PASS (39954842478). Landed 51f7687ec; the free-camera math moved to `crates/game/src/bin/cursor/mod.rs` so the pinned `arena.rs` held at 5,101 lines, and `f32_from_f64` owns the winit f64 boundary.
- [x] GS42 game: announcer lines as data with a last-played throttle - the crowd has
  per-reaction cooldowns (`audio/src/crowd.rs`) but the bout has no voice. Add
  `crates/game/src/announcer.rs`: a rule table `(FightCue, line name, min_gap_ticks)`
  in list order; `Announcer::on_cue(tick, cue) -> Option<&'static str>` returns the
  first rule that matches and whose gap since its own last play has passed, else
  `None`, at most one line per call. Donor: `abadugu13/football_llm_commentary`
  `commentary_pipeline.py` (`commentary_pipeline`: ordered rules, per-rule `last_step`
  cooldown, first match wins). Names only; the clips come from a content row, a missing
  clip falls back to the cue bank's synth take. `audio_mix.rs` queues the returned name
  (one call). Diff: regular. Tier B. Size S. Owns `crates/game/src/announcer.rs`, its
  `lib.rs` line and the one call in `crates/game/src/audio_mix.rs`. Accept: `cargo test
  -p engine-game --lib announcer` x2: the same cue twice inside its gap gives one line
  and then `None`; after the gap it speaks again; two matching rules give the first
  listed; the same cue script gives the same lines; golden unchanged. F2P: no
  `announcer` module. P2P: `cargo test -p engine-game --lib audio_mix` green. needs: none.
  Landed b66e8c878. `crates/game/src/announcer.rs` (415 lines: fixed rule table in
  list order, per-rule `min_gap_ticks`, first match wins, at most one line per call,
  distinct voice keys) plus the one queue call in `audio_mix.rs` and the `lib.rs`
  line. `cargo test -p engine-game --lib announcer` PASS: 10 passed, 0 failed
  (8 announcer + 2 audio_mix `announcer_line_*`), 586 filtered, run 39940798965;
  covers hold-inside-gap then None, speak-again-after-gap, per-rule gaps,
  first-listed-wins, same script same lines, rewind never refires early,
  default table answers every cue kind. Presentation only (names, never the sim);
  golden `d57b5c8ae3096614` re-proven at d10b1c007 (run 39939140557).
