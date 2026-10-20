# Compaction baseline - the measured size of engine2040 and of an agent's context

Measured on 2026-10-20 at HEAD `77789c2a` (the tree moves every wave; the
numbers below are a snapshot, the commands reproduce them).

**Machine-readable baseline** (the drift gate in SPEC B reads these four
lines; update them in the same commit as any size change):

```
- crates: 42
- src lines: 816611
- multi-version lock names: 80
- dead feature flags: 7
```

**Line definition used everywhere in this file:** physical lines, blank lines
included (the count `wc -l` gives). Where a second number matters it is
labelled *non-blank*. Two counts exist because two tools count differently:
`Get-Content | Measure-Object -Line` skips blank lines, a node
`split("\n").length - 1` does not. The baseline uses the second.

**How to reproduce the crate and file numbers:**

```
git archive HEAD crates | tar -x -C <tmp>
node <script that walks <tmp>/crates/*/src/*.rs and counts split("\n").length - 1>
```

**How to reproduce the lock numbers:**

```
git show HEAD:Cargo.lock > <tmp>/Cargo.lock
```

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
| workspace members | 43 (42 library crates under `crates/*` + the root package `engine2040`) |
| crates with a `src/` | 42 |
| `crates/*/src` .rs files | 1,153 |
| `crates/*/src` lines (all) | **816,611** |
| `crates/*/src` lines (non-blank) | 771,870 |
| `crates/**` .rs files (with `tests/`) | 1,358 |
| `crates/**` lines (all) | 861,384 |
| top 3 crates | render + endpoint + game = 270,098 lines = 33.1% |

### 1.2 Lines per crate (`crates/<c>/src`, all physical lines)

| crate | files | lines | non-blank | % of src |
|---|---|---|---|---|
| render | 139 | 113,519 | 107,807 | 13.9% |
| endpoint | 118 | 78,701 | 74,503 | 9.6% |
| game | 82 | 77,878 | 73,997 | 9.5% |
| net | 66 | 52,588 | 49,483 | 6.4% |
| assets | 58 | 41,307 | 39,065 | 5.1% |
| playtest | 57 | 40,147 | 37,798 | 4.9% |
| editor | 40 | 38,202 | 36,008 | 4.7% |
| anim | 61 | 37,220 | 35,332 | 4.6% |
| spatial | 44 | 36,587 | 34,773 | 4.5% |
| ai | 37 | 26,008 | 24,503 | 3.2% |
| physics | 26 | 25,571 | 24,300 | 3.1% |
| combat | 27 | 23,741 | 22,515 | 2.9% |
| provider | 31 | 21,984 | 20,623 | 2.7% |
| arena | 24 | 20,276 | 19,239 | 2.5% |
| audio | 25 | 19,989 | 18,770 | 2.4% |
| proof | 30 | 16,947 | 16,062 | 2.1% |
| destruction | 25 | 16,172 | 15,381 | 2.0% |
| perf | 18 | 12,246 | 11,515 | 1.5% |
| ecs | 16 | 11,534 | 10,957 | 1.4% |
| ui | 9 | 10,792 | 10,105 | 1.3% |
| material | 17 | 9,953 | 9,341 | 1.2% |
| nav | 14 | 9,291 | 8,795 | 1.1% |
| web | 20 | 9,247 | 8,606 | 1.1% |
| distill | 12 | 6,261 | 5,901 | 0.8% |
| core | 24 | 6,241 | 5,795 | 0.8% |
| scene | 10 | 6,039 | 5,611 | 0.7% |
| physfx | 25 | 6,002 | 5,643 | 0.7% |
| observe | 8 | 5,928 | 5,490 | 0.7% |
| geo | 13 | 5,468 | 5,161 | 0.7% |
| kam2-game | 14 | 5,400 | 5,041 | 0.7% |
| rules | 11 | 4,935 | 4,646 | 0.6% |
| world | 9 | 4,526 | 4,288 | 0.6% |
| app | 10 | 3,575 | 3,325 | 0.4% |
| input | 7 | 2,761 | 2,547 | 0.3% |
| kam2-sim | 6 | 2,528 | 2,375 | 0.3% |
| ossim | 6 | 2,407 | 2,263 | 0.3% |
| nn | 3 | 2,136 | 2,009 | 0.3% |
| save | 4 | 1,675 | 1,556 | 0.2% |
| kam2-web | 4 | 593 | 537 | 0.1% |
| clock | 1 | 138 | 117 | 0.0% |
| engine-chaos | 1 | 96 | 85 | 0.0% |
| workspace-hack | 1 | 2 | 2 | 0.0% |

### 1.3 The 20 largest files (74,176 lines = 9.1% of the tree)

| # | file | lines |
|---|---|---|
| 1 | crates/game/src/fighter_look.rs | 6,288 |
| 2 | crates/arena/src/lib.rs | 5,326 |
| 3 | crates/game/src/bin/arena.rs | 4,775 |
| 4 | crates/render/src/ibl.rs | 4,409 |
| 5 | crates/combat/src/fight.rs | 3,960 |
| 6 | crates/game/src/lib.rs | 3,896 |
| 7 | crates/render/src/gtao.rs | 3,486 |
| 8 | crates/game/src/windowed.rs | 3,472 |
| 9 | crates/game/src/camera.rs | 3,462 |
| 10 | crates/editor/src/commands.rs | 3,441 |
| 11 | crates/endpoint/src/scene_tools.rs | 3,419 |
| 12 | crates/render/src/present/gpu_present/presenter.rs | 3,380 |
| 13 | crates/physics/src/ragdoll.rs | 3,376 |
| 14 | crates/spatial/src/relation.rs | 3,250 |
| 15 | crates/render/src/present/gpu_present/submit.rs | 3,221 |
| 16 | crates/ai/src/depth.rs | 3,143 |
| 17 | crates/render/src/present/gpu_present/live.rs | 3,068 |
| 18 | crates/endpoint/src/discover.rs | 2,947 |
| 19 | crates/ui/src/lib.rs | 2,932 |
| 20 | crates/endpoint/src/build_level.rs | 2,925 |

### 1.4 The lock: 80 names with more than one version, 101 extra copies

`Cargo.lock` at `77789c2a`: **1,032 packages**, **80 names carry more than one
version**, **101 extra version copies** are compiled.

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
| bit-set | 3 | 0.5.3 0.9.1 0.10.0 |
| getrandom | 3 | 0.2.17 0.3.4 0.4.3 |
| itertools | 3 | 0.13.0 0.14.0 0.15.0 |
| rand | 3 | 0.8.8 0.9.5 0.10.3 |
| rand_core | 3 | 0.6.4 0.9.5 0.10.1 |
| redox_syscall | 3 | 0.4.1 0.5.18 0.9.4 |
| syn | 3 | 1.0.109 2.0.119 3.0.6 |
| bit-vec, bitflags, block-buffer, block2, calloop, calloop-wayland-source, constant_time_eq, convert_case, core-foundation, cpufeatures, crypto-common, digest, downcast-rs, embedded-hal, fixedbitset, foldhash, jni-sys, libloading, linux-raw-sys, miniz_oxide, naga, nb, ndarray, nix, nom, num-derive, objc2, objc2-app-kit, objc2-foundation, objc2-metal, objc2-quartz-core, objc2-ui-kit, ordered-float, pastey, r-efi, rand_chacha, rand_distr, rustc-hash, rustix, safetensors, sha2, shlex, signal-hook, smithay-client-toolkit, synstructure, thiserror, thiserror-impl, webpki-roots, wgpu, wgpu-core, wgpu-core-deps-apple, wgpu-core-deps-emscripten, wgpu-core-deps-windows-linux-android, wgpu-hal, wgpu-naga-bridge, wgpu-types, windows_aarch64_gnullvm, windows_aarch64_msvc, windows_i686_gnu, windows_i686_gnullvm, windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm, windows_x86_64_msvc, windows-targets, zip | 2 each | see `Cargo.lock` |

`serde_json` = **one** version (1.0.151). `num-traits` = **one** version
(0.2.19) and is **not** a direct dependency of any crate.

### 1.5 Feature flags: 54 declared, 7 dead, 7 weak

Dead = declared, no other crate requests it, no `cfg(feature = ...)` in its
own crate.

| crate | dead feature | body |
|---|---|---|
| engine-endpoint | `tls` | `["dep:engine-provider", "engine-provider/tls"]` |
| engine-geo | `default` | `[]` |
| engine-geo | `kit` | `["dep:engine-assets", "engine-assets/import"]` |
| engine-geo | `all` | `["boolean", "uv", "lod", "collision", "bake", "kit", "fbx"]` |
| kam2-game | `window` | `["dep:winit", "dep:wgpu", "dep:pollster", "dep:image", "engine-render/window"]` |
| engine-playtest | `tls` | `["engine-provider/tls"]` |
| engine-scene | `math` | `[]` |

Weak = no external request, exactly one `cfg` use: `engine-core/test-util`,
`engine-geo/bake`, `engine-geo/fbx`, `engine-proof/arena`,
`engine-proof/spine`, `engine-proof/ragdoll`, `engine-rules/arena`.

### 1.6 Dead lines: 4,334 lines no production path reaches

| file | lines (all) | non-blank | evidence | verdict |
|---|---|---|---|---|
| crates/nav/src/orca.rs | 971 | 910 | `git grep -n "orca::" -- crates/` = 0 hits; `git grep -n "Orca" -- crates/` = 0 hits outside the file | dead: 0 callers |
| crates/render/src/heal.rs | 1,090 | 1,014 | `RenderHeal` only in its own file; `crates/render/src/present/surface.rs:252` is a doc link (`/// Consumer: [`crate::heal::RenderHeal`]`) | no production constructor |
| crates/audio/src/heal.rs | 749 | 686 | `DeviceHeal::new` only in `crates/game/src/audio_mix.rs:390`, inside `#[cfg(test)] mod tests` (`crates/game/src/audio_mix.rs:171`) | no production constructor |
| crates/physics/src/kcc.rs | 928 | 925 | `git grep -n "kcc::" -- crates/` = 0 hits | dead: 0 callers |
| crates/observe/src/rerun_sink.rs | 321 | 297 | `RerunSink` only in `crates/observe/src/timeline.rs` (doc links) and `crates/observe/tests/it/w1_pf9_rerun.rs:9` | no outside caller |
| crates/ai/src/behavior.rs | 275 | 260 | `git grep -n "behavior::" -- crates/` = 2 hits, both inside engine-ai (`crates/ai/src/lib.rs:23` re-export, `crates/ai/src/gladiator.rs:12` doc comment); `engine_ai::Behavior`, `engine_ai::tick`, `engine_ai::Status` = 0 hits each | dead: re-exported, never called |
| **total** | **4,334** | 4,090 | | |

**Live, do not delete:** `crates/ai/src/utility.rs` (354 lines, used by
`crates/ai/src/gladiator.rs:144` and `crates/ai/src/personality.rs:155`),
`crates/anim/src/lod.rs` (194 lines, used by `crates/anim/src/crowd_scale.rs:10`),
`crates/audio/src/crowd.rs` (512 lines, six intra-crate callers plus
`crates/game/src/audio_director.rs:17`), `crates/assets/src/import_repair.rs`
(1,310 lines, wired - see section 6).

### 1.7 Disk

| item | value |
|---|---|
| `target/debug` | 84,947 files, 30,974,305,957 bytes (28.9 GiB) |
| tracked `.profraw` in git | **91 files, 55,519,832 bytes** (90 at the repo root, 1 in `crates/game/`) |

### 1.8 Docs

| item | value |
|---|---|
| `docs/` | 1,207 files, 54,519,106 bytes |
| `docs/` .md only | 627 files, 9,207,076 bytes |
| `docs/archive/` | 851 files, 41,803,786 bytes = **76.7% of all docs bytes** |
| `docs/plan/` | 70 files, 1,170,549 bytes |
| `docs/research/` | 109 files, 1,662,692 bytes |
| `docs/plan/kam2-ui.md` | not listed in `docs/plan/README.md` |

### 1.9 Agent context (tokens = bytes / 4)

| file | lines | bytes | approx tokens |
|---|---|---|---|
| llms.txt | 41 | 3,422 | 856 |
| AGENTS.md | 34 | 5,679 | 1,420 |
| docs/book/for-ai.md | 45 | 6,047 | 1,512 |
| docs/CRATE-MAP.md | 48 | 8,037 | 2,009 |
| .opencode/repomap.md | 483 | 15,866 | 3,966 |
| docs/plan/README.md | 177 | 19,759 | 4,940 |
| **entry set total** | | | **~14,703** |
| .opencode/modmap.md (whole) | 1,397 | 144,615 | 36,154 |

One crate's modmap slice (the map is grep-only, never read whole): render
4,068 tok (11.3% of the map), endpoint 3,810, game 2,652, assets 2,289, net
2,136, anim 1,561, playtest 1,540, editor 1,191, spatial 1,153, ai 973, perf
971, physics 947, combat 843, proof 842, audio 820, provider 810, core 797.
Top 3 crates = 10,529 tok = 29.1% of the map. A packet for one crate (map
slice + CRATE-MAP row + plan line + board row) must stay **under 6,000
tokens**.

---

## 2. SPEC A - extend `tools/crate-map.mjs` (one tool, not a second tool)

Do **not** create `tools/crate-stats.mjs`. The project rule is no zoo: never
add a copy, a variant, a twin. `tools/crate-map.mjs` already walks every
`crates/<dir>` and reads every `Cargo.toml`; the line counts, the lock
version counts and the dead-feature count belong there, and
`tools/workspace-check.mjs` calls them.

All of this is a **spec** - the lead applies it. Nothing below was applied by
the lane that measured it.

**A1. New import.** Line 6 stays as it is (`readdirSync` with
`{ withFileTypes: true }` is enough for a recursive walk); no new import is
needed.

**A2. Add after `export const dependencyNames = ...` (after line 33):**

```js
// Physical lines of Rust under crates/<dir>/src (blank lines included, the
// count `wc -l` gives). One walk per crate. Pure helper for tests.
export const sourceLines = (cratesDir, dir) => {
  const src = join(cratesDir, dir, "src");
  if (!existsSync(src)) return { files: 0, lines: 0 };
  let files = 0;
  let lines = 0;
  const walk = (p) => {
    for (const e of readdirSync(p, { withFileTypes: true })) {
      const q = join(p, e.name);
      if (e.isDirectory()) walk(q);
      else if (e.name.endsWith(".rs")) {
        files += 1;
        lines += readFileSync(q, "utf8").split("\n").length - 1;
      }
    }
  };
  walk(src);
  return { files, lines };
};
```

**A3. Add after `export const instability = ...` (after line 61):**

```js
// Multi-version names in Cargo.lock: what `cargo tree --duplicates` prints,
// without calling cargo. One pass over the lock.
export const lockVersions = (root = ROOT) => {
  const lockPath = join(root, "Cargo.lock");
  if (!existsSync(lockPath)) return { packages: 0, names: 0, copies: 0, byName: [] };
  const versions = new Map();
  let packages = 0;
  for (const block of readFileSync(lockPath, "utf8").split("[[package]]").slice(1)) {
    const name = /name = "([^"]+)"/.exec(block)?.[1];
    const version = /version = "([^"]+)"/.exec(block)?.[1];
    if (!name || !version) continue;
    packages += 1;
    if (!versions.has(name)) versions.set(name, new Set());
    versions.get(name).add(version);
  }
  const byName = [...versions.entries()]
    .filter(([, v]) => v.size > 1)
    .map(([name, v]) => ({ name, copies: v.size, versions: [...v].sort() }))
    .sort((a, b) => b.copies - a.copies || a.name.localeCompare(b.name));
  return { packages, names: byName.length, copies: byName.reduce((a, r) => a + r.copies - 1, 0), byName };
};

// Feature flags that no other crate requests and that their own crate never
// reads with cfg(feature = ...). One pass over crates/*/Cargo.toml plus one
// pass over crates/*/src. Pure helper for tests.
export const deadFeatures = (root = ROOT) => {
  const cratesDir = join(root, "crates");
  const dirs = existsSync(cratesDir)
    ? readdirSync(cratesDir).filter((d) => existsSync(join(cratesDir, d, "Cargo.toml"))).sort()
    : [];
  const pkgOf = new Map();
  for (const d of dirs) pkgOf.set(d, packageName(readFileSync(join(cratesDir, d, "Cargo.toml"), "utf8")));
  const declared = new Map();   // "pkg/feature" -> body
  for (const d of dirs) {
    const text = readFileSync(join(cratesDir, d, "Cargo.toml"), "utf8");
    let inFeatures = false;
    for (const raw of text.replace(/\r\n/g, "\n").split("\n")) {
      const line = raw.trim();
      if (/^\[/.test(line)) { inFeatures = line === "[features]"; continue; }
      if (!inFeatures) continue;
      const m = /^([A-Za-z0-9_.-]+)\s*=\s*(.*)$/.exec(line);
      if (m) declared.set(`${pkgOf.get(d)}/${m[1]}`, m[2]);
    }
  }
  const requested = new Set();  // "pkg/feature" asked for by another crate
  for (const d of dirs) {
    const text = readFileSync(join(cratesDir, d, "Cargo.toml"), "utf8");
    for (const m of text.matchAll(/^\s*([A-Za-z0-9_-]+)\s*=\s*\{([^}]*)\}/gm)) {
      const fm = /features\s*=\s*\[([^\]]*)\]/.exec(m[2]);
      if (!fm) continue;
      for (const f of fm[1].split(",").map((s) => s.trim().replace(/^"|"$/g, "")).filter(Boolean)) {
        requested.add(`${m[1]}/${f}`);
      }
    }
  }
  const used = new Set();       // "pkg/feature" read with cfg(feature = ...)
  for (const d of dirs) {
    const src = join(cratesDir, d, "src");
    if (!existsSync(src)) continue;
    const walk = (p) => {
      for (const e of readdirSync(p, { withFileTypes: true })) {
        const q = join(p, e.name);
        if (e.isDirectory()) walk(q);
        else if (e.name.endsWith(".rs")) {
          for (const m of readFileSync(q, "utf8").matchAll(/feature\s*=\s*"([^"]+)"/g)) {
            used.add(`${pkgOf.get(d)}/${m[1]}`);
          }
        }
      }
    };
    walk(src);
  }
  const dead = [];
  for (const [feature, body] of declared) {
    if (!requested.has(feature) && !used.has(feature)) dead.push({ feature, body });
  }
  dead.sort((a, b) => a.feature.localeCompare(b.feature));
  return dead;
};
```

**A4. In `crateRows` (lines 38-52), add the line counts to every row:**

```js
    const src = sourceLines(crates, d);
    return { dir: d, name: packageName(toml), job: clip(job, JOB_MAX).replace(/\|/g, "/"), deps: dependencyNames(toml), srcFiles: src.files, srcLines: src.lines };
```

**A5. In `renderCrateMap` (lines 66-77), add two columns and one legend
line:**

```js
export const renderCrateMap = (rows) => [
  "# Crate map",
  "",
  "Generated by `node tools/crate-map.mjs` from each `crates/<dir>/Cargo.toml` and its `lib.rs` `//!` line. Do not edit by hand.",
  "`Depends on` = engine crates in `[dependencies]`; `Used by` = the reverse. Both drop the `engine-` prefix.",
  "`Instability` = Ce / (Ca + Ce): fan-out over fan-in plus fan-out. 0 is stable, 1 is unstable.",
  "`src files` / `src lines` = the .rs files under `crates/<dir>/src` and their physical lines (blank lines included).",
  "",
  "| Package | Dir | Job (lib.rs) | Depends on | Used by | Instability | src files | src lines |",
  "|---|---|---|---|---|---|---|---|",
  ...rows.map((r) => `| \`${r.name}\` | \`crates/${r.dir}\` | ${r.job} | ${list(r.deps)} | ${list(r.usedBy)} | ${instability(r).toFixed(2)} | ${r.srcFiles} | ${r.srcLines} |`),
  "",
].join("\n");
```

**A6. Add a `--baseline` mode to the CLI (lines 81-94):** widen the usage
guard to `!["--check", "--baseline"].includes(args[0])` and add, before the
`--check` branch:

```js
  if (args[0] === "--baseline") {
    const rows = crateRows();
    const lock = lockVersions();
    const dead = deadFeatures();
    const srcLines = rows.reduce((a, r) => a + r.srcLines, 0);
    console.log(`crates: ${rows.length}`);
    console.log(`src lines: ${srcLines}`);
    console.log(`lock packages: ${lock.packages}`);
    console.log(`multi-version lock names: ${lock.names}`);
    console.log(`extra version copies: ${lock.copies}`);
    console.log(`dead feature flags: ${dead.length}`);
    for (const r of lock.byName) console.log(`  ${r.name} (${r.copies}): ${r.versions.join(" ")}`);
    for (const f of dead) console.log(`  dead feature: ${f.feature}`);
    process.exit(0);
  }
```

**A7. After the change**, `docs/CRATE-MAP.md` gains two columns: the lead
runs `node tools/crate-map.mjs` once and commits `docs/CRATE-MAP.md` with the
change, so `node tools/crate-map.mjs --check` still prints
`PASS docs/CRATE-MAP.md`.

**A8. Tests, in the existing `tools/crate-map.test.mjs` (no new test file):**
`sourceLines` counts a temp `crates/<d>/src` tree; `lockVersions` counts a
temp `Cargo.lock` (2 names, 1 extra copy); `deadFeatures` finds one dead and
one used feature in a temp tree; `crateRows` returns `srcFiles` and
`srcLines` for every row.

---

## 3. SPEC B - the `tools/workspace-check.mjs` gate lines

**Do not edit `tools/workspace-check.mjs` from this lane** - the speedup lane
may be in there. This is the exact patch the lead applies.

**B1. One import, next to the existing ones (after line 12):**

```js
import { crateRows, deadFeatures, lockVersions } from "./crate-map.mjs";
```

**B2. The gate block.** Paste it into `audit(repoDir)` immediately **before**
the line `if (errors.length) throw new Error(errors.join("\n"));`:

```js
  // CMP01 baseline drift gate (lane 09, 2026-10-20): the measured size of the
  // engine must not move without docs/compaction-baseline.md moving with it.
  // The counts come from tools/crate-map.mjs, so there is one tool, not a
  // zoo, and no second walk of the tree.
  let baselineCrates = 0;
  let baselineSrcLines = 0;
  let baselineLockNames = 0;
  let baselineDeadFlags = 0;
  try {
    const basePath = join(repoDir, "docs", "compaction-baseline.md");
    const base = existsSync(basePath) ? readFileSync(basePath, "utf8") : "";
    const rows = crateRows(repoDir);
    const lock = lockVersions(repoDir);
    const dead = deadFeatures(repoDir);
    const srcLines = rows.reduce((a, r) => a + (r.srcLines ?? 0), 0);
    const want = {
      crates: Number(/^- crates: (\d+)$/m.exec(base)?.[1] ?? -1),
      srcLines: Number(/^- src lines: (\d+)$/m.exec(base)?.[1] ?? -1),
      lockNames: Number(/^- multi-version lock names: (\d+)$/m.exec(base)?.[1] ?? -1),
      deadFlags: Number(/^- dead feature flags: (\d+)$/m.exec(base)?.[1] ?? -1),
    };
    baselineCrates = rows.length;
    baselineSrcLines = srcLines;
    baselineLockNames = lock.names;
    baselineDeadFlags = dead.length;
    if (want.crates !== rows.length) errors.push(`baseline: crates ${want.crates} -> ${rows.length}`);
    if (want.srcLines !== srcLines) errors.push(`baseline: src lines ${want.srcLines} -> ${srcLines}`);
    if (want.lockNames !== lock.names) errors.push(`baseline: multi-version lock names ${want.lockNames} -> ${lock.names}`);
    // The dead flags are a warning, not a failure: deleting one is the point
    // of row CMP-08, and the lead updates this file in the same commit.
    if (want.deadFlags !== dead.length) {
      console.log(`WARN baseline: dead feature flags ${want.deadFlags} -> ${dead.length}: update docs/compaction-baseline.md in the same commit`);
    }
  } catch (error) {
    errors.push(`baseline drift gate could not run: ${error.message}`);
  }
```

**B3. The PASS line.** Add it with the other PASS lines, after
`console.log(\`PASS maps: ...\`)`:

```js
  console.log(`PASS baseline: ${baselineCrates} crates, ${baselineSrcLines} src lines, ${baselineLockNames} multi-version lock names, ${baselineDeadFlags} dead feature flags`);
```

**B4. The baseline file must carry the four machine-read lines** so the
regexes above match. Put this block near the top of
`docs/compaction-baseline.md`:

```
- crates: 42
- src lines: 816611
- multi-version lock names: 80
- dead feature flags: 7
```

**B5. Expected output after the patch:**

```
PASS baseline: 42 crates, 816611 src lines, 80 multi-version lock names, 7 dead feature flags
```

and when a number moves without the baseline moving:

```
FAIL workspace audit: baseline: src lines 816611 -> 816700
```

**B6.** `node --test tools/workspace-check.test.mjs` still passes; the gate
adds no `cargo` call and no second tree walk (the counts come from the
in-process helpers).

---

## 4. SPEC C - the 91 `.profraw` files: `.gitignore` line and the index command

**C1. The exact `.gitignore` line** (the lead appends it; the lane did not
touch `.gitignore`):

```
# CMP10: cargo profile artifacts are build output, never tracked (91 were, 55.5 MB).
*.profraw
```

**C2. The exact command** (the lead runs it; the lane never touched the
index):

```
git rm --cached -- "*.profraw"
```

Verified read-only on 2026-10-20:
`git rm --cached -n -- "*.profraw"` prints exactly **91** `rm '<path>'` lines
(the list below), and `git ls-files "*.profraw"` prints 91 paths.

**C3. The 91 paths** (`git ls-files "*.profraw"`):

```
crates/game/default_6592166020663119231_0_21600.profraw
default_11531410208419812330_0_14908.profraw
default_11531410208419812330_0_21932.profraw
default_11531410208419812330_0_29260.profraw
default_11531410208419812330_0_32408.profraw
default_12114940404156725336_0_14908.profraw
default_12114940404156725336_0_21932.profraw
default_12114940404156725336_0_29260.profraw
default_12114940404156725336_0_32408.profraw
default_12766043644524746671_0_14908.profraw
default_12766043644524746671_0_21932.profraw
default_12766043644524746671_0_29260.profraw
default_12766043644524746671_0_32408.profraw
default_13151844070501877706_0_14908.profraw
default_13151844070501877706_0_21932.profraw
default_13151844070501877706_0_25864.profraw
default_13151844070501877706_0_29260.profraw
default_13151844070501877706_0_32408.profraw
default_14592561836793919282_0_14908.profraw
default_14592561836793919282_0_21932.profraw
default_14592561836793919282_0_29260.profraw
default_14592561836793919282_0_32408.profraw
default_15407677909019594497_0_14908.profraw
default_15407677909019594497_0_22912.profraw
default_15407677909019594497_0_32408.profraw
default_15407677909019594497_0_34352.profraw
default_15407677909019594497_0_34684.profraw
default_15407677909019594497_0_35944.profraw
default_15407677909019594497_0_9272.profraw
default_15421558880541078991_0_14908.profraw
default_15421558880541078991_0_21932.profraw
default_15421558880541078991_0_22912.profraw
default_15421558880541078991_0_29260.profraw
default_15421558880541078991_0_32408.profraw
default_15421558880541078991_0_34352.profraw
default_15421558880541078991_0_34684.profraw
default_15421558880541078991_0_35944.profraw
default_15421558880541078991_0_9272.profraw
default_15538312708823871364_0_14908.profraw
default_15538312708823871364_0_21932.profraw
default_15538312708823871364_0_22912.profraw
default_15538312708823871364_0_25864.profraw
default_15538312708823871364_0_29260.profraw
default_15538312708823871364_0_32408.profraw
default_15538312708823871364_0_34352.profraw
default_15538312708823871364_0_34684.profraw
default_15538312708823871364_0_35944.profraw
default_15538312708823871364_0_9272.profraw
default_15763605578570292452_0_14908.profraw
default_15763605578570292452_0_21932.profraw
default_15763605578570292452_0_22912.profraw
default_15763605578570292452_0_25864.profraw
default_15763605578570292452_0_29260.profraw
default_15763605578570292452_0_32408.profraw
default_15763605578570292452_0_34352.profraw
default_15763605578570292452_0_34684.profraw
default_15763605578570292452_0_35944.profraw
default_15763605578570292452_0_9272.profraw
default_15999050843751615430_0_14908.profraw
default_15999050843751615430_0_21932.profraw
default_15999050843751615430_0_29260.profraw
default_15999050843751615430_0_32408.profraw
default_16628792701168341809_0_14908.profraw
default_16628792701168341809_0_21932.profraw
default_16628792701168341809_0_29260.profraw
default_16628792701168341809_0_32408.profraw
default_4151401301713125766_0_14908.profraw
default_4151401301713125766_0_21932.profraw
default_4151401301713125766_0_29260.profraw
default_4151401301713125766_0_32408.profraw
default_4526706205214488633_0_14908.profraw
default_4526706205214488633_0_22912.profraw
default_4526706205214488633_0_32408.profraw
default_4526706205214488633_0_34352.profraw
default_4526706205214488633_0_34684.profraw
default_4526706205214488633_0_35944.profraw
default_4526706205214488633_0_9272.profraw
default_5217871785964836725_0_14908.profraw
default_5217871785964836725_0_21932.profraw
default_5217871785964836725_0_25840.profraw
default_5217871785964836725_0_32408.profraw
default_6393772639977521661_0_14908.profraw
default_6393772639977521661_0_21932.profraw
default_6393772639977521661_0_25840.profraw
default_6393772639977521661_0_32408.profraw
default_8070615554531034728_0_14908.profraw
default_8070615554531034728_0_21932.profraw
default_8070615554531034728_0_29260.profraw
default_8070615554531034728_0_32408.profraw
default_874140065828270945_0_14908.profraw
default_874140065828270945_0_21932.profraw
default_874140065828270945_0_29260.profraw
default_874140065828270945_0_32408.profraw
```

**C4. The disk half (E86):** `node tools/clean.mjs --dry-run` then
`node tools/clean.mjs --auto --include-debug`. The dry run must list no
`target/debug` entry afterwards; anything removed goes to
`C:\_quarantine\<date>\` with a `MOVES.tsv` line, never a hard delete.

---

## 5. SPEC D - the deletion manifest (the lead applies it, one crate at a time)

Every row: the crate's own test must stay green and the golden
`d57b5c8ae3096614` must not move. The proof command is
`node tools/gh-check.mjs cargo test -p <crate> --lib`, which prints
`PASS on the runner: cargo test -p <crate> --lib (run <url>)` and exits 0.
The golden check is `cargo run --locked -- --verify`, which prints
`final_hash=d57b5c8ae3096614` and `verified=true`, twice.

| # | path | lines | evidence (must print this) | crate test that stays green | golden |
|---|---|---|---|---|---|
| D1 | crates/nav/src/orca.rs | 971 | `git grep -n "orca::" -- crates/` = 0 hits | `cargo test -p engine-nav --lib` | unchanged |
| D2 | crates/render/src/heal.rs | 1,090 | `git grep -n "RenderHeal::new" -- crates/` = 0 hits | `cargo test -p engine-render --lib` | unchanged |
| D3 | crates/audio/src/heal.rs | 749 | `git grep -n "DeviceHeal::new" -- crates/` = 0 hits outside `#[cfg(test)]` | `cargo test -p engine-audio --lib` | unchanged |
| D4 | crates/physics/src/kcc.rs | 928 | `git grep -n "kcc::" -- crates/` = 0 hits | `cargo test -p engine-physics --lib` | unchanged |
| D5 | crates/observe/src/rerun_sink.rs | 321 | `git grep -n "RerunSink::" -- crates/` = 0 hits outside `crates/observe/src/rerun_sink.rs` | `cargo test -p engine-observe --lib` | unchanged |
| D6 | crates/ai/src/behavior.rs | 275 | `git grep -n "behavior::" -- crates/` = 0 hits after the cut (2 today, both inside engine-ai: `crates/ai/src/lib.rs:23`, `crates/ai/src/gladiator.rs:12`); `engine_ai::Behavior` / `engine_ai::tick` / `engine_ai::Status` = 0 hits | `cargo test -p engine-ai --lib` | unchanged |
| **total** | | **4,334** | | | |

Each cut also touches the owning crate's `src/lib.rs` (the `pub mod` line) and
the test file that covered the module; the row lists the owned paths in
`docs/research/powerups-2026-10-20/proposed-rows-09-code-compaction.md`.

Order the lead should follow: one crate at a time, the crate's test green
before the next crate starts, the golden hash checked after each crate, and
`docs/compaction-baseline.md` updated in the same commit (the drift gate
would otherwise fail).

**Do not cut** (live code, verified 2026-10-20): `crates/ai/src/utility.rs`,
`crates/anim/src/lod.rs`, `crates/audio/src/crowd.rs`,
`crates/assets/src/import_repair.rs`.

**Do not drop** (they are live, only the *flag* is dead): the `engine-provider`
`tls` feature, the `engine-geo` `boolean` / `uv` / `lod` / `collision` /
`bake` / `fbx` features, the `engine-core` `test-util` feature, the
`engine-proof` `arena` / `spine` / `ragdoll` features, the `engine-rules`
`arena` feature. The dead ones are the seven in section 1.5 and nothing else.

---

## 6. Corrections to the stale audit (three + one doc line)

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
puts `dep:engine-playtest` in the endpoint's `mcp` feature; line 32 defines
`verify = ["mcp", "dep:engine-proof"]`; line 79 declares
`engine-playtest = { workspace = true, optional = true }`. The call site is
`crates/endpoint/src/packs/verify.rs:34-37`:

```
use engine_playtest::mirror::{run_mirror, MirrorReport};
use engine_playtest::mirror_kit::MirrorRow;
use engine_playtest::replay::{bisect_tape, verify_tape, ReplayError};
use engine_playtest::seeded_bug::{...};
```

plus `crates/endpoint/src/look_host.rs:7` and `crates/endpoint/src/packs/view.rs:49`.
One stale comment survives and must be fixed:
`crates/endpoint/src/assistant/fork.rs:19` still says "`engine-playtest` stays
outside the endpoint dep edge".

**(c) There is NO `nav::crowd` in the tree.** Zero-hit proof, 2026-10-20:

```
git grep -n "nav::crowd" -- crates/       -> no output, exit 1
git grep -n "engine_nav::crowd" -- crates/ -> no output, exit 1
```

`crates/nav/src/` holds 14 files and no `crowd.rs`: bake.rs, cursor.rs,
flow.rs, funnel.rs, layers.rs, lib.rs, links.rs, mesh.rs, orca.rs, quant.rs,
reach.rs, rebake.rs, spline.rs, tiles.rs. The ORCA code the audit meant is
`crates/nav/src/orca.rs` (971 lines, zero callers - row D1). The shared cue
type is `crates/audio/src/crowd.rs` (512 lines, live: six intra-crate
callers plus `crates/game/src/audio_director.rs:17`).

**Doc line that must change (`docs/SMOOTH-BUILD.md`, "Build speed facts") and
`docs/SPRINT-13-engine-edge.md:34` (the OPT8 progress note).** Today they
say "16 syn, 12 serde_json, 14 num_traits copies in target/debug/deps before
the hack". Those are **build-artifact counts from 2026-10-17**, not
`Cargo.lock` facts. The corrected line:

> `Cargo.lock` holds 1,032 packages; 80 names carry more than one version and
> 101 extra copies are compiled. `serde_json` is one version (1.0.151),
> `num-traits` is one version (0.2.19) and is not a direct dependency of any
> crate, and `syn` is three versions (1.0.109, 2.0.119, 3.0.6). The
> "16 / 12 / 14" figures counted files in `target/debug/deps`, which drift
> with every build.

---

## For the OPT8 lane

Measured at HEAD `77789c2a` on 2026-10-20, `Cargo.lock`:

- **80 names carry more than one version; 101 extra version copies; 1,032
  packages in the lock.**
- **Fix these five first (19 of the 101 copies from five names):** `hashbrown`
  5 versions (0.13.2 0.14.5 0.15.5 0.16.1 0.17.1), `glam` 4 (0.30.10 0.31.1
  0.32.1 0.33.9), `windows-sys` 4 (0.52.0 0.59.0 0.60.2 0.61.2), `rand` 3
  (0.8.8 0.9.5 0.10.3), `syn` 3 (1.0.109 2.0.119 3.0.6).
- **Not the target:** `serde_json` is one version and `num-traits` is one
  version and is not a direct dependency - the old "12 serde_json / 14
  num_traits" numbers were build artifacts, not dependencies.
- **12 dependency names are declared by more than one crate with divergent
  feature sets** (this is what breaks unification): `image` 3 variants,
  `engine-assets` 3, `engine-render` 3, `engine-core`, `engine-arena`,
  `engine-anim`, `engine-scene`, `engine-game`, `wgpu`, `uuid`, `tokio`,
  `web-sys` 2 each.
- `crates/workspace-hack` is 2 lines of `lib.rs` with 4 externals and **no
  dependent crate**, so it unifies nothing today.
- **Bar for the row:** the dependency rebuild count across five lane builds
  drops by a third. Our number to compare against, measured from `Cargo.lock`
  at HEAD `77789c2a`: 80 names with more than one version and 101 extra
  copies; after the lane it must be at most 67 names and at most 67 extra
  copies.
- This lane lands no code for OPT8.
