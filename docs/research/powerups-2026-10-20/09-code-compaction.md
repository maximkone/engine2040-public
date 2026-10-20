# 09 - code compaction: make the engine smaller, make the agent's working set smaller

Two jobs, both measured on our own tree at HEAD `8fc85da1` (2026-10-20). Job A is
the repo: 42 library crates, 771,684 lines of Rust under `crates/*/src`, 1,032
locked packages, 80 crate names carrying more than one version, 91 profile
artifacts committed to git, and a `target/debug` of 29.5 GB on this PC. Job B is
the agent's context: `llms.txt` + `AGENTS.md` + `docs/book/for-ai.md` +
`docs/CRATE-MAP.md` + `docs/plan/README.md` + `.opencode/repomap.md` cost about
14,700 tokens today, and `.opencode/modmap.md` adds 36,154 more if it is read
whole. The answer is not one new tool: most of the compaction tools we would
import are already present in this repo as node scripts, and the win is in
wiring them, deleting what they name, and giving an agent one crate's worth of
map instead of the whole tree.

## Measured: our own numbers (OBSERVED 2026-10-20 at HEAD `8fc85da1`)

Command for the crate table: `git archive HEAD crates | tar -x -C <tmp>`, then
`Get-ChildItem crates -Directory | ... (Get-Content <file> | Measure-Object -Line).Lines`
over `crates/<c>/src/**.rs`.

**Crate count and lines.** `members = ["crates/*"]` in the root `Cargo.toml`
plus the root package `engine2040` = **43 workspace members, 42 library
crates**. `crates/*/src` = **1,153 .rs files, 771,684 lines**. With
`crates/*/tests` (205 files) the whole Rust tree is **1,358 files, 814,011
lines**; tests are 42,327 lines = 5.2%.

| crate | src files | src lines | % of src |
|---|---|---|---|
| render | 139 | 107,807 | 14.0% |
| endpoint | 118 | 74,503 | 9.7% |
| game | 82 | 73,997 | 9.6% |
| net | 66 | 49,483 | 6.4% |
| assets | 58 | 39,065 | 5.1% |
| playtest | 57 | 37,798 | 4.9% |
| editor | 40 | 36,008 | 4.7% |
| anim | 61 | 35,332 | 4.6% |
| spatial | 44 | 34,773 | 4.5% |
| ai | 37 | 24,503 | 3.2% |
| physics | 26 | 24,300 | 3.1% |
| combat | 27 | 22,515 | 2.9% |
| provider | 31 | 20,623 | 2.7% |
| arena | 24 | 19,053 | 2.5% |
| audio | 25 | 18,770 | 2.4% |
| proof | 30 | 16,062 | 2.1% |
| destruction | 25 | 15,381 | 2.0% |
| perf | 18 | 11,515 | 1.5% |
| ecs | 16 | 10,957 | 1.4% |
| ui | 9 | 10,105 | 1.3% |
| material | 17 | 9,341 | 1.2% |
| nav | 14 | 8,795 | 1.1% |
| web | 20 | 8,606 | 1.1% |
| distill | 12 | 5,901 | 0.8% |
| core | 24 | 5,795 | 0.8% |
| physfx | 25 | 5,643 | 0.7% |
| scene | 10 | 5,611 | 0.7% |
| observe | 8 | 5,490 | 0.7% |
| geo | 13 | 5,161 | 0.7% |
| kam2-game | 14 | 5,041 | 0.7% |
| rules | 11 | 4,646 | 0.6% |
| world | 9 | 4,288 | 0.6% |
| app | 10 | 3,325 | 0.4% |
| input | 7 | 2,547 | 0.3% |
| kam2-sim | 6 | 2,375 | 0.3% |
| ossim | 6 | 2,263 | 0.3% |
| nn | 3 | 2,009 | 0.3% |
| save | 4 | 1,556 | 0.2% |
| kam2-web | 4 | 537 | 0.1% |
| clock | 1 | 117 | 0.0% |
| engine-chaos | 1 | 85 | 0.0% |
| workspace-hack | 1 | 2 | 0.0% |

Top 3 crates = 256,307 lines = **33.2%** of the tree. `engine-render` is also
the compile critical path (294 s of a 347 s dev build, `docs/SMOOTH-BUILD.md`).

**The largest 20 files (HEAD `8fc85da1`).** Sum = **70,835 lines = 9.2% of src**.

| # | file | lines |
|---|---|---|
| 1 | crates/game/src/fighter_look.rs | 6,038 |
| 2 | crates/arena/src/lib.rs | 4,940 |
| 3 | crates/game/src/bin/arena.rs | 4,599 |
| 4 | crates/render/src/ibl.rs | 4,245 |
| 5 | crates/combat/src/fight.rs | 3,809 |
| 6 | crates/game/src/lib.rs | 3,707 |
| 7 | crates/render/src/gtao.rs | 3,336 |
| 8 | crates/game/src/camera.rs | 3,311 |
| 9 | crates/game/src/windowed.rs | 3,304 |
| 10 | crates/endpoint/src/scene_tools.rs | 3,303 |
| 11 | crates/editor/src/commands.rs | 3,281 |
| 12 | crates/physics/src/ragdoll.rs | 3,247 |
| 13 | crates/render/src/present/gpu_present/presenter.rs | 3,244 |
| 14 | crates/spatial/src/relation.rs | 3,103 |
| 15 | crates/render/src/present/gpu_present/submit.rs | 3,102 |
| 16 | crates/render/src/present/gpu_present/live.rs | 2,959 |
| 17 | crates/ai/src/depth.rs | 2,957 |
| 18 | crates/endpoint/src/build_level.rs | 2,812 |
| 19 | crates/endpoint/src/discover.rs | 2,791 |
| 20 | crates/ui/src/lib.rs | 2,747 |

**Duplicate dependencies.** `Cargo.lock` at HEAD: **1,032 packages, 80 names
carry more than one version, 101 extra version copies.** Worst offenders by
copy count: `hashbrown` 5 versions (0.13/0.14/0.15/0.16/0.17), `glam` 4
(0.30/0.31/0.32/0.33), `windows-sys` 4, `rand` 3, `syn` 3 (1.0.109/2.0.119/3.0.6),
`wgpu` 2 (29.0.4/30.0.1), `bitflags` 2, `hashbrown`/`itertools`/`naga`/`zip`
2 each. `serde_json` = 1 version, `num-traits` = 1 version.

CORRECTION to the board (OPT8 progress note, `docs/SPRINT-13-engine-edge.md:34`:
"16 syn, 12 serde_json, 14 num_traits copies in target/debug/deps before the
hack"). Those are build-artifact counts from 2026-10-17, not dependency counts.
Today `target/debug/deps` holds 16 `syn-*`, 13 `serde_json-*`, 9 `num_traits-*`
artifacts (OBSERVED), and the lock holds **one** `serde_json` and **one**
`num-traits`. `num-traits` is not a direct dependency of any crate; it arrives
transitively. The unification target is the 80 multi-version names, not
serde_json/num-traits.

Direct dependency declarations across the 42 crates: **81 distinct names, 162
declarations; 29 names are declared by more than one crate** (13 internal
`engine-*` crates, 16 external). 12 of them carry **divergent feature sets**,
which is what breaks unification: `image` (3 variants: none / `png` /
`hdr+jpeg+png`), `engine-assets` (3), `engine-render` (3), `engine-core`,
`engine-arena`, `engine-anim`, `engine-scene`, `engine-game`, `wgpu`, `uuid`,
`tokio`, `web-sys` (2 each). `crates/workspace-hack` exists (2 lines of
`lib.rs`, 4 deps) and **no crate depends on it** - it unifies nothing
(`docs/SMOOTH-BUILD.md`, OPT36).

**Dead feature flags.** Our own scan (declared feature, no other crate requests
it, no `cfg(feature = ...)` in its own crate): **7 dead** - `endpoint/tls`,
`geo/default`, `geo/kit`, `geo/all`, `kam2-game/window`, `playtest/tls`,
`scene/math`. **7 weak** (no external request, 1 cfg use): `core/test-util`,
`geo/bake`, `geo/fbx`, `proof/arena`, `proof/spine`, `proof/ragdoll`,
`rules/arena`. 54 features are declared across 22 crates in total.

**Unwired modules.** `docs/plan/forge-adoption.md` A.3 #5 named
`assets/import_repair`, `observe/rerun_sink`, `RenderHeal`, `anim::lod`,
`ai::behavior`, `ai::utility`, `nav::crowd`. Re-checked today by `git grep`:

| module | lines | verdict today (OBSERVED) |
|---|---|---|
| crates/physics/src/kcc.rs | 925 | **0 references anywhere** - dead |
| crates/ai/src/behavior.rs | 260 | **0 references anywhere** - dead |
| crates/render/src/heal.rs (`RenderHeal`) | 1,014 | type only in its own file + doc links in `present/surface.rs`; constructed only in its own tests - no production constructor |
| crates/audio/src/heal.rs (`DeviceHeal`) | 686 | constructed only in `crates/game/src/audio_mix.rs:413` inside `#[cfg(test)] mod tests` - no production constructor |
| crates/observe/src/rerun_sink.rs | 297 | own file + doc links in `observe/src/timeline.rs` + own test - no outside caller |
| crates/ai/src/utility.rs | 327 | live intra-crate (`ai/src/gladiator.rs`); no outside caller |
| crates/anim/src/lod.rs | 183 | live intra-crate (`anim/src/crowd_scale.rs`); no outside caller |
| crates/assets/src/import_repair.rs | 1,260 | **NOW WIRED** - `crates/render/src/v1.rs:256` and `crates/render/src/present/gpu_present/presenter.rs:2197` call `engine_assets::import_repair::recompute_tangents`. A.3 #5 is stale on this one |
| crates/audio/src/crowd.rs | - | live intra-crate (`audio/src/coherence.rs`, `combat_cues.rs`, `director.rs`, `mix.rs`). There is **no `crates/nav/src/crowd.rs`**; A.3 #5's "nav::crowd (ORCA)" is the ORCA bench at `crates/ai/tests/crowd_orca_bench.rs`. A.3 #5 is stale on the path |

Dead (0 references) = 1,185 lines. Test-only = 1,997 lines. **3,182 lines that
no production path reaches** (0.41% of src).

**`engine-playtest` has a consumer now.** `crates/endpoint/Cargo.toml:16,75`
lists it as optional and `crates/endpoint/src/packs/verify.rs` links it behind
feature `verify` (which needs `mcp`), with verbs `replay.verify`,
`replay.bisect`, `bug.replay`, `trace.summary`, `verify.proof`. A.3 #3's "the
playtest crate has no consumer" is stale. One stale comment survives:
`crates/endpoint/src/assistant/fork.rs:19` still says "`engine-playtest` stays
outside the endpoint dep edge".

**Stale docs.** `docs/` = 1,196 files, 54.3 MB. `docs/archive/` alone = 851
files, 41.8 MB = **77% of docs bytes** (303 .md, 32,569 lines, 3.49 MB).
`docs/plan/` = 70 files, 11,648 lines, 1.17 MB. `docs/research/` = 109 files,
8,560 lines, 1.66 MB. `docs/plan/kam2-ui.md` is **not** in the
`docs/plan/README.md` index although the index says `tools/plan-index.test.mjs`
checks every plan file.

**Disk and git.** `target/debug` = 84,858 files, **29.5 GB**; `target/release`
3.9 GB; `target/wasm32-unknown-unknown` 731 MB (E86 is still open).
**91 `.profraw` files, 55.5 MB, are tracked in git** (`git ls-files "*.profraw"`
= 91) - profile artifacts live in the repository.

**The agent's working set today (Job B, OBSERVED).** Token estimates are
bytes/4.

| file | lines | bytes | approx tokens |
|---|---|---|---|
| llms.txt | 41 | 3,422 | 856 |
| AGENTS.md | 34 | 5,679 | 1,420 |
| docs/book/for-ai.md | 45 | 6,047 | 1,512 |
| docs/CRATE-MAP.md | 48 | 8,018 | 2,004 |
| .opencode/repomap.md | 483 | 15,866 | 3,966 |
| docs/plan/README.md | 177 | 19,759 | 4,940 |
| **entry set total** | | | **~14,700** |
| .opencode/modmap.md (whole) | 1,440 | 144,615 | 36,154 |

One crate's modmap slice (`.opencode/modmap.md`, grep it, never read it whole):
render 4,068 tok (11.3% of the map), endpoint 3,810, game 2,652, assets 2,289,
net 2,136, anim 1,561, playtest 1,540, editor 1,191, ai 973, physics 947,
core 797, clock 93. Top 3 crates = 10,529 tok = 29.1% of the map. So an agent
working on one crate needs **300-4,100 tokens of map**, not 36,154.

## Tool findings

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Per-crate compile cost: counts LLVM IR lines per generic instantiation, `-p <crate>` per crate. The only donor that answers "which crate costs the build" | cargo-llvm-lines 0.4.48, https://github.com/dtolnay/cargo-llvm-lines (crates.io version page + `master/LICENSE-MIT`, read live 2026-10-20) | MIT OR Apache-2.0 (crates.io 0.4.48; `master/LICENSE-MIT` read live) | OPT11 (crate fan-out cut) + E64 (the five cuts) | the `-p` loop and the per-crate attribution, not the code: `cargo llvm-lines -p engine-render` before and after the render split; write the numbers into `docs/SMOOTH-BUILD.md` beside the 294 s critical path | S | after the split, `engine-render`'s share does not drop 30% -> close OPT11 with the numbers | PORT |
| 2 | Unused-dependency finder. Fast, no nightly, workspace-aware (`[workspace.metadata.cargo-machete]`, `--all-features` metadata) | cargo-machete 0.9.2, https://github.com/bnjbvr/cargo-machete (crates.io version page + `main/LICENSE.md`, read live 2026-10-20) | MIT (crates.io 0.9.2; `main/LICENSE.md` read live) | no row, propose a new row **OPT37 unused-dependency gate** (feeds OPT8's dependency rebuild count) | the scan and its ignore list, not the code; run once per crate, publish the list of deps to delete | S | 0 unused deps found across 42 crates -> close OPT37 with the scan | PORT |
| 3 | Finds enabled-but-unused feature flags and prunes them. Exactly our 7 dead + 7 weak features | cargo-unused-features 0.2.0, https://github.com/TimonPost/cargo-unused-features (crates.io version page + `main/LICENSE`, read live 2026-10-20) | MIT (crates.io 0.2.0; `main/LICENSE` read live) | no row, propose a new row **OPT38 dead-feature gate** (feeds E64) | `unused-features analyze` output as the delete list for the 7 dead flags (`endpoint/tls`, `geo/default`, `geo/kit`, `geo/all`, `kam2-game/window`, `playtest/tls`, `scene/math`) | M | it needs a nightly cargo the pinned toolchain (`rust-toolchain.toml` = 1.97.1) does not have, and a second toolchain is refused -> keep our own node scan and close the row | PORT |
| 4 | Feature unification: generates the union of external features into one workspace-hack crate so every crate compiles one copy | cargo-hakari 0.9.39, https://crates.io/crates/cargo-hakari (version page, read live 2026-10-20; the repo now lives inside https://github.com/guppy-rs/guppy) | MIT OR Apache-2.0 (crates.io 0.9.39 license field, read live) | OPT8 (feature unification spike, the row already names cargo-hakari) | `cargo hakari generate` output into `crates/workspace-hack/Cargo.toml`, and one real dependent crate so the 2-line hack crate stops unifying nothing | M | the union crate still has no dependent after the change -> close OPT8 with the measured rebuild count | PORT |
| 5 | Per-crate binary size: `--crates` attributes `.text` bytes to each crate. The web build and the Windows exe are both size-bound | cargo-bloat 0.12.1, https://github.com/RazrFalcon/cargo-bloat (crates.io version page + `master/LICENSE`, read live 2026-10-20) | MIT (crates.io 0.12.1; `master/LICENSE` read live) | no row, propose a new row **OPT39 binary-size budget** (wasm download + exe) | the `--crates` attribution only; run `cargo bloat --release --crates -n 20` on the arena binary and on the wasm build | M | no crate is over 10% of `.text` -> close OPT39 with the table | PORT |
| 6 | Test runner with per-test processes. Already in our CI (`.github/workflows/ci.yml:125-130`), absent on the build PC, which is why OPT6 is parked ("cargo-nextest absent exit 101", DO NOT DISPATCH 2026-10-05) | cargo-nextest 0.9.148, https://github.com/nextest-rs/nextest (crates.io version page + `main/LICENSE-MIT`, read live 2026-10-20) | Apache-2.0 OR MIT (crates.io 0.9.148; `main/LICENSE-MIT` read live) | OPT6 (nextest spike) | nothing to port: install the binary on the build PC and post the version the row asks for, then run the same proof legs under it | S | wall time does not improve with the same failures visible -> keep `cargo test` | PORT |
| 7 | Finds broken intra-doc links after `cargo doc`. We check doc *presence* (`tools/docs-coverage.mjs`) but never doc *links* | cargo-deadlinks 0.8.1, https://github.com/deadlinks/cargo-deadlinks (crates.io version page + `master/LICENSE-MIT`, read live 2026-10-20) | MIT OR Apache-2.0 (crates.io 0.8.1; `master/LICENSE-MIT` read live) | no row, propose a new row **OPT40 doc-link gate** (sits beside API2 / `tools/docs-coverage.mjs`) | the link check only; run it after `cargo doc --workspace --no-deps` in CI | S | 0 broken links -> close OPT40 | PORT |
| 8 | Generates a per-crate license attribution sheet with thresholds. `deny.toml` only checks the `window` feature graph, so a crate outside that graph has no licence list | cargo-about 0.9.2, https://github.com/EmbarkStudios/cargo-about (crates.io version page + `main/LICENSE-MIT`, read live 2026-10-20) | MIT OR Apache-2.0 (crates.io 0.9.2; `main/LICENSE-MIT` read live) | no row, propose a new row **OPT41 per-crate licence sheet** (sits beside the E07 licence decision) | `cargo about generate` with a template, one sheet per crate, next to `docs/DEPENDENCIES.md` | S | the sheet says nothing `deny.toml` does not already say -> close OPT41 | PORT |
| 9 | Packs a repository into one file for an LLM. The packet shape an agent can read whole | repomix, https://github.com/yamadashy/repomix (GitHub API repository object, `license.spdx_id = MIT`, read live 2026-10-20) | MIT (GitHub API license field, read live) | no row, propose a new row **PKT1 agent packet context builder** (feeds LI100 in `docs/plan/landscape-ideas.md`) | the packer shape only: `repomix --style xml --include "crates/<c>/src/**"` per crate, one packet per crate, output under 8k tokens | S | one crate's packet exceeds 8k tokens -> the modmap slice wins and the row closes | PORT |
| 10 | Repository map by symbol graph + PageRank under a token cap. Our `tools/modmap.mjs` builds the map but has no ranking and no cap | aider `repomap.py`, https://github.com/Aider-AI/aider (GitHub API `license.spdx_id = Apache-2.0`, read live 2026-10-20) and the docs page https://aider.chat/docs/repomap.html (HTTP 200, read live 2026-10-20); tree-sitter https://github.com/tree-sitter/tree-sitter and grammar https://github.com/tree-sitter/tree-sitter-rust (`master/LICENSE`, read live 2026-10-20) | aider Apache-2.0; tree-sitter MIT; tree-sitter-rust MIT (all read live) | LI100 (`docs/plan/landscape-ideas.md:106`, which already names Aider's `repomap.py`) | the ranking, not the bytes: symbol graph -> pagerank -> token cap inside `tools/modmap.mjs`; the Rust grammar is tree-sitter-rust, so a Rust workspace needs no language plugins | M | ranking does not change which 20 files an agent opens -> close LI100 as HAVE | PORT |
| 11 | Semver check against a baseline. We already ship the same verdict shape from rustdoc JSON | cargo-semver-checks 0.51.0, https://github.com/obi1kenobi/cargo-semver-checks (crates.io version page, read live 2026-10-20; our own `tools/api-break.mjs:1-14` already names 0.51.0 and its licence) | Apache-2.0 OR MIT (crates.io 0.51.0; rustc 1.93 minimum, our pin is 1.97.1) | OPT23 (API-break check) | nothing to port: `tools/api-break.mjs` is the same gate with a committed baseline | S | `git tag` holds 5 tags (`archive/*`, `limbloss-r1`, `rules-clippy-r1`) and none is a semver release of an engine crate, so "the last tagged release" does not exist -> keep `api-break.mjs` and close OPT23 with its numbers | REJECT |
| 12 | Finds dependencies that are never used, including macro-only uses that a text scan misses. Needs a nightly cargo | cargo-udeps 0.1.61, https://github.com/est31/cargo-udeps (crates.io version page + `master/LICENSE` "MIT or Apache License 2.0", read live 2026-10-20) | MIT OR Apache-2.0 (crates.io 0.1.61; `master/LICENSE` read live) | OPT37 (with row 2) | only the second pass: run it after cargo-machete on the crates that still look suspicious | M | the nightly toolchain is refused (same kill as row 3) -> close OPT37 on cargo-machete alone | RESEARCH-ONLY |

### Job B: what an agent must read for one crate, and how to cut the rest

**What to read, in order (measured cost).** `llms.txt` (856 tok) names
`docs/book/for-ai.md` (1,512) and the two proof commands. Then one crate's
modmap slice (300-4,100 tok), `docs/CRATE-MAP.md`'s one row for that crate
(2,004 for the whole table), the `docs/plan/README.md` line for that plan
(4,940 for the whole index), and the board row itself via
`node tools/board-slice.mjs <ID>` (a few hundred tokens, not the 84-line
`docs/SPRINT.md`). Total for a normal crate task: **~6,000-9,000 tokens**.
Everything else in `docs/` (616 .md files, 76,501 lines, 8.95 MB) is history,
not instructions.

**How to cut the rest.** Four cuts, all ours, no donor needed:
`docs/archive/` (851 files, 41.8 MB, 77% of docs bytes) leaves the default
read path; `docs/research/` (109 files, 1.66 MB) is reference, read by
`docs/research/LADDER.md` and the steals files, never at session start;
`docs/plan/` keeps only the NOW-tagged files in the index line and moves LATER
files to one archive; `.opencode/modmap.md` stays grep-only (36,154 tok if read
whole) and the per-crate slice is what an agent loads.

**Papers and tools read live for Job B** (all read 2026-10-20, arXiv abstract
or paper page HTTP 200 unless stated): "Lost in the Middle: How Language Models
Use Long Contexts" https://arxiv.org/abs/2307.03172 (position in the context
matters, so a packed repo must be ordered, not concatenated); LLMLingua
https://arxiv.org/abs/2310.05736 and LongLLMLingua
https://arxiv.org/abs/2310.06839 (prompt compression by token dropping, a
query-compression tool not a repo packer); RepoCoder
https://arxiv.org/abs/2303.12570 and Repoformer
https://arxiv.org/abs/2403.10059 (retrieval over a repo, iterative and
selective); Agentless https://arxiv.org/abs/2407.01489 (localize, then act:
a small retrieved slice beats a whole-repo prompt); RepoBench
https://arxiv.org/abs/2306.03091 and CodeRAG
https://arxiv.org/abs/2509.16112 (the benchmark and the RAG shape for
repo-level completion); SWE-smith https://arxiv.org/abs/2504.21798 (task
data, not context); MemGPT https://arxiv.org/abs/2310.08560 and "Agentic
Context Engineering" https://arxiv.org/abs/2510.04618 (context as an evolving
managed store, the shape our `docs/plan/README.md` + modmap already has);
"Context Rot" https://research.trychroma.com/context-rot (HTTP 200, read live
2026-10-20 - long inputs degrade even when they are relevant, which is the
argument for the small packet). One ID from memory failed live: arXiv
2305.14034 is **not** "Selective Context", it is a graphene-harmonics paper
(read live 2026-10-20), so that donor is not cited here.

## Compaction plan (5 steps, each with its number)

1. **Pin the baseline.** One node script (`node tools/crate-stats.mjs`) writes
   crate count, lines per crate, top-20 files, lock duplicate names and the
   feature-dead list into `docs/compaction-baseline.md`, and
   `tools/workspace-check.mjs` fails when the numbers drift. Start value:
   42 crates, 771,684 src lines, 1,153 src files, 1,032 lock packages, 80
   multi-version names, 101 extra copies, 7 dead features, 5,317 unwired pub
   items (`tools/wiring-scan.baseline.json`), 91 tracked `.profraw` (55.5 MB).
   Done when the file exists and the gate runs.
2. **Delete what no path reaches.** Cut the 1,185 dead lines
   (`crates/physics/src/kcc.rs`, `crates/ai/src/behavior.rs`) and decide the
   1,997 test-only lines (`render::heal`, `audio::heal`,
   `observe::rerun_sink`: wire or delete). Drop the 7 dead feature flags.
   Untrack the 91 `.profraw` files (55.5 MB in git) and cap `target/debug`
   (29.5 GB) with `node tools/clean.mjs --dry-run` then `--auto
   --include-debug` (E86). Accept: golden `d57b5c8ae3096614` unchanged x2 and
   git history smaller by 55.5 MB.
3. **Unify the dependency graph.** Give `crates/workspace-hack` a real
   dependent and run `cargo hakari generate` (OPT8). Then attack the 80
   multi-version names by copy count: `hashbrown` 5, `glam` 4, `windows-sys`
   4, `rand` 3, `syn` 3 = 19 of the 101 extra copies from 5 names. Use
   cargo-machete and cargo-unused-features to delete the dead ends first.
   Accept: OPT8's own bar - dependency rebuild count across five lane builds
   drops by a third.
4. **Split the heavy crates and prove it with numbers.** `engine-render`
   (107,807 lines, 155 files, 294 s of the 347 s dev build) is the lever;
   `engine-endpoint` (74,503) and `engine-game` (73,997) are next. Run
   `cargo llvm-lines -p engine-render` before and after (OPT11) and keep the
   split only if the touch-rebuild of that crate drops >= 30% (OPT11's own
   Accept). The E64 cuts are the cheap half of this step.
5. **Give the agent one crate, not the tree.** Ship a per-crate packet
   generator (repomix shape, row 9) that emits: the crate's modmap slice,
   its `docs/CRATE-MAP.md` row, its plan-index line, its board row from
   `tools/board-slice.mjs`, and its own `//!` headers. Target: a first packet
   under 6,000 tokens. Move `docs/archive/` (41.8 MB, 77% of docs bytes) out
   of the default read path and add the missing `docs/plan/kam2-ui.md` index
   line. Accept: an agent names every file it may open from the packet alone,
   and the entry set (14,700 tok today) does not grow.

## Rejections

- **cargo-udeps** - row 12, RESEARCH-ONLY: needs a nightly cargo the pinned
  1.97.1 toolchain does not have, and cargo-machete covers most of it.
- **twiggy** 0.8.0 (https://crates.io/crates/twiggy, license field
  "Apache-2.0/MIT" read live 2026-10-20) - REJECT: size attribution for a
  wasm binary; cargo-bloat does the same axis for our exe and wasm with a
  maintained tool.
- **dhat** 0.3.3 (https://crates.io/crates/dhat, MIT OR Apache-2.0 read live
  2026-10-20) - REJECT for compaction: it is a heap profiler, the wrong axis
  (repo size and agent context, not process memory).
- **cargo-insta** 1.49.0 (Apache-2.0, crates.io read live 2026-10-20) -
  REJECT: snapshot testing needs a new snapshot-baseline system; the repo
  already has golden hashes and `tools/proof.mjs`.
- **proptest** 1.12.0 (Apache-2.0, crates.io read live 2026-10-20) -
  RESEARCH-ONLY: a library, not a workspace tool; the plan §3 already names
  property tests but no row owns them.
- **bolero** 0.13.7 (crates.io read live 2026-10-20; repo https://github.com/camshaft/bolero,
  `master/LICENSE` MIT read live) - RESEARCH-ONLY: same gap as proptest, and it
  wants nightly; no board row owns property tests.
- **criterion** 0.8.2 (Apache-2.0, crates.io read live 2026-10-20) - REJECT:
  the repo has `crates/perf` baselines and `tools/bench-compare.mjs`; a second
  bench gate is a second gate.
- **sccache** 0.18.0 (Apache-2.0 OR MIT, crates.io read live 2026-10-20) -
  ALREADY OURS: CI sets `RUSTC_WRAPPER=sccache`; the local run measured DROP
  on 2026-10-20 (`docs/receipts/2026-10-20-opt9-compile-cache-spike.md`), so
  OPT9 closes with its numbers rather than a new tool.
- **mold** (MIT, GitHub API read live 2026-10-20) - ALREADY OURS on Linux CI
  (`-C link-arg=-fuse-ld=mold`), Windows keeps `rust-lld.exe`; no row left.
- **lld** - ALREADY OURS: `.cargo/config.toml` sets `linker = "rust-lld.exe"`,
  121 s -> 79 s cold link (OPT4).
- **gitingest** (MIT, `main/LICENSE` read live 2026-10-20 at
  https://github.com/coderamp-labs/gitingest) - REJECT: a repomix clone; one
  packer is enough.
- **code2prompt** (MIT, GitHub API read live 2026-10-20 at
  https://github.com/mufeedvh/code2prompt) - REJECT: another clone, and it
  concatenates whole files where we want a ranked map.
- **ast-grep** (MIT, GitHub API read live 2026-10-20 at
  https://github.com/ast-grep/ast-grep) - RESEARCH-ONLY: structural search is
  real value, but `git grep` plus the modmap answer every query we ask today;
  revisit if a query becomes a row.
- **cargo-public-api** (MIT, `main/LICENSE` read live 2026-10-20) - REJECT:
  `tools/api-break.mjs` already prints the public surface and diffs it.
- **cargo-hack** (MIT OR Apache-2.0, crates.io 0.6.45 read live 2026-10-20) -
  RESEARCH-ONLY: `--feature-powerset` is the right way to prove the dead
  features are dead, but it multiplies the gate; use it once, not per commit.
- **cargo-deny** - ALREADY OURS (`deny.toml` + CI `cargo deny check`); it is
  the licence gate, not a size tool.
- **Aider repo map as a donor** - already in our tree as LI100
  (`docs/plan/landscape-ideas.md:106`) and `docs/research/steals-2026-10-11/engine-landscape.md:317`;
  row 10 is a CORRECTION with the live licence and the measured modmap cost.

## Top 3

1. **cargo-llvm-lines** (MIT OR Apache-2.0, read live 2026-10-20) -
   https://github.com/dtolnay/cargo-llvm-lines - closes **OPT11**: the only
   cheap way to name which crate costs the build, with `engine-render`
   (107,807 lines, 294 s of 347 s) as the measured target.
2. **cargo-hakari** (MIT OR Apache-2.0, read live 2026-10-20) -
   https://crates.io/crates/cargo-hakari - closes **OPT8**: our union crate
   `crates/workspace-hack` has no dependent, so today it unifies nothing; the
   real target is the 80 multi-version names (101 extra copies).
3. **cargo-machete** (MIT, read live 2026-10-20) -
   https://github.com/bnjbvr/cargo-machete - closes new row **OPT37**: the
   cheapest of the three (no nightly, minutes not hours) and the one that
   removes dependencies from 42 manifests before we spend a week on the
   heavy ones.
