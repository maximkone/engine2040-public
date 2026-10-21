# SMOOTH-BUILD — build the game without friction; let problems fix themselves

Practical features only, each with a board row in `SPRINT.md`. The test for
every item: does it save a person or an AI time on a normal day of building
the arena game? Sections 1-6 are planned workflows, not shipped features;
"Build speed facts" at the end is what is true today.

The doctor (`node tools/doctor.mjs`) is report-only unless you pass
`--consent`. There is no workspace-wide `/fix` command; its rules (report
first, failing package only, dirty-set check, one retry) live in `AGENTS.md`
"Proof" and "Evidence and blockers".

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
   package-scoped compiler suggestions; no workspace-wide mutation, no blanket `--allow-dirty`, and no changes to unfamiliar user work.
   Review suggestions and re-run the same failing command plus the full gate.
   One diagnosed retry; the same failure twice gets a blocker with exact output.
   Unsupported compiler errors remain red, not "fixed" by disabling checks.
   The standalone command file is gone (retired in `c71841f`); these rules
   now live in `AGENTS.md`.
- Startup checks the environment read-only. Installing tools, deleting
   caches or repairing locks are separate explicit operations. A green gate
   never authorizes committing somebody else's dirty files.

## 4. The project fixes itself

- **Doctor (DOC1, closed 2026-10-05):** `node tools/doctor.mjs` checks the pinned
  Rust toolchain and components, Node, cargo-deny, the
  lockfile, the workspace and board audit, disk space and git state. Bare
  `--fix` only reports what it would do; live leases are never released;
  stale release needs `--consent=lock` and names the holder; pool eviction
  needs `--consent=cache` plus low disk. Proof: `node --test tools/doctor.test.mjs`
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

## Build speed facts (2026-10-17)

- Linker: `rust-lld.exe` on Windows (`.cargo/config.toml`), 121 s -> 79 s cold link vs `link.exe` (OPT4).
- Profiles: `release` (thin LTO, 16 units, since OPT45 on 2026-10-21), `profiling` (release, thin LTO, line tables), `ship` (fat LTO, 1 unit, the packaged artifact and the CI release-proof leg) in root `Cargo.toml`; `fast-test` (dev, debug 0) in `.cargo/config.toml` (OPT5). The `perf-test` profile is gone (OPT46): it was the same shape as `release` under a second name, so it meant a second target dir and a second full compile of the tree.
- Affected gate: `node tools/affected.mjs` maps a diff to changed packages plus reverse deps; docs-only diffs skip `cargo metadata` (OPT7).
- Compile cache: `sccache` is opt-in per command only (`RUSTC_WRAPPER=sccache` and `CARGO_INCREMENTAL=0` on that one command). No global `rustc-wrapper`: `.cargo/config.test.mjs` guards it. CI sets `CARGO_INCREMENTAL=0` (OPT9, OPT32).
- Cache across lane pools: measured DROP 2026-10-20 (`docs/receipts/2026-10-20-opt9-compile-cache-spike.md`). One lane = `cargo test -j 4 --locked -p engine-physics --all-targets --no-run`: cold 21.6 s, and a filled cache in a fresh `CARGO_TARGET_DIR` served 0 of 2 cacheable units (warm 13.6 s, within the 23.5% run spread), 2 lanes with different target dirs get different keys even with `SCCACHE_BASEDIRS`. Only the rlib units are cacheable at all (`crate-type` units are refused: 24 of 30 requests). It pays in one target dir (clean rebuild, branch switch) and on CI with a stable checkout path; a fresh dir per lane gains nothing, so no wrapper comes back.
- Parallel builds: give each one its own `CARGO_TARGET_DIR` so they never wait on one build-dir lock. Heavy test and clippy runs go to GitHub (`node tools/gh-check.mjs`, skill `github-check`).
- Dev build (observed 2026-09-30, warm, `-j 4`): 347 s wall; `engine-render` is the critical path at 294 s (frontend 99.7 s, codegen 194.3 s), then `engine-assets`, `engine-nav`, `engine-proof`, `engine-audio` at 150-170 s each.
- Dependency duplicates (measured 2026-10-21 by parsing `Cargo.lock` at HEAD `c36c060e`, not by counting build artifacts): 1,032 packages, 80 names carry more than one version, 101 extra version copies. `serde_json` is one version (1.0.151) and `num-traits` is one version (0.2.19) and is not a direct dependency of any crate; `syn` is three versions (1.0.109, 2.0.119, 3.0.6). The old "16 syn / 12 serde_json / 14 num_traits" figures were 2026-10-17 build-artifact counts in `target/debug/deps` and drift with every build, so the unification target is the 80 multi-version names, not serde_json or num-traits.
- Release shape (measured 2026-10-21 on the GitHub runner, same line `cargo test -p engine2040 --release --test proof_cli default_and_max_seed`, the same 22 crates compiled from scratch in both runs): thin LTO with 16 codegen units **38.08 s** (run 39576029326) vs fat LTO with one codegen unit **58.84 s** (run 39576660515), and the pre-change fat baseline **1 m 05 s** (run 39575327069). All three PASS with `"final_hash":"d57b5c8ae3096614"`. On this PC the fat arena release build took **11 m 41 s**, ~9.7 min of it in the one-unit relink. The 60 fps floor is unchanged and the 300-frame check on the thin binary is still open (OPT45).
- Open: `engine-workspace-hack` exists but no crate depends on it, so it unifies nothing yet (OPT36); `cargo nextest` is not installed (OPT6 parked); hot-patching gameplay code is unmeasured (OPT13); splitting `engine-render` is the biggest build-time lever left (OPT11); the 2026-10-21 build-CPU card (`docs/research/build-cpu-2026-10-21/CARD.md`) ranks the remaining levers - the thin-LTO build shape landed as OPT45, the rest are still ideas.

The OPT measurement logs of 2026-09-30 to 2026-10-16 (about 40 runs) left this page on 2026-10-17. Read them with `git log -p -- docs/SMOOTH-BUILD.md`.
