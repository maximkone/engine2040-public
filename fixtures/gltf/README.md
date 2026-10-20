# fixtures/gltf

The glTF conformance corpus of row CT10. The test that reads it is
`crates/assets/tests/gltf_conformance.rs`; run it with
`cargo test -p engine-assets --features import,registry --test gltf_conformance`.

Every model here is selected on purpose, carries its licence, and is either a
model the cook must take all the way through (import, place, save, reopen,
play) or a model the cook must refuse with a typed error and never a panic.

The Khronos glTF sample models live next door in `fixtures/khronos/`; their
licence rows are in `docs/EVIDENCE.md` (CC0-1.0). The test reads that ledger
before it cooks anything: a model whose licence row is missing never reaches
the cook.

## The models

| Model | File | Source | License | What it proves |
|---|---|---|---|---|
| `khronos-triangle` | `khronos/Triangle.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 | an indexed triangle imports, places, saves, reopens and plays as a static asset |
| `khronos-triangle-no-indices` | `khronos/TriangleWithoutIndices.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 | a mesh without an index buffer cooks to the same indexed mesh |
| `khronos-animated-triangle` | `khronos/AnimatedTriangle.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 | a rotation clip imports, survives the save and reopen, and moves when it plays |
| `blender-actions` | `gltf/blender_actions.gltf` | fresh write for CT10 | CC0-1.0 | one glTF animation per Blender action (per-action export) |
| `blender-broadcast` | `gltf/blender_broadcast.gltf` | fresh write for CT10 | CC0-1.0 | one animation with every frame sampled (NLA broadcast export) |
| `blender-pbr` | `gltf/blender_pbr.gltf` | fresh write for CT10 | CC0-1.0 | the PBR mapping of a Principled BSDF: factors, base-colour image and texture slot |
| `blender-step` | `gltf/blender_step.gltf` | fresh write for CT10 | CC0-1.0 | a `STEP` sampled clip is refused with a typed error, never a panic |
| `broken-buffer` | `gltf/broken_buffer.gltf` | fresh write for CT10 | CC0-1.0 | a buffer that claims more bytes than it carries is refused with a typed error |

## Blender acceptance

The five `gltf/` files follow the shapes the Blender glTF exporter writes
(animation sampling, one animation per action against one broadcast animation,
and the Principled BSDF mapping). The exporter is
[KhronosGroup/glTF-Blender-IO](https://github.com/KhronosGroup/glTF-Blender-IO)
(Apache-2.0), used here as an acceptance reference only: no donor code and no
donor bytes are copied, the files are written fresh for this corpus.

Blender stays outside the engine. It is driven over MCP by the content lane
(row CT4); the engine never links it and this corpus never needs it.

## Licence rows

`blender-actions`, `blender-broadcast`, `blender-pbr`, `blender-step` and
`broken-buffer` are written fresh for this repository and carry CC0-1.0. The
three Khronos sample models are covered by the rows in `docs/EVIDENCE.md`.
