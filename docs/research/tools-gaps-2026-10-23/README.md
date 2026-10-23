# Tools and dev-loop gaps (2026-10-23)

**Verdict.** The *inside* of the loop is mature: fmt, clippy, rust-quality ratchet, cargo-deny, nextest with CTRF,
Miri/loom/Kani, golden matrix, ship reproducibility, mdBook plus a book link gate — all wired and running.
The weak half is everything that measures quality *over time* and everything that *ships*: coverage, perf regression,
fuzz/mutation campaigns, browser/pixel gates in standing CI, release/version/changelog, hook install, SBOM.
**And the tool layer was not kept as an inventory:** its own index missed 29 of 126 live tools, 3 tools were named
nowhere, one 78 KB untracked near-copy of the workspace gate sat beside it, and 5 gates only run on a throwaway branch.
The inventory gate (`tools/tools-index.mjs --check`) and the README/book fixes landed 2026-10-23.

Counts (re-measured 2026-10-23, `tools/` top level, before the fix): **126 live `.mjs` + 151 `*.test.mjs`**
(now 127 + 151), plus the JSON data, the `.ps1` pool script, the `.py` helper and the subdirs. **22 tools are named in
`.github/workflows`, 21 of them executed by a workflow step** (gh-check is the dispatcher client, not a step).
**Orphans then: 3 named nowhere; 13 with no test mate; 29 missing from `tools/README.md`; 48 missing from
`docs/book/ref-tools.md`.** The gate and the two indexes now hold the README, book and stale-entry counts at 0; the 13
no-mate tools are listed with a reason each. **8 duplicate clusters cover 73 of the 126 live tools**, plus one exact duplicate.

## What is already better than most peers

- Rust gates: `cargo fmt --check`, `clippy -D warnings`, `rust-quality.mjs` ratchet, `cargo deny`, `rustfmt.toml`
  pinned, `rust-toolchain.toml` pinned, `--locked` everywhere (`ci.yml` quick + clippy jobs).
- Semantic proofs: Miri (3 legs + `miri-sites.json` audit), loom job pool, Kani (20 harnesses), golden matrix on
  5 machines, differential ship runs, tape/replay gates (`proof.mjs`, `tape-checksum.mjs`, `golden-matrix.mjs`).
- Build: sccache + rust-cache in CI, jobserver slot pool, `gh-check.mjs` farm client, `affected.mjs` closure,
  compile-cache measured and correctly rejected (DROP) with the receipt.
- Docs: mdBook (`docs/book/book.toml`), `book-links.mjs` in CI, `door-reference.mjs` generated reference.
- Board/agents: `board-slice`, `board-memo`, `finish`, `part-score`, `ticked-rows-audit`, `leftovers`, `doctor`.

## Ranked gaps

| # | Gap | Who has it | Ours | Class | Smallest build | Risk |
|---|---|---|---|---|---|---|
| 1 | Browser gates not in standing CI | Godot/Unity/Flutter run web E2E per PR; standard practice | `web-build.yml` runs only on `web-build-*` / dispatch; `pages.yml` only builds. The 6 web gates (`web-browse`, `web-perf`, `web-input-check`, `web-budget`, `web-headers`, `webgpu-cts`) never run on a PR | Wired but never run | Add `browser` job to `ci.yml` (build wasm + `web-browse.mjs` + `web-input-check.mjs` on headless Chrome) | Slow (~6 min), flaky software adapter |
| 2 | No perf/instruction regression leg | UE/Godot/Flax ship perf CI; Rust peers use `iai-callgrind`/gungraun | `crates/perf/benches/instructions.rs` (gungraun), `bench-compare.mjs`, `proof --perf`, `tools/perf-baseline.json` all exist; **no workflow runs `cargo bench` or `--perf`** | Half-done | `perf` job on main: `cargo bench -p engine-perf --bench instructions` then `node tools/bench-compare.mjs` vs `perf-baseline.json` | Noisy unless instruction counts only; needs gungraun-runner |
| 3 | No coverage gate | cargo-llvm-cov/tarpaulin are standard; consoles measure hot-path coverage | `wiring-cov.mjs` needs `cargo llvm-cov`, is NOOP without it, manual only (`SPRINT-18-steals.md` GS2). No `llvm-cov` string in `.github` | Totally missing | `coverage` job: `cargo llvm-cov --workspace --lcov -p engine-core -p engine-physics -p engine-arena`, upload artifact, ratchet a floor per crate | Coverage chasing; cost ~15 min |
| 4 | Tool inventory is unmaintained | Engines keep a tool index that a gate enforces | 29/126 live tools were missing from `tools/README.md`; 48 from `docs/book/ref-tools.md`; the README claimed "each script has a `*.test.mjs` mate" but 13 did not; the test-only list named 29 of 37. Fixed 2026-10-23: `tools/tools-index.mjs --check` runs in the `ci.yml` quick job, both indexes name every live tool, and the 13 no-mate tools carry a reason each | Half-done | `tools/tools-index.mjs --check` in the `ci.yml` quick job: every live tool named in README + a test, no stale entry | None; pure text |
| 5 | Dead and duplicate tools | CI rejects an orphaned entry | 3 named nowhere (`content-cross`, `faction-war-map`, `starter-classes`: zero refs, no test mate); `tools/.qw4-check.mjs` is an untracked 78 KB near-copy of `workspace-check.mjs`, idle 9.5 h, an older pre-deletion snapshot of the removed PW51 baseline gate (exits 0 on errors, stale edge maps), not pending work | Totally missing | Owner call: wire the 3 content-gate orphans into a content runner or delete them; restore PW51 or drop the `.qw4` copy; keep one workspace audit | Someone's in-flight copy: coordinate, never delete in a hurry |
| 6 | API-break gate not in CI | `cargo-semver-checks` in CI for any published crate | `api-break.mjs` + a committed `tools/api-break.baseline.json` (36k lines of pinned item hashes) exist; no workflow names it | Half-done | Add `node tools/api-break.mjs` to `release-proof` (build already done) | rustdoc JSON cost; baseline bless discipline |
| 7 | Hooks are not shipped or installable | Most engines commit `.githooks/` + `core.hooksPath` | `tools/hooks/pre-commit` exists; the **pre-push** hook (git-lfs + `cargo deny` + `license-gate.mjs`) lives only in this PC's `.git/hooks`; `core.hooksPath` unset; no installer | Half-done | Commit `tools/hooks/pre-push`, add `tools/hooks-install.mjs` (sets `core.hooksPath`), `doctor.mjs` checks it | A fresh clone silently skips the ratchet and secret scan |
| 8 | No release/version/changelog plumbing | `release-please`, `git-cliff`, `cargo-dist`, `cargo-cyclonedx` | No `CHANGELOG.md`, no tag workflow, no version bump tool, no SBOM; `api-break` says "bless after the version bump" but nothing checks it; ship-check builds but publishes nothing | Totally missing | `tools/release.mjs`: bump `[workspace.package] version`, generate CHANGELOG from board rows since last tag, tag, write an SBOM, attach `ship-check` artifacts | Version drift vs baseline; needs an owner call on versioning policy |
| 9 | Fuzz and mutation never scheduled | Engines run nightly fuzz/mutation | `crates/proof/src/fuzz.rs` (feature `fuzz`) and `tape_fuzz.rs` exist with unit tests; `mutants.mjs` is opt-in `proof --mutants`; no cron, no corpus growth, no artifact | Half-done | Weekly `fuzz.yml` + `mutants.yml`: seed sweep, `--features fuzz`, 10k tape mutations, keep corpora as artifacts | Runtime; needs triage owner |
| 10 | Flake quarantine consumes nothing | nextest retries + quarantine list | `flaky-quarantine.mjs` is called only as `--self-test` by `workspace-check` (report-only WARN); `proof.mjs` mentions it in a comment but never spawns it; nothing writes `target/flaky-counts.json` | Half-done | Parse nextest `retried` lines in `proof.mjs` into counts, fail at ≥2, move the test to `--perf` | Hides a real flake as "quarantined" |
| 11 | rustdoc warnings are counted, not gated | `RUSTDOCFLAGS=-D warnings` in CI | `pages.yml` prints `rustdoc warnings: N` into the summary; `Cargo.toml` lints set `broken_intra_doc_links = "warn"`, comment says "verify by hand" | Half-done | Fail `pages.yml` when `RUSTDOC_WARNINGS > 0` (or ratchet a file) | Existing warning debt: ratchet, do not hard-fail |
| 12 | Docs links checked only inside the book | mdBook + linkinator/lychee repo-wide | `book-links.mjs` covers `docs/book` + the assembled site; 1,375 files under `docs/` outside the book are unchecked; `index-check.test.mjs` and `stale-refs.test.mjs` cover one file each, as tests | Partial | Extend `book-links.mjs` with `--docs` over `docs/**` against a baseline, run in `ci.yml` quick | Dead links in 1.3k research docs: baseline, ratchet down |
| 13 | Secret scan is staged-diff only | gitleaks/trufflehog on every CI run and on history | `secret-gate.mjs` runs from the pre-commit hook on the staged diff; no CI step, no history scan, hook not installed by default (see #7) | Partial | Add `node tools/secret-gate.mjs --all` (or gitleaks) to `ci.yml` quick | Public mirror: a leak is public the moment it runs |
| 14 | Crash minidump is a stub | Crashpad/minidumper + symbol store | `crates/game/src/crash.rs` writes a report and a zero-byte-style stub dump (`write_minidump_stub`); no capture, no symbolication, no collection | Half-done | Real minidump writer behind a dev feature + `crash` door verb that bundles the last crash | Windows-only path, foreign crates; big |
| 15 | No project templates (TPL1) | `godot new`, UE templates, Flax samples | `SMOOTH-BUILD.md` TPL1 and `docs/book/recipes-*.md` describe arena/prop/weapon/fighter templates; no template tree, no `engine new`, TPL1 absent from `tools/` and `crates/` | Totally missing | `tools/new.mjs` writing an arena item from a checked template + a boot test | Scope creep; needs an owner call on the first template |
| 16 | No devcontainer, thin onboarding | Bevy/Godot ship devcontainers | `.devcontainer` absent, no `.vscode/`, no `.tool-versions`; onboarding = `doctor.mjs` + prose | Totally missing | `.devcontainer/devcontainer.json` (rust 1.97.1 + node 24 + cargo-deny) and `doctor.mjs` in `postCreate` | Windows-first repo: a Linux image proves less than it claims |
| 17 | OS matrix: no macOS, Windows not the unit suite | Engines run mac/win/linux tests | `macos` appears in no workflow; Windows gets `cargo check` (`window-compile`), differential runs and ship-check, never the 4 nextest groups | Partial | Add `windows-latest` to one `test` group; macOS only after a runner budget exists | Windows test cost and flake |
| 18 | Local runner ≠ CI runner | Same runner everywhere | CI uses `cargo nextest` (`profile ci`); local proofs use `cargo test`; `nextest-spike` is parked and `SMOOTH-BUILD.md` still says nextest is not installed; `opencode.jsonc` denies `cargo nextest*` | Half-done | Install nextest; let `proof.mjs` use the already-written `proof` profile, or delete the profile to stop drift | Two runners, two flake distributions |
| 19 | No commit-level bisect helper | `git bisect run` wrappers are common | `replay.bisect` exists *inside* the engine; no tool finds "which commit broke the golden/perf" (`git bisect` appears nowhere in `tools/`) | Totally missing | `tools/bisect.mjs <cmd>`: `git bisect run` over `proof.mjs --skip-build` or a bench filter, in a temp index | Rewrites checkouts: must use a dry-run/abort path |
| 20 | No dependency update automation | Dependabot/Renovate default | `.github` holds only `workflows/`; 80 multi-version names in `Cargo.lock` (`SMOOTH-BUILD.md`); no `dependabot.yml` | Totally missing | `.github/dependabot.yml` weekly cargo + actions, grouped | PR noise in a shared tree |

## Orphan and near-orphan tools (top 15)

| Path | Last referenced where | Verdict |
|---|---|---|
| `tools/.qw4-check.mjs` | untracked; idle 9.5 h (mtime 2026-10-23T11:04Z); no doc, no test, no runner | Older pre-deletion snapshot of the removed PW51 baseline gate, a near-copy of `workspace-check.mjs`: it exits 0 on errors and has stale edge maps. Not pending work. Owner decision: restore PW51 or drop the copy; this seat left it untouched |
| `tools/content-cross.mjs` | nowhere | Zero refs, no mate. It gates the live `content/systems` ids and `content/classes` files; owner call to wire it into a content runner or delete it. Left in place 2026-10-23 |
| `tools/faction-war-map.mjs` | nowhere | Zero refs, no mate. It reads the live `content/maps/contested_middle.json` and `content/systems/war.json`; same owner call. Left in place |
| `tools/starter-classes.mjs` | nowhere | Zero refs, no mate. One-shot writer for the live `content/classes` starters; same owner call. Left in place |
| `tools/fk1_determinism.mjs` | `tools/README.md` only | No test, no runner; manual FK1 double-run |
| `tools/unwired-code-gate.mjs` | `docs/plan/parked-2026-10-22.md` + comments in `workspace-check.mjs` | No test, no runner; overlaps `wiring-scan` + `workspace-check` |
| `tools/gate-silhouette.mjs` | `docs/research/powerups-2026-10-21/BOARD-MERGE.md` | No test, no runner; needs a PNG pair nobody publishes |
| `tools/arena-shot.mjs` | `docs/plan/quality.md`, `docs/plan/render-look.md` | No test mate; GPU-only, manual, no standing run |
| `tools/faction-war-report.mjs` | `docs/research/faction-war-2026-10-21/art-report.md` | No test; its only caller (`class-validate`) is itself dead |
| `tools/class-validate.mjs` | `tools/faction-war-report.mjs` | Only caller is a dead tool |
| `tools/kam2-host.mjs` | `docs/plan/kam2-hosting.md` | No test; manual, tied to a plan |
| `tools/kam2-web.mjs` | `tools/kam2-host.mjs` | Chain of manual tools, no test |
| `tools/reference-captures.mjs` | `tools/reference-web.mjs` | Chain of manual tools, no test |
| `tools/nextest-spike.mjs` | `docs/plan/build-cpu.md`, `tools/jobserver.test.mjs` | Parked spike; CI installs nextest directly, so the tool never runs |
| `tools/cargo-guard.mjs` | `tools/README.md` + `docs/book/ref-tools.md` only | Index-only; no runner, no spawn anywhere |

Index-only set also includes `rdc-diff`, `rerun-timeline`, `seal-keys`, `p4-netsim`, `hit-list` (README/ref-tools
only, no runner). `tools/README.md` now lists all 37 test-only mates; the inventory gate checks live tools only.

## Duplicate clusters (one job, several scripts)

| Cluster | Tools | Note |
|---|---|---|
| Repo/API static audit | 13: `workspace-check`, `.qw4-check`, `arch-fitness`, `crate-map`, `feature-scan`, `feature-unify`, `unwired-code-gate`, `wiring-scan`, `api-break`, `docs-coverage`, `cast-safe`, `sim-source-scan`, `review-gate` | Only `workspace-check`, `rust-quality`, `api-break` (never run), `docs-coverage` (via `proof`) reach a gate |
| Visual/capture QA | 12: `arena-shot`, `frame-triage`, `frame-v2v3`, `frame-moved`, `iqa-gate`, `gate-silhouette`, `q1-check`, `ra8-check`, `ra8-manifest`, `rdc-diff`, `pick-gold`, `reference-captures` | None runs in standing CI (no GPU runner); `frame-*` is three tools for one frame diff |
| Web/wasm | 11: `web-build`, `web-run`, `web-doctor`, `web-budget`, `web-perf`, `web-browse`, `web-headers`, `web-input-check`, `wasm-gate`, `wasm-info`, `webgpu-cts` | `web-run` re-wraps `web-build`; only `wasm-gate` runs on main/PR |
| Determinism/proof | 7: `proof`, `tape-checksum`, `golden-matrix`, `desync-diff`, `fk1_determinism`, `hash-util`, `fnv1a64` | `fk1_determinism` is a naming variant of the tape gate, not a new capability |
| Build speed/farm | 11: `gh-check`, `farm`, `gpu-farm`, `affected`, `jobserver`, `compile-cache`, `compile-cache-spike`, `nextest-spike`, `dep-rebuild-count`, `quiet-run`, `cargo-guard` | `compile-cache` prints a plan, `compile-cache-spike` measures it, `dep-rebuild-count` re-measures it |
| Board/plan | 6: `board-slice`, `board-memo`, `plan-atlas`, `finish`, `ticked-rows-audit`, `ladder-counts` | `finish` already wraps two of them |
| Product bars | 3 tools + 3 files: `product-check`, `potato-check`, `part-score`, `product-bar.json`, `pass-bar.json`, `potato-bar.json` | Three bar files for one product verdict |
| Content/fighters | 10: `fighter-count`, `fighter-report`, `level-check`, `beat-lint`, `hit-list`, `cook-char`, `gltf-intake`, `arena-sources`, `arena-report`, `content-cross` | `arena-report` already calls `arena-sources`; `fighter-count` looks like a subset of `fighter-report` |

## Top 5 to build this month

1. **Browser gates on every PR** (#1). Six tools are already written and tested; they run on a branch nobody pushes.
   Add one `browser` job to `ci.yml` (`web-build.mjs`, `web-browse.mjs`, `web-input-check.mjs`): the cheapest large
   coverage win in the repo, and it protects `crates/web`, the only shipped artifact with a URL.
2. **Perf/instruction regression leg** (#2). `gungraun` benches, `bench-compare.mjs` and `perf-baseline.json` are
   finished; only the job is missing. Instruction counts are noise-free on shared runners, so this can gate.
3. **Coverage leg on the sim crates** (#3). One `cargo llvm-cov` job plus a per-crate floor ratchet. It is the only
   way to see that "2,000 tests" actually reach the tick, and `wiring-cov.mjs` already proves the tool works on the PC.
4. **Tool inventory + orphan cleanup** (#4, #5). One `--check` gate over `tools/README.md` (every live tool named,
   every tool has a test) landed 2026-10-23 with the gate in the mirror's quick job. Left: the owner call on the 3
   zero-ref content gates and the `.qw4-check` copy. Small, mechanical, and it stops the slow rot that produced 30
   undocumented tools.
5. **Release and hook plumbing** (#7, #8). One commit: ship `tools/hooks/pre-push`, add `hooks-install.mjs` (so a
   fresh clone cannot skip the ratchet/secret gate) and `tools/release.mjs` (version bump, CHANGELOG from board rows,
   tag, attach ship artifacts). This is what makes `api-break` and `ship-check` mean something.

## Method (read-only, no heavy cargo run)

- Listings: `Get-ChildItem tools` (297 files, 17 subdirs), `tools/README.md`, `docs/book/ref-tools.md`.
- Workflows read: all 14 in `.github/workflows/`; tool invocations extracted by grep over `node tools/*.mjs`
  (22 distinct, 21 executed by a step).
- Reference graph: a Node pass over `.github/`, `docs/`, `skills/`, `tools/`, `.cargo/`, `AGENTS.md`,
  `opencode.jsonc` for every `tools/<name>.mjs`, classified CI / hook / chain / skill / doc-only / index-only / orphan.
- Missing-capability proofs: `llvm-cov|tarpaulin` and `cargo-fuzz|libfuzzer` in workflows = 0 hits; `schedule|cron`
  = one job (`differential.yml` weekly); `macos` = 0; `cargo bench` in workflows = 0; `api-break|license-gate|
  provenance-audit|skills-lint` in workflows = 0.
- Existing: `bench-compare` / `mutants` / `flaky-quarantine` are named only in docs, comments and their own tests.
- Not re-audited: board rows, and the engine features behind them (`docs/SPRINT*.md`, `docs/FOCUS.md`).
