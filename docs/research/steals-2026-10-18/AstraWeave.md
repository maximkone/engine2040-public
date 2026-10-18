STEAL CARD: ASTRAWEAVE (wave 2026-10-18, block 7, position 17). Read-only pass; donor files read live
2026-10-18, paths taken from the pinned tree. No bytes vendored.
Repo: https://github.com/lazyxeon/AstraWeave @ 5bde0d8fa9d3 (main, updated 2026-10-18T14:28Z, 7 stars).
Licence: MIT (LICENSE:1-3 "MIT License / Copyright (c) 2025 Pheonetic Coder", read 2026-10-18).
What it is: a very large Rust game-engine monorepo (27k files: `astraweave-*` crates, an egui editor, ~30 tools and a
"Veilweaver" vertical-slice runtime), built by human+AI collaboration. Wide and uneven: many `_archive/` baselines and
`mutation_resistant_*` test files. Read for structure and proof discipline, not for code volume.

=== 1. WE HAVE ===
- A golden hash and per-system split with the first diverging system (crates/arena/src/sync.rs:29-46, :65-85) plus
  tools/desync-diff.mjs printing the first diverging system, and crates/physics/src/rapier.rs:283 `state_hash`.
- A runner-lane check runner (node tools/gh-check.mjs; a pass with 0 tests reruns; exit 2 is "could not run") and the
  open HS2 row (per-OS hash legs, per-tick rows) in the finish-line gap list.
- Perf receipts and tiers (crates/perf, tools/potato-check.mjs, a perf CSV).

=== 2. STEALS (ranked) ===
1. N-run determinism traces with a per-subsystem fingerprint and a divergence report | veilweaver_slice_runtime/src/determinism.rs:1-26,
   :44-57, :81-101, :230-284, :288-300, :322-372, :518-565, :642-657 (read 2026-10-18) | MIT | SIM (proof, headless) |
   lands crates/proof or tools/desync-diff.mjs | S-M
   Idea: a run records one `StateFingerprint` per tick - tick + named sub-hashes (game_loop, hud, telemetry, vfx) + a
   combined hash (":44-57"), a `DeterminismTrace` per run, then `compare` walks the ticks and pushes a `DeterminismMismatch`
   per bad tick (":230-284"); the report names the FIRST divergence tick AND the subsystem (`identify_divergence`, ":288-300";
   `first_divergence_tick`, ":351"). A `MultiRunValidator` holds N traces and compares each against the first as baseline
   (":540-565"), and the summary is one printable line (":356-372"). Tests pin it: a divergent HUD run reports tick 5 and
   subsystem "hud", a missing tick reports "missing_in_b" (":642-672").
   Maps: our verify_rollback compares a resume against the recorded rows inside ONE run (crates/arena/src/sync.rs:168-186)
   and desync-diff prints the first diverging system; we have no cross-RUN report, no missing-tick/ratio view and no
   one-line summary to paste into a receipt.
   Moves: keep our SystemHashes; add a compare over N recorded run files that reports first divergence tick + system +
   missing ticks and a matched ratio, and print one line (`run A vs run B: 600/600 ticks, first divergence none`).
   Proof: two identical tapes report 600/600; one flipped bit in one system reports the tick and that system by name.
   Kill: kill the split if it names more systems than the golden already splits. A per-entity or per-field hash at 600
   ticks buys nothing new over SYSTEM_NAMES and costs CI time.

2. A required cross-OS hash check whose compare job fails on a MISSING case | .github/workflows/determinism-replay.yml:1-10,
   :37-43, :56-67, :76-87, :89-122, :133-178 (read 2026-10-18) | MIT | SIM/PROOF (hash law, CI) | lands the HS2 row +
   tools/gh-check.mjs lane recipe | M
   Idea: a `hash` job runs a matrix of ubuntu/windows/macos with `fail-fast: false` and 45 min (":37-43"), pinning the
   codegen baseline first - `RUSTFLAGS=-C target-cpu=x86-64` on x86_64, `target-cpu=native` forbidden (":56-67") - then runs
   one named replay test per crate (":76-87") and merges the per-crate JSON, failing on a case the merge does not see
   (":89-122"). The `compare` job (10 min) downloads the per-OS artifacts and fails when a case is missing on any OS or
   when the distinct digests for a case are not exactly one (":133-178"). The header keeps the required case list itself,
   so deleting a test is a deliberate edit, and it says the job has NO retry loop (":1-10").
   Maps: our hash is in-crate and one machine; docs/HASH-LAW.md wants the same hash on every PC, and the HS2 row asks for
   per-OS legs. Our gh-check lane runs one cargo line per call with no cross-leg compare and no pinned target-cpu.
   Moves: one chaotic scene, one hash per leg, uploaded as an artifact; a compare step that fails on a missing case as
   loudly as on a diff; the required case list written in the workflow. Add the target-cpu pin so codegen cannot differ.
   Proof: two legs on the PC (x86-64 and a second target) print the same digest; renaming a case turns compare red.
   Kill: kill it below two OS legs (one leg says nothing about another PC) or if the artifact round trip pushes the job
   past its timeout; keep the PC legs and drop the expensive one.

3. One frame-budget line against a named machine class, gating p95 | aravath_runtime/src/frame_budget.rs:7-10, :21-50,
   :56-66 (read 2026-10-18) | MIT | PERF | lands crates/perf + tools/potato-check.mjs output | S
   Idea: the budget is a constant WITH a class name (`FRAME_BUDGET_MS = 11.0`, `BASELINE = "GTX 1660 Ti-class"`, ":7-10");
   samples are nanos and report min/p50/p95/max (":21-45"); the gate is `p95 < budget` (":47-49"); and every run prints one
   line naming n, min, p50, p95, max, the budget and the class (":56-66"). The header says a software adapter is not that
   GPU (":1-5").
   Maps: our perf receipts report numbers (tools/potato-check.mjs, the perf CSV) but a budget without its class, or a mean
   instead of p95, hides both the machine and the spike.
   Moves: one budget line per check, class in the same string, p95 as the gate; the tier name travels with the number.
   Proof: run the check on the potato tier and on the dev box: both print the class and their own p95; the gate fails only
   when p95 crosses.
   Kill: kill any budget line with no machine class, and any gate on the mean. A mean over 600 ticks hides the one bad frame
   that the duel feels.

4. A benchmark history keyed by tree state, with a percentage regression gate | tools/benchmark-dashboard/benchmark-data/history.jsonl:1,
   tools/benchmark-dashboard/README.md:5, :23-26 (read 2026-10-18) | MIT | PERF/TOOLING | lands the perf CSV + a gate |
   M
   Idea: every row carries the value plus the tree it came from - `git_branch`, `git_sha`, `git_dirty`, `timestamp`, crate,
   `benchmark_name` (history.jsonl:1), so a point can be re-checked; the dashboard reads a threshold file and its CI opens
   an issue for a regression above 10% over the 30-day window (README:23-26).
   Maps: our perf CSV is the same idea; the two columns worth adding are the commit sha and a dirty flag, because a number
   taken on a dirty tree must not become a baseline.
   Moves: refuse to record a row when the tree is dirty (or mark it `dirty: true` and let the gate ignore it); gate on a
   percentage of the SAME fixture, not on absolute milliseconds.
   Proof: run the same bench twice at one sha (rows differ only in timestamp); run on a dirty tree and the gate reports
   "no baseline".
   Kill: kill the percentage gate if the fixture or the machine class is not pinned in the row, or if a dirty-tree number
   can enter the baseline window. Then it retires good changes and hides real ones.

=== 3. NOT WORTH IT ===
- The 27k-file surface: `_archive/renderer.rs.baseline`, `.broken.keep`, per-subsystem `mutation_resistant_*.rs` (30+ files).
  Names are not evidence; say what a test decides and what it costs.
- aw_trace_sync (tools/aw_trace_sync/src/main.rs:1-13, :21-23, :30-40: YAML front-matter is the source of truth, a
  marker-bounded generated table, `--check` gates drift, `--list-untraced` names a module no trace claims): close to what
  tools/workspace-check.mjs, tools/plan-index.test.mjs, tools/wiring-cov.mjs and tools/stale-refs.test.mjs already do; a
  YAML front-matter contract on every doc is a second source of truth we do not need.
- The egui editor and its ~40 panels, the D3 dashboard page (needs a JS CDN) and the ecs/ai/render mega-crates: no consumer
  in engine2040; render/anim risk is covered by our own cards.
- The `determinism-replay` case list as a model for engine2040 test naming: it is a useful idea, not a file to copy.

DONE 4 steals, 8 donor files read.
