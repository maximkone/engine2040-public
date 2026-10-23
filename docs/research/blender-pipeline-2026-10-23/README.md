# Blender as a content and look factory — what lands in weeks

Read 2026-10-23. One question: the pinned Blender (5.2.0 LTS) is already our
DCC tool (`tools/dcc/blender-cook.mjs`, rows GEO1a/GEO1b). What else can it
generate for the legionary game before it costs a row?

Baseline measured today on this tree:

- Recipe surface: 4 ops (`cube`, `bevel`, `mirror`, `array`), one GLB out plus
  4 digests; a fresh `blender -b` per recipe (cold start 5.48-6.68 s warm,
  18.07 s under load, GEO1a).
- Map kit (`content/maps/src/*.gltf`): **15 pieces, 25,968 tris, 1 material
  each, 0 images**. Flat `baseColorFactor`, UVs 0..1, no tiling.
- Arena kit: 8 pieces, 7,472 tris, 1-2 materials, 3-4 textures each, 15 KTX2
  (VR2 landed the PBR intake and the KTX2 cook).
- The map kit already comes from a deterministic JS generator
  (`content/maps/tools/gen.mjs`, integer-lattice noise, box stacks, COL_/FRAC_/
  SOCKET_ parts). LOD + simplify already live in Rust
  (`crates/assets/src/gen_intake.rs`, `PROP_MAX_TRIS` 5000).
- A siege fort is 7 actors; a wall is one 8 m mesh scaled `sx 8`
  (`content/maps/placement_siege.json`). Stretched boxes, not a wall run.

So the gaps are not "more boxes" and not "LOD" (owned). They are: a wall that
follows a plan, maps that carry detail, and startup cost per cook.

## 1. The paid product, read as a spec

`Engine Generator` by Malc, $9.99, Superhive/BlenderMarket listing read live
2026-10-23 (<https://superhivemarket.com/products/enginegenerator>); the
Gumroad page (<https://malcd.gumroad.com/l/EngineGenerator>) is a JS shell and
returned no text to a plain fetch. Snippet from search: "The Ultimate Dynamic
4 Stroke Engine Generator for Blender! Built from the ground up with geometry
nodes. Requires Blender 4.1.1 or later."

What it actually is (from the listing):

- Blender 4.1-5.2, geometry nodes, "animated and render ready", 70+ controls:
  bore, stroke, cam lobe profile, firing order, cam timing, RPM, flywheel
  radius, bore spacing.
- Every part moves because an input moved: crankshaft, rods, pistons, valves,
  cams. One node graph, not a model.
- "Error messages that give realtime feedback (including a check engine
  light)" — the graph refuses an invalid parameter combination and says which.
- The node graph is open: a buyer can extend it.

**The spec we keep is the contract, not the engine.** A named parameter block,
one pure function from that block to geometry plus motion, a visible refusal
for a bad combination, and a graph a reader can open. That is exactly what our
recipe format lacks today: 4 ops with ad-hoc fields, no parameter block, no
named refusal beyond `recipe.unknown_op`.

Careful: BlenderNation's 2022 free "Geometry Nodes: Engine Generator" is
Digitalpottery's, a different asset. Same words, different product.

## 2. Free candidates read live (default branch, pinned)

| # | repo@rev | what it does | licence | last commit | headless? | what a cook step would use it for | one number |
|---|---|---|---|---|---|---|---|
| 1 | `ranjian0/building_tools@0216534` | footprint → floors, walls, doors, windows, roofs, balconies; `btools/api` takes dataclasses | MIT | 2025-05-16 (branch push 2026-09-24) | **no on 5.2 as-is**: import fails, then crashes (§3) | the option checklist for our own wall/gate op; code port is now unattractive | wall run from 1 JSON vs a mesh scaled `sx 8` |
| 2 | `ahujasid/mcp-for-blender@5a3618f` | socket server inside Blender, JSON protocol, main-thread queue, 32k stars | MIT | 2026-10-20 | partial: has a `bpy.app.background` branch, timers registered | the protocol + a warm cook process | cook wall seconds |
| 3 | `franMarz/TexTools-Blender@89cb4cf` | UV toolset + bake ops (bake, texel density, island align) | GPL-3.0 | 2024-12-02 | likely (operators on a real object) | the bake op set to copy as behaviour | maps per piece 0 → 2-3 |
| 4 | `alessandro-zomparelli/tissue@d2e8393` | tessellate, dual mesh, lattice, weight tools; extension manifest, Blender 4.5+ | GPL-2.0-or-later | 2026-06-07 | likely (numpy loops, no UI needed) | stone courses, roof tiles, shield rows on a surface | tris per detail band |
| 5 | `leukbaars/DreamUV@aecc2b5` | trim-sheet UV: boxwrap, trim, stitch, transfer | **CC0** | 2026-04-17 | viewport-modal; only the UV math ports | trim-sheet UV for walls and stands (VR2's left leg) | material count per kit |
| 6 | `nortikin/sverchok@4bf746c` | big parametric node graph (fields, lists, meshes) | GPL-3.0 | 2026-10-22 | undocumented in background | nothing direct; a reference for a param-block graph | node count |
| 7 | `jpcy/xatlas@f700c77` | UV atlas packing library, MIT, no CLI | MIT | 2022-07-25 | library, not Blender | atlas across kit pieces | 15 × 512² pages → 1 sheet (bigger bytes here) |
| 8 | `wjakob/instant-meshes@7b31608` | field-aligned retopo, GUI only | BSD-3-Clause | 2019-11-03 | no batch mode in the README | **reject**: nothing to retopo, our meshes are 1-3k tris and `gen_intake` simplifies | — |

Not listed: BlenderProc (GPL-3.0, 2026-01-20) — a headless Blender pipeline,
which our cook already is; and `s-leger/archipack` — last commit 2020, Blender
2.79, dead.

## 3. Two probes run on the pinned Blender today

Both in `C:\Users\me\AppData\Local\Temp\opencode`, nothing written to the tree.

1. **Bake works headless.** Cube + 1 material, 256² image, Cycles CPU, 8
   samples, AO bake: `E2040-BAKE ok 0.38 s`, PNG 2,293 bytes, pixels
   `min 0.000 max 1.000 mean 0.569` — a real gradient, not a flat fill. One
   process, factory startup, no addon, no GPU.
2. **building_tools does not run on 5.2.** `import building_tools.btools.api`
   raises `ValueError: mutable default <ArchOptions>` (Python 3.13 dataclasses;
   the file writes `arch: ArchOptions = ArchOptions()`). Patched to
   `field(default_factory=...)` in the temp copy, `create_floorplan(default)`
   crashed with `EXCEPTION_ACCESS_VIOLATION`; with explicit options it returned
   an Object but the scene held 1 quad (4 tris). `bl_info` says Blender 3.3.
   Verdict: read its option model, do not port its code.

## 4. The three steals, ranked

### S1 — a wall run from a layout, not one stretched box (P5)

Spec source: the Engine Generator contract (param block, pure geometry,
named refusal). Option checklist: building_tools (MIT, shape only).

- **Home:** `tools/dcc/blender/run_recipe.py` (new ops `loft`, `merlon`,
  `gate`), `tools/dcc/recipes/fort.json`, the op guard in
  `tools/dcc/blender-cook.mjs`; output consumed by `crates/assets`
  (`gen_intake`) into `content/maps/src/`.
- **Bounded reproduction:** one `fort.json` — 4 wall runs 40 m, 2 towers at the
  corners, 1 gate opening, merlons every 1.5 m, height band 3 m — cooked twice.
  Compare against the 7-actor `placement_siege.json` whose wall is one 1920-tri
  mesh scaled `sx 8`. Report: GLB sha256 x2 equal, tris, cook seconds, and that
  the exported parts still carry the `COL_`/`FRAC_`/`SOCKET_` names (box stacks
  today depend on them).
- **Kill:** the two digests differ; a cook passes 30 s; or the exported parts
  lose the `COL_`/`FRAC_`/`SOCKET_` naming, so physics, fracture and sockets
  break downstream.
- **Number:** 1 layout file replaces 15 hand pieces (25,968 tris) and every
  wall length becomes exact instead of stretched.

### S2 — bake the maps the map kit never had (P4)

Source: Blender's own Cycles bake (measured above); TexTools' bake op set
(GPL-3.0, behaviour only).

- **Home:** `tools/dcc/blender/bake_maps.py` + one recipe op `bake`; the PNG
  then rides the existing KTX2 cook (`crates/assets/src/ktx2_cook.rs`, VR2) into
  the bundle.
- **Bounded reproduction:** the 15 map-kit pieces (1 material, 0 images) get AO
  and a curvature-driven grime mask at 512², plus a normal from the bevel;
  one piece first. Report: maps per piece 0 → 2, baked bytes, cook seconds,
  and a capture with the map on vs off that must differ in a masked region.
- **Kill:** a map comes out flat (`min == max`), a 512² map passes 2 s on this
  PC, or the on-vs-off capture shows no pixel difference.
- **Number:** 15 flat pieces get grime and cavity shading; material count stays
  1, texture bytes are new and must earn their slot with a capture.

### S3 — one warm Blender per cook batch (P3)

Source: mcp-for-blender's protocol (MIT): one JSON request, executed on
Blender's main thread, one response; it already asks `bpy.app.background`.

- **Home:** `tools/dcc/blender-cook.mjs` (`cookRecipe`) and a `serve` mode in
  `tools/dcc/blender/run_recipe.py`.
- **Bounded reproduction:** cook the same 10 recipes two ways — 10 fresh
  processes (today) and one resident Blender that calls
  `read_factory_settings` between recipes. Report: wall seconds both ways and
  all 10 GLB digests equal.
- **Kill:** any digest differs from the fresh-process cook (then the cold start
  is the price of determinism and the idea dies), or the batch is under 2×
  faster.
- **Number:** 5.5-18 s of startup per recipe → one startup per batch.

Order: S1 is the content gap (a fortress, a Dacian wall, an arena shell from a
plan); S2 is the look gap on the kit that has no maps; S3 is the cook clock.
S1 and S2 can land together — the loft op emits the mesh the bake op needs.

## 5. What this does not settle

- `tissue` and `TexTools` headless on 5.2: both look compatible (5.2 sits inside
  tissue's 4.5+ manifest), neither was installed or run.
- Whether a GN-built wall can keep the `COL_`/`FRAC_`/`SOCKET_` part names
  through the GLB exporter: GEO1b exports with `export_apply=True`, which
  collapses modifiers; unproven.
- The Engine Generator's 70 controls were read from a listing, never opened. The
  node graph itself is paid and stays unread.
- No capture score was measured; S2's "look" claim is a plan, not a number.

## Reproduce

```pwsh
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" -b --factory-startup `
  -noaudio --python-exit-code 1 -P <probe>.py -- <arg>
```

`bake_probe.py` and `bt_probe.py` were the two scripts above; the first prints
`E2040-BAKE`, the second `E2040-BT`.
