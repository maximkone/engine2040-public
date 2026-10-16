STEAL SCOUT: MATERIALS AND CHARACTER LOOK (gladiators up close)    scout date 2026-10-12, read-only run
Plans: docs/plan/render-look.md (owns every row below; cap UV also touches content/bodies). The earlier scout render-steals.md rows 2 and 12 (aniso/Charlie/coat
terms, separable SSS) are not repeated: row 2 is already ported in crates/material, row 12 is only used here as a donor of constants.
Shas are branch HEADs fetched 2026-10-12. "ms" figures are targets for the RTX 3050 at 1080p, not measurements. Every steal is PRESENTATION (never feeds the tick).

=== 1. WE HAVE ===
1. A full material compiler that the renderer never calls. crates/material/src/record.rs:171-198 (Look: coat, sheen, anisotropy, transmission, subsurface, emission,
   4 texture slots) and compile.rs:405-592 (Filament-model WGSL: GGX + correlated Smith, Burley, coat, Charlie sheen, aniso GGX, wrapped SSS, thin transmission, ACES).
   crates/render has no engine-material dependency (Grep: 0 hits) and no material_pipeline.rs; plan row NS31c2 (docs/SPRINT-01-rome-first.md:191-203) was never built.
2. Energy is solid inside the compiler: baked DFG + sheen tables (dfg.rs:17-96), white-furnace test, bar 1.02, aniso 1.03 (furnace.rs:6-10).
3. The live fragment is much poorer: forward.wgsl:397-420 = factor x ONE base texture, no normal/ORM/emissive, occlusion hard-coded 1.0 (:415), sun shadow is a 15% cascade
   dim (:409). DrawItem = mesh + 4-float PbrMaterial + 1 TextureKey (present.rs:375-387, v1.rs:528-537). Seven BRDF functions exist twice (forward.wgsl:264-327 and
   compile.rs:623-669). Multi-scatter is on the sun term (:322-327) but NOT on the IBL term (:175).
4. IBL is a studio HDRI bake (ibl.rs:1), so polished armor reflects a studio, not the Colosseum. No geometric specular AA anywhere (Grep for dpdx/fwidth: only the normal-map
   helper, compile.rs:705).
5. Layers wet/dirt/blood/damage/burn (record.rs:223-235) are driven by 5 u8 coverage channels PER OBJECT, replay-safe from the event tape (channels.rs:26-127). An event
   carries no place (channels.rs:41-52). Fighters draw up to 16 parts (fighter_look.rs:62) and the hit tint recolours the whole body (:1034-1041).
6. Decals: DecalPool = 128 slots, 64x64 RGBA8 tiles, centre in VIEW space, CPU depth reject, 600-tick (10 s) fade, blood roughness 0.15 -> 0.65, oldest-first eviction
   (decals.rs:22-45, 626-646, 416, 831). No non-test caller (Grep). The window "submit" is a bind probe that copies words (decals.rs:968-989), and upload_instances always writes
   BLOOD_COLOR (:918). combat has its own integer GoreEvent (gore.rs:40-78, DecalStamp at the victim's feet) and render a float one (decals.rs:111): two types, no bridge.
7. Cuts: sever_cook.rs:40-52 cooks a stump cap as i32 mm positions + triangles only. No UV, normal or material, and no render/game reader (Grep).
8. Skin, hair, eyes: none in render (frontier.md:143; row NS10 waits on VS5b, frontier.md:177). Backlog lines LI4, LI6, LI27, LI30 (landscape-ideas.md:16,18,39,42) name no read
   donor file. GS19 wound maps (SPRINT-18-steals.md:215-223) = a paint budget queue only: no GPU pass, no UV2. Sand glint = one fixed lobe (material_layer.rs:295-307).
9. AA is TAA on single-sample targets (post/taa_resolve.wgsl; present/gpu_present/submit.rs:129 sample_count 1), so alpha-to-coverage is out; hashed alpha pairs with TAA.

=== 2. STEALS (ranked by value for the Rome duel look per hour of work) ===
1. Forward adopts the compiled lobe set (own-code wiring, unlocks 2-14) | google/filament@764fe6c:shaders/src/surface_shading_lit.fs (read 2026-10-12; shape only: one lit entry
   built from feature defines) + our compile.rs | Apache-2.0 | PRESENTATION | lands in crates/render/src/material_pipeline.rs (new), shaders/forward.wgsl, present.rs | M
   Idea: stop hand-writing the fragment. forward.wgsl keeps vertex, sky, fog, shadow and the IBL fetch; the lit part is the generated module keyed by Structure (compile.rs:84-190).
   Moves: lobes a draw can reach 1 -> 6, texture slots per draw 1 -> 4, hand-written BRDF fns in forward.wgsl 10 -> 0, so armor, leather and cloth can use ORM, normal, sheen, coat.
   Proof: cargo test -p engine-render --lib --features window material_pipeline (21/21 shipped records, <=12 pipelines, 0 validation errors). Kill: if 2 fighters + arena p95 >16.6 ms
   on the 3050, drop coat + transmission on fighters.
2. IBL polish: compensated IBL specular, multi-bounce AO, specular AO, geometric specular AA | google/filament@764fe6c:shaders/src/surface_ambient_occlusion.fs and surface_shading_lit.fs
   (read 2026-10-12) ; o3de/o3de@12ff514:Gems/Atom/Feature/Common/Assets/ShaderLib/Atom/Features/PBR/SpecularAA.azsli (read 2026-10-12) | Apache-2.0 ; Apache-2.0 OR MIT | PRESENTATION |
   lands in compile.rs chunks + the forward IBL call (forward.wgsl:147-176) | S
   Idea: IBL spec x (1 + f0 (1/E - 1)) like the sun term; gtaoMultiBounce (cubic in AO, albedo-tinted) so cavities keep colour; Lagarde specular AO; normal-derivative variance
   kernel = min(2 x 0.25 x (|dn/dx|^2 + |dn/dy|^2), 0.18) added to alpha^2 (O3DE numbers).
   Moves: IBL furnace of a rough metal 0.31 (dfg.rs:89) -> >=0.9; occlusion const 1.0 (forward.wgsl:415) -> GTAO-fed; armor highlight shimmer over a 60-frame orbit down (aa_scale
   SMAA_SHIMMER_MAX_VAR measure). Cost <=0.05 ms. Proof: new test ibl_furnace + orbit capture pair. Kill: Q1 rubric score drops on 3 of 10 hero shots (cavities too dark).
3. Fighter coverage by part, then by vertex (sweat, blood, dirt where they landed) | turanszkij/WickedEngine@cee348b:WickedEngine/shaders/objectHF.hlsli (read 2026-10-12: per-vertex wetmap
   read into the surface) | MIT | PRESENTATION | lands in channels.rs, fighter_look.rs push_draws, present.rs DrawItem, compile.rs channel_expr | S then M
   Idea: take the mask transport, not the response (Wicked darkens and raises roughness; our Wet layer already glosses, compile.rs:465-469). Step A: the event carries a body-part id and
   DrawItem carries its own 5-channel block (a draw is a part, so it is free). Step B: u8x4 per-vertex mask, 25k verts = 100 KB per hero, written from the same tape (replay-safe).
   Moves: coverage identical on all parts today (channels.rs:26) -> after a torso hit torso blood >=0.5, legs <=0.1. Proof: cargo test -p engine-game --lib fighter_coverage (+ replay x2).
   Kill: part split does not match anatomy -> do step B only if step A fails the capture rubric.
4. Sand ground state: blood, footprints, drag marks that stay | naelstrof/SkinnedMeshDecals@66f7d02 (branch upm):Shaders/DecalProjectorAlphaBlend.shader and DilationShaderAlpha.shader
   (read 2026-10-12) | MIT | PRESENTATION | lands in crates/render/src/ground_state.rs (new; reuses DecalPool evict/fade math), the forward floor sample | M
   Idea: one persistent planar RGBA8 map over the pit (rome-spec.md:13, 20 x 20 m). Hits blit stamps (alpha blend, per-frame stamp cap); a slow subtractive blit dries/fades; a byte cap
   evicts. R blood, G coarse age (wet -> dry colour), B scuff depth (normal), A wet. Planar world XZ, so no UV2 and no dilation.
   Moves: blood on sand lasts 10 s and 128 decals (decals.rs:37,22) -> the whole bout; 4 MB (1024^2 at 2 cm, potato) or 16 MB (2048^2 at 1 cm); <=16 stamps per frame; floor cost 1 tap,
   <=0.1 ms. Proof: cargo test -p engine-render --lib ground_state (>=95% of blood mass kept at tick 3600 after 40 stamps; bytes x2) + captures at 5 s and 60 s.
   Risk: a web search surfaced trail-map patents (US 11,534,688 and US 12,263,404; search snippet only, unverified). Do NOT add a blur "ooze" step; stamps + fade only.
5. Skin: thin transmittance + terminator LUT (one tap, potato-safe) | google/filament@764fe6c:shaders/src/surface_shading_model_subsurface.fs ; o3de/o3de@12ff514:Gems/Atom/Feature/Common/Assets/
   ShaderLib/Atom/Features/PBR/BackLighting.azsli ; godotengine/godot@232e6b1:servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl ; iryoku/separable-sss@b217468:SeparableSSS.h
   (all read 2026-10-12) | Apache-2.0 ; Apache-2.0 OR MIT ; MIT ; BSD-style (credit line "Uses Separable SSS ..." needed in binary credits) | PRESENTATION |
   lands in crates/material/src/skin.rs (new) + a compile.rs chunk | M
   Idea: Filament thickness-scaled wrap with a spherical-Gaussian power; O3DE thin-object transmittance on the fitted Gaussian-sum skin profile (variances 0.0064 to 7.41, RGB weights, in
   SeparableSSS.h); LUT indexed by (N.L, curvature from fwidth(normal)/fwidth(pos)), built in Rust from the same profile. The LUT generator itself is IDEA-ONLY (Penner 2011; no code donor
   found); only the profile constants and the transmittance are ports.
   Moves: skin terminator today = one wrap + one tint (compile.rs:532-545) -> red wider than blue; LUT 64x64x4 B = 16 KB; <=0.05 ms. Proof: cargo test -p engine-material --lib skin_lut
   (bytes x2; R/B width >=1.5x at curvature 0.5, a proposal) + forearm capture crop. Kill: crop does not beat plain wrap in the judge pair.
6. Hex-tiled sand + biplanar kit walls (no visible repeat, constant texel density) | mmikk/hextile-demo@43c3ed7 (branch main):hextile-demo/hextiling.h (read 2026-10-12) ;
   BabylonJS/Babylon.js@a6b8e22:packages/dev/core/src/Materials/Node/Blocks/biPlanarBlock.pure.ts (read 2026-10-12; has a WGSL branch) | MIT ; Apache-2.0 | PRESENTATION |
   lands in compile.rs CHUNK_TILING + the arena kit material in game | S
   Idea: hex: 3 taps, per-vertex hash offset + rotation, weights raised to the 7th power for a contrast-keeping blend (colour and normal-map variants). Biplanar: take the two largest
   normal axes, 2 taps, weights ((w - 0.5773)/(1 - 0.5773))^(k/8). KitActor carries size_m (render-look.md:232), so a non-uniform scale stretches UVs.
   Moves: sand 1 tap with a visible period -> 3 taps; luminance autocorrelation at one period lag 1.0 -> <=0.5 on a 4x4 patch; +2 taps on ~35% of pixels, <=0.15 ms.
   Proof: cargo test -p engine-render --lib hextile + frame-60 sand crop. Kill: sand crop MTF50 drops >10% (the RP8 measure).
7. Arena reflection probe with a box-projected sample (polished armor reflects the Colosseum) | bevyengine/bevy@c9c21c2:crates/bevy_pbr/src/light_probe/environment_map.wesl (read 2026-10-12)
   | MIT OR Apache-2.0 | PRESENTATION | lands in crates/render/src/ibl.rs (bake input) + the forward IBL fetch | M
   Idea: bake one 128^2 cube from the arena kit at pit centre (1.6 m up), prefilter with the existing path; sample with a ray-vs-box in probe space and lean N toward R by roughness
   (their radiance_sample_direction); blend with the sky env until the weights sum to 1.
   Moves: reflection source studio HDRI (ibl.rs:1) -> arena; ~1 MB (128*128*6*8 B + mips); +1 ray-box, <=0.05 ms. Proof: bake bytes x2; capture of a steel sphere in the pit shows the stands
   in the mirror. Kill the parallax step (keep the bake) if the ray-box bends near-wall reflections by >5 degrees.
8. Cap material for severed limbs (muscle ring, bone core, wet coat) | DavidArayan/ezy-slice@073efdc (branch master):EzySlice/Slicer.cs (read 2026-10-12, partly: cap goes to its own submesh
   with a region-UV remap, mirrored winding) | MIT | PRESENTATION | lands in crates/assets/src/sever_cook.rs + a cap draw in game | M. IDEA-ONLY for the flesh/bone shading itself (EzySlice has no
   flesh shader; it only proves the own-submesh + UV-region pattern).
   Idea: cook adds per cap vertex a planar-polar UV (radius from the ring centroid), a normal and a ring id; the cap draws with 2 shipped MaterialRecords (muscle: SSS + coat 1.33 wet; bone: matte
   pale) picked by radius. Moves: cap vertex data 3 ints -> pos + uv + normal + id; render readers 0 -> 1. Proof: cargo test -p engine-assets --lib sever_cook (cook x2 equal) + a cut-forearm
   capture. Risk: the cook digest moves; add the new data as a separate section. Golden d57b5c8ae3096614 is sim-side and untouched.
9. Forward decal list for walls and props, roughness override, world-space pool | godotengine/godot@232e6b1:servers/rendering/renderer_rd/shaders/forward_clustered/scene_forward_clustered.glsl
   (decal loop) ; bevyengine/bevy@c9c21c2:crates/bevy_pbr/src/decal/clustered.wesl ; o3de/o3de@12ff514:.../PBR/Decals.azsli (all read 2026-10-12) | MIT ; MIT OR Apache-2.0 ; Apache-2.0 OR MIT |
   PRESENTATION | lands in decals.rs + compile.rs CHUNK_DECAL | M
   Idea: <=16 decals in a uniform array; each tests the fragment in its local box (+/-0.5), fades by upper/lower distance and by normal angle (O3DE smoothstep from 0.55), OVER-blends colour,
   mixes roughness/metal by decal alpha, whiteout-blends the normal. Fix the pool: store WORLD centres (decals.rs:626 is view-space, so an orbiting camera detaches blood).
   Moves: decals drawn per frame 0 -> N, pool lifetime logic kept (evict oldest, free tile), <=0.5 ms (decals.rs:45). Proof: cargo test -p engine-render --lib decal_forward + two-camera
   capture pair. Kill: >0.5 ms with 16 decals -> cap at 8.
10. Hair and beard cards: Marschner-lite + hashed alpha under TAA | Scthe/frostbitten-hair-webgpu@4478dd1 (branch master):src/passes/hairShadingPass/marschner.wgsl.ts (read 2026-10-12, ~170 lines
   WGSL) ; godotengine/godot@232e6b1:servers/rendering/renderer_rd/shaders/scene_forward_aa_inc.glsl (read 2026-10-12) | MIT ; MIT | PRESENTATION | lands in compile.rs CHUNK_HAIR, one head
   draw per fighter | M (needs a content row for card meshes)
   Idea: tangent from card UV; R, TT and TRT lobes with Gaussian longitudinal terms (tilts -2s, s, 4s; widths r^2, r^2/2, 2 r^2), tinted absorption for TT/TRT; opacity by hashed alpha (hash on the
   bind-pose position, log2-snapped scale, derivative-scaled) so TAA resolves it with no sorting. Moves: hair draws 0 -> 1 per head; <=0.3 ms for 8 heads; card-edge ghost <=2 px.
   Proof: cargo test -p engine-render --lib hair_lobes (energy <=1) + head crop. Kill: no hair mesh in content/fighters yet; cards >0.3 ms or aliasing -> textured scalp.
11. Wound maps, GPU stage (deepens GS19) | naelstrof/SkinnedMeshDecals@66f7d02 (branch upm):Shaders/DecalProjectorAlphaBlend.shader and DilationShaderAlpha.shader (read 2026-10-12) | MIT |
   PRESENTATION | lands in crates/render/src/wound_map.rs (GS19), fighter_look.rs | M
   Idea: draw the fighter mesh in UV2 clip space, project the splat from world, smoothstep fade on back faces, no depth test; then an 8-neighbour dilation (9 taps) hides seams. The donor
   divides by the alpha sum with no zero guard: add one. Needs a unique UV2 per fighter (fighter_look.rs:416-421 uploads uv0 only; pack with xatlas offline).
   Moves: GS19 queue only -> cuts that follow the body; 1024^2 x 4 B = 4 MB per hero, GS19 caps the rest. Proof: capture after 3 sword hits shows 3 distinct marks; bytes x2.
   Kill: hero map >8 MB or the UV2 pack fails on a fighter.
12. Rim and readability: per-fighter rim term + a fighter-only key light | godotengine/godot@232e6b1:servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl (rim path, read 2026-10-12)
   | MIT | PRESENTATION | lands in the compile.rs lit chunk + game lights | S
   Idea: rim grows with 1 - N.V, scaled by roughness, tinted between white and albedo (rim_tint), on fighter draws only. Moves: rim term 0 -> 1; edge-band contrast (mean luma of the fighter
   outline band vs the ring behind it) unmeasured today -> >=0.15 in V3 fight shots (a proposal). Proof: V3 judge pair rim on/off (EG8 rule: the picture must change) + the band metric.
   Kill: band metric gains <0.03 or halo shows on bright sand.
13. Material LOD tier in the structure key | google/filament@764fe6c:shaders/src/surface_brdf.fs and surface_material.fs (read 2026-10-12; two FILAMENT_QUALITY branches each) | Apache-2.0 |
   PRESENTATION | lands in compile.rs Structure::key (:174) + the render tier table (GS10) | S
   Idea: below High use the fast Smith visibility (Hammon), at Low use f90 = 1 Fresnel and a cheaper clear-coat F0; a potato rule drops sheen, aniso, coat, transmission and SSS to wrap + base.
    Moves: <=12 pipelines per tier; fragment ALU on potato -25% (proposal); frame ms on the Intel UHD through GS16 potato-check (tools/potato-check.mjs on disk).
   Proof: cargo test -p engine-material --lib tier_key (same record, 3 tiers, 3 keys, potato text shorter). Kill: a cut that fails the rubric on 2 of 10 shots.
14. Screen-space SSS, High tier only (diffuse-only target + Burley blur) | BabylonJS/Babylon.js@a6b8e22:packages/dev/core/src/ShadersWGSL/subSurfaceScattering.fragment.fx (read 2026-10-12) ;
   godotengine/godot@232e6b1:servers/rendering/renderer_rd/shaders/effects/subsurface_scattering.glsl (read 2026-10-12, 189 lines) | Apache-2.0 ; MIT | PRESENTATION |
   lands in the render post chain + a second colour target on the skinned pipeline | L
   Idea: skin draws also write diffuse-only irradiance into a 2nd attachment (alpha = profile id); one pass blurs it with a normalized Burley profile, taps scaled by projected size and capped
   at 40; the result is added as albedo x blurred irradiance on the colour that still holds specular. Godot's variant: 13/9/6 taps, skin flag in the alpha sign, width ~ 1/depth.
   Moves: build only if row 5 fails the judge pair; est. 0.3-0.5 ms full frame, <=0.25 ms with a skin-only dispatch. Proof: PF2 GPU timing + judge pair.
   Kill: >0.4 ms on the 3050 (laptop bandwidth ~168 GB/s, spec-sheet figure, not checked here).
15. Cook-time cavity and edge-wear vertex mask for armor (wear where metal is hit) | IDEA-ONLY (Substance and Blender docs only; no code donor found) | n/a | PRESENTATION |
   lands in the crates/assets mesh cook -> the per-vertex mask of row 3 | M
   Idea: curvature from face-normal angles at cook; convex = chips, concave = dirt, times a noise grunge in the shader. Moves: armor wear 0 -> one mask channel. Proof: cook bytes x2.
   Kill: do not schedule until row 3 step B lands.

=== 3. BOARD ROWS (prefix XX, the lead numbers them; plan owner is docs/plan/render-look.md section 7 unless noted) ===
- [ ] XX1 render: forward uses the compiled material — the window fragment is one texture times four floats (forward.wgsl:397-420, present.rs:375-387) while compile.rs:405-592 already
  writes six lobes and four texture slots; ten BRDF fns live in forward.wgsl:264-327, seven of them copies of compile.rs:623-669. Plan: render-look.md (supersedes NS31c2/c3 text).
  Diff: good. Tier A. Size M. Owns crates/render/src/material_pipeline.rs (new), crates/render/src/shaders/forward.wgsl, crates/render/src/present.rs, crates/render/Cargo.toml,
  crates/material/src/compile.rs. Accept: cargo test -p engine-render --lib --features window material_pipeline: 21 of 21 shipped records build in <=12 pipelines with 0 validation errors;
  DrawItem texture keys 1 -> 4; BRDF fns in forward.wgsl 10 -> 0. F2P: no material_pipeline test exists; a draw cannot reach sheen, coat or a normal map. P2P: golden unchanged. needs: none.
- [ ] XX2 render: IBL polish — IBL specular has no energy compensation (forward.wgsl:175), occlusion is the constant 1.0 (:415), the table shows 0.31 of the light surviving at the rough end
  (dfg.rs:89), and no specular AA exists. Plan: render-look.md. Diff: regular. Tier A. Size S. Owns crates/material/src/compile.rs, crates/render/src/shaders/forward.wgsl.
  Accept: cargo test -p engine-render --lib ibl_furnace: furnace ratio of a perceptual-roughness-1.0 metal >=0.9 (today 0.31 at the worst view); orbit shimmer variance below
  SMAA_SHIMMER_MAX_VAR. F2P: ibl_furnace fails on the current IBL term. P2P: golden unchanged. needs: XX1.
- [ ] XX3 game: blood, sweat and dirt where they landed — coverage is per object and events carry no place (channels.rs:26-52); the hit tint recolours the whole body
  (fighter_look.rs:1034-1041). Plan: render-look.md (shader side) plus the bodies plan for the event field. Diff: good. Tier A. Size M. Owns crates/material/src/channels.rs,
  crates/game/src/fighter_look.rs, crates/render/src/present.rs. Accept: cargo test -p engine-game --lib fighter_coverage: after 3 torso hits torso blood u8 >=128 and legs <=25; replay
  from the tape twice gives equal bytes; step B mask is 4 B per vertex. F2P: every part shows the same coverage today. P2P: golden unchanged. needs: XX1.
- [ ] XX4 render: sand remembers the fight — blood on sand lasts 600 ticks in a 128-slot view-space pool with no caller (decals.rs:22,37,626). New planar ground state with stamps, fade and a
  byte cap. Plan: render-look.md (RP3 decals narrowed to walls and props). Diff: good. Tier A. Size M. Owns crates/render/src/ground_state.rs (new), crates/render/src/shaders/forward.wgsl,
  crates/render/src/decals.rs. Accept: cargo test -p engine-render --lib ground_state: 40 stamps over 3600 ticks keep >=95% of blood mass at tick 3600; map 1024x1024x4 = 4,194,304 B;
  stamps per frame <=16; bytes x2 equal. F2P: no ground_state module; the current pool holds 0 decals after 10 s. P2P: golden unchanged. needs: XX1.
- [ ] XX5 material: skin that is not plastic — the skin lobe is one wrap and one tint with ad-hoc constants (compile.rs:532-545) and the live shader has none. Thin transmittance + terminator LUT
  from the Gaussian-sum profile. Plan: render-look.md (closes the skin half of NS10). Diff: good. Tier B. Size M. Owns crates/material/src/skin.rs (new), crates/material/src/compile.rs.
  Accept: cargo test -p engine-material --lib skin_lut: LUT 64x64x4 B = 16,384 B, bytes x2 equal; red terminator width >=1.5x blue at curvature 0.5; furnace bar 1.02 holds.
  F2P: no skin.rs; red and blue terminators are equal today. P2P: golden unchanged. needs: XX1.
- [ ] XX6 render: sand and walls without repeat — one sand tap, and kit meshes carry size_m (render-look.md:232) that can stretch UVs. Hex-tiling for the floor, biplanar for kit walls.
  Plan: render-look.md with content.md CT26 supplying the sand maps. Diff: regular. Tier B. Size S. Owns crates/material/src/compile.rs, crates/render/src/shaders/forward.wgsl.
  Accept: cargo test -p engine-render --lib hextile: luminance autocorrelation at one period lag 1.0 -> <=0.5 on a 4x4 patch; sand taps 1 -> 3, wall taps 2. F2P: autocorrelation is 1.0 on a pure
  repeat today. P2P: golden unchanged. needs: XX1.

=== 4. PLAN TEXT (render-look.md section 3 "Target design" items 3a-3e; section 4 "Hard problems" items 4a-4c) ===
3a. One shader text home: compile.rs composes the lit fragment from named chunks keyed by Structure; forward.wgsl keeps vertex, sky, fog, shadow and the IBL fetch. A BRDF is never written twice.
3b. Surface state lives at three sizes: per draw (5 coverage channels from the event tape), per vertex (u8x4 mask, fighters only, written from the same tape), per world texel (planar ground
    state over the 20 x 20 m pit: 1024^2 at 2 cm potato, 2048^2 at 1 cm comfort). World-space decals (<=16) are for walls and props only; wounds on bodies are UV2 wound maps (GS19). All replay-safe.
3c. Skin: thin transmittance + terminator LUT on every tier; the diffuse-only Burley blur is a High-tier option that must first beat the LUT in the judge pair.
3d. Metal: compensated IBL, multi-bounce AO, specular AO, geometric specular AA, and an arena-baked probe with a box-projected sample; the studio HDRI stays only as the fallback.
3e. Hair: cards, Marschner-lite, hashed alpha resolved by TAA, no strands. Floor and walls: hex-tiled sand, biplanar kit. Cuts: cook-time cap UV, normal and two material ids.
4a. Blood that stays: a 10-second view-space pool cannot show a bout's history (decals.rs:37,626); the ground state keeps it with a coarse age channel and a byte cap, stamps only (no blur).
4b. Same look on three tiers: the tier enters the Structure key (Filament quality branches), so potato is a shorter shader, not another renderer.
4c. Cuts need a face: sever_cook caps are positions only (sever_cook.rs:48-51); give them UV, normal and ring id at cook, with the digest change kept in a new section.

=== 5. NOT WORTH IT (checked, rejected for this game) ===
- Strand hair (Scthe software rasterizer, TressFX-class) and the full Chiang16 BCSDF (NVIDIAGameWorks/Falcor@759aad0:Source/Falcor/Rendering/Materials/HairChiang16.slang, BSD-3, read 2026-10-12):
  tile/OIT passes and a 10-term Bessel series per lobe; fails the potato tier and heads are small in a duel. Cards + Marschner-lite are enough.
- RT or path-traced skin and hair (NVIDIA-RTX/RTXCR: Chiang hair on linear-swept spheres, Burley + single-scatter skin; license file not read, only a path-tracer sample): RT tier only.
- Texture-space diffusion and a full-frame separable blur as the DEFAULT: 2 passes x 8 fighters of memory and bandwidth on a laptop 3050; kept only as row 14, High tier.
- Glint BRDF for sand and scratched steel (Deliot and Belcour 2023): no public code found; a one-pixel flake shimmers under TAA. Keep the fixed lobe (material_layer.rs:295-307) + hex-tiled normals.
- O3DE 3-layer per-fragment blend (StandardMultilayerPBR; its include files returned 404, so only the layer-count header was read) and wrinkle maps (Skin_WrinkleMaps.lua): triples the taps;
  fighter parts are separate draws and there is no face rig.
- G-buffer / DBuffer decals and parallax occlusion on sand (Bevy has parallax_mapping.wesl): we are forward on single-sample targets; POM on sub-pixel grain sparkles under TAA.

DONE 15 steals, 27 donor files read.
