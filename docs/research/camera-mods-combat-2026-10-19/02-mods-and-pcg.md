# Mods and PCG — 2026-10-19

ModDB and GitHub, read live. Ideas only.

## A. ModDB: what modders built that we can steal

### A1. Camera mods are the most-downloaded mods there are
- Source: discovermods "Best Camera Mods 2026" list (read 2026-10-19): across
  1361 games the top camera mods have huge download counts (Immersive Cam
  688k, Absolute Camera 371k, Enhanced Camera with Cinematic Shake Effect 175k,
  FOV sliders, Smooth Camera Motion, Photo Mode Unlocked). ModDB's "Better Camera
  Mod" for Total War: Warhammer III (closer/further heights, K hides UI, N zooms
  without unit fade, Alt+K more cinematic) is the same idea.
- Lesson: a camera is a **modding surface** players ask for by name. If our
  writer exposes a small `CameraTuning` (heights, distance, FOV, smoothing, shake
  scale, hide-UI) as data, an AI can generate camera mods as text. P5.
- Home: `DirectorParams` already has most fields; expose them as a rules-style
  data file. `crates/rules` is the existing "AI writes text, sandbox runs it"
  crate.

### A2. ES: Randomaze (Half-Life) — a randomising tool that generates maze maps
- Source: ModDB `esrm` (read 2026-10-19): a stand-alone tool generates
  maze-shaped maps, connects them, and hands the game the first map; every next
  map is generated as needed, from console commands, without leaving the game.
- Steal: the **tool-as-a-door** shape. Generation is a callable, parameterised
  command; maps chain; the game never restarts. Our `endpoint` door is the same
  shape (`world.bake`-style verb with a seed and params).
- Home: `crates/world` + `crates/endpoint`.

### A3. Cogmind's procedural map devlog (the best design write-up found)
- Source: Grid Sage Games blog "Procedural Map Generation" (2014) and "Roguelike
  Level Design Addendum: Procedural Layouts" (2019), read 2026-10-19.
- The method: (a) draw orientation mockups first (in a paint tool), (b) place
  **guaranteed entrances/exits far apart and build the layout around them**, (c)
  seed **tunnelers** that dig outward from each point of interest until they
  collide, giving circuitous routes, (d) add optional barrier bands that tunnelers
  cannot cross, (e) keep the mix of **static and procedural** explicit — static
  anchors the story and the community's shared landmarks, procedural gives
  replay.
- Steal, in order: exits-first layout; tunneler backbone; barrier bands;
  "static map with procedural pockets" as the default, not all-procedural.
- Home: `crates/world` bake (deterministic seed -> bytes) and `nav` bake.
- Kill rule: the baked bytes must hash x2 identical for a fixed seed — a rule we
  already have, so PCG must be bake-time, never in the tick.

### A4. Other ModDB PCG hits
- `Roguelike Detour` (Serious Sam's Bogus Detour): activate a room to spawn its
  horde, rewards open the map, boss guards the exit. Steal: **room activation as
  content gating** — one flag per room, no new systems.
- Neverwinter Vault `Geomorph Roguelike Dungeon Procedural Generation`: stitch
  hand-made geomorph tiles, entrances/exits as waypoints/triggers on each side,
  seed a required tile at a coordinate ("preflight"). Steal: **geomorph + a
  preflight constraint list** — the cheapest way to get a hand-authored look from
  procedural stitching.
- Theralite devlog (ModDB news, 2025): zone/biome parameters, one of four level
  generators per zone, walker (drunkard's walk, width-3 tunnels, guarantee every
  floor tile is reachable), classic Rogue room-and-corridor, template barracks.
  Steal: **generator-per-zone** as a parameter, and "guarantee reachability" as
  an assertion after generation.

## B. GitHub: mods and tools worth reading

| Repo | What | Steal |
|---|---|---|
| `mxgmn/WaveFunctionCollapse` (25.4k stars, read 2026-10-19) | Bitmap/tilemap generation from one example; simple-tiled model; used in Bad North, Caves of Qud, Townscaper, Matrix Awakens; ports in Rust, Go, C#, and adapters for Unity/UE5/Godot 4/Houdini | The **simple tiled model** (tiles + adjacency) is the practical form for us: a small adjacency table an AI can author. Constraint propagation, contradiction restart |
| `krychu/wfc` (MIT, single-file C) | Overlapping WFC with measured solves (cave.png 308 tiles, 128x128 in 371 ms; 256x256 in 1729 ms on a Core Ultra 9) | Real numbers to beat. One header, no deps |
| `Coac/wave-function-collapse` (Python) | 1D/2D/3D samples | 3D form (voxel) for a world bake |
| `ReplayMod/ReplayMod` (GPL-3.0) | Session record/replay, keyframe camera, tripods, render to video | See `01-camera.md` C2 |
| `MinecraftFreecam/Freecam` (MIT) | Freecam with tripods and collision toggles | See `01-camera.md` C3 |
| Modrinth "Camera Utils" collection | Zoomify, Freecam, Camera Utils, OrthoCamera, Motion Capture, Creator Overlays | Zoom ladder; orthographic view (a debug/measure view); motion record+replay; framing overlays |

### B1. Orthographic debug view
- `OrthoCamera` (Modrinth, read 2026-10-19): a configurable orthographic camera
  mode. Steal: an ortho mode is the right way to check layout, clearances and
  spacing without perspective lying to you. Very cheap in our renderer (one
  projection flag) and a real tool for the AI-door ("is this room 4 m wide?").
- Home: `crates/render` + `camera.rs` `CameraMode`.

## C. PCG in our engine: where it is allowed

The hash law decides this. `engine-core` is std-only and the tick hash covers
sim bytes; PCG that runs inside the tick must be a pure function of the seed (it
is, if it is our own splitmix-based code) or it must run at bake time and be
hashed at the end.

- **Bake-time PCG** (preferred): `crates/world` "seed N + size M -> versioned
  bytes, std-only". Add WFC / tunneler / geoform passes there. `crates/geo`
  already does cook-time mesh booleans and UV.
- **Runtime selection**: pick a variant by seed (one integer), not by generating
  geometry mid-tick.
- **Authored text**: `crates/rules` lets an AI write game rules as text and runs
  them replay-exact. A generation recipe should be the same: text or a small
  table, not code.
- **Asset PCG**: forge `factory/` already has `recipes.json` + `recipe-proof.mjs`
  + `Sidecars` for asset jobs. A PCG recipe belongs beside those.

## D. Papers about PCG live in `04-arxiv.md`

The PCG Benchmark (arXiv 2503.21474, 12 problems, quality/diversity/
controllability) is worth more than any single generator: it is a **testbed we
could adopt as a proof surface** for `crates/world`, the way the combat fixture
is a proof surface for `combat`.
