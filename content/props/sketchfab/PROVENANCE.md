# Roman arena props from Sketchfab: provenance

Twenty-three glTF props fetched from the Sketchfab download API in two rounds on 2026-10-24 with the
seat's own `SKETCHFAB_TOKEN` (env var, never stored in the tree). Every model page was
read through `GET /v3/models/{uid}` the same day: it names the author and states the
licence below, and `isDownloadable` is true for all of them. The download endpoint
(`GET /v3/models/{uid}/download`) served one self-contained GLB per model (embedded
1K textures, no external files); the untouched originals stay in
`target/sketchfab/<uid>/` and are not committed.

Fit to the prop budget: each GLB was decimated headless in Blender 5.2 (one process at
a time) to the tri count in the row, below `PROP_MAX_TRIS` (5000, `crates/assets/src/mesh_cook.rs`):
weapons 1,200 to 1,500, armour and small props 1,000 to 2,500, architecture and the bust
4,000 to 5,000. Every embedded texture was already 1024 px or less, so none needed a
resize and none is above the potato 1024 px cap. Sizes and SHA-256 below are the landed
bytes on disk.

The cook was a throwaway headless Blender script (join meshes, Collapse decimate to the
target ratio, apply, export GLB with `export_apply=True`); the staged output sat in
`target/sketchfab/out/` and was copied here byte for byte.

CC-BY attribution: same author and licence per row. Attribution text must stay with the
file wherever it ships; see `CREDITS.md` beside this table.

No caller loads these yet: they are source art beside `content/props/roman/` and
`content/props/polyhaven/`. The kit loader is the SHOW4 row; until it loads one, that
prop is OPEN, not done.

| Sketchfab id | file | author | licence | source URL | bytes | tris | sha256 | kind |
|---|---|---|---|---|---|---|---|---|
| `b123c4388a6f4307aff353c165c94f92` | `gladius_direraven.glb` | DireRaven | CC-BY | https://sketchfab.com/3d-models/roman-gladius-b123c4388a6f4307aff353c165c94f92 | 2,677,188 | 1,500 | `6ba31ee82d45fab5597648f536b8b17b3a3ffd16f954f975d71b24a472f03a39` | gladius |
| `7261d334924f4a9b835ef4b15c1e16d0` | `gladius_samize.glb` | Samize | CC-BY | https://sketchfab.com/3d-models/ancient-roman-gladius-sword-7261d334924f4a9b835ef4b15c1e16d0 | 1,888,488 | 1,500 | `b7b7c14dd5a7d15902ba05f29297eec1898c1fdc8404536c1e71c1b09ed9b010` | gladius |
| `eeae43c837484f1b8f98acc60a7b2597` | `gladius_aren_boada.glb` | Aren Boada | CC-BY | https://sketchfab.com/3d-models/gladius-roman-empire-sword-eeae43c837484f1b8f98acc60a7b2597 | 4,028,400 | 1,499 | `28462b4ffb76285db5caf615ded09d29513269a6cbbd207530ead15fcd134f6d` | gladius |
| `4fbd48b57c3a440f87072226686bcf60` | `pugio_davicolt.glb` | Davicolt | CC-BY | https://sketchfab.com/3d-models/roman-dagger-pugio-70-130-ad-4fbd48b57c3a440f87072226686bcf60 | 2,700,348 | 1,500 | `2c892a4b50328d3b474612d08f3d5b7b1b152303e7668dedb582570557c79a68` | pugio |
| `b700cb582841482abd7fb60b7df71bb3` | `pilum_paulohare.glb` | Paul.OHare | CC-BY | https://sketchfab.com/3d-models/pilum-b700cb582841482abd7fb60b7df71bb3 | 2,168,644 | 662 | `bcdb6b5014d555259d1e8ea6d3a9417d3b787cdfc88e8c88e953a46dadda90f3` | pilum |
| `10038602333f4901bd28cde290a2e397` | `scutum_paulohare.glb` | Paul.OHare | CC-BY | https://sketchfab.com/3d-models/scutum-roman-shield-10038602333f4901bd28cde290a2e397 | 2,803,944 | 990 | `8c2d8cde91d6e6ca79de34086d108d8cfde807b858d5ed453b18650e9596be83` | scutum |
| `d67564c8980b4e84acde74144da941ea` | `scutum_dennisvanmalderen.glb` | DennisVanMalderen | CC-BY | https://sketchfab.com/3d-models/roman-scutum-shield-d67564c8980b4e84acde74144da941ea | 4,365,408 | 1,400 | `a48edadf3f8d7f91860c76aaa838dd60e7a07b70137519e8fefcd27d9ec3216d` | scutum |
| `77c9be620d894e0abff935f8375cf548` | `lorica_segmentata.glb` | Mitro12345 | CC-BY | https://sketchfab.com/3d-models/lorica-segmentata-77c9be620d894e0abff935f8375cf548 | 108,604 | 1,499 | `b6b1dadd501a9c5237e1b7f0fac2c96f9fcb6442f68bbc58d6e7e8c6a28808a8` | lorica segmentata |
| `7d1205de2a8048938cd1cc30b79b5f47` | `galea_killband.glb` | Killband | CC-BY | https://sketchfab.com/3d-models/roman-galea-helmet-7d1205de2a8048938cd1cc30b79b5f47 | 2,719,780 | 2,000 | `1ece3922ae4d3c524c75243e1762e43626355f288e0fbc73951d57c576c8e7dc` | galea helmet |
| `faed6c9c08b049839740328c72136de5` | `amphora_shadic02.glb` | Brian | CC-BY | https://sketchfab.com/3d-models/roman-amphora-faed6c9c08b049839740328c72136de5 | 330,460 | 2,000 | `d3cba6e352b925a008110be34ff046e1c507a7caa61befdaec13c1aecea73955` | amphora |
| `fa194c993d99411d8b211b0df5a4bf8c` | `brazier_notmybest3d.glb` | ARTcheo3D | CC-BY | https://sketchfab.com/3d-models/ancient-roman-bronze-brazier-fa194c993d99411d8b211b0df5a4bf8c | 5,526,696 | 2,500 | `2a0c838f0cb7ddec844e9f9488428fd45fd6fa42e0f4e8359d2141297d1cd2ba` | brazier |
| `76cf35f293b345829aec96b499bc1313` | `corinthian_column_nickhavas.glb` | Nick Havas | CC-BY | https://sketchfab.com/3d-models/corinthian-column-76cf35f293b345829aec96b499bc1313 | 7,791,940 | 3,999 | `d39fa4cf69c9a4e1f410be1a77df37cf1c03840c53986976ba6bd10f81e4fcd8` | column |
| `194a677589814658be9e2c7c296e380f` | `hadrian_bust_avi.glb` | avi | CC-BY | https://sketchfab.com/3d-models/marble-bust-of-emperor-hadrian-194a677589814658be9e2c7c296e380f | 2,110,844 | 4,000 | `feb0962234a655d29d32d6223a73597a8853f220c95a501c2452126c3d8f9aa5` | marble bust |
| `f9b800d747034c2d9908696c30b2338e` | `colosseum_gits3d.glb` | giga | CC-BY | https://sketchfab.com/3d-models/colosseum-f9b800d747034c2d9908696c30b2338e | 724,096 | 4,500 | `2309ef973bf54f0e59df35e7a00f859363fe8de68b6585f1410566b063560f62` | colosseum massing |
| `d9a8a96e16534bdab76be23c1f2c45fc` | `colosseum_facade_jeremy.glb` | jeremy@avrod.org | CC-BY | https://sketchfab.com/3d-models/colosseum-facade-rome-italy-d9a8a96e16534bdab76be23c1f2c45fc | 2,646,188 | 4,500 | `80c7d18064675da6b329866a288c46060da6ff0698b72f7579d331bbac5b84cb` | colosseum facade |
| `6459307c93e54394bed44b2583701dc6` | `colosseum_cloud9.glb` | cloud9_studio | CC-BY | https://sketchfab.com/3d-models/ancient-roman-colosseum-6459307c93e54394bed44b2583701dc6 | 1,593,876 | 5,000 | `3b1bddf195d2093e7f73c21da96436a9e25c237e905b3870a9c183cad73acb24` | colosseum massing |
| `39e39dba81354367af36a4b975dd06aa` | `colosseum_arch_ring_lanagre.glb` | LanaGre | CC-BY | https://sketchfab.com/3d-models/roman-colosseum-arch-ring-marble-3d-model-39e39dba81354367af36a4b975dd06aa | 2,032,312 | 4,500 | `c37c3451d4a7944eacab03bd10ad2d63dcc831428bc1dc08cd4f4246b745de2e` | arch ring module |
| `e0d44cc5da8e4dde8e06e6266102f4f3` | `parma_shield.glb` | alessandro_volpe | CC-BY | https://sketchfab.com/3d-models/simple-parma-wood-shield-e0d44cc5da8e4dde8e06e6266102f4f3 | 4,091,024 | 2,999 | `6a66fa84a04129fe2d361707447c2f36b02e39e8427fe50a4f20f69fe83fd098` | parma (round shield) |
| `6526c1512fcc4b7ba50c1740fedf83cc` | `trident_rusty.glb` | julius.j.bib | CC-BY | https://sketchfab.com/3d-models/rusty-trident-6526c1512fcc4b7ba50c1740fedf83cc | 2,491,232 | 1,499 | `f65a89e5b6c6f7b7510b2281f124fc75c6687fdc9c0aa0ae0d40d3b1528fbeb2` | retiarius trident |
| `4f15f84201964d1aba2e3c437e56694c` | `podium_railing.glb` | ARTcheo3D | CC-BY | https://sketchfab.com/3d-models/roman-wooden-railing-kit-4f15f84201964d1aba2e3c437e56694c | 6,744,544 | 3,999 | `a7cf381b740eb183db2dfb5e17d40b2c8557cd40daf16f9f49090eb67ae2426b` | podium balustrade |
| `1d84a657fe0445618f2733aeb498646b` | `arena_gate_door.glb` | MWOgier | CC-BY | https://sketchfab.com/3d-models/ancient-door-1d84a657fe0445618f2733aeb498646b | 2,022,924 | 5,000 | `435f4956d62d25a84dc3cc146dc7d4a006415772b905191ace7f317db4235455` | stone gate with wooden doors |
| `6e5ede2ebea147bd82ffeafaf96cf6a2` | `murmillo_helmet.glb` | Sylvain Delandre | CC-BY | https://sketchfab.com/3d-models/gladiator-helmet-6e5ede2ebea147bd82ffeafaf96cf6a2 | 4,281,256 | 2,000 | `4c8f56bb2e300715b4a324155fedaf35a13ad66be5da2909de3d4f4fa6db5d67` | murmillo-style visored helmet |
| `9dc85b3f08bf4126818ad7333808d81e` | `borghese_gladiator_statue.glb` | matousekfoto | CC-BY | https://sketchfab.com/3d-models/borghese-gladiator-9dc85b3f08bf4126818ad7333808d81e | 1,423,036 | 5,000 | `4dc5fd957af43954dd94edc4e78c595766945de8163a323284ec20c0f3d16eee` | marble statue, full figure |

Total on disk: 67,271,232 bytes (64.2 MiB) in 23 GLBs, all Git LFS.

## Skipped

- `411dfd9889044c0eba1cbd66520f9403` "Ancient Roman Arches" (Shanza_Khan, CC-BY): 1,020
  tris and no image at all, a flat blockout rather than a realistic surface.
- `3446255227b14a1f83e56b24d619f8e0` "Retiarius Gladiator (Low Poly)": the model name
  says low poly, the row asks for realistic, so the trident came from
  `rusty trident` instead; no separate net model was found.
- `db6713ea0b854e83ac52604b7fd381b9` "Roman Gladiator Helmet" (theacidrose, CC-BY):
  spiked dark-fantasy helmet, not a historical murmillo; the visored helm from
  `6e5ede2ebea147bd82ffeafaf96cf6a2` landed instead.
- `abdca0fd52834dbabccc22fdfe5e5899` "Sand Rake" (sameedalvi, CC-BY): a modern
  sand-trap rake with wheels and flat colours, not an arena tool.
- `fbe2aec8ddb84cccac723848a08530ec` "A simple basic package of helmet molds" (Kanpai):
  a mould package, not a finished helmet.
- `2f55a8747dc44ed9a06a486540ceb0d7` "Gladiator Helmet" (HarryPS): an unfinished white
  clay model.
- No realistic downloadable manica (arm guard), ocrea (greave), velarium panel or
  retiarius net at CC0/CC-BY inside the box; the queries are in the report.

