# CT25 SPLIT-RECEIPT (piece 1) — builder-content-r6, 2026-10-08

Claim: `CT25 | builder-content-r6 | 2026-10-08T05:49Z | content/crowd/`
(`target/queue/claims.txt`). No other run claimed `content/crowd/` in the
last 2 h (checked before claiming). Eligibility on take: CT25 open, Tier A,
needs CT2/CT8 both ticked done. Size M, so this run lands ONE splitter piece
only; the row stays open.

## Ran (real output, pasted, not invented)

`node content/crowd/tools/gen.mjs` twice:

- run1: `crowd_impostor_01.gltf bytes=10947
  sha256=c0856c0fc6e0168b8833e22c30cfffb581bfa87bd03031300a6fec217c648926`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`
- run2: identical bytes, identical sha, identical mesh counts.

`manifest.json` records sha256 + bytes per generated file (provenance admit).

## Did NOT run green (stated, not hidden)

`cargo run --offline --manifest-path content/crowd/proof/Cargo.toml`
(the CT2 cook-twice proof over `cook_gltf_bytes`, Prop class):

- attempt 1 `--locked`: refused — no `Cargo.lock` yet (exit 1, fast).
- attempt 2 (lock generate + build): machine-slot timeout at ~300 s, zero
  output lines (pipe-buffered kill), incremental cache kept.
- attempt 3 after dropping the unused `registry` feature: machine-slot
  timeout again at ~300 s. Second identical timeout ends the step per the
  loop rules; no further retries this run.

Cause: cold compile of the `import` feature graph (`gltf` + `image` +
`engine-anim` -> `engine-combat` + `engine-nn`) exceeds the 5-minute slot on
the loaded box. `proof/target/` keeps the incremental cache, so a resume run
starts warm. No cook numbers are claimed; nothing is marked passing.

## Files landed (this piece)

- `content/crowd/tools/gen.mjs` (deterministic generator)
- `content/crowd/src/crowd_impostor_01.gltf` (10947 bytes, embedded buffers)
- `content/crowd/manifest.json` (sha256 + bytes + specimen counts)
- `content/crowd/proof/Cargo.toml` + `content/crowd/proof/src/main.rs`
  (CT2 cook-twice checks; not yet executed green)
- `content/crowd/README.md` (scope, budgets, remainder)

## Remainder

Resume `cargo run` in `content/crowd/proof/` to green (warm cache), then
`report-split1.json` lands with LOD/hull/digest numbers; full kit, soak
staging (CT50), variety (CT38) and eyes (CT44) stay open. Row Accept
(soak captures populated, 0 findings, cook x2) is NOT met — do not tick CT25.

## Piece 2 attempt — builder-content-r8, 2026-10-08T07:55Z (no green, caches warmer)

Claim: `CT25 | builder-content-r8 | 2026-10-08T07:00Z | content/crowd/`
(`target/queue/claims.txt`). Size M, so this run also lands no board tick;
the row stays open.

### Re-verified (real output, pasted, not invented)

`node content/crowd/tools/gen.mjs` (third run overall, first this run):

- `crowd_impostor_01.gltf bytes=10947
  sha256=c0856c0fc6e0168b8833e22c30cfffb581bfa87bd03031300a6fec217c648926`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`
- byte-identical to `manifest.json` (piece 1 numbers hold).

Proof source checked against the landed tree (no build): `cook_gltf_bytes`,
`asset_digest`, `MeshClass`, `PROP_MAX_TRIS` re-exported at
`crates/assets/src/lib.rs:71-79`; `MeshCookOutput` carries
`base_tris`/`lods`/`hulls`/`digest` (`mesh_cook.rs:1404-1421`);
`MAX_LOD_LEVELS = 4` (`mesh_cook.rs:43`, matches the proof's 4-LOD check);
`contains_mesh(&self, &CookedMesh)` (`mesh_cook.rs:1223`, matches the call).

Conformance gap found: `cargo test --locked -p engine-assets --lib mesh_cook`
matches 0 tests (118 filtered out, exit 0) because `mesh_cook` sits behind
`#[cfg(feature = "import")]` (`lib.rs:40-43`). The real gate is
`cargo test --locked -p engine-assets --features import mesh_cook`.

### Did NOT run green (stated, not hidden)

Two heavy-slot attempts, both `exitCode=124 timedOut=true` while compiling
the `import` graph (image/rav1e stack). Incremental caches kept in
`content/crowd/proof/target/` and the workspace `target/`; each attempt
compiled further than the last.

- attempt 1 (test `--features import mesh_cook` && proof run): compiled
  through `thiserror-impl`, then the cap. Receipt (ephemeral):
  `heavy-run-receipt-1791445501812-27112-24ccdb51.log`.
- attempt 2 (proof run only, warmest cache): compiled through `ravif`,
  then the cap. Receipt (ephemeral):
  `heavy-run-receipt-1791446102976-21380-c42ac4b8.log`.

Second identical timeout ends the step per the loop rules; no further
retries this run. No cook numbers are claimed; nothing is marked passing.

## Remainder

Resume `cargo run --offline --manifest-path content/crowd/proof/Cargo.toml`
to green (both caches warm now), then `report-split1.json` lands; run the
`--features import mesh_cook` gate beside it. Full kit, soak staging (CT50),
variety (CT38) and eyes (CT44) stay open. Row Accept is NOT met.

## Piece 3 — builder-content-r10, 2026-10-08T08:03Z (asset-only, no heavy build)

Claim: `CT25 | builder-content-r10 | 2026-10-08T08:03Z | content/crowd/`
(`target/queue/claims.txt`). No other run claimed `content/crowd/` in the
last 2 h (claims.txt holds only `CT28 | builder-content-r3 | crates/assets`
nearby; old r6/r8 CT25 lines already reaped by the keeper). Eligibility on
take: CT25 open, Tier A, needs CT2/CT8 both ticked done. Size M, so this run
lands ONE splitter piece only; the row stays open.

### Ran (real output, pasted, not invented)

`node content/crowd/tools/gen.mjs` twice (after adding variant 1):

- run1: `crowd_impostor_01.gltf bytes=10947
  sha256=c0856c0fc6e0168b8833e22c30cfffb581bfa87bd03031300a6fec217c648926`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`;
  `crowd_impostor_02.gltf bytes=10916
  sha256=6c5b4df51c6a6f0398678b2bd7cee4cc2270a21c97ad71f5b68d59973b3d73e3`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`
- run2: identical bytes, identical shas, identical mesh counts (both files).

01 bytes/sha hold frozen from pieces 1-2; 02 is new, Prop class
(96 LOD0 tris <= 5000), CC0-1.0 original, no donor bytes.

### Did NOT run (stated, not hidden)

No `cargo` build or test this run: pieces 1-2 already showed the
`import`-feature compile exceeds the machine slot twice (second identical
timeout ends that step), and this asset-only piece touches no crate. No cook
numbers are claimed; nothing is marked passing.

## Files landed (this piece)

- `content/crowd/tools/gen.mjs` (variant 1 builder + second emit)
- `content/crowd/src/crowd_impostor_02.gltf` (10916 bytes, embedded buffers)
- `content/crowd/manifest.json` (both files sha256 + bytes, two specimens)
- `content/crowd/README.md` (two-specimen scope + budgets)

## Remainder

Extend `content/crowd/proof/src/main.rs` to cook both specimens (or one
loop over `src/*.gltf`) and resume the warm-cache `cargo run` to green for
`report-split1.json`; then run the `--features import mesh_cook` gate.
Full kit, billboard chain, soak staging (CT50), variety (CT38) and eyes
(CT44) stay open. Row Accept (soak captures populated, 0 findings, cook x2
via `cargo test --locked -p engine-assets mesh_cook`) is NOT met — do not
tick CT25.

## Piece 4 — CT25 piece-2 continuation, 2026-10-08T12:30Z (cook runs, LOD2 empty)

### Re-verified (real output, pasted, not invented)

`node content/crowd/tools/gen.mjs` twice:

- run1: `crowd_impostor_01.gltf bytes=10947
  sha256=c0856c0fc6e0168b8833e22c30cfffb581bfa87bd03031300a6fec217c648926`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`;
  `crowd_impostor_02.gltf bytes=10916
  sha256=6c5b4df51c6a6f0398678b2bd7cee4cc2270a21c97ad71f5b68d59973b3d73e3`,
  `mesh Body_torso verts=54 tris=48`, `mesh Body_head verts=54 tris=48`
- run2: identical bytes, identical shas, identical mesh counts (both files).
  Baseline holds: 01 frozen since pieces 1-2, 02 since piece 3.

### Cook proof (compiled warm, ran red — real output, not claimed green)

`cargo run --offline --manifest-path content/crowd/proof/Cargo.toml`
(via heavy slot; 3m10s wait + 2m02s build first run, 13.41s rebuild):

- `CT25-PROOF mesh 0 lod 0 tris=48`
- `CT25-PROOF mesh 0 lod 1 tris=18`
- `CT25-PROOF mesh 0 lod 2 tris=0`
- `CT25-PROOF FAIL: mesh 0 lod 2 empty` (exit 1, no `report-split1.json`).

Targets for base 48: L1=24 (got 18, under target), L2=12 (got 0, empty).
Diagnosis: `box(...,2)` (48 tris, 6 disconnected face islands) empties at
the 0.25 ratio in the greedy CT2 pass; `content/kit-common.mjs:44-46`
recommends the default `n=4` subdivision to hold the whole 4-LOD chain.
Fix needs a generator subdivision bump (changes frozen bytes/sha, manifest
re-measure) or an engine simplify floor (outside `content/crowd/` scope,
not touched). Proof `src/main.rs` gained one `eprintln!` per-LOD line so
the next run pastes numbers without a rebuild cycle.

### Did NOT run green (stated, not hidden)

`cargo test --offline --locked -p engine-assets --features import mesh_cook`
(via heavy slot; 6m25s wait, compiled through `av-scenechange`, then the
machine cap): `exitCode=124 timedOut=true`. First timeout for that command
this run; step ends here per budget (S 10 min). No cook numbers claimed;
nothing marked passing.

## Remainder (piece 4 — superseded by piece 5 below)

Bump `tools/gen.mjs` subdivision toward `n=4`, re-run generator twice,
re-measure `manifest.json`, resume warm `cargo run` to green for
`report-split1.json` (extend to both specimens per piece-3 note), then run
the `--features import mesh_cook` gate. Full kit, soak staging (CT50),
variety (CT38) and eyes (CT44) stay open. Row Accept is NOT met.

## Piece 5 — CT25 rework (judge FAIL fixes), 2026-10-09

Fixes: (1) `tools/gen.mjs` subdivision `n=2` -> `n=4` per
`content/kit-common.mjs:44-46`; (2) `proof/src/main.rs` cooks both specimens
(`cook_one` loop over `src/*.gltf`) so 02 is wired; (3) `UNLOCKS: CT50` named
with caller below.

### Re-verified (real output, pasted, not invented)

`node content/crowd/tools/gen.mjs` twice:

- run1: `crowd_impostor_01.gltf bytes=27869
  sha256=21295d4413a4a61224a52ffb4fb0fa7eaf3835a7c1e61d4491b7dbc6d6824f7c`,
  `mesh Body_torso verts=150 tris=192`, `mesh Body_head verts=150 tris=192`;
  `crowd_impostor_02.gltf bytes=27838
  sha256=bd59050fe8eb178c39ef60e28597628838d77dec860e00e840b252a3c74eb334`,
  `mesh Body_torso verts=150 tris=192`, `mesh Body_head verts=150 tris=192`
- run2: identical bytes, identical shas, identical mesh counts (both files).
  Deterministic byte-identical x2. `manifest.json` re-measured with these real
  sha/bytes (n=2 bytes/sha superseded by this subdivision fix, not frozen).

### Cook proof (GREEN — real output, pasted, not invented)

`cargo run --offline --manifest-path content/crowd/proof/Cargo.toml`
(via heavy slot; 8m27s wait + 4m07s build, warm cache, exit 0):

- `CT25-PROOF src/crowd_impostor_01.gltf mesh 0 lod 0 tris=192`
- `CT25-PROOF src/crowd_impostor_01.gltf mesh 0 lod 1 tris=96`
- `CT25-PROOF src/crowd_impostor_01.gltf mesh 0 lod 2 tris=48`
- `CT25-PROOF src/crowd_impostor_01.gltf mesh 0 lod 3 tris=24`
- mesh 1 identical 192/96/48/24; specimen 02 both meshes identical
  192/96/48/24; hull containment holds all 16 checks; cook identical twice
  both files.
- `CT25-PROOF GREEN specimens=2 total_lod0=768`
- `report-split1.json` exists: 4 non-empty LODs per mesh, Prop budget holds
  (384 LOD0 tris/specimen <= 5000), asset digests
  01=`854961ae632d164b` 02=`043f77fb0398e730`.

Consumer: `content/crowd/proof/src/main.rs` (`cook_one` loop) reads
`src/crowd_impostor_02.gltf` directly — 02 is wired, not a loose file.

`UNLOCKS: CT50` (caller: `content/crowd/proof/src/main.rs` cooks 02 green;
next: CT50 soak staging over CT25 + CT44).

### Did NOT run green (stated, not hidden)

`cargo test --offline --locked -p engine-assets --features import mesh_cook`
(via heavy slot; 19m14s wait, then the machine cap): `exitCode=124
timedOut=true`. First timeout for that command this run; no numbers claimed
from it (the proof-binary cook above is the green cook evidence). Step ends
here per budget (slot waits alone exceeded S 10 min + packet cap 25 min).

## Remainder (piece 5)

Run the `--features import mesh_cook` gate on a warm cache (workspace
`target/` now holds the compiled `import` graph from this run's green proof
build), then stage soak (CT50), variety (CT38) and eyes (CT44). Row Accept
(soak captures populated, 0 findings, cook x2) is NOT met — do not tick CT25.

## Re-check 2026-10-17 (scaffold only, row stays open)

- `node content/crowd/tools/gen.mjs` run twice: both specimen files keep the manifest SHA-256 (`21295d44...` and `bd59050f...`). Deterministic.
- GitHub `cargo check --locked --manifest-path content/crowd/proof/Cargo.toml`: FAIL, run 39064515835. `--locked` refused because `content/crowd/proof/Cargo.lock` does not match the crate. Not fixed in this pass.
- The cook proof (`cargo run` of the proof binary) is not on the gh-check allow-list (check, test, clippy only), so `report-split1.json` is still not produced. CT25 Accept (soak captures, cook x2) is not claimed.
