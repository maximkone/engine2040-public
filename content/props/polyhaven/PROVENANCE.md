# content/props/polyhaven — provenance

Every model here is Poly Haven, CC0-1.0, fetched from the Poly Haven file API
(`https://api.polyhaven.com/files/<id>`). No contributor attribution is required
by CC0; the author is recorded anyway so a reviewer can trace a file.

| file | source | author | licence | bytes | digest |
|---|---|---|---|---|---|
| `marble_bust_01/marble_bust_01_1k.gltf` | Poly Haven `marble_bust_01`, glTF 1k | Rico Cilliers | CC0-1.0 | 2,793 | md5 `599d6dd71c2dcdfa064b1c1d85882f4` |
| `marble_bust_01/marble_bust_01.bin` | the same entry, buffer for the glTF | Rico Cilliers | CC0-1.0 | 416,608 | — |
| `marble_bust_01/textures/marble_bust_01_diff_1k.jpg` | the same entry, diffuse 1k | Rico Cilliers | CC0-1.0 | 160,337 | — |
| `marble_bust_01/textures/marble_bust_01_nor_gl_1k.jpg` | the same entry, OpenGL normal 1k | Rico Cilliers | CC0-1.0 | 124,627 | — |
| `marble_bust_01/textures/marble_bust_01_rough_1k.jpg` | the same entry, roughness 1k | Rico Cilliers | CC0-1.0 | 193,918 | — |
| `marble_bust_01/marble_bust_01.glb` | the five files above, wrapped by `tools/polyhaven-glb.mjs` | Rico Cilliers | CC0-1.0 | 897,256 | — |

Licence text: <https://polyhaven.com/license> ("All assets on Poly Haven are
released under the CC0 license").

`marble_bust_01.glb` is the one file the intake reads
(`crates/assets/src/gen_intake.rs`, `PROP_MAX_TRIS` = 5,000). It is a pure wrap:
the glTF keeps its own document, the `.bin` becomes the GLB binary chunk and the
three JPEGs move into buffer views, so no external URI survives. Re-running the
wrapper on these inputs gives the same bytes (`node tools/polyhaven-glb.mjs
content/props/polyhaven/marble_bust_01 marble_bust_01_1k.gltf marble_bust_01.glb`).

Source triangles: 17,456 (the Poly Haven listing). The intake simplifies LOD0 to
the prop budget; the row `SW121` in `docs/SPRINT-39-steal-wave.md` carries the
acceptance.
