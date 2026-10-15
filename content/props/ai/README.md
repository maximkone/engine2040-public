# CT40 — AI prop intake (`content/props/ai/`)

Three game props through the CT4 loop. Tier B, Size S. Needs CT4 (done).

## What this is

Offline image-to-3D weights (TRELLIS, TripoSR) cannot run on this box:
no GPU runner, no weight fetch. These three props are Blender 5.2
synthetic rough meshes of the same class (dense shells, no UVs) that run
the same CT4 rough-to-prop path: rough mesh, decimate, unwrap, bake
placeholder, export, provenance. The MIT image-to-3D representative is
`VAST-AI/TripoSG` (MIT, `github.com/VAST-AI-Research/TripoSG@fc5c409`);
TRELLIS/TripoSR rough meshes enter through the same licence gate when a
runner is available. Nothing here claims a TRELLIS byte was cooked.

## Decimate / unwrap / bake path

1. Rough mesh, no UVs: barrel is a subdivided icosphere stretched to
   barrel proportions; crate is a subdivided cube with a small bevel;
   amphora is a screw-spun silhouette at 128 steps.
2. Decimate modifier at a fixed ratio (barrel 0.22, crate 0.30,
   amphora 0.45), applied. No randomness anywhere.
3. Edit-mode select-all, smart UV project (angle 66, island margin 0.02),
   quads convert to tris.
4. One Principled BSDF material (base color, roughness 0.8-0.85).
   No image bake: untextured prop path, 0 maps.
5. Select-only GLB export with +Y up and applied transforms.
6. Provenance JSON per prop: generator, model id plus SPDX licence,
   component chain, prompt text plus its sha256 digest, seed, artifact
   sha256 plus bytes, rough and game counts, verifier flags.

The engine-side bundle path this mirrors is
`crates/assets/src/gen_intake.rs`: licence gate first
(`check_model` line 459, `check_license` line 408, allow-list line 32,
restricted prefixes line 50), then repair, pose, simplify to at most
`PROP_MAX_TRIS` (`crates/assets/src/mesh_cook.rs` line 41: 5,000),
smooth normals, prop cook with LOD chain and hulls, E204BNDL bundle,
one JSON provenance row (`intake` line 1240). Untextured output matches
that path's `textured:false` shape.

## Budgets

Each prop holds at most 5,000 tris and at most 2k maps. All three are
untextured (0 maps). Measured game tris: barrel 1126, crate 2432,
amphora 1094. Receipt: `COOK-RECEIPT.md`.

## Licence gate

Two gates refuse unreadable model terms before any cook:

- CT4 loop gate `content/ai-loop/license_gate.py`: `check_generator`
  passes CC0-1.0/MIT/Apache-2.0 only with a readable source; empty,
  unknown, non-commercial, EULA and GPL/AGPL inputs raise typed
  `LicenseRefusal` codes. Harness `test_license_gate.py`: 10 tests OK.
- Engine gate `gen_intake.rs`: `check_license` returns `license_refused`
  or `license_not_spdx`; `check_model` returns `model_restricted` for the
  restricted prefixes (Hunyuan3D, BRIA RMBG, diso, PartPacker,
  MeshAnything, FLUX dev, Pixal3D). A refused model costs no cook.

## Reproduce

From the repo root, headless Blender 5.2, deterministic:

- `blender --background --python <script> -- content/props/ai/ai_barrel.glb`
- `blender --background --python <script> -- content/props/ai/ai_crate.glb`
- `blender --background --python <script> -- content/props/ai/ai_amphora.glb`

Build scripts are operator one-offs (kept outside git with the run
notes). Each prop re-exports byte-identical: same command twice gives
the same sha256 (receipt pastes all three).

## Files

- `ai_barrel.glb`, `ai_crate.glb`, `ai_amphora.glb`: the three props.
- `ai_barrel.provenance.json`, `ai_crate.provenance.json`,
  `ai_amphora.provenance.json`: per-file provenance.
- `COOK-RECEIPT.md`: measured cook, conformance and dedupe numbers.
