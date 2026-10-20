# Proposed rows - code compaction (lane 09)

One row per find, measured on our own tree at HEAD `77789c2a` on 2026-10-20.
Every number below was printed by a command in the row's Accept line. The lead
merges the board; this file edits nothing in `docs/SPRINT*.md`,
`docs/FOCUS.md` or `docs/plan/finish-line.md`.

Row shape: id, title, what it is, donor, the engine2040 row it closes, owned
paths, Accept, kill line, cost.

Shared proof commands (quoted from the tree, read 2026-10-20):
- `node tools/gh-check.mjs cargo test -p <crate> --lib` prints
  `PASS on the runner: cargo test -p <crate> --lib (run <url>)` and exits 0.
- `cargo run --locked -- --verify` prints `final_hash=d57b5c8ae3096614` and
  `verified=true` (`llms.txt`, "Prove it works").

---

## CMP-01 - the compaction baseline: one file the lead reads first

**What it is.** A committed baseline of the measured size of the engine and of
the agent's context: crate count, lines per crate, the 20 largest files, the
80 multi-version lock names, the 7 dead feature flags, the dead-line manifest,
disk and docs sizes, token sizes. One tool (`tools/crate-map.mjs`) grows the
line and version counts; `tools/workspace-check.mjs` gates the drift.

**Donor.** None (our own tree). Measurement snapshot: `git archive HEAD crates`
at `77789c2a`. Path written: `docs/compaction-baseline.md`. Licence: our own
Apache-2.0 tree. Read 2026-10-20.

**Closes.** E51 (measured hotspot bundle) + new row **CMP01-GATE** (baseline
drift gate).

**Owned paths.** `docs/compaction-baseline.md`, `tools/crate-map.mjs`,
`tools/workspace-check.mjs`, `tools/crate-map.test.mjs`,
`tools/workspace-check.test.mjs`.

**Accept.** `node tools/workspace-check.mjs` prints
`PASS baseline: 42 crates, 816611 src lines, 80 multi-version lock names, 7 dead feature flags` and exits 0; a moved number fails with
`FAIL baseline: src lines 816611 -> 816700`. `node tools/crate-map.mjs --check`
prints `PASS docs/CRATE-MAP.md` and exits 0.

**Kill line.** The gate cannot be made to run in under 2 s on top of the
existing audit -> keep the baseline file as a document, drop the gate.

**Cost.** S.

---

## CMP-02 - crates/physics/src/kcc.rs: 928 lines, no caller at all

**What it is.** A character-controller module in the physics crate that no
code path reaches. The whole file is dead weight in the crate that also owns
the ragdoll (3,376 lines) and the determinism law.

**Donor.** None (our own tree). Path `crates/physics/src/kcc.rs`, 928 lines.
Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** E64 (five scouted Rust cuts) + new row **CMP02-KCC**.

**Owned paths.** `crates/physics/src/kcc.rs`, `crates/physics/src/lib.rs`,
`crates/physics/tests/it/*`.

**Accept.** `git grep -n "kcc::" -- crates/` prints nothing (0 hits before and
after, the module path is never written); `node tools/gh-check.mjs cargo test
-p engine-physics --lib` prints `PASS on the runner: cargo test -p
engine-physics --lib (run <url>)`; `cargo run --locked -- --verify` still
prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** Any production caller appears -> wire it instead of deleting.

**Cost.** S.

---

## CMP-03 - crates/ai/src/behavior.rs: 275 lines, re-exported but never called

**What it is.** A behaviour-tree module in the ai crate. The crate has its own
live `bt.rs` and `goap.rs`, so this second tree is a duplicate surface with
zero users: `crates/ai/src/lib.rs:23` re-exports it
(`pub use behavior::{tick, Behavior, Status};`) but nothing calls the three
names - `git grep -n "engine_ai::Behavior" -- crates/`,
`git grep -n "engine_ai::tick" -- crates/` and
`git grep -n "engine_ai::Status" -- crates/` all print 0 hits. The only other
`behavior::` hit in the tree is a doc comment at `crates/ai/src/gladiator.rs:12`.

**Donor.** None (our own tree). Path `crates/ai/src/behavior.rs`, 275 lines,
plus the re-export at `crates/ai/src/lib.rs:23`. Licence: our own Apache-2.0
tree. Read 2026-10-20.

**Closes.** E64 + new row **CMP03-BT**.

**Owned paths.** `crates/ai/src/behavior.rs`, `crates/ai/src/lib.rs`,
`crates/ai/tests/it/*`.

**Accept.** `git grep -n "engine_ai::Behavior" -- crates/`,
`git grep -n "engine_ai::tick" -- crates/` and `git grep -n "engine_ai::Status" -- crates/` print nothing, and `git grep -n "behavior::" -- crates/` prints nothing after the cut (today it prints 2 lines, both inside engine-ai); `node tools/gh-check.mjs cargo test -p engine-ai --lib` prints `PASS on the
runner: cargo test -p engine-ai --lib (run <url>)`; `cargo run --locked --
--verify` prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** Any production caller appears -> wire it instead of deleting.

**Cost.** S.

---

## CMP-04 - crates/nav/src/orca.rs: 971 lines, the real "nav::crowd"

**What it is.** The ORCA crowd avoidance module in the nav crate, exported as
`pub mod orca;` and never reached. The stale audit calls this
"nav::crowd (ORCA)" - there is no `nav::crowd` (see CMP-15); this is the
file the audit meant.

**Donor.** None (our own tree). Path `crates/nav/src/orca.rs`, 971 lines, with
`crates/nav/src/lib.rs:12` (`pub mod orca;`). Licence: our own Apache-2.0
tree. Read 2026-10-20.

**Closes.** E64 + new row **CMP04-ORCA**.

**Owned paths.** `crates/nav/src/orca.rs`, `crates/nav/src/lib.rs`,
`crates/nav/tests/it/*`.

**Accept.** `git grep -n "orca::" -- crates/` prints nothing; `git grep -n
"Orca" -- crates/` prints nothing outside `crates/nav/src/orca.rs`;
`node tools/gh-check.mjs cargo test -p engine-nav --lib` prints `PASS on the
runner: cargo test -p engine-nav --lib (run <url>)`; `cargo run --locked --
--verify` prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** The nav crowd work lands in a row and needs this file -> keep it
and park the row.

**Cost.** S.

---

## CMP-05 - crates/render/src/heal.rs: 1,090 lines, only its own tests build it

**What it is.** The renderer heal catalogue (`RenderHeal`). The type is
mentioned in `crates/render/src/present/surface.rs:252` in a doc comment only
(`/// Consumer: [`crate::heal::RenderHeal`]`), and the only construction is
inside the file's own test module.

**Donor.** None (our own tree). Path `crates/render/src/heal.rs`, 1,090 lines.
Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** E64 + new row **CMP05-RENDERHEAL**.

**Owned paths.** `crates/render/src/heal.rs`, `crates/render/src/lib.rs`,
`crates/render/src/present/surface.rs`, `crates/render/tests/it/*`.

**Accept.** `git grep -n "RenderHeal::new" -- crates/` prints nothing;
`node tools/gh-check.mjs cargo test -p engine-render --lib` prints `PASS on the
runner: cargo test -p engine-render --lib (run <url>)`; `cargo run --locked --
--verify` prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** A supervisor or present path starts calling `RenderHeal` -> wire
it instead of deleting.

**Cost.** S.

---

## CMP-06 - crates/audio/src/heal.rs: 749 lines, only a test builds it

**What it is.** The audio device heal (`DeviceHeal`). The only construction
outside the file is `crates/game/src/audio_mix.rs:390`, inside `#[cfg(test)] mod
tests` (`crates/game/src/audio_mix.rs:171`), so no production path reaches it.

**Donor.** None (our own tree). Path `crates/audio/src/heal.rs`, 749 lines.
Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** E64 + new row **CMP06-DEVICEHEAL**.

**Owned paths.** `crates/audio/src/heal.rs`, `crates/audio/src/lib.rs`,
`crates/audio/src/device.rs`, `crates/audio/tests/it/*`,
`crates/game/src/audio_mix.rs`.

**Accept.** `git grep -n "DeviceHeal::new" -- crates/` prints nothing outside
a `#[cfg(test)]` module; `node tools/gh-check.mjs cargo test -p engine-audio
--lib` prints `PASS on the runner: cargo test -p engine-audio --lib (run
<url>)`; `cargo run --locked -- --verify` prints
`final_hash=d57b5c8ae3096614` twice.

**Kill line.** A production audio path starts calling `DeviceHeal` -> wire it.

**Cost.** S.

---

## CMP-07 - crates/observe/src/rerun_sink.rs: 321 lines, no outside caller

**What it is.** The bounded 120-frame rerun ring. `crates/observe/src/timeline.rs`
only links to it in doc comments; the only use is
`crates/observe/tests/it/w1_pf9_rerun.rs:9`.

**Donor.** None (our own tree). Path `crates/observe/src/rerun_sink.rs`, 321
lines. Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** E64 + new row **CMP07-RERUN**.

**Owned paths.** `crates/observe/src/rerun_sink.rs`,
`crates/observe/src/timeline.rs`, `crates/observe/src/lib.rs`,
`crates/observe/tests/it/w1_pf9_rerun.rs`.

**Accept.** `git grep -n "RerunSink::" -- crates/` prints nothing outside
`crates/observe/src/rerun_sink.rs`; `node tools/gh-check.mjs cargo test -p
engine-observe --lib` prints `PASS on the runner: cargo test -p
engine-observe --lib (run <url>)`; `cargo run --locked -- --verify` prints
`final_hash=d57b5c8ae3096614` twice.

**Kill line.** The rerun ring is wired into the arena run (the N-11 ask in
`docs/plan/forge-adoption.md`) -> keep it and mark the row done instead.

**Cost.** S.

---

## CMP-08 - the 7 dead feature flags

**What it is.** Seven crate features that no other crate asks for and that
their own crate never reads with `cfg(feature = ...)`: `endpoint/tls`
(`crates/endpoint/Cargo.toml`), `geo/default`, `geo/kit`, `geo/all`
(`crates/geo/Cargo.toml`), `kam2-game/window` (`crates/kam2-game/Cargo.toml`),
`playtest/tls` (`crates/playtest/Cargo.toml`), `scene/math`
(`crates/scene/Cargo.toml`). Seven more are weak (one `cfg` use, no external
request): `core/test-util`, `geo/bake`, `geo/fbx`, `proof/arena`, `proof/spine`,
`proof/ragdoll`, `rules/arena`.

**Donor.** cargo-unused-features 0.2.0,
https://crates.io/crates/cargo-unused-features (version page, read live
2026-10-20; source https://github.com/TimonPost/cargo-unused-features, path
`main/LICENSE`), licence MIT. Our own scan at HEAD `77789c2a` produced the
same 7 dead and 7 weak names.

**Closes.** New row **CMP08-DEADFLAGS** + E64.

**Owned paths.** the seven `Cargo.toml` files above, plus the `cfg(feature =)`
lines in their crates.

**Accept.** `node tools/crate-map.mjs --check` prints `PASS
docs/CRATE-MAP.md`; a feature scan over `crates/*/Cargo.toml` prints
`dead features: 0` (today it prints `dead features: 7`); `cargo run --locked
-- --verify` prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** A flag turns out to be requested by a script or a CI job that
cargo cannot see -> keep the flag and say so in the row.

**Cost.** S.

---

## CMP-09 - 101 duplicate dependency copies across 80 names

**What it is.** `Cargo.lock` at HEAD holds 1,032 packages; 80 crate names
carry more than one version and 101 extra copies are compiled. The first five
to fix: `hashbrown` 5 versions, `glam` 4, `windows-sys` 4, `rand` 3, `syn` 3 -
19 of the 101 extra copies from five names. 16 external dependency names are
declared by more than one crate and 12 of them carry divergent feature sets
(`image` 3 variants, `engine-assets` 3, `engine-render` 3, `engine-core`,
`engine-arena`, `engine-anim`, `engine-scene`, `engine-game`, `wgpu`, `uuid`,
`tokio`, `web-sys` 2 each).

**Donor.** cargo-hakari 0.9.39, https://crates.io/crates/cargo-hakari
(version page, read live 2026-10-20; the code now lives at
https://github.com/guppy-rs/guppy), licence MIT OR Apache-2.0. Our own
measurement: `Cargo.lock` at `77789c2a`.

**Closes.** OPT8 (feature unification spike) - **owned by the speedup lane**,
so this row carries the numbers only and lands no code.

**Owned paths.** none from this lane; the OPT8 lane owns
`crates/workspace-hack/Cargo.toml` and the manifests it touches.

**Accept.** the OPT8 lane's own bar: the dependency rebuild count across five
lane builds drops by a third. Our number to compare against, measured from
`Cargo.lock` at HEAD `77789c2a`: 80 names with more than one version and 101
extra copies; after the lane it must be at most 67 names and at most 67 extra
copies.

**Kill line.** The union crate still has no dependent crate after the change ->
close OPT8 with the measured count instead of keeping the hack crate.

**Cost.** M (the speedup lane).

---

## CMP-10 - 91 tracked .profraw files and a 28.9 GiB target/debug

**What it is.** 91 cargo profile artifacts are tracked in git (90 at the repo
root, 1 in `crates/game/`), 55,519,832 bytes. `target/debug` on this PC holds
84,947 files and 30,974,305,957 bytes (28.9 GiB) and is not capped yet.

**Donor.** None (our own tree). Paths: the 91 tracked `.profraw` files (`git
ls-files "*.profraw"`), and `target/debug/`. Licence: our own Apache-2.0 tree.
Read 2026-10-20.

**Closes.** E86 (cap target/debug) + new row **CMP10-PROFRAW**.

**Owned paths.** `.gitignore`, the 91 `.profraw` paths, `target/`.

**Accept.** `git ls-files "*.profraw"` prints nothing (today it prints 91
lines); `node tools/clean.mjs --dry-run` lists no `target/debug` entry;
`cargo run --locked -- --verify` still prints `final_hash=d57b5c8ae3096614`.

**Kill line.** A test needs a committed `.profraw` as a golden -> keep that one
file and say which.

**Cost.** S.

---

## CMP-11 - the agent's working set: a per-crate packet under 6,000 tokens

**What it is.** Today an agent's entry set is about 14,703 tokens
(`llms.txt` 856, `AGENTS.md` 1,420, `docs/book/for-ai.md` 1,512,
`docs/CRATE-MAP.md` 2,009, `.opencode/repomap.md` 3,966,
`docs/plan/README.md` 4,940) and `.opencode/modmap.md` adds 36,154 more if
read whole (144,615 bytes, 1,397 lines, 42 crate sections). One crate's slice
is 300-4,068 tokens (render 4,068 = 11.3%, endpoint 3,810, game 2,652, assets
2,289, net 2,136, anim 1,561, playtest 1,540, editor 1,191, ai 973, physics
947, core 797). The packet is what an agent should get: one crate's map
slice, its `docs/CRATE-MAP.md` row, its plan-index line, its board row from
`node tools/board-slice.mjs <ID>`.

**Donor.** repomix, https://github.com/yamadashy/repomix (GitHub API
repository object, `license.spdx_id = MIT`, read live 2026-10-20), licence
MIT. Aider's ranked map, https://github.com/Aider-AI/aider (`license.spdx_id =
Apache-2.0`, read live 2026-10-20), path `aider/repomap.py`, licence
Apache-2.0; tree-sitter https://github.com/tree-sitter/tree-sitter and
https://github.com/tree-sitter/tree-sitter-rust (path `master/LICENSE`), both
MIT, read live 2026-10-20.

**Closes.** LI100 (`docs/plan/landscape-ideas.md:106`) + new row **CMP11-PACKET**.

**Owned paths.** `tools/modmap.mjs`, `.opencode/modmap.md`, `docs/INDEX.md`,
`llms.txt`.

**Accept.** `node tools/modmap.mjs --check` prints `PASS .opencode/modmap.md`;
a packet for `crates/render` (map slice + CRATE-MAP row + plan line + board
row) measures under 6,000 tokens; `node tools/for-ai.test.mjs` passes.

**Kill line.** A packet over 6,000 tokens for one crate -> the map slice wins
and the packet is dropped.

**Cost.** S.

---

## CMP-12 - stale plan docs: docs/archive is 76.7% of docs bytes

**What it is.** `docs/` holds 1,207 files and 54,519,106 bytes; `docs/archive`
alone holds 851 files and 41,803,786 bytes (76.7%). `docs/plan/` holds 70
files and 1,170,549 bytes. `docs/plan/kam2-ui.md` is not in the
`docs/plan/README.md` index although the index says `tools/plan-index.test.mjs`
checks every plan file.

**Donor.** None (our own tree). Paths `docs/archive/`, `docs/plan/README.md`,
`docs/plan/kam2-ui.md`. Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** New row **CMP12-DOCS**.

**Owned paths.** `docs/plan/README.md`, `docs/plan/kam2-ui.md`,
`tools/plan-index.test.mjs`.

**Accept.** `node --test tools/plan-index.test.mjs` passes with
`docs/plan/kam2-ui.md` in the index; `node tools/docs-coverage.mjs` prints
`RESULT PASS`; the index line for every plan file stays one line per file.

**Kill line.** `docs/archive` is read by a live tool -> move that tool's
readers to the live path first.

**Cost.** S.

---

## CMP-13 - CORRECTION: crates/assets/import_repair IS wired

**What it is.** `docs/plan/forge-adoption.md` A.3 item 5 lists
`assets/import_repair` as "landed code with no production caller". That is
stale. The caller is `crates/render/src/v1.rs:256`:
`let tangents = engine_assets::import_repair::recompute_tangents(&proxy);`
and a second one at
`crates/render/src/present/gpu_present/presenter.rs:2197` (the same call).
The path in the request, `crates/render/src/present/gpu_present/v1.rs`, does
not exist in the tree (`Test-Path` prints False); the real file is
`crates/render/src/v1.rs`. `crates/render/Cargo.toml:54` says so too.

**Donor.** None (our own tree). Paths `crates/render/src/v1.rs:256`,
`crates/render/src/present/gpu_present/presenter.rs:2197`,
`crates/assets/src/import_repair.rs` (1,310 lines). Licence: our own
Apache-2.0 tree. Read 2026-10-20.

**Closes.** new row **CMP13-CORRECTIONS** (the dated correction note for
`docs/plan/forge-adoption.md` A.3 item 5).

**Owned paths.** `docs/plan/forge-adoption.md` (one dated correction line),
`docs/plan/2026-10-20-progress.md`.

**Accept.** `git grep -n "import_repair::recompute_tangents" -- crates/`
prints 2 hits (today it prints 2); the correction line in
`docs/plan/forge-adoption.md` is dated 2026-10-20.

**Kill line.** The render v1 caller is removed -> the claim becomes true again
and the row flips back.

**Cost.** S.

---

## CMP-14 - CORRECTION: engine-playtest HAS a consumer

**What it is.** `docs/plan/forge-adoption.md` A.3 item 3 says "the playtest
crate has no consumer". That is stale. `crates/endpoint/Cargo.toml:16` puts
`dep:engine-playtest` in the endpoint's `mcp` feature, line 32 defines
`verify = ["mcp", "dep:engine-proof"]`, and line 79 declares
`engine-playtest = { workspace = true, optional = true }`. The call sites are
`crates/endpoint/src/packs/verify.rs:34-37`
(`use engine_playtest::mirror::{run_mirror, MirrorReport};`,
`use engine_playtest::mirror_kit::MirrorRow;`,
`use engine_playtest::replay::{bisect_tape, verify_tape, ReplayError};`,
`use engine_playtest::seeded_bug::{...}`), plus
`crates/endpoint/src/look_host.rs:7` and `crates/endpoint/src/packs/view.rs:49`.
One stale comment survives: `crates/endpoint/src/assistant/fork.rs:19` still
says "`engine-playtest` stays outside the endpoint dep edge".

**Donor.** None (our own tree). Paths `crates/endpoint/Cargo.toml:16,32,79`,
`crates/endpoint/src/packs/verify.rs:34-37`. Licence: our own Apache-2.0 tree.
Read 2026-10-20.

**Closes.** new row **CMP13-CORRECTIONS**.

**Owned paths.** `docs/plan/forge-adoption.md` (one dated correction line),
`crates/endpoint/src/assistant/fork.rs:19` (the stale comment).

**Accept.** `git grep -c "engine_playtest" -- crates/endpoint/src/packs/verify.rs`
prints `4`; the comment at `crates/endpoint/src/assistant/fork.rs:19` no longer
says the dep edge does not exist; `node tools/gh-check.mjs cargo build -p
engine-endpoint --features mcp,verify` prints `PASS on the runner: cargo build
-p engine-endpoint --features mcp,verify (run <url>)`.

**Kill line.** The endpoint `verify` pack is removed -> the claim becomes true
again.

**Cost.** S.

---

## CMP-15 - CORRECTION: there is NO nav::crowd in the tree

**What it is.** `docs/plan/forge-adoption.md` A.3 item 5 names
`nav::crowd` (ORCA) as an unwired module. There is no such module.
`git grep -n "nav::crowd" -- crates/` prints nothing (exit 1);
`git grep -n "engine_nav::crowd" -- crates/` prints nothing (exit 1);
`crates/nav/src/` has 14 files and no `crowd.rs` (bake, cursor, flow, funnel,
layers, lib, links, mesh, orca, quant, reach, rebake, spline, tiles). The
ORCA code the audit meant is `crates/nav/src/orca.rs` (971 lines, row CMP-04),
and the shared cue type is `crates/audio/src/crowd.rs` (512 lines), which is
live (six intra-crate callers plus `crates/game/src/audio_director.rs:17`).
The workspace-underused lane repeated `nav::crowd` from the stale list; the
lead must not merge that claim.

**Donor.** None (our own tree). Paths `crates/nav/src/` (listing),
`crates/nav/src/orca.rs`, `crates/audio/src/crowd.rs`. Licence: our own
Apache-2.0 tree. Read 2026-10-20.

**Closes.** new row **CMP13-CORRECTIONS**.

**Owned paths.** `docs/plan/forge-adoption.md` (one dated correction line).

**Accept.** `git grep -n "nav::crowd" -- crates/` prints nothing; the
correction line in `docs/plan/forge-adoption.md` names `crates/nav/src/orca.rs`
instead of `nav::crowd` and is dated 2026-10-20.

**Kill line.** A `nav::crowd` module appears -> the claim becomes true again.

**Cost.** S.

---

## CMP-16 - CORRECTION: the SMOOTH-BUILD dependency numbers are build artifacts

**What it is.** `docs/SPRINT-13-engine-edge.md:34` (the OPT8 progress note)
and the board say "16 syn, 12 serde_json, 14 num_traits copies in
target/debug/deps before the hack". Those are build-artifact counts from
2026-10-17, not `Cargo.lock` facts. At HEAD `77789c2a` the lock holds
**1,032 packages, 80 names with more than one version, 101 extra copies**;
`serde_json` is **one** version (1.0.151), `num-traits` is **one** version
(0.2.19) and is not a direct dependency of any crate (it arrives
transitively), and `syn` is three versions (1.0.109, 2.0.119, 3.0.6).
`target/debug/deps` on this PC holds 16 `syn-*`, 13 `serde_json-*` and 9
`num_traits-*` artifacts, which is why the old numbers drift every day.

**Donor.** None (our own tree). Paths `Cargo.lock`,
`docs/SPRINT-13-engine-edge.md:34`, `docs/SMOOTH-BUILD.md` ("Build speed
facts"). Licence: our own Apache-2.0 tree. Read 2026-10-20.

**Closes.** new row **CMP13-CORRECTIONS** + OPT8 (the numbers the row needs).

**Owned paths.** `docs/SPRINT-13-engine-edge.md` (one line, board shard),
`docs/SMOOTH-BUILD.md` (one line in "Build speed facts").

**Accept.** the OPT8 progress note reads "80 names carry more than one version
in Cargo.lock (101 extra copies); serde_json and num-traits are one version
each; num-traits is not a direct dependency" instead of the three artifact
counts; `cargo tree --workspace --duplicates` lists 80 names.

**Kill line.** the board shard is rewritten by the OPT8 lane first -> the
correction rides that commit instead.

**Cost.** S.

---

## CMP-17 - split engine-render: 13.9% of the tree and the compile critical path

**What it is.** `crates/render` is 113,519 lines in 139 files (13.9% of
816,611) and owns the dev build's critical path (294 s of a 347 s build,
`docs/SMOOTH-BUILD.md`). `engine-endpoint` is 78,701 (9.6%) and
`engine-game` 77,878 (9.5%). The 20 largest files hold 74,176 lines (9.1% of
the tree) and 7 of them are in render or game.

**Donor.** cargo-llvm-lines 0.4.48,
https://github.com/dtolnay/cargo-llvm-lines (crates.io version page and
`master/LICENSE-MIT`, read live 2026-10-20), licence MIT OR Apache-2.0; path
`src/main.rs` (the `-p` per-crate attribution). Our measurement at
`77789c2a`.

**Closes.** OPT11 (crate fan-out cut) + E64.

**Owned paths.** `crates/render/**` (the split the lead picks),
`docs/SMOOTH-BUILD.md` (the before/after numbers).

**Accept.** `cargo llvm-lines -p engine-render` prints a per-crate table
before the split and after it; the touch-rebuild of the chosen crate drops by
at least 30% (OPT11's own bar) or the row closes with its numbers; `cargo run
--locked -- --verify` prints `final_hash=d57b5c8ae3096614` twice.

**Kill line.** The split does not cut the touch-rebuild by 30% -> close OPT11
with the numbers and stop.

**Cost.** M.
