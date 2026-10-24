# Grapple / hookshot / rope / pull-enemy: plugins, addons, assets (2026-10-24)

Scope: real, linked grapple, hookshot, rope swing, tether, cable and pull-enemy packages for Unreal, Unity, Godot, Flax, Bevy/Rust, Roblox, plus open-source grapple code on GitHub. Every entry below was checked one of three ways, and the "Checked" column says which:

- **fetched**: the page or source file loaded and was read for this write-up.
- **store page**: the store page loaded, but it had no feature text, so only price/metadata are confirmed.
- **search excerpt**: the page returned 403 or 404 to our fetch, so the description comes from the search index only. Treat these as unverified.

Numbers are only listed where a page or source file actually showed them. Where a number is missing, the page did not show it. Nothing here was run or tested.

---

## Entries

### Godot

| # | Name | Link | Checked | License / version | What it does | Feel details (as shown) |
|---|---|---|---|---|---|---|
| G1 | **DestroHook** (OverlordDestro) | https://github.com/OverlordDestro/DestroHook | fetched (`hook_controller.gd`) | MIT, Godot 4.3, 3D | Spring-based 3D grapple add-on. Files: `scripts/hook_controller.gd`, `hook_model.gd`, `player.gd`, `moving_box.gd`, `rope_swish.gdshader`. | See "Feel numbers: DestroHook" below. This is the richest verified source of tuning numbers in this list. |
| G2 | **Grappling Hook** (Asago, 2D) | https://asago.itch.io/grappling-hook | fetched | Godot 4.7+, 2D only, paid ($4.99 list, $1.99+ shown, bundle $30) | Five modes: Swing (keeps momentum), Pull Player (hookshot accelerates character to anchor), Pull Object (pulls `RigidBody2D`, anchor follows object), Fixed Rope (strict tether), Elastic Rope (spring with configurable strength and damping). | No numbers on page. The mode split is the useful part. |
| G3 | **Verlet Rope 4** (Tshmofen, fork of 2nafish117/godot-verlet-rope) | https://github.com/Tshmofen/verlet-rope-4 | fetched | MIT, Godot 4.4 .NET | Rope physics, not a grapple. `VerletRopeSimulated` (reacts to wind, gravity, collisions) and `VerletRopeRigidBody` (rope of rigid bodies that can push things). `DistanceForceJoint` is a distance joint that works without a rope. | No default values on page (points to wiki). |
| G4 | **vlabo/Godot_rope_addon** | https://github.com/vlabo/Godot_rope_addon | fetched | MIT, archived Dec 2020, Godot 2 only | Rope made from rigid bodies between curve points with pin joints. | Stale. Listed only as a reference. Do not port the code. |

### Unity

| # | Name | Link | Checked | Publisher / price | What it does | Feel details (as shown) |
|---|---|---|---|---|---|---|
| U1 | **Obi Rope** | https://assetstore.unity.com/packages/tools/physics/obi-rope-55579 | store page | Virtual Method, $37, 151 reviews, v7.2 | Particle rope on the Obi solver (XPBD-style, oriented particles for twist). Spline editing. | No feel numbers on the store page. Reviewer report: rope disappears/pink on Quest unless materials match the render pipeline. |
| U2 | **Grappling & Rope Swing PRO** | https://assetstore.unity.com/packages/slug/183951 | store page + search excerpt | Groovy Games, $15, 3 ratings | Grappling system with first- and third-person prefabs, hook model, sample scene. Latch to object, reel object in, or reel self in. | Feature text not on page. Excerpt only: variables editable in inspector. |
| U3 | **Grapple Hook 2D** | https://assetstore-fallback.unity.com/packages/tools/physics/grapple-hook-2d-169421 | store page + search excerpt | João Ramiro, $7.99 | Click-to-throw hook, throw and rappel modes, can latch moving objects. | Feature text not on page. |
| U4 | **3D Wrapping Grapple Hook** | https://assetstore-fallback.unity.com/packages/tools/physics/3d-wrapping-grapple-hook-188786 | store page + search excerpt | A Trillion Games, $4.99, v2.0 (Jan 2025), Built-in/URP/HDRP | Raycast-based grapple line that wraps around obstacles. Excerpt says this avoids a full rope sim. | Description empty on page. Keywords: joint, rope swing, web shooting. |
| U5 | **Advanced Grappling System** | https://assetstore.unity.com/packages/templates/systems/advanced-grappling-system-199785 | store page + search excerpt | Alpha00, $9.99, v1.4 (May 2025), 482.8 MB | Grappling gun with rope animation and aim assist (excerpt). | Description empty on page. Aim assist is from the excerpt only. |
| U6 | **Rope Toolkit** (Gustav Olsson AB) | https://gustavolsson.itch.io/rope-toolkit | fetched | v2.2.0, requires Burst 1.1.2+, full source. Store status: "currently unavailable", demo zip available. | Job/Burst-based rope with four connection types: Pin Rope To Transform, Pin Transform To Rope, **Pull Rigidbody To Rope**, Two Way Coupling. `SplitAt()` splits ropes at runtime. | Perf (example scene): with collisions ~0.2 ms in jobs and ~0.7 ms main thread; without collisions ~0.15 ms jobs and ~0.35 ms main. Length multiplier example 0.9. Collisions only on convex Mesh/Box/Sphere/Capsule. Grapple scene not included; a commenter built one. |
| U7 | **UnityVerletRope** (alirezaft) | https://github.com/alirezaft/UnityVerletRope | fetched | No license stated, Unity 2022.3.16 | Verlet rope: particles with distance constraints, collisions, procedural cylinder mesh each step. | Iteration count controls stiffness (no value given). No numbers on page. |
| U8 | **Grab'n Speed / GrabenSpeed** (ErwanHAMIDA) | https://github.com/ErwanHAMIDA/GrabenSpeed | fetched (repo root) + search excerpt | No license, Unity (version not stated) | 2D platformer prototype with grappling hook; you can shoot enemies (excerpt). | Repo page has no description and no script list. No numbers. |
| U9 | **Make a 2D Grappling Hook Game in Unity, Part 1** (Sean Duffy, Kodeco) | https://www.kodeco.com/348-make-a-2d-grappling-hook-game-in-unity-part-1 | fetched | Tutorial with starter and finished projects (free account), Unity 2017.1+ | Aim, line renderer, `DistanceJoint2D` rope, wrap around objects, swing. Part 2 unwraps the rope past pivots. | See "Feel numbers: Kodeco" below. |
| U10 | **SpiderMan Web Swing 2D** (High Level, itch) + repo `GabrielGameDev/SpiderMan2DWebSwing` | https://high-level.itch.io/spiderman-web-swing-2d and https://github.com/GabrielGameDev/SpiderMan2DWebSwing | fetched (itch + repo landing) | Free, Unity (per page), no license shown on repo | Web swing prototype: LMB launches web, RMB releases, goal is distance. | No numbers. Repo README body and scripts not readable from the landing page. |

### Unreal

| # | Name | Link | Checked | Price / version | What it does | Feel details (as shown) |
|---|---|---|---|---|---|---|
| R1 | **Grapple Hook System / Surface Hook Traversal Kit** (Fab) | https://www.fab.com/listings/d50097ec-9c33-44f1-bce6-6ac039ae6d7f | search excerpt (Fab returned 403 to fetch) | Not shown | Blueprint-only first-person traversal kit: target detection, visible cable, pull player to target, crosshair changes when an anchor is in range, demo level. | Crosshair color change on valid anchor is from the excerpt. No numbers. |
| R2 | **Physics-Based Grappling Hook System** (Fab) | https://www.fab.com/listings/12ae72f7-d885-4227-b1ad-45d26d3e4202 | search excerpt (403 to fetch) | Engine not named in excerpt | Physics-driven grapple that simulates rope tension and momentum. | Excerpt only. |
| R3 | **Dynamic Rope** (Fab, Keno team) | https://www.fab.com/listings/17f62a8f-387b-48b0-9198-af7273dc3733 | search excerpt (403 to fetch); docs linked at teamkeno.github.io/DynamicRopeDocs (not fetched) | UE 5.5–5.8, Win64, C++ project | GPU XPBD rope with a CPU fallback. Ropes wrap skeletal bones and ragdolls via Chaos constraints. Full C++ source, SDF authoring tool. | Excerpt: no network replication, dedicated servers fall back to CPU, one SDF asset is needed per skeletal mesh for per-bone collision. Lists lasso, whip, tow cable, vine use cases. Does not name grapple hooks. |
| R4 | **Grapple Hook + Pull (Multiplayer)** (kirosho, Gumroad) | https://kirosho.gumroad.com/l/grapplehookpro | store page + search excerpt | About $20 (excerpt); engine version not shown | Blueprint kit. Pulls replicated objectives or players, chosen by a configurable tag. Third person and first person. | No feel numbers on page. |
| R5 | **Grappling Hook tutorial** (Unreal Community Wiki) | https://unrealcommunity.wiki/grappling-hook-tutorial-g05n71p8 | fetched | Free, First Person Blueprint template | Blueprint grapple on H key. Trace 5000 units from camera; if the hit is a Character, pull the pawn to the player; else launch the player to the point. Two looping timers gate the pulls. | See "Feel numbers: UC wiki" below. |
| R6 | **Grapple-Game** (giodestone) | https://github.com/giodestone/Grapple-Game | fetched (README via raw) | UE4 C++ university coursework, no license shown | Surface grapple (interpolated move, cable shown), whip, turrets. Cancel or hit stops the move and keeps momentum. Actors tagged `NoGrapple` or `Whipable` are excluded. | No numbers in README. Its enemy-pull is not built in; you would add it. |
| R7 | **CableComponent** (built-in, Epic) | https://docs.unrealengine.com/4.27/en-US/Basics/Components/CableComponent | search excerpt (docs returned 403 to fetch) | Built into the engine | The rope primitive a grapple uses. Properties: `CableLength` (rest length), `NumSegments`, `SolverIterations` (constraint passes per step; more = stiffer). Enable Collision is experimental and costly. | No default values in excerpt. |

### Rust / Bevy

| # | Name | Link | Checked | License / version | What it does | Feel details |
|---|---|---|---|---|---|---|
| B1 | **avian_pickup** | https://docs.rs/crate/avian_pickup/0.6.0 | fetched | License not shown, v0.6.0, depends on avian3d | Gravity-gun style pickup of dynamic rigid bodies for Avian on Bevy. Modeled after HL2 gravity gun. | Not a grapple. Useful as a "pull a body to the hand" reference. Pull force, hold distance, damping not on page. |

No Bevy grapple crate was found. Rapier and Avian are physics engines that a grapple would be built on, not grapples.

### Flax, Roblox

- **Flax:** no grapple or rope plugin found. Only the FlaxAPI repo (https://portrait.gitee.com/long33/FlaxAPI) came up, and it does not mention ropes.
- **Roblox:** no verified free grapple kit. A BuiltByBit "Grapple System v1.0" ($5.99) appeared in search but its page returned 403, so it is dropped. "Grapple Climb" is a game, not a kit, and is dropped.

---

## Feel numbers (verified in source or page)

### Feel numbers: DestroHook (G1), from `hook_controller.gd`

- `stiffness = 10.0`, `damping = 1.0`, `thrust_mult = 1.0`.
- `rest_length_center_fraction = 3` (int, range 1 to 10). At launch, `rest_length_center = distance / 3` and `rest_length = distance - rest_length_center`. So the rope starts at 2/3 of the hook distance.
- Shorten and extend change `rest_length` by 1 per key press. Shorten and extend are bound to the action names `MWD` and `MWU` (presumably mouse wheel down and up; the mapping is set in the project input map, not the script). Launch and retract are both bound to LMB. Holding jump (`action_jump`) adds a boost toward the hook scaled by `thrust_mult`.
- Per physics frame: spring force = `stiffness * (distance - rest_length)`, **doubled when stretched and zeroed when slack**. So the rope only pulls, never pushes, and pulls harder the more it is stretched. A damping term acts along the rope.
- Not stated in source: max length, raycast range (set in the RayCast3D node), pull or retract speed, lerp or tween speeds, FOV or camera code, hit pause.

### Feel numbers: Kodeco 2D (U9)

- `ropeMaxCastDistance = 20f`: raycast length cap.
- On attach: `Vector2.Distance(playerPosition, hit.point)` sets the joint distance. The rope length is fixed at the attach distance.
- Hop impulse on attach: `new Vector2(0f, 2f)`, `ForceMode2D.Impulse`.
- Joint disabled in `Awake()`, enabled only on a valid hit. Line renderer enabled on click, disabled on miss, reset to `positionCount = 2`.
- Crosshair orbits the player at radius `1f`. Aim angle from `Mathf.Atan2`, from `ScreenToWorldPoint`.
- Frequency, damping ratio, swing force, line width: not stated.

### Feel numbers: UC wiki (R5)

- Trace: 5000 units from camera, object types Pawn, WorldStatic, WorldDynamic.
- Pull-to-location launch: `Launch Velocity = player location - hit location`, vector length clamped to at least `125`.
- Pawn pull gate closes when the distance drops below a threshold of `125` (the page's formula is `2*Capsule Radius + X`, values not given).
- Pull speed, interp speed, cooldowns, FOV: not stated.

### Feel numbers: Rope Toolkit (U6)

- Perf: collisions on, about 0.2 ms in jobs and 0.7 ms on the main thread. Collisions off, about 0.15 ms and 0.35 ms.
- Length multiplier example: 0.9 (rope is 90% of original length).

---

## Gaps

Searched for these and did not find a verified source with the numbers asked for:

- **Camera FOV kick, hit pause, wind-up** appear in none of the verified sources. Nothing to cite.
- **Aim assist / snap radius**: only Advanced Grappling (U5) and Fab R1 (crosshair color) claim it, both from excerpts.
- **Swing momentum boost**: Asago (G2) says swing preserves momentum, Grapple-Game (R6) says the player keeps momentum on cancel. No numbers either way.
- **Dani's grapple code (Karlson)**: the GitHub repo it was linked from returned 404. Dropped.
- **Brackeys grapple tutorial**: not found in search. Dropped.
- **Sebastian Lague grapple**: not searched for, not included.
- **UE GAS grapple examples**: searched once. Only resource lists and an itch prototype, no GAS grapple repo verified. Dropped.
- **Cuberact Godot library, Heathen Verlet Tools, GSharpRope (Godot 3.2)**: seen in search, not fetched or stale. Dropped.
- **Ultimate Grappling Hook (trevor-takahashi, itch)**: itch page returned 404. Dropped.
- **AFPC + Grapple Hook System (nostalgic-games-tm)**: search only, not verified. Dropped.
- **FS Rope Swinging System ($19)** and **Dual Hookshot System ($2.50)**: search only. Dropped.

---

## Steal list (ranked by how much each would improve grapple feel)

1. **Asymmetric spring rope (DestroHook G1).** Force = `stiffness * (d - rest)`, doubled when stretched, zero when slack. Only pulls, never pushes. Stiffness 10, damping 1 as a starting point. Gives taut-but-swingy feel without a full rope sim.
2. **Rest length set to 2/3 of the hook distance at launch, then shorten/extend in steps of 1 per tick (DestroHook G1).** The rope starts slack-ish so the first swing does not snap.
3. **Minimum launch magnitude (UC wiki R5).** Clamp the pull velocity to a minimum length (125 units in the source) so short hooks still give a visible yank.
4. **Hop impulse on latch (Kodeco U9).** Small upward `(0, 2)` impulse the moment the hook attaches. It makes the attach read as a physical event.
5. **Keep momentum on cancel or interrupt (Grapple-Game R6, Asago G2 swing mode).** Cancelling a grapple should release you with velocity, not stop you dead.
6. **Branch the pull on the hit type (UC wiki R5, Grapple-Game R6 tags).** Hit a character: pull them to you. Hit geometry: pull yourself. Use tags such as `NoGrapple` to block targets.
7. **Two pull modes: pull-player and pull-object (Asago G2, Rope Toolkit U6).** Pull a rigidbody to you, with the anchor following the object, rather than only yanking the player.
8. **Crosshair change when an anchor is in range (Fab R1, excerpt).** Clear feedback for a valid target. Verify the implementation before copying.
9. **Rope visual lifecycle (Kodeco U9).** Cable is hidden until fire, two-point line until attach, reset on miss. Keep the rope visual tied to the joint state, not always on.
10. **Keep rope cheap (Rope Toolkit U6, Cable Component R7).** Budget about 0.2 to 0.7 ms per scene with collisions (Rope Toolkit). Expose `NumSegments` and `SolverIterations` as the knobs for stiffness and cost, as in CableComponent.

---

## Sources

Verified (fetched):
- https://github.com/OverlordDestro/DestroHook (and `addons/destrohook/scripts/hook_controller.gd`)
- https://asago.itch.io/grappling-hook
- https://github.com/Tshmofen/verlet-rope-4
- https://github.com/vlabo/Godot_rope_addon
- https://gustavolsson.itch.io/rope-toolkit
- https://github.com/alirezaft/UnityVerletRope
- https://github.com/ErwanHAMIDA/GrabenSpeed
- https://www.kodeco.com/348-make-a-2d-grappling-hook-game-in-unity-part-1
- https://high-level.itch.io/spiderman-web-swing-2d
- https://github.com/GabrielGameDev/SpiderMan2DWebSwing
- https://unrealcommunity.wiki/grappling-hook-tutorial-g05n71p8
- https://github.com/giodestone/Grapple-Game
- https://docs.rs/crate/avian_pickup/0.6.0
- https://kirosho.gumroad.com/l/grapplehookpro (title and metadata only)
- https://assetstore.unity.com/packages/tools/physics/obi-rope-55579 (metadata only)
- https://assetstore.unity.com/packages/slug/183951 (metadata only)
- https://assetstore-fallback.unity.com/packages/tools/physics/grapple-hook-2d-169421 (metadata only)
- https://assetstore-fallback.unity.com/packages/tools/physics/3d-wrapping-grapple-hook-188786 (metadata only)
- https://assetstore.unity.com/packages/templates/systems/advanced-grappling-system-199785 (metadata only)

Search excerpt only (fetch blocked or not verified):
- https://www.fab.com/listings/d50097ec-9c33-44f1-bce6-6ac039ae6d7f
- https://www.fab.com/listings/12ae72f7-d885-4227-b1ad-45d26d3e4202
- https://www.fab.com/listings/17f62a8f-387b-48b0-9198-af7273dc3733
- https://docs.unrealengine.com/4.27/en-US/Basics/Components/CableComponent
