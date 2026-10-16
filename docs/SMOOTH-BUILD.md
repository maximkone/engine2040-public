# SMOOTH-BUILD — build the game without friction; let problems fix themselves

Collapse 2026-10-16: removed evidence-only duplicates (kept one latest table per repeated family: OPT8 re-measure 11 speed-20261014-1843, speed-measure cold triage speed-20261014-2117, OPT9 re-measure 4 speed-20261014-2136); narrative plus unique families kept; full history in git log -p -- docs/SMOOTH-BUILD.md.

Practical features only, each with a board task in `SPRINT.md`. The test for
every item: does it save a person or an agent time on a normal day of
building the arena game? These are planned workflows, not shipped features.

Review 2026-09-23: the doctor and `/fix` files exist, but their failure-path
drills are unverified. **Use the doctor in report-only mode. Do not run
`doctor --fix` or the current workspace-wide `/fix` sequence until DOC1/FX3
hardening passes.** The current doctor only cleans cache on one tree
`C:\engine2040` on master; claims live in `target/queue/claims.txt`;
the current fix command can rewrite unrelated dirty crates. See `BLOCKERS.md`.

Close 2026-10-05: DOC1 passed — `tools/doctor.mjs` is consent-scoped and
`node --test tools/doctor.test.mjs` is 5/5 green (re-run this day), with a
healthy live report. `doctor --fix` without `--consent` still changes nothing
by design, so unattended repair stays report-only. FX3's standalone `/fix`
command was retired the same day it hardened (commit `c71841f`, one-command
loop); no workspace-wide fix sequence exists to run. Its scoping rules
(report first, failing package only, dirty-set check, one retry) live on in
`.opencode/commands/sprint.md` sections 2 and 4.

## 1. See every change instantly

- **Hot reload everything that is data (HR1–HR3):** scenes, move tables, AI
  curves, materials, shaders, textures, sounds. Save a file and the running
   game shows it within about a second on the declared small fixture (a target
   to measure, not a universal promise). Presentation data swaps at a frame
   boundary; authoritative rules swap only in practice mode at a recorded tick.
   Competitive matches pin their rules/content versions. A broken file keeps
   the last good version and shows a diagnostic instead of silently accepting it.
- **Live tuning while playing (TU1):** change damage, stamina, speeds and AI
   aggression in a practice fight; every change previews through the command
   service. Saving makes a new rules version; existing tapes retain the old
   version. Undo does not rewrite the past of an active match.
- **Instant play, restart and replay (PL1):** play from the editor camera;
  restart the round from its snapshot in under a second; replay the last 30
   seconds within the recorded fixture/platform scope once the arena snapshot/
   replay path exists. A legacy proof-CLI replay is not this editor workflow.
- **Start from templates (TPL1):** new arena, prop, weapon or fighter from a
   working template with sensible defaults and a test, instead of from zero.
   Include an import preset, locomotion bindings and a runnable check; refuse
   to overwrite an existing project. Templates use the same public APIs as users.

## 2. The engine fixes content for you

- **Scene fixers (FX1):** findings offer a typed repair proposal or a refusal
   with a reason, affected IDs and a next step. A floating prop may propose a
   ground snap; overlapping props and blocked doors require an intent-preserving
   choice. Moving cover or generating collision is not intrinsically safe.
   "Fix all" previews one batch, checks the resulting scene, then applies only
   with approval at that revision. Saving may run diagnostics, never silently
   move objects. Impossible fixes leave the scene unchanged with a conflict set.
- **Import fixers (FX2):** diagnose units, missing tangents, color-space slots,
   LOD/lightmap needs and texture budgets. Units require metadata or an explicit
   user setting, not a guessed bounding-box scale. Validated tangent generation
   and per-slot color handling are deterministic cook operations. Topology,
   LOD, UV and downscale changes need settings and preview. Source bytes remain
   untouched; a report records settings/digests and a new cooked version. Reject
   bad geometry instead of hiding it with a substitute and calling import passed.
- **Performance doctor (PD1):** when a frame is too slow, it names the
  top costs (too many shadow casters, a huge texture, too many particles) with
   a measured, reversible suggestion for each. The presentation-only autopilot
   (PA1) attempts the frame budget within declared quality floors; it cannot
   guarantee 60 fps. Baseline captures disable adaptation, and adaptive captures
   record every quality change so a lower-quality frame cannot fake improvement.
- **Migration:** supported old versions migrate into a new file/generation;
   preserve the original. Unknown/future versions refuse rather than guessing.
   The format museum (LT1) tests the released versions, not every possible file.

## 3. The engine fixes code for you

- **`/fix` (FX3, closed 2026-10-05 — command retired, rules kept):** diagnose first, then propose scoped repairs
   for an exclusively owned failing crate. Use `cargo fmt -p <crate>` and
   package-scoped compiler suggestions; no workspace-wide mutation on one tree `C:\engine2040` on master,
   no blanket `--allow-dirty`, and no changes to unfamiliar user work.
   Review suggestions and re-run the same failing command plus the full gate.
   One diagnosed retry; the same failure twice gets a blocker with exact output.
   Unsupported compiler errors remain red, not "fixed" by disabling checks.
   The standalone command file is gone (retired in `c71841f`); the loop
   applies these rules through `.opencode/commands/sprint.md`.
- Sprint startup checks the environment read-only. Installing tools, deleting
   caches or repairing locks are separate explicit operations. A green gate
   never authorizes committing somebody else's dirty files.

## 4. The project fixes itself

- **Doctor (DOC1, closed 2026-10-05):** `node tools/doctor.mjs` checks the pinned
  Rust toolchain and components, Node, cargo-deny, a stale sprint lock, the
  lockfile, the workspace and board audit, disk space and git state. Bare
  `--fix` only reports what it would do; live leases are never released;
  stale release needs `--consent=lock` and names the holder; pool eviction
  needs `--consent=cache` plus low disk on one tree `C:\engine2040` on master; claims
  in `target/queue/claims.txt` are never touched. Proof: `node --test tools/doctor.test.mjs`
  5/5 green (foreign-lock, stale-lock, low-disk with and without cache,
  bad scope), re-run 2026-10-05, live report healthy.
  Report unknown probes as unknown (not healthy), and distinguish workstation
  readiness from the build gate and game health.

## 5. Bugs arrive ready to fix

- **Bug bundles (BUG1):** a crash or a failed check saves the input tape,
   build/rules/content versions and bounded redacted events. Attach a screenshot
   when capture is available; headless or device-loss reports say why it is
   absent. Reproduction requires the matching recorded inputs/artifacts and a
   supported determinism tier, not "any machine". OB2 regenerates covered sim
   events, not missing GPU, OS or provider logs.
- **Rewind (TT1):** scrub any playtest by tick and resume from any moment.
- **Changes in plain words (SD1):** every edit, human or AI, is described as
  "brazier moved from the north wall to the gate", so reviews take seconds.
- **Undo just the AI (SU1):** revert what one AI session did and keep your own
  edits.

## 6. Playtests run themselves

- **Background playtests (PT1):** debounce relevant changes; run a bounded
   queue of cancellable headless forks of an immutable revision. Coalesce stale
   jobs, never starve the editor/tick, and label old results as stale. Return
   invariant failures promptly; a win-rate claim needs a declared seed suite,
   sample count and uncertainty, not one fight.
- **Balance assistant (BAL1):** after playtests it points at outliers ("the
   sword wins 70% of this seed suite") and offers a tweak checked on separate
   validation seeds. Show uncertainty and the diff; apply only after approval
   as one undoable batch. Never optimize away the rules the verifier checks.
- **Exploit finder (RV1):** a playtest agent that looks for ways to break
  fights (endless stamina, wall clipping); each exploit becomes a regression
  test.

## Rules for anything that fixes itself

1. Only enumerated presentation/service retries may be automatic. Semantic
   scene/rules edits preview and require scoped approval.
2. Authoring fixes are undoable transactions. Recovery actions have durable
   receipts and preserve originals; not every external action can be undone.
3. Fix to a validated known-good state, never by guessing. A recorded invalid
   simulation correction is still invalid; fail loudly when recovery is unsafe.
4. Never weaken a test, check or budget to make something pass.
5. After the retry budget, stop and report with a reproduction.

## First useful workflows (same project, not separate demos)

| Workflow | Consumer and tasks | Evidence required |
|---|---|---|
| Drop in a prop, diagnose its materials/units, place/save/reopen/play | Import report + editor; A1/A2, E1–E3, MV1 | Stable asset IDs, real rendered fixture, bad import leaves last-good intact |
| Ask any supported model to repair a misplaced prop | Task panel + shared commands; EA1, PV2, CAP1, SP1, FX1 | Same manual/local/remote task, preview, cancellation, stale refusal, undo and play receipt |
| Make a fighter move well | Locomotion inspector/course; AN1, LOC1/LOC2, PH2, A3, R3, GM1 | Start/stop/turn/strafe/air clips, collision/root reconciliation, tape checkpoints and visual review |
| Change a material or practice move without restarting | Reload subscribers + tuning; HR1–HR4, HR5, TU1 | Measured end-to-end latency, bad reload rejected, old tape still replays |
| Diagnose a crash and retry safely | Play controller + bug bundle; E3, BUG1, OB2 | Recover document, bounded redacted artifacts, named first divergent tick or explicit unsupported reproduction |

No new shell/MCP super-tool is needed. Tool discovery, jobs, receipts, fixes and
the UI reuse the canonical contracts in `EDITOR_AI_CONTROL.md`. GitHub sources
and their evidence limits are recorded in `PLAN-REVIEW.md` and `EVIDENCE.md`.

## OPT1 build baselines (observed-only, speed102, 2026-09-30)

Observed-only run on one tree `C:\engine2040` on master, not a promotion baseline (promotion is OPT32 on the
quiet box). Shared pool `target/pool-1`, flags
`--locked -j 4`. Command: `cargo build --manifest-path
Cargo.toml --target-dir target/pool-1 -j 4 --locked
--timings` (dev profile). Wall 347 s (5 m 47 s). Timing report
`target/pool-1/cargo-timings/cargo-timing-20260930T005506382Z-1efcc15018e4cd63.html`
copied to `target/evidence/build/` as `dev-timing-observed.html` with
`dev-observed.json`. No crate file was touched (touch rebuilds skipped;
claims in `target/queue/claims.txt` respected; they ride with OPT32). `fast-test` and `release`
clean + touch matrices are follow-up on the quiet box.

## OPT4 re-verify (2026-10-12, KEEP rust-lld, no config change)

`rust-lld.exe` stays the Windows linker; `.cargo/config.toml` untouched.
Quiet-box `cargo test -p engine-game --lib --no-run --locked -j 2` in one
`node tools/quiet.mjs` run (QUIET, 3 of 3 slots, no cargo running): 26.65 s
first compile, 1.26 s warm immediately after. `cargo test -p engine-core
--lib` green (90 passed, 0 failed, 2 ignored). `cargo test -p engine-proof
--lib golden_hash_untouched` green (1 passed). The standing cold bar 121 s
-> 79 s (-35%, clears the 20% keep rule) stands; no fresh link.exe A/B
re-run (OPT4 landed, re-verify only). Pre-existing failure on this tree, not
this packet: `bench::tests::pq1_seeded_fixture_holds_p95` (319 passed, 1
failed, timing flake on a loaded box). Golden `d57b5c8ae3096614` unchanged
(docs-only diff).

| Profile | Wall | Longest unit (critical path) | Notes |
|---|---|---|---|
| dev (observed, warm pool-1, -j4) | 347 s | engine-render 294.0 s (frontend 99.7 s, codegen 194.3 s) | next: engine-assets 168.8 s, engine-nav 166.7 s, engine-proof 157.9 s, engine-audio 152.8 s, bin link 26.1 s |
| fast-test | PENDING | PENDING until quiet-box run | inherits dev, separate profile dir |
| release (fat LTO, 1 codegen unit) | PENDING | PENDING until quiet-box run | promotion is OPT32 x2 within 10% |

Critical path: `engine-render` codegen (194.3 s of its 294.0 s) dominates the
dev build; the bin link (`engine2040` bin unit) is 26.1 s after `engine-nav`
unblocks it at 320.95 s. Per-unit table lives in the timing HTML; the JSON
summary lives in `target/evidence/build/dev-observed.json`. Golden
`d57b5c8ae3096614` untouched (measure + report only, no code change).

## Limits (merged from the gateway-health adapter; pattern steal from forge `docs/ship/FAQ.md` merged from `KNOWN-LIMITS.md`)

What the health view proves today, and what it does not. The adapter is
`tools/gateway-health.mjs` (own code; health patterns from forge
`gateway/GatewayProbeState.cs`, `gateway/LaneStatus.cs`,
`gateway/GatewayDoctor.cs`, `gateway/GatewayTimeoutTiers.cs` and
`docs/g23-heartbeat.md`).

### Proven (offline, from the repo root)

`node tools/gateway-health.mjs --selftest` is green: probe order
(connected wins, timeout is slow, refused is down), merged endpoint states
(`none`, `live`, `batch`, `remote`, `down`) with no probing inside the
merge, token and path redact on every free-text detail, heartbeat age with
a 60 s stale bar where unknown age is never old, and doctor exits 0/1.

### Not yet proven (needs a live endpoint)

Merged view against a running endpoint with real cards and beats: no run
on record. Stale-holder refuse on a dead beat file: no run on record.
Doctor roll-up wired into `node tools/doctor.mjs`: not wired, by choice.

### Rule

No claim in this file goes past its proof line above.
A check with no log stays open, never met.

## OPT8 hakari spike (2026-10-12, builder-build-r3, evidence-only, no config change)

`cargo-hakari` workspace-hack pattern (union of dependency features into one
crate every member depends on) would cut the `target/debug/deps` split seen
today. No manifest, profile or linker change in this packet: `Cargo.toml`,
`.cargo/config.toml` and `rust-toolchain.toml` untouched.

Evidence (same box, quiet, no code change):

- `target/debug/deps` holds 16 builds of `syn`, 12 of `serde_json`, 14 of
  `num_traits` (OPT36 claimed 16/7/7; `syn` matches, the other two are worse),
  9350 files total at the time. `crates/workspace-hack/` and `tools/feature-unify.mjs` now exist (OPT36 landed, Test-Path True 2026-10-16).
- Quiet-box `cargo test -p engine-game --lib --no-run --locked -j 2` x2 in one
  `node tools/quiet.mjs` run (QUIET, 3 of 3 slots, no cargo running): 24.84 s
  first compile, 11.80 s immediately after, both exit 0. Prior standing line
  26.65 -> 1.26 s: cold matches within 7%, warm is slower today (dirty tree,
  24 helpers working; second run still recompiled `engine-combat`,
  `engine-render`, `engine-spatial`, `engine-game` in 11.38 s, not fresh).
- Build green: both `--no-run` legs exit 0 with the same two warnings as HEAD
  (`engine-combat` unused `TABLES_V1/V2` import, `engine-render` dead
  `DynRes::fresh/with_floor`). Golden `d57b5c8ae3096614` untouched (docs-only
  diff; `engine-proof` golden leg re-run before report).
- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check 0.5 s,
  OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows PENDING).

Books: Cargo builds one copy with the union of requested features
[trpl | Cargo Workspaces]; `cargo-book` feature-unification rule (one
`Compiling shared` line serves both users). License: `cargo-hakari` pattern
is MIT OR Apache-2.0; no dependency added here, so no `deny.toml` check was
needed. The full hack (crate plus `tools/feature-unify.mjs`) belongs to
OPT36, which owns `crates/workspace-hack/` and `tools/feature-unify.mjs`
outside this packet's paths: landed since, not built in this packet.

Next: OPT36 full hack or PF rows need a quiesced reference-GPU pilot.

## OPT4/OPT5/OPT7 landed evidence (2026-10-12, builder-build-r6, docs-only, no re-verify)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; this section records the hunks so a
`closes:` packet can tick them. Never re-verified per standing order.

Landed hunks (read-only, same box):

- OPT4 linker: `.cargo/config.toml:38-39` sets
  `[target.x86_64-pc-windows-msvc] linker = "rust-lld.exe"`.
- OPT5 profiles: root `Cargo.toml:275-291` holds `[profile.profiling]`
  (release, thin LTO, line-tables-only), `[profile.perf-test]` (release,
  thin LTO, 16 codegen units) and `[profile.ship]` (fat LTO, one unit);
  `.cargo/config.toml:22-31` holds `[profile.fast-test]` (dev, debug 0,
  opt-level 0, build-override 2). Profiles are per-profile customization
  [trpl | Customizing Builds with Release Profiles].
- OPT7 affected gate: `tools/affected.mjs` maps a diff to changed packages
  plus reverse deps via `cargo metadata`; docs-only diffs skip metadata
  (`needsCrateMetadata` false, metadataMs 0). Workspace members resolve
  under `[workspace]` [trpl | Creating a Workspace].

Evidence (same box, quiet, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  1.1 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior lines: r3 0.5 s, r5 0.6 s; slower today, loaded box with
  20 helpers working.
- `node tools/bins.mjs` prints all four STALE (runner/perf rebuild, not
  this packet): `engine2040` 5 commits, `staged_fight` 33, `perf` 8,
  `arena` 5. Prior r5 line had `engine2040` FRESH; five commits since
  18:49Z flipped it.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Cold `cargo test -p engine-game --lib --no-run --locked -j 2` under
  `node tools/quiet.mjs` is RED on this tree, pre-existing and out of
  paths: `crates/combat/src/lib.rs:12` names `pub mod classes` with no
  `crates/combat/src/classes.rs` on disk (concurrent GS31 claim
  `builder-gameplay-r6-fix-20261012` holds it since 18:15Z). QUIET (3 of 3
  slots held, no cargo running), wall 357.2 s, `error[E0583]` for `classes`.
  Prior r3 line 24.84 s green; r5 line RED with no seconds. Not fixed here;
  another writer's red becomes a bug packet per kernel rule 4. No slowest
  edge cut in this packet: the tree is red and root `Cargo.toml` is held by
  `builder-ss-sim-20261012` since 19:41Z, so a dependency, feature or
  profile cut would be unmeasurable and out of paths.
- Build still green on a clean crate: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` in one hold prints
  `test result: ok. 90 passed; 0 failed; 2 ignored` twice (inner 0.36 s
  then 0.73 s), outer wall total 196.2 s QUIET. Load stated beside numbers;
  no improvement claimed on a red tree. Prior r5 line 306.07 s LOADED then
  5.15 s.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `the keeper names the change in its next continue (the loop also reads
  the file each wave)` with every rule knob `next pass`; no `restart`
  line today. `.opencode/plugin/loop-keeper.js:953-957` (`knobsNews` plus
  `knobValue` read the file live each pass) and `:1236` (`knobsSeen`
  advance) already cover it. Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; no new ready packet.

Next: keeper lands `closes:` for OPT4, OPT5 and OPT7 from these hunks;
OPT36 needs a green tree (GS31 landed) plus a quiet box.

## OPT9 sccache landed evidence (2026-10-12, builder-build-r7, docs-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT6 stays PARKED (`cargo nextest --version` prints `no such command`,
same as the row's 2026-10-05 note); OPT8 was re-measured by r5; OPT4/OPT5/OPT7
landed evidence sits above (r6). This section records the OPT9 hunks so a
`closes:` packet can tick it.

Landed hunks (read-only, same box):

- OPT9 cache: `.cargo/config.toml:54-55` sets `[build]
  rustc-wrapper = ".cargo/sccache-pool.cmd"` (pool-scoped only; plain
  `target/debug` runs rustc directly, incremental intact). Wrapper
  `.cargo/sccache-pool.cmd` exists (pool check, else direct). `sccache
  --version` prints `0.18.0`. Report `target/evidence/opt9/report.md`
  absent after the 2026-10-10 `target/` wipe (same as `target/evidence/build/`).

Evidence (same box, quiet, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.6 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r6 line 1.1 s; r5 0.6 s matches today.
- `node tools/bins.mjs` prints all four STALE (runner/perf rebuild, not
  this packet): `engine2040` 10 commits, `staged_fight` 38, `perf` 13,
  `arena` 10. Prior r6 line 5/33/8/5; commits since flipped counts.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Build green x2 QUIET: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` in one hold prints
  `test result: ok. 90 passed; 0 failed; 2 ignored` twice (inner 0.28 s
  then 0.50 s; compile 1.17 s then 0.62 s), QUIET (3 of 3 slots, no cargo
  running). Includes `golden_d57b5c8ae3096614_unchanged ok` twice.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; golden leg green
  above, no Rust, manifest or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `the keeper names the change in its next continue (the loop also reads
  the file each wave)` with every rule knob `next pass`; no `restart`
  line today. `.opencode/plugin/loop-keeper.js:954-965` (`knobsNews` plus
  `knobValue` read the file live each pass) already covers it. Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`; no
  new ready packet.

Next: keeper lands `closes:` for OPT9 from these hunks; OPT32 needs the
operator quiet-box window; OPT6 stays PARKED until the owner installs nextest.

## OPT32 proposal: warm sccache + one target-dir per slot (2026-10-13, config-only, no build run)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml` untouched (config
carries foreign uncommitted changes, so it was SKIPPED, not edited). No
`cargo` build/check/test was run here. Golden `d57b5c8ae3096614` untouched.

Before (documented, this box): dev observed 347 s wall, warm pool-1, -j4
(engine-render 294.0 s dominates; bin link 26.1 s). sccache OPT9 note: cold
`cargo check -p engine-arena` 20.2 s -> 1.1 s at 100% hits. Linker bar:
121 s link.exe -> 79 s rust-lld (-35%). `sccache --show-stats` today prints
0 requests / 0 hits (cold local cache). Last warm leg r21 QUIET 0.55 s ->
0.49 s (90 passed x2, no improvement claimed).

Change in this packet (config only): `.github/workflows/ci.yml` sets
`CARGO_INCREMENTAL: "0"` (sccache cannot cache incremental artifacts, so
cold-every-run CI maximizes hit rate; local dev keeps incremental on and
lanes pass `CARGO_INCREMENTAL=0` per cold command only, `COLD_HINT`).
Per-slot rule (convention, no code): map quiet slot k / lane pool k to one
`CARGO_TARGET_DIR` each (`target/slot-k` locally, `target/pool-k` for
lanes), so parallel builds never share a build-dir lock. Warm sccache stays
opt-in per cold command, never a global `rustc-wrapper`
(`.cargo/config.test.mjs` guards both).

After: PENDING quiet-box re-measure (expected: cold-pool wall down at warm
sccache hits, lock-queue near zero with one dir per slot; no number claimed
until `node tools/quiet.mjs --times 2 --` green legs land it).

## OPT6 spike check (2026-10-14, speed-20261014-0236, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed today). OPT8 skipped (claim
`speed-20261014-0041` at 00:41Z inside the last 2 h). OPT9 skipped (claim
`speed-20261014-0121` at 01:21Z inside the last 2 h). OPT9 hunks on disk
(`.cargo/config.toml` linker, `.cargo/sccache-pool.cmd` exists, `sccache
0.18.0`); the foreign `.cargo/config.toml` fast-test hunk is preserved,
not touched here. The full hakari hack stays OPT36 (owns
`crates/workspace-hack/` plus `tools/feature-unify.mjs` outside this
packet's paths): not built here.

Evidence (same box, quiet, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.1 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior line r0121 0.6 s; fluctuates with load, no improvement
  claimed.
- `node tools/bins.mjs` prints 2 FRESH / 2 STALE (runner/perf rebuild, not
  this packet): `engine2040` FRESH built 2026-10-14T02:29:58.189Z, `arena`
  FRESH built 2026-10-14T02:37:09.299Z, `staged_fight` STALE 84 commits,
  `perf` STALE 4 commits. Prior r0121 line all 4 STALE; runner rebuilt two
  since.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Build green x2 QUIET in one hold: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` prints `QUIET (3 of 3
  cargo slots held, no cargo or rustc running)`, `Finished test profile
  in 0.34 s` then `in 0.27 s`, `test result: ok. 107 passed; 0 failed; 2
  ignored` twice (inner 0.14 s then 0.14 s). Before 0.34 s -> after 0.27 s
  in the same quiet hold on the same box; warm tree, no improvement
  claimed. Golden pins (`v1_golden_unchanged`, `digest_golden`,
  `golden_d57b5c8ae3096614_unchanged`) green inside the 107. Prior green
  r0121 LOADED 53.05 s -> 93 s (107 passed x2). Three pre-existing
  `checked.rs` `unused_must_use` warnings on the dirty tree.
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  held for the core green above; tree carries 251 dirty files plus the
  foreign `.cargo/config.toml` hunk). Last lines: r3 green 24.84 s then
  11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`. That break is gone
  today (`crates/combat/src/lib.rs` names no `classes` mod). No slowest
  edge cut in this packet: root `Cargo.toml` carries one uncommitted
  unfamiliar line (`overflow-checks = true` under `[profile.release]`,
  preserved, not touched here), so a dependency, feature or profile cut
  would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched; core green x2 above).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `the keeper names the change in its next continue (the loop also reads
  the file each wave)` with every rule knob `next pass`; no `restart`
  line today. `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`
  `C:/engine2040/.opencode/knobs.json`) plus `.opencode/knobs.json:3`
  (`loop reads this file at Start and every wave`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) already cover it; the old
  `.opencode/plugin/loop-keeper.js` path does not exist. Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, done file present (no new ready packet).

Books: incremental compilation cleanup plus release rebuild
[handsrust | Distributing Your Game]; Cargo invokes rustc plus lockfile
shape [progrust | rustup and Cargo]; one workspace lock plus union features
means one copy [trpl | Depending on an External Package]; profiles are
per-profile customization [trpl | Customizing Builds with Release Profiles].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before optimizing
holds, so no edge cut on a dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT11 fan-out measure (2026-10-14, speed-20261014-1332, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5, OPT7 and OPT9 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed today). OPT8 skipped (claim
`speed-20261014-1301` at 13:01Z inside the last 2 h window 11:32-13:32Z).
OPT12 is an unknown row (`board-slice: unknown row`). OPT34 is
render-owned (`crates/render/src/cache.rs`, not this packet's paths).
OPT11 is the fallback fan-out row (reverse-dep closure times; cut the
slowest crate edge): measured here, no edge cut for the reasons below.
The foreign `.cargo/config.toml` fast-test hunk (1 insertion, 2 deletions)
and the foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks are
preserved, not touched here. The full hakari hack stays OPT36 (owns
`crates/workspace-hack/` plus `tools/feature-unify.mjs` outside this
packet's paths): not built here.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1301 line 0.2 s, r0512 line 0.1 s; fluctuates with load,
  no improvement claimed.
- `node tools/bins.mjs` prints 4 STALE plus 1 MISSING (runner/perf rebuild,
  not this packet): `engine2040` STALE 1 commit, `staged_fight` STALE 4,
  `perf` STALE 4, `arena` STALE 4, `e2040-editor` MISSING (dirty
  `tools/bins.mjs` adds the 5th binary, preserved here). Prior r1301 line
  4 STALE plus 1 MISSING 3/3/3/3; one commit since flipped `engine2040`
  from 3 to 1.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Cargo slots would not free for a quiet green leg in this run:
  `node tools/quiet.mjs --times 2 -- cargo test -p engine-core --lib
  --locked -j 2` timed out at 120 s with no output (30 helpers running per
  `target/loop-keeper.json`, 3 cargo slots held by `quiet-21992`
  `perf-quiet`, file-lock wait). One file-lock failure ends the attempt per
  `cargo-slot-wait` and kernel rule 4; the same cargo line is not retried
  a second time here; no improvement claimed on a loaded box. Last green
  stays r0512 (QUIET 0.69 s -> 0.69 s, 108 passed twice, inner 0.24 s then
  0.21 s, golden pins green inside the 108).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  busy, same wait as above; tree carries 318 dirty files plus the foreign
  `.cargo/config.toml` and `tools/bins.mjs` hunks). Last lines: r3 green
  24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`. That
  break is gone today (`crates/combat/src/lib.rs` names no `classes` mod).
  No slowest edge cut in this packet: the foreign hunks are preserved, not
  touched here, plus 318 dirty files, so a dependency, feature or profile
  cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched; golden leg last green r0512 x2).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 28m ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue, `knobsSeen` records the version),
  `:94` (`knobs: C:/engine2040/.opencode/knobs.json`), and `:953-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist
  (284840 bytes, 2026-10-13T21:39Z); plus
  `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`),
  `.opencode/knobs.json:3` (`loop reads this file at Start and every wave`),
  and `.opencode/seat-guard.mjs:35-36` (`readKnobs`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles]; one workspace lock plus
union features means one copy [trpl | Depending on an External Package].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

SPIKE | P1 | fan-out edge cut needs a green quiet tree | SMOOTH-BUILD r1301 plus this run | cold game --no-run last green 24.84 s then 11.80 s, no fresh before -> after on a loaded dirty tree (318 files, slots held) | REJECTED because no measurable edge cut without a quiet green; keep OPT11 open for a quiesced run.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT6 re-check (2026-10-14, speed-20261014-1351, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed today). OPT8 skipped (claim
`speed-20261014-1301` at 13:01Z inside the last 2 h window 11:51-13:51Z).
OPT11 skipped (claim `speed-20261014-1332` at 13:32Z inside the same
window). OPT12 is an unknown row (`board-slice: unknown row`). OPT34 is
render-owned (`crates/render/src/cache.rs`, not this packet's paths).
The foreign `.cargo/config.toml` fast-test hunk (1 insertion, 2 deletions)
and the foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks are
preserved, not touched here. The full hakari hack stays OPT36 (owns
`crates/workspace-hack/` plus `tools/feature-unify.mjs` outside this
packet's paths): not built here.

Evidence (same box, one quiet hold, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1332 line 0.2 s; fluctuates with load, no improvement
  claimed.
- `node tools/bins.mjs` prints 2 FRESH / 2 STALE plus 1 MISSING (runner/perf
  rebuild, not this packet): `engine2040` FRESH built 2026-10-14T13:51:04Z,
  `perf` FRESH built 2026-10-14T13:46:16Z, `staged_fight` STALE 6 commits,
  `arena` STALE 6 commits, `e2040-editor` MISSING (dirty `tools/bins.mjs`
  adds the 5th binary, preserved here). Prior r1332 line 4 STALE plus
  1 MISSING 1/4/4/4; runner rebuilt two since.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Build green x2 LOADED in one hold: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` prints `LOADED 0 of 3
  slots busy, 31 cargo/rustc running`, `Finished test profile in 2m 42s`
  then `in 52.75s`, `test result: ok. 109 passed; 0 failed; 2 ignored`
  twice (inner 0.18 s then 0.25 s). Before 162 s -> after 52.75 s in the
  same quiet hold on the same box; warm tree, no improvement claimed.
  Golden pins (`v1_golden_unchanged`, `digest_golden`,
  `golden_d57b5c8ae3096614_unchanged`) green inside the 109.
  Prior green r0512 QUIET 0.69 s -> 0.69 s, 108 passed twice.
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  held for the core green above; tree carries 314 dirty files plus the
  foreign `.cargo/config.toml` and `tools/bins.mjs` hunks). Last lines:
  r3 green 24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing
  `classes.rs`. That break is gone today (`crates/combat/src/lib.rs` names
  no `classes` mod). No slowest edge cut in this packet: the foreign hunks
  are preserved, not touched here, plus 314 dirty files, so a dependency,
  feature or profile cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched; core green x2 above).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 46m ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue, `knobsSeen` records the version),
  `:94` (`knobs: C:/engine2040/.opencode/knobs.json`), and `:953-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist;
  plus `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`),
  `.opencode/knobs.json:3` (`loop reads this file at Start and every wave`),
  and `.opencode/seat-guard.mjs:35-36` (`readKnobs`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles]; one workspace lock plus
union features means one copy [trpl | Depending on an External Package].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

SPIKE | P1 | nextest proof legs need the owner install | SMOOTH-BUILD r1351 re-probe | `cargo nextest --version` still `no such command`, same as 2026-10-05 PARKED note | REJECTED because the tool is absent; keep OPT6 PARKED for the owner install.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT34 triage (2026-10-14, speed-20261014-1441, evidence-only, BLOCKED, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed today). OPT8 skipped (claim
`speed-20261014-1301` at 13:01Z inside the last 2 h window 12:41-14:41Z).
OPT11 skipped (claim `speed-20261014-1332` at 13:32Z inside the same
window). OPT6 skipped (claim `speed-20261014-1351` at 13:51Z inside the
same window). OPT9 skipped (claim `speed-20261014-1416` at 14:16Z inside
the same window). OPT12 is an unknown row (`board-slice: unknown row`).
OPT34 is render-owned (`crates/render/src/cache.rs`, not this packet's
paths): no `cache.rs` edit here, BLOCKED with the remainder below.
The foreign `.cargo/config.toml` fast-test hunk and the foreign
`tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved, not
touched here.

Evidence (same box, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1416 line 0.2 s; same today, no improvement claimed.
- `node tools/bins.mjs` prints 2 FRESH / 2 STALE plus 1 MISSING (runner/perf
  rebuild, not this packet): `engine2040` FRESH built 2026-10-14T14:35:13Z,
  `perf` FRESH built 2026-10-14T13:46:16Z, `staged_fight` STALE 6 commits,
  `arena` STALE 6 commits, `e2040-editor` MISSING (dirty `tools/bins.mjs`
  adds the 5th binary, preserved here). Prior r1416 line 2 FRESH / 2 STALE
  plus 1 MISSING 14:02:29Z/13:46:16Z/6/6; runner rebuilt `engine2040` since.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Build green observed QUIET: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` prints `QUIET (3 of 3
  cargo slots held, no cargo or rustc running)`, `Finished test profile
  in 2.03 s`, `test result: ok. 109 passed; 0 failed; 2 ignored` with
  `golden_d57b5c8ae3096614_unchanged ok` (head and tail of two holds on
  the same box). A third summary invocation timed out at 120 s waiting
  for slots; per `cargo-slot-wait` and kernel rule 4 it is not retried a
  second time here. Last paired green stays r1416 (LOADED 6.22 s ->
  0.62 s, 109 passed twice). No improvement claimed.
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  busy, same wait as above; tree carries 316 dirty files plus the foreign
  `.cargo/config.toml` and `tools/bins.mjs` hunks). Last lines: r3 green
  24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`. That
  break is gone today (`crates/combat/src/lib.rs` names no `classes` mod).
  No slowest edge cut in this packet: the foreign hunks are preserved, not
  touched here, plus 316 dirty files, so a dependency, feature or profile
  cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched; `golden_d57b5c8ae3096614_unchanged` ok above).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 1.7h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue, `knobsSeen` records the version),
  `:94` (`knobs: C:/engine2040/.opencode/knobs.json`), and `:953-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist
  (284840 bytes); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`) and `.opencode/seat-guard.mjs:35-36` (`readKnobs`).
  Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; ready file absent (no new
  ready packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles]. Subsystem review: no
loop, timestep, budget, alloc, lifetime, job, determinism or init change
in a docs-only packet; profile before optimizing holds, so no edge cut on
a dirty tree.

SPIKE | P1 | pipeline-creation counters need the render owner | SMOOTH-BUILD r1441 triage | QUIET core green observed (109 passed, golden ok), summary hold timed out at 120 s, no cold game before -> after on a dirty tree (316 files, slots held, foreign hunks preserved) | BLOCKED because `crates/render/src/cache.rs` is outside this packet's paths; keep OPT34 open for the render seat.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT6 re-measure 16 (2026-10-14, speed-20261014-1600, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, re-probed
16:00Z). OPT9 hunks on disk (linker plus wrapper, sccache 0.18.0). The
full hakari hack stays OPT36 (owns `crates/workspace-hack/` plus
`tools/feature-unify.mjs` outside this packet's paths): not built here.
OPT34 (14:41Z) and OPT9 (14:16Z) carry claims inside the last 2 h, so they
are skipped this run; OPT6 (13:51Z) is outside 2 h and is the first open
ops row taken here.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1441 line 0.2 s; same today, no improvement claimed.
- `node tools/bins.mjs` prints 1 FRESH / 3 STALE plus 1 MISSING: `engine2040`
  FRESH built 2026-10-14T15:51:09Z, `staged_fight` STALE 7 commits, `perf`
  STALE 1 commit, `arena` STALE 7 commits, `e2040-editor` MISSING (5th binary
  from the dirty `tools/bins.mjs` hunk, preserved here).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only).
- Build green not re-proven here: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` held 180 s with no output
  and timed out (slots busy, 12+ helpers working). Per `cargo-slot-wait`
  the same cargo line is not retried a second time here; no improvement
  claimed on a loaded box. Last green stays r1441 QUIET (109 passed twice,
  `golden_d57b5c8ae3096614_unchanged` ok).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  held for the core attempt above; tree carries 300+ dirty files plus the
  foreign `.cargo/config.toml` hunk and the `overflow-checks = true` line
  under `[profile.release]`). Last lines: r3 green 24.84 s then 11.80 s;
  r6 RED QUIET 357.2 s on missing `classes.rs`. That break is gone today
  (`crates/combat/src/lib.rs` names no `classes` mod). No slowest edge cut
  in this packet: the foreign hunks are preserved, not touched here, so a
  dependency, feature or profile cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 2.8h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), and `:954-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist
  (284840 bytes); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`) and `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent (no new ready packet; the keeper itself is center code,
  not this repo).

Books: tests run in parallel by default with thread control
[trpl | Running Tests in Parallel or Consecutively]; profiles are
per-profile customization [trpl | Customizing Builds with Release
Profiles]. Subsystem review: no loop, timestep, budget, alloc, lifetime,
job, determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT13 triage 1 (2026-10-14, speed-20261014-1630, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z inside the last
2 h window 14:30-16:30Z). OPT8 skipped (claim `speed-20261014-1605` at
16:05Z inside the same window). OPT9 skipped (claim `speed-20261014-1618`
at 16:18Z inside the same window). OPT34 skipped (claim
`speed-20261014-1441` at 14:41Z inside the same window, plus render-owned
`crates/render/src/cache.rs` outside this packet's paths). OPT12 is an
unknown row (`board-slice: unknown row`). OPT13 is the first free ops row
(hot-patch spike; owns the spike report): triaged here, no hot-patch for
the reasons below. The foreign `.cargo/config.toml` fast-test hunk and the
foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved,
not touched here. Cold `cargo test -p engine-game --lib --no-run` not
re-run here on purpose: r1605 (quiet 360 s timeout, LOADED 36 running,
file-lock wait) and r1618 (quiet 185 s timeout, LOADED, no output) are the
first failures on this loaded tree, so the same quiet cargo line is not
retried a second time here per stop rules; tree carries 201 dirty files
per `git diff --stat` plus the foreign hunks, so a dependency, feature or
profile cut would be unmeasurable and out of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1618 line 0.2 s; same today, no improvement claimed.
- `node tools/bins.mjs` wall 0.43 s (Measure-Command 16:30Z, loaded):
  3 STALE plus 1 FRESH plus 1 MISSING (`engine2040` STALE 2 commits,
  `staged_fight` STALE 12 commits, `perf` FRESH, `arena` STALE 12 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk). After-edit re-run same command on the same box stays in the same
  band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.48 s
  (Measure-Command 16:30Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped 0 ms, unmapped docs only).
  After-edit re-run same command stays 0 ms metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/affected.test.mjs` 19/19 pass, `node --test tools/bins.test.mjs`
  3/3 pass (16:30Z, same box).
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 3.4h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), and `:954-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist;
  plus `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | gameplay hot-patch needs a running window plus a 2 s state-kept edit | SMOOTH-BUILD r1630 triage | no window run here (201 dirty files, loaded box, foreign hunks preserved), no hot-lib-reloader built (licenses unread, outside owned paths) | REJECTED because no measurable subsecond reach without a green window build; keep OPT13 open for a window seat.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT11 triage 2 (2026-10-14, speed-20261014-1637, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z inside the last
2 h window 14:37-16:37Z, plus PARKED: `cargo nextest --version` prints
`no such command`, re-probed 16:37Z, same as the row's 2026-10-05 note).
OPT8 skipped (claim `speed-20261014-1605` at 16:05Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 16:37Z).
OPT9 skipped (claim `speed-20261014-1618` at 16:18Z inside the same window).
OPT13 skipped (claim `speed-20261014-1630` at 16:30Z inside the same window,
just triaged REJECTED above). OPT34 skipped (claims `speed-20261014-1441`
at 14:41Z and `builder-opt34-cache-counters-20261014` at 15:07Z inside the
same window, plus render-owned `crates/render/src/cache.rs` outside this
packet's paths). OPT12 is an unknown row (`board-slice: unknown row`).
OPT32 is HELD for the operator quiet-box window (owns the `target/`
promotion, outside this packet's paths). OPT11 is the first free ops row
(fan-out cut; owns the report plus the named split): triaged here, no split
for the reasons below. The foreign `Cargo.toml:222`
(`overflow-checks = true` under `[profile.release]`), the foreign
`.cargo/config.toml` fast-test hunk and the foreign `tools/bins.mjs`
5th-binary plus MISSING-line hunks are preserved, not touched here. Cold
`cargo test -p engine-game --lib --no-run` not re-run here on purpose:
r1605 (quiet 360 s timeout, LOADED 36 running, file-lock wait) and r1618
(quiet 185 s timeout, LOADED, no output) are the failures on this loaded
tree, so the same quiet cargo line is not retried a second time here per
stop rules; tree carries 201 dirty files per `git diff --name-only` (316
per `git status --porcelain`) plus the foreign hunks, so a dependency,
feature or profile cut would be unmeasurable and out of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.1 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1630 line 0.2 s; fluctuates with load, no improvement
  claimed.
- `node tools/bins.mjs` wall 0.27 s (Measure-Command 16:37Z, loaded):
  3 STALE plus 1 FRESH plus 1 MISSING (`engine2040` STALE 2 commits since
  2026-10-14T16:19:48.603Z, `staged_fight` STALE 12 commits,
  `perf` FRESH built 2026-10-14T16:26:30.515Z, `arena` STALE 12 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk). After-edit re-run same command on the same box stays in the same
  band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.12 s
  (Measure-Command 16:37Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped 0 ms, unmapped docs only).
  After-edit re-run same command stays 0 ms metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/affected.test.mjs` 19 pass 0 fail, `node --test tools/bins.test.mjs`
  3 pass 0 fail (16:37Z, same box).
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 3.5h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), and `:954-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist
  (284840 bytes, 4152 lines); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`) and `.opencode/seat-guard.mjs:35` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | crate fan-out cut needs a quiet touch-rebuild pair plus a named split | SMOOTH-BUILD r1637 triage | no quiet pair here (201 dirty files, 316 status, loaded box, foreign hunks preserved, root Cargo.toml:222 overflow-checks preserved), no dependency, feature or profile cut (unmeasurable and out of paths) | REJECTED because no measurable 30% touch-rebuild drop without a quiet green tree; keep OPT11 open for a quiesced run.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT32 triage 1 (2026-10-14, speed-20261014-1646, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z inside the last
2 h window 14:46-16:46Z, plus PARKED: `cargo nextest --version` prints
`no such command`, re-probed 16:46Z, same as the row's 2026-10-05 note).
OPT8 skipped (claim `speed-20261014-1605` at 16:05Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 16:46Z).
OPT9 skipped (claim `speed-20261014-1618` at 16:18Z inside the same window).
OPT13 skipped (claim `speed-20261014-1630` at 16:30Z inside the same window,
just triaged REJECTED above). OPT11 skipped (claim `speed-20261014-1637`
at 16:37Z inside the same window, just triaged REJECTED above). OPT34
skipped (claim `builder-opt34-cache-counters-20261014` at 15:07Z inside the
same window, plus render-owned `crates/render/src/cache.rs` outside this
packet's paths). OPT12 is an unknown row (`board-slice: unknown row`).
OPT32 is the first free ops row (build-baseline promotion on the quiet box;
owns the `target/` promotion, outside this packet's paths): triaged here, no
promotion for the reasons below. The foreign `.cargo/config.toml` fast-test
hunk and the foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks are
preserved, not touched here. Cold `cargo test -p engine-game --lib --no-run`
not re-run here on purpose: r1605 (quiet 360 s timeout, LOADED 36 running,
file-lock wait) and r1618 (quiet 185 s timeout, LOADED, no output) are the
failures on this loaded tree, so the same quiet cargo line is not retried a
second time here per stop rules; tree carries dirty files plus the foreign
hunks, so a dependency, feature or profile cut would be unmeasurable and out
of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1637 line 0.1 s; fluctuates with load, no improvement
  claimed.
- `node tools/bins.mjs` wall 0.37 s (Measure-Command 16:46Z, loaded):
  1 FRESH plus 3 STALE plus 1 MISSING (`engine2040` FRESH built
  2026-10-14T16:44:47.069Z, `staged_fight` STALE 13 commits,
  `perf` STALE 1 commit, `arena` STALE 13 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk). After-edit re-run same command on the same box stays in the same
  band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.11 s
  (Measure-Command 16:46Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped 0 ms, unmapped docs only).
  After-edit re-run same command stays 0 ms metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/affected.test.mjs` 19 pass 0 fail, `node --test tools/bins.test.mjs`
  3 pass 0 fail (16:46Z, same box).
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 3.7h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), and `:954-965`
  (`knobsNews` plus `knobValue` read the file live each pass) exist
  (284840 bytes, 4152 lines); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`) and `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | build-baseline promotion needs the operator quiet-box window | SMOOTH-BUILD r1646 triage | no quiet pair here (dirty tree, loaded box, foreign hunks preserved), no clean plus 4-crate touches x 3 profiles rerun (single cargo, lanes halted, load under 10 percent, root target) | REJECTED because promotion needs a quiesced operator run; keep OPT32 open for the quiet-box window.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## speed cold-measure (2026-10-14, speed-20261014-1654, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z inside the last
2 h window 14:54-16:54Z, plus PARKED: `cargo nextest --version` prints
`no such command`). OPT8 skipped (claim `speed-20261014-1605` at 16:05Z
inside the same window; `cargo hakari --version` prints `no such command`).
OPT9 skipped (claim `speed-20261014-1618` at 16:18Z inside the same window).
OPT13 skipped (claim `speed-20261014-1630` at 16:30Z inside the same window).
OPT11 skipped (claim `speed-20261014-1637` at 16:37Z inside the same window).
OPT32 skipped (claim `speed-20261014-1646` at 16:46Z inside the same window,
HELD for the operator quiet-box window). OPT34 skipped (claim
`builder-opt34-cache-counters-20261014` at 15:07Z inside the same window,
plus render-owned `crates/render/src/cache.rs` outside this packet's paths).
OPT12 is an unknown row (`board-slice: unknown row`). No free ops row owns
this packet's paths, so the Else cold-measure runs here with no edge cut
for the reasons below. The foreign `Cargo.toml:222`
(`overflow-checks = true` under `[profile.release]`), the foreign
`.cargo/config.toml` fast-test hunk and the foreign `tools/bins.mjs`
5th-binary plus MISSING-line hunks are preserved, not touched here. Cold
`cargo test -p engine-game --lib --no-run` not re-run here on purpose:
r1605 (quiet 360 s timeout, LOADED 36 running, file-lock wait) and r1618
(quiet 185 s timeout, LOADED, no output) are the two failures on this loaded
tree, so the same quiet cargo line is not retried per stop rules; one quiet
core attempt today timed out at 120 s (21 helpers working, slots held), not
retried per `cargo-slot-wait`, so a dependency, feature or profile cut would
be unmeasurable and out of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1646 line 0.2 s; same today, no improvement claimed.
- `node tools/bins.mjs` wall 0.25 s (Measure-Command 16:54Z, loaded):
  1 FRESH plus 2 STALE plus 1 MISSING plus 1 FRESH (`engine2040` FRESH built
  2026-10-14T16:44:47.069Z, `staged_fight` STALE 13 commits,
  `perf` FRESH built 2026-10-14T16:48:24.123Z, `arena` STALE 13 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk). After-edit re-run same command on the same box stays in the same
  band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.14 s
  (Measure-Command 16:54Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped, unmapped docs only).
  After-edit re-run same command stays metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/affected.test.mjs` 19 pass 0 fail, `node --test tools/bins.test.mjs`
  3 pass 0 fail (16:54Z, same box, duration 321 ms and 326 ms).
- Quiet core not green here: `node tools/quiet.mjs --times 2 --
  cargo test -p engine-core --lib --locked -j 2` timed out at 120 s waiting
  for slots (21 helpers working); not retried per `cargo-slot-wait`. Last
  green stays r0502 QUIET 0.71 s -> 3.56 s (108 passed x2, golden pins green).
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 3.8h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), and `:954-964`
  (`knobsNews` reads the file live each pass) exist (284840 bytes);
  plus `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT36 workspace-hack triage (2026-10-14, speed-20261014-1708, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z plus PARKED:
`cargo nextest --version` prints `no such command`). OPT8 skipped (claim
`speed-20261014-1605` at 16:05Z; `cargo hakari --version` prints `no such
command`). OPT9 skipped (claim `speed-20261014-1618` at 16:18Z). OPT13
skipped (claim `speed-20261014-1630` at 16:30Z). OPT11 skipped (claim
`speed-20261014-1637` at 16:37Z, needs OPT7). OPT32 skipped (claim
`speed-20261014-1646` at 16:46Z, HELD for the operator quiet-box window).
`speed-cold-measure` skipped (claim `speed-20261014-1654` at 16:54Z in the
2 h window 15:08-17:08Z). OPT34 skipped (claim
`builder-opt34-cache-counters-20261014` at 15:07Z outside the 2 h window
but render-owned `crates/render/src/cache.rs` outside this packet's paths).
OPT12 is an unknown row (`board-slice: unknown row`). OPT36 is the first
free speed row (one dependency feature set for every `-p` test; owns root
`Cargo.toml`, `crates/workspace-hack/` new, `tools/feature-unify.mjs`
new): triaged here, no hack for the reasons below. The foreign
`.cargo/config.toml` fast-test hunk and the foreign `tools/bins.mjs`
5th-binary plus MISSING-line hunks are preserved, not touched here. Cold
`cargo test -p engine-game --lib --no-run` not re-run here on purpose:
r1605 (quiet 360 s timeout) and r1618 (quiet 185 s timeout) are the two
failures on this loaded tree, so the same quiet cargo line is not retried
per stop rules; tree carries 200+ dirty files plus foreign hunks, so a
dependency, feature or profile cut would be unmeasurable and out of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same as r1654, no improvement claimed.
- `node tools/bins.mjs` wall 0.23 s (Measure-Command 17:06Z, loaded):
  2 FRESH plus 2 STALE plus 1 MISSING (`engine2040` FRESH built
  2026-10-14T16:57:49.420Z, `perf` FRESH built 2026-10-14T16:48:24.123Z,
  `staged_fight` STALE 13 commits, `arena` STALE 13 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk). After-edit re-run same command on the same box stays in the same
  band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.14 s
  (Measure-Command 17:06Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped 0 ms, unmapped docs only).
  After-edit re-run same command stays metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs` 3 pass 0 fail, `node --test tools/affected.test.mjs`
  19 pass 0 fail (17:06Z, same box).
- Quiet core not re-run here: slots held by helpers; per `cargo-slot-wait`
  not retried. Last green stays r0502 QUIET 0.71 s -> 3.56 s.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 4.0h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:68` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), `:954` (`knobsNews`),
  `:965` (`knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (284840 bytes, 4152 lines); plus
  `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: workspaces group members [trpl | Creating a Workspace]
(`node tools/books.mjs search "cargo workspace feature unification"`);
profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box.

## OPT34 pipeline-counter triage (2026-10-14, speed-20261014-1715, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-1600` at 16:00Z inside the 2 h
window 15:15-17:15Z, plus PARKED: `cargo nextest --version` prints
`no such command`, re-probed 17:15Z, same as the row's 2026-10-05 note).
OPT8 skipped (claim `speed-20261014-1605` at 16:05Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 17:15Z).
OPT9 skipped (claim `speed-20261014-1618` at 16:18Z inside the same window).
OPT13 skipped (claim `speed-20261014-1630` at 16:30Z inside the same window).
OPT11 skipped (claim `speed-20261014-1637` at 16:37Z inside the same window,
needs OPT7). OPT32 skipped (claim `speed-20261014-1646` at 16:46Z inside the
same window, HELD for the operator quiet-box window). `speed-cold-measure`
skipped (claim `speed-20261014-1654` at 16:54Z inside the same window).
OPT36 skipped (claim `speed-20261014-1708` at 17:08Z inside the same window,
HELD for a green tree plus a quiet box). OPT34 is the first free row in the
packet list (claims `speed-20261014-1441` at 14:41Z and
`builder-opt34-cache-counters-20261014` at 15:07Z both outside the 2 h
window 15:15-17:15Z): triaged here, no counters, row stays open for the
render seat because `crates/render/src/cache.rs` is outside this packet's
paths. OPT12 is an unknown row (`board-slice: unknown row`). The foreign
`.cargo/config.toml` fast-test hunk and the foreign `tools/bins.mjs`
5th-binary plus MISSING-line hunks are preserved, not touched here. Cold
`cargo test -p engine-game --lib --no-run` not re-run here on purpose:
r1605 (quiet 360 s timeout) and r1618 (quiet 185 s timeout) are the two
failures on this loaded tree, so the same quiet cargo line is not retried
per stop rules; tree carries 200+ dirty files plus foreign hunks, so a
dependency, feature or profile cut would be unmeasurable and out of paths.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same as r1708, no improvement claimed.
- `node tools/bins.mjs` wall 0.22 s (Measure-Command 17:15Z, loaded):
  4 STALE plus 1 MISSING (`engine2040` STALE 4 commits since
  2026-10-14T17:07:34.648Z, `staged_fight` STALE 17 commits,
  `perf` STALE 4 commits since 2026-10-14T16:48:24.123Z, `arena` STALE 17
  commits, `e2040-editor` MISSING with single-ENV rebuild line from the
  preserved hunk). After-edit re-run same command on the same box stays in
  the same band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.13 s
  (Measure-Command 17:15Z, loaded): no crate closure (docs-only, lane gate
  skips crate tests, 0 of 35, metadata skipped 0 ms, unmapped docs only).
  After-edit re-run same command stays metadata-skipped.
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs` 3 pass 0 fail, `node --test tools/affected.test.mjs`
  19 pass 0 fail (17:15Z, same box).
- Quiet core not re-run here: slots held by helpers; per `cargo-slot-wait`
  not retried. Last green stays r0502 QUIET 0.71 s -> 3.56 s.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 4.1h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:68` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), `:954` (`knobsNews`),
  `:965` (`knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (284840 bytes, 4152 lines); plus
  `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: workspaces group members [trpl | Creating a Workspace]
(`node tools/books.mjs search "cargo workspace feature unification"`);
profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## OPT8 re-measure 11 (2026-10-14, speed-20261014-1843, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed 18:47Z; `sccache 0.18.0`). OPT8 is free
(last claim `speed-20261014-1605` at 16:05Z, outside the 2 h window
16:43-18:43Z) and first after the landed plus PARKED skips. OPT9 is free
(last claim `speed-20261014-1618` at 16:18Z, outside the same window) but
later than OPT8, so not taken. OPT12 is an unknown row
(`board-slice: unknown row`). OPT34 skipped (claim `speed-20261014-1715`
at 17:15Z inside the same window, plus render-owned
`crates/render/src/cache.rs` outside this packet's paths). OPT36, OPT11,
OPT13, OPT32 and `speed-measure` claims at 16:46Z-18:01Z sit inside the
same window and are skipped. The full hakari hack stays OPT36 (owns
`crates/workspace-hack/` plus `tools/feature-unify.mjs` outside this
packet's paths): not built here. The foreign `.cargo/config.toml`
fast-test hunk (1 insertion, 2 deletions) and the foreign
`tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved, not
touched here.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Prior r1800 line 0.1 s; fluctuates with load, no improvement
  claimed.
- `node tools/bins.mjs` wall 0.2559 s before -> 0.2020 s after
  (Measure-Command 18:47Z, same command, same box, loaded): 2 FRESH / 2
  STALE plus 1 MISSING: `engine2040` FRESH built 2026-10-14T18:20:57.163Z,
  `perf` FRESH built 2026-10-14T18:45:04.217Z (runner rebuild since r1800),
  `staged_fight` STALE 20 commits, `arena` STALE 20 commits,
  `e2040-editor` MISSING with single-ENV rebuild line from the preserved
  hunk. Prior r1800 line 4 STALE plus 1 MISSING then 1 FRESH; same band,
  no improvement claimed.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.1213 s
  before -> 0.1032 s after (same command, same box): no crate closure
  (docs-only, lane gate skips crate tests, 0 of 35, metadata skipped 0 ms,
  unmapped docs only). No improvement claimed.
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs` 3 pass 0 fail, `node --test tools/affected.test.mjs`
  19 pass 0 fail (18:47Z, same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (11
  helpers running, 9 waiting per loop-keeper queue; tree carries 300+
  dirty files plus the foreign `.cargo/config.toml` and `tools/bins.mjs`
  hunks). Last lines: r3 green 24.84 s then 11.80 s; r6 RED QUIET 357.2 s
  on missing `classes.rs`. That break is gone today
  (`crates/combat/src/lib.rs` names no `classes` mod). No slowest edge cut
  in this packet: the foreign hunks are preserved, not touched here, so a
  dependency, feature or profile cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 5.7h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js` exists (True)
  plus `.opencode/plugins/loop-keeper/index.mjs:17` (`knobsFile`) and
  `.opencode/seat-guard.mjs:35-36` (`readKnobs`) plus
  `.opencode/knobs.json:3` (`reads at Start and every wave`). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`); one workspace lock plus union
features means one copy [trpl | Depending on an External Package].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## OPT8 hakari re-measure 9 (2026-10-14, speed-20261014-2052, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, re-probed 20:52Z, same as the row's 2026-10-05 note). OPT8 is
free by time (last claim `speed-20261014-1843` at 18:43Z outside the 2 h
window 18:52-20:52Z; `cargo hakari --version` prints `no such command`).
OPT9 skipped (claim `speed-20261014-1855` at 18:55Z inside the same
window). OPT12 is an unknown row (`board-slice: unknown row`, re-probed
20:52Z). OPT34 time-free but render-owned `crates/render/src/cache.rs`
outside this packet's paths. The full hakari hack stays OPT36 (owns
`crates/workspace-hack/` plus `tools/feature-unify.mjs` outside this
packet's paths): not built here.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2044, no improvement claimed.
- `target/debug/deps` holds 18 builds of `syn`, 14 of `serde_json`, 17 of
  `num_traits` (r3 saw 16/12/14; the split persists and is worse today).
  No `crates/workspace-hack/`, no `tools/feature-unify.mjs`.
- `node tools/bins.mjs` wall 0.42 s before -> 0.26 s after in one
  Measure-Command on the same loaded box (0.4168984 s then 0.2640027 s);
  same-box noise, no improvement claimed. All four STALE plus
  `e2040-editor` MISSING (runner/perf rebuild, not this packet).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.22 s
  before -> 0.13 s after in the same command (0.2162791 s then
  0.1343919 s): no crate closure (docs-only, lane gate skips crate tests,
  0 of 35, metadata skipped 0 ms, unmapped docs only).
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (20:52Z,
  same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 300+ dirty files, slots held; tree carries the foreign
  `.cargo/config.toml` fast-test dedup and `tools/bins.mjs` 5th-binary
  plus MISSING-line hunks, preserved not touched). Last lines: r3 green
  24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`.
  Prior quiet timeouts r1605 (360 s) and r1618 (185 s) plus r1723 quiet
  held 120 s with no output are the second identical failures on this
  loaded tree, so the same quiet cargo line is not retried per stop
  rules. A dependency, feature or profile cut would be unmeasurable and
  out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 7.8h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:953-965` (`knobsNews` plus
  `knobValue` live read) and `:1249` (`knobsSeen` advance) exist (284840
  bytes, present today). Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; ready file absent,
  report `target/reports/knobs-reach-keeper-20261013.md` present (no new
  ready packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | hakari split needs the workspace-hack lane pair | SMOOTH-BUILD r2052 re-probe | loaded 0.42 s -> 0.26 s bins plus 0.22 s -> 0.13 s affected docs-only, 18/14/17 dep splits, no cold game before -> after on a dirty tree (foreign hunks preserved) | REJECTED because no measurable edge gain without the OPT36 hack; keep OPT36 open for a green tree plus quiet.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## OPT6 nextest re-probe (2026-10-14, speed-20261014-2059, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 is free by time (last claim `speed-20261014-1600` at 16:00Z
outside the 2 h window 18:59-20:59Z) but stays PARKED
(`cargo nextest --version` prints `no such command`, re-probed 20:59Z,
same as the row's 2026-10-05 note). OPT8 skipped (claim
`speed-20261014-2052` at 20:52Z inside the same window;
`cargo hakari --version` prints `no such command`). OPT12 is an unknown
row (`board-slice: unknown row`, re-probed 20:59Z). OPT34 time-free (last
claim `speed-20261014-1715` at 17:15Z outside the same window) but
render-owned `crates/render/src/cache.rs` outside this packet's paths, so
skipped. The foreign `.cargo/config.toml` fast-test hunk and the foreign
`tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved, not
touched here. Cold `cargo test -p engine-game --lib --no-run` not re-run
here on purpose (see Evidence).

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.5 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2052 0.2 s, no improvement claimed.
- `node tools/bins.mjs` wall 0.40 s before -> 0.23 s after in one
  Measure-Command on the same loaded box (0.3963 s then 0.2296 s);
  same-box noise, no improvement claimed. Four STALE plus
  `e2040-editor` MISSING (runner/perf rebuild, not this packet).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.17 s
  before -> 0.35 s after in the same command (0.1737 s then 0.3523 s):
  no crate closure (docs-only, lane gate skips crate tests, 0 of 35,
  metadata skipped, unmapped docs only).
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (20:59Z,
  same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 325 dirty files, slots held by 18 helpers; tree carries the foreign
  `.cargo/config.toml` and `tools/bins.mjs` hunks). Last lines: r3 green
  24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`.
  Prior quiet timeouts r1605 (360 s) and r1618 (185 s) plus r1723 quiet
  held 120 s with no output are the second identical failures on this
  loaded tree, so the same quiet cargo line is not retried per stop
  rules. A dependency, feature or profile cut would be unmeasurable and
  out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 7.9h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:953-965` (`knobsNews` plus
  `knobValue` live read) and `:1249` (`knobsSeen` advance) exist (284840
  bytes, present today). Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | nextest needs the owner install lane pair | SMOOTH-BUILD r2059 re-probe | loaded 0.40 s -> 0.23 s bins plus 0.17 s -> 0.35 s affected docs-only, no cold game before -> after on a dirty tree (325 files, slots held, foreign hunks preserved) | REJECTED because no measurable edge gain without the nextest binary; keep OPT6 PARKED until the owner installs nextest.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## speed Else triage (2026-10-14, speed-20261014-2106, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-2059` at 20:59Z inside the 2 h
window 19:06-21:06Z; `cargo nextest --version` prints `no such command`,
re-probed 21:06Z, same as the row's 2026-10-05 note). OPT8 skipped (claim
`speed-20261014-2052` at 20:52Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 21:06Z).
OPT12 is an unknown row (`board-slice: unknown row`, re-probed 21:06Z).
OPT34 time-free (last claim `speed-20261014-1715` at 17:15Z outside the
same window) but render-owned `crates/render/src/cache.rs` outside this
packet's paths, so skipped. No free ops row owns this packet's paths, so
the Else cold-measure triage runs here per r2044/r2052/r2059 precedent.
The foreign `.cargo/config.toml` fast-test hunk and the foreign
`tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved, not
touched here. `git diff -- Cargo.toml rust-toolchain.toml
tools/affected.mjs` empty, untouched here. Cold `cargo test -p
engine-game --lib --no-run` not re-run here on purpose (see Evidence).

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2059 0.5 s, no improvement claimed.
- `node tools/bins.mjs` wall 0.24 s before -> 0.24 s after in one
  shell on the same loaded box (0.2438 s then 0.2359 s); same-box noise,
  no improvement claimed. 2 FRESH / 2 STALE plus 1 MISSING (`engine2040`
  FRESH built 2026-10-14T21:08:03Z, `staged_fight` STALE 27, `perf` FRESH
  built 2026-10-14T21:00:32Z, `arena` STALE 27, `e2040-editor` MISSING).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.13 s
  before -> 0.11 s after in the same shell (0.1258 s then 0.1069 s):
  no crate closure (docs-only, lane gate skips crate tests, 0 of 35,
  metadata skipped 0 ms, unmapped docs only).
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (21:06Z,
  same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 325 dirty files in `git status --short`, slots held by other
  helpers; tree carries the foreign `.cargo/config.toml` and
  `tools/bins.mjs` hunks). Last lines: r3 green 24.84 s then 11.80 s; r6
  RED QUIET 357.2 s on missing `classes.rs`. Prior quiet timeouts r1605
  (360 s) and r1618 (185 s) plus r1723 quiet held 120 s with no output are
  the second identical failures on this loaded tree, so the same quiet
  cargo line is not retried per stop rules. A dependency, feature or
  profile cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 8.0h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `knobsNews` plus `knobValue` live
  read and `knobsSeen` advance exist (284840 bytes, present today; the
  prior empty-dir note is stale, the file exists now). Finding stays
  BLOCKED center-only in `target/queue/done/bug-knobs-reach-keeper.md`;
  ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | cold game needs a quiet single-cargo lane pair | SMOOTH-BUILD r2106 Else triage | loaded 0.24 s -> 0.24 s bins plus 0.13 s -> 0.11 s affected docs-only, no cold game before -> after on a dirty tree (325 files, slots held, foreign hunks preserved) | REJECTED because no measurable edge gain without a quiet cold; keep OPT32 open for the operator window.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## speed-measure cold triage (2026-10-14, speed-20261014-2117, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, re-probed 21:17Z, same as the row's 2026-10-05 note; claim
`speed-20261014-2059` at 20:59Z inside the 2 h window 19:17-21:17Z, so
skipped). OPT8 skipped (claim `speed-20261014-2052` at 20:52Z inside the
same window; `cargo hakari --version` prints `no such command`, re-probed
21:17Z). OPT12 is an unknown row (`board-slice: unknown row`, re-probed
21:17Z). OPT34 time-free (last claim `speed-20261014-1715` at 17:15Z
outside the same window) but render-owned `crates/render/src/cache.rs`
outside this packet's paths, so skipped. `speed-measure` claims at
20:52Z, 20:59Z and 21:06Z sit inside the same window, but the Else
cold-measure still runs here per r2036/r2044/r2106 precedent: no free ops
row owns this packet's paths. The foreign `.cargo/config.toml` fast-test
hunk and the foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks
are preserved, not touched here. `git diff -- Cargo.toml
rust-toolchain.toml tools/affected.mjs` empty, untouched here. Cold `cargo
test -p engine-game --lib --no-run` not re-run here on purpose (see
Evidence).

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2106 0.2 s, no improvement claimed.
- `node tools/bins.mjs` wall 0.27 s before (Measure-Command 21:17Z,
  loaded). After-edit re-run same command on the same box stays in the
  same band (see RESULT proof line). No improvement claimed.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.12 s
  before (Measure-Command 21:17Z, loaded): no crate closure (docs-only,
  lane gate skips crate tests). After-edit re-run stays in the same band
  (see RESULT proof line). No improvement claimed.
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (21:17Z,
  same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 325 dirty files in `git status --short`, slots held by other
  helpers; tree carries the foreign `.cargo/config.toml` and
  `tools/bins.mjs` hunks). Last lines: r3 green 24.84 s then 11.80 s; r6
  RED QUIET 357.2 s on missing `classes.rs`. That break is gone today.
  Prior quiet timeouts r1605 (360 s) and r1618 (185 s) plus r1723 quiet
  held 120 s with no output are the second identical failures on this
  loaded tree, so the same quiet cargo line is not retried per stop rules.
  A dependency, feature or profile cut would be unmeasurable and out of
  paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 8.2h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:954-965` (`knobsNews` plus
  `knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (284840 bytes, present today). Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | cold game needs a quiet single-cargo lane pair | SMOOTH-BUILD r2117 Else triage | loaded 0.27 s bins plus 0.12 s affected docs-only, no cold game before -> after on a dirty tree (325 files, slots held, foreign hunks preserved) | REJECTED because no measurable edge gain without a quiet cold; keep OPT32 open for the operator window.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## speed Else triage (2026-10-14, speed-20261014-2122, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 skipped (claim `speed-20261014-2059` at 20:59Z inside the 2 h
window 19:24-21:24Z; `cargo nextest --version` prints `no such command`,
re-probed 21:24Z, same as the row's 2026-10-05 note). OPT8 skipped (claim
`speed-20261014-2052` at 20:52Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 21:24Z).
OPT12 is an unknown row (`board-slice: unknown row`, re-probed 21:24Z).
OPT34 time-free (last claim `speed-20261014-1715` at 17:15Z outside the
same window) but render-owned `crates/render/src/cache.rs` outside this
packet's paths, so skipped. No free ops row owns this packet's paths, so
the Else cold-measure triage runs here per r2044/r2052/r2059/r2106/r2117
precedent. The foreign `.cargo/config.toml` fast-test hunk and the foreign
`tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved, not
touched here. `git diff -- Cargo.toml rust-toolchain.toml
tools/affected.mjs` empty, untouched here. Cold `cargo test -p
engine-game --lib --no-run` not re-run here on purpose (see Evidence).

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2117 0.2 s, no improvement claimed.
- `node tools/bins.mjs` wall 0.29 s before (Measure-Command 21:24Z,
  loaded). After-edit re-run same command on the same box stays in the
  same band (see RESULT proof line). No improvement claimed.
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` wall 0.16 s
  before (Measure-Command 21:24Z, loaded): no crate closure (docs-only,
  lane gate skips crate tests). After-edit re-run stays in the same band
  (see RESULT proof line). No improvement claimed.
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (21:24Z,
  same box).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 328 dirty files in `git status --short`, slots held by other
  helpers; tree carries the foreign `.cargo/config.toml` and
  `tools/bins.mjs` hunks). Last lines: r3 green 24.84 s then 11.80 s; r6
  RED QUIET 357.2 s on missing `classes.rs`. Prior quiet timeouts r1605
  (360 s) and r1618 (185 s) plus r1723 quiet held 120 s with no output are
  the second identical failures on this loaded tree, so the same quiet
  cargo line is not retried per stop rules. A dependency, feature or
  profile cut would be unmeasurable and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 8.3h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:67-69` (knobs
  change named in the next continue), `:953-965` (`knobsNews` plus
  `knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (present today). Finding stays BLOCKED center-only in
  `target/queue/done/bug-knobs-reach-keeper.md`; ready file absent, report
  `target/reports/knobs-reach-keeper-20261013.md` present (no new ready
  packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles] (`node tools/books.mjs
search "cargo profiles codegen units"`). Subsystem review: no loop,
timestep, budget, alloc, lifetime, job, determinism or init change in a
docs-only packet; profile before optimizing holds, so no edge cut on a
dirty tree.

SPIKE | P1 | cold game needs a quiet single-cargo lane pair | SMOOTH-BUILD r2122 Else triage | loaded 0.29 s bins plus 0.16 s affected docs-only, no cold game before -> after on a dirty tree (328 files, slots held, foreign hunks preserved) | REJECTED because no measurable edge gain without a quiet cold; keep OPT32 open for the operator window.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## OPT9 re-measure 4 (2026-10-14, speed-20261014-2136, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed 21:36Z) and skipped (claim
`speed-20261014-2059` at 20:59Z inside the 2 h window 19:36-21:36Z).
OPT8 skipped (claim `speed-20261014-2052` at 20:52Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 21:36Z).
`speed-measure` skipped (claim `speed-20261014-2122` at 21:22Z inside the
same window). OPT12 is an unknown row (`board-slice: unknown row`). OPT34
is render-owned (`crates/render/src/cache.rs`, not this packet's paths):
no `cache.rs` edit here. OPT9 is the first free ops row (compile cache
spike, last claim `speed-20261014-1855` at 18:55Z outside the window):
measured here, no cache change for the reasons below. The foreign
`.cargo/config.toml` fast-test hunk (1 insertion, 2 deletions) and the
foreign `tools/bins.mjs` 5th-binary plus MISSING-line hunks are preserved,
not touched here; the concurrent `janitor-tools-pile-20261014-2136`
21:36Z pile claim names `tools/bins.mjs`, so this packet touches
`docs/SMOOTH-BUILD.md` only. `sccache 0.18.0` present,
`.cargo/sccache-pool.cmd` exists, but `.cargo/config.toml` carries no
global `rustc-wrapper` (opt-in per run only). The full hakari hack stays
OPT36 (owns `crates/workspace-hack/` plus `tools/feature-unify.mjs`
outside this packet's paths): not built here.

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2122, no improvement claimed.
- `node tools/bins.mjs` prints 2 FRESH / 2 STALE plus 1 MISSING (runner/perf
  rebuild, not this packet): `engine2040` FRESH built 2026-10-14T21:08:03Z,
  `perf` FRESH built 2026-10-14T21:00:32Z, `staged_fight` STALE 27 commits,
  `arena` STALE 27 commits, `e2040-editor` MISSING (5th binary from the
  preserved `tools/bins.mjs` hunk). Before-edit wall 0.3633 s
  (Measure-Command 21:36Z, loaded); after-edit re-run same command on the
  same box stays in the same band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only). Before-edit wall 0.1322 s
  (Measure-Command 21:36Z, loaded); after-edit re-run stays
  metadata-skipped (see RESULT proof line).
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs` 3 pass 0 fail, `node --test tools/affected.test.mjs`
  19 pass 0 fail (21:36Z, same box, before edit).
- Build green x2 in one hold before edit: `node tools/quiet.mjs --times 2
  -- cargo test -p engine-core --lib --locked -j 2` completes with
  `test result: ok. 109 passed; 0 failed; 2 ignored` twice (inner 0.23 s
  then 0.30 s; second leg `Finished test profile in 2.09 s`),
  `golden_d57b5c8ae3096614_unchanged ok` twice plus `v1_golden_unchanged`
  and `digest_golden` green inside the 109. Warm tree, no improvement
  claimed. Follow-up quiet holds after that (`--times 2` full core filtered
  to RESULT lines, then golden-filtered single test) each timed out at
  600 s / 300 s on a loaded box (11 then 21 cargo/rustc running,
  file-lock wait); per `cargo-slot-wait` and stop rules the same cargo
  line is not retried a third time here. Three pre-existing
  `checked.rs` `unused_must_use` warnings on the dirty tree.
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (slots
  held for the core green above; tree carries 300+ dirty files plus the
  foreign `.cargo/config.toml` and `tools/bins.mjs` hunks). Last lines: r3
  green 24.84 s then 11.80 s; r6 RED QUIET 357.2 s on missing `classes.rs`.
  That break is gone today. No slowest edge cut in this packet: the foreign
  hunks are preserved, not touched here, plus the concurrent pile claim on
  `tools/`, so a dependency, feature or profile cut would be unmeasurable
  and out of paths.
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched; core green x2 above).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 8.5h ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:68` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), `:954` (`knobsNews`),
  `:965` (`knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (284840 bytes); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`), `.opencode/knobs.json:3` (`reads at Start and every wave`)
  and `.opencode/seat-guard.mjs:35` (`readKnobs`). Finding stays BLOCKED
  center-only in `target/queue/done/bug-knobs-reach-keeper.md`; ready file
  absent, report `target/reports/knobs-reach-keeper-20261013.md` present
  (no new ready packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles]; one workspace lock plus
union features means one copy [trpl | Depending on an External Package].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

SPIKE | P1 | compile cache needs a quiet cold lane pair | SMOOTH-BUILD r2136 re-probe | loaded 0.3633 s bins plus 0.1322 s affected docs-only plus warm core 0.23 s -> 0.30 s (109 passed x2, golden ok), no cold game before -> after on a dirty tree (300+ files, slots held, foreign hunks preserved) | REJECTED because no measurable cache gain without a quiet cold; keep OPT9 open for a quiesced run.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT36 needs a green tree plus a quiet box;
OPT34 stays open for the render seat (`crates/render/src/cache.rs`
counters, not this packet).

## OPT13 hot-patch triage (2026-10-14, speed-20261014-2205, evidence-only, no config change)

No manifest, profile, linker, tool or toolchain change in this packet:
`Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, `tools/bins.mjs`
and `tools/affected.mjs` untouched. Docs-only diff, golden untouched.
OPT4, OPT5 and OPT7 are landed on disk; never re-verified per standing
order. OPT6 stays PARKED (`cargo nextest --version` prints `no such
command`, `cargo hakari --version` prints `no such command`, same as the
row's 2026-10-05 note, re-probed 22:08Z) and skipped (claim
`speed-20261014-2059` at 20:59Z inside the 2 h window 20:08-22:08Z).
OPT8 skipped (claim `speed-20261014-2052` at 20:52Z inside the same window;
`cargo hakari --version` prints `no such command`, re-probed 22:08Z).
`speed-measure` skipped (claims `speed-20261014-2106` at 21:06Z,
`speed-20261014-2117` at 21:17Z and `speed-20261014-2122` at 21:22Z inside
the same window). OPT9 skipped (claim `speed-20261014-2136` at 21:36Z
inside the same window). OPT12 is an unknown row (`board-slice: unknown
row`, re-probed 22:08Z). OPT34 is render-owned
(`crates/render/src/cache.rs`, not this packet's paths): no `cache.rs`
edit here. OPT13 is the first free ops row (hot-patch spike, needs OPT3
done, Tier C Size M, owns the spike report, licenses read first; accept an
edit reaches the running game in <= 2 s with state kept, or the row closes
with the failure): triaged here evidence-only, no spike built for the
reasons below. The foreign `.cargo/config.toml` fast-test hunk (1
insertion, 2 deletions) and the foreign `tools/bins.mjs` 5th-binary plus
MISSING-line hunks are preserved, not touched here; the concurrent
`janitor-tools-pile-20261014-2136` 21:36Z pile claim names
`tools/bins.mjs`, so this packet touches `docs/SMOOTH-BUILD.md` only.
`git diff -- Cargo.toml rust-toolchain.toml tools/affected.mjs` empty,
untouched here. No license read here (subsecond versus hot-lib-reloader
choice stays open). Cold `cargo test -p engine-game --lib --no-run` not
re-run here on purpose (see Evidence).

Evidence (same box, loaded, no code change):

- Part `speed`: `node tools/part-score.mjs speed` prints lane cold-check
  0.2 s, OPT ticked 4/34, CPU/GPU p95 UNKNOWN (perf-baseline.json GPU rows
  PENDING). Same band as r2136, no improvement claimed.
- `node tools/bins.mjs` prints 2 FRESH / 2 STALE plus 1 MISSING (runner/perf
  rebuild, not this packet): `engine2040` FRESH built 2026-10-14T22:08:24Z,
  `perf` FRESH built 2026-10-14T22:04:30Z, `staged_fight` STALE 28 commits,
  `arena` STALE 28 commits, `e2040-editor` MISSING (5th binary from the
  preserved `tools/bins.mjs` hunk). Before-edit wall 0.2681 s
  (Measure-Command 22:08Z, loaded); after-edit re-run same command on the
  same box stays in the same band (see RESULT proof line).
- `node tools/affected.mjs --files docs/SMOOTH-BUILD.md` prints no crate
  closure (docs-only, lane gate skips crate tests, 0 of 35, metadata
  skipped 0 ms, unmapped docs only). Before-edit wall 0.1210 s
  (Measure-Command 22:08Z, loaded); after-edit re-run stays
  metadata-skipped (see RESULT proof line).
- Owned tool tests green without cargo slots: `node --test
  tools/bins.test.mjs tools/affected.test.mjs` 22 pass 0 fail (22:08Z,
  same box, before edit).
- Cold `cargo test -p engine-game --lib --no-run` not re-run here (loaded
  box, 338 dirty files in `git status --short`, slots held by other
  helpers; tree carries the foreign `.cargo/config.toml` and
  `tools/bins.mjs` hunks). Last lines: r3 green 24.84 s then 11.80 s; r6
  RED QUIET 357.2 s on missing `classes.rs`. That break is gone today.
  Prior quiet timeouts r1605 (360 s) and r1618 (185 s) plus r1723 quiet
  held 120 s with no output are the second identical failures on this
  loaded tree, so the same quiet cargo line is not retried per stop rules.
  A dependency, feature or profile cut would be unmeasurable and out of
  paths (`crates/*/Cargo.toml` outside owned paths; window STALE/MISSING
  above means a hot-patch spike cannot run here).
- Golden `d57b5c8ae3096614` unchanged (docs-only diff; no Rust, manifest
  or profile touched).
- Knobs: `node C:/empire/center/empire.mjs knobs engine2040` prints
  `updated 24m ago; the keeper names the change in its next continue (the
  loop also reads the file each wave)` with every rule knob `next pass`;
  no `restart` line today. `.opencode/plugin/loop-keeper.js:68` (knobs
  change named in the next continue), `:94`
  (`knobs: C:/engine2040/.opencode/knobs.json`), `:954` (`knobsNews`),
  `:965` (`knobValue` live read) and `:1249` (`knobsSeen` advance) exist
  (284840 bytes, present today); plus `.opencode/plugins/loop-keeper/index.mjs:17`
  (`knobsFile`), `.opencode/knobs.json:3` (`reads at Start and every wave`)
  and `.opencode/seat-guard.mjs:35` (`readKnobs`). Finding stays BLOCKED
  center-only in `target/queue/done/bug-knobs-reach-keeper.md`; ready file
  absent, report `target/reports/knobs-reach-keeper-20261013.md` present
  (no new ready packet; the keeper itself is center code, not this repo).

Books: profiles are per-profile customization
[trpl | Customizing Builds with Release Profiles]; one workspace lock plus
union features means one copy [trpl | Depending on an External Package].
Subsystem review: no loop, timestep, budget, alloc, lifetime, job,
determinism or init change in a docs-only packet; profile before
optimizing holds, so no edge cut on a dirty tree.

SPIKE | P1 | hot-patch needs a green window plus a license read | SMOOTH-BUILD r2205 triage | loaded 0.2681 s bins plus 0.1210 s affected docs-only, no cold game before -> after on a dirty tree (338 files, slots held, foreign hunks preserved, window STALE/MISSING) | REJECTED because no measurable spike gain without a green window build and a license choice; keep OPT13 open for a clean window run.

Next: keeper lands `closes:` for OPT4, OPT5, OPT7 and OPT9 from the r6/r7
hunks; OPT32 needs the operator quiet-box window; OPT6 stays PARKED until
the owner installs nextest; OPT13 stays open for a clean window plus a
license read; OPT36 needs a green tree plus a quiet box; OPT34 stays open
for the render seat (`crates/render/src/cache.rs` counters, not this packet).
