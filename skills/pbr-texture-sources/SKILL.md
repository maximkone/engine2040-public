---
name: pbr-texture-sources
description: "Use when you add or change PBR texture sets for the arena or other content in engine2040: where raw CC0 sets live (content/arena/sources), the PROVENANCE.md row, ambientCG download rules, the LFS pointer pitfall, and tools/arena-sources.mjs."
---

# PBR texture sources

Raw, uncooked source sets for the arena kit live in `content/arena/sources/<AssetId>/`. Cooking to KTX2 (BC on desktop, ASTC where available) is the separate second half of row VR2 and reads from here. Policy: `docs/plan/content.md`, skill `engine-assets`.

## Licence rules
- Only CC0 (or CC-BY with the credit recorded) goes into git. ambientCG and Poly Haven are CC0. Read the licence page live and quote its line in `PROVENANCE.md`.
- Mixamo, Fab and similar raw files never enter git: git-ignored `content/local/`, shipped only as cooked output.
- AI-generated textures only where the licence is read (a free Meshy output is CC BY, paid is commercial).
- Porting policy (owner 2026-10-10): no EVIDENCE row and no licence-gate paperwork; the provenance row is enough. `deny.toml` is for Cargo crates only.
- A download is a file download: ask the owner in chat first and say what, from where, how big. VR2 landing 1 records "owner approved the download in chat" with the date.

## What one set holds
- Folder `content/arena/sources/<AssetId>/` (the ambientCG id, for example `Bricks097`) with only the image files and `LICENCE.txt`.
- Take the 1K JPG zip: `https://ambientcg.com/get?file=<AssetId>_1K-JPG.zip` (2K only if a row needs it; size matters for the web bundle). Keep `Color`, `NormalGL`, `Roughness`, and `AmbientOcclusion`, `Metalness`, `Displacement` when the set has them. Drop the DirectX normal, `.blend`, `.usdc`, `.mtlx`, `.tres` and preview PNGs.
- `NormalGL` only: OpenGL convention, +Y up. A DX normal is the GL one with the green channel inverted, so never flip a GL one again.
- `Color` is sRGB, the others are linear data. Say so in the cook, not here.
- `LICENCE.txt` holds the source URL, the licence line and URL, the download date, the normal convention and what was dropped (copy a neighbour).

## The PROVENANCE row
`content/arena/sources/PROVENANCE.md` has one table row per folder, seven columns:
`asset id | source URL | licence (must contain CC0) | downloaded date | files (bytes) | normal | arena material`.
- The files cell lists every file as `name size` separated by `;`, `LICENCE.txt` included, sizes exact from disk (`(Get-Item f).Length`). A set with no AO map says so in that cell.
- Update the "Total on disk" line.

## Check: tools/arena-sources.mjs
- `node tools/arena-sources.mjs` prints `arena sources ok` (exit 0) or one problem per line (exit 1). Test: `node --test tools/arena-sources.test.mjs`. Pure reads, no network.
- It fails when: a folder has no row, a row has no folder, the licence cell lacks `CC0`, a Color/Normal/Roughness image is missing, `LICENCE.txt` is missing or lacks `CC0`, a file other than an image (`jpg`, `jpeg`, `png`, `exr`) or `LICENCE.txt` is present, a row's `name size` does not match the disk, or the folder total is over 60 MiB.
- `node tools/workspace-check.mjs` also fails a NEW tracked non-LFS blob over 5 MB.

## The LFS pointer pitfall
- `.gitattributes` sends `content/**/*.jpg`, `.png`, `.hdr`, `.exr`, `.ktx2`, `.glb` and more to Git LFS. The git blob is a tiny pointer (`version https://git-lfs.github.com/spec/v1`); the working tree holds the real bytes.
- So `git show HEAD:<path>`, `git archive` and a runner checkout without LFS give pointers, and `arena-sources.mjs` then reports size mismatches. Sizes in `PROVENANCE.md` are real-file sizes: run the check on the PC working tree.
- `node tools/public-export.mjs` turns pointers into real bytes from the local LFS store (the public mirror has no LFS). If it reports `LFS object missing locally`, run `git lfs fetch` / `git lfs pull` for that path first; never commit a pointer you wrote by hand.
- Installed here: `git lfs version` works. Do not run `git lfs migrate` or rewrite history.

## Order of work for a new set
1. Ask the owner (download approval). 2. Fetch (`node tools/fetch-content.mjs --list <file> [--dry-run]` lists total bytes and licences first, one request per second, sends a User-Agent, writes md5 and a licence sidecar to `content/_incoming/`; or take the zip by hand), unzip into a scratch folder, keep only the files above. 3. Copy to `content/arena/sources/<AssetId>/`, write `LICENCE.txt`, add the PROVENANCE row. 4. `node tools/arena-sources.mjs`. 5. Commit by path (skill `commit-by-path`); update the row's progress note on the board (skill `board-row-writing`).

## Pitfalls
- Hand-typing a byte size: copy from the disk listing, the check compares exactly.
- Keeping the DX normal "just in case": two normal conventions in one folder invite a flipped-green lighting bug. Keep `NormalGL` only.
- Leaving the zip or a `.blend` in the folder: the check fails on any non-image file. A leftover preview PNG passes the check only if its bytes are listed in the row, so delete it instead.
- Putting 2K or 4K sets in git for a web target: the first bundle bar is 60 MB compressed for everything.
