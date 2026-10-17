# Fighter look and gore: cheap realistic shading and impact feel

Date: 2026-10-17. Scope: 12 techniques for fighter skin, cloth, metal, wounds,
blood, hit feel, camera and trails that fit the 60 and 30 fps budgets on weak
GPUs. Owner policy: port from any source; each entry still names the licence
we found so the port carries the right PORT() note.

Method and limits:
- WebSearch (standard mode) found the leads. WebFetch read the HTML pages named
  under each technique.
- WebFetch cannot read PDF bodies here, and pdftoppm is missing, so the Read
  tool cannot render PDFs. PDF-only sources are marked "PDF not read".
- Anything not fetched as a page is marked "search snippet" or "unverified".
- Numbers below are copied from fetched pages. None is invented.

Our side (checked 2026-10-17):
- Plan budgets (docs/plan/render-look.md section 3, 1080p, reference GPU):
  GTAO 0.8 ms, sky 0.4, post 1.5, particles 1.0, decals 0.5, fog 0.2,
  contact 0.3, SMAA 0.5 (sum 5.2 ms); trails 0.15 ms also listed. The gore set
  (decals 0.5, particles 1.0, trails 0.15) is 1.65 ms of that look budget.
- Frame times: 60 fps = 16.6 ms, 30 fps = 33.3 ms.
- Files that exist: crates/render/src/decals.rs (bind probe, RP3 open),
  particles_gpu.rs (bind probe), ibl.rs, sky.rs, v1_skin.rs, aa_scale.rs,
  present/gpu_present/live.rs (forward + blit only), crates/material/src/compile.rs,
  crates/game/src/fighter_look.rs, fighter_look_feet.rs, fighter_look_inertia.rs,
  fighter_look_proofs.rs, ragdoll_look.rs, camera_feed.rs (scripted orbit),
  crates/combat/src/gore.rs (DismemberLite cue only), crates/physfx.
- No weapon-trail module found by name in crates/render or crates/game.

## 1. One material table, one BRDF

What: one GGX specular plus Lambert path, with a material table (lit,
subsurface, cloth, unlit) and fp16-safe clamps. Matches RL46 "one BRDF".
Source: Filament material docs, https://google.github.io/filament/main/filament.html
(fetched). Licence: Apache-2.0, https://github.com/google/filament (fetched).
Numbers stated: clear-coat roughness clamp [0.089, 1.0]; roughness clamp 0.089
(1/roughness^4 there is about 6.274e-5); smallest half normal 2^-14 (about
6.1e-5); anisotropy floor 0.001; Frostbite analytic-light clamp 0.045, fp32 only.
Maps to: crates/material/src/compile.rs, the forward material branch.
Cost: S.
- [ ] QL1 look material: one table, one BRDF, Filament clamps as named consts. Diff: material table generated into compile.rs; clamps 0.089 / 0.001 / 2^-14 as consts; no second BRDF text. Tier B. Size S. Owns crates/material, forward WGSL material part. Accept: CPU naga gate green (CQ23); every material id maps to one table row; grep finds one BRDF text. F2P: test fails when a clamp is removed. P2P: golden d57b5c8ae3096614 unchanged. needs: RL46a.

## 2. Metal energy compensation

What: GGX single scatter loses energy at high roughness. Compensate with the
directional-albedo table that IBL already needs.
Source: Filament energy-loss section (fetched, above); Kulla and Conty, Imageworks,
SIGGRAPH 2017 course (PDF not read); blog.selfshadow.com/2018/05/13/multi-faceted-part-1/ (fetched, HTML).
Licence: Filament Apache-2.0. Kulla-Conty slide licence unverified.
Numbers stated: f_r = f_ss + F0 (1/r - 1) f_ss, with r = integral D V <n.l> dl
stored in the DFG table. Filament code line: energyCompensation = 1.0 + f0 *
(1.0 / dfg.y - 1.0). LUT size not stated. Self Shadow 2018: uncompensated GGX
conductor at reflectance 1 loses "close to 60% of the light" at top roughness.
Maps to: crates/render/src/ibl.rs DFG path (LF2), metal branch of RL47.
Cost: S (M if LF2 is not yet in).
- [ ] QL2 look metal: energy compensation from the DFG table. Diff: ibl.rs DFG gives r; forward metal multiplies by 1 + F0 (1/r - 1). Tier B. Size S. Owns crates/render/src/ibl.rs, forward metal path. Accept: white-furnace GGX conductor at roughness 1.0 recovers most of the ~60% loss (tolerance printed); per-pass ms on RTX 3050. F2P: furnace test fails without the factor. P2P: golden unchanged; ibl bake bytes x2. needs: LF2.

## 3. Cloth: Charlie sheen lobe

What: cloth lobe for capes, tabards, banners. Fighter cloth is presentation only,
so this is a material change, not a sim change.
Source: Filament Cloth section (fetched, same page as 1). Original: Estevez and
Kulla, "Production Friendly Microfacet Sheen BRDF", Imageworks, SIGGRAPH 2017
course (PDF not read; title and authors from search snippet and Self Shadow listing).
Licence: Filament Apache-2.0. Estevez-Kulla licence unverified.
Numbers stated: Charlie D = (2 + 1/alpha) sin^(1/alpha) / (2 pi); sin^2 clamp
2^-7 (0.0078125); visibility denominator 4 (NoL + NoV - NoL NoV); Fresnel removed;
sheen colour default 0.04.
Maps to: crates/material/src/compile.rs cloth material id; cape presentation.
Cost: S.
- [ ] QL3 look cloth: Charlie sheen lobe for capes and tabards. Diff: cloth material id with Charlie D, the 4(NoL+NoV-NoL NoV) denominator, the 2^-7 clamp. Tier B. Size S. Owns crates/material, cape presentation material. Accept: CPU reference matches Filament formula within 1e-3 at 9 angles; backlit cape shows a rim in a posted capture. F2P: reference test fails with lobe removed. P2P: golden unchanged (cloth never hashed). needs: RL46a.

## 4. Skin: wrapped diffuse before the terminator LUT

What: wrapped Lambert so skin does not go black at the terminator. Fallback for
RL50 until that LUT lands.
Source: Filament Cloth section, optional wrapped diffuse (fetched). Filament's
Subsurface section is "[TODO]" on that page, so it gives no skin parameters.
Screen-space SSS (Jimenez-style): not verified here.
Licence: Apache-2.0.
Numbers stated: f_d = (c/pi)(1-F) <(NoL + w)/(1+w)^2> <c_sss + NoL>, w fixed 0.5
(the 2.25 is (1+0.5)^2).
Maps to: fighter_look.rs skin uniform; forward skin branch; RL50 replaces it.
Cost: S.
- [ ] QL4 look skin: wrapped diffuse fallback until the terminator LUT. Diff: skin material uses w 0.5 with the 2.25 normalisation; RL50 replaces it. Tier B. Size S. Owns crates/game/src/fighter_look.rs skin uniform, forward skin branch. Accept: skin per-pass ms posted on RTX 3050 with 8 fighters; terminator band in a fixed-light capture is narrower than the Lambert baseline (posted). F2P: test that wrap term is 2.25 at NoL 1, w 0.5. P2P: golden unchanged. needs: RL46a.

## 5. Potato guard: fp16 and aliasing

What: shared fp16 and specular-aliasing guards so the same shaders run on iGPU
and 2-4 GB VRAM (potato floor, memory potato-target).
Source: Filament (fetched): target OpenGL ES 3.x class; clamps as in 1. The page
states no frame-time or ALU numbers.
Maps to: aa_scale.rs (scale holds), guard consts in compile.rs, web tier WEB8.
Cost: S.
- [ ] QL5 look potato: fp16 and specular-aliasing guards in every look shader. Diff: one guard function (clamps from QL1); potato preset switches the fp32-only clamp off. Tier B. Size S. Owns crates/render shader consts. Accept: potato preset capture (720p, 2 GB VRAM emulated) shows no specular shimmer above the aa_scale floor; per-pass ms posted. F2P: naga gate fails if a look shader lacks the guard. P2P: golden unchanged. needs: QL1.

## 6. Wound region mask, clip plane and cap

What: a wounded part is clipped at a plane and capped; the region mask comes from
the hit, not a second mesh. Matches BX7 and BX41 (stump cap).
Source: Valve, Left 4 Dead 2: Alex Vlachos, GDC 2010, "Rendering Wounds in Left
4 Dead 2". Slides http://alex.vlachos.com/graphics/Vlachos-GDC10-Left4Dead2Wounds.pdf
(PDF not read). Valve Infected shader wiki https://developer.valvesoftware.com/wiki/Infected_(shader)
returned HTTP 403; search snippet: shader removes the shot part, supports slash
types and dynamic gore. 2019 GPUOpen zombie talk (PDF not read, fetch too large)
credits Vlachos for per-vertex region masks (search snippet).
Licence: unverified; pattern only, no code read.
Numbers: none read. Plan bar: cut presentation <= 0.3 ms per cut (body-and-break section 2).
Maps to: fighter_look*.rs per-part state; combat gore.rs (today DismemberLite only);
clip in v1_skin.rs; cap cook from BX8. Presentation only: reads the Cut event,
never the pose.
Cost: M.
- [ ] QL6 look wound: clip plane and cap from a per-part region mask. Diff: gore.rs maps Cut to a mask bit per part; skinned draw clips at the plane and draws a two-sided cap (BX41 shape). Tier B. Size M. Owns crates/game/src/fighter_look.rs, crates/combat/src/gore.rs, skin clip in v1_skin.rs. Accept: scripted cut at a fixed tick draws the cap on 8 fighters within 0.3 ms; Off, Reduced, Full give one golden hash. F2P: mask changes with Cut and not with the pose. P2P: golden d57b5c8ae3096614 unchanged. needs: BX7 BX8.

## 7. Hit-stop as a presentation freeze

What: hold the presented pose 50-100 ms on a heavy hit. The 60 Hz tick keeps
running (hash rule); only the presented sample freezes.
Source: Wayline "Game Feel & Juice" lesson 2, https://www.wayline.io/learn/game-feel/1
(fetched): "Freezing for even 50-100ms on a heavy hit makes it feel like it
connected with real force." Its bullets say "a few milliseconds". eastondev blog
(search snippet, not fetched): 0.05-0.1 s "works best". No shipped game verified.
Godot Engine.time_scale doc, https://docs.godotengine.org/en/stable/classes/class_engine.html
(fetched): default 1.0; scales delta; not auto-adjusting physics_ticks_per_second;
large factors reduce physics precision. We do not use time_scale for hit-stop.
Licence: lesson page licence unknown; we take only the numbers.
Maps to: presenter sampling in crates/game (windowed loop), not the tick.
Cost: S.
- [ ] QL7 feel hit-stop: 50-100 ms presentation freeze on heavy hits. Diff: presenter holds a pose sample for N ms from Cut energy; N capped at 100 ms; tick unchanged. Tier B. Size S. Owns crates/game presentation sampling, windowed loop. Accept: hit-stop on and off give one golden hash; freeze printed in the receipt, within 50-100 ms on heavy hits; no freeze over 100 ms in a 600-tick run. F2P: tick count unchanged during freeze. P2P: golden d57b5c8ae3096614 unchanged. needs: BX9.

## 8. Screen shake: trauma, rotation only, capped

What: trauma value that decays; shake is rotation, not translation, and small.
Source: Squirrel Eiserloh, GDC 2016, "Sprucing up cameras with math" (title from
https://www.gamedeveloper.com/programming/video-sprucing-up-cameras-with-math,
fetched; that page gives no formula; slides not read). Wayline lesson (fetched):
"keep it small; big shake nauseates." Rotation-in-3D advice: search snippet from
a jccc blog (secondhand). Vlambeer's Jan Willem Nijman gave "The art of
screenshake" (snippet only).
Licence: none read. Trauma exponent and max angle: unverified; do not port from memory.
Maps to: camera_feed.rs (scripted orbit, presentation). Zero sim effect.
Cost: S.
- [ ] QL8 feel camera: trauma shake, rotation only, hard cap. Diff: camera_feed adds a rotation offset from a decaying trauma value; cap in degrees set by PF2 captures. Tier B. Size S. Owns crates/game/src/camera_feed.rs. Accept: cap printed in degrees; translation offset zero in a test; shake off in the proof capture. F2P: translation offset is zero. P2P: golden unchanged. needs: -.

## 9. Kill cam and slow motion from a stored track

What: slow replay of the killing blow from a ring of presentation samples.
Why not re-run: App::presentation re-simulates from tick 0 on every call
(render-look section 1; body-and-break section 1). Per-frame calls would hitch.
Source: mechanism from the Godot time_scale doc (see 7). No shipped kill cam verified.
Licence: Godot docs; Godot is MIT per its project (unverified here).
Maps to: App::presentation in crates/game/src/lib.rs (never per frame); a ring of
the last 2 s of samples; camera_feed.rs for the path; playback below 1.0 rate.
Cost: M.
- [ ] QL9 feel kill cam: slow replay from a stored pose ring. Diff: ring of 2 s of presentation samples; replay reads the ring at a rate below 1.0; App::presentation not called during replay. Tier B. Size M. Owns crates/game/src/lib.rs presentation ring, camera_feed.rs. Accept: no frame over 16.6 ms in a 600-tick run that triggers a kill cam; ring bytes posted. F2P: App::presentation runs zero times during replay. P2P: golden unchanged. needs: QL7.

## 10. Weapon trails

What: ribbon from the last N blade samples per swing; previous-palette vectors for
fast blades (steals RL37/RL38).
Source: Godot "Sword Slash Effect - 3 Different Versions", https://godotengine.org/asset-library/asset/edit/1809
(search snippet; repo AlphinAlbukhari/Godot-Sword-Slash-with-Visual-Shader, GLES2
and GLES3 Visual Shader demo). Licence MIT per the snippet (unverified).
Unity lightsaber, Tvtig/UnityLightsaber: 12 triangles per frame (search snippet);
README says "educational" with no licence, so do not port code from it.
Our budget: trails 0.15 ms at 1080p on the reference GPU (render-look section 3).
Maps to: new trail pass file in crates/render, after look passes, before bloom;
samples from the Cut tape.
Cost: S.
- [ ] QL10 look trail: ribbon from blade samples with motion-vector history. Diff: ribbon of the last 8 blade samples per swing; alpha ramp; previous-palette vectors for fast blades. Tier B. Size S. Owns crates/render trail pass, crates/game blade samples. Accept: trail pass <= 0.15 ms at 1080p on RTX 3050 with 8 fighters swinging (PF2); no ghosting on the 100 px/frame blade case in an A/B. F2P: 8 samples give 7 quads. P2P: golden unchanged. needs: RL37.

## 11. Clustered decals for blood and wounds

What: projected decals grouped in clusters, so cost is per cluster; cap per match.
Source: Bevy examples/3d lists decal.rs, clustered_decals.rs and
clustered_decal_maps.rs (https://github.com/bevyengine/bevy/tree/main/examples/3d,
fetched). Bevy licence: not on that page; believed MIT OR Apache-2.0 (unverified;
check LICENSE before any port). DecalCo, Godot (MIT per asset-library snippet,
unverified): https://godotengine.org/asset-library/asset/edit/2815 . Gib and
blood pools: no source verified.
Numbers: plan bar decals 0.5 ms at 1080p (render-look section 3). No decal count
stated by Bevy examples.
Maps to: crates/render/src/decals.rs (bind probe; RP3 open). Wound maps from QL6
feed the atlas. Blood pools go to the RL49 ground-state map past the cap.
Cost: M.
- [ ] QL11 look blood decals: clustered projected decals with a per-match cap. Diff: decals.rs gets cluster lists and a count cap; past the cap, blood pools write to the RL49 ground map. Tier B. Size M. Owns crates/render/src/decals.rs, decal atlas. Accept: decal pass <= 0.5 ms at 1080p on RTX 3050 with 8 fighters after 600 ticks; cap printed; 0 validation errors. F2P: the 201st decal (at cap 200) is refused with a typed error. P2P: golden unchanged. needs: RP3, QL6.

## 12. Hit sparks, dust and blood spray

What: GPU particles for sparks, dust and blood spray, with counts capped per tier.
Source: no verified count or timing. Intel Chaos and Niagara article redirected
and was not read. GDC 2026 Virtuos "Controlled Chaos" session (search snippet)
lists no gore topics.
Numbers: plan bar particles 1.0 ms at 1080p (render-look section 3).
Maps to: crates/render/src/particles_gpu.rs (bind probe, submit open). Spray reads
the Cut event; particle state is presentation, never hashed.
Cost: M.
- [ ] QL12 look particles: sparks, dust and blood spray with per-tier caps. Diff: GPU emitters for three kinds, count caps by tier; submit path replaces the bind probe. Tier B. Size M. Owns crates/render/src/particles_gpu.rs. Accept: particle pass <= 1.0 ms at 1080p on RTX 3050 in a fight capture with 8 fighters; per-tier counts printed; 0 validation errors. F2P: a tier cap refuses a spawn over the cap. P2P: golden unchanged. needs: RP2.

## What must render at 60 and 30 fps on weak GPUs

- Every tier, near-free: QL1 (one BRDF), QL5 (guards), QL4 (skin wrap),
  QL7 (presentation hit-stop), QL8 (rotation shake). S each, no new pass.
- 3050 tier, scaled on potato: QL2 (one multiply per metal pixel), QL3 (cloth
  lobe), QL6 (cap and mask, capped visible cuts), QL10 (trail, 0.15 ms).
- Measured before it stays: QL9 (ring memory), QL11 (decal cap), QL12
  (particles). Each needs a PF2 per-pass number; a miss kills the row with the
  number.
- Arithmetic only: the named look passes sum to 5.2 ms of the 16.6 ms frame, so
  the gore set (1.65 ms) must fit inside what the other passes leave. 30 fps
  (33.3 ms) is the honest potato floor.

## Corrections to search results (checked)

- The GDC 2016 camera talk is "Sprucing up cameras with math", not "Juicing your cameras".
- "Juice it or lose it" is by Martin Jonasson and Petri Purho (snippet); Nijman
  of Vlambeer gave "The art of screenshake".
- The Chaos search found no gore, blood or decal technique.

## Unverified (do not port numbers from these)

- Eiserloh trauma exponent and max angle (talk slides not read).
- Vlachos 2010 wound technique and counts (PDF not read).
- Valve Infected shader details (wiki returned 403).
- Kulla-Conty, Estevez-Kulla and Frostbite 2014 PDF content beyond Filament's restatement.
- 0.05-0.1 s hit-stop range from eastondev (search snippet).
- Licences of Bevy, DecalCo and the Godot sword demo (not read on the repos).
- 12 triangles per frame trail (one educational repo).
- L4D2 console values violence_hblood and violence_hgibs (blog snippet).

## Proposed board rows

QL1 to QL12 are the bullet rows under each technique. None touches the golden
hash. Each needs a PF2 per-pass number before it closes.
