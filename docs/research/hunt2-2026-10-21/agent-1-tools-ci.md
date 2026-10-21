# Hunter 1: the toolchain and CI layer

Scope: the parts of engine2040 that are node scripts and GitHub workflows, not game
crates. `tools/`, `.github/workflows/`, the root `Cargo.toml` profiles, the board
shards and the pre-commit hook.

What I measured in our own layer before I read GitHub (every number below is read
from the tree, not remembered):

| our thing | measured |
|---|---|
| live node tools in `tools/` (`*.mjs`, no test mate) | 115 files, 1,613,355 bytes |
| tool test mates (`tools/*.test.mjs`) | 140 files |
| GitHub workflows | 11 files in `.github/workflows/` |
| board shards (`docs/SPRINT*.md`, archive excluded) | 34 files, 1,492 rows, 770 open, 722 ticked, 1,644,110 bytes |
| CI test groups (`.github/workflows/ci.yml:99-107`) | 4 hand-written `-p` groups: render, game, net, rest |
| compile-cache spike result (`tools/compile-cache-spike.mjs`, PW46 text) | cold 21.6 s vs warm 13.6 s, **0 of 2 cacheable units hit** |
| sccache env (`.github/workflows/ci.yml:22-30`) | `RUSTC_WRAPPER=sccache`, `CARGO_INCREMENTAL=0`, `SCCACHE_CACHE_SIZE=5G` |
| board slice budget (`tools/board-slice.mjs:33`) | `SLICE_BUDGET = 8192` bytes |
| golden hash | `d57b5c8ae3096614` (600 ticks, seed 2040) |

Wave 1 (`docs/research/hunt-2026-10-21/`) was read first; no donor below is in its
donor map. Nothing was run except `gh` and `node`; no cargo, no build, no write
outside this file.

## Ranked donors (1..12)

| # | donor (owner/repo + url) | license | stars | what to steal (one sentence) | our file:line | our row id | measurable accept (a number or an exact command) |
|---|---|---|---|---|---|---|---|
| 1 | `Kobzol/cargo-wizard` https://github.com/Kobzol/cargo-wizard | MIT (`LICENSE`, 1.1K) | 1087 | Three ready-made cargo profile templates in one file (`src/predefined.rs`, 4.5K): fast-compile = `debug = 0` + linker `lld` + (nightly) `codegen-backend = "cranelift"` + `frontend-threads = cores`; fast-runtime = fat LTO + `codegen-units = 1` + `panic = "abort"`; min-size = `debug = false` + `strip` + `opt-level = "z"`. | `Cargo.toml:265` (`[profile.dev]`), `Cargo.toml:267` (`[profile.dev.package."*"]`), `Cargo.toml:242` (`[profile.release]`) | **OPT42** (nightly front-end threads and Cranelift for dev), **OPT39** (`debug = false` for deps), **OPT45** | OPT42's own accept, verbatim: a receipt with **six wall times (3 setups x 2 runs)** and the run spread in `docs/receipts/`; command `node tools/gh-check.mjs cargo build -p engine-game --timings` three ways (stable dev / `-Zthreads=8` / `codegen-backend = "cranelift"`). The donor names the two flags the row already asks for. |
| 2 | `Bachmann1234/diff_cover` https://github.com/Bachmann1234/diff_cover | Apache-2.0 (`LICENSE`, 9.9K, opens "Apache License Version 2.0") | 849 | Coverage restricted to the diff: `diff_cover/git_diff.py` (4.8K) turns `git diff` into changed line ranges per file, `diff_cover/diff_cover_tool.py` (14.6K) intersects them with the coverage JSON, and `diff_cover/snippets.py` (19.5K) prints the uncovered changed lines as a snippet instead of a percentage. | `tools/wiring-cov.mjs:1` ("which functions does the real arena never run?") | **GS2** (wiring coverage, nightly and report only) | `node tools/wiring-cov.mjs --from-json <llvm-cov.json> --diff <base>..<head>` writes `target/reports/wiring-cov-diff.json` listing **only the changed functions with zero hits**; the count is 0 for a seeded change that has a test and > 0 for one that does not. GS2's accept already wants `target/reports/wiring-cov.json`; this adds the diff half. |
| 3 | `azhar9/shardfit` https://github.com/azhar9/shardfit | MIT (`LICENSE`, 1.0K) | 1 | Deterministic longest-processing-time sharding from past runtimes: `internal/split/split.go` (2.6K) sorts by expected duration and puts each item in the least-loaded bucket (ties to the lowest index), `internal/timings/store.go` (5.1K) keeps a ring buffer of the last N durations per test, and `docs/timings-format.md` (2.3K) says a new test is estimated at the **median** of known tests, unknown keys are ignored, and a store with a newer `version` is refused with a clear error rather than silently mis-parsed. | `.github/workflows/ci.yml:99` (the matrix `include:` with the four fixed `-p` groups), `tools/gh-check.mjs:313` (`reuseKey`) | **OPT6** (nextest spike: wall time and flake counts before/after), **PW46** (owns `ci.yml`) | `node tools/test-shard.mjs --plan 4` prints the bucket map; over 5 CI runs the ratio **max bucket wall time / min bucket wall time <= 1.15** (today the four groups are hand-split by crate name, so the ratio is whatever the crates weigh). The shard math needs no nextest install, so it can start while OPT6 stays parked. |
| 4 | `smixs/code-quality` https://github.com/smixs/code-quality | MIT (`LICENSE`, 1.0K) | 19 | The gate that cannot be bought: `scripts/lib/tamper.ts` (27.2K) detects a **deleted test file**, a **deleted test block**, a **strong assertion replaced by a weak one** (a table of `assert.equal` -> `assert.ok` style regex pairs), a **test that only got fewer assertions**, an **inline suppression mark** (`qg:allow`, `gitleaks:allow`) and a **mock** that fakes a pass; `scripts/lib/crap.ts` (22.9K) adds the Change-Risk Anti-Patterns metric (complexity x churn) per changed function. | `tools/workspace-check.mjs:19` (`allowedEdges`, where a new rule row goes), `tools/hooks/pre-commit:1` (the hook that already reads the staged diff) | **HX29** (the referee and the mutant: every referee must fail its mutant), **AF5** (mutation testing names the missing tests) | A seeded diff that deletes a `#[test]` fn or swaps `assert_eq!` for `assert!` in a crate makes `node tools/workspace-check.mjs` exit **1** and name the file and the hunk; the same diff carrying one `qg:allow` marker passes. It is the gate that makes HX29's "every referee must fail its mutant" true for our own gates, not only for the playtest referees. |
| 5 | `rwx-research/captain` https://github.com/rwx-research/captain | MIT (`LICENSE`, 1.1K) | 100 | Flaky handling as four separate parts: per-file timing estimates (`internal/testing/test_file_timing.go`, 0.9K), **targeted** retries that re-run only the failed test instead of the whole file (`internal/targetedretries/*`, 16 framework substitutions), a quarantine list, and a Markdown failure summary under 1 MB (`internal/reporting/markdown.go`, 11.5K). | `tools/flaky-quarantine.mjs:79` (`quarantineList`), `tools/ctrf-report.mjs:352` (`buildCtrf`) | **HX13** (the browser proof under stress: "a red that repeats is not a red"), **AW6** (flake triage) | `node tools/flaky-quarantine.mjs --counts target/flaky-counts.json` prints the **failing test id and its retry count** (today it prints only names); the CTRF report gains a `flaky` field so the CI summary names the flaky tests, and the report stays a pure function of the receipt bytes. |
| 6 | `mozilla/cargo-vet` https://github.com/mozilla/cargo-vet | Apache-2.0 OR MIT (`LICENSE-APACHE` 11.1K + `LICENSE-MIT` 1.0K) | 998 | Supply-chain review as **audits by a trusted entity** with a criteria file: `src/criteria.rs` (8.8K) gives each dependency a tier (full review, suggested, rejected) and `src/format.rs` (57.8K) is the signed criteria format, so "we use this crate" becomes a reviewed statement instead of a `deny.toml` allow line. | `tools/license-gate.mjs:1` (the gate that already prints unlicensed counts and exits 1) | **PW56** (the licence gate and the job runner reach the door; fails closed) | `node tools/license-gate.mjs` exits **1** and names a dependency that has no audit row, next to the existing "unlicensed packages" count; `deny.toml` stays the verdict for Cargo licences, the audit file answers the second question ("is this crate reviewed?"). |
| 7 | `fzakaria/rewindvm` https://github.com/fzakaria/rewindvm | MIT (`LICENSE`, 1.0K) | 84 | The bisector that names **the first step where two runs part**: the README shows "the two are the same run until step 5139", then narrows the range by perturbing only `steps 3629..5140` and shrinks it to one step, and forks the run with a different interleaving from any point. | `tools/desync-diff.mjs:1` ("prints the first diverging byte"), `tools/proof.mjs:417` (`driftVerdict`) | **AIN4** (a warned run leaves a bundle and a bisect), **AW6** (the `-j4` cause named with a repro) | The KVM does not transfer; the **method** does. AIN4's accept says "the bisect names the seeded tick twice": a seeded fault at tick T makes `node tools/desync-diff.mjs` print `first divergence at tick T` and a narrowing step that shrinks `[T0,T1]` to one tick, and a clean run prints nothing. Today `tools/flaky-quarantine.mjs` only counts failures, it cannot say *where* two runs part. |
| 8 | `TraceMachina/nativelink` https://github.com/TraceMachina/nativelink | **FSL-1.1-Apache-2.0** (`LICENSE`, 3.6K: "Functional Source License, Version 1.1, Apache 2.0 Future License" - source-available, not OSI) | 1624 | A **content-addressable** build cache and remote execution server: the same key idea PW46 needs, where the key is the content of the inputs and not the path of the target directory, and "reduced test flakiness" is a stated benefit of a shared cache. | `tools/compile-cache-spike.mjs:44` (`SCCACHE_BASEDIRS`), `tools/compile-cache-spike.mjs:48` (the `rebuild` cell that measures the CI shape) | **PW46** (a compile cache across the lane pools; closes OPT9) | PW46's own accept, verbatim: the lane-pool case reports **at least 30% faster with a hit rate above zero** - today the spike prints **0 of 2 cacheable units hit** because the per-lane target path is part of the sccache key. Note the licence: FSL-1.1 is source-available, so read it before lifting a byte; AGENTS rule 4 allows porting any donor, so the idea is safe. |
| 9 | `Kobzol/cargo-pgo` https://github.com/Kobzol/cargo-pgo | MIT (`LICENSE`, 1.1K) | 793 | Profile-guided optimisation as a repeatable recipe: `src/pgo/instrument.rs` (4.3K) builds an instrumented binary, `src/pgo/optimize.rs` (7.9K) trains it on a workload and re-links with the profile, and `ci/pgo.yml` (1.2K) is the CI leg that keeps the profile as an artifact. | `Cargo.toml:320` (`[profile.ship]`, the fat-LTO one-unit profile CI builds) | **OPT45** (thin LTO local, fat LTO shipped) | The shipped profile gets a PGO leg whose accept is the golden itself: the 600-tick proof under PGO still prints `final_hash=d57b5c8ae3096614`, and the 300-frame arena run stays at or above the floor in `tools/product-bar.json` (that bar is never lowered). If PGO moves the hash, the leg is dropped - that is the point of measuring it. |
| 10 | `chainloop-dev/chainloop` https://github.com/chainloop-dev/chainloop | Apache-2.0 (`LICENSE.md`, 11.1K) | 603 | Signed records of what the CI and the agents did, checked against policies as code: `docs/evidence-transformation.md` (13.5K) is the contract that turns a raw run into a signed record, and `docs/examples/policies/chainloop-qa.yaml` (2.0K) plus `docs/examples/contracts/empty/generic.yaml` show the policy shape and the empty-case contract. | `tools/proof.mjs:398` ("AD4 green record: a gate that ends GREEN writes target/proof/<gate>-latest.json ({ gate, sha, time })"), `tools/public-export.mjs:1` (the snapshot export) | **HX3** (the AI bill of materials: every door edit records who made it) | The green record grows from `{gate, sha, time}` to `{gate, sha, time, agent, inputs_digest, signature}`: a policy check fails the export when a record names an agent id that is not on the allow list, and the receipt still says which gate and which sha. The record stays a pure function of the run (no clock, no locale), as `tools/proof.mjs` already promises. |
| 11 | `taiki-e/cargo-hack` https://github.com/taiki-e/cargo-hack | Apache-2.0 OR MIT (`LICENSE-APACHE` 9.9K + `LICENSE-MIT` 1.0K) | 875 | The **feature powerset**: `src/features.rs` (18.2K) enumerates the feature combinations of a crate and `src/main.rs` (27.6K) runs the build or the tests on each one, so a crate that only compiles with one feature set is found by machine, not by luck. | `tools/feature-unify.mjs:1` ("OPT36 feature-unify (cargo-hakari pattern)"), `tools/workspace-check.mjs:19` (the `allowedEdges` map a new feature edge must be added to) | **PW45** (feature unification, the hack crate gets a dependent; closes OPT8) | PW45's accept, verbatim: the number of names with more than one version in `Cargo.lock` is **at most 67** (today 80) and at most 67 extra copies (today 101). The powerset leg is the new part: it names the **combination** that breaks, so the union in `crates/workspace-hack/Cargo.toml` is cut by evidence instead of by one crate at a time. |
| 12 | `cargo-bins/cargo-quickinstall` https://github.com/cargo-bins/cargo-quickinstall | Apache-2.0 OR MIT (`LICENSE-APACHE` 11,357 B + `LICENSE-MIT` 1,078 B) | 315 | Pre-built binaries for `cargo install`, so a helper tool is a download instead of a compile on a machine whose CPU is shared by every AI. | `tools/doctor.mjs:1` ("toolchain, deny, cache" scopes; installs need explicit `--consent`) | **AF5** (`.tools/bin/cargo-mutants`, installed 2026-10-21), **AF6** (`.tools/bin/cargo-shear`) | `node tools/doctor.mjs` reports the version and the sha256 of every `.tools/bin/*.exe` and offers the pre-built download; the doctor run that installs `cargo-shear` takes **under 30 s** instead of a cargo build on the shared pool. Measured today: `.tools/bin/` holds only `cargo-machete.exe` and `cargo-mutants.exe`. |

## Wild ideas (10)

Not repos yet, but each has a number that would prove it.

1. **One target dir for the whole PC pool, with the sccache key normalized.** The
   pool hands each lane a fresh `CARGO_TARGET_DIR`, so the target path is part of
   the key and 0 of 2 units hit. Measure: the hit rate printed by
   `tools/compile-cache-spike.mjs` goes above zero and the lane wall time is at
   least 30 % under cold (PW46's own bar).
2. **Shard the 140 tool test files with Node's own `--test-shard`.**
   `node --test --test-shard=1/4 "tools/**/*.test.mjs"` needs no dependency.
   Measure: the four shards' max/min wall ratio <= 1.15 and each file appears in
   exactly one shard (count the file names in the four logs).
3. **A shard plan built from nextest's own JUnit receipt.** The receipt already
   carries a `time` per test; the bucket map can be derived from it instead of a
   new store. Measure: the map changes when the receipt changes and is byte-equal
   for the same receipt (the rule `tools/ctrf-report.mjs` already follows).
4. **A "the gate was bought" diff check.** A commit that adds a `pub fn` to a
   crate must touch a `#[test]` in the same crate or say why in the message.
   Measure: over the last 50 commits, the share that add a public item with no
   test in the same diff (today that number is unknown).
5. **A golden receipt that names the machine, not just the sha.**
   `tools/proof.mjs:398` writes `{gate, sha, time}`; add the rustc version, the
   target triple, the worker count and the CPU model. Measure: three records from
   three machines produce a receipt diff that names exactly the field that
   differs, and the same machine twice produces byte-equal records.
6. **A `--only-changed` golden.** The golden only needs re-running when the sim
   crates or the fixture change; hash the closure the way `tools/gh-check.mjs:313`
   already hashes its reuse key. Measure: the share of commits that skip the
   golden over 20 commits (today every commit pays for it).
7. **CI minutes as a printed number.** Each workflow leg prints its runner
   minutes into the run summary, so the 40-seat lane knows what is expensive.
   Measure: the minutes per leg in the summary of 10 runs; the two heaviest legs
   are named.
8. **An owner-crate meter for the 770 open rows.** `tools/board-slice.mjs --ready`
   prints rows; it could also print them grouped by owner crate with a median
   age, so the lead sees the shape of the backlog, not the list. Measure: the
   number of distinct owner crates among the 770 open rows, and the median age
   of the oldest ten, printed in one line.
9. **A local sparse registry mirror.** One vendored index so a lane build never
   waits on crates.io. Measure: the fetch seconds of a cold lane build with the
   network blocked (target: 0 fetch seconds, versus today's fetch time).
10. **A one-command red-to-red path.** A red CI leg leaves the failing test id,
    the seed and the input tape, so the same leg re-runs on the PC in one
    command. Measure: the minutes from "CI is red" to "the red is reproduced
    locally", measured over 5 real failures.

## Rejected (10)

Looked at, measured, dropped.

1. `max-sixty/worktrunk` (9360 stars, licence NOASSERTION) - the best tool for
   "one worktree per agent", but AGENTS.md rule 5 says **no worktrees**, and one
   branch `master` on one tree. The owner rule outranks the star count. Licence is
   also NOASSERTION, so there is nothing clean to lift.
2. `jj-vcs/jj` (32186 stars, Apache-2.0) - would replace git. AGENTS.md rule 5:
   one branch, one tree, never rebase, never amend. Not a tool we can add.
3. `facebook/sapling` (7182 stars, GPL-2.0) - same conflict as jj, and GPL-2.0.
   Its IntelliMerge idea is worth a paragraph in a design note, not a dependency.
4. `automazeio/ccpm` (8414 stars, MIT) - a kanban for agents built on **GitHub
   Issues and worktrees**. The repo is standalone (AGENTS.md: "no other repo"),
   and the board is a set of markdown shards, not issues. Nothing to port.
5. `Mic92/hestia` (122 stars, **no LICENSE file** - GitHub reports licence
   `none`) - Nix-only, and there is nothing to lift without a licence.
6. `firebuild/firebuild` (150 stars, licence NOASSERTION, C++ daemon) - a build
   cache is exactly our PW46 pain, but a C++ daemon on a Windows box is a new
   service to keep alive, and the licence is NOASSERTION. Idea only.
7. `xd009642/tarpaulin` (3020 stars, Apache-2.0) - a second Rust coverage tool
   when GS2 already names `taiki-e/cargo-llvm-cov`. One coverage tool is enough.
8. `orf/cargo-bloat-action` (114 stars, MIT) - GitHub reports `archived: true` and
   the last push is 2022-12-27. An archived 2022 action cannot be a donor.
9. `box/flaky` (398 stars, Apache-2.0) - a pytest plugin; our stack is
   cargo-nextest plus node. The idea is already in rank 5 (`captain`) in a form
   we can use.
10. `jonico/awesome-runners` (901 stars, Apache-2.0) - a comparison list of
    self-hosted runners, not code. `tools/gh-check.mjs` already spends GitHub's
    own free public runners, which is the cheaper answer.

## Donors seen but skipped

Already mined by wave 1 or already named in our own rows, so not re-mined here.

- `cranelift/sccache` and `mozilla-actions/sccache-action` (wave 1; already used
  at `.github/workflows/ci.yml:120`).
- `Swatinem/rust-cache` (wave 1; already used at `.github/workflows/ci.yml:126`).
- `LukeMathWalker/cargo-chef` (2724 stars, wave 1; named by the PW46 row already).
- `ctrf-io/github-test-reporter` (378 stars, MIT, wave 1; HX12 landed, the tool is
  `tools/ctrf-report.mjs`).
- `rust-secure-code/cargo-repro` (58) and `omnibor/omnibor-rs` (30), wave 1; HX10
  landed.
- `gastownhall/beads` (28056, MIT), wave 1; the differential ship check.
- `SummerEngine/summer` (85, MIT), wave 1; the referee and the mutant (HX29) -
  rank 4 above extends that idea to our own gates rather than repeating it.
- `mizchi/chaosbringer` (48, MIT), wave 1; the chaos-in-a-browser pattern (HX13).
- `gpuweb/cts` (180, BSD-3), wave 1; the WebGPU conformance suite (HX4).
- `gotestyourself/gotestsum` - already credited in our own
  `tools/flaky-quarantine.mjs:1` as a pattern donor.
- `EmbarkStudios/cargo-deny` - already the verdict behind `tools/license-gate.mjs:11`.
- `dtolnay/cargo-llvm-lines` (529), `bheisler/criterion.rs`, `mitsuhiko/insta` -
  wave 1, no toolchain gap left.
- `nextest-rs/quick-junit` (13, Apache-2.0 OR MIT) - skipped as a donor because
  HX12 already landed and our `tools/ctrf-report.mjs:79` parses JUnit itself; its
  strict-parser corpus (7 malformed fixtures under
  `crates/quick-junit-tests/tests/integration/`) is a **wild idea** worth folding
  into `tools/ctrf-report.test.mjs`.
