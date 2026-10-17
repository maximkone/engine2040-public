# Cheap realism that runs in a browser

Scout 06, 2026-10-17. Read: `docs/plan/web-and-realism.md` §1-2, `docs/plan/render-look.md` table, `docs/SPRINT-22-web-realism.md` VR1-VR3. "(snippet)" = search text only. "GUESS" = not read.

## Ideas

| Idea | Source + URL | Licence | Do | Row | Effort | GPU cost |
|---|---|---|---|---|---|---|
| GTAO by tier | XeGTAO https://github.com/GameTechDev/XeGTAO | MIT; archived 2024 | Keep half-res + bilateral upsample. Web tier: lower preset (2 slices, 4 samples). | LF3 | M | 1080p, author figures: RTX 2060 High 0.56 ms; GTX 1050 Medium 1.5 ms, High 2.2 ms; iGPU 2.39 ms. |
| Hue-keeping tonemap | Khronos PBR Neutral https://github.com/KhronosGroup/ToneMapping | Apache-2.0 | Port README formula (F90 0.04, start 0.76, desat 0.15). README: "no hue shifts". Replaces the six ACES copies. | RL34 | S | Few ALU ops (GUESS) |
| AgX, Tier C only | https://github.com/sobotka/AgX | No licence on page | A/B against PBR Neutral on the torch night shot. | RL34 Tier C | S | As Neutral (GUESS) |
| Sky and aerial perspective | Hillaire, EG CGF 2020 https://diglib.eg.org/handle/10.1111/cgf14050 (abstract, snippet) | Paper; reimplement | Small sky-view and aerial LUTs, no high-dimensional LUT. One bake feeds sky, IBL, fog. | LF1, RL8, RL7 | L | Plan budget: sky 0.4 ms, fog 0.2 ms |
| Contact shadows | drei https://drei.docs.pmnd.rs/staging/contact-shadows | MIT (GUESS) | Blurred occluder under fighters and props. Static props render once (`frames={1}`). | RL7 contact | S | Docs: "rather expensive" |
| Baked lightmaps, static kit | three.js ProgressiveLightMapGPU https://threejs.org/docs/pages/ProgressiveLightMap.html (snippet) | MIT (GUESS) | Bake AO and sun bounce for sand and walls into UV2, with validity mask. Fighters stay on RL2 SH. | RL5, VR2 | M-L | One fetch; storage is the risk |
| Box-projected reflection | Lagarde and Zanuttini, SIGGRAPH 2012 (snippet) | Method; reimplement | One baked cube per arena zone, box-projected in forward IBL. SSR misses fall back to it. | LF2, RL47 | S | One cube sample (GUESS) |
| Shadows: one sun map first | three.js CSM docs https://threejs.org/docs/pages/CSM.html (snippet) | MIT (snippet) | Keep the 2048 map. Cascades (RL27) only if sun range needs them. WebGPU uses CSMShadowNode. | EG11, RL27 | M | One depth pass per cascade (GUESS) |
| Parallax occlusion, close only | PlayCanvas docs https://developer.playcanvas.com/user-manual/graphics/physical-rendering/parallax-mapping/ (snippet) | Docs; engine MIT (GUESS) | Normal and ORM first. POM on stone near camera, 8 taps on web. Engine default cap 16. | VR2 | S | Heavier than normal maps (snippet) |
| CC0 texture sets, trim sheets | Poly Haven https://polyhaven.com/license; ambientCG https://ambientcg.com/ | CC0 (read) | Stone, sand, wood, iron sets. One trim sheet per wall and stand. | VR2 | M (art) | Fewer atlases and draws (GUESS) |
| Bloom | Jimenez, CoD AW slides https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/ (snippet) | Slides; reimplement | Pyramid bloom with threshold, half-res chain. | LF4 | S-M | Plan budget 0.5 ms (GUESS) |
| Blood decals | Godot decal docs https://docs.godotengine.org/en/latest/tutorials/3d/using_decals.html (snippet) | MIT (GUESS) | Project at render time, no new geometry. Cap count, fade with distance. | RP3 | M | Covered area first, then count (snippet) |

## Top 5 this week (my ranking, not measured)

1. CC0 texture sets and trim sheets on the kit (VR2 slice). Biggest gain over flat colour; no runtime cost.
2. Hue-keeping tonemap (RL34, PBR Neutral). Small change; keeps torch hue.
3. Contact shadows under fighters and props (RL7). Grounds the figures.
4. Box-projected cube reflection (LF2). Reads as real on metal and stone.
5. Half-res GTAO on the web tier (LF3). Already planned; needed for the weak-GPU floor.

## Traps

- AI textures: Meshy's pricing page says free-plan outputs are CC BY 4.0 (credit required); paid Pro and above are full commercial (fetched). A Cinevva promo guide says free is non-commercial. Read the terms before shipping.
- render-look §5 (2026-10-15) allows porting donor code but keeps GTAO and SMAA pattern-only. XeGTAO is HLSL and archived. Decide before copying.
- PBR Neutral's README targets well-exposed product scenes. A Fab listing says very dark scenes look darker (unverified). Test the dark arena.
- glTF forum replies say glTF has no lightmap slot (workaround: emissive slot). Check the spec first.
- PlayCanvas's runtime-lightmap blog (older, snippet) reports 60 MB+ HDR for one Sponza. Cap sizes and compress.
- teamchong/splat (MIT) sorts on a CPU worker, 5-30 ms for millions (author figure, no GPU named). Not for the web tier yet. Visionary (arXiv 2512.08478) has no licence and no numbers read.
- TAA needs motion vectors and history clears on cuts (luma.gl, snippet). Use SMAA or FXAA as the web default.
- The only web contact-shadow number is a forum anecdote (~30 fps, 7-year-old iGPU, snippet).
- No shipped WebGPU game with this look was found. Babylon and PlayCanvas WebGPU claims come from a vendor promo post.
- Hillaire PDF returned 403; sky claims rest on the abstract.
