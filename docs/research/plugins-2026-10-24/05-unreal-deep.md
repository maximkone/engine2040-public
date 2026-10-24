# Unreal deep dive: Epic systems and Fab picks for a Roman melee + city game on weak PCs

Research date: 2026-10-24. Target: engine2040 (Rust): Roman melee, city builder, multiplayer, ragdoll and destruction, AI agents driving the editor, must run on a potato (iGPU, 2-4 GB VRAM, 8 GB RAM).

## How to read this

- **[verified]**: an Epic primary doc page was fetched in this pass and the facts and numbers below come from that page's text.
- **[search-only]**: the fact comes from search-result text, a Fab listing text, a forum, a blog, or an Epic landing page that carried no detail. Re-check before relying on it.
- Fab (fab.com) returns HTTP 403 to fetchers, so all Fab facts are vendor text from search results. Prices are given only where captured; otherwise "not captured".
- Where a search result showed several Fab listing IDs without names, the name-to-ID mapping is marked "unmapped" and every candidate ID is listed.
- Epic landing pages for PCG, StateTree and Mutable returned only overview text in the fetch. Their deeper claims are [search-only].

## Skip and overlap rules applied

- **Skipped (already in donors-seen.txt or the three plugins-2026-10-24 files):** Chaos Destruction, Chaos Flesh, Cable Component, Physical Animation Component, Chaos Vehicles, Niagara Fluids, FluidNinja, RayFire, DinoFracture, Obi (rope, softbody, fluid), Ultimate Rope Editor, Magica Cloth, PuppetMaster, Edy's Vehicle Physics, Slasher Core, Dungeon Architect, Mass Battle (Fab), Fantastic City Generator, Ability & Combat Toolkit, Feel, Final IK, Behavior Designer, Automatic LOD, Amplify Impostors, Odin, Ghost Recorder, Fracturing & Destruction (Ultimate Game Tools), chongdashu/unreal-mcp (donors-seen line 375), Red Faction, The Finals, Blast, Houdini Bullet glue.
- **Overlap flagged, kept because the brief names them:** Mass Entity (donors-seen lines 422 `component/MassEntity` and 1180 `MassEntity/SmartObjects`), Smart Objects (line 1180), Motion Matching (lines 564 `dreaw131313/MotionMatchingByDreaw`, 777 `Gems/MotionMatching`, 1286 `Multiplayer/MotionMatching`, 1389 `orangeduck/Motion-Matching`). Those donor entries are other implementations; the entries below are Epic's own systems. Drop entries 2, 5 and 8 if the earlier pass already covered them.

---

## A. Epic built-in systems (UE5)

### 1. Animation Budget Allocator [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/animation-budget-allocator-in-unreal-engine
- Price: built-in (plugin).
- What it does: caps animation work on registered skeletal meshes under a per-platform game-thread budget in milliseconds.
- Key technique: ranks registered components by significance (Auto Calculate Significance, or code through `SetComponentSignificance`). Over budget, it applies levers in order: stop ticking components (favouring a leader pose), lower the update rate, interpolate between updates, or enable reduced work. Defaults from the page: `a.Budget.BudgetMs` 1.0; `a.Budget.MaxTickRate` 10; `a.Budget.MaxTickedOffscreen` 4; `a.Budget.MaxInterpolatedComponents` 16; `a.Budget.InterpolationMaxRate` 6; emergency reduced work at pressure 2.5. The budgeted mesh class is `SkeletalMeshComponentBudgeted`. The page does not give the significance formula.
- Steal for engine2040: one global "animation ms budget" in the frame scheduler, with per-character significance from distance and screen size, and the four levers as a throttle ladder. Applies to citizens, distant fighters and crowd extras alike.
- Potato note: the cheapest scalability lever in this list. Set the budget per device tier through scalability settings.

### 2. Mass Entity and Mass Gameplay (crowds, representation LOD, replication LOD) [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/overview-of-mass-gameplay-in-unreal-engine
- Price: built-in. ISM animation and Mass Replication are marked experimental.
- What it does: data-oriented entity framework. Mass Gameplay adds spawning, in-world representation, LOD, replication and StateTree configuration.
- Key technique: Mass Representation chooses per LOD level one of: high-res Actor, low-res Actor, Instanced Static Mesh (ISM), or no representation. ISM is the cheapest. Mass LOD has four levels (High, Medium, Low, Off) with configurable distances and entity caps, and a significance from 0.0 (High) to 3.0 (Off). Three users of LOD: visualization (distance and frustum), simulation (variable-frequency updates by LOD) and replication (per-client relevancy). StateTree only configures entity data; the docs say its logic "is only used to configure or set data for the entities". Spawning uses a MassSpawner Actor and a Mass Entity Definition Asset.
- Steal for engine2040: the three-tier representation (full mesh near, ISM mid, nothing far) as the crowd rule for citizens and soldiers. Fragments and archetypes map onto our data-oriented crate layout.
- Potato note: this is the path that makes thousands of citizens possible on a 2-4 GB card. Keep ISM mid-range and no-mesh far.

### 3. City Sample (reference project) [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/city-sample-project-unreal-engine-demonstration (Fab listing https://fab.com/s/5e8f5eda64d8, listing name unmapped)
- Price: free sample project.
- What it does: a playable UE5 learning city (Big City about 4 km by 4 km, and a Small City) built from The Matrix Awakens tech. It shows World Partition, crowds, traffic, MetaHumans, Lumen, Nanite, Virtual Shadow Maps, TSR, Chaos Vehicles, MetaSounds and Niagara.
- Key technique: Houdini generates city layout data, and a custom Rule Processor turns it into instances (the page does not mention PCG). Mass Spawners drive crowds (StateTree behaviour) and traffic (no StateTree). Crowd members near the camera are fully rigged MetaHumans; distant ones are vertex-animated meshes.
- Steal for engine2040: the Rule Processor pattern (layout data in, instances out) is the model for our city builder's build step. Vertex-animated far crowds are the fallback for distant citizens.
- Potato note: the stated hardware (12-core CPU, 64 GB RAM, RTX 2080 class, 8 GB VRAM) is far above target. Use it as an architecture reference, not a performance reference.

### 4. PCG framework (Procedural Content Generation) [search-only]
- Link: https://dev.epicgames.com/documentation/en-us/unreal-engine/pcg-development-guides ; generation modes https://dev.epicgames.com/documentation/unreal-engine/using-pcg-generation-modes-in-unreal-engine
- Price: built-in.
- What it does: a toolset for procedural content inside the editor, graph-based over point data.
- Key technique: generation modes split the domain into grid cells, each with its own local PCG component. Large cells suit trees and boulders; small cells suit grass. Four modes, non-partitioned by default. Guides cover World Partition integration with Data Layers and HLOD Layers, shape grammar (a fence example), and runtime generation. Houdini Digital Assets can run inside a PCG graph (SideFX doc). Production-ready-in-5.7 claims come only from third-party blogs.
- Steal for engine2040: cell-based generation for city blocks. A block is generated when its cell streams in, and a seed rebuilds it. Our seeded rule graph does the same outside the engine.
- Potato note: runtime generation cost was not measured. Bake grass and props to ISM or static meshes, and generate only streamed cells.

### 5. Pose Search / Motion Matching [verified]  (overlap flagged, see above)
- Link: https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine
- Price: built-in. Most Pose Search APIs are marked Experimental.
- What it does: query-based pose selection, a replacement for state machines and blendspaces.
- Key technique: a Pose Search Database indexes animations through a Pose Search Schema with weighted, normalized channels (Trajectory, Pose bone positions/velocities/rotations, Position, Velocity, Heading, Phase). Queries run Brute Force, PCAKDTree or VPTree (VPTree is experimental and not for production). Query cost is the sum of channel costs, so fewer channels is cheaper. Sample rate and permutations trade memory for accuracy; pose pruning merges similar poses.
- Steal for engine2040: a small schema for melee locomotion (position, velocity, heading: three channels). Our fighter clips become the database and the engine picks the next pose on attack and block cues.
- Potato note: each extra channel and permutation multiplies cost. Keep three channels, use PCAKDTree, cap the database.

### 6. Motion Warping [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/motion-warping-in-unreal-engine
- Price: built-in.
- What it does: adjusts root motion so a montage lands on a target.
- Key technique: Anim Notify States define warp windows in a montage. Blueprint or C++ adds a warp target by name. Point providers: none, a static transform, or a bone. Modifier family: Skew Warp (ends at the target location and rotation), Precomputed Warp, Adjustment Blend Warp. Options: Warp Translation and Ignore Z-Axis.
- Steal for engine2040: melee lunges and finisher snaps. The attack montage warps the fighter onto the opponent's hit point, so one clip covers many distances. Rows store the target name and the window frames.
- Potato note: root-motion only, so no IK solver cost.

### 7. IK Rig and IK Retargeter [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/ik-rig-animation-retargeting-in-unreal-engine
- Price: built-in.
- What it does: IK Rig defines solvers for pose editing on skeletal meshes. IK Retargeter moves animation between skeletons with different proportions.
- Key technique: retargets across different bone counts, bone names and orientations. Optional IK keeps hands and feet in contact. FK, IK and root motion retargeting can be toggled independently for debugging. Both IK Rig assets need retarget chains defined first. Control Rig is a separate system and was not covered in this pass.
- Steal for engine2040: offline retarget of Mixamo or Rocketbox clips onto our rig with foot-contact IK, baked to our clip format.
- Potato note: bake at build time. Never run the retargeter on a potato at runtime.

### 8. Smart Objects [verified]  (overlap flagged, see above)
- Link: https://dev.epicgames.com/documentation/unreal-engine/smart-objects-in-unreal-engine---overview
- Price: built-in.
- What it does: level-placed objects that agents and players use, queried at runtime.
- Key technique: a global database with spatial partitioning. An agent calls `FindSmartObjects` (user tags, activity tags, behaviour class, search area), then `ClaimSmartObject` to reserve a slot (claim handle), walks to the slot transform, calls `Use` (slot Occupied), and calls `Release` when done. If a Smart Object changes state it releases its slots and notifies through `OnSlotInvalidatedDelegate`. Objects carry no logic. Behaviour definitions are Mass Entity Behavior or Gameplay Behavior.
- Steal for engine2040: a job and facility registry. A bench, well, forge or hitching post is a claimable slot, and a citizen queries by need tags and walks there. This is the city economy's job assignment.
- Potato note: claims avoid per-agent scans, so the cost stays low. Keep search radius small.

### 9. Gameplay Interactions plugin [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/GameplayInteractionsModule
- Price: built-in.
- What it does: runtime module that runs Smart Object behaviour through StateTree.
- Key technique: a Smart Object Behavior Definition wraps StateTree execution; a Gameplay Interaction StateTree schema; slot user-data records which actor occupies a slot.
- Steal for engine2040: a citizen's activity (bake bread, sweep the street) is a StateTree attached to the Smart Object slot. The object supplies the behaviour, the actor just runs it.
- Potato note: one StateTree per behaviour class, shared across slots, to keep memory down.

### 10. StateTree [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/state-tree-in-unreal-engine
- Price: built-in.
- What it does: hierarchical state machine that merges behaviour-tree selectors with state-machine states and transitions.
- Key technique: the landing page gave only the concept and the phrase "highly performant logic". States, tasks, conditions, evaluators and considerations are in the linked overview pages, which were not read in this pass.
- Steal for engine2040: enemy and citizen AI as data (rows) with an in-editor debugger. This fits the AI-drives-the-editor setup.
- Potato note: no cost figures found. Keep per-entity ticking rare, as Mass does.

### 11. World Partition, HLOD, Data Layers, One File Per Actor [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/world-partition-in-unreal-engine
- Price: built-in.
- What it does: streams a large world in grid cells.
- Key technique: one persistent level divided into streamable cells by a runtime grid (2D Runtime Hash is the default). Actors are assigned by Is Spatially Loaded. Streaming sources: Player Controllers by default, plus a World Partition Streaming Source component (priority, target state Loaded or Activated, target grid, target HLOD Layer). The page's example loading range is 768 m. HLODs are built from the Build menu or the HLODs Builder commandlet using HLOD Layers. A console variable caps how many cells load at once. Actors in a disabled Data Layer do not load. Warnings: multiple runtime grids reduce performance; Level Blueprint references pin actors to Always Loaded, so prefer Blueprint Classes; One File Per Actor reduces file checkouts.
- Steal for engine2040: stream the city by district, one cell per block. HLOD for far districts. Data Layers for faction or era state. One File Per Actor for several agents editing the scene.
- Potato note: the streaming radius is the memory lever. HLODs are built offline, so the distant city is cheap.

### 12. Lumen low-end path and scalability levels [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine
- Price: built-in.
- What it does: Epic's dynamic global illumination and reflections, with modes for weak GPUs.
- Key technique: Software Ray Tracing traces one merged Global Distance Field and is the recommended path for GPUs without hardware RT. Hardware RT is more accurate and costs more; its top-level acceleration structure is rebuilt every frame, so keep the ray-tracing scene under about 100,000 instances after culling. At Medium, Lumen reflections fall back to Screen Space Reflections and Medium GI uses Irradiance Field Gather (aimed at low-end PCs). At Low, Lumen is off, so compensate with `r.SkylightIntensityMultiplier=0.7`. Cvars named on the page: `sg.GlobalIlluminationQuality`, `sg.ReflectionQuality`, `r.Lumen.Reflections.Allow`, `r.Lumen.Reflections.DownsampleFactor` (1 or 2), `r.Lumen.ScreenProbeGather.RadianceCache.NumProbesToTraceBudget`.
- Steal for engine2040: a three-tier lighting ladder (probes, SSR, none) behind one quality knob, plus an instance cap on any ray-traced pass.
- Potato note: Low is the potato path. Irradiance Field Gather is the Medium answer.

### 13. Nanite fallback mesh control [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/nanite-technical-details
- Price: built-in.
- What it does: a coarse stand-in mesh for Nanite meshes.
- Key technique: the fallback is generated from the LOD0 slot, or a custom one can be used. Fallback Target: Auto, Fallback Triangle Percent (0 to 100), or Fallback Relative Error. It is used on platforms without Nanite, for complex per-poly collision, for baked lightmaps, and for Lumen hardware ray-traced reflections. Custom path: Fallback Triangle Percent 0, traditional LODs, Minimum LOD 1. Foliage should enable Preserve Area; Max Edge Length Factor protects WPO-deformed meshes.
- Steal for engine2040: building kit pieces (walls, arches) as Nanite with an explicit fallback percent. Tiny props stay on hand-made LODs.
- Potato note: Nanite is not free on a potato. Use traditional LODs where fallback overhead outweighs the gain. Skeletal Nanite status is unverified (sources conflict on the cvar name and whether it is beta or experimental). Do not rely on it.

### 14. Procedural Vegetation Editor (PVE) and Nanite foliage [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/procedural-vegetation-editor-pve-in-unreal-engine
- Price: built-in (experimental).
- What it does: a graph-based tool for Nanite-ready vegetation. Presets ("Megaplants") bundle skeletal meshes and materials. Needs the PCG and Dynamic Wind plugins.
- Key technique: graph-driven generation. A third-party comparison (StraySpark, unverified) suggests HISM for grass and rocks and Nanite foliage for dense trees. Epic labels Nanite foliage experimental in 5.7.
- Steal for engine2040: vegetation presets as data, matching our Sapling-style tree presets in the Blender list.
- Potato note: HISM for ground cover. Nanite foliage only for hero trees, if at all.

### 15. Chaos Cloth [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/physics-in-unreal-engine
- Price: built-in.
- What it does: cloth simulation for games and real-time work, with wind, Animation Drive and Blueprint parameters.
- Key technique: the shared simulation config has a local-space option that reduces jitter far from the world origin. A forum thread (about the NvCloth era, so labels may be stale) says clothing constraints force CPU skinning of the simulation mesh, which shows as its own cost.
- Steal for engine2040: cloth for banners and cloaks only. Sacks and capes can use the Verlet cable approach already picked in the physics list.
- Potato note: the CPU-skinned simulation is the risk. Cap the cloth count and turn cloth off at distance.

### 16. Mutable (character customization) [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/mutable-overview-in-unreal-engine
- Price: built-in plugin. Community wiki says experimental in 5.1 and beta in 5.5.
- What it does: generates skeletal meshes, materials and textures from CustomizableObject assets, in the editor or at runtime. Per-character CustomizableObjectInstance.
- Key technique: parameters drive mesh and material composition. C++ and Blueprint APIs for in-game changes. MetaHuman guide exists. Epic FAQ: Mutable is not a character library, so artists still make the parts. The fetch of the overview returned only a table of contents, so merging and atlasing mechanics are unverified.
- Steal for engine2040: soldier and citizen variants (tunic colour, helmet, cloak, scars) from one object, merged at load into one mesh and atlas, so a crowd of 200 costs a few draw calls. Verify the merge claim first.
- Potato note: runtime generation spends CPU at load. Bake static variants for the potato tier.

### 17. Gameplay Ability System (GAS) [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/gameplay-attributes-and-attribute-sets-for-the-gameplay-ability-system-in-unreal-engine
- Price: built-in.
- What it does: framework for abilities and attributes of RPG or MOBA type.
- Key technique: Attribute Sets register with the actor's Ability System Component. Each attribute keeps a base and a current value; effects change current (temporary) or base (permanent). Gameplay Effects are data-only. Gameplay Tags gate abilities. Gameplay Cues handle feedback.
- Steal for engine2040: stamina, poise and guard as attributes, with block, parry and riposte as tag-gated abilities. Effects as rows, not code.
- Potato note: no performance figures in the pages read. Measure it in our engine.

### 18. Lyra sample game (Game Feature plugins) [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/lyra-sample-game-in-unreal-engine ; abilities https://dev.epicgames.com/documentation/unreal-engine/abilities-in-lyra-in-unreal-engine
- Price: free sample (not rechecked).
- What it does: learning project for UE5 frameworks: Game Feature plugins, Experiences, GAS, input, inventory, interaction.
- Key technique: content loads through Game Feature plugins (ShooterCore holds pawns and mechanics). Abilities come from hero data, Game Features, the Experience, or Equipment. A new mechanic is a new ability asset plus a grant.
- Steal for engine2040: faction packs (Roman legion, Gallic tribes, Germanic warbands) as load-on-demand plugins, and equipment as ability grants.
- Potato note: load only the packs a match needs. That saves memory on 8 GB machines.

### 19. Mover (experimental CMC successor) [verified]
- Link: https://dev.epicgames.com/documentation/unreal-engine/mover-in-unreal-engine
- Price: built-in (experimental).
- What it does: modular actor movement with rollback networking, through the Network Prediction plugin or Chaos networked physics.
- Key technique: rollback movement. The Mover API's liaison component decides reconciliation: `ShouldReconcile` compares predicted and authority sync states. Epic intends Mover to replace the Character Movement Component eventually; CMC stays supported with advance notice.
- Steal for engine2040: deterministic rollback for melee hit timing across clients. Use it as the reference for resimulation; our engine already has replay laws.
- Potato note: no meaningful cost beyond the movement and networking work itself.

### 20. Iris replication system [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/introduction-to-iris-in-unreal-engine ; migration https://dev.epicgames.com/documentation/unreal-engine/migrate-to-iris-in-unreal-engine
- Price: built-in (opt-in).
- What it does: opt-in replication for larger worlds and higher player counts, with lower server cost (Epic's claim).
- Key technique: keeps a copy of all networked state, tracks per-connection state, filters relevancy per connection, and prioritises replication order. Needs the registered subobject list. Objects without push model fall back to polling by NetUpdateFrequency. The migration page's support table conflicts with its text on subobject support, so check before use.
- Steal for engine2040: per-connection relevancy and priority for city-scale state, so a client only receives citizens near it.
- Potato note: server-side benefit. Our host machine gains; clients see no cost.

### 21. Replication Graph [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/migrate-to-iris-in-unreal-engine (section "Differences Specific to Replication Graph")
- Price: built-in.
- What it does: class-based relevancy and priority graph for many actors.
- Key technique: the migration table lists Replication Graph as supporting both ReplicateSubobjects and the registered subobjects list. A dedicated Replication Graph page was not found in this pass.
- Steal for engine2040: spatial grid relevancy for citizens and buildings, as the alternative to Iris.
- Potato note: relevancy on the server keeps the city from being sent in full.

### 22. MetaSounds [search-only]
- Link: https://dev.epicgames.com/documentation/unreal-engine/metasounds-the-next-generation-sound-sources-in-unreal-engine
- Price: built-in.
- What it does: DSP-graph sound sources, procedural at runtime. Replaces Sound Cues.
- Key technique: sample-accurate, buffer-level timing. Each instance renders as its own graph. Graph widgets (knobs, sliders, event buttons) and a real-time output meter.
- Steal for engine2040: synthesise impact and clash sounds from parameters (weight, material, speed) instead of shipping many wav files. Saves RAM on the potato tier.
- Potato note: DSP runs on the CPU. Keep graphs small and share sources, since per-hit instances add up.

### 23. Temporal upscaling and screen percentage (TSR) [verified via the City Sample page]
- Link: https://dev.epicgames.com/documentation/unreal-engine/city-sample-project-unreal-engine-demonstration (TSR example on that page; the TSR doc itself was not read)
- Price: built-in.
- What it does: render below native resolution and upscale.
- Key technique: the City Sample page reports GPU frame time falling from 57.50 ms to 33.37 ms when upscaling 1080p to 4K with TSR. For lower-spec systems the page recommends reducing viewport screen percentage.
- Steal for engine2040: a render-scale setting in the potato preset (around 0.67) with temporal upscaling, exposed to the AI settings tooling.
- Potato note: the largest single lever for an iGPU. Test at 1080p and upscale.

### 24. Epic Online Services (EOS) [search-only]
- Link: https://onlineservices.epicgames.com/ (SDK reference is on the Developer Portal, not this domain)
- Price: free per Epic's page: no royalty or hosting fees, no caps on titles or accounts. Deeper anti-cheat tiers are a paid agreement.
- What it does: online services for Unreal, Unity and custom engines: lobbies, sessions, P2P, matchmaking and related services.
- Key technique: lobbies come before the game server starts and are often peer-to-peer. Lobby attributes are numeric, string or boolean key-value pairs, and the lobby owner runs the matchmaking step (Edgegap guide, third party). Commercial Fab wrappers: EOSCore (listing claims maintenance since 2020) and EOS Kit (listing IDs 8e78818d-4662-4c6d-8ab2-6e22b9d3595e and acbbf98b-d863-4b33-9699-25eb429b4d08, names unmapped).
- Steal for engine2040: lobby and P2P join for Roman skirmish co-op. Hit confirmation stays host-authoritative in engine2040.
- Potato note: P2P needs upload bandwidth, so a dedicated host helps on poor connections. A third-party guide says player-hosted servers invite cheating.

---

## B. Fab and third-party (vendor text, search-only)

### 25. Gameplay Moveset System (Fab) [search-only]
- Link: listing ID 31584c47-047e-4646-9b83-8f895e1c3c81, or 0c79e08b-0440-4dfc-9321-79ba4a9dcbe3, or 134c516c-8841-4a35-b587-cc13b92ac1a9 (the search does not map these to names; the Moveset name is one of the three)
- Price: not captured. Targets UE 5.7.
- What it does: combo state machines, hit windows, replication. Leaves damage, VFX and sound to the developer.
- Key technique: visual combo graph editor; hitbox AnimNotifyState pipeline; Enhanced Input auto-binding. Warns that remote-controlled pawns need VisibilityBasedAnimTickOption set to Always Tick Pose or Refresh Bones, or authority-side hit detection silently fails.
- Steal for engine2040: attacks as hit-window rows (startup, active, recovery frames) with per-frame hitboxes, the same shape as our combat rows. Pre-empt the tick-option bug by running hit detection on the authority only.
- Potato note: run hitboxes on the authority, not on every client.

### 26. Ultimate Hit Reaction System (Fab) [search-only]
- Link: listing ID unmapped; one of 31584c47-047e-4646-9b83-8f895e1c3c81, 0c79e08b-0440-4dfc-9321-79ba4a9dcbe3, 134c516c-8841-4a35-b587-cc13b92ac1a9.
- Price: not captured.
- What it does: hit reactions set up in Blueprint and a Chooser DataTable, with no coding. 168 motion-captured reactions. Selects by hit direction and intensity. Version 1.4 adds IK-based reaction feedback.
- Key technique: a Chooser table maps (direction, intensity) to a reaction clip.
- Steal for engine2040: a direction by intensity lookup table in rows, choosing the reaction clip. Our hit reactions can use the same shape.
- Potato note: the lookup is cheap, but 168 clips are memory. Cull to the clips in use.

### 27. Medieval Combat System Vol 01 (Fab) [search-only]
- Link: listing ID unmapped (as entry 26).
- Price: not captured.
- What it does: third-person melee pack with light, heavy, charged and roll attacks, and reactions tied to attacks. No audio included.
- Key technique: modular Blueprints and attack-to-reaction pairing.
- Steal for engine2040: a reference attack taxonomy for a sword-and-shield set. Read it for the list, not for the code.
- Potato note: none.

### 28. Roman Suburbs: Ancient Greco-Roman Insulae Kit (Fab) [search-only]
- Link: https://www.fab.com/listings/b84e1fd4-2dca-4f91-987a-bbb9605eac89 (also sold on CGTrader)
- Price: not captured; licence not captured.
- What it does: modular Roman city environment pack for multi-storey insulae, with stone and brick walls, wooden balconies, tiled roofs, arched windows, shutters, market stalls, awnings and hanging laundry. Facade texture variants. Tint, brightness and contrast controls in the UE and Unity versions.
- Key technique: modular facades with several texture variants to reduce repetition.
- Steal for engine2040: an art source for insulae facades. The variant set can become a facade-id attribute in the city rows.
- Potato note: check triangle and material counts before use. CGTrader says the buildings have empty interiors.

### 29. Fab PCG building, city and road generator (one vendor, three listings) [search-only]
- Link: https://www.fab.com/listings/f205fd21-1428-4a4b-b067-a46d1a2fe86f (one of three listings; name-to-ID mapping unconfirmed; the other IDs are fab.com/s/636c99bf4a7e, fab.com/s/359b0240bbd0 and fab.com/s/112e409832f5)
- Price: not captured. Vendor not named in the search text.
- What it does: Building Generator (PCG-based; American and European presets; editor or runtime generation on sampled points or splines). Road Generator (OSM import, landscape splines, optional PCGEx and Geometry Script, side splines for sidewalks, guardrails and lamps). City Generator combines both and claims a 6,000-building demo. Works with Megascans modular building meshes.
- Key technique: buildings are placed along splines or sampled points, and OSM roads drive the layout.
- Steal for engine2040: the OSM-road to spline to lot to building chain as a model for our city layout step. Drop the Megascans dependency.
- Potato note: thousands of buildings at runtime need instancing and cell streaming. Bake or stream only.

### 30. Houdini Engine for Unreal (SideFX, free licence tiers) [search-only]
- Link: https://www.sidefx.com/unreal/ ; FAQ https://www.sidefx.com/faq/question/houdini-engine-plugin-free-for-unity-and-unreal/
- Price: free tiers. SideFX Unreal page: up to 3 Indie licences for limited commercial use, opening .hda and .hdalc files. SideFX FAQ: up to 10 free licences per studio, .hda only, commercial customers. Apprentice is not supported. A third-party blog says the Indie cap is under USD 100K revenue (unverified). The SideFX pages are dated; confirm the current terms.
- What it does: runs Houdini Digital Assets inside Unreal.
- Key technique: HDAs are cooked inside the engine. PCG interop is claimed only by a third-party blog (StraySpark), not by SideFX or Epic.
- Steal for engine2040: author city layouts as HDAs offline and bake them to our format. Houdini cannot run at engine2040 runtime, so it stays an author-time tool.
- Potato note: author-time only. Never cook HDAs on a potato.

### 31. Unreal MCP servers (AI drives the editor) [search-only]
- Link: https://github.com/chir24/unreal_mcp ; https://glama.ai/mcp/servers/vkpfwqo3eh ; https://enterprisedna.co/directories/mcp/najemwehbe-unreal-ai-connection ; https://mcpservers.org/servers/AlexKissiJr/UnrealMCP
- Price: open source. Enterprise DNA plugin listed as MIT.
- What it does: MCP servers that let an AI control the UE editor: scene edits, asset import, builds, tests.
- Key technique: the engine's Python Editor Script Plugin and Remote Control API, reached over local HTTP (Glama listing: 72+ tools; ChiR24 TypeScript server).
- Steal for engine2040: our editor bridge already does this. Compare tool surfaces (Glama 72+ tools; Enterprise DNA 143 tools for UE 5.7, 4 GitHub stars, so little tested). Copy the Remote Control transport pattern, not the tool list.
- Potato note: none (tooling). The donor chongdashu/unreal-mcp is already seen and was skipped.

### 32. Advanced Locomotion System V4 (ALS, Fab) [search-only] (avoid)
- Link: https://fab.com/s/0eb8afffb774 ; C++ ports https://github.com/PranavSK/ALSV4_CPP and https://github.com/Sixze/ALS-Refactored
- Price: not captured.
- What it does: curve-driven movement and rotation, blended in a data-driven Animation Blueprint. Not a plug-and-play system.
- Key technique: curve-based locomotion blending. Not motion matching. The maintainer of the UE 5.4 fork calls the approach outdated and points to Lyra.
- Steal for engine2040: none beyond the curve-driven blend idea. Use Pose Search instead.
- Potato note: none recommended.

### 33. Lyra-style GAS input sample (intrxx, GitHub) [search-only]
- Link: https://github.com/intrxx/Enhanced-Input-Ability-Activation
- Price: free, open source (licence not checked).
- What it does: unofficial sample of Lyra-style GAS input with Gameplay Tags. Written for UE 5.2.
- Key technique: input actions mapped to ability tags.
- Steal for engine2040: input to ability-tag mapping for a combo buffer.
- Potato note: none.

### 34. Data-driven GAS template (Fab 8f1179bf-ad3d-452c-9026-e32143c6e141) [search-only]
- Link: https://www.fab.com/listings/8f1179bf-ad3d-452c-9026-e32143c6e141
- Price: not captured.
- What it does: GAS template with data assets and Gameplay Tags, inspired by Lyra's architecture.
- Key technique: abilities and effects as data assets.
- Steal for engine2040: a reference for mapping rows to abilities.
- Potato note: none.

### 35. Project ELG (itch.io) [search-only]
- Link: https://drumangel92.itch.io/project-elg
- Price: not captured.
- What it does: extends Lyra as a Game Feature plugin, adding weapons and abilities.
- Key technique: a Game Feature plugin layered on Lyra.
- Steal for engine2040: a worked example of shipping faction content as a Game Feature plugin. Check the licence before taking any code.
- Potato note: none.

---

## Gaps and notes

- No Fab AI, crowd or behaviour plugin with verified reviews was found beyond the items already listed in the plugins files. Mass Battle (Fab) appeared again in search but was skipped as already listed.
- No verified October 2026 Fab free pick. Epic's Fab Limited-Time Free programme (https://www.fab.com/limited-time-free) releases new free items about every two weeks; the latest window found in search ended in early September 2026. Check the page directly before any download, and ask first.
- Control Rig, Replication Graph and the TSR doc itself were not read in full.
- Fab top-seller rankings were not found. The StraySpark roundups are commentary by a publisher and were not used for rankings.

---

## TOP 15 (ranked by impact for a Roman melee + city game on weak PCs)

1. **Hit-window attack data model** (Moveset System pattern, entry 25): attacks as frame rows with hit windows and authority-side hitboxes. Melee is the core loop; this is cheap. [search-only]
2. **Motion Warping for lunges and finishers** (entry 6): one clip covers many distances; root-motion only, no IK cost. [search-only]
3. **Animation Budget Allocator** (entry 1): a global anim-ms budget with a throttle ladder. The single biggest scalability lever for crowds. [verified]
4. **Mass representation LOD** (entry 2): full mesh near, ISM mid, nothing far. The path to thousands of citizens on 2-4 GB VRAM. [verified]
5. **World Partition with HLOD and Data Layers** (entry 11): district cells and offline HLODs for the city. [verified]
6. **Lumen low-end ladder** (entry 12): software RT and SSR-to-probes tiers behind one knob. [verified]
7. **Nanite fallback control** (entry 13): explicit fallback percent for building kit pieces; traditional LODs for props. [verified]
8. **Temporal upscaling / render scale** (entry 23): 1080p render upscaled, with a 57.5 to 33.4 ms GPU frame-time example from the City Sample page. [verified]
9. **Hit reaction lookup by direction and intensity** (entry 26): one table picks the reaction clip. [search-only]
10. **Smart Objects claim and release** (entry 8): jobs and facilities as claimable slots for the city economy. [verified]
11. **Cell-based PCG generation with ISM output** (entry 4): city blocks generated when streamed in, from a seed. [search-only]
12. **Pose Search with three channels** (entry 5): melee locomotion from our clips, with PCAKDTree. [verified]
13. **Lyra-style Game Feature packs** (entry 18): faction packs loaded on demand to fit 8 GB. [search-only]
14. **EOS lobbies and P2P** (entry 24): free co-op join for skirmishes; host-authoritative hits. [search-only]
15. **Mover rollback reference** (entry 19): the resimulation model for melee timing across clients, later stage. [verified]

Sources (verified pages fetched in this pass):
- https://dev.epicgames.com/documentation/unreal-engine/animation-budget-allocator-in-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/overview-of-mass-gameplay-in-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/motion-matching-in-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/smart-objects-in-unreal-engine---overview
- https://dev.epicgames.com/documentation/unreal-engine/world-partition-in-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/nanite-technical-details
- https://dev.epicgames.com/documentation/unreal-engine/city-sample-project-unreal-engine-demonstration
- https://dev.epicgames.com/documentation/unreal-engine/mover-in-unreal-engine
- Other Epic and Fab items are search-only and listed in their entries.
