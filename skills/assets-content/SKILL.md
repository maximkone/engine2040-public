---
name: assets-content
description: "Use when you need new content or touch content/ or crates/assets: find, license-check, download, cook or budget meshes, textures, HDRIs or audio; PROVENANCE rows, ambientCG PBR sets, LFS pointers, tri counts, cook twice equal."
---

# Content: sources, cook, budgets

Plan: `docs/plan/content.md`. Take one content row: its text, its owned `content/` paths, its budgets and the consumer that must load the result. Commit by path (skill `commit-and-board`). Report files, provenance rows, budgets and the proof.
Detail in `reference.md`: the MPFB lane, the Rocketbox look study, Mixamo packs, the ambientCG set procedure and its LFS pitfall, and the full budget and mesh tables.

## Realistic fighter into the game (the path the arena uses)

One command, from a Rocketbox avatar to a fighter the arena draws: `node tools/asset-gen/fighter.mjs --avatar Professions/Military_Male_01 --name rb_legionary --prove`. `--dry-run` prints the steps; `--help` the flags.

- Steps: fetch the avatar (`rocketbox.mjs`), build the Roman kit (`tools/blender/roman_kit.py`: gladius and scutum into `content/generated/roman_kit`, `--kit none` skips it), cook (`tools/blender/rocketbox_fighter.py`: body plus six clips on one skeleton, Mixamo sword-and-shield mocap, weapon and shield grafted to the hands), check the manifest row against the files, and with `--prove` open the real window.
- Output: `content/fighters/rocketbox/` (`glb/<name>_body.glb`, `glb/clips/<name>_<role>.glb` for idle, run, attack1-3, death, `<name>.cook.json`, `manifest.json`). The manifest is the game's only variant list (`fighter_bind::rocketbox_variants`): cooking a second avatar adds its row and keeps the others. No Rust edit adds a fighter.
- Clips: the proven pack map is `SWORD_AND_SHIELD` in `fighter.mjs`; `--clip attack3=<file.fbx>` swaps one role. The pack lives in `C:\forge-media\mixamo\sword_and_shield`.
- What the engine refuses, so you see it instead of a broken picture: a joint carrying scale (`BodyImportError::ScaledJoint`: apply scale in Blender), a pelvis outside 0.6 to 1.4 m (`check_body_units`: wrong units), fewer than 45 joints, not six clips, a body that does not match its manifest sha, a variant name outside lowercase/digits/_.
- `--prove` runs `E2040_BODIES=rocketbox target/debug/arena.exe ... --capture-dir target/clips/<name>` and passes only on exit 0 with `view=ok` on every capture line (skill `door-and-headless-runbook`). Then open the frames yourself: the checks catch broken views, not ugly ones.
- Proof in CI: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib rocketbox` (binder and family load, every variant held to the same bar).
- Tests: `node --test tools/asset-gen/fighter.test.mjs`.

The other lanes (MPFB bodies, the Rocketbox look study, Mixamo packs) are in `reference.md`; none of them reaches the game.

## 1. Sources and licence

- Find sources with websearch: Quaternius, Poly Haven, ambientCG, Kenney, 100STYLE and other CC0 or CC-BY libraries. Read each licence page live.
- Only CC0, or CC-BY with the credit recorded, goes into git. ambientCG and Poly Haven are CC0. Quote the licence line in the provenance record.
- Mixamo, Fab and similar raw files never enter git. They stay in git-ignored `content/local/` and ship only as cooked output.
- AI-generated textures only where the licence is read (a free Meshy output is CC BY, paid is commercial).
- A download is a file download: ask the owner in chat first. Say what, from where, how big. Record "owner approved the download in chat" with the date.
- Porting policy (owner decision D5, 2026-10-10): any donor code may be ported. No EVIDENCE row, no licence-gate paperwork. `deny.toml` is for Cargo crates only. Content assets still record source and terms (PROVENANCE).

## 2. Cook

1. Download into a staging folder: `content/_incoming/` (the `fetch-content.mjs` default) and `content/incoming/` are git-ignored (two carve-outs stay vendored: the studio HDRI `content/incoming/studio_small_09_1k.hdr` with its sidecar, and `content/fighters/incoming/batch-a/`). `node tools/fetch-content.mjs --list <file> [--dry-run]` lists total bytes and licences first. It sends one request per second with a User-Agent, and writes an md5 and a licence sidecar to `content/_incoming/`.
2. Cook through the engine's own paths: retarget, mesh cook (`crates/assets/src/mesh_cook.rs::cook_mesh`), texture cook (`crates/assets/src/texture.rs`). Stay inside the triangle and texture budgets. Use Blender headless for fixes when it is installed.
3. Write the source URL and SHA into the asset's cook manifest.
4. Prove it: the consumer loads it (window build or headless import test). The cook is identical twice (x2 equal bytes). Budgets hold.
5. If no caller loads it yet, report OPEN with a named follow-up row. Do not call it done.
6. Eyes: render a preview of every cooked asset and open it.

## 3. PBR sets and the arena PROVENANCE

Sets live in `content/arena/sources/<AssetId>/` and get one row each in `content/arena/sources/PROVENANCE.md` with exact on-disk sizes. Check with `node tools/arena-sources.mjs` (exit 1 lists every problem: missing row, a licence cell without `CC0`, a missing Color/Normal/Roughness, a stray file, a wrong size, a folder over 60 MiB); test `node --test tools/arena-sources.test.mjs`.

Three rules that bite:

- `NormalGL` only (OpenGL, +Y up). Never flip a GL normal again; a DX normal is the GL one with green inverted.
- `Color` is sRGB, the others are linear data. Say so in the cook.
- Sizes in PROVENANCE are real-file sizes from the PC working tree. Under `.gitattributes` the git blob for images and `.glb` is an LFS pointer, so a runner checkout reports size mismatches. `node tools/public-export.mjs` turns pointers back into bytes.

The full seven-column format, the file list to keep, the order of work and the LFS pitfall are in `reference.md`.

## 4. Budgets and 5. Mesh counts

The cooker reports, the caller decides, and over-budget never lands silently. PASS needs `tri_count` plus `validate_mesh` green and the cook identical twice — never a picture or a vert count alone.

- One `Caps` table (`crates/assets/src/budget.rs::Caps`): POTATO 1500 tris, COMFORT 4000, both 75 bones, 4 influences, one material and one 1024 px atlas. `check` takes plain `MeshStats` and returns findings that name the limit and the measured value.
- The cook's own limits are separate: `mesh_cook.rs::HERO_MAX_TRIS` (40000) and `PROP_MAX_TRIS` (5000). Do not mix them up with `Caps`.
- Tris are faces (`indices / 3`), winding stays CCW, verts are shared, and each LOD in `LOD_RATIOS` holds fewer tris.
- Do: one table, both numbers at the gate, refuse the file. Don't: limits inside cooks, silent clamps, or counting verts as tris.

The nine budget rules and the nine mesh rules, with their symbols and flags, are in `reference.md`.

## 6. Eyes

Render a preview of every cooked asset (Blender headless or the engine capture) and open it. Check silhouette, scale next to a 1.8 m reference, UV stretch, seams and rig breakage. Refuse what looks broken even if the import test passes.

Tangents and the tangent-handedness check: skill `render-pipeline-math` check 6. Pixel and lighting proof: skill `render-lighting`.

Sources: Game Engine Gems 1 ch. 1.3; Lengyel, Foundations of Game Engine Development Vol 1 ch. 3.1-3.2.
