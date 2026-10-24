# Physics and destruction plugins: steal list (2026-10-24)

Scope: physics, destruction, cloth, rope, ragdoll, vehicle and fluid kits on Unreal, Unity, Godot and standalone/open-source engines. Target: a deterministic Rust engine for a melee/Roman game that must run on a potato PC (iGPU, 8 GB RAM).

How to read the entries:
- **(seen)** = the repo slug already appears in `docs/research/hunt-2026-10-21/donors-seen.txt` (checked by grep, not by reading the list).
- **[F]** = the page was fetched and the claim comes from that page.
- **[S]** = the claim comes only from a search snippet or a secondary source. Re-check before relying on it.
- Prices are as shown in listing snippets on the day of the search. Confirm on the store page before buying.

---

## Unreal Engine

### 1. Chaos Destruction (Geometry Collection) [F]
- Link: https://dev.epicgames.com/documentation/en-us/unreal-engine/destruction-overview
- Engine: Unreal 5. Built into the engine, no separate price.
- What it does: real-time fracture and clustering of pre-fractured or skeletal meshes, built as a Geometry Collection.
- Key technique: at simulation start each fractured body is linked to its nearest neighbours. Each link is a rigid constraint with an initial strain value. A link breaks when a collision or Physics Field impulse exceeds its limit. Clustering treats connected pieces as one rigid body by summing their mass and inertia. Physics Fields lower a link's strain, which weakens a structure on purpose. A Cache System replays heavy destruction at low runtime cost.
- Numbers: none on the page (no strain or threshold values).

### 2. Chaos Flesh [S]
- Link: https://dev.epicgames.com/documentation/unreal-engine/chaos-flesh-overview
- Engine: Unreal 5.5+ (per quickstart). Built in.
- What it does: muscle deformation for characters during skeletal animation.
- Key technique: runs a low-resolution live simulation next to pre-baked high-resolution results. Tetrahedra push back to hold volume so the skin does not collapse.
- Potato verdict: skip. This is a GPU character-deformation system. A forum user reported tetrahedra exploding when only a few muscles were simulated.

### 3. Cable Component (built-in) [S]
- Link: https://dev.epicgames.com/documentation/unreal-engine/cable-components-in-unreal-engine
- Engine: Unreal. Built in, no price.
- What it does: swaying cables, ropes and chains.
- Key technique: Verlet integration. Particles with distance constraints, ends pinned to their attach points, middle particles fall under gravity. Stiffness is set by the number of constraint-solving iterations per step, which is a cheap and easy-to-tune knob. Collision (`bEnableCollision`) is marked experimental.
- Potato note: very cheap. Good fit for banners, chains and rope.

### 4. Physical Animation Component and Physical Animation Profile [S]
- Link: https://docs.unrealengine.com/4.26/API/Runtime/Engine/PhysicsEngine/UPhysicalAnimationComponent/index.html
- Engine: Unreal. Built in.
- What it does: drives a ragdoll toward an animated pose. A profile is applied to one bone and all bones below it.
- Key technique: a strength multiplier scales the force of the active motors. That lets you blend from fully animated (0) to fully physical (1) and back. This is the feel trick for knockdowns and hit reactions.
- Caveat: a forum thread says the individual profile parameters are poorly documented. Tooltips are the only guidance.

### 5. Chaos Vehicles, wheel and suspension [S]
- Link: https://docs.unrealengine.com/4.27/API/Plugins/ChaosVehicles/UChaosVehicleWheel/index.html
- Engine: Unreal. Built in.
- What it does: wheeled-vehicle suspension, ABS, traction.
- Key technique: suspension trace defaults to a ray and can be changed to a shape or sweep. The page gives a rule of thumb of 5 to 10 for the suspension damping ratio on standard cars. `EnableSuspension` can bypass suspension forces entirely.
- Relevance: low for a melee game. Listed for completeness.

### 6. Niagara Fluids (built-in) [S]
- Link: https://dev.epicgames.com/documentation/unreal-engine/niagara-fluids-in-unreal-engine
- Engine: Unreal. Built in.
- What it does: real-time fire, smoke, gas and liquid templates.
- Key technique: 2D templates (2D Gas, 2D Liquid, Shallow Water) are cheaper and suit games. 3D templates look better but cost much more memory and GPU time, so keep them for hero shots. Simulations can be baked to flipbooks for materials, which is the potato-friendly path.
- Caveat: Epic's docs warn that fluid scenes are GPU-heavy and can crash the GPU on Windows.

### 7. FluidNinja LIVE-2 (Fab) [S]
- Link: https://www.cgchannel.com/?p=178469 (news item; the Fab listing was not fetched)
- Engine: Unreal 4.26 to 5.8 (per listing). Price unconfirmed; one report says free.
- What it does: a Blueprint project that wraps Niagara into responsive character and environment effects.
- Key technique: VFX wiring, not physics. Not a steal for the physics engine. Listed for FX only.

Not found: a Fab "Dynamic Combat" ragdoll or hit-reaction plugin. Searches returned only an unrelated animation tool (DynamicNode). Dropped.

---

## Unity

### 8. RayFire for Unity [S]
- Link: https://assetstore.unity.com/packages/tools/game-toolkits/rayfire-for-unity-obsolete-148690
- Engine: Unity 2018.1+. Price: $170 at the 2019 launch (current price not found, so treat as stale). The store slug says "obsolete" and the status is unclear. Reviews mention recent updates.
- What it does: runtime demolition, pre-shattering in edit mode.
- Key technique: fragmentation modes (regular Voronoi, wood splinter, slabs, radial, plane slicing, tetrahedral). Clustering that can simulate and break concave objects. Advanced simulation control, such as activating fragments at a set time. Fragments can be pushed by shots, explosions, wind and vortex forces.
- Numbers: none on the page apart from the launch price.

### 9. DinoFracture [S]
- Link: https://assetstore.unity.com/packages/slug/26599
- Engine: Unity (Built-in, URP, HDRP; WebGL supported). $40 per seat. Latest version 2.11.4, released Jul 20 2026.
- What it does: runtime and editor mesh fracture.
- Key technique: joints are transferred automatically to fractured pieces, so connected objects keep their links. Topology errors are flagged and most can be fixed during fracture. Helper scripts start fracture on collision, play sounds and notify other objects.
- Caveat: one reviewer (anecdotal) says it handled skinned-mesh destruction that RayFire did not.

### 10. Obi Rope [S]
- Link: https://marketplace.unity.com/packages/tools/physics/obi-rope-55579
- Price $37. Latest version 7.1.1, released Apr 23 2026.
- What it does: spline-driven ropes.
- Key technique: XPBD particles with oriented particles on top, which give twist and torsion. Rope mesh is generated along the curve.

### 11. Obi Softbody [S]
- Link: https://assetstore.unity.com/packages/tools/physics/obi-softbody-130029
- Price $55 (from the reviews page; the main listing snippet did not show it). Version unclear.
- Shared Obi technique: XPBD, with the developer saying the solver uses ellipsoidal particles. Runs on CPU or GPU (GPU needs compute shaders). Tearable cloth, two-way rigid body interaction, cloth self-collision. A reviewer said self-collision was weak in an older version.

### 12. Obi Fluid [S]
- Link: https://assetstore.unity.com/packages/tools/physics/obi-fluid-63067
- Price $60. Latest version 7.1.1, released Apr 22 2026.
- Key technique: the simulation outputs a fluid surface mesh, so any shader can render it. Only the mesh has to be drawn.

### 13. Ultimate Rope Editor [S]
- Link: https://marketplace.unity.com/packages/tools/physics/ultimate-rope-editor-7279
- Price $45. Latest version 1.40, released Mar 22 2024. Forum thread dates to 2013, so it has a long maintenance history.
- What it does: procedural ropes, and rope physics on existing meshes.
- Key technique: ropes can snap under load, rope length can change at runtime, and parameters can be tuned in play mode with the state saved back to edit mode.

### 14. Magica Cloth and Magica Cloth 2 [S]
- Link: https://discussions.unity.com/t/released-magica-cloth/773425 (release thread; Asset Store price not verified)
- Original: Job System and Burst, bone-driven and mesh-driven cloth, does not run on WebGL.
- Magica Cloth 2: DOTS-based. Adds edge collision, backstop and self-collision, plus a collider wizard for humanoids. Price found only on resellers (Gumroad, Booth), so confirm the official listing.
- Key technique: bone-driven cloth on the job system and Burst is the cheap path that scales to many characters.

### 15. PuppetMaster with Final IK compatibility [S]
- Link: https://assetstore.unity.com/packages/tools/physics/puppetmaster-48977
- Price not verified (a reviewer says it is not free).
- What it does: active ragdoll. Reviewers mention partial unpinning, hit reactions, falls and getups, and baking ragdoll motion offline.
- Caveat: setup is hard, and a reviewer says the ragdoll can fumble when it touches a wall.

### 16. Edy's Vehicle Physics [S]
- Link: https://assetstore.unity.com/packages/tools/physics/edy-s-vehicle-physics-403
- Price $60 (current listing, from the reviews page snippet).
- What it does: vehicle physics on top of Unity's WheelCollider.
- Key technique: edits the wheel friction curves every frame to work around a known WheelCollider traction bug. The author's old README claims dozens of vehicles at low CPU cost. Those numbers are old and unverified.
- Relevance: low for melee. Note the bundled-model licensing caveat in the old README.

Dropped: Exploder. The only listing found was an old forum post with a stale price, and no current store page confirmed the fracture method.

---

## Godot

### 17. godot-jolt (Jolt for Godot 3D) [F for the docs, S for the repo]
- Link: https://godotengine.org/asset-library/asset/1918 (repo: https://github.com/godot-jolt/godot-jolt)
- Engine: Godot 4.3 and 4.4 for version 0.15.0 (per listing). Free. License not verified.
- What it does: drop-in Jolt backend for Godot 3D physics.
- Key technique and status: the official Godot docs say Jolt became the built-in 3D option in Godot 4.4 and that the extension is now in maintenance mode. The docs say the only missing feature is related joints. The docs also say soft bodies are more robust in Jolt than in GodotPhysics3D.
- Use: on new Godot 4.4+ projects, use the built-in Jolt path instead.

### 18. Verlet Rope 4 [S]
- Link: https://store.godotengine.org/asset/tshmofen/verlet-rope-4/ (repo: https://github.com/Tshmofen/verlet-rope-4)
- Engine: Godot 4.4.1+ with .NET, plus the .NET 8 SDK. C# only. MIT per search snippet (not verified on the repo page).
- What it does: Verlet ropes in the Source-engine style. `VerletRopeSimulated` is a visual rope that reacts to wind, gravity and collisions. `VerletRopeRigidBody` pushes other bodies.
- Key technique: Catmull-Rom spline tessellation of the rope mesh, with joints for attaching to bodies.

### 19. Jummit godot-destruction-plugin [S]
- Link: https://github.com/Jummit/godot-destruction-plugin
- Engine: Godot 4.2 target. MIT (per listing snippet).
- What it does: a Destruction node that swaps an object for a pre-shattered version.
- Key technique: swap-to-prefractured. Simple and cheap, but there is no real-time fracture or stress model.

### 20. Destructibles CSharp [S]
- Link: https://github.com/ZachAR3/Destructibles-CSharp
- Engine: Godot. CC0. Check the current version against the repo, since the Asset Library entries list different Godot versions.
- What it does: turns a shard mesh into mesh instances with generated collision shapes, one rigid body per shard. Built around Blender's cell-fracture workflow.

### 21. ShatterKit (itch.io) [S, vendor claims unverified]
- Link: https://indiedevwilliam.itch.io/shatterkit
- Engine: Godot 4. Pure GDScript, no dependencies, runtime generation.
- Vendor claims: seeded Voronoi fracture, progressive damage, structural collapse, and an effects layer. Not verified. Price not checked.

---

## Standalone and open source

### 22. Jolt Physics (seen) [F]
- Link: https://github.com/jrouwe/JoltPhysics
- License: MIT. Engine: C++ (used by Godot's built-in option and by Rust wrappers).
- What it does: general 3D rigid-body physics.
- Key features from the repo page: soft bodies (cloth, soft ball) with collision against rigid bodies. Ragdolls keyed three ways: hard keying with kinematic bodies, soft keying by setting velocities, and driving constraint motors toward an animated pose. Capsule character controllers in two forms: rigid-body (accurate with dynamic bodies) and virtual (cheaper, less accurate). Wheeled, tracked and motorcycle vehicles. Simulation runs deterministically when inputs are replicated. Multi-core friendly. CCD.
- Not on the page: sub-stepping, sleeping details and numbers.

### 23. Box2D v3, Soft Step solver (seen) [F]
- Link: https://box2d.org/posts/2024/08/releasing-box2d-3.0
- License: MIT per the project (not in the fetched text).
- Key technique: Soft Step (formerly TGS_Soft) uses soft constraints and sub-stepping, and came from the Solver2D experiment. The author says it is more stable than v2.4 in almost every way: higher mass ratios, longer body chains and larger stacks. It is slightly weaker than v2.4's NGS block solver for vertical stacks.
- Numbers: "more than twice as fast as v2.4" (author's tests). No substep count, relax-iteration count or contact frequency is given in the post.

### 24. Box3D (seen) [S]
- Link: https://blog.desdelinux.net/en/box3d-launch-motor-physics-erin-catto-box2d/
- Erin Catto's 3D successor to Box2D. The search snippet lists a "Robust Soft Step" rigid-body solver among its features (taken from the secondary blog; check the repo).

### 25. Teardown custom voxel physics engine [F for the 80.lv article; S for the rest]
- Link: https://80.lv/articles/see-what-s-new-in-teardown-creator-s-custom-voxel-physics-engine/
- Second source: https://www.pcgameshardware.de/Teardown-Spiel-73677/Specials/PC-Game-Test-Benchmark-Voxel-Dynamic-Destruction-1394545/2/ (German, [S])
- Key techniques:
  - Sub-stepping instead of more solver iterations. The article calls this "Temporal Gauss-Seidel". The author cites Box2D and Box3D as the basis.
  - A parallel solver spreads heavy piles across threads. Contact creation and broad-phase were also upgraded.
  - Physics acceleration grids are derived from the render voxels at runtime, so they need no separate storage. Save files use a simple run-length encoding. The PCGH article says big levels hold about half a billion volume pixels, which would otherwise mean more than 0.5 GB of save data.
  - Structural integrity prototype with a per-voxel integrity value. Damage recalculates neighbouring integrity and checks for failure from stress. The per-voxel method is from a community forum thread, not a Teardown post: https://discussions.unity.com/t/want-to-recreate-teardown-destructable-voxel-physics/828648 [S].
- Numbers: one simulation step is about 5 ms on an i9 with an RTX 4080 Ti using 32 threads (article test rig). Not a potato number.

### 26. NVIDIA Blast (not seen) [F]
- Link: https://github.com/NVIDIAGameWorks/Blast
- Engine: C++ SDK, replaces APEX Destruction. License not shown on the repo page (a PACKAGE-LICENSES folder exists). Repo snapshot: 457 stars, 87 forks.
- Key technique: a support graph keeps fractured pieces linked as actors. One asset can hold several chunk hierarchies and mix chunks from different fracture depths, so you can choose a depth per LOD. Damage behaviour comes from user-supplied shader functions. The low-level API has no physics or graphics of its own, so the host engine supplies those. Designed for pre-fractured rigid bodies; runtime fracture is only mentioned as a possible future.
- Community port: https://github.com/ReV3nus/Unity-Blast [S].

### 27. NVIDIA PhysX 5 (Omniverse repo) [F]
- Link: https://github.com/NVIDIA-Omniverse/PhysX
- License: BSD-3-Clause.
- What the page shows: release mapping (Omniverse PhysX 110.1 pairs with PhysX 5.9.0). The page does not describe TGS, cloth, ragdolls, GPU rigid bodies or determinism, so no feature claims here. Destruction is handled by the separate Blast SDK.

### 28. Rapier (seen) [F]
- Link: https://github.com/dimforge/rapier
- License: Apache-2.0. Language: Rust, with C/C++ bindings for 2D/3D, f32/f64 and soft bodies.
- Key features from the page: SIMD-batched constraint solving and contact processing, always on, with scalar fallback. Optional `parallel` feature multithreads the step through rayon. `set_dedicated_thread_pool` can restrict the step to performance cores on hybrid CPUs. The page does not list CCD, sleeping or determinism, so those are not claimed here.

### 29. MuJoCo soft constraints (seen) [F]
- Link: https://mujoco.readthedocs.io/en/stable/modeling.html
- Key technique: constraints are soft. `solref` default is `0.02 1` (timeconst, dampratio). Timeconst should be at least 2x the simulation timestep or the system can go unstable. `solimp` default is `0.9 0.95 0.001 0.5 2`, which sets an impedance curve over constraint violation. Impedance is clamped to [0.0001, 0.9999].
- Use: a tuning reference for soft joint limits and ragdoll motors. Not a plugin.

### 30. XPBD and unified particles (papers) [S]
- XPBD: Macklin, Müller and Chentanez, "XPBD: position-based simulation of compliant constrained dynamics", MIG 2016. Compliance-based constraints that are stable as iteration count changes. Link: https://mmacklin.com/xpbd.pdf (the PDF downloaded but could not be parsed in this session, so the formulas are not verified here).
- Unified particles: Macklin, Müller, Chentanez and Kim, "Unified Particle Physics for Real-Time Applications", SIGGRAPH 2014 / ACM TOG 33(4). Treats gases, liquids, deformable solids, rigid bodies and cloth as particles with constraints. Link: https://history.siggraph.org/?p=103633 [S].
- Original PBD: Müller et al., 2007, Journal of Visual Communication and Image Representation (no link checked).

### 31. Houdini Bullet glue network (offline) [S]
- Link: https://sidefx.com/forum/post/126993 (forum thread, Houdini 12 era, node names may be outdated)
- Workflow: Voronoi fracture, then glue constraints built with a Glue Adjacent step. The constraint network is cached and used in DOPs through a SOP Solver. Pinning the base and animating a top pin carries the whole glued network. Offline only; use as design reference.

---

## Destruction case studies

### 32. Red Faction: Guerrilla, Geo-Mod 2.0 (Volition, 2009) [S]
- Links: https://www.redfactionwiki.com/wiki/GeoMod_2.0 ; https://gamesbeat.com/red-faction-guerrilla-review/ ; https://www.pcworld.com/article/528435/red_faction_guerrilla.html ; https://multiplayer.it/articoli/gdc-2009-red-faction-guerrilla-anteprima.html (Italian)
- Key technique: man-made structures use pre-broken meshes that react to stress and impacts, rather than real-time terrain carving. Each building runs a from-scratch stress system, and removing supports on several sides brings the structure down (2009 review). Debris was kept chunky by hardware limits. Poles and mesh links were indestructible in the preview.
- Numbers: one secondary (Italian) source says about 10,000 interactive objects, with similar structures sharing base data until destroyed. The GDC 2009 talk was not found, so treat the 10,000 figure as unverified.

### 33. The Finals, "Engineering Mayhem" (Embark, GDC 2024) [S]
- Links: https://schedule.gdconf.com/session/engineering-mayhem-technical-deep-dive-into-environmental-destruction-in-the-finals/900179 ; https://kitguru.net/gaming/joao-silva/embark-studios-reveals-major-destruction-system-update-for-the-finals ; https://games.gg/news/the-finals-new-chapter-october-20/
- Key technique (talk topics per schedule): connection graph construction, managing the physics state of simulated debris, replication and simulation performance. The destruction is server-side. Two layers: one for structural integrity of map elements, one for debris (KitGuru).
- Newer update (games.gg, undated, likely 2026): a gradual layer makes floors sag and walls lean before they collapse, so buildings degrade in stages instead of exploding into rubble.
- The GDC Vault recording was members-only in the search, so the talk itself was not read.

### 34. Support-graph collapse (weak sources) [S]
- Links: https://www.curseforge.com/hytale/mods/rubbleworks/files/7802504 ; https://www.mado-hosting.com/satisfactory-mods/GravityCheck
- Technique: BFS from grounded anchors finds unsupported components. Unsafe pieces fall after a timer, and anything resting on them falls too. A stress-based mode is the expensive version.
- Caveat: both are mods on non-standard hosts. Use only as a pattern. The Chaos connection graph (item 1) is the better-sourced version.

---

## Dropped (could not verify)

- Instruments of Destruction: a Radiangames vehicle game. No talk or technique source found.
- Dynamic Combat Fab plugin: not found.
- Exploder (Unity): no current listing found.
- Teardown dev blog posts: not reached. Voxel details come from secondary articles.
- Blast license: not shown on the page.
- Havok, NVIDIA Flex, Unity Cloth deprecation: not searched in this pass.

---

## TOP 12 steal list (feel impact for melee/Roman, potato-first)

Ranked by how much it changes the feel of hits and collapses, with cost on a 2-4 GB VRAM / 8 GB RAM machine in mind.

1. **Connection (support) graph with per-link strain** (Chaos Destruction; Blast support graph; The Finals). Structures hold until the right hit, then fail in a readable chain. Build the graph once per asset, and only walk it on break events. Source: items 1, 26, 33.
2. **Impulse-threshold breaking** (Chaos, collision and field impulses). A blade strike and a shove break different things. Physics Fields or "weak point" zones lower the strain threshold. Source: item 1.
3. **Clustered rigid bodies until broken, mass and inertia summed** (Chaos). Pieces stay one body until they fail, which keeps pre-break cost low. Source: item 1.
4. **Two-layer model: structure graph plus a separate debris layer** (The Finals). Debris gets its own cheap state, which is where you cap counts and sleep. The talk lists debris physics-state management as a topic, but the sleeping details are not verified. Source: item 33.
5. **Staged degradation before collapse** (The Finals update). Floors sag and walls lean before they fall. It is a state machine on top of the graph, so the cost is low and the read is much better. Source: item 33.
6. **Multiple fracture depths in one asset, chosen per LOD** (Blast chunk hierarchies). On a potato, pick a coarse depth at distance and a fine depth near the camera. Source: item 26.
7. **Sub-stepped soft constraints over iteration count** (Box2D v3 Soft Step; Teardown sub-stepping). Stacks and ragdolls stay stable with fewer solver passes. Source: items 23, 25. Tune constraint stiffness with the MuJoCo timeconst rule (at least 2x the timestep). Source: item 29.
8. **Verlet cables with iteration-count stiffness** (UE Cable Component). Banners, chains and rope for almost no cost, and easy to keep deterministic. Source: item 3.
9. **XPBD compliance for cloth and tearable sacks** (Obi; the XPBD paper). Use it only for hero cloth, because cloth particle count is the potato budget. Source: items 11, 30.
10. **Physical-animation strength blend 0 to 1** (UE Physical Animation Profile). Blend ragdoll motors in and out for knockdowns and hit reactions. Source: item 4.
11. **Pre-fracture with transferred joints and topology checks** (DinoFracture). Authoring pipeline for walls, palisades, shields and pottery. The joint transfer is the part worth copying. Source: item 9.
12. **Dust and splash as 2D sims baked to flipbooks** (Niagara Fluids). Cheap FX on weak GPUs; avoid 3D. Source: item 6.

Honourable mentions: Red Faction's shared base data for thousands of pieces until destroyed (item 32, secondary source). Teardown's run-length save and runtime-derived acceleration grids (item 25), useful if save size matters.

---

## Notes

- No file other than this one was touched. No git operations.
- Search depth: about 30 searches and 15 fetches. Several primary pages (Epic Chaos docs, The Finals GDC Vault, Chaos Flesh, the EG 2011 real-time fracture paper and the XPBD PDF) were either gated, returned 403, or yielded no text. Those claims are marked [S].
