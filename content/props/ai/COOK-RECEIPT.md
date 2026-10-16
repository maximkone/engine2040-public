# CT40 cook receipt — measured numbers only

Cooked 2026-10-09 on this box, headless Blender 5.2.0 LTS
(hash fbe6228777e7 built 2026-07-14 01:35:40). No invented figures:
every number below is pasted from the Blender `CT40_*` print lines, the
sha256 comparison script, or the licence harness output.

## Per-prop cook (Blender print lines)

- `CT40_BARREL rough_verts=2562 rough_polys=5120 game_tris=1126 verts=565 has_uv=True bytes=116112`
- `CT40_CRATE rough_verts=3752 rough_polys=3750 game_tris=2432 verts=1218 has_uv=True bytes=65224`
- `CT40_AMPHORA rough_verts=1281 rough_polys=1280 game_tris=1094 verts=612 has_uv=True bytes=31368`

Conformance: each game mesh is triangulated, carries one UV layer from
smart project, one Principled material, exports as glTF 2.0 GLB with +Y
up. Budget check: 1126, 2432 and 1094 tris are each <= 5000
(`PROP_MAX_TRIS`, `crates/assets/src/mesh_cook.rs:41`); maps per prop
are 0 (untextured), each <= 2k.

## Cook x2 identical (sha256 over two headless exports each)

- `ai_barrel.glb: bytes=116112 sha256=bfdd3a22297495a999a154d07e97984cafb8f3f2dea6b6c6e9bdd14e2927e3e8 x2_bytes=116112 x2_sha256=bfdd3a22297495a999a154d07e97984cafb8f3f2dea6b6c6e9bdd14e2927e3e8 x2_identical=True`
- `ai_crate.glb: bytes=65224 sha256=4e3feb856fc55f0fcc393ced10b0eaf34f39d2e40f98acdf0f433a23d88ae27f x2_bytes=65224 x2_sha256=4e3feb856fc55f0fcc393ced10b0eaf34f39d2e40f98acdf0f433a23d88ae27f x2_identical=True`
- `ai_amphora.glb: bytes=31368 sha256=ba600e7fbb1e37640d5fd01641a0d06bce45fac49b605cbc79a6fecc6a489372 x2_bytes=31368 x2_sha256=ba600e7fbb1e37640d5fd01641a0d06bce45fac49b605cbc79a6fecc6a489372 x2_identical=True`

Note: the first amphora build joined three primitives and its two
exports differed in bytes (same length, different hash). It was rebuilt
as a single screw-spun shell with no join; the rebuilt pair above is
byte-identical. The non-deterministic join variant is not shipped.

## Provenance digests (sha256 of the prompt text, computed locally)

- barrel `a wooden barrel with iron bands, game prop` = `16c0af48a97db735e5087bd606f1a63933494c7abea6e084e0e18fb6a37d8d45`
- crate `a beveled supply crate, game prop` = `089cc9d6efba8493a5e708c56bcb9dc49d364ba9b4232adfdc36e4adc7480baa`
- amphora `a clay amphora with two handles, game prop` = `bfed33f99e21af6a2cb774b748ed1d3b51c1cf36eb683e51e0eab1b2fff3a7a2`

Each `*.provenance.json` names generator `ct40-synthetic-rough`
(CC0-1.0, source in `README.md`), model `VAST-AI/TripoSG` (MIT),
components `ZhengPeng7/BiRefNet` (MIT) and `facebook/dinov2-large`
(Apache-2.0), the prompt digest above, and the seed (4243/4244/4245).

## Licence gate (refuses unreadable model terms)

- `python .lanes/ct40/content/ai-loop/test_license_gate.py`: `Ran 10 tests`, `OK`.
  Covers `UNKNOWN_GENERATOR` (unnamed record), `UNREADABLE_LICENSE`
  (empty, source-less and unknown tags), `FORBIDDEN_LICENSE`
  (CC-BY-NC-4.0, CC-BY-ND-4.0, CC-BY-NC-SA-4.0, Fab-UE-Only,
  NonCommercial) and `INFECTING_LICENSE` (GPL-3.0, AGPL-3.0).
- Engine gate cites (not re-run here; import-feature build exceeds this
  run's slot): `crates/assets/src/gen_intake.rs:408 check_license`
  (`license_refused` / `license_not_spdx`) and `:459 check_model`
  (`model_restricted` for Hunyuan3D, BRIA RMBG, diso and the other
  restricted prefixes at `:50`).

## Dedupe

Three artifact hashes, pairwise distinct, so no two props share bytes:

- `bfdd3a22297495a999a154d07e97984cafb8f3f2dea6b6c6e9bdd14e2927e3e8`
- `4e3feb856fc55f0fcc393ced10b0eaf34f39d2e40f98acdf0f433a23d88ae27f`
- `ba600e7fbb1e37640d5fd01641a0d06bce45fac49b605cbc79a6fecc6a489372`

Bytes differ too (116112 / 65224 / 31368). No dedupe merge applies.
