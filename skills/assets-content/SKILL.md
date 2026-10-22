---
name: assets-content
description: "Use when you need new content or touch content/ or crates/assets: find, license-check, download, cook or budget meshes, textures, HDRIs or audio; PROVENANCE rows, ambientCG PBR sets, LFS pointers, tri counts, cook twice equal."
---

# Content: sources, cook, budgets

Plan: `docs/plan/content.md`. Take one content row: its text, its owned `content/` paths, its budgets and the consumer that must load the result. Commit by path (skill `commit-and-board`). Report files, provenance rows, budgets and the proof.

## Make a fighter body

One command per lane. Each runs its steps in order and stops at the first failure, which names the step.

- MPFB body (the lane the game uses now): `node tools/asset-gen/character.mjs --from mpfb --name <id> --faction ashkin|wardens [--height 1.9] [--bulk 1.0] [--out target/chargen/<id>]`. It runs mpfb_body, proportion_pass, style_cook, the 30 m gate, rig22 and the page budget check (LOD0 tris, 256 px atlas cap, first-bundle and memory bytes). It writes `receipt.json` in the out dir and prints one `RECEIPT` line. Exit 3 means the gate or the budget said no; exit 1 means a step failed.
- Concept image (the older lane): `node tools/asset-gen/character.mjs --concept <image> --name <id> --faction ashkin`. It runs sf3d, style_cook, the gate and rig22.
- Tests: `node --test tools/asset-gen/character.test.mjs`.
- Not wired yet: a cooked MPFB body does not reach `content/fighters` or `fighters.e2bundle`. The receipt's `next_step` says what is missing.

## Realistic fighter into the game (the path the arena uses)

One command, from a Rocketbox avatar to a fighter the arena draws: `node tools/asset-gen/fighter.mjs --avatar Professions/Military_Male_01 --name rb_legionary --prove`. `--dry-run` prints the steps; `--help` the flags.

- Steps: fetch the avatar (`rocketbox.mjs`), build the Roman kit (`tools/blender/roman_kit.py`: gladius and scutum into `content/generated/roman_kit`, `--kit none` skips it), cook (`tools/blender/rocketbox_fighter.py`: body plus six clips on one skeleton, Mixamo sword-and-shield mocap, weapon and shield grafted to the hands), check the manifest row against the files, and with `--prove` open the real window.
- Output: `content/fighters/rocketbox/` (`glb/<name>_body.glb`, `glb/clips/<name>_<role>.glb` for idle, run, attack1-3, death, `<name>.cook.json`, `manifest.json`). The manifest is the game's only variant list (`fighter_bind::rocketbox_variants`): cooking a second avatar adds its row and keeps the others. No Rust edit adds a fighter.
- Clips: the proven pack map is `SWORD_AND_SHIELD` in `fighter.mjs`; `--clip attack3=<file.fbx>` swaps one role. The pack lives in `C:\forge-media\mixamo\sword_and_shield` (section Mixamo packs below).
- What the engine refuses, so you see it instead of a broken picture: a joint carrying scale (`BodyImportError::ScaledJoint`: apply scale in Blender), a pelvis outside 0.6 to 1.4 m (`check_body_units`: wrong units), fewer than 45 joints, not six clips, a body that does not match its manifest sha, a variant name outside lowercase/digits/_.
- `--prove` runs `E2040_BODIES=rocketbox target/debug/arena.exe ... --capture-dir target/clips/<name>` and passes only on exit 0 with `view=ok` on every capture line (skill `door-and-headless-runbook`). Then open the frames yourself: the checks catch broken views, not ugly ones.
- Proof in CI: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib rocketbox` (binder and family load, every variant held to the same bar).
- Tests: `node --test tools/asset-gen/fighter.test.mjs`.

## Realistic characters (Rocketbox), the look study

This lane is for studying an avatar (LODs, the 30 m gate, the page budget). It does not reach the game; the section above does.

Microsoft Rocketbox avatars (MIT, 115 rigged humans, 3ds Max Biped skeleton) are the realistic body. One command fetches one avatar and cooks it: `node tools/asset-gen/character.mjs --from rocketbox --avatar Adults/Male_Adult_08 --name rb_male08 [--home C:\forge-media\rocketbox] [--tris0 7400] [--out target/chargen/rb_male08]`. Groups are Adults, Children and Professions.

- The fetch is `tools/asset-gen/rocketbox.mjs`: the FBX and textures go once into `--home` (never into the repo), resumable, one download at a time, 3 MB/s cap. `--dry-run` shows the plan and makes no network call once `textures.txt` is cached.
- The cook is `tools/blender/rocketbox_cook.py`: bones renamed to the contract charset (`bip01_l_upperarm`, the names `crates/assets/src/rocketbox_map.rs` expects), colour and normal maps at 1024 px, LOD0 to LOD3 GLBs with the skin kept, `hero.png` at 80% of the frame, `cook.json` with the MIT licence and the source sha256.
- Out dir: `cook/` (lod0..lod3.glb, textures/, hero.png, cook.json), `gate/silhouette_30m.png`, `receipt.json`, and one `RECEIPT` line. Exit 3 means the 30 m gate or the page budget said no; exit 1 means a step failed. Look at `cook/hero.png` before anything else.
- Same gates as the other lanes: the 30 m fill band (0.40 to 0.70) and `budgetCheck`. As of 2026-10-22 both proven avatars stop there: Male_Adult_08 fills 0.30 at 30 m and its lane is 6.7 MB against the 2.04 MB first-bundle budget; Military_Male_01 fills 0.35 and its lane is 12.8 MB. A human with arms out fills less of its box than a blockout, so the band itself is the open question.
- Not the game path: `fighter.mjs` (above) is. The receipt's `next_step` says so.
- Tests: `node --test tools/asset-gen/character.test.mjs` (no network, no Blender).

## Mixamo packs

Mixamo animation packs, fetched by `tools/asset-gen/mixamo.mjs` into `C:\forge-media\mixamo\<pack_slug>\` (outside the repo; never commit them). The owner signs in once; the tool reuses that browser profile (`C:\forge-media\mixamo-lane\profile2`, override with env `MIXAMO_PROFILE`).

- Sign in once (owner, by hand; the tool never types credentials): `node tools/asset-gen/mixamo.mjs --login`
- List packs with animation counts: `node tools/asset-gen/mixamo.mjs --list sword` (exit 4 means not signed in: run `--login` first)
- Fetch one pack as FBX, T-pose with skin, no keyframe reduction: `node tools/asset-gen/mixamo.mjs --pack "Sword and Shield Pack" [--fps 60] [--out <dir>]`
- Plan only, no browser: `node tools/asset-gen/mixamo.mjs --pack "<Pack Name>" --dry-run`
- API trace for debugging: add `--trace` (method, path and status to stderr; never headers or bodies)
- Each pack writes `PROVENANCE.json` (source "Adobe Mixamo", licence note: royalty-free for use in games; do not redistribute the raw files as a library). A pack with one FBX per animation plus one is skipped.
- Tests: `node --test tools/asset-gen/mixamo.test.mjs`

## 1. Sources and licence

- Find sources with websearch: Quaternius, Poly Haven, ambientCG, Kenney, 100STYLE and other CC0 or CC-BY libraries. Read each licence page live.
- Only CC0, or CC-BY with the credit recorded, goes into git. ambientCG and Poly Haven are CC0. Quote the licence line in the provenance record.
- Mixamo, Fab and similar raw files never enter git. They stay in git-ignored `content/local/` and ship only as cooked output.
- AI-generated textures only where the licence is read (a free Meshy output is CC BY, paid is commercial).
- A download is a file download: ask the owner in chat first. Say what, from where, how big. Record "owner approved the download in chat" with the date.
- Porting policy (owner decision D5, 2026-10-10): any donor code may be ported. No EVIDENCE row, no licence-gate paperwork. `deny.toml` is for Cargo crates only. Content assets still record source and terms (PROVENANCE).

## 2. Cook

1. Download into a staging folder: `content/_incoming/` (the `fetch-content.mjs` default) and `content/incoming/` are git-ignored (two carve-outs stay vendored: the studio HDRI `content/incoming/studio_small_09_1k.hdr` with its sidecar, and `content/fighters/incoming/batch-a/`). `node tools/fetch-content.mjs --list <file> [--dry-run]` lists total bytes and licences first. It sends one request per second with a User-Agent, and writes an md5 and a licence sidecar to `content/_incoming/`.
2. Cook through the engine's own paths: retarget, mesh cook (`crates/assets/src/mesh_cook.rs::cook_mesh`), texture cook (`crates/assets/src/texture.rs`). Stay inside the triangle and texture budgets (sections 4 and 5). Use Blender headless for fixes when it is installed.
3. Write the source URL and SHA into the asset's cook manifest.
4. Prove it: the consumer loads it (window build or headless import test). The cook is identical twice (x2 equal bytes). Budgets hold.
5. If no caller loads it yet, report OPEN with a named follow-up row. Do not call it done.
6. Eyes (section 6): render a preview of every cooked asset and open it.

## 3. PBR sets for the arena

Raw, uncooked sets live in `content/arena/sources/<AssetId>/`. Cooking to KTX2 (BC on desktop, ASTC where available) is the separate second half of row VR2 and reads from here.

- Folder name is the ambientCG id, for example `Bricks097`. It holds only image files and `LICENCE.txt`.
- Take the 1K JPG zip: `https://ambientcg.com/get?file=<AssetId>_1K-JPG.zip`. Take 2K only if a row needs it (size matters for the web bundle). Keep `Color`, `NormalGL`, `Roughness`, and `AmbientOcclusion`, `Metalness`, `Displacement` when the set has them. Drop the DirectX normal, `.blend`, `.usdc`, `.mtlx`, `.tres` and preview PNGs.
- `NormalGL` only: OpenGL convention, +Y up. A DX normal is the GL one with the green channel inverted, so never flip a GL one again. Two normal conventions in one folder invite a flipped-green lighting bug.
- `Color` is sRGB. The others are linear data. Say so in the cook.
- `LICENCE.txt` holds the source URL, the licence line and URL, the download date, the normal convention and what was dropped. Copy a neighbour (for example `content/arena/sources/Planks039/LICENCE.txt`).

The PROVENANCE row: `content/arena/sources/PROVENANCE.md` has one table row per folder, seven columns:

`asset id | source URL | licence (must contain CC0) | downloaded date | files (bytes) | normal | arena material`

- The files cell lists every file as `name size` separated by `;`, `LICENCE.txt` included. Sizes are exact from the disk (`(Get-Item f).Length`). Never type a size by hand. A set with no AO map says so in that cell.
- Update the "Total on disk" line.

Check: `node tools/arena-sources.mjs` prints `arena sources ok` (exit 0) or one problem per line (exit 1). Test: `node --test tools/arena-sources.test.mjs`. It reads only, no network. It fails when:

- a folder has no row, or a row has no folder;
- the licence cell lacks `CC0`;
- a Color, Normal or Roughness image is missing;
- `LICENCE.txt` is missing or lacks `CC0`;
- a file other than an image (`jpg`, `jpeg`, `png`, `exr`) or `LICENCE.txt` is present (delete the zip, the `.blend` and the preview PNG);
- a row's `name size` does not match the disk;
- the folder total is over 60 MiB (`SIZE_BUDGET_BYTES`). Do not put 2K or 4K sets in git for a web target.

`node tools/workspace-check.mjs` also fails a NEW tracked non-LFS blob over 5 MB.

Order of work for a new set: ask the owner. Fetch (`tools/fetch-content.mjs`, or take the zip by hand). Unzip into a scratch folder and keep only the files above. Copy to `content/arena/sources/<AssetId>/`. Write `LICENCE.txt`. Add the PROVENANCE row. Run `node tools/arena-sources.mjs`. Commit by path. Update the row's progress note (skill `commit-and-board`).

### The LFS pointer pitfall

- `.gitattributes` sends `content/**/*.jpg`, `.png`, `.hdr`, `.exr`, `.ktx2`, `.glb` and more to Git LFS. The git blob is a tiny pointer (`version https://git-lfs.github.com/spec/v1`). The working tree holds the real bytes.
- So `git show HEAD:<path>`, `git archive` and a runner checkout without LFS give pointers, and `arena-sources.mjs` then reports size mismatches. Sizes in `PROVENANCE.md` are real-file sizes. Run the check on the PC working tree.
- `node tools/public-export.mjs` turns pointers into real bytes from the local LFS store (the public mirror has no LFS). If it reports `LFS object missing locally`, run `git lfs fetch` or `git lfs pull` for that path first. Never commit a pointer you wrote by hand.
- Do not run `git lfs migrate` or rewrite history.

## 4. Budgets: one `Caps` table

The cooker reports. The caller decides. Over-budget never lands silently. Source: Game Engine Gems 1 ch. 1.3.

| # | Rule | Flag | Symbols |
|---|---|---|---|
| 1 | One caps table: POTATO and COMFORT live as one `Caps` struct. | magic 4000 or 1500 scattered in cooks | `crates/assets/src/budget.rs::Caps`, `POTATO`, `COMFORT` |
| 2 | Stats first: `check` takes a plain `MeshStats` and returns `Vec<BudgetFinding>`. | reading GLB bytes inside the budget test | `budget.rs::check`, `mesh_cook.rs::tri_count` |
| 3 | Each finding names the limit and the measured value. | a bare refusal with no counts | `import_repair.rs::CODE_BUDGET_TRIS` |
| 4 | 75 bones per section, enforced before skinning. | a 76-bone section that cooks clean | `budget.rs::POTATO`, `import_repair.rs::BudgetSet` |
| 5 | Skin weights clamp to 4 influences with renormalized sums. | a 5th weight dropped with no finding | `skin_import.rs::MAX_INFLUENCES`, `WEIGHT_SUM_EPS` |
| 6 | One material and one shared atlas per skin. | two materials on a crowd member | `cooked.rs::validate_material`, `lib.rs::GATES` |
| 7 | Texture pixels are capped. Potato base color is 1024x1024 (`Caps::max_tex_px` 1048576). The importer ceiling is separate: `MAX_TEXTURE_DIM` 4096, `MAX_TEXTURE_PIXELS` 16777216. | a 2k albedo on a potato fighter | `texture.rs` |
| 8 | The budget module only reports. The caller allocates and refuses. | `new` inside the budget path with no owner | `lib.rs`, `bundle_cook.rs::cook_and_write` |
| 9 | `check` is pure: x2 equal bytes, no clock, threads or hash order. | `Instant::now` or `HashMap` iteration in the budget path | `import.rs::gate_results`, `crates/perf/src/vram.rs` |

Caps today (`crates/assets/src/budget.rs`):

| Table | Tris | Bones | Influences | Materials per skin | Texture px |
|---|---|---|---|---|---|
| `POTATO` | 1500 | 75 | 4 | 1 | 1048576 |
| `COMFORT` | 4000 | 75 | 4 | 1 | 1048576 |

The cook has its own tri limits per kind: `mesh_cook.rs::HERO_MAX_TRIS` (40000) and `PROP_MAX_TRIS` (5000). Do not mix them up with `Caps`.

Do: keep one `Caps` table, report both numbers, refuse the file at the gate. Don't: sprinkle limits in cooks, clamp silently, or let the budget allocate from the main heap.

## 5. Mesh counts

PASS needs `tri_count` plus `validate_mesh` green and x2 equal, never a picture alone. Source: Lengyel Vol 1 ch. 3.1-3.2.

| # | Rule | Flag |
|---|---|---|
| 1 | Tris are faces: `mesh_cook.rs::tri_count` returns indices / 3 from `cooked.rs::CookedMesh`, not verts. | a vert count passed as tris hides a 3x error |
| 2 | Indices form triples: `cooked.rs::validate_mesh` refuses `index_len % 3 != 0` and out-of-range indices from `mesh_import.rs::import_gltf_bytes`. | stray indices that cook |
| 3 | Winding stays CCW. `validate_mesh` keeps outward faces. Zero-area triangles are refused by `ZERO_AREA_M2` (1e-12) in `mesh_cook.rs`. | flipped faces with dark lighting |
| 4 | Verts are shared: one box is 8 verts and 12 tris. `mesh_cook.rs::optimize_index_order` keeps sharing. | 36 verts for a box inflates budgets |
| 5 | Closed hulls satisfy V-E+F=2 (`mesh_cook.rs::CollisionHull`). Open meshes are marked open, not forced shut. | an open cape counted as closed |
| 6 | Face normals are `cross(edge1, edge2)`, unit length. A missing NORMAL is filled area-weighted in `mesh_import.rs`. | unnormalized normals that shade seams |
| 7 | Each LOD in `mesh_cook.rs::LOD_RATIOS` (1.0, 0.5, 0.25, 0.125) holds fewer tris via `simplify` in `crates/assets/src/collapse.rs`. `lod_swap.rs` checks with `tri_count`. | LOD1 larger than LOD0 |
| 8 | The budget check reads plain stats (tris, bones, influences, materials, tex_px) and reports both numbers (section 4). | GLB bytes read inside the budget test |
| 9 | `tri_count` plus `cook_mesh` are pure: x2 equal bytes, no clock or hash order. | run-to-run tri drift |

Do: count faces, keep winding, share verts, report both numbers at the gate. Don't: count verts as tris, flip winding, or split verts to pass a budget.

## 6. Eyes

Render a preview of every cooked asset (Blender headless or the engine capture) and open it. Check silhouette, scale next to a 1.8 m reference, UV stretch, seams and rig breakage. Refuse what looks broken even if the import test passes.

Tangents and the tangent-handedness check: skill `render-pipeline-math` check 6. Pixel and lighting proof: skill `render-lighting`.

Sources: Game Engine Gems 1 ch. 1.3; Lengyel, Foundations of Game Engine Development Vol 1 ch. 3.1-3.2.
