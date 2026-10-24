# Unity, Godot and itch.io tools: deeper picks for engine2040 (2026-10-24)

Scope: city-builder tooling, crowds and formations, melee and hit-feel, procedural animation and IK, terrain/water/weather, stylized look on weak GPUs, optimization (instancing, culling, LOD, mesh combining), save/replay/undo, editor productivity, audio, and wild itch.io tools. Skips everything already in `03-market-sweep.md`, `01-blender-generators.md`, `02-physics-destruction.md` and the donor list.

## How to read the entries

- **[V]** = the store, repo or vendor page was fetched in this session and the facts below come from that fetch.
- **[S]** = the URL and facts come only from a search-result snippet. Re-check before relying on them.
- Prices are the ones shown on the fetched page. Where a search snippet disagreed with the fetched page, the fetched price is used and the disagreement is noted.
- "Reviews" counts are the number shown on the page. Asset Store pages show no average star rating, so "loved" is a proxy (review volume plus sentiment quoted from reviews).
- The "Why devs love it" lines paraphrase review or listing text. They are not endorsements from us.

## donors-seen check

`C:\engine2040\docs\research\hunt-2026-10-21\donors-seen.txt` was grepped for every pick's repo and publisher token. No pick is an exact repeat. Related seen repos are flagged inside the entries: `zeux/meshoptimizer` (for UnityMeshSimplifier), `Unity-Technologies/Animation-Instancing` and `chengkehan/GPUSkinning` (for GPU crowds), `deepnight/gamefeel` (same author as LDtk, different repo), `dgreenheck/OpenFracture` and `ezy-slice` (not used here).

---

## 1. City-builder tooling: roads, traffic, placement

### 1. Road System (Maxi Barmetler) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/level-design/road-system-192818
- Price: FREE. Latest v2.2.1 (Feb 6, 2026). Reviews: 43 (count shown beside the title, not labelled). Built-in, URP and HDRP compatible (original version 2020.2.1).
- Why devs love it: reviewers call it easy to use and strong enough for production, and recommend it for open environments and cities (search snippet).
- KEY IDEA: a spline-driven road network that generates road meshes along the curves, so a road is data, not hand-placed geometry.
- Steal idea: model roads and zoning as spline rows in our .rows data; generate the road mesh at bake time, not at runtime.

### 2. Easy Roads3D Mesh Gen (KrisDevelopment) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/modeling/easy-roads-mesh-gen-43327
- Price: $9.99. Latest v2023.2.2 (Feb 15, 2024). Reviews: 57, no average shown.
- Why devs love it: reviewers praise ease of use, but say it cannot make intersections or roundabouts (search snippet, reviews page).
- KEY IDEA: road mesh generation from splines with a low entry price.
- Steal idea: a cheap reference for road mesh output. The gap (no intersections) is the part our city builder would have to solve itself.

### 3. Godot Road Generator (TheDuckCow) [V]
- Store: Godot Asset Library / GitHub. Link: https://github.com/TheDuckCow/godot-road-generator
- Price: free, MIT. About 1.2k stars. Godot 4.4+ on `main`/`dev`, plugin 0.9.0 on the `godot4.3` branch. 58 open issues, 13 open PRs (so active).
- Why devs love it: 3D highways and streets with procedural intersections, and a `RoadLaneAgent` node that follows roads (search snippet).
- KEY IDEA: roads as scene nodes, with lane agents that move along the road graph.
- Steal idea: the lane-agent pattern for carts and citizens walking the Roman street network, driven by the same road graph that the AI edits.

### 4. Mobile Traffic System v3 (Gley) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/behavior-ai/mobile-traffic-system-v3-305800
- Price: $129 (seats). Latest v3.6.5 (Sep 29, 2026). Reviews: 66.
- Why devs love it: the search snippet claims traffic-light and priority intersections, roundabouts, lane waiting, and about 300 cars on a 2018 Android device. The fetched page had no feature text, so those claims are [S].
- KEY IDEA: intersections as rules (priority, light, roundabout), not as physics.
- Steal idea: intersection-as-rule for carts and citizen traffic on the potato tier, where full vehicle physics is out of budget.

### 5. Simple Traffic System [S]
- Store: Unity Asset Store (per search). Link: https://discussions.unity.com/t/simple-traffic-system/769273 (forum thread; store page not fetched).
- Price: $35 per search snippet, Unity 2022.3 per the same snippet. Reviews: not checked.
- Why devs love it: waypoint routes that connect modularly, one AI Traffic Controller, job-based logic, traffic lights and yield triggers, and a pool that disables distant cars (forum post).
- KEY IDEA: one controller with a registry of cars, plus a pool that switches off distant agents.
- Steal idea: the controller-plus-pool pattern for citizen carts, with distant agents parked as data.

### 6. DOTS Traffic City [S]
- Store: docs (no store listing checked). Link: https://dotstrafficcity.readthedocs.io/en/lite/
- Price: not checked. Reviews: none found.
- Why devs love it: described as a tool for quickly creating performant city traffic on DOTS (docs).
- KEY IDEA: a DOTS-based city traffic system.
- Steal idea: a reference for the data-oriented traffic layout only. Not a dependency, since it is DOTS-only and the store page was not verified.

### 7. Spline Architect [S]
- Store: Unity Asset Store (per 80.lv article). Link: https://80.lv/articles/spline-architect-s-new-version-adds-gpu-instancing-for-1-000-000-objects-along-splines/
- Price and reviews: not located. The store page was not found in search. Unity 2022 and 6 across pipelines (article).
- Why devs love it: spline tool for roads, intersections and traffic systems, plus mesh deforming. Version 2.7.2 adds instancing for objects placed along splines (article). The article itself warns that the 1,000,000 figure refers to instance records drawn through instanced rendering, not a million GameObjects.
- KEY IDEA: spline-placed instanced props (fences, aqueducts, street dressing).
- Steal idea: instanced placement along a spline for street props on the city builder, with the same caveat applied to our claims.

---

## 2. Crowds and formations

### 8. Enemy Masses Professional (Arawn) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/enemy-masses-professional-massive-crowds-1-click-enemy-ai-rts-fo-371542
- Price: $89 (seats). Latest v2.0.0 (Oct 20, 2026). Reviews: 4. Unity 6000.6.0 (Built-in, URP and HDRP).
- Why devs love it (vendor docs): renders 5,000+ animated units on screen and targets horde and RTS city crowds. Only 4 reviews, so this is mostly vendor text.
- Hard dependency (docs, [V]): requires GPU Instancer Pro for rendering, plus its Crowd Animator add-on. So the real cost is the $89 plus GPU Instancer Pro ($128) plus the add-on ($60, [S]).
- KEY IDEA: a crowd controller that sits on top of GPU instancing and an animation instancer, not a standalone AI.
- Steal idea: the crowd tier is "instanced draw plus simple AI". Copy the layering, not the vendor package. Related seen: `Unity-Technologies/Animation-Instancing`, `chengkehan/GPUSkinning`.

### 9. GPU Instancer Pro (GurBu Technologies) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/gpu-instancer-pro-290293
- Price: $128 (seats), excluding tax. Latest v0.12.20 (Oct 7, 2026). Reviews: 35, no average. Built-in, URP and HDRP on 2022.3.32f1.
- Why devs love it: the store describes GPU instancing and compute-shader rendering that cut draw calls, with culling, LOD management and terrain, tree and prefab support (search and store text). The keyword list on the page points at instancing, occlusion, culling, vegetation and foliage.
- Note: a search snippet gave $64 on sale. The fetched page shows $128, so $128 is used here.
- KEY IDEA: indirect GPU instancing with culling and LOD as one package.
- Steal idea: our potato-tier draw path. Instancing plus culling plus LOD for citizens and props, written as our own Rust draw batcher, with the bundle's feature list as the checklist.

### 10. GPU Instancer Crowd Animations add-on [S]
- Store: Unity Asset Store (per search). Link: https://assetstore.unity.com/packages/tools/animation/gpu-instancer-crowd-animations-145114
- Price: $60 (per search snippet). Reviews: not checked.
- Why devs love it: GPU-based skeletal animation for crowds (listing text via Fab results, vendor claim).
- KEY IDEA: baked animation textures played back on instanced crowds.
- Steal idea: animation-texture crowd playback for citizens and battle crowds, to compare with the seen `chengkehan/GPUSkinning`.

### 11. RTS Entity Controller (Godot Asset Library 4784) [V]
- Store: Godot Asset Library. Link: https://godotengine.org/asset-library/asset/4784
- Price: free, MIT. Godot 4.4. Version 1.1.0.
- Why devs love it: component-based selection, move and attack toolkit for RTS-style units. The page lists no formation or steering component.
- KEY IDEA: units are composed from components, so behaviours can be swapped without rewriting the unit.
- Steal idea: a component pattern for unit roles (legion, auxilia, cavalry). Formation and steering would still have to be built, since the page shows none.

---

## 3. Melee, hit-feel and game juice

### 12. Wallop Lite (Godot Asset Library 5526) [V]
- Store: Godot Asset Library. Link: https://godotengine.org/asset-library/asset/5526
- Price: free, MIT. Godot 4.7 (tested on 4.7.1). Version 1.0.0.
- Why devs love it: trauma-based camera shake for 2D and 3D with seeded noise (repeatable motion), plus hit-stop that restores the previous time scale, so a player's own slow-motion survives. Overlapping hit-stops extend to the longest duration, use the lowest scale, and can be capped.
- KEY IDEA: hit-stop as a composable time-scale stack, not a single `Engine.time_scale` assignment.
- Steal idea: our hit-stop as a stack (overlap rules: longest duration, lowest scale, cap) so combo and parry freezes do not fight each other. Matches our melee hit-feel work.

### 13. Sparkle Lite (Godot Asset Library 5066) [V]
- Store: Godot Asset Library. Link: https://godotengine.org/asset-library/asset/5066
- Price: free, MIT. Godot 4.2+. Version 1.0.0. GDScript only, no dependencies.
- Why devs love it: a `FeedbackPlayer` node chains camera shake, hit pause, screen flash, pooled audio with random pitch, and scale punch into one `play()` call. Sequences are authored in the inspector.
- KEY IDEA: feedback as a data-authored sequence fired by a hit event.
- Steal idea: hit feedback as designer-edited sequence rows that the hit resolver fires. The free counterpart to the Feel idea in the 03 sweep.

### 14. Health, HitBoxes, HurtBoxes and HitScans (cluttered-code) [S]
- Store: Godot Asset Library (edit listing per search; repo `cluttered-code/godot-health-hitbox-hurtbox`, not fetched). Link: https://godotengine.org/asset-library/asset/edit/15463
- Price: free, MIT (per listing). Latest listed 5.0.4 for Godot 4.5 (asset-library edit, search). Earlier 5.0.3 for Godot 4.4.
- Why devs love it: 2D and 3D components for health, damage and healing, with hitbox, hurtbox and hitscan types (listing text).
- KEY IDEA: split hit detection into hitbox (deals), hurtbox (receives) and hitscan (ray), so each kind of attack resolves in one place.
- Steal idea: the hitbox/hurtbox/hitscan split for our melee resolver. Check the repo before copying any code.

### 15. Reactive Bones (breakbox) [V]
- Store: itch.io. Link: https://breakbox.itch.io/reactive-bones
- Price: $19.99 minimum, pay-what-you-want. Reviews: not shown (comments only).
- Why devs love it (listing): bone-based secondary motion (jiggle, hair, tail) for Unity characters, with a setup wizard for existing rigs and a Reactive Rig Painter that paints bone chains and skin weights from mesh regions (search snippet).
- KEY IDEA: paint a region, get the bone chains and weights generated.
- Steal idea: painted-mask auto bones for capes, banners and loose cloth on fighters, instead of hand-rigging each one.

### 16. DOTween (Demigiant) [V]
- Store: official site, also on the Asset Store. Link: https://dotween.demigiant.com/
- Price: free and open source. Pro adds visual editor, path editor and visual manager for a one-time fee (amount not on the page). Unity 6 and newer, and 2022.3 (older releases to 2020.3 and Unity 5).
- Why devs love it: fast, type-safe tween engine with a free core. A single Asset Store reviewer says it is not maintained for a year (one opinion, not verified).
- KEY IDEA: tweens as chainable, type-safe sequences.
- Steal idea: tween curves for telegraph flashes and UI timing. Trivial to add in our own UI layer; the value is the design pattern, not the dependency.

---

## 4. Procedural animation and IK

### 17. Unity Animation Rigging (official package) [S]
- Store: Unity package (Package Manager). Link: https://docs.unity3d.com/Packages/com.unity.animation.rigging@1.2/manual/ConstraintComponents.html
- Price: free (Unity package). Store page not fetched.
- Why devs love it: official constraint components built on the C# Animation Jobs API: Two Bone IK, Multi-Aim, Chain IK, Damped Transform, Override Transform, Twist Chain and more (docs).
- KEY IDEA: IK as constraint components on the rig, not custom IK code.
- Steal idea: two-bone leg IK for foot placement on uneven ground, and multi-aim for head and weapon tracking. A free baseline to beat before buying Final IK.

### 18. Godot 4.7 built-in IK (SkeletonModifier3D) [V]
- Store: built into Godot. Link: https://docs.godotengine.org/en/stable/classes/class_skeletonmodifier3d.html
- Price: free (engine).
- What the page shows: IKModifier3D, SkeletonIK3D and LookAtModifier3D inherit SkeletonModifier3D. The names TwoBoneIK, FABRIK, CCDIK and Jiggle come from a 2020 PR mirror (search) and do not appear on this 4.7 page, so check the current class list before writing code against them.
- KEY IDEA: IK modifiers run on the skeleton inside the engine.
- Steal idea: look-at and two-bone solve on the engine's own modifiers for melee aim and head tracking before any addon.

---

## 5. Terrain, water and weather

### 19. Crest Water 5 (Wave Harmonic) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/particles-effects/crest-water-5-oceans-rivers-lakes-268614
- Price: $240 (seats). Latest v5.10.2 (Sep 30, 2026). Reviews: 127, no average. Built-in, URP and HDRP on 2022.3.62f3.
- Why devs love it (store text): LOD strategies and GPU acceleration, custom shape, foam and dynamic-wave input, used in shipped games and SIGGRAPH talks (search snippet of store text).
- KEY IDEA: an ocean system built around LOD and GPU, with custom wave and foam input.
- Steal idea: a river and lake water tier with LOD for the potato build. The full ocean is more than the map needs, and $240 is the cost of the reference.

### 20. Weather FX Free (heyheythere) [V]
- Store: Godot Asset Library. Link: https://store.godotengine.org/asset/heyheythere/weather-fx-free/
- Price: free, CC BY 4.0 (credit required). Godot 4.3 to 4.7. Version 1.0.2 (marked unstable).
- Why devs love it: rain (with splashes where it meets the ground), snow and fog, each a single shader on one node, with no textures or particle setup and a pixel-art mode.
- KEY IDEA: weather as one cheap screen or node shader per effect.
- Steal idea: a fog and rain shader for the potato tier, with draw distance hidden by fog. Check the CC BY credit before shipping.

### 21. WeatherSystem2D [V]
- Store: Godot Asset Library. Link: https://godotengine.org/asset-library/asset/5328
- Price: free, MIT. Listed Godot 4.3 (version string 4.7).
- What it does: rain, snow and fog, procedural terrain, and reflective water that forms a beach or flows as a river.
- KEY IDEA: a 2D atmosphere kit where terrain and water are generated, not authored.
- Steal idea: a seeded beach and river generator as a 2D preview tool for city maps. Not a runtime dependency.

---

## 6. Stylized look on weak GPUs

### 22. Toony Colors Pro 2 (Unity Asset Store) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/vfx/shaders/toony-colors-pro-2-8105
- Price: $75 list (no sale shown). Latest v2.9.22 (Jul 24, 2026). Reviews: 551, count unlabelled. Built-in and URP. No HDRP.
- Why devs love it: a shader generator covering low- and high-poly toon looks and PBR for URP. Reviewers praise support since 2015. One user found the outline only worked on Built-in before a renderer-feature fix (developer reply).
- KEY IDEA: one shader generator covers the stylized look, so the art direction is a material setting, not a new shader per asset.
- Steal idea: a small, fixed set of stylized shader variants for the city and units, which keeps the shader permutation count low on weak GPUs.

### 23. Bakery GPU Lightmapper (Mr F) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/level-design/bakery-gpu-lightmapper-122218
- Price: $55. Latest v1.985 (Sep 19, 2026). Reviews: 347, no average. Store text: needs a modern Nvidia GPU and 64-bit Windows; no AMD or Mac baking (search snippet).
- Why devs love it: ray-traced bakes that reviewers say look much better than Unity's built-in baking, plus directional modes (dominant direction, RNM, SH, normal-to-diffuse) and AI denoising (search snippet).
- KEY IDEA: GPU ray-traced lightmaps and directional baking, so the runtime needs no real-time GI.
- Steal idea: bake the Roman city's lighting offline and ship the lightmaps. That keeps the runtime GPU cost low, which is the whole point of the potato tier.

### 24. Pixelorama (Orama Interactive) [V]
- Store: GitHub (also itch.io and Steam per README). Link: https://github.com/Orama-Interactive/Pixelorama
- Price: free, MIT. About 10.8k stars. Godot 4.7.1.
- Why devs love it: a Godot-built sprite editor with an animation timeline, onion skinning, tagged frame groups, and free community extensions (for example, converting 2D pixels into 3D voxels, per README). The README warns that dev builds can be unstable.
- KEY IDEA: a sprite editor built on the same engine as the game, with an extension system.
- Steal idea: sprite prep for the 2D UI and icon pipeline, plus its extension pattern for an AI-facing tool that converts art into engine assets.

### 25. Ogmo Editor 3 [V]
- Store: itch.io. Link: https://ogmoeditor.itch.io/editor
- Price: free, MIT, free for personal and commercial use. Latest v3.4.0 (Dec 31, 2020), so the project is older and not actively updated.
- Why devs love it: tile, decal, entity and grid layers, with levels saved as JSON that is easy to parse (page text).
- KEY IDEA: a level file that is plain JSON with typed entity layers.
- Steal idea: an entity-layer JSON format for 2D tactical maps, so the AI can write levels as data without a GUI.

---

## 7. Optimization: batching, LOD, mesh combining

### 26. Mesh Baker (Ian Deane) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/mesh-baker-5017
- Price: $79 (seats). Latest v3.40.2 (Jun 26, 2026). Reviews: 434, no average. Built-in, URP and HDRP on 2022.3.5f1 and 2019.4.0f1.
- Why devs love it: combines meshes and builds texture atlases or texture arrays to cut draw calls, with non-destructive workflow and a runtime API (store text, search snippet). The store page calls it a long-standing optimization tool.
- KEY IDEA: bake a set of props into one atlas and one combined mesh, then swap the originals for the bake.
- Steal idea: a prop batch-bake step in our asset pipeline, so a block of Roman houses or a market stall set becomes one draw.

### 27. UnityMeshSimplifier (Whinarn) [V]
- Store: GitHub. Link: https://github.com/Whinarn/UnityMeshSimplifier
- Price: free, MIT. About 2.1k stars. Last commit date not shown on the page.
- Status: the README says the author is looking for someone to take over the repo ("Up for adoption"), so treat it as unmaintained.
- Why devs love it: a C# port of Fast Quadric Mesh Simplification, with smart linking to avoid holes, border/seam/UV-fold preservation, and a LOD generator API (repo README).
- Related seen: `zeux/meshoptimizer` is already on the donor list and is a faster, maintained choice for the same job.
- KEY IDEA: quadric-error edge collapse with border and seam preservation.
- Steal idea: the border and seam preservation rules for our LOD bake, with meshoptimizer (seen) as the actual implementation.

### 28. Easy Save 3 (Moodkie) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/easy-save-the-complete-save-load-asset-768
- Price: $59 (list, no sale shown). Latest v3.5.27 (Jul 18, 2026). Reviews: about 1,038 (count not labelled; inferred). Original Unity version 2019.4.38.
- Why devs love it (search snippet): key-value save API, auto save with no code, encryption, compression, save slots, cloud storage and backups.
- KEY IDEA: save as a key-value dictionary with slots, encryption and auto save.
- Steal idea: slot and backup rules for player saves. Our own binary format still wins for the world state; the value is the slot and backup policy.

### 29. Ultimate Replay 3.0 (Trial) (Trivial Interactive) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/camera/ultimate-replay-3-0-trial-version-261742
- Price: FREE trial. Latest v3.3.3 (May 16, 2026). Reviews: "not enough ratings"; 22 favourites. Original Unity version 2021.3.45.
- Why devs love it (search snippet): state-based replay that stores snapshots at intervals and interpolates between them, for killcams and ghost cars.
- Note: this records object state. Our replays are input-driven (deterministic), so it is a contrast, not a substitute.
- KEY IDEA: snapshot and interpolate, which is cheap to add but not deterministic.
- Steal idea: a debug timeline that shows physics snapshots for a bug report, beside our input-replay system.

### 30. Save Vault (Godot, Static-Signal) [S]
- Store: Godot Asset Library (listing per search). Repo: https://github.com/Static-Signal/godot-save-vault (not fetched). The store page returned 404 on direct fetch.
- Price: free, MIT. Godot 4.3. Version 1.1.0.
- Why devs love it (listing): encrypted multi-slot save and load with optional Steam Cloud sync, cloud-versus-local conflict resolution, non-blocking autosave, JSON or binary encoding and save/load signals.
- KEY IDEA: cloud-versus-local conflict resolution as an explicit save-slot policy.
- Steal idea: the conflict-resolution rule for multiplayer profiles (which save wins when local and cloud disagree).

---

## 8. Editor productivity

### 31. NaughtyAttributes (dbrizov) [V]
- Store: GitHub (also Unity Asset Store). Link: https://github.com/dbrizov/NaughtyAttributes
- Price: free, MIT. About 5.2k stars. Unity 2022.3 or later (current repo README; older README said 2017.1+).
- Why devs love it: inspector attributes (ShowIf, HideIf, Required, ValidateInput, BoxGroup, Foldout, Dropdown, MinMaxSlider, ProgressBar) without writing custom editors. Works on non-serialized fields and functions.
- KEY IDEA: validation and visibility as attributes on the data field, not in a separate editor class.
- Steal idea: schema-level attributes for our .rows editor (required fields, conditional fields, dropdowns from known enums), so the AI and humans see the same validation.

---

## 9. Audio, sfxr-likes and music tools

### 32. Bfxr (increpare) [V]
- Store: browser tool (bfxr.net), also linked as open source. Link: https://www.bfxr.net/
- Price: not stated on the page (sponsorship and PayPal donation links only). Open source per the page; license name not stated.
- Why devs love it: a sound-effect generator built on drpetter's sfxr, with WAV export and .bfxr save/open (search snippet and page).
- Related sfxr-likes (search only [S]): jsfxr (browser port of sfxr), GodotSfxr and gdfxr (Godot editor plugins), sfxp (PICO-8). Links are the itch.io "games like Bfxr" page, not fetched individually.
- KEY IDEA: parameter-based sound effects, so one seed and a few sliders make a whole effect set.
- Steal idea: a seeded parameter-to-WAV step for hit, parry and footstep sounds in our asset pipeline, so an AI can generate the placeholder sound set.

### 33. Bosca Ceoil (Terry Cavanagh) [V]
- Store: itch.io. Link: https://terrycavanagh.itch.io/bosca-ceoil
- Price: free. Source is open (GitHub: https://github.com/TerryCavanagh/boscaceoil; license name not stated on the page). Marked ARCHIVED on the page.
- Successor (from the page): Bosca Ceoil Blue, an open-source remake, at https://yurisizov.itch.io/boscaceoil-blue
- Why devs love it: a beginner music tool that can be learned in about five minutes, with scale and chord support and over a hundred instrument presets including chiptune (page text). Search snippet: 4.8 out of 5 from 416 ratings.
- KEY IDEA: a constrained music tool (scale and chord locks) that beginners can use.
- Steal idea: scale and chord locks as the constraint for a generated placeholder music loop, so the AI makes music that cannot be out of key.

### 34. SunVox (NightRadio) [V]
- Store: itch.io. Link: https://nightradio.itch.io/sunvox
- Price: free for most systems (page text; no per-platform prices listed). Engine license not stated on the page (search snippet cites BSD-3 for the engine; not verified on the page). Latest v2.1.3 (download section; newest devlog Dec 14, 2024).
- Why devs love it: a modular synth with a pattern-based tracker, WAV/AIFF/XI samples, multitrack WAV export, MIDI in/out and XM/MOD import. A free cross-platform library lets other apps use the engine (page text).
- KEY IDEA: a tracker plus modular synth that exports to WAV, with a library for embedding.
- Steal idea: an offline music and SFX renderer for the potato build's audio assets, exporting WAVs that the pipeline compresses.

---

## 10. Wild itch.io and open tools: level editors, voxels, 3D, physics levels

### 35. LDtk (Deepnight) [V]
- Store: itch.io. Link: https://deepnight.itch.io/ldtk
- Price: name your own price (listed Free). Code MIT. Latest v1.5.3 (Jan 15, 2024). Platforms: Windows, macOS, Linux (macOS and Linux experimental).
- Why devs love it: a 2D level editor from a former Dead Cells lead with tile, entity and integer-grid layers, and auto-layers that draw tiles from a collision map (search snippet). The JSON is easy for any engine to parse and needs no Haxe.
- Related seen: `deepnight/gamefeel` (same author, different repo; not LDtk).
- KEY IDEA: a level editor whose auto-layers turn a collision map into tiles.
- Steal idea: auto-layers and JSON as the map format for the 2D tactical or siege maps, so the AI writes a collision map and the tiles follow.

### 36. Goxel (Guillaume Chereau) [V]
- Store: goxel.xyz (free), source on GitHub. Link: https://goxel.xyz/
- Price: free. License conflict: search snippets say GPLv3 with a separate commercial license from the developer, but the site footer reads "All rights reserved 2019" and names no license. Check the repo before using any code.
- Why devs love it (search snippet): low-poly voxel painting with 24-bit colour, layers, unlimited undo, marching cubes rendering and export to OBJ, PLY, glTF 2, MagicaVoxel and Qubicle. Runs on Windows, macOS, Linux, iOS and Android.
- KEY IDEA: a 3D pixel-art tool where undo is unlimited and layers are first-class.
- Steal idea: the layered unlimited-undo model for our editor's voxel or greybox props. Use the idea only until the license is confirmed.

### 37. TrenchBroom (TrenchBroom project) [V]
- Store: GitHub (free, open source). Link: https://github.com/TrenchBroom/TrenchBroom
- Price: free. License GPL-3.0. About 2.9k stars. Supports Quake, Quake 2, partial Quake 3, Hexen 2, Daikatana and a Generic option. Entity definitions load from FGD and DEF files.
- Why devs love it (search snippet and repo text): brush CSG and vertex editing, a 3D view with optional 2D views, unlimited undo and redo, automatic backups, and an issue browser with automatic quick fixes.
- KEY IDEA: a level editor where the validation list has one-click quick fixes.
- Steal idea: the issue browser with quick fixes for our level validation panel. The AI reports problems and the human clicks the fix.

### 38. Blockbench [S]
- Store: official site (blockbench.net). Link: https://blockbench.net/
- Price: free "for any type of project, forever" (site text). Source GPL v3 (site text, per search). Latest version not checked.
- Why devs love it (site text): cuboid-based low-poly modeling, UV map generation, an animation editor for rigging and keyframing, and a plugin store for extra tools and export formats.
- KEY IDEA: a low-poly editor where the model is cubes and an animation editor sits in the same tool.
- Steal idea: the plugin store pattern for export formats, so a new asset format is a plugin, not a core change.

### 39. Crocotile 3D [V]
- Store: Steam (full) and itch.io (trial). Links: https://store.steampowered.com/app/1244040/ (Steam); https://prominent.itch.io/crocotile3d (itch, search URL, not fetched)
- Price: Steam base price shown in Israeli shekel (about 111 ILS, 20% off until Nov 1). USD price not fetched. The itch.io trial is limited to 100 saved tiles and no OBJ export (GameFromScratch, search snippet).
- Why devs love it (Steam): released May 7, 2015, 261 reviews, 88% positive. Draw and edit modes for tiles in 3D, skinning and rigging, and animated tiles or UVs (search snippet).
- KEY IDEA: 2D tile placement carried into 3D, so a level or building is stamped from tiles.
- Steal idea: tile-stamp input for the city block layout that becomes geometry. Pairs with the Blender kit ideas in `01-blender-generators.md`.

---

## 11. Pathfinding

### 40. A* Pathfinding Project Pro (Aron Granberg) [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/behavior-ai/a-pathfinding-project-pro-87744
- Price: $140. Latest v5.4.7 (Aug 29, 2026). Reviews: 834 (count unlabelled). Original Unity version 2021.3.45.
- Why devs love it (reviews, search snippet): a strong NavMesh alternative with depth and developer support. Warned about a significant learning curve. One review says local avoidance never worked for them.
- Other store option (search snippet): "Pathfinding" (package 17809) has mixed reviews, with one reviewer calling it slow.
- KEY IDEA: grid and navmesh graphs with fast queries, plus local avoidance as a separate step.
- Steal idea: a reference for batching crowd path queries and separating path query from local avoidance. Our own nav and crowd code already covers the core; use this as the benchmark.

---

## Dropped or not verified (kept out of the 40)

- Polarith AI (Unity 92029): free, but last updated Aug 23, 2021 and the page lists no crowd, swarm or formation features.
- Ahab Bones (Unity): deprecated from the Asset Store (store page text).
- EZ Replay Manager (Unity 690): deprecated from the Asset Store (store page text).
- CityCrafter3D (Godot 17891): asset ID not found on fetch.
- Stylized Water 2: the Unity Store page was not located. The only price found (Gumroad, $12.50) is not the Unity Store listing.
- Enviro Lite (Unity 70673): $20, version 2.4.2 (Aug 8, 2022). The publisher could not be confirmed as Mistaken Ideas, so it is left out.
- Simple Traffic System, DOTS Traffic City, Spline Architect, GPU Instancer Crowd Animations, Save Vault, Health/HitBoxes: kept in the list as [S] because the store page or repo was not fetched.
- Godot save add-ons GES, Save/Load Lite, SaveGuard (paid), SaveForge (paid): not fetched or paid. GES (MIT, Godot 4.3, 2.0.0) is a fallback if Save Vault is not verified.
- Hitfeel Kit Lite (inkharbor, itch): found in search, no URL fetched.
- Acro's Hitbox G4 (Godot, MIT, 1.0): found in search, no URL fetched.
- Kenney Starter Kit City Builder (Godot GridMap, MIT per search): URL not confirmed.
- Juicee, Godot Rollback, Feel, Final IK and the other items in `03-market-sweep.md`: already covered, so not repeated.

## Gaps

- Asset Store pages show review counts but no average rating, and some counts are unlabelled. "Loved" is a proxy.
- Prices differ between search snippets and fetched pages. Fetched prices are used, but check the live store page at purchase: GPU Instancer Pro ($64 per snippet vs $128 fetched), Bakery ($27.50 sale per snippet vs $55 fetched).
- Godot IK class names (TwoBoneIK, FABRIK, CCDIK, Jiggle) are not on the current 4.7 class page, so re-check before coding against them.
- Goxel's license is unresolved.
- Fab (fab.com) listings are not included here (not fetchable in earlier research, and out of scope).

---

## TOP 15 (ranked by impact for engine2040)

Ranked by how much each idea would move melee feel, the Roman city builder, multiplayer, and the potato-tier budget, with the cost and licence risk of each as a tiebreak.

1. **GPU Instancer Pro (Unity, $128)**: indirect instancing with culling and LOD. The crowd tier and the potato draw path both need it, and it is the base layer Enemy Masses depends on.
2. **Wallop Lite (Godot, MIT)**: composable hit-stop that restores the previous time scale. Direct melee feel win, free.
3. **Sparkle Lite (Godot, MIT)**: hit feedback as authored sequences. Pairs with Wallop Lite for hit-feel rows.
4. **Unity Animation Rigging + Godot built-in IK (free)**: two-bone leg IK and multi-aim as the free baseline before buying Final IK.
5. **Mesh Baker (Unity, $79)**: prop batch-bake (atlas plus combined mesh). Biggest draw-call cut for city props.
6. **Bakery (Unity, $55)**: offline ray-traced lightmaps so the runtime carries no real-time GI. Nvidia-only.
7. **Road System (Unity, free) + Godot Road Generator (MIT)**: road network as data, with lane agents on the same graph. Core for the city builder.
8. **Mobile Traffic System v3 (Unity, $129)**: intersection-as-rule (priority, light, roundabout). The cheap traffic model for the potato tier. Features [S].
9. **A* Pathfinding Project Pro (Unity, $140)**: the benchmark for batched crowd path queries and separate local avoidance.
10. **Toony Colors Pro 2 (Unity, $75)**: a fixed set of stylized shader variants keeps the look cheap on weak GPUs.
11. **Crest Water 5 (Unity, $240)**: LOD-first water as the reference for a river and lake tier. Expensive, and only a reference.
12. **Weather FX Free (Godot, CC BY 4.0)**: one shader per rain, snow and fog effect, plus fog that hides draw distance.
13. **UnityMeshSimplifier (MIT, up for adoption)**: LOD border and seam preservation rules. Use zeux/meshoptimizer (seen) for the implementation.
14. **Easy Save 3 (Unity, $59)**: slot and backup policy for player saves, with cloud conflict handling as a model (Save Vault [S] as the Godot counterpart).
15. **TrenchBroom (GPL-3.0)**: the issue browser with one-click quick fixes for level validation, where the AI reports and the human fixes.

## Sources
Store and repo pages above (each [V] entry links to its fetched page). Search-only [S] entries link to the page where the snippet was seen.
