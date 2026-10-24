# Roman props for the arena, fetched 2026-10-24 (GS23 list, no token needed)

Four glTF props from Poly Pizza, the community mirror of Google Poly and independent
authors. Every page was opened live on 2026-10-23/24 and states "Licence: CC-BY 3.0"
with the creator named; the CC-BY attribution text is part of the row and must stay
with the file wherever it ships.

Licence line the pages state: Creative Commons Attribution 3.0
(https://creativecommons.org/licenses/by/3.0/).

Retrieved with `node tools/fetch-content.mjs content/fetch-roman.list` into
`content/_incoming/url/<id>/`; the bytes here are byte-equal to the staged files
(md5 re-checked on disk after the copy). `Roman Arch` and `Coliseum` are the two the
arena shell wants: one arch module to repeat around the podium wall, one 24k-tri
massing for the far stands silhouette. The two soldier models are scale references
for the Rocketbox legionary, not crowd bodies (that lane is RL54/QA10).

| file | source URL | licence | author | attribution text | bytes | md5 | tris | potato |
|---|---|---|---|---|---|---|---|---|
| `roman_arch.glb` | https://poly.pizza/m/agiRDMN0VR | CC-BY 3.0 | Icawfype | "Roman Arch" by Icawfype, CC-BY 3.0 (https://creativecommons.org/licenses/by/3.0/), via Poly Pizza (https://poly.pizza/m/agiRDMN0VR) | 239,244 | 55b1d20bb0aa61e7b67fab51e8f9102f | 5,198 | yes (already inside HERO_MAX_TRIS 40,000; under PROP_MAX_TRIS 5,000 would need one cut) |
| `roman_coliseum.glb` | https://poly.pizza/m/4raJTBfhAfc | CC-BY 3.0 | Poly by Google | "Coliseum" by Poly by Google, CC-BY 3.0 (https://creativecommons.org/licenses/by/3.0/), via Poly Pizza (https://poly.pizza/m/4raJTBfhAfc) | 1,350,000 | a5225ae8c54aabb85cfbe23cbe0899a0 | 24,153 | needs LOD (bake one 6k LOD1 for the far stands) |
| `roman_centurion.glb` | https://poly.pizza/m/g2ckFsGszB | CC-BY 3.0 | blaeksprut | "Roman Centurion" by blaeksprut, CC-BY 3.0 (https://creativecommons.org/licenses/by/3.0/), via Poly Pizza (https://poly.pizza/m/g2ckFsGszB) | 212,456 | 0477ac0e23c5d718987d8e191d2de007 | 2,346 | yes |
| `roman_legionnaire.glb` | https://poly.pizza/m/czyveAaJ0hd | CC-BY 3.0 | Warren Blyth | "roman legionnaire grunt" by Warren Blyth, CC-BY 3.0 (https://creativecommons.org/licenses/by/3.0/), via Poly Pizza (https://poly.pizza/m/czyveAaJ0hd) | 85,108 | ec744972b64ececeac909da6ce7d457a | 1,064 | yes |

## Not cooked yet

No caller loads these: they sit beside `content/props/ai/` as source art. Cooking is
the normal node path (`crates/assets` mesh cook + `content/arena/kit.e2bundle`
placement); the consumer row is RA14 (arena content through the presenter) for the
arch and the coliseum massing, and RL54/QA10 for crowd bodies (these two soldier
meshes are static, so they are scale references only). Until a caller loads them they
are OPEN, not done.

Textures: the pages list no texture set for these four, so each ships with its own
vertex colours / flat materials. A 2K PBR pass over the arch is a later row, not this
one.
