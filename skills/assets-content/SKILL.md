---
name: assets-content
description: "Use when you need new content or touch content/ or crates/assets: find, license-check, download, cook or budget meshes, textures, HDRIs or audio; PROVENANCE rows, ambientCG PBR sets, LFS pointers, tri counts, cook twice equal."
---

# Content: sources, cook, budgets

Plan: `docs/plan/content.md`. Take one content row: its text, its owned `content/` paths, its budgets and the consumer that must load the result. Commit by path (skill `commit-and-board`). Report files, provenance rows, budgets and the proof.

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
