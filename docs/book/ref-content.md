# Content, cook and bundle reference

Scope: the content directories under `content/`, the asset kinds the `engine-assets` crate knows, the cook entry points (Node scripts, Blender script, Rust bins and examples), the two bundle formats (`E204BNDL` and `E2BUNDLE`), the collider blob `E2CC`, the `game.toml` status, and the skill directories under `skills/`. Every statement was read from code or files in the repo at `C:\engine2040` on 2026-10-17. Paths are relative to the repo root unless they are absolute.

## Contents

1. Content roots
2. Asset kinds
3. Cook entry points
4. Binary formats: E204BNDL, E2BUNDLE, E2CC
5. game.toml
6. Skills
7. Not documented
8. Authored generators: rewrite rules and Rome service walkers

## 1. Content roots

Root: `content/` (checked-in content; git-ignored subtrees are marked). Readers are the crates or tools that open the path.

| Path | What lives there | Read by |
|---|---|---|
| `content/manifest.json` | CT37/CT23 character intake manifest (combat rows, licence marks, digests). `"tool": "tools/cook-char.mjs"` | `tools/cook-char.mjs --check` (default `--manifest`) |
| `content/arena/` | Arena kit: `placement.json` (kit), `manifest.json`, `dressing.json`, `establishing.json`, `hdri/`, `pbr/`, `textures/`, `src/`, `previews/`, `proof/` | `crates/arena/src/kit.rs:17` (`include_str!` of `placement.json`); `crates/game/src/arena_kit.rs:575-582` (walks up from the exe to find `content/arena/placement.json`); `crates/render/src/ibl.rs:3426`, `crates/render/src/v1_gpu.rs:105`, `crates/render/src/present/gpu_present.rs:16` (`include_bytes!` of `hdri/polyhaven-studio-small-09-1k.hdr`); editor `crates/editor/src/a2_session.rs:35`, `a3_session.rs:33` (`arena/manifest.json`) |
| `content/fighters/` | Fighter GLB bodies and clip GLBs (`human_male_run.glb` etc.), `manifest.json` (CC0-1.0 source for the intake licence gate), `lod/` (`hero.json`), `previews/`, `sidecars/` | `crates/game/src/fighter_look.rs:136-141` (clip file names), `:937-944` (walks up to `content/fighters`); `crates/game/src/bin/staged_fight.rs:135`; `tools/gltf-intake.mjs` (`DEFAULT_MANIFEST`); editor `a2_session.rs:36`, `a3_session.rs:34` (`fighters/manifest.json`); `crates/assets/examples/ct35_lod_bake.rs` (writes `lod/hero.json` per its header) |
| `content/audio/` | `impacts/`, `footsteps/`, `ambience/`, `engine2040-vol0-manifest.json` | `crates/audio/src/decode.rs:214` (`load_cue_bank`, WAV decode); `crates/audio/src/cue.rs` (fixed file names such as `impacts/flesh-small.wav`); `crates/game/src/windowed.rs:791-796`, `:2325` |
| `content/design/` | Text tables `*.rows`: `ai.rows`, `enemies.rows`, `arena-zones.rows`, `combat.rows`, `controls.rows`, `cues.rows`, `ladder.rows`, `loop.rows`, `weapons.rows`, plus `LICENCE` | `include_str!`: `crates/ai/src/curves_text.rs:235`, `crates/ai/src/depth.rs:2482`, `crates/ai/src/gladiator.rs:323`, `crates/ai/src/tier.rs:141`, `crates/arena/src/zones.rs:173`. Runtime read: `crates/game/src/practice.rs:109` (`DESIGN_COMBAT_ROWS_PATH`), parsed by `crates/combat/src/tables_text.rs`. Tests: `crates/game/tests/it/design_*.rs` |
| `content/physics/` | `unit_cube_hull.e2cc`: golden collider blob (untracked in git status at session start) | `include_bytes!` in `crates/physics/src/collider.rs:474`, `crates/arena/src/kit_zone.rs:504`, `crates/assets/src/collider_cook.rs:851`, `crates/endpoint/src/packs/physics.rs:569` |
| `content/props/ai/` | `urn.glb` (AI-generated prop example) | Only `crates/assets/src/provenance.rs` tests (`content/props/ai/urn.glb`) |
| `content/sky/` | `preset_day.json`, `preset_dusk.json`, `preset_night.json`, `LICENCE` | 2 files mention it; reader not traced |
| `content/materials/` | `reference/`, `siege/`, `LICENCE` | 2 files mention it; reader not traced |
| `content/maps/` | `placement_district.json`, `placement_ludus.json`, `placement_siege.json`, `manifest.json`, `district/`, `ludus/`, `hdri/`, `textures/`, `urban-war-pilot/` | 4 files mention it; reader not traced |
| `content/locomotion/` | `bake.json`, `clips.json`, `sidecars/`, `real/`, `ATTRIBUTION.md`, `STYLE-BAKE.md` | 4 files mention it. `walk.anim` is named only in `crates/endpoint/src/insight/` (see section 7) |
| `content/vfx/`, `content/urban/`, `content/sprites/` | VFX and sprite art (`sprites/` holds PNG cover art and `LICENSES.md`) | 1-2 files mention each; reader not traced |
| `content/crowd/`, `content/ai-loop/` | `crowd/` has `manifest.json`, `SPLIT-RECEIPT.md`, `src/`, `tools/`, `proof/`; `ai-loop/` contents not traced | No reference in `crates`, `tools`, `games`, `examples`, `scripts` or `skills` |
| `content/local/` | Real locomotion bytes; git-ignored (`.gitignore:8`) | Mentioned only in `tools/cook-char.mjs` header comments |
| `content/incoming/` | Downloads awaiting intake; git-ignored (`.gitignore:15`, and `content/fighters/incoming/` at line 16) | Mentioned by 3 files; the `fetch-content.mjs` default is `content/_incoming` (not present, not ignored) |

## 2. Asset kinds

The code has no single enum for content kinds. The table lists each kind the code reads or writes, with its extension, the function or reader, and the source file.

| Kind | Extension(s) | Reader or cook function | Source file |
|---|---|---|---|
| glTF 2.0 source | `.gltf`, `.glb` | `Format::Glb` / `Format::Gltf` chosen from the extension; `import::import`, `mesh_import::import_gltf_path`, `mesh_import::import_gltf_bytes` (extension gate `.gltf`/`.glb`) | `crates/assets/src/import.rs:20-25`, `:106-115`; `crates/assets/src/mesh_import.rs:200-203` |
| Cooked mesh and LOD chain | source `.glb`/`.gltf` | `mesh_cook::cook_gltf_bytes`, `cook_mesh`, `build_lod_chain`, `check_budget`; `MeshClass` | `crates/assets/src/mesh_cook.rs` |
| Skinned body, skeleton, clip | `.glb`/`.gltf` (skins, animations) | `skin_import::import_skinned_bytes`, `import_skeleton`, `import_clip`, `cook_clip_tables`; `retarget::retarget_cook` for clip batches | `crates/assets/src/skin_import.rs`, `crates/assets/src/retarget.rs`, `crates/assets/src/clip_intake.rs` |
| Collider | glTF nodes named with the `COL_` prefix, cooked to the `E2CC` blob | `collider_cook::cook_gltf_colliders`, `cook_hull`, `cook_mesh`, `encode`; prefix constant `COL_PREFIX` | `crates/assets/src/collider_cook.rs`; `crates/assets/src/mesh_cook.rs` |
| Texture source | encoded image bytes (PNG and JPEG are the formats in the repo's fixtures and the `--texture-png` path) | `texture::cook_texture(bytes, TextureSlot)`; `BaseColor` is sRGB-linearized on cook; `image` 0.25 decodes | `crates/assets/src/texture.rs` |
| Texture output | `.ktx2` (BC block-compressed) | `texture_cook` (BC codec in `bc.rs`); `import::validate_ktx2` on import | `crates/assets/src/texture_cook.rs`, `crates/assets/src/bc.rs`, `crates/assets/src/import.rs:179`, `:231` |
| Motion source | `.bvh`, `.fbx`, `.gltf`, `.glb` | `tools/cook-char.mjs --cook` starts Blender with `tools/blender/cook_char.py` | `tools/cook-char.mjs:145-146` (`COOK_INPUT_EXTS`), `:149` (`buildCookArgs`) |
| Generated mesh intake | GLB from an offline image-to-3D runner, plus its provenance | `gen_intake`: licence gate, repair, prop cook, bundle, provenance row. Consumer: `asset.intake` verb (feature `intake`) | `crates/assets/src/gen_intake.rs` |
| Cooked entries (inside E204BNDL) | none on disk on their own | `BundleWriter::add_mesh` / `add_material` / `add_texture` / `add_skin` / `add_clip`. Wire kinds 1-5 | `crates/assets/src/bundle.rs:183-211`; `crates/assets/src/archive.rs:53-64` |
| Packed content file | any file (path is the key) | `asset_source::pack_to`, `bundle_pack`, `BundleSource::open` | `crates/assets/src/asset_source/bundle.rs` |
| WAV audio | `.wav` | `audio::decode_wav` (hound); `load_cue_bank` reads the fixed cue paths under `content/audio/` | `crates/audio/src/decode.rs:89`, `:214`; `crates/audio/src/cue.rs` |
| Ogg Vorbis audio | `.ogg` | `audio::decode_vorbis` (lewton). No non-test caller found | `crates/audio/src/decode.rs:158` |
| Design tables | `.rows` (text) | `include_str!` in `crates/ai`, `crates/arena/src/zones.rs`; runtime parse `combat::tables_text` for `content/design/combat.rows` | `crates/combat/src/tables_text.rs`; `crates/game/src/practice.rs:109` |
| Arena kit placement | `.json` | `include_str!` as `KIT_JSON` | `crates/arena/src/kit.rs:17` |
| Schema-gated JSON | `.json` | `schema::Schema` (std-only schema gate: compile once, refuse with key paths) | `crates/assets/src/schema.rs` |
| Indent-tree text | no extension fixed | `datanode::parse` (indent is hierarchy, whitespace split tokens) | `crates/assets/src/datanode.rs` |
| Environment map | `.hdr` | `include_bytes!` in render and tests; the decoder was not traced | `content/arena/hdri/polyhaven-studio-small-09-1k.hdr` |
| Collider blob | `.e2cc` | `physics::collider::decode` (reader); `collider_cook::encode` (writer) | `crates/physics/src/collider.rs:262`; `crates/assets/src/collider_cook.rs:550` |
| Museum fixtures | `FormatKind::Tape` `.bin`, `Scene` `.json`, `Bundle` `.bin` | `FormatKind::extension()` (fixture naming only) | `crates/assets/src/museum.rs:38-70` |
| Source watch events | n/a (`FileKind`: Created, Modified, Removed) | `watcher::Watcher::poll` feeds `recook::Recook` | `crates/assets/src/watcher.rs:26`; `crates/assets/src/recook.rs` |

## 3. Cook entry points

Summary. Failure texts are listed under each entry.

| Entry point | Invocation (exact flags from code) | Input | Output | Test file |
|---|---|---|---|---|
| `tools/cook-char.mjs` (check) | `node tools/cook-char.mjs --check [--manifest <path>]` | manifest JSON, default `content/manifest.json` | stdout `RESULT cook-char ok landed=N/80 ...`, exit 0. No bytes written | `tools/cook-char.test.mjs` |
| `tools/cook-char.mjs` (cook) | `node tools/cook-char.mjs --cook <in> <out> [--script <path>] [--blender <exe>]` | `.bvh`, `.fbx`, `.gltf`, `.glb` | `<out>` (`.glb` or `.gltf`) written by Blender; stdout `cook: peak_ws_mb=N` | `tools/cook-char.test.mjs` |
| `tools/blender/cook_char.py` | run by cook-char only: `blender -b -P tools/blender/cook_char.py -- <in> <out>` | as above | `<out>`; stdout `cook_char: objects=N actions=N bytes=N out=PATH` | none found |
| `tools/gltf-intake.mjs` | `node tools/gltf-intake.mjs <in.gltf\|in.glb> <out.glb> [--licence SPDX] [--manifest path] [--author name] [--licence-read text] [--provenance path]` | glTF 2.0 file, licence must resolve to `CC0-1.0` | `<out.glb>` plus provenance JSON at `--provenance` or `<out>.prov.json`; stdout `RESULT gltf-intake ok in=N out=N issues=N triangles=N` and one JSON line | `tools/gltf-intake.test.mjs` |
| `tools/fetch-content.mjs` | `node tools/fetch-content.mjs --list <file> [--out <dir>] [--dry-run] [--rate-ms N]` (`--help`, `-h`) | list file rows: `<source> <id> <url> [md5] [license] [author]`; source is `polyhaven`, `ambientcg` or `url` | `<out>/<source>/<id>/<file>` plus `<id>.license.json` `{url, author, license, retrieved}`. Default `<out>` is `content/_incoming`. Rate 1000 ms by default | `tools/fetch-content.test.mjs` |
| `bundle_cook` (bin) | `cargo run --release -p engine-assets --features import,registry --bin bundle_cook -- --out <bundle.bin> [--texture-png <name=path>]... [--empty] [--allow-stub]`; check: `... -- --check <bundle.bin> [--allow-stub]` (form used by `tools/ship.mjs:517-530`) | demo staging, or PNG files named by `--texture-png` | E204BNDL file. Stdout: receipt line; `--check` also prints `entry <kind> <name> <digest>` per entry | inline `#[cfg(test)]` module in `crates/assets/src/bin/bundle_cook.rs`; `tools/ship.test.mjs` for the ship caller |
| `e2bundle` (bin) | `e2bundle pack <root> <out.e2bundle> [--include <prefix>]... [--priority <prefix>=<tier>]...`; `e2bundle verify <bundle> [<root>]` | directory tree (pack) or `.e2bundle` (verify) | `.e2bundle` file. Stdout `e2bundle pack OK entries=N bytes=N digest=<16 hex>` | inline tests in `crates/assets/src/bin/e2bundle.rs` |
| `ct35_lod_bake` (example) | `ct35_lod_bake <hero_body.glb> <out hero.json>`; `Cargo.toml` sets `required-features = ["import"]` | `hero_body.glb` | `hero.json` LOD chain table. Writes nothing on any refusal | none listed |

Library cook functions (no CLI, called from Rust): `texture::cook_texture` (`crates/assets/src/texture.rs`), `texture_cook` (`crates/assets/src/texture_cook.rs`), `mesh_import::import_gltf_path` (`crates/assets/src/mesh_import.rs`), `mesh_cook::cook_gltf_bytes` (`crates/assets/src/mesh_cook.rs`), `skin_import::import_skinned_bytes` (`crates/assets/src/skin_import.rs`), `retarget::retarget_cook` (`crates/assets/src/retarget.rs`), `collider_cook::cook_gltf_colliders` (`crates/assets/src/collider_cook.rs`), `gen_intake` (`crates/assets/src/gen_intake.rs`), `cooker::Cooker` (`crates/assets/src/cooker.rs`, digest cache), `recook::Recook` (`crates/assets/src/recook.rs`). Integration tests: `crates/assets/tests/it/blockcomp_cook.rs`, `meshlet_lod_cook.rs`, `gltf_conformance.rs`, `gltf_cc0_corpus.rs`.

### 3.1 tools/cook-char.mjs

- Usage (from `--help`): `node tools/cook-char.mjs --check [--manifest content/manifest.json]` and `node tools/cook-char.mjs --cook <in.bvh|fbx|gltf|glb> <out.glb|gltf> [--script <path>] [--blender <exe>]`.
- Blender lookup: `--blender`, else `E2040_BLENDER`, else `blender` on PATH. Default script `tools/blender/cook_char.py`.
- `--check` never fetches, cooks or writes. Named refusals go to stderr with exit 2: `E_MANIFEST_ABSENT`, `E_MANIFEST_EMPTY`, `E_PARSE`. Findings print as `FAIL <line>` on stderr with exit 1.
- `--cook` refusals (exit 2): `E_USAGE` (parse), `E_COOK_USAGE`, `E_COOK_INPUT` (input absent), `E_COOK_FORMAT` (bad extension), `E_COOK_SCRIPT` (script absent). Blender spawn: `E_BLENDER_SPAWN`. Blender exit code is passed back; a failed cook is exit 3 from `cook_char.py`, reported as `E_COOK_EXIT <code>`.
- `cook_char.py` refusals: `E_COOK_USAGE` (args after `--` must be exactly two), `E_COOK_NO_BPY` (not run with `blender -b -P`), `E_COOK_EXPORT`, `E_COOK_NO_OUTPUT` (empty file or not a GLB). Exit 2 for bad args, 3 for cook failure.
- Checked run on 2026-10-17: `node tools/cook-char.mjs --check` printed `RESULT cook-char ok landed=30/80 locomotion=planned combat=30+50 manifest=C:\engine2040\content\manifest.json` and exit 0.
- Manifest rules (from `checkManifest`): CC-BY-4.0 required on `locomotion.record_100style`; accepted SPDX `CC0-1.0`, `CC-BY-4.0`; refused marks `geno`, `lafan1`, `mixamo`, `qal`, `unreal`; combat target 80; `keys` in 432, 475, 819, 862.

### 3.2 tools/gltf-intake.mjs

- Input rules: `.gltf` must parse as JSON with `asset.version` `"2.0"`; `.glb` must start with the `glTF` magic. Anything else: `E_PARSE`.
- Licence: `--licence` wins; else `--manifest` (default `content/fighters/manifest.json`), first of `spdx`, `licence`, `license`, `sources[].spdx`, with `CC0-1.0` preferred. Not `CC0-1.0`: `E_LICENCE`, nothing copied.
- Pipeline: validate input with `gltf-validator`, `dedup` and `prune` (`@gltf-transform`, no compression), validate output, write. Output may not lie inside `content/fighters` (`E_READONLY`).
- Refusals (exit 2): `E_INPUT_ABSENT`, `E_INPUT_EMPTY`, `E_LICENCE`, `E_PARSE`, `E_INPUT_INVALID`, `E_OUTPUT_INVALID`, `E_READONLY`. Unexpected throw: `E_INTAKE_CRASH` (exit 1).
- Its dependencies load from `tools/node/package.json`, which is not in the tree (see section 7).

### 3.3 tools/fetch-content.mjs

- Downloads files. Do not run it without the user's permission (it fetches from network hosts).
- Each row needs `id` and `url`. A `-` url or the `polyhaven` and `ambientcg` sources resolve to their API roots; the `url` source needs an explicit URL. `md5` must be 32 hex characters if given.
- Output per file: `OK <source> <id> <bytes>B <path>` on stdout; a file whose md5 already matches is `SKIP ... md5-match`. `--dry-run` prints `DRY total_bytes=N files=N` and one `LICENSE` line per row, writes nothing.
- Failure texts: `fetch-content: error: --list <file> is required` (exit 2); `fetch-content: error: list file absent: <file>` (exit 2); md5 mismatch prints `FAIL <source> <id> md5 mismatch want=... got=...`, deletes the file, and at the end `fetch-content: error: N file(s) failed md5 and were deleted` (exit 1). Success: `RESULT fetch-content ok downloaded=N skipped=N bytes=N`.
- User-Agent `engine2040-fetch-content/1.0 (+https://github.com/engine2040)`.

### 3.4 crates/assets/src/bin/bundle_cook.rs

- Needs `--features import,registry`; without them it prints `bundle_cook needs --features import,registry` and exits 2.
- Default cook is the demo staging (non-empty). `--empty` writes the 16-byte stub, refused without `--allow-stub`. `--texture-png name=path` adds one BaseColor texture through `cook_texture`.
- Parse errors (exit 1, printed as `bundle_cook FAIL: <msg>`): `missing value for --out`, `pass --check or --out, not both`, `pass --out or --check`, `unknown flag <flag>; pass --help`, `--texture-png wants <name=path>`, `--empty takes no --texture-png`.
- `--help` / `-h` returns the usage text as an error, so it also exits 1 (see section 7).

### 3.5 crates/assets/src/bin/e2bundle.rs

- Native only; the wasm build of the bin is empty.
- `pack` options: `--include <prefix>` (repeatable; plain string prefix), `--priority <prefix>=<tier>` (tier 0-255; longest prefix wins).
- `verify <bundle> [<root>]` reads every blob (checks each hash), prints `e2bundle verify OK entries=N bytes=N digest=<16 hex>` and `  tier N: K files, ends at byte B` lines, and with `<root>` compares each file byte for byte and prints `every file matches <root>`.
- Failures: `e2bundle FAIL: <msg>` on stderr, exit 1. Unknown command or no args: the usage text is printed in the same FAIL form. `verify` mismatch: `"<path>" differs between the bundle and <root>`.
- No script in `tools/` or `crates/` runs `e2bundle pack` (see section 7).

### 3.6 crates/assets/examples/ct35_lod_bake.rs

- Requires `--features import`. Refuses (non-zero exit, writes nothing) on cook failure, digest drift, a growing chain or a hull miss.
- Its header names `content/fighters/lod/hero.json` as the consumer. That file exists in the tree.

### 3.7 tools/blender/cook_char.py

- Run only through `cook-char.mjs --cook`. Blender keeps its own flags before `--`; the script reads exactly two arguments after it (`args_after_dashes`).

## 4. Binary formats: E204BNDL, E2BUNDLE, E2CC

Three formats. The names are close: `E204BNDL` is a cooked-asset bundle (`.bin` in every path found), `E2BUNDLE` is a packed file tree (`.e2bundle`), and `E2CC` is a collider blob.

### 4.1 E204BNDL (cooked-asset bundle)

- Code: `crates/assets/src/bundle.rs`. Writer `BundleWriter::finish` (`bundle.rs:215`). Reader `BundleLoader::load` (`:366`) and `parse` (`:407`), `load_bundle` (`:380`).
- Whole-file helpers: `cook_bundle_bytes` (`crates/assets/src/bundle_cook.rs:229`), `write_bundle_file` (`:340`), `cook_and_write` (`:368`), `load_bundle_file` (`:383`), `verify_bundle_bytes` (`:299`), `header_entry_count` (`:261`), `is_stub_bundle` (`:291`).
- Shipped as `content/arena-bundle.bin` (`tools/ship.mjs:319`). The ship checker `checkBundleHeader` (around `tools/ship.mjs:463`) reads the same header.
- Constants: magic `E204BNDL`, `BUNDLE_VERSION = 1`, `MAX_ENTRIES = 4096`, `MAX_NAME_CHARS = 128`, `MAX_NAME_BYTES = 512`, `MAX_PAYLOAD_BYTES = 64 MiB`, `MAX_BUNDLE_BYTES = 512 MiB`, `MAX_BUNDLE_INDICES = 8,000,000`, `MAX_BUNDLE_REFS = 8`.

Layout (all integers little-endian; there is no trailer):

| Offset | Field | Type | Notes |
|---|---|---|---|
| 0 | magic | 8 bytes | `E204BNDL` |
| 8 | version | u32 | must be 1. Higher: `FutureVersion`. Other: `Corrupt` |
| 12 | entry count | u32 | at most 4096 |
| 16 | entries | repeated `count` times, in writer order | see below |

Each entry:

| Field | Type | Notes |
|---|---|---|
| kind | u8 | `ArchiveKind` wire value: 1 Mesh, 2 Material, 3 Texture, 4 Skin, 5 Clip. Other values: `Corrupt` |
| name length | u16 | bytes, at most 512 |
| name | UTF-8 | 1 to 128 characters; unique in the bundle |
| payload length | u32 | at most 64 MiB |
| payload | bytes | encoded per kind (below) |
| digest | 64 ASCII hex | blake3 hex of the payload (`content_digest`, `crates/scene/src/ids.rs:89`). Mismatch: `DigestMismatch` naming the entry |

After the last entry, extra bytes are refused as `trailing bytes`. A stub bundle is exactly 16 bytes: magic, version 1, count 0.

Payload field order, from the `encode_*` functions in `bundle.rs`:

| Kind | Field order |
|---|---|
| Mesh (`encode_mesh`, `:578`) | u32 vertex count; positions (3 f32 per vertex); normals; tangents; u32 index count; u32 indices |
| Material (`encode_material`, `:669`) | base colour factor (f32 each); metallic f32; roughness f32; u32 texture ref count; per ref: u64 asset id, u8 slot |
| Texture (`encode_texture`, `:715`) | u32 width; u32 height; u8 slot; u8 encoding; u32 mip count; per mip: u32 width, u32 height, then width x height texel values (element encoding in `encode_texture`, not traced). Limits: dimension `MAX_TEXTURE_DIM`, mips `MAX_MIPS`, total texels `MAX_TEXTURE_PIXELS` |
| Skin (`encode_skin`, `:814`) | u32 row count; per row u16 joint indices; then the weights (not traced further) |
| Clip (`encode_clip`, `:873`) | u16 name length; name bytes; f32 duration; u32 position-track count and tracks; u32 rotation-track count and tracks. Track layout: `encode_track` (`:908`) |

Ship manifest (`SHIP_MANIFEST`, `bundle.rs`, checked by `verify_ship_entries`): Mesh `hero`, Material `hero-mat`, Texture `hero-albedo`, Skin `hero-skin`, Clip `hero-step`.

Determinism: the same entries added in the same order give the same bytes (`finish_is_deterministic` test).

### 4.2 E2BUNDLE (`.e2bundle`, packed file tree)

- Code: `crates/assets/src/asset_source/bundle.rs`. Writer `pack_to` (`:220`, any `Write`) and `bundle_pack` (`:264`, native file; removes a partial output on failure; refuses an output inside the content root). Reader `BundleSource::open` (`:336`) calls `parse_header` (`:451`), `parse_index` (`:493`), `check_layout` (`:540`). `BundleSource::read` (`:390`) checks each blob hash. CLI: `crates/assets/src/bin/e2bundle.rs`.
- Module header (`bundle.rs:1-20`) gives the layout; the code follows it. Constants: `E2BUNDLE_MAGIC = b"E2BUNDLE"`, `E2BUNDLE_VERSION = 1`, `MAX_E2BUNDLE_ENTRIES = 65,536`, `HEADER_LEN = 32`.

Header (32 bytes):

| Offset | Field | Type | Notes |
|---|---|---|---|
| 0 | magic | 8 bytes | `E2BUNDLE` |
| 8 | version | u32 | must equal 1; other values refused |
| 12 | entry count | u32 | at most 65,536 |
| 16 | index byte length | u64 | |
| 24 | framing hash | u64 | FNV-1a 64 over: version (u32), count (u32), index length (u64), index bytes (`framing_hash`, `:207`) |

Index: `count` rows, sorted by path, strictly ascending (checked on read). Each row:

| Field | Type | Notes |
|---|---|---|
| path length | u16 | |
| path | UTF-8 | rules in `validate_path` (`asset_source.rs:135`): relative, `/` separated, no empty part, no `.` or `..`, at most 512 bytes |
| priority | u8 | tier; 0 is fetched first. Tiers `0..=p` form one prefix of the file |
| offset | u64 | absolute file offset of the blob |
| length | u64 | blob length |
| hash | u64 | FNV-1a 64 of the blob bytes (`engine_core::hash_bytes`, `crates/core/src/hash.rs:116`) |

Blobs: back to back, ordered by (priority, path), no padding. `check_layout` requires each offset to equal its layout position and the last blob to end at end of file. Otherwise the reader refuses with the name of the blob.

Integrity only: the index hash catches damage in header and index, and each blob hash is checked on every read, so one damaged blob fails by name and the others still load. FNV-1a is not a security check (module doc).

Pack rules (`PackRules`): `include(prefix)` selects files, `priority(prefix, level)` sets the tier, longest matching prefix wins, unmatched files are tier 0. The pack report digest is `ContentDigest` (`asset_source.rs:184`) over path, length and hash of each file.

### 4.3 E2CC (collider blob)

- Code: writer `collider_cook::encode` (`crates/assets/src/collider_cook.rs:550`); reader `physics::collider::decode` (`crates/physics/src/collider.rs:262`). Constants: `MAGIC = b"E2CC"`, `VERSION = 1`, `MAX_COLLIDERS = 256`.
- Layout (little-endian, from the module header): `"E2CC"` | u16 version | u16 count | per collider: u8 name length, name, u32 layer, u8 kind.
  - kind 0 (hull): u16 point count, points (3 f32 each), u16 face count, faces (3 u16 each).
  - kind 1 (mesh): u32 vertex count, vertices (3 f32 each), u32 index count, indices (u32 each).
- Golden file: `content/physics/unit_cube_hull.e2cc`.

### 4.4 Other format facts

- `E204ARCV` (`crates/assets/src/archive.rs:22-24`): archive container, `ARCHIVE_VERSION = 1`, `ArchiveKind` 1-5 (same kinds as E204BNDL). `crates/assets/src/lib.rs` marks `archive` as "planned, no caller yet (verified 2026-10-16)".
- `archived_cook` (`COOK_MAGIC`, `COOK_VERSION`): also marked "planned, no caller yet" in `lib.rs`.

## 5. game.toml

planned: AIS1

Searched with `grep "game.toml"` over `crates`, `tools`, `content`, `docs`, `skills`, `games`, `examples`, `scripts`, `src` and `tests`, excluding `target/`. No parser, no file, and no code reference was found. The only match is a plan line: `docs/SPRINT-22-web-realism.md:44` describes a planned `game.toml` (name, targets windows and web, quality tier, physics layers, gore level, content roots). The board ID `AIS1` appears in `docs/plan/web-and-realism.md` (lines 9 and 57) and in `docs/research/web-2026-10-17/05-ai-web-loop-and-ci.md:19`. No key list is documented here because no parser exists.

## 6. Skills

The skills live in `skills/` (26 folders since the 2026-10-17 merge of 86). The map, the triggers and the old-name-to-new-home table are in `skills/SKILLS.md`; the file rules are in `skills/README.md` and are checked by `tools/skills-index.test.mjs`. `.opencode/skills/` no longer exists.

## 7. Not documented

Searched for, not found:

- `game.toml` parser or file: none (section 5).
- A runtime reader for `.ktx2`: none in `crates/render` or `crates/game` (texture_cook writes it; import validates it).
- A reader for `.anim`: `content/locomotion/walk.anim` appears only in `crates/endpoint/src/insight/` (`eval.rs:336`, `:448`, `index.rs`, `graph.rs`). No file was checked on disk.
- `content/kit.json`: no such file. `crates/game/src/reload.rs:449-460` only passes the name to `gate.note_changed` (in tests). The arena kit is `content/arena/placement.json`.
- `tools/node/` (dependency folder for `gltf-intake.mjs`, `heavyDeps` reads `tools/node/package.json`): not present in the tree and not git-ignored. The intake pipeline cannot start on this tree.
- `content/arena-bundle.bin` and `content/contents.txt`: not present. `tools/ship.mjs` writes them at package time; `crates/editor/src/play.rs:132` lists them in `SHIP_FILES`.
- Callers of `e2bundle pack`: none in `tools/`, `crates/web`, `games/`, `clients/` or `scripts/`. The bin's header says "the web build packs the arena content once"; no such step was found. Uses of `BundleSource` outside `crates/assets` are test-only (`crates/game/src/arena_kit.rs`, `crates/game/src/fight_ring.rs`).
- Non-test caller of `audio::decode_vorbis`: none found.
- Readers for `content/sky`, `content/materials`, `content/maps`, `content/vfx`, `content/urban`, `content/sprites`: mentions counted only, readers not traced.
- The `.hdr` decoder behind `include_bytes!` in `crates/render`: not traced.
- Root directories `games/` and `fixtures/`: not traced for content roots.

Suspicious items:

- `crates/assets/src/bin/bundle_cook.rs` `parse_args`: `--help` and `-h` return the usage as an error, so the usage is printed as `bundle_cook FAIL: ...` with exit 1.
- Two different bundle formats are both called bundles and both are documented in code: `E204BNDL` (usage text says `bundle.bin`, shipped as `content/arena-bundle.bin`) and `E2BUNDLE` (`.e2bundle`). Use the file magic, not the extension, to tell them apart.
- `tools/ship.mjs` declares `function checkBundleHeader` twice (lines 463 and 1413, both at top level). `node --check tools/ship.mjs` passes. Which definition runs was not traced.
- `crates/assets/src/bin/e2bundle.rs` header claims a web-build step that is not in the tree (see above).
- `crates/game/src/reload.rs` watches `content/kit.json`, which is not a file in the tree.
- `tools/gltf-intake.mjs` header says deps "live in tools/node" (missing, see above).
- No test file was found for `tools/blender/cook_char.py`.
- `lib.rs` marks `archive`, `archived_cook`, `stream` and `uv2_chart` as "planned, no caller yet (verified 2026-10-16)".

Checks run for this doc (read-only): `node tools/cook-char.mjs --check` (exit 0), `--help` for cook-char, gltf-intake and fetch-content, and `node --check tools/ship.mjs`. No cook, fetch, cargo or Blender run was made. No test file was run.

## 8. Authored generators: rewrite rules and Rome service walkers

These two systems take authored text and turn it into a result. Both are new on 2026-10-24, and neither is wired into a door or a map yet.

### 8.1 Rewrite rules (MJ1)

`crates/world/src/rewrite.rs` is a seeded grid rewrite generator. The idea comes from MarkovJunior. The text format is ours, and it is 2D. It is bake-time only and never touches the tick or the sim hash (module doc). Its tests live in `crates/world/src/rewrite/tests.rs`.

**Rule file format.** The example files are `content/maps/rewrite/maze_streets.rules`, `insula_block.rules` and `ruined_wall.rules`. The format is set out in the module doc at `rewrite.rs:21-64`.

- One statement per line. `#` starts a comment. Blank lines are ignored. `{` and `}` are separate words.
- `size W H` must come first. W and H are each 1 to 128.
- `alphabet ABC` lists the symbols. Each symbol is one ASCII character other than `# / * = { }`. The first symbol fills the grid at the start.
- `put X Y SYM` sets one cell. `rect X Y W H SYM` fills a rectangle.
- `one { ... }` takes one match per step, chosen uniformly. `all { ... }` takes one round per step, with matches that do not overlap. `prl { ... }` applies every match of the round at once. `sequence { ... }` runs its children in order. `markov { ... }` takes one step of the first child that can step, then starts again.
- `steps=N` caps a `one`, `all` or `prl` node. Without it the node runs until nothing matches.
- `rule IN = OUT [p=N] [sym=S]` is one rule. Rows are joined by `/`. `*` matches any symbol in IN and leaves the cell alone in OUT. A match must change at least one cell.
- `p=N` (1 to 100, default 100) is the percent chance that a drawn match is applied. `sym=` picks the orientations: `none`, `flip`, `rot` or `all` (the default, all eight).
- The whole file is one `sequence`.

**Use.** There is no CLI and no door yet. From Rust, `Program::parse(text)` (`rewrite.rs:897`) reads the rules and `Program::run(seed)` (`rewrite.rs:957`) returns a `Grid`. `Grid::render()` (`rewrite.rs:383`) returns the grid as text. `Grid::hash()` (`rewrite.rs:397`) is its digest. The same text and seed always give the same grid.

**Types.** `RulesError` (`rewrite.rs:112`) carries the 1-based line and a `RulesErrorKind`. `RunError` (`rewrite.rs:199`) is `StepCap` or `WorkCap`. `Program` is at `rewrite.rs:312`, and `Grid` at `rewrite.rs:324`.

**Limits.** The caps are constants at `rewrite.rs:87-105`. They are `MAX_SIDE` 128, `MAX_SYMBOLS` 62, `MAX_PATTERN_SIDE` 16, `MAX_RULES` 256, `MAX_NODES` 256, `MAX_DEPTH` 16, `MAX_STEPS` 200 000, `MAX_WORK` 400 000 000 cell comparisons and `MAX_NODE_STEPS` 1 000 000. A run past the step cap or the work cap returns a `RunError` and no partial grid.

- Not wired yet: the `world.rewrite` door and the first map that uses it (MJ2 row, open).
- Nothing in the tree calls `engine_world::rewrite` yet. The module doc names arena and street bakes, and the UW12 and UW20 insula floor plans, as future consumers.

**Tests.** `node tools/gh-check.mjs cargo test -p engine-world --lib rewrite` runs 18 tests. They include `every_example_terminates_and_reproduces_byte_identically`, `a_known_rule_pair_gives_an_exact_grid_whatever_the_seed`, `markov_restarts_at_the_first_child_after_every_step`, `a_rule_set_that_never_settles_is_refused_not_cut_short`, `bad_rules_are_refused_with_their_line_number`, `maze_streets_are_one_connected_walled_block`, `insula_has_a_courtyard_and_doors_onto_the_street`, `ruined_wall_has_gaps_and_rubble` and `the_maze_hash_is_pinned`.

### 8.2 Service walkers (CITY1)

Caesar III's core city mechanic. A service building with workers spawns a walker. The walker roams the roads, and it sets the coverage of the houses near each road tile it reaches. The rules are read from Julius and Augustus, the open re-implementations (module doc, `crates/rome/src/city.rs:1-19`). Everything is integer. All storage is sized when the city is made, and `City::tick` never allocates (`crates/rome/src/city.rs:527`).

**The table.** `content/rome/service.toml` is embedded at build time. `ServiceRules::shipped()` (`crates/rome/src/service.rs:215`) loads it, and `ServiceRules::parse(text)` (`service.rs:227`) reads any text in the same shape. The sections are:

- `[clock]`: 50 ticks a day, and the tick of each daily chore.
- `[walk]`: 15 ticks a tile, a service radius of 2 tiles, a maximum coverage of 96, a decay of 1 a day, and a covered cap of 300.
- `[walker]`: a cap of 64 live walkers.
- `[service.prefect]`, `[service.temple]`, `[service.market]`, `[service.bathhouse]` and `[service.well]`: each sets its size, its labour count, its roam length and its spawn delays.
- `[labour]`: 60 percent of the people of working age work, and each category has a quantum.
- `[house.l0]` to `[house.l5]`: the house ladder.
- `[growth]`: one person a day, and two grace days before a house devolves.

Every section has a `ref` (its source) and a `certainty` (`reconstructed` or `invented`). The house ladder, the walker cap and the growth rule are ours, because the original data file is not in either source tree (the file header).

**Use.** `City::shipped(width, height, seed)` (`city.rs:258`) makes a city on the shipped table. `City::new(rules, width, height, seed)` (`city.rs:266`) takes any table. Sides are up to `MAX_SIDE` 64. `MAX_BUILDINGS` is 512, and `MAX_WALKERS` is 64 (`city.rs:25-31`).

- Lay out the city with `add_road` (`city.rs:373`), `add_roadblock` (`:384`), `set_roadblock_pass` (`:397`), `add_house` (`:426`) and `add_service(Service, x, y)` (`:435`).
- Run it with `run_ticks(n)` (`city.rs:514`), `run_days(n)` (`:521`) or `tick()` (`:527`).
- Read it with `building(at)` (`city.rs:326`), `walkers_alive()` (`:332`), `walker_at(slot)` (`:338`) and `tile(x, y)` (`:367`). `hash()` (`:984`) is the digest.

**Types.** `Service` (`service.rs:35`) is `Prefect`, `Temple`, `Market`, `Bathhouse` or `Well`. The well has no walker. The houses next to it simply have water. `City`, `Building`, `Tile`, `Kind`, `Labour` and `Stats` live in `city.rs`.

**Limits.**

- Not wired yet: nothing outside `crates/rome` calls `city` or `service`. The game has no city view (SHOW9 row, open).
- Open: the rest of the service loop, CITY2 row. It covers desirability, the market lady's goods, the prefect and engineer risk resets, fountains, the full house ladder, and the Latin names of walkers.
- `service.rs` has no tests of its own. The shipped table is tested through `city.rs`.

**Tests.** `node tools/gh-check.mjs cargo test -p engine-rome --lib city` runs 11 tests. They include `the_shipped_table_loads_with_julius_numbers`, `a_prefect_covers_the_houses_by_the_road_and_the_cover_decays`, `a_roadblock_stops_a_roaming_walker`, `a_roadblock_does_not_stop_a_walker_going_home`, `an_unstaffed_building_spawns_nothing`, `houses_evolve_by_coverage_and_devolve_without_it`, `a_short_labour_pool_is_shared_out_by_the_round_robin`, `the_walker_cap_holds`, `the_same_seed_gives_the_same_city` and `the_tick_never_grows_a_buffer`.
