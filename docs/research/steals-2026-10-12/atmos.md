STEAL SCOUT r2: ATMOSPHERE, VFX AND CROWDS (2026-10-12)
Method: donor files read through WebFetch (it summarizes the page, so shapes are reliable, exact lines are not). Shas are the HEAD seen on
2026-10-12. Numbers marked "proposal" are targets, not measurements; numbers marked "donor" are the donor's own post. All steals are
PRESENTATION unless said; none touches the tick, golden d57b5c8ae3096614 stays. Earlier scout (render-steals.md:43-47) already named
Bevy atmosphere, Bevy volumetric fog (raymarch, no froxels), Bevy forward decals, Hanabi (whole), bevy_trail; not repeated except where
I add a read file or a number.

1. WE HAVE
- Particles: CPU seeded pool, 3 fixed emitters dust/flame/sparks, cap 4096 (hard 8192), budget 1.0 ms, flipbook grids per kind
  (particles_gpu.rs:27-33,94-133,191-251). The "GPU submit" is a bind PROBE: PARTICLE_BIND_WGSL copies words (dst=src+exposure*0), submit.rs:71-90
  steps the pool and probes; no pixel is drawn (particles_gpu.rs:751-772). EG8 (SPRINT-18-gaps.md:168-177) lists sky and decals as PROBE; particles are
  the same shape. A second CPU sim particles.rs (Flipbook :78) feeds only v1_vfx.rs:30 twin bytes and tests (slim: two sims, one is dead weight).
- Decals: CPU DecalPool with atlas tiles, evict-oldest, blood fade 600 ticks, MAX_DECALS 256 (decals.rs:24,37-43,689-870,831) plus a bind probe
  (decals.rs:971-989). Nothing is projected on screen. Footprints, sand marks: none.
- Fog: live fog = closed-form height fog inside forward.wgsl:356-382 fed by a 128 B uniform, plane top 8 m (present/fog_fwd.rs:24,27). fog.rs holds
  the CPU bake, FOG_PASS compute plan, budgets 0.2/0.3 ms (fog.rs:24,32-34), sun tint HG g=0.76 (fog.rs:472-566, RL10, Tier C). No froxel volume,
  no shadow tap in fog, no shaft: Grep froxel|volumetric|shaft in crates\render\src finds nothing real.
- Sky: analytic closed form, 64x32 capture (sky.rs:34-36, BETA_R :48, scatter_above :217); one capture feeds sky, IBL, fog ramp (render-look.md:243-248).
  Bruneton/Hillaire LUT bake is rowed RL31, LATER (SPRINT-03-finish-line.md:238). No aerial perspective on scene color.
- Crowd render: none (BOOK-GAPS-2026-10-12.md:136 "no crowd in render/src"). draw.rs caps 256 draws and 256x1024 instances, CPU-built, 1000-agent storm
  fixture (draw.rs:22-33). nav/crowd.rs is ORCA, MAX_AGENTS 256 (:21); LOD_NEAR 15 m / IMPOSTOR 60 m (:17-19) are labels with no renderer.
  Crowd reaction is audio only: Reaction levels 1-3 (audio/src/crowd.rs:94-127) -> game/lib.rs:978 pushes out.reactions. Board: GS20 = VAT bake only
  (SPRINT-18-steals.md:224), CT25/CT34/CT38/CT44/CT64 (content.md:226-258), C3 20k is Tier C (content.md:80), far ring = VG4 impostors (animation.md:80).
- Sword: sim yields integer blade segments p0/p1 per tick, up to 8/tick (combat/sweep.rs:19,197-206). No trail renderer; the word "trail" in render
  is only the TAA ghost gate (post.rs:60).
- Kit has Banner, Torch (2 actors each) and one Stands mesh (game/arena_kit.rs:105-122). No wind, no distortion pass anywhere in render/src.
- Lighting a particle can reuse: forward.wgsl direct_light :333, ibl_irradiance :102, fog_factor_finite :364. One sun uniform (:30); torch light loop
  waits for VS7. The 32-byte LookFrame is frozen, one writer (render-look.md:303).
- Plan next: render-look.md:95-98 puts volumetrics in Tier C "only on measured demand"; FOCUS.md:40 (D7) freezes 2040-look rows until the first real
  frame; arena wiring VS1/VS3/VS4/RA14 comes first (render-look.md:384).

2. STEALS (ranked by value for the duel look/feel per hour)
1. Lit soft flipbook billboards, actually drawn | turanszkij/WickedEngine@cee348b7:WickedEngine/shaders/emittedparticlePS_soft.hlsl + djeedai/bevy_hanabi@04680ac2:src/render/vfx_render.wgsl + src/modifier/output.rs (read 2026-10-12) | MIT / Apache-2.0 | PRESENTATION | lands in crates/render/src/particles_gpu.rs (replace the probe) + shaders/particle.wgsl | M
   Shape: instanced quad per live particle, data by instance id from a storage buffer (Hanabi); soft fade = gather 4 scene depths, take the farthest, (sceneLin-fragLin)/size, saturate, times alpha (Wicked); two-frame flipbook lerp (frame_blend); lit = ambient + sun on a normal rebuilt from the sprite uv, then fog; AlongVelocity stretch for sparks (axis_x = velocity, axis_y = cross(view,x)). Keep our CPU seeded sim (bytes x2), sort 4096 keys on CPU for flame and smoke.
   Moves: pixels drawn by particles 0 (probe) -> 3 emitter kinds, EG8 row PROBE -> LIVE; <= 1.0 ms at 1080p, 4096 live, RTX 3050 (proposal, PARTICLE_BUDGET_MS :33). Proof: `cargo test -p engine-render --lib particles_gpu` with new gpu_draw_changes_pixels (skips NoAdapter) + EG8 on/off arena pair.
   Risk: additive overdraw and TAA ghost. Draw AFTER the TAA resolve, before bloom (Wicked's PS has no motion vectors); ghost gate post.rs:60 must hold. Kill: > 1.0 ms -> dust/smoke at half-res; still over -> cap 2048.
2. Sword swing ribbon trail from the integer sweep | djeedai/bevy_hanabi@04680ac2:src/render/vfx_render.wgsl (ribbon branch) + celyk/GPUTrail@bd0f151e:GPUTrail3D.gd + shaders/trail_draw_pass.gdshader (read 2026-10-12) | Apache-2.0 / MIT | PRESENTATION (reads the sweep, never writes) | lands in new crates/render/src/trail.rs, fed by crates/game | S
   Shape: ring of 16 tick samples of blade tip (WeaponSweep.p1_mm, sweep.rs:197-206) and base (grip joint from the pose); one quad per consecutive pair (Hanabi: previous particle with the same ribbon_id gives axis_x = delta, size.x = length); width from a 1D curve, alpha from a ramp (GPUTrail), width axis = cross(segment, view). Catmull-Rom between ticks because the tip moves metres per tick. One dynamic vertex buffer, one draw.
   Moves: trail code 0 -> 2 fighters x 1 ribbon x 16 segments, 1 draw, <= 0.15 ms (proposal); tip within 1 cm of sweep p1 on a staged slash. Proof: new trail_ring_bytes_x2 via `cargo test -p engine-render --lib trail` + capture of the staged slash.
   Risk: a trail that disagrees with the hit volume lies to the player (gameplay.md:26 "spectacle serves information"). Kill: tip off the sweep by > 5 cm anywhere in the staged tape.
3. Crowd from a static seat buffer + VAT states in the shader | HK416/bevy_open_vat@9d40177f:assets/shaders/openvat_common.wgsl + examples/many_foxes.rs; antzGames/Godot_Vertex_Animation_Textures_Plugin@99ca132d:addons/godot_vat/shaders/vat_multiple_anims.gdshader (read 2026-10-12); numbers from the 80.lv "6,000 agents" article (blog, no code) | Apache-2.0 (+MIT file) / MIT | PRESENTATION | lands in new crates/render/src/crowd.rs + shaders/crowd.wgsl, data from GS20 assets/src/vat.rs | L
   Shape: texture row = frame, column = vertex, bbox remap, position + normal rows, manual lerp floor/ceil frame with nearest filter. Per-instance data is written ONCE (seat pos, yaw, variant, phase offset, start frame, end frame, rate, loop flag); time is one global uniform, so the CPU sends ~64 B per frame, not 20k matrices. Neither donor cross-fades clips; ours blends clip A and B in the vertex shader (4 fetches). Seats are cooked from stands.gltf tier rows.
   Moves: spectators drawn 0 -> 20,000 seats; LOD1 VAT (~600 verts, 6 states) for the nearest ~4,000 in <= 8 draws, <= 2.5 ms total at 1080p RTX 3050 (proposal). Donor warning: 6,072 VAT spectators cost 17.5 ms GPU on an RTX 3070 in UE5 (276 draws) vs 32 ms skeletal; so LOD1 must stay low-poly. Godot plugin limits: 8192 verts, 8192 frames.
   Proof: CT34 card line (content.md:231) + `cargo test -p engine-render --lib crowd` bytes x2. Kill: LOD1 > 1.5 ms at 4,000 -> halve the count, hand the rest to steal 9.
4. Crowd reaction wave from sim cues | IDEA-ONLY (80.lv article: global wave start + sweep duration, per-seat trigger from instance id; no code) | n/a | PRESENTATION | lands in crowd.rs (uniform) + crates/game/src/lib.rs:978 (feed out.reactions) | S
   Shape: <= 8 live events (origin angle, start tick, level 1-3 from audio::Reaction crowd.rs:94-127, kind ooh/cheer/gasp) in one uniform; each seat computes excitement = sum level * bump(t - t0 - dist/speed) with a per-seat hash jitter and blends idle -> duck/cheer/stand clips (duck ticks already in crowd.rs:28-33). A Mexican wave is one scripted event with ring arc-length delay. Zero per-seat CPU work.
   Moves: reacting seats 0 (audio only) -> all; uniform <= 8 x 32 B; same cue tape -> same uniform bytes x2 (TickOutcome.reactions is already presentation). Proof: new wave_uniform_bytes_x2 in `cargo test -p engine-game --lib`.
   Risk: reads as noise; kill if the Q1 A/B pair shows no readable response to a knockdown within 20 ticks.
5. Froxel volumetric fog with sun shafts through arches | godotengine/godot@232e6b14:servers/rendering/renderer_rd/shaders/environment/volumetric_fog_process.glsl (read 2026-10-12) | MIT | PRESENTATION | lands in crates/render/src/fog.rs (FOG_PASS) + shaders/froxel_fog.wgsl + forward.wgsl:397 composite | L
   Shape (4 modes, one file): density + in-scatter per froxel (4x4x4 groups; sun picks a cascade by view depth, one shadow tap, Henyey-Greenstein, emission, sky ambient), front-to-back Beer-Lambert integrate over z (8x8 columns), 7-tap separable blur, temporal reproject with Halton jitter and blend, NaN guard. Slices use pow(z, spread), not pure exponential. Ours: sun only (drop omni/spot/area/GI), g=0.76 as fog.rs:472. The earlier scout said no open froxel WGSL exists; this is a GLSL donor to rewrite.
   Moves: shaft contrast (luma inside vs beside an arch shaft) 1.0 with closed-form fog -> >= 1.3; volume 160x90x64 rgba16f = 7.4 MB, x2 history = 14.7 MB < FOG_TRANSIENT_BUDGET_BYTES 20 MB (fog.rs:41); <= 0.8 ms (today 0.2, fog.rs:32: budget change). All proposal. Proof: `cargo test -p engine-render --lib fog` + EG8 on/off shaft ratio on the arena capture.
   Risk: needs RL27 sun cascades (open), VS1 arches, and lifts "volumetrics" out of Tier C / D7 freeze: lead decides. Potato tier = off, keep closed-form fog. Kill: > 0.8 ms or shimmer over SMAA_SHIMMER_MAX_VAR.
6. Hit flash light + sand burst emitter | IDEA-ONLY for the flash (no donor found); Hanabi AlongVelocity + ScreenSpaceSize read (modifier/output.rs @04680ac2) for streak sparks | Apache-2.0 | PRESENTATION | lands in particles_gpu.rs (EmitterKind rows; EMITTER_COUNT :31 3 -> 4, test :1332 updated) + shadow_cluster/torch_cluster.rs hunk | S
   Shape: a hit spawns >= 24 stretched sparks, one sand burst (64 soft dust) and one point light for 3 ticks into the clustered list, then removes it. Light comes from the cue, not from the particles.
   Moves: lights per hit 0 -> 1 for 3 ticks (back to baseline at tick+4); sand burst kind 0 -> 1. Proof: staged hit test in `cargo test -p engine-render --lib particles_gpu`, bytes x2. Risk: flash blows exposure; kill: auto-exposure step > 0.3 EV (RL18) from one flash.
7. Palette-indexed crowd variety | persontoperson-ptah935/Horde@19cf72ac:instanced-animated-octahedral.ts (read 2026-10-12; MIT but the LICENSE names another author, so provenance is unclear: pattern only, no bytes) | MIT | PRESENTATION | lands in crowd.rs + assets cook | S
   Shape: one grey-coded albedo whose red channel x 255 indexes a palette row; seat hash picks the row (tunic, skin, hair columns). CT38 "variety via the CT4 loop" (content.md:234) becomes a 4 KB texture, not more models.
   Moves: distinct looks per variant 1 -> >= 48 colour triples over 20k seats at +0 draws, +4 KB. Proof: crowd palette test (seat hash -> row histogram x2). Risk: flat look; kill if Q1 pair still reads as one costume.
8. Crowd tier classify + indirect draws on the GPU | djeedai/bevy_hanabi@04680ac2:src/render/vfx_update.wgsl + vfx_indirect.wgsl (read 2026-10-12; atomicAdd into a draw-indirect instance_count, ping-pong read index, prefix-sum of alive counts) + GS18 donor zeux/niagara | Apache-2.0 / MIT | PRESENTATION | lands in crowd.rs on top of GS18/GS18b gpu_cull | M
   Shape: one compute pass over static seats: distance + frustum -> LOD1 list, impostor list or culled; writes two indirect draws. Same pass-0 count reset as vfx_indirect. Fits wgpu without MULTI_DRAW_INDIRECT_COUNT (GS18 note).
   Moves: CPU work per frame for 20k seats O(n) -> 0; draws <= 8 in every camera. Proof: GPU lists equal cpu classify on 20k seeded seats (skips NoAdapter). Risk: iGPU lacks features; kill: potato falls back to impostor-only.
9. Seat-facing animated octahedral impostor ring | Horde@19cf72ac:instanced-animated-octahedral.ts (read) + ektogamat/octahedral-impostor-component (page only: 3-cell barycentric blend, MIT) | MIT | PRESENTATION | lands in crowd.rs second tier + assets cook | L
   Shape: texture-array atlas = views x animation frames; shader maps the camera dir in the seat's local space to an octahedral cell, layer = base + floor(frame + offset); camera-facing quad; per-instance state in storage buffers; donor has no culling and no LOD (we add via steal 8). Our twist: spectators face the pit, so bake only a +-70 deg yaw fan x 3 pitches = 21 views x 32 frames x 48x64: 8.3 MB raw, ~2 MB BC7 per texture.
   Moves: far seats drawn 0 -> ~46,000 as 2-triangle quads in <= 2 draws, <= 0.7 ms (proposal); triangles ~0.1 M vs 28 M if all were LOD1 (46k x 600). Proof: view-pick table x2 + the C3 card. Risk: tier pop and TAA shimmer; use GS17 dither fade. Kill: visible seam at the tier edge in the Q1 pair.
10. Banner and awning wind | mreliptik "3D simple animated flag shader", godotshaders.com/shader/3d-simple-animated-flag-shader (read 2026-10-12) | CC0 | PRESENTATION | lands in forward.wgsl as a vs_wind entry beside vs_skinned :216 | S
   Shape: traveling sine + cosine on vertex y/x/z driven by uv.x, every term times uv.x so the pinned edge stays fixed, plus a lean term. The donor never writes normals; we derive them from the analytic derivative. Wind vector goes in its own small uniform (the 32-byte LookFrame is frozen) and also drifts dust and smoke.
   Moves: animated cloth 0 -> 2 banners (arena_kit.rs:116). Proof: banner vertex bytes x2 at fixed time + EG8 on/off on a banner mask. Risk: negligible; kill if shimmer under TAA exceeds the gate.
11. Heat haze above torches | Wicked emittedparticlePS_soft.hlsl distortion define (read: normal-slot read as a signed offset; the consuming pass was NOT read) | MIT | PRESENTATION | lands in particles_gpu.rs (distortion flag) + post composite_compute.wgsl | S
   Shape: haze particles write a signed 2-D offset to a small target; composite shifts the HDR sample by it. Moves: refraction 0 -> <= 4 px above 2 torches, <= 0.1 ms. Proof: EG8 on/off torch-column mask. Risk: TAA smear; kill on any post.rs:60 ghost gate failure.
12. Dust motes lit by the fog volume | IDEA-ONLY (no donor) | n/a | PRESENTATION | lands in particles_gpu.rs after steals 1 and 5 | S
   Shape: 2,000 tiny additive motes in a box around the camera; brightness = froxel in-scatter sampled at the mote, so they glow only inside shafts. Moves: extra passes 0; shaft readability up. Kill if steal 5 is killed.
13. Footprints and drag marks in sand | IDEA-ONLY (no read donor; commercial TrackMark pattern: one instanced buffer of marks, surface-typed) | n/a | PRESENTATION | lands in decals.rs (DecalKind) once decals are drawn | M
   Shape: foot-contact events stamp instanced decals; 256 marks (MAX_DECALS decals.rs:24) fade over 600 ticks (BLOOD_FADE_TICKS :37). Moves: marks 0 -> 256. Proof: decal placement bytes x2. Risk: blocked until RP3 decals draw (EG8 says PROBE); kill if RP3 is dropped.

3. BOARD ROWS (prefix XX; the lead numbers them)
- [ ] XX1 render: particles drawn, soft, lit, flipbook-blended — RP2 is a bind probe that copies words and draws nothing (particles_gpu.rs:751-772,
  present/gpu_present/submit.rs:71-90; EG8 sets the PROBE/LIVE bar, SPRINT-18-gaps.md:168). Diff: good. Tier A. Size M. Owns
  crates/render/src/particles_gpu.rs, crates/render/src/shaders/particle.wgsl. Accept: `cargo test -p engine-render --lib particles_gpu` green with new
  gpu_draw_changes_pixels (torch-region mask differs above the EG8 epsilon, ON vs OFF, 4096 live) and particle_ms <= 1.0 at 1080p on the RTX 3050; ghost
  metric post.rs:60 not worse. F2P: ON and OFF frames are byte-equal today (probe). P2P: golden unchanged. needs: none. Plan: render-look.md (RP2).
- [ ] XX2 render: sword swing ribbon trail from the integer sweep — no trail code exists; sim already yields blade segments (combat/sweep.rs:197-206),
  gameplay.md:26 wants the telegraph readable. Diff: regular. Tier A. Size S. Owns crates/render/src/trail.rs (new), crates/render/src/shaders/trail.wgsl,
  the lib.rs line. Accept: `cargo test -p engine-render --lib trail` green: 16-sample ring gives vertex bytes x2 equal, tip within 1 cm of sweep p1 on a
  staged slash, 1 draw, trail_ms <= 0.15. F2P: no trail module. P2P: golden unchanged. needs: none. Plan: render-look.md (Feel, FOCUS group 5).
- [ ] XX3 render: crowd draws 20k spectators from a static seat buffer with VAT states — nothing draws a spectator (BOOK-GAPS-2026-10-12.md:136; draw.rs:22-33
  CPU list, 256x1024). Diff: hard. Tier B. Size L. Owns crates/render/src/crowd.rs (new), crates/render/src/shaders/crowd.wgsl, lib.rs line. Accept:
  `cargo test -p engine-render --lib crowd` green and the CT34 card prints seats 20000, LOD1 4000 in <= 8 draws, crowd_ms <= 2.5 at 1080p RTX 3050, CPU upload
  <= 256 B per frame, bytes x2. F2P: no crowd module. P2P: golden unchanged. needs: GS20 CT25. Plan: content.md (C3, CT34); render-look.md owns the pass.
- [ ] XX4 game: crowd reaction wave from sim cues — reactions are audio only (audio/src/crowd.rs:94-127 -> game/lib.rs:978). Diff: regular. Tier B. Size S.
  Owns crates/render/src/crowd.rs (wave uniform), crates/game/src/lib.rs (one hunk reading out.reactions). Accept: `cargo test -p engine-game --lib crowd_wave`
  green: a staged cue tape (hit, parry, knockdown) gives uniform bytes x2 equal, <= 8 events x 32 B, and >= 90 percent of seats within 25 m of the origin leave
  idle within 20 ticks. F2P: no visual reaction. P2P: golden unchanged. needs: XX3. Plan: content.md (CT64).
- [ ] XX5 render: froxel sun fog and arch light shafts — flat closed-form fog only (forward.wgsl:356-382); render-look.md:95 holds volumetrics in Tier C, so
  the lead moves it. Diff: hard. Tier B. Size L. Owns crates/render/src/fog.rs, crates/render/src/shaders/froxel_fog.wgsl, the forward.wgsl:397 composite hunk.
  Accept: `cargo test -p engine-render --lib fog` green; EG8 on/off shaft luma ratio >= 1.3 (closed-form fog: 1.0); fog_ms <= 0.8 at 1080p; volume bytes <=
  20,000,000 (fog.rs:41). F2P: ratio is 1.0 today. P2P: golden unchanged, potato tier renders with it off. needs: RL27 RL9 VS1. Plan: render-look.md.
- [ ] XX6 render: hit flash light and sand burst emitter — hits give sparks only, no light, no dust kick (particles_gpu.rs:94-101,231). Diff: regular. Tier A.
  Size S. Owns crates/render/src/particles_gpu.rs, crates/render/src/shadow_cluster/torch_cluster.rs (one hunk). Accept: `cargo test -p engine-render --lib
  particles_gpu` green: a staged hit spawns >= 24 sparks + 1 sand burst of 64 + 1 flash light for 3 ticks, light count back to baseline at tick+4, bytes x2.
  F2P: no flash light kind, EMITTER_COUNT is 3. P2P: golden unchanged. needs: XX1 VS7. Plan: render-look.md (Feel, FOCUS group 5).

4. PLAN TEXT (render-look.md section 3 and 4; crowd lines also content.md Tier C)
- Section 3 flow: add "froxel fog (compute: sun in-scatter, integrate, reproject) -> forward composite" and "particles and trails draw AFTER the TAA resolve,
  before bloom (soft depth fade, sun+SH lit, frame-blended flipbook)". Particle sim stays the seeded CPU pool (bytes x2); only drawing moves to the GPU.
- Section 3 budgets (1080p, RTX 3050, proposals until PF2 posts): particles 1.0 ms, trails 0.15, crowd 2.5 total (LOD1 + impostor + wave), froxel fog 0.8 (raises
  fog from 0.2), hit flash 0.0 (one light). Transient add: froxel pair 14.7 MB inside the 20 MB fog budget; crowd atlas ~4 MB + VAT ~3 MB.
- Section 3 crowd: crowd.rs owns a static seat buffer written once; per frame only a global time plus <= 8 reaction events (from TickOutcome.reactions).
  Tiers: LOD1 VAT ring, animated impostor ring (seat-facing fan, ~2 MB per texture), GPU classify into 2 indirect draws (GS18/GS18b). No ORCA, no skeletal.
- Section 4 new hard problems: (a) 20k seats on a 6 GB laptop without a per-seat CPU cost; (b) light you can see: shafts through arches without cascade shimmer;
  (c) additive particles and trails without TAA ghosting; (d) a crowd that answers a knockdown within 20 ticks and never costs the tick.
- Potato knobs for GS10: crowd_lod1_count (0 = impostor only), froxel (off, keep closed-form fog), particle_cap, particle_half_res.
- Slim: merge particles.rs into particles_gpu.rs or mark it test-only; wind lives in its own uniform, never in the frozen 32-byte LookFrame.
- Decisions for the lead: move "volumetrics" from Tier C to Tier B for shafts only; lift D7 for XX3 and XX5 after VS1/RA14 land; keep Tier A for XX1, XX2, XX6.

5. NOT WORTH IT
- Physically based aerial perspective on scene color (Bevy aerial_view_lut.wesl, Wicked aerialPerspectiveCS.hlsl; both read): arena depth is <= 300 m; Rayleigh
  blue optical depth = 0.0331/km x 0.3 km ~ 0.01 (sky.rs:48), a 1 percent change, while our artistic fog is 0.02 per m (fog.rs:146). RL31 stays for sun tint only.
- Hanabi GPU alive-list / free-list / indirect sim for particles (vfx_update.wgsl): pool <= 8192 (particles_gpu.rs:29) costs microseconds on the CPU; it pays above
  ~32k and atomic order breaks the bytes-x2 test. Keep the seeded CPU sim.
- Depth-buffer particle collision (Wicked simulateCS.hlsl): the arena floor is flat sand, bounce on a plane; depth-history reprojection fights TAA jitter.
- Screen-space radial-blur god rays: fail when the sun is off screen, which is the arch case from inside the bowl; froxel (steal 5) is the fix.
- GPU XPBD cloth for banners (jspdown/cloth, MIT, graph-coloured Gauss-Seidel; page only): 2 banners (arena_kit.rs:116); sine wind plus analytic normals is enough
  at 5 m and more; L cost and a second solver to keep.
- Skeletal animation or ORCA for 20k seated spectators: seats are static; crowd.rs MAX_AGENTS is 256 (:21); ORCA stays for ingress and egress (NS22 only).

DONE 13 steals, 17 donor files read.
