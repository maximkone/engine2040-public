# Market sweep: most-loved engine plugins (Godot, Unity, Unreal)

Date: 2026-10-24. Scope: Godot Asset Library, Unity Asset Store, Unreal Fab (old Marketplace). Plain art packs skipped.

## Method and limits

- Searched each store and forum per category, then fetched each candidate page to confirm it loads and to pull price, version and review count. GitHub repos were fetched directly.
- Unity Asset Store pages return review counts and favourites in the fetched text but no star averages, so "loved" here means review volume plus reviewer sentiment quoted from search snippets, not a rating.
- Fab (fab.com) listing pages return HTTP 403 to the fetcher. Fab picks are marked UNVERIFIED and come from search-index listings only. Treat their prices and claims as vendor text.
- "(seen)" marks a pick whose repo or publisher token appears in `docs/research/hunt-2026-10-21/donors-seen.txt`. Matching is by name token, so it is a flag, not a proof of the same thing.
- Dropped anything without a fetched or repo-verified link, and anything deprecated (listed under Dropped).

Legend: `[V]` = fetched and verified, `[U]` = unverified (search index only), `[S]` = seen donor.

---

## 1. Gameplay frameworks (combat, abilities, melee, dialogue)

**Ability & Combat Toolkit (ABC)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/game-toolkits/ability-combat-toolkit-155168
- Price: $39.12 on sale (list $78.25). 83 reviews, 1,221 favourites. Latest v2.4.1 (May 1, 2025).
- Why devs like it: one kit for weapons, attacks, spells and effects. 9 premade weapons, 50+ abilities, 40+ effects. Reviews are mixed; one says the listing oversells bows and dual wield.
- Steal idea: a weapon is a data bundle of timed attacks with effect tags. Put attack frames and effect tags in our rows, not in code.

**Slasher Core** [U]
- Store: Unreal Fab. Link: https://www.fab.com/listings/9c122ccf-3ccf-4ade-bf9d-0cfb5911e714 (Fab returned 403 to the fetcher; listing title and price not confirmed).
- Price: not confirmed.
- Why devs like it: GAS-based melee with combos, block, perfect parry, counter, target lock and motion-warp targeting. Damage types are tags: slash, pierce, strike, melee, unarmed.
- Steal idea: typed damage tags (slash / pierce / strike) feeding armour and detach rules, which fits our armour and detach work.

**Fray - Godot Combat Framework** [V]
- Store: itch.io. Link: https://pyxus.itch.io/fray
- Price: name your own price (can be free). Godot 3.x demo; GitHub repo is 4.2+. Last release v1.0.0 (Jan 3, 2023). The author says it is not in active development.
- Why devs like it: fighting-game style input buffering, complex inputs and hitbox management.
- Steal idea: an input buffer and motion-input parser for the parry and riposte window. Stale, so steal the idea, not the code.

**GoHitHurt** [V]
- Store: Godot Asset Library. Link: https://store.godotengine.org/asset/charles-partous/gohithurt/
- Price: free, MIT. Godot 4.7, v1.0 (marked unstable). 2D and 3D.
- Why devs like it: a hitbox scans and a hurtbox receives, and the signal carries arbitrary data with each hit.
- Steal idea: the hit payload is the attack record (damage, type, knockback), passed per hit and not stored on the collider.

**Saltmire Hitbox Lite** [V]
- Store: Godot Asset Library. Link: https://store.godotengine.org/asset/saltmire/saltmire-hitbox-lite/
- Price: free, MIT. Godot 4.6+, v1.0.0 (marked unstable).
- Why devs like it: drop-in 2D hitbox and hurtbox with teams (no friendly fire), knockback and i-frame windows.
- Steal idea: team filter and i-frame window as reject rules inside hit resolution.

**Dialogue Manager 3** [V]
- Store: GitHub (Godot Asset Library listing not fetched). Link: https://github.com/nathanhoad/godot_dialogue_manager
- Price: free, MIT. 3.9k stars. Godot 4.6+.
- Why devs like it: nonlinear dialogue, localization, custom dialogue balloons.
- Steal idea: swap the balloon UI per context, so Roman advisors and citizens can share one dialogue system with different skins.

---

## 2. Animation and feel (motion matching, IK, ragdoll, game juice, camera)

**Feel (MMFeedbacks inside)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/particles-effects/feel-183370
- Price: $55 (seats). 244 reviews. v6.3 (Oct 20, 2026). Official site: https://feel.moremountains.com/
- Why devs like it: chainable feedbacks (shake, tween, sound, particles, post, UI, haptics) triggered from Unity events or code. Reviews praise effects and support. One reviewer criticises code architecture.
- Steal idea: author juice as data sequences that designers edit, then fire them from hit events. Related seen: deepnight/gamefeel [S].

**Juicee** [V]
- Store: Godot Asset Library. Link: https://store.godotengine.org/asset/kelpekk/juicee/
- Price: free, MIT. Godot 4.3+. v2.0.0 (marked unstable). 117 effects.
- Why devs like it: screen shake, hit-stop, squash and stretch, particle bursts, damage numbers. Driven from code, Inspector or graph editor. Reduced-motion, flashing, shake and vibration switches.
- Steal idea: accessibility switches and a global "juice off" toggle, which doubles as a potato-tier kill switch.

**Final IK** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/animation/final-ik-14290
- Price: $90 (seats). 877 reviews. v2.5 (Apr 10, 2026). Publisher: RootMotion.
- Why devs like it: VRIK and FBBIK solvers, calibration, demo scenes, responsive support. Reviewers note it does not rig for you.
- Steal idea: runtime look-at and foot-plant IK for melee aim and hit reactions, instead of more authored clips.

**PuppetMaster** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/physics/puppetmaster-48977
- Price: $90 (seats). 221 reviews. v1.5 (Apr 10, 2026). Publisher: RootMotion.
- Why devs like it: runtime active ragdolls plus offline baking. Reviewers call it essential for ragdolls. Complaints: hard setup, walls push ragdolls, a Final IK clash, and frame drops with 60 characters (forum).
- Steal idea: per-bone pin and unpin weights, blended from animation into physics for hit reactions. Our physics crate already has ragdoll and limb modules, so this is a direct compare. Related seen: ashleve/ActiveRagdoll [S], sergioabreu-g/active-ragdolls [S].

**Motion Matching for Unity (MxM)** [V] (dated)
- Store: Unity Asset Store (listing price not fetched). Link: https://discussions.unity.com/t/released-motion-matching-for-unity-advanced-character-animation/744955
- Price: not confirmed. Last forum post June 2019 (v1.3b). Demos: MxM v2.1 Stress Test.
- Why devs like it: motion matching in place of Mecanim state machines. Quality depends on source clip coverage.
- Steal idea: a pose-search database over our clips for locomotion. Related seen: orangeduck/Motion-Matching [S], dreaw131313/MotionMatchingByDreaw [S].
- Caveat: the store's other motion-matching listing (Motion Matching System for Unity, about $54.50) has a reviewer saying the docs disappeared and the developer stopped maintaining it.

Note on camera: Cinemachine 3 (free Unity Registry package, not a store item) has a third-person follower and shoulder-pivot aim, which is the baseline to beat. Phantom Camera (Godot) is seen [S] as ramokz/phantom-camera.

---

## 3. Procedural generation (dungeon, city, terrain, WFC, world gen)

**MapMagic 2** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/terrain/mapmagic-2-165180
- Price: base asset FREE. Brush module $45 (per search snippet). 135 reviews. v2.1.20 (Jul 28, 2026). Source code included.
- Why devs like it: a node graph where each node is one generator step (noise, Voronoi, blend, erosion, splines, biomes). Infinite, seeded terrain. Output is standard Unity terrain.
- Known issues (reviews): Unity 6 TerrainPreviewURP shows pink areas; Apple Silicon problems; a duplicate-keyword shader warning.
- Steal idea: a deterministic node graph where the AI edits a graph, not a scene. Same seed and inputs give the same chunk.

**Gaia Pro for Unity 2021** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/terrain/gaia-pro-for-unity-2021-193476
- Price: $149 (seats). 75 reviews. v3.4.1 (Jul 18, 2023). Publisher: Procedural Worlds.
- Why devs like it: fast artist-driven terrain plus population. One reviewer says it produces populated terrain quickly.
- Caveats (reviews): bugs and compile errors, weather and lighting systems clash, confusing product lineup (Gaia Pro, Gaia VS, biome packs).
- Steal idea: biomes as spawn rules that populate a region. Map it to city zoning (farms, quarters, roads).

**Tessera Pro** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/level-design/tessera-pro-161077
- Price: $49.99 (seats). 29 reviews, 654 favourites. v6.4.1 (Mar 2, 2026). Publisher: Boris the Brave.
- Why devs like it: wave function collapse level builder. Support praised; reviewers call it easier to learn than most procedural tools. Complaints: bugs, thin docs.
- Steal idea: WFC with socket rules for modular streets and blocks in the Roman builder. Note: the same author's open-source DeBroglie is seen [S] (BorisTheBrave/DeBroglie).

**Dungeon Architect** [U] (forum verified, Fab listing not)
- Store: Unreal Fab. Fab link: https://www.fab.com/listings/0ad73dc2-3daa-4c29-a70a-61fb9cea0c7c (403 to the fetcher). Forum thread (verified): https://forums.unrealengine.com/t/dungeon-architect/28094
- Price: not confirmed. Free basic tier listed on Fab ("Dungeon Architect Basic").
- Why devs like it: layout and theme are separate, so you can swap an art theme without rebuilding the layout. Reviewers and the vendor describe PCG integration and a Voronoi cell-flow builder. Vendor claims v3.6 (Jun 2026) and UE 5.8 support; not verified.
- Steal idea: send the seed to clients, not the layout, so every client builds the same map. Useful for our multiplayer.

**Terrain3D** [V]
- Store: GitHub (Godot Asset Library listing not fetched). Link: https://github.com/TokisanGames/Terrain3D
- Price: free, MIT. 4.4k stars. Godot 4 (C++ GDExtension).
- Why devs like it: editable, high-performance terrain using geometry clipmaps. Sculpt, holes, texture paint, foliage instancing, heightmap import.
- Steal idea: clipmap terrain with LOD for the potato tier. Better frame cost than a full-res heightmap on iGPUs.

**Zylann's Voxel Tools (godot_voxel)** [V]
- Store: GitHub (Godot Asset Library listing not fetched). Link: https://github.com/Zylann/godot_voxel
- Price: free, MIT. 4.0k stars. Godot 4 (C++ module).
- Why devs like it: volumetric terrain with Transvoxel LOD seams, overhangs and tunnels, runtime editing, infinite paging.
- Steal idea: destructible walls and cut terrain for siege play, instead of heightmap-only.

---

## 4. AI and NPC (behavior trees, utility AI, crowds, LLM NPCs)

**Behavior Designer (Opsive)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/behavior-ai/behavior-designer-behavior-trees-for-everyone-15277
- Price: $95 (seats). 762 reviews, about 10,154 favourites. v1.7.15 (Sep 11, 2026).
- Why devs like it: visual behavior trees with easy custom tasks and many integrations (Playmaker, Visual Scripting). Classic line gets maintenance only; Behavior Designer Pro is the DOTS successor (per the publisher).
- Steal idea: a custom-task registry, so an AI agent can add a new task as a code module without touching the tree.

**Unity Behavior (official)** [V]
- Store: Unity package (com.unity.behavior). Link: https://docs.unity3d.com/Manual/com.unity.behavior.html
- Price: not stated on the doc page. Unity 6.6, package 1.0.16. Real-time debugging.
- Why devs like it: first-party graph-based trees with prebuilt nodes. An optional Muse subscription adds LLM-based tree generation (from the unity docs search snippet).
- Steal idea: have an LLM emit a behavior graph and then validate it, which matches our AI-drives-the-editor setup.

**LimboAI** [V]
- Store: GitHub (Godot Asset Library not fetched). Link: https://github.com/limbonaut/limboai
- Price: free, MIT. 3.1k stars. Godot 4.6 (plugin 1.8.x for Godot 4.7).
- Why devs like it: behavior trees and state machines in C++ with editor tooling and debugging.
- Steal idea: an in-editor AI debugger overlay that shows the active node and the blackboard for each agent.

**Beehave** [V]
- Store: GitHub. Link: https://github.com/bitbrain/beehave
- Price: free, MIT. 3.3k stars. Godot 4.5+ (Beehave 2.10+).
- Why devs like it: behavior trees built as scene nodes, for NPC and enemy AI.
- Steal idea: trees as scene composition, so an AI editing a scene also edits its brain.

**Convai Unity SDK** [V]
- Store: Unity Asset Store (via docs). Link: https://docs.convai.com/api-docs/plugins-and-integrations/unity-plugin
- Price: Core edition free to start (sample scene, basic scripts, character downloader). Paid tiers not stated on the doc page. Unity versions not stated.
- Why devs like it: LLM characters with a backstory, a knowledge base and voice. Unity lists it as a verified AI-marketplace solution.
- Steal idea: an NPC is a data asset (backstory, knowledge, voice), so every citizen is authored data.

**NobodyWho (Godot)** [V]
- Store: GitHub (Godot Asset Library listing for it per search). Link: https://github.com/nobodywho-ooo/nobodywho
- Price: free, EUPL-1.2. 1.6k stars. Godot 4.5+ AssetLib; desktop and Android, no iOS.
- Why devs like it: runs LLMs locally with no API key. Nodes NobodyWhoModel and NobodyWhoChat.
- Steal idea: offline LLM NPCs for dev and playtest. Heavy on weak hardware (needs a GGUF model), so keep it off the potato tier.

Dropped from this category: Mass Battle and Crowd Simulation System Pro (Fab, no fetched link, vendor-only claims), Inworld (no link), Player2 AI NPC Godot (GPL-3.0, 32 stars, low signal).

---

## 5. Rendering, VFX and look on weak hardware

**Amplify Impostors** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/amplify-impostors-119877
- Price: $60. 112 reviews. v1.0.4 (May 28, 2026). Publisher: Amplify Creations.
- Why devs like it: baked impostors cut triangle counts sharply (reviewer: 10000 to 6 triangles). Developer support praised. The store says it is beta and only does in-editor baking; real-time generation is promised.
- Steal idea: impostor far-LOD for crowds of citizens and tree lines. Related seen: ektogamat/octahedral-impostor-component [S].

**Rawteous** [V]
- Store: itch.io. Link: https://voidoverzero.itch.io/rawteous (the link redirects from hugobeyer.itch.io)
- Price: $29 minimum, pay what you want above that. Unity 6+ with URP 12+. HDRP planned. No reviews found.
- Why devs like it: automatic billboard LODs generated from 3D models to cut draw calls and memory. Newest entry here, so treat it as unproven.
- Steal idea: generate billboard LODs in our import pipeline and keep the billboard set as a build artifact.

**Automatic LOD (Ultimate Game Tools)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/modeling/automatic-lod-43652
- Price: $90 (seats). 118 reviews. v1.63 (Apr 3, 2026).
- Why devs like it: long-running LOD generator and mesh decimator, generally strong reviews. Reviewers say it lacks batch processing and impostor support is limited.
- Steal idea: auto LOD chain on import with a triangle budget per potato tier. Epic's built-in Mesh Simplification Tool and HLOD are the free baseline for Unreal (Epic docs, not a Fab item).

**Dynamic Fog & Mist 2 (Kronnect)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/vfx/shaders/fullscreen-camera-effects/dynamic-fog-mist-48200
- Price: $29.99 (single entity). 62 reviews. v9.2.5 (Oct 23, 2026). URP, desktop, mobile and VR.
- Why devs like it: animated volumetric-looking fog that works across desktop, mobile and VR at low cost.
- Steal idea: cheap depth fog that hides draw distance on the potato tier.

Dropped from this category: Stylized Water 1 (deprecated by its publisher), Vegetation Studio Pro (publisher reply says deprecated; Unity terrain now has foliage instancing), Buto (no link fetched; desktop-class performance), Epic Simplygon (dropped support).

---

## 6. Tools and editor (inspector, console, debug, AI editor bridges)

**Odin Inspector and Serializer** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/odin-inspector-and-serializer-89041
- Price: $55 per seat. About 792 reviews, about 12,382 favourites. v4.1.0.0 (Oct 17, 2026).
- Why devs like it: powerful inspector customization and a serializer with polymorphism, cyclic references and null handling. Full source access. One critic says Unity's UI Toolkit and AI tools are overlapping it.
- Steal idea: generate a schema-driven editor from our .rows schema, so both humans and AI see the same typed form.

**Editor Console Pro (FlyingWorm)** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/utilities/editor-console-pro-11889
- Price: $30 per seat. 453 reviews. v3.984 (Sep 15, 2026). Reviewers call it a must-have that replaces Unity's console.
- Steal idea: searchable, filterable log with stack traces for our agent and build logs. Modest, but cheap to copy.

**Unity MCP (CoplayDev/unity-mcp)** [V] [S]
- Store: GitHub. Link: https://github.com/CoplayDev/unity-mcp. MIT, 15.4k stars.
- Why devs like it: an MCP server that lets an AI assistant edit scenes and GameObjects, edit C# scripts, manage assets, run tests and build.
- Steal idea: the pattern we already use for the editor bridge. Flagged seen: the donor list has CoplayDev/unity-mcp.

Dropped from this category: Debug Drawing Extension (publisher deprecated it), Prefabricator and Prefab Tool (no fetched link, prefab tool limits BSP), Prefab Brush (Unity-only per listing).

---

## 7. Networking (rollback, lag compensation, server-authoritative)

**Godot Rollback Netcode (dsnopek)** [V]
- Store: Godot Asset Library. Link: https://godotengine.org/asset-library/asset/2450
- Price: free, MIT. v1.0.0-alpha. Godot 4.2.
- Why devs like it: rollback and prediction that also syncs timers, animation, random numbers and sound, with debugging tools.
- Steal idea: our engine already has deterministic hashing and replay laws. Port the RNG-sync and debug-tracing idea to Rust so melee rolls back cleanly.

**FishNet** [V]
- Store: Unity Asset Store (price not fetched) and GitHub. Link: https://github.com/FirstGearGames/FishNet
- Price: not confirmed. 2.1k stars on GitHub. GitHub page shows no license name and no release.
- Why devs like it: server-authoritative by default, built for bandwidth-sensitive games. A studio says it shipped an MMO on it (promotional). A forum comment says a comparison chart lists it as "source available" with paid features. Unverified, so check the license before use.
- Steal idea: server-authoritative hit confirm with client prediction and reconciliation for melee.

**Mirror** [V]
- Store: Unity Asset Store (package 129321, price not fetched). Link: https://mirror-networking.com/
- Why devs like it: open-source, simple API, big community. Forum comparisons still list it next to FishNet and NGO.
- Steal idea: a transport-agnostic layer to prototype lobbies before committing to a netcode.

**GGRS (bevy_ggrs)** [V] [S]
- Store: GitHub (gschup/ggrs). Flagged seen: donors list has gschup/ggrs and gschup/bevy_ggrs.
- Why devs like it: GGPO-style rollback in Rust, which fits our Rust engine.
- Steal idea: use it as the reference for our rollback loop. Flagged seen, so not a new donor.

Dropped from this category: Delta Rollback (asset-library edit records only, repo page not fetched).

---

## 8. Weird and wild

**Unity Sentis (now Inference Engine naming in places)** [V]
- Store: Unity package com.unity.sentis (docs: https://docs.unity3d.com/Packages/com.unity.sentis@2.0/manual/install.html). Price: free package. Unity 2023.2+.
- Why devs like it: runs ONNX models on-device in the game, for smart NPCs and perception. Unity's own launch post cites object identification and speech recognition.
- Steal idea: small on-device models for NPC decisions (fast, offline). Keep the model tiny for the potato tier.

**Fracturing & Destruction (Ultimate Game Tools)** [V]
- Store: Unity Marketplace. Link: https://marketplace.unity.com/packages/tools/physics/fracturing-destruction-9411
- Price: $60 (seats). 228 reviews. v1.41 (Apr 3, 2026). BSP and Voronoi fracture, plus an event editor for particles, prefabs and sounds.
- Steal idea: runtime Voronoi breakage for walls and gates in siege play. Related seen: dgreenheck/OpenFracture [S], DavidArayan/ezy-slice [S].

**Ghost Recorder** [V]
- Store: Unity Asset Store. Link: https://assetstore.unity.com/packages/tools/integration/ghost-recorder-181480
- Price: $6.99. Only 3 reviews. v1.0 (Oct 23, 2020). Records a run and replays it as a ghost, with save and load.
- Steal idea: ghost replays for AI playtests, so we can diff two runs. Low signal, so treat as a cheap experiment.

Dropped from this category: Destructible Wall (itch, no reviews, not fetched), Kinematica (no sources returned).

---

## Dropped (not verified or deprecated)

- Mass Battle (Fab), Crowd Simulation System Pro (Fab), Prefabricator (Fab), Buto (Unity), Inworld (Unity): no fetched link or vendor-only claims.
- Simple Motion Matching: publisher says it is deprecated.
- Stylized Water 1 (v2.2.1): publisher says it is deprecated.
- Vegetation Studio Pro: publisher reply says deprecated.
- Debug Drawing Extension: publisher says it is deprecated.
- Fantastic City Generator ($300, 67 reviews): verified, but mostly art-pack content and reviews say it is hard to extend. Not picked.
- Delta Rollback: repo not fetched.

---

## TOP 15 (ranked by how much the idea would make our games or AI-driven building better)

1. **MapMagic 2 (Unity)**: deterministic node graph with seeded chunks. An AI edits a graph, not a scene. Biggest fit for AI-driven building.
2. **LimboAI (Godot)**: behavior trees plus an in-editor debugger that shows the active node and blackboard per agent. Steal the debugger, not the engine.
3. **Godot Rollback Netcode**: rollback with RNG and timer sync. Melee rolls back cleanly only if the RNG is in the sync set.
4. **FishNet (Unity)**: server-authoritative hit confirm with client prediction and reconciliation. Check the license first.
5. **PuppetMaster (Unity)**: per-bone pin and unpin weights blended from animation into physics for hit reactions.
6. **Feel (Unity)**: feedback chains as designer-edited data, fired from hit events.
7. **Ability & Combat Toolkit (Unity)**: weapons and attacks as timed data bundles with effect tags.
8. **Final IK (Unity)**: runtime look-at and foot-plant IK for melee aim and recovery.
9. **Tessera Pro (Unity)**: WFC with socket rules for modular streets and blocks.
10. **Dungeon Architect (UE)**: send the seed, not the layout, so every client builds the same map. Layout and theme kept separate.
11. **Amplify Impostors (Unity)**: impostors for far-LOD crowds and tree lines.
12. **Automatic LOD (Unity)**: LOD chains on import with a triangle budget per potato tier.
13. **Odin Inspector (Unity)**: generate a typed editor from the .rows schema so humans and AI share one form.
14. **Juicee (Godot)**: juice with accessibility switches and a global off switch, which doubles as a potato-tier kill switch.
15. **Terrain3D (Godot)**: clipmap terrain with LOD for iGPU and low-VRAM machines.

---

## Seen-donor flags (summary)

- Saltmire juice listing: seen (saltmire/saltmire-juice). Not picked (Juicee is picked instead).
- Feel: related seen (deepnight/gamefeel).
- Tessera Pro: author's DeBroglie seen (BorisTheBrave/DeBroglie).
- MxM: seen (orangeduck/Motion-Matching, dreaw131313/MotionMatchingByDreaw).
- PuppetMaster: related seen (ashleve/ActiveRagdoll, sergioabreu-g/active-ragdolls).
- Amplify Impostors: related seen (ektogamat/octahedral-impostor-component).
- Phantom Camera: seen (ramokz/phantom-camera).
- Unity MCP: seen (CoplayDev/unity-mcp).
- GGRS: seen (gschup/ggrs).
- Fracturing: related seen (dgreenheck/OpenFracture, DavidArayan/ezy-slice).
- Not seen: Behavior Designer, Unity Behavior, LimboAI, Beehave, Dialogue Manager, Terrain3D, Voxel Tools, MapMagic 2, Gaia, Feel (repo-level), Juicee, Final IK, PuppetMaster, Odin, Editor Console Pro, Amplify Impostors, Automatic LOD, Dynamic Fog & Mist 2, Rawteous, Sentis, Ghost Recorder, FishNet, Mirror, Godot Rollback Netcode, Ability & Combat Toolkit, GoHitHurt, Saltmire Hitbox Lite, Fray, Slasher Core, Dungeon Architect, NobodyWho, Convai.

## Gaps

- Fab: the fetcher gets 403 from fab.com, so all Fab prices and claims are unverified. Search returned mostly listing IDs with no titles.
- Ratings: Unity pages show review counts but no star averages, and Godot pages show no download counts. "Most loved" is a proxy.
- Weird and innovative: search found little real developer praise for anything in the "game changer" category. Picks here lean on verified listings, not on proof of impact.
- Utility AI, Smart Objects, and Kinematica: no verified plugin found.
- Cinemachine alternatives on the store and Unreal Mass-AI crowd tools: no verified plugin with reviews.
