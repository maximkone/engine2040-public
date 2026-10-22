# Compaction baseline - the measured size of engine2040 and of an agent's context

The four machine-readable lines below are the numbers the drift gate reads, and
they move with every size change. The tables in section 1 are a dated snapshot
of the committed tree at HEAD `06124ea96` (2026-10-22); the commands reproduce
them.

**Machine-readable baseline.** The drift gate in `tools/workspace-check.mjs`
reads these four lines. Update them in the commit that lands the size change:

```
- crates: 43
- src lines: 890802
- multi-version lock names: 80
- dead feature flags: 0
```

**How to refresh this file.** Run `node tools/crate-map.mjs --baseline`. It
counts the committed tree (HEAD) and prints the four numbers; paste them into
the block above. The audit then prints
`PASS baseline: 43 crates, 888420 src lines, 80 multi-version lock names,
0 dead feature flags`. A number that moves without this file moving fails the
audit with `FAIL baseline: src lines 888420 -> 888517`, so the recorded size
of the engine cannot go stale without a signal.

**Why the committed tree.** The baseline is a snapshot of a commit, so the
gate measures HEAD and not the working tree. Another session's uncommitted
work must not turn the shared audit red; a commit that moves the size
without this file moving does. The numbers in sections 1.1 to 1.6 are the
committed tree too. The docs, token and disk numbers (1.7 to 1.9) are this
PC's working tree, because `.opencode/` is a local file and `target/` is
build output.

**Line definition used everywhere in this file:** physical lines, blank lines
included (the count `wc -l` gives). Where a second number matters it is
labelled *non-blank*. Two counts exist because two tools count differently:
`Get-Content | Measure-Object -Line` skips blank lines, a node
`split("\n").length - 1` does not. The baseline uses the second.

**How to reproduce the crate and file numbers:** `node tools/crate-map.mjs
--baseline` counts them from the committed tree. The same numbers come out of
`git archive HEAD crates | tar -x -C <tmp>` and a walk of
`<tmp>/crates/*/src/*.rs` that counts `split("\n").length - 1`.

**How to reproduce the lock numbers:** `git show HEAD:Cargo.lock > <tmp>/Cargo.lock`
(one pass over the `[[package]]` blocks: a name with more than one version is
a multi-version name, the extra copies are `copies - 1` per name).

**How to reproduce the orphan evidence** (no production caller reaches the
module): search every `crates/**/*.rs` for the module path itself -
`crate::<mod>::`, `engine_<crate>::<mod>::`, `super::<mod>::` - and ignore the
module's own file. A type name alone is not evidence: `World`, `Tier`,
`Action`, `Crowd` and `repair` match in unrelated crates.

---

## 1. The baseline

### 1.1 Crate count

| item | value |
|---|---|
| workspace members | 44 (43 library crates under `crates/*` + the root package `engine2040`) |
| crates with a `src/` | 43 |
| `crates/*/src` .rs files | 1,256 |
| `crates/*/src` lines (all) | **887,923** |
| `crates/*/src` lines (non-blank) | 838,886 |
| `crates/**` .rs files (with `tests/`) | 1,469 |
| `crates/**` lines (all) | 934,787 |
| top 3 crates | render + endpoint + game = 289,736 lines = 32.6% |

`crates/rome` is new since the last snapshot (the Rome slice).

### 1.2 Lines per crate (`crates/<c>/src`, all physical lines)

| crate | files | lines | non-blank | % of src |
|---|---|---|---|---|
| render | 151 | 120,477 | 114,396 | 13.6% |
| endpoint | 127 | 84,749 | 80,217 | 9.5% |
| game | 89 | 84,510 | 80,234 | 9.5% |
| net | 68 | 55,534 | 52,271 | 6.3% |
| assets | 69 | 45,781 | 43,288 | 5.2% |
| playtest | 65 | 44,692 | 42,076 | 5.0% |
| anim | 63 | 39,434 | 37,449 | 4.4% |
| spatial | 45 | 38,355 | 36,450 | 4.3% |
| editor | 40 | 38,236 | 36,042 | 4.3% |
| physics | 31 | 28,135 | 26,667 | 3.2% |
| ai | 38 | 26,996 | 25,434 | 3.0% |
| arena | 32 | 26,593 | 25,186 | 3.0% |
| combat | 29 | 24,681 | 23,382 | 2.8% |
| provider | 32 | 22,572 | 21,161 | 2.5% |
| audio | 25 | 20,729 | 19,471 | 2.3% |
| proof | 33 | 18,568 | 17,577 | 2.1% |
| destruction | 25 | 17,108 | 16,272 | 1.9% |
| perf | 18 | 12,329 | 11,600 | 1.4% |
| ui | 9 | 11,776 | 11,046 | 1.3% |
| ecs | 16 | 11,542 | 10,965 | 1.3% |
| web | 21 | 10,585 | 9,841 | 1.2% |
| material | 17 | 9,953 | 9,341 | 1.1% |
| nav | 14 | 9,572 | 9,057 | 1.1% |
| rome | 12 | 8,065 | 7,485 | 0.9% |
| physfx | 32 | 7,928 | 7,439 | 0.9% |
| core | 26 | 7,872 | 7,335 | 0.9% |
| observe | 10 | 7,282 | 6,762 | 0.8% |
| world | 11 | 6,322 | 5,957 | 0.7% |
| distill | 12 | 6,261 | 5,901 | 0.7% |
| kam2-game | 14 | 6,189 | 5,781 | 0.7% |
| scene | 10 | 6,041 | 5,613 | 0.7% |
| geo | 14 | 5,810 | 5,480 | 0.7% |
| rules | 11 | 5,148 | 4,852 | 0.6% |
| app | 10 | 3,586 | 3,336 | 0.4% |
| input | 8 | 2,972 | 2,742 | 0.3% |
| kam2-sim | 6 | 2,853 | 2,692 | 0.3% |
| ossim | 6 | 2,504 | 2,357 | 0.3% |
| nn | 3 | 2,136 | 2,009 | 0.2% |
| kam2-web | 7 | 2,067 | 1,897 | 0.2% |
| save | 4 | 1,675 | 1,556 | 0.2% |
| engine-chaos | 1 | 165 | 150 | 0.0% |
| clock | 1 | 138 | 117 | 0.0% |
| workspace-hack | 1 | 2 | 2 | 0.0% |

### 1.3 The 20 largest files (76,737 lines = 8.6% of the tree)

| # | file | lines | non-blank |
|---|---|---|---|
| 1 | crates/game/src/fighter_look.rs | 6,599 | 6,335 |
| 2 | crates/arena/src/lib.rs | 5,390 | 5,169 |
| 3 | crates/game/src/bin/arena.rs | 4,949 | 4,768 |
| 4 | crates/render/src/ibl.rs | 4,409 | 4,245 |
| 5 | crates/combat/src/fight.rs | 4,137 | 3,970 |
| 6 | crates/game/src/lib.rs | 4,004 | 3,811 |
| 7 | crates/ui/src/lib.rs | 3,907 | 3,679 |
| 8 | crates/game/src/windowed.rs | 3,588 | 3,412 |
| 9 | crates/render/src/gtao.rs | 3,486 | 3,336 |
| 10 | crates/game/src/camera.rs | 3,459 | 3,308 |
| 11 | crates/render/src/present/gpu_present/presenter.rs | 3,453 | 3,311 |
| 12 | crates/editor/src/commands.rs | 3,441 | 3,281 |
| 13 | crates/endpoint/src/scene_tools.rs | 3,422 | 3,306 |
| 14 | crates/physics/src/ragdoll.rs | 3,376 | 3,247 |
| 15 | crates/render/src/present/gpu_present/live.rs | 3,326 | 3,206 |
| 16 | crates/spatial/src/relation.rs | 3,250 | 3,103 |
| 17 | crates/render/src/present/gpu_present/submit.rs | 3,249 | 3,129 |
| 18 | crates/ai/src/depth.rs | 3,145 | 2,959 |
| 19 | crates/net/src/interest.rs | 3,076 | 2,948 |
| 20 | crates/endpoint/src/mcp.rs | 3,071 | 2,884 |

`tools/rust-quality.mjs` pins every file over 3,000 lines and fails when one
grows, so this table and that gate must agree.

### 1.4 The lock: 80 names with more than one version, 101 extra copies

`Cargo.lock`: **1,038 packages**, **80 names carry more than one version**,
**101 extra version copies** are compiled.

First five to fix (19 of the 101 copies from five names): `hashbrown` 5,
`glam` 4, `windows-sys` 4, `rand` 3, `syn` 3.

| name | copies | versions |
|---|---|---|
| hashbrown | 5 | 0.13.2 0.14.5 0.15.5 0.16.1 0.17.1 |
| darling | 4 | 0.20.11 0.21.3 0.23.0 0.24.1 |
| darling_core | 4 | 0.20.11 0.21.3 0.23.0 0.24.1 |
| darling_macro | 4 | 0.20.11 0.21.3 0.23.0 0.24.1 |
| glam | 4 | 0.30.10 0.31.1 0.32.1 0.33.9 |
| windows-sys | 4 | 0.52.0 0.59.0 0.60.2 0.61.2 |
| base64 | 3 | 0.13.1 0.22.1 0.23.1 |
| bit-set | 3 | 0.10.0 0.5.3 0.9.1 |
| getrandom | 3 | 0.2.17 0.3.4 0.4.3 |
| itertools | 3 | 0.13.0 0.14.0 0.15.0 |
| rand | 3 | 0.10.3 0.8.8 0.9.5 |
| rand_core | 3 | 0.10.1 0.6.4 0.9.5 |
| redox_syscall | 3 | 0.4.1 0.5.18 0.9.4 |
| syn | 3 | 1.0.109 2.0.119 3.0.6 |
| bit-vec, bitflags, block-buffer, block2, calloop, calloop-wayland-source, constant_time_eq, convert_case, core-foundation, cpufeatures, crypto-common, digest, downcast-rs, embedded-hal, fixedbitset, foldhash, jni-sys, libloading, linux-raw-sys, miniz_oxide, naga, nb, ndarray, nix, nom, num-derive, objc2, objc2-app-kit, objc2-foundation, objc2-metal, objc2-quartz-core, objc2-ui-kit, ordered-float, pastey, r-efi, rand_chacha, rand_distr, rustc-hash, rustix, safetensors, sha2, shlex, signal-hook, smithay-client-toolkit, synstructure, thiserror, thiserror-impl, webpki-roots, wgpu, wgpu-core, wgpu-core-deps-apple, wgpu-core-deps-emscripten, wgpu-core-deps-windows-linux-android, wgpu-hal, wgpu-naga-bridge, wgpu-types, windows_aarch64_gnullvm, windows_aarch64_msvc, windows_i686_gnu, windows_i686_gnullvm, windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm, windows_x86_64_msvc, windows-targets, zip | 2 each | see `Cargo.lock` |

`serde_json` = **one** version (1.0.151). `num-traits` = **one** version
(0.2.19) and is **not** a direct dependency of any crate.

### 1.5 Feature flags: 50 declared, 0 dead, 6 weak

Dead = declared, no other crate requests it, no `cfg(feature = ...)` in its
own crate. **There are none left**: the seven of the last snapshot
(`engine-endpoint/tls`, `engine-geo/default`, `engine-geo/kit`,
`engine-geo/all`, `kam2-game/window`, `engine-playtest/tls`,
`engine-scene/math`) are cut or live now. The count is measured by
`node tools/feature-scan.mjs` and by `node tools/crate-map.mjs --baseline`.

Weak = no external request, exactly one `cfg` use: `engine-geo/bake`,
`engine-geo/fbx`, `engine-playtest/live`, `engine-proof/arena`,
`engine-proof/ragdoll`, `engine-rules/arena`.

### 1.6 Dead lines: 3,101 lines no production path reaches

| file | lines (all) | non-blank | evidence | verdict |
|---|---|---|---|---|
| crates/nav/src/orca.rs | 1,252 | 1,172 | `git grep -n "orca::" -- crates/` = 0 hits | dead: 0 callers |
| crates/render/src/heal.rs | 1,137 | 1,060 | `RenderHeal` only in its own file; `crates/render/src/present/surface.rs:261` and `:281` are doc links | dead: no production constructor |
| crates/observe/src/rerun_sink.rs | 437 | 405 | `RerunSink` only in its own file, one doc link in `crates/observe/src/timeline.rs` and `crates/observe/tests/it/w1_pf9_rerun.rs` | dead: no outside caller |
| crates/ai/src/behavior.rs | 275 | 260 | `git grep -n "behavior::" -- crates/` = 2 hits, both inside engine-ai (`crates/ai/src/lib.rs` re-export, `crates/ai/src/gladiator.rs` doc comment) | dead: re-exported, never called |
| **total** | **3,101** | 2,897 | | |

Two rows of the old list are no longer dead:

| file | lines (all) | verdict |
|---|---|---|
| crates/audio/src/heal.rs | 792 | live: `crates/audio/src/engine.rs:77` builds `DeviceHeal::new()` |
| crates/physics/src/kcc.rs | - | gone: cut by board row PW50 |

**Live, do not delete:** `crates/ai/src/utility.rs`, `crates/anim/src/lod.rs`,
`crates/audio/src/crowd.rs`, `crates/assets/src/import_repair.rs`.

### 1.7 Disk (this PC)

| item | value |
|---|---|
| `target/debug` | 16,006 files, 6,411,562,986 bytes (6.0 GiB) |
| tracked `.profraw` in git | **0 files, 0 bytes** (the 91 of the last snapshot are gone) |

### 1.8 Docs (this PC)

| item | value |
|---|---|
| `docs/` | 1,336 files, 67,372,356 bytes |
| `docs/` .md only | 734 files, 11,857,555 bytes |
| `docs/archive/` | 852 files, 41,884,650 bytes = **62.2% of all docs bytes** |
| `docs/plan/` | 80 files, 1,623,839 bytes |
| `docs/research/` | 223 files, 13,497,820 bytes |

### 1.9 Agent context (this PC, tokens = bytes / 4)

| file | lines | bytes | approx tokens |
|---|---|---|---|
| llms.txt | 58 | 3,607 | 902 |
| AGENTS.md | 59 | 8,957 | 2,239 |
| docs/book/for-ai.md | 62 | 6,047 | 1,512 |
| docs/CRATE-MAP.md | 64 | 8,964 | 2,241 |
| .opencode/repomap.md | 486 | 15,868 | 3,967 |
| docs/plan/README.md | 217 | 23,002 | 5,751 |
| **entry set total** | | | **~16,612** |
| .opencode/modmap.md (whole) | 1,552 | 157,888 | 39,472 |

One crate's modmap slice (the map is grep-only, never read whole): render
4,309 tok (10.9% of the map), endpoint 4,016, game 2,807, assets 2,709, net
2,166, playtest 1,781, anim 1,670, editor 1,193, spatial 1,182, physics 1,112,
ai 996, perf 974, physfx 926, proof 922, combat 901. Top 3 crates = 11,132
tok = 28.2% of the map. A packet for one crate (map slice + CRATE-MAP row +
plan line + board row) must stay **under 6,000 tokens**.

---

## 2. How the drift gate works

Two halves, one rule, no number written in a tool:

1. **The measured side** is `tools/crate-map.mjs`. It walks every
   `crates/<dir>` once and counts the crates, the physical lines of Rust under
   `crates/<dir>/src`, the names that carry more than one version in
   `Cargo.lock` and the dead feature flags (the dead rule is
   `tools/feature-scan.mjs`, so there is one rule and not two).
   `node tools/crate-map.mjs --baseline` prints the numbers of the committed
   tree: `git archive HEAD` and `tar` read it into a temp folder, and the
   same helpers measure it, so the gate and this file always count the same
   bytes.
2. **The recorded side** is the four machine-readable lines at the top of this
   file. `tools/workspace-check.mjs` reads them with `readBaseline`, measures
   the committed tree with `baselineCounts` and compares them with
   `baselineDrift`.

The audit prints one PASS line:

```
PASS baseline: 43 crates, 888420 src lines, 80 multi-version lock names, 0 dead feature flags
```

and fails with the old and the new value when a number moved:

```
FAIL baseline: src lines 888420 -> 888517
```

The audit then exits 1 and says which file to record. The dead feature flags
are a WARN, not a failure: cutting one is the point of the compaction work,
and this file is re-recorded with the size change.

**The workflow for a size change.** Land the change, then run
`node tools/crate-map.mjs --baseline` and commit the four numbers with it.
The doc update is the commit after the size change, because the numbers are
the ones of the committed tree.

Proofs: `node --test tools/crate-map.test.mjs` (7 pass),
`node --test tools/workspace-check.test.mjs` (67 pass),
`node tools/crate-map.mjs --check` prints `PASS docs/CRATE-MAP.md`.

---

## 3. The deletion manifest

Every row: the crate's own test must stay green and the golden
`d57b5c8ae3096614` must not move. The proof command is
`node tools/gh-check.mjs cargo test -p <crate> --lib`, which prints
`PASS on the runner: cargo test -p <crate> --lib (run <url>)` and exits 0.

| # | path | lines | evidence (must print this) | crate test that stays green |
|---|---|---|---|---|
| D1 | crates/nav/src/orca.rs | 1,252 | `git grep -n "orca::" -- crates/` = 0 hits | `cargo test -p engine-nav --lib` |
| D2 | crates/render/src/heal.rs | 1,137 | `git grep -n "RenderHeal::new" -- crates/` = 0 hits outside the file | `cargo test -p engine-render --lib` |
| D3 | crates/observe/src/rerun_sink.rs | 437 | `git grep -n "RerunSink::" -- crates/` = 0 hits outside the file | `cargo test -p engine-observe --lib` |
| D4 | crates/ai/src/behavior.rs | 275 | `git grep -n "behavior::" -- crates/` = 0 hits after the cut | `cargo test -p engine-ai --lib` |
| **total** | | **3,101** | | |

Each cut also touches the owning crate's `src/lib.rs` (the `pub mod` line) and
the test file that covered the module. Record the new numbers at the top of
this file with the cut (the drift gate would otherwise fail).

**Do not cut** (live code): `crates/ai/src/utility.rs`,
`crates/anim/src/lod.rs`, `crates/audio/src/crowd.rs`,
`crates/assets/src/import_repair.rs`, `crates/audio/src/heal.rs`.

---

## 4. Corrections to the stale audit (three + one doc line)

These three are in `docs/plan/forge-adoption.md` section A.3 item 5 (and
item 3). Other lanes repeated them from that stale list, so they are
repeated here with the live proof. **The A.3 list is stale on these three
points and needs a dated correction.**

**(a) `crates/assets/import_repair` IS wired.** The caller is
`crates/render/src/v1.rs:256`:

```
let tangents = engine_assets::import_repair::recompute_tangents(&proxy);
```

and a second one at `crates/render/src/present/gpu_present/presenter.rs:2197`
(same call). `crates/render/Cargo.toml:54` says the same in prose. The path
`crates/render/src/present/gpu_present/v1.rs` does **not** exist
(`Test-Path` prints False); the real file is `crates/render/src/v1.rs`.
`crates/assets/src/import_repair.rs` is 1,310 lines and live.

**(b) `engine-playtest` HAS a consumer.** `crates/endpoint/Cargo.toml:16`
puts `dep:engine-playtest` in the endpoint's `mcp` feature, line 32 defines
`verify = ["mcp", "dep:engine-proof"]`, and line 79 declares
`engine-playtest = { workspace = true, optional = true }`. The call site is
`crates/endpoint/src/packs/verify.rs:34-37`:

```
use engine_playtest::mirror::{run_mirror, MirrorReport};
use engine_playtest::mirror_kit::MirrorRow;
use engine_playtest::replay::{bisect_tape, verify_tape, ReplayError};
use engine_playtest::seeded_bug::{...};
```

plus `crates/endpoint/src/look_host.rs:7` and
`crates/endpoint/src/packs/view.rs:49`. One stale comment survives and must
be fixed: `crates/endpoint/src/assistant/fork.rs:19` still says
"`engine-playtest` stays outside the endpoint dep edge".

**(c) There is NO `nav::crowd` in the tree.** Zero-hit proof:

```
git grep -n "nav::crowd" -- crates/       -> no output, exit 1
git grep -n "engine_nav::crowd" -- crates/ -> no output, exit 1
```

`crates/nav/src/` holds 14 files and no `crowd.rs`: bake.rs, cursor.rs,
flow.rs, funnel.rs, layers.rs, lib.rs, links.rs, mesh.rs, orca.rs, quant.rs,
reach.rs, rebake.rs, spline.rs, tiles.rs. The ORCA code the audit meant is
`crates/nav/src/orca.rs` (1,252 lines, zero callers - row D1). The shared cue
type is `crates/audio/src/crowd.rs`, which is live.

**Doc line that must change (`docs/SMOOTH-BUILD.md`, "Build speed facts") and
`docs/SPRINT-13-engine-edge.md:34` (the OPT8 progress note).** They said
"16 syn, 12 serde_json, 14 num_traits copies in target/debug/deps before the
hack". Those were **build-artifact counts from 2026-10-17**, not `Cargo.lock`
facts. The corrected line:

> `Cargo.lock` holds 1,038 packages; 80 names carry more than one version and
> 101 extra copies are compiled. `serde_json` is one version (1.0.151),
> `num-traits` is one version (0.2.19) and is not a direct dependency of any
> crate, and `syn` is three versions (1.0.109, 2.0.119, 3.0.6). The
> "16 / 12 / 14" figures counted files in `target/debug/deps`, which drift
> with every build.

Status 2026-10-21: this correction is in the tree.
