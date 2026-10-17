# CT77 arena src commit receipt (crate + torch)

The pair is committed only with this receipt current: cook x2 identical, SP5 0 findings, SHAs and budgets named.

<!-- gen:src-receipt -->
| File | Bytes | SHA-256 | Manifest | LOD0 tris | Tri budget | Byte budget |
| --- | --- | --- | --- | --- | --- | --- |
| `src/crate.gltf` | 6879480 | `322d4cf3a807ab572ce6d9af25b0707c7793752238fd0eeaa27d5436fd84b1b7` | match | 1536 | 5000 | 8388608 |
| `src/torch.gltf` | 6821759 | `b04cde0a248a1ea269559736dafe5994dcc756b1eb155daa6f5343d37162f090` | match | 384 | 5000 | 8388608 |

- Cook x2: bundle identical_twice true, spatial identical_twice true (`report.json`).
- SP5 spatial over the placed kit: 11 actors, 0 findings, verdict clean, hash `cbf29ce484222325`.
- Golden hash `d57b5c8ae3096614` is untouched: the kit is float content outside the hashed simulation.
- Gate: `node tools/arena-report.mjs` fails when either file's bytes move without this receipt being rewritten with `--write`.
<!-- /gen:src-receipt -->
