# Editor UI, art side, artist side: the gaps and what to steal

Read 2026-10-24. One question: a person and an artist each work one day in this
engine. Where do they get stuck, and which paper or repo already solved it?
Sources are the arXiv API (queries in "Reproduce"), pinned GitHub reads, and the
tree itself. Nothing was built. Every claim about us names a file. Every claim
about a donor names a pinned file or an abstract.

The tree count that sets the frame: **104 door verbs**, **9 editor panels**,
**46 editor source files**. The door is rich. The surface a person touches is
thin. Most gaps below are not "we cannot" - they are "the artist cannot reach
what already exists" or "the tool stops one step before the artist's question".

## 0. What is already good (so a gap is a real gap)

| Fact | Evidence |
|---|---|
| Preview -> approval -> apply -> undo; 256 ops per batch | `crates/endpoint/src/mcp.rs:123` `MAX_TX_OPS`, `tx.*` |
| 104 verbs in packs; a non-default pack costs 0 chars of the tool list | `crates/endpoint/src/packs/*.rs` `TOOL_*` |
| Dock shell, 9 panels, layout is data + hash, warm dark theme | `crates/editor/src/shell.rs:65` `Panel`, `theme.rs` |
| Gizmo translate/rotate/scale, snapping, W/E/R/T, orbit/pan/zoom/F | `gizmo.rs` (on `transform-gizmo`), `viewport.rs` |
| Prefabs, drag a GLB in, play-in-editor, live tuning, crash journal | `prefab.rs`, `browser.rs`, `play_in.rs`, `tuning.rs`, `wal.rs` |
| Material record: 8 layers, 6 lobes, 5 coverage channels, CIEDE2000, KTX2 | `material/src/record.rs:33,266`, `compile.rs:134` |
| Mesh substrate: xatlas UV, seam-locked LOD, V-HACD collision, normal bake | `geo/src/{uv,lod,collision,bake}.rs` |
| Asset intake: repair, simplify, LOD chain, hulls, bundle, provenance | `assets/src/gen_intake.rs:13` |
| Machine eyes: screen rects per body, SSIMULACRA2/FLIP/DeltaE, L1-L9 rubric | `view.rs:575`, `playtest/src/look/{metric,judge}.rs` |
| HUD already has 4 palettes and a contrast check | `ui/src/access.rs` `PaletteContrast` |

So: the substrate is strong. The missing part is the **artist's loop**:
see it, judge it, fix it, prove it, show it.

## 1. Editor UI gaps

| # | Gap | Evidence | Smallest shape | Effort |
|---|---|---|---|---|
| E1 | **One object at a time.** No multi-select, marquee, group transform, copy/paste or duplicate-by-drag. | 0 hits for `marquee`, `multi_select`, `clipboard` in `crates/editor/src`; `gizmo::drag_batch` takes one `ObjectId` (`gizmo.rs:261`) | Selection set + one batch; the door already accepts 256 ops | S |
| E2 | **No command palette / search-everything.** 104 verbs, 9 panels, no keyboard index; the outliner filters only. | 0 hits for `command_palette`; `outline.rs` filter only | One palette over verbs + nodes + assets; picking a verb shows its args | S |
| E3 | **No spatial pins, comments or claim markers.** The activity feed is rows of text. A person cannot point at a stone and say "this one". | `activity.rs` ring only; 1 hit for `comment` | `note.pin` (world pos, text, author) drawn in viewport, linked to `scene.diff` | S |
| E4 | **No asset thumbnails.** The browser lists names and a drop receipt; it never shows a pixel. | `browser.rs` `BundleEntry` has kind/name/tags; 0 hits for `thumbnail` | 256px thumbnail cache from the capture path, content-addressed | S |
| E5 | **No visible undo history.** Undo/redo returns receipts and the WAL journal records every command, but the editor has no history panel, no jump, no named state. | `lib.rs` `History` (layout only); `a3_session.rs:119` receipts; `wal.rs` journal | List the journal with author/tick and jump | S |
| E6 | **No animation timeline, dopesheet or curve editor.** Clips and state machines are verbs; play-in plays them; nothing shows keys. | `anim` pack has `anim.clip.*`/`anim.graph.*`; 0 hits for `dopesheet`; "timeline" hits are the tune tape only | A sheet document + preview frames; the UI can come after | M |
| E7 | **No material graph UI.** `graph.rs` is data-only; the material panel is sliders. | `editor/src/graph.rs` (no `NodeKind` consumer); `material` pack has set/param/preview/compare | Canvas + typed nodes + bake (see S4) | M |
| E8 | **No reference overlay, split view or region compare.** The metric crate has `Rect` and `Mask`, but `view.compare` always scores `Region::Full`. | `playtest/src/look/metric.rs:149`; `packs/view.rs:438` | Pass `region` through; slider/onion-skin in viewport | S |
| E9 | **No console panel, profiler overlay or GPU capture in the editor.** The game ships a 9-verb dev console behind a read-only door; the editor has only its log panel; perf is a verb you call by name; no Perfetto trace. | `game/src/console.rs` (EG26), `endpoint/src/console_bridge.rs`; editor has no console panel; finish-line item 13 | Console panel over the same door; perf HUD fed by `perf.table`; trace file later | M |
| E10 | **No reimport watch.** Drag a GLB once, drag again after every DCC change. | `browser.rs` has no watcher; EI3 was drop only | Watch the source dir, re-run intake on change, diff the receipt | S |
| E11 | **No color management authoring.** ACES, exposure and grade exist; no LUT file, no OCIO config, no white balance. | `render/src/post/chain.rs` has `tonemap_aces`; no LUT/OCIO parser | LUT file (`.cube`) + one config path; art direction presets | M |
| E12 | **No camera objects or shot list.** One viewport camera; no framing score, no trailer capture. | `viewport.rs` fixed eye; camera survey `2506.00974` | Camera node + shot list + `view.frame` score (S7) | M |
| E13 | **No HUD authoring surface.** `ui` is a data lane with palettes and contrast; no layout tool, no preview. | `ui/src/lib.rs`, `access.rs`; 0 `ui.` verbs | `ui.layout` constraints + preview PNG (S7) | M |

Not gaps, and deliberately not re-opened: docking (layout is data), one-click
starter scene, prefabs, play-in-editor, tuning, fracture, nav bake, level paint
(`docs/research/editor-verbs-2026-10-23/README.md`).

## 2. Art-side gaps (the content itself)

| # | Gap | Evidence | What it costs the look |
|---|---|---|---|
| A1 | **Materials have a strong record and no surface.** No procedural graph, no AO/curvature/grime bake, no texture authoring; `Textures` holds refs only. | `material/src/record.rs:147`, `compile.rs`; pack has set/param/preview/compare | Every material is hand-tuned scalars; stone, cloth and metal cannot get the maps that sell them |
| A2 | **The mesh tools are unreachable by the door.** `geo` has UV, LOD, collision, bake and FBX modules; the pack exposes only `geo.preview` / `geo.apply`. Intake never unwraps or bakes. | `geo/src/lib.rs` modules vs `packs/geo.rs` two verbs; `gen_intake.rs:13` | A prop that arrives without UVs stays without UVs; no budget report, no re-cook |
| A3 | **No debug buffers for art.** No normal/depth/albedo/material-id capture. | `view.observe` returns camera + bodies; no buffer path | "Why is this black?" needs a person with a GPU debugger; the AI sees a beauty PNG only |
| A4 | **Clips are position-only, no sockets.** Rotation additive exists; packed clips carry no quaternion lanes and no weapon socket. | `anim/src/clip.rs:6` (`QTrack` keys are `[u16;3]` position triples); finish-line item 14 | Hands slide, weapons float, retarget cannot land cleanly |
| A5 | **Terrain exists and no verb reaches it.** `world/src/terrain.rs` bakes a heightmap, a hole mask and 3-way splat (TVER), but no pack names terrain and the door cannot bake or edit one. | `terrain::{bake_terrain,parse_terrain,blocked}`; 0 hits for `terrain` under `crates/endpoint/src` | Rome camps, hills and sieges sit on a flat plate |
| A6 | **No FX authoring.** Particle and decal passes render; emitters are not data an artist can write. | render passes exist; no `fx.*` authoring verb beyond ragdoll/limb/break/blast | Dust, sparks and blood are one-shot effects, not a palette |
| A7 | **No look/staging presets.** No time of day, palette lock, key/fill ratio or sky preset as data. | `post/chain.rs` knobs; no preset file | Every scene starts from scratch; the game drifts shot to shot |
| A8 | **Impostor data is wired and nobody can see or set the rule.** `anim/src/crowd_scale.rs` hands far agents to `FarImpostorSink`; `render/src/crowd_impostor.rs` picks an octahedral impostor per distance and `assets/tests/it/crowd_cook.rs` cooks two specimens; no verb exposes the threshold, the budget or a card-vs-mesh check. | `crowd_inst.rs:538`, `crowd_impostor.rs`; 0 `impostor` verbs | One verb: threshold + budget + a capture that proves the card matches the mesh | S |
| A9 | **Audio has no cue sheet tied to ticks.** Synth and mixer exist; cues are not authored data. | `audio` crate; no `audio.*` verb | Impact sound is wired by hand per event |

## 3. Artist-side gaps (the workflow around the content)

| # | Gap | Evidence | What an artist says |
|---|---|---|---|
| W1 | **No import presets or asset lint.** Scale, pivot, naming, UVs, collider, sockets are not checked on the way in. | `asset.intake` repairs geometry; no `asset.lint` | "Why did my prop arrive 100x too big and with no collider?" |
| W2 | **No review surface.** One capture is scored; a set is judged L1-L9; there is no contact sheet, no per-asset gate, no worst-first list. | `view.compare` per pair; `look.judge` over a capture dir | "Show me the ten props, tell me which three are ugly, and why." |
| W3 | **No visual asset search.** Text/kind/tag search only; no previews, no scene-aware or multi-modal retrieval. | `browser.rs` `SearchQuery`; MetaFind `2510.04057` | "Find me a wall piece that fits this arch and this palette." |
| W4 | **No pose or clip library.** `pose_tape` records one pose; there is no shelf of poses/clips to reuse. | `pose_tape.rs`; no library verb/UI | "Give me the shield-wall idle I made last week." |
| W5 | **No reference-driven loop.** Drop a concept image; no overlay, no region diff, no "match this" score. | `view.compare` full frame only; no overlay | "Make it look like this" is a conversation, not a number |
| W6 | **No cost visibility.** Tris, texture bytes, draw calls and VRAM are not shown per asset or per level. | intake has `lod_tris`; no report verb/UI | "Which prop is eating the potato budget?" |
| W7 | **No named art versions.** Undo exists; "the darker grade from Tuesday" does not. | layout `History` + journal; no snapshot name | "Go back to the version the lead approved." |
| W8 | **No guided art path.** One-click starters reach a playable scene, but not import -> material -> light -> capture -> score. | `onboard.rs` covers first fight only | A new artist stalls at the second step |

## 4. Top steals, ranked

Each card: source, what we take, home, bounded reproduction, kill, number.

### S1 - `view.sheet`: a contact sheet with worst-first scores (P4 look)

- Source: our own `view.compare` (SSIMULACRA2 + diff PNG) and `look.judge`
  (L1-L9) already score pixels, but only one pair or one set at a time. The
  studio practice read from `donmccurdy/glTF-Transform@8e2a94a2`
  `packages/cli/src/inspect.ts`: one command prints a table per scene, mesh,
  material and texture, with `renderVertexCount`, `gpuVertexCount`, `gpuSize`
  and texture `gpuSize`. Review is a table, not a folder of PNGs.
- What we take: one call renders N views of M assets into one PNG sheet and
  returns a row per tile: asset, view, `ssimulacra2_x10000`, `delta_e`, worst
  32x32 tile rect, and a `worst` list sorted ascending.
- Home: `crates/endpoint/src/packs/view.rs` (`view.sheet`), reusing the capture
  path; compose in `engine_playtest::look` (it already decodes PNGs and writes
  the diff), so the pack only orchestrates.
- Bounded reproduction: 6 arena kit pieces x 3 views at 256x256 -> one sheet
  PNG (1536x768) plus JSON; same inputs twice give the same PNG bytes and the
  same digest; a deliberately dark piece sorts first.
- Kill: two runs differ; 18 tiles pass 5 s on this PC; or a tile cannot be
  traced back to its capture path and entity id.
- Number: review of 18 captures is 1 call and 1 file instead of 18 opens; the
  worst asset is named by the tool, not by the person.

### S2 - `view.buffers`: art debug buffers and better machine eyes (P4/P2)

- Source: SPEAR, arXiv `2607.06701` (ECCV 2026): a programmatic UE path that
  returns, beside beauty, "a non-diffuse intrinsic image decomposition,
  material IDs, and physically based shading parameters" at 73 fps 1080p. The
  point is not speed; it is that **beauty is the worst debugging image**.
- What we take: one capture call can write normal, depth, albedo, roughness and
  material-id buffers beside the beauty frame, each with a digest. Material id
  is the "click a pixel, get the asset" path.
- Home: `crates/render` (the GTAO prepass already produces depth; the normal
  target exists for lighting) and `packs/view.rs` (`view.buffers`).
- Bounded reproduction: one arena frame -> 4 buffers at 1280x720; beauty,
  normal and depth differ; changing only the sun moves depth by 0 and normal
  by 0 but moves beauty; a material-id lookup at a known fighter pixel returns
  that fighter's id.
- Kill: a buffer is a copy of beauty (byte-equal), or albedo changes when only
  a light moves (then it is not albedo), or a buffer needs a window.
- Number: 4 debug answers per frame instead of 0; the AI gets geometry facts,
  not a picture to guess from.

### S3 - `asset.cook`: expose the mesh chain the crate already owns (P5 build)

- Source: `zeux/meshoptimizer@f0a684f1` `src/meshoptimizer.h`: `simplify`,
  `simplifyWithAttributes` (position + normal + UV + caller channels),
  `SimplifyLockBorder`, per-vertex lock/priority flags, and `result_error` so a
  caller can refuse a bad reduction. `glTF-Transform` `inspect` for the report
  shape. Our `geo` crate already binds all of this.
- What we take: one verb that takes a GLB and options (`uv: keep|unwrap`,
  `lod: [1.0, 0.5, 0.25]`, `collision: none|hull|convex`, `bake: none|normal`)
  and returns the cooked bundle plus a budget table:
  triangles per LOD, hull count, UV chart overlap, map bytes, and a warning
  per budget miss.
- Home: `crates/endpoint/src/packs/asset.rs` (new `asset.cook`), chaining
  `geo::{uv,lod,collision,bake}` and `assets` bundle cook. No new geometry
  algorithm.
- Bounded reproduction: the 1,920-tri wall mesh from `placement_siege.json` ->
  3 LODs, one hull set, one 1024 atlas, one normal map; cook twice gives equal
  bundle bytes; the report names any LOD over budget; `asset.refs` still
  resolves the name.
- Kill: two cooks differ; a LOD exceeds its triangle target; UV
  `overlapping_chart_pairs` is non-zero; or the chain needs a window.
- Number: 4 unreachable modules become 1 call; an artist gets a budget row per
  asset instead of a failed cook late.

### S4 - `material.graph` + `material.bake`: make material text authorable (P4)

- Source: `RodZill4/material-maker@424dec64` (MIT): "textures and brushes are
  described as interconnected nodes" over Godot's `GraphEdit`, with bake to
  maps and 3D paint. Papers: GLOSS `2608.25461` (artist-selected reference
  patches, PBR, piloted as a Blender addon with professionals) and MatLat
  `2512.17302` (CVPR 2026 highlight) show the quality bar; we take the
  **authoring shape**, not the diffusion model.
- What we take: a small typed graph (noise, tile, gradient, curve, warp,
  blend) over the existing 8-layer material record, plus a bake that writes
  base-color, ORM, normal and emissive at 512 or 1024, and a preview thumbnail.
- Home: `crates/material` (evaluate graph -> layer/map values) and
  `packs/material.rs` (`material.graph.set|get`, `material.bake`); the editor
  canvas comes later from `graph.rs` + `egui-snarl` if needed.
- Bounded reproduction: one brick-wall graph -> 4 maps at 512; the same graph
  twice gives equal digests; applying it to the wall kit changes a capture
  inside the wall's screen rect and not outside it.
- Kill: graph evaluation is not deterministic; a 512 map passes 2 s; or a map
  comes out flat (`min == max`).
- Number: 1 text graph replaces N hand-set scalars; the map kit has 0 baked
  maps per piece today (`docs/research/blender-pipeline-2026-10-23/README.md`
  measured the same gap from the Blender side).

### S5 - `view.marks` + pins: let the AI and the person point (P2)

- Source: Set-of-Mark, arXiv `2310.11441`: overlay numbered regions on an
  image and a multimodal model grounds references far better. We already have
  `screen: Option<[f32;4]>` per body (`packs/view.rs:575`), so the marks are a
  render of data we own.
- What we take: `view.observe` returns `marks` (id, label, screen rect) and an
  optional marked PNG; `scene.pick` accepts a mark id. Pins add the person's
  half: `note.pin` at a world point, author, text, and an optional verb receipt.
- Home: `packs/view.rs` (marks), `editor/src/activity.rs` + `shell_ui.rs`
  (pins). No new model, no new renderer.
- Bounded reproduction: a 3-body frame -> 3 non-overlapping marks whose ids
  resolve through `scene.pick`; two runs give the same ids; the marked PNG
  differs from beauty only inside the mark rectangles.
- Kill: two marks cover the same pixels so an id is ambiguous, or the marked
  image differs outside the boxes, or ids move when nothing in the scene moved.
- Number: "the second fighter" becomes id 2, not a paragraph; a pin turns a
  chat sentence into a scene address.

### S6 - `look.propose`: an art-director loop that measures its own claim (P4/P2)

- Source: EZBlender `2601.07143` (plan + local ReAct, latency and cost as
  first-class), Layout-as-Policy `2605.25326` (refine a structured state with
  translate/rotate/rescale actions), and the confabulation study `2607.23213`
  (24 non-experts; users cannot separate a confident claim from scene fact,
  and awareness saturates under load). The lesson is not "ask a model"; it is
  **claim, then measure, then show both**.
- What we take: a verb that takes a capture + a reference and returns 3
  candidate deltas (exposure EV, key/fill ratio, palette shift), each with a
  predicted score; `tx.preview` applies one; `view.compare` returns the
  measured score. The reply carries predicted and measured side by side.
- Home: `packs/view.rs` (or a `look` pack) over `post/chain.rs` knobs and
  `look.judge`; the model is optional (a fixed 3-candidate ladder works with
  no model at all).
- Bounded reproduction: one dark arena capture and its reference -> 3 deltas;
  the chosen delta raises SSIMULACRA2 and does not move any pixel outside the
  frame; predicted vs measured are both printed; undo returns the exact prior
  look bytes.
- Kill: a delta cannot be expressed as an existing knob; the measured score
  contradicts the predicted ordering on the fixture; or a claim is printed
  without its measurement.
- Number: 1 call gives 3 art decisions with numbers; the confabulation risk is
  visible instead of trusted.

### S7 - the rest, in one table

| Steal | Source | Why it is cheap here | Effort |
|---|---|---|---|
| `ui.layout` (grid/group/whitespace constraints -> rects + preview) | ASP UI layouts `2609.03918` (user studies beat random and heuristics) | `ui` is already a data lane with palettes and a contrast check (`access.rs`) | M |
| `anim.sequence` (sheet of tracks, keys, easing) + region edit | Theatre.js `@6ea82b93` (project state `sheetsById` + revision), SMART `2610.28210` (localized edits + memory) | `anim` clips and state machine already exist; a sheet is another document | L |
| Sketch an arrow -> a joint (`joint.from_sketch`) | Sketch2Arti `2604.25781`, deep sketch-based modeling survey `2603.03287` | `pose_tape` already turns pointer input into a validated joint tape; a hinge axis is the same shape | M |
| `view.frame` (shot score: coverage, headroom, thirds, horizon) | camera survey `2506.00974`, GenDoP `2504.07083` | `look` metrics already compute regions and silhouettes | S |
| `world.terrain` verbs (bake, edit heights/holes/splat) over the existing TVER data | Vangers `2608.17390` (6 methods, TIN best, editable multi-layer, memory cost stated) | `terrain.rs` already bakes height + holes + 3 splat; only the verb is missing | S |
| Terrain/foliage scatter + WFC as verbs | `world/src/{wfc,scatter}.rs` have no consumer; LDtk rules in `editor-verbs-2026-10-23` | substrate already landed | S |
| Palette lock (capture palette vs target, DeltaE) | color harmony `2508.15777`; `material/src/compare.rs` CIEDE2000 | one histogram + one DeltaE | S |
| Asset retrieval (text/image/3D, scene-aware) | MetaFind `2510.04057`, multi-modal retrieval `2209.00682` | `asset.refs` and bundle tags exist; needs an index | M |
| Command palette + schema controls | VSCode quick input `@abf21692` (`IQuickItem`: label, description, detail, buttons); Leva `@402a1e9d` | 104 verbs are the index already | S |
| UI snapshot tests | `emilk/egui@bf5ac0da` `crates/egui_kittest` | editor UI has no pixel regression today | S |

## 5. Cool ideas that are not gaps, but would sell the engine

- **Four-up look-dev view**: beauty / albedo / normal / depth side by side. One
  key. The Unreal buffer-visualization habit, on our own buffers (S2).
- **Pixel -> asset**: click a pixel, get the material id and the asset name
  (S2). The fastest "what is that?" in any editor.
- **Cost heatmap**: tint the scene by triangles, texture bytes or shadow
  casters. A potato-tier debug view that no AAA editor ships because they do
  not have a potato tier.
- **Weathering strip**: dry -> wet -> blood on one material, driven by the
  existing 5 coverage channels. We already have the unusual part
  (`material/src/channels.rs`); show it.
- **Deterministic turntable**: 4 views + a contact sheet as the asset
  thumbnail. Same bytes every run, so it can be a gate (S1).
- **Before/after slider** in the viewport, with the diff PNG as a third tab
  (S1/E8).
- **Pose shelf** from `pose_tape`: recorded poses become thumbnails an artist
  can drag onto a fighter.
- **One-click `look.pass`**: a named style (e.g. "dusty noon", "arena dusk")
  sets exposure, key/fill, fog, grade and palette; capture; score; undo.
- **AI art director with receipts**: every suggestion carries predicted and
  measured numbers (S6). This is the honest version of "AI makes it pretty".
- **Scene lint as a receipt**: floating props, missing colliders, scale
  outliers, unlit regions, over-budget textures, no-op materials - one call,
  codes, fixes named.

## 6. What this does not settle

- No donor was executed: no Godot, Blender, Material Maker, Theatre or VSCode
  ran on this PC. Donor facts are static reads of the pinned commits.
- No timing was measured; the "number" lines are call counts and files, not
  seconds.
- arXiv sources are abstracts (and the camera survey's structure), not
  reproduced methods. A paper claim is not a reproduced number.
- The terrain card is the largest and least proven: the Vangers paper is a
  wgpu measurement of one data path, not a design for our world.
- Deliberately left out (already covered elsewhere): `level.paint`,
  `anim.retarget`, `nav.mark`, dialogue/quest (`editor-verbs-2026-10-23`);
  Blender cook ops and bakes (`blender-pipeline-2026-10-23`); fracture
  (`breakbench-blender-2026-10-23`).
- Paid and closed tools (Unreal buffer viz, Substance, SpeedTree, Houdini)
  are named as habits, never read as code.

## 7. Small-repo donors (0-300 stars), read 2026-10-24

A small repo can be read whole, so the mechanism is what we take. Every row is a
pinned commit read through the GitHub API, not a search hit.

| Donor | Stars | Licence | Gap | What we take | Where it plugs |
|---|---|---|---|---|---|
| `cachevector/image-diff@f23a2be8` | 2 | MIT | region diff | `Region{x,y,w,h}` + ignore list + one per-pixel pass (CIEDE2000, diff PNG, `score = 1 - diff/total`), with tests | `view.compare` region arg (S1/E8) |
| `ratatui/kasuari@4d1f3b25` | 56 | Apache-2.0 | layout solver | maintained Cassowary: `add_edit_variable`/`suggest_value` for drag-resize, `fetch_changes` for moved vars, strengths + tags | `ui.layout` solver upgrade (E13) |
| `lburdier/EasyUV@e4619716` | 0 | GPL-3.0 | UV unwrap | LSCM + ARAP relax, disk-chart split, and a quality score with texel-density spread and seam length as named parts | `geo.uv` preview score (A2/S3) |
| `SighFyi/kibaudio@691980f1` | 0 | Apache-2.0 | audio cues | cue anchor (beat/downbeat/marker) + offset, one tick = one beat, forward-only clock smoother, ducking as data | `audio.cue` sheet (A9) |
| `Skyfish1/lut-utility@1ef4ca05` | 6 | MIT | LUT | single-file `.cube` parse + trilinear sample + clamp, plus a writer | render `post::lut` (E11) |
| `avinkrisv/egui-xyflow@8532bcda` | 19 | MIT | marquee | screen->flow rect transform, overlap-area test with `Partial`/`Full` modes, skip hidden and unselectable | editor selection (E1) |
| `nochinator/open_desktop@915216cd` | 1 | MIT | command palette | two-tier match (word prefix for <=2 chars, skim score after) and a frecency boost capped at +60% so usage breaks ties but never beats text | editor palette ranking (E2) |
| `FrankelJb/vcsr@61ad7995` | 11 | MIT | contact sheet | grid sizing from target width, per-tile caption with a shadow pass, a real per-frame score, sort by score | `view.sheet` layout (S1) |
| `rdeioris/shadewire@31242c2b` | 0 | MIT | node graph | typed `PortType` with parse round-trip and colour-space flag; interface ports + nodes + named outputs | `material.graph` typing (S4) |
| `xKrvZ/Terra@63e822ad` | 9 | MIT | height field | layer stack with a fixed evaluation ladder (blueprint -> fields -> climate -> blend -> hydro -> materials) that keeps editable layers deterministic | `world.terrain` layers (A5) |

Not taken: `BadMachine/egui-marquee` (clean marquee state machine, no licence
file); `huxingyi/simpleuv` and `jpcy/xatlas` are above the star cap here and
xatlas is already bound by `crates/geo/src/uv.rs`.

## 8. Landed the same day (2026-10-24)

The wave below was built after this report, from the same tree. Each row names the
door surface or crate API, not a plan. Commit short shas are on `master`.

| Gap | Landed surface | Commit |
|---|---|---|
| E1 | `editor::select::{Selection, marquee, Clipboard}`, `gizmo::drag_set` (one op per object, one revision) | `7d479e946`, `6fc36c796` |
| E2 | `editor::palette` index + frecency + `PaletteState`, overlay, Ctrl+K | `7d479e946`, `6fc36c796`, `81a0ee23c` |
| E3 | `stage.pin.add/list/clear` in a non-default stage pack | `b349cabd8` |
| E4 | `editor::thumbnail::ThumbnailCache`, `browser::rows_with_thumbs` (a miss is `—`) | `6fc36c796` |
| E5 | `Panel::History`, `History::{rows_newest_first, activate, name, version}` | `6fc36c796`, `81a0ee23c` |
| E6 | `editor::timeline` sheet (keys, easing, region) + `Panel::Timeline` | `79ff81cfe`, `f9244f27d` |
| E7 | `editor::graph_canvas` typed ports + `Panel::Graph`; `Graph::evaluate()` bakes the canvas through `engine-material` | `f9244f27d`, `a4f75e839`, `924676633` |
| E8 | `view.compare` `region` + `ignore` rects | `6b8ef7df3`, `0fd6a37f2` |
| E9 | `editor::console::ConsolePanel`, `editor::perf_panel` overlay | `81a0ee23c`, `a4f75e839` |
| E10 | `editor::watch::{Watch, drop_report, ReimportReceipt}`, `ImportPump` polled in the frame loop | `6fc36c796`, `79ff81cfe`, `dd1efe240` |
| E11 | `render::post::lut` (`.cube` parse, trilinear, f16 table), chain tap, `LookPreset`, GPU 3D upload + display pass with a device oracle | `af34a4894`, `57dc567fe`, `eca148329`, `0146c57f5`, `22d23ebba` |
| E12 | `view.frame` score, `stage.camera.*`, `stage.shot.*` | `dac09035d` |
| E13 | `engine_ui::layout` solver + pins, `ui.layout.solve/resize` verbs | `f77a2f545`, `f6ea70eb4`, `6d56102d0` |
| A1 | `material::graph` typed nodes, `material.graph.set/get`, `material.bake` 4 maps + 5 coverage maps, weathering strip, KTX2 | `3127a6673`, `28ff4afbb`, `ecb53b8a5` |
| A2 | `geo.uv/lod/hull/bake` preview+apply, `UvScore` with texel-density spread | `9b1c12af8`, `be0066121` |
| A3 | `render::buffers::DebugFrame` (normal, depth, albedo, id, digests), `lookup_id`, GPU `read_frame`, `fs_gbuffer` + `Presenter::debug_capture` with a device oracle | `57dc567fe`, `0146c57f5`, `69dbe9287`, `d95428859`, `afb8e7674` |
| A4 | `anim::clip_rot` lane, socket, blob, bake/sample, `anim.rot.preview`, cook sidecar | `a6326d85f`, `733cbe75e`, `be0066121` |
| A5 | TVER writer/paint, `world.terrain.*`, layers, `world.scatter`, `world.wfc` | `3ace33cb1`, `60e0639c7`, `6dc17a9a7` |
| A6 | `fx.preset.list/preview/set/clear` with a deterministic emitter preview | `f48581da1` |
| A8 | `asset.impostor` + an impostor row in `asset.budget` | `be8b8f072` |
| A9 | `engine_audio::cue_sheet` + anchors/clock, `audio.cues`, `audio.cue.resolve` | `ce1529853`, `da8e385a3`, `6d56102d0` |
| W1 | `asset.lint` with codes and fixes | `92a6e3cf6` |
| W2 | `view.sheet` contact sheet, worst-first | `888802e4f` |
| W3 | `asset.search` (text, kind, palette) | `be0066121` |
| W4 | `editor::pose_shelf` + `Panel::Poses` | `6fc36c796`, `f9244f27d` |
| W5 | `look.propose` predicted vs measured | `888802e4f` |
| W6 | `asset.budget` per-asset cost rows | `92a6e3cf6` |
| W7 | `History::name` / `version` named states | `6fc36c796` |
| W8 | `editor::art_path` five-rung ladder + overlay | `f9244f27d`, `a4f75e839` |

What still needs a caller, and why: the game loop has to call
`Presenter::set_look` when the look changes (that crate is another lane); terrain
has a draw path and a data path but no nav wiring; no Perfetto trace yet; and one
honest mismatch to fix: `render` turns crowd cards at 45 m while `anim`'s far tier
starts at 60 m, so `asset.impostor` reports the anim threshold and its warning
names the render one. One red gate is not ours and is written down in
`docs/BLOCKERS.md` (2026-10-24T07:03Z): the capture twin's sky-off pair is not
repeatable, which predates this wave (`2862f593a`, with the LUT and debug slices
reverted), so "two captures of one list are equal" is not yet a promise the art
loop can lean on.

## Reproduce

```pwsh
# arXiv (queries used, accessed 2026-10-23T21:20Z..21:22Z)
# https://export.arxiv.org/api/query?search_query=all:%22scene%20editing%22%20AND%20all:%22language%20model%22&max_results=12
# ... all:"motion editing" AND all:"text"; all:"PBR" AND all:"texture generation";
#     all:"sketch" AND all:"3D modeling"; all:"camera" AND all:"cinematography";
#     all:"procedural content generation" AND all:"level"; all:"terrain" AND all:"authoring";
#     all:"GUI" AND all:"layout generation"; ti:"Set-of-Mark"; all:"3D asset retrieval";
#     all:"color harmony" AND all:"palette"
gh repo view RodZill4/material-maker --json nameWithOwner,defaultBranchRef,licenseInfo
gh api repos/zeux/meshoptimizer/contents/src/meshoptimizer.h?ref=f0a684f11ead064e176775c9b796f82a55910e92
```

Pinned donors, all read live 2026-10-24:

| repo@rev | licence | what was read |
|---|---|---|
| `RodZill4/material-maker@424dec64` | MIT | `README.md` (node graph, bake, paint) |
| `theatre-js/theatre@6ea82b93` | Apache-2.0 | `packages/core/README.md`, `projects/Project.ts` (on-disk state, revision, sheets) |
| `donmccurdy/glTF-Transform@8e2a94a2` | MIT | `packages/cli/src/inspect.ts` (per-scene/mesh/material/texture table, gpu sizes) |
| `zeux/meshoptimizer@f0a684f1` | MIT | `src/meshoptimizer.h` (`simplifyWithAttributes`, lock/priority flags, `result_error`) |
| `microsoft/vscode@abf21692` | MIT | `src/vs/platform/quickinput/common/quickInput.ts` (`IQuickItem`) |
| `pmndrs/leva@402a1e9d` | MIT | schema-driven control pattern (repo) |
| `emilk/egui@bf5ac0da` | Apache-2.0 | `crates/egui_kittest` (UI snapshot tests) |
| `AcademySoftwareFoundation/OpenColorIO@22dd9c4f` | BSD-3-Clause | color config/LUT gap (repo) |
| `godotengine/godot@c8256a8e` | MIT | import dock / asset browser habit (already cited in editor-verbs) |

Paper list, all cited above: `2310.11441` (Set-of-Mark), `2504.07083` (GenDoP),
`2506.00974` (camera trajectory survey), `2508.15777` (color pairing),
`2510.04057` (MetaFind), `2512.17302` (MatLat), `2601.07143` (EZBlender),
`2603.03287` (sketch 3D survey), `2604.25781` (Sketch2Arti), `2605.25326`
(Layout-as-Policy), `2607.06701` (SPEAR), `2607.23213` (confabulations),
`2608.17390` (Vangers height fields), `2608.25461` (GLOSS), `2609.03918`
(constraint UI layouts), `2610.04529` (TAME), `2610.28210` (SMART). Also read
but not ranked: `2606.20146` (BIM-Edit: best model 49.5% average, no model
fully solves more than 3.4% of edit tasks - the reason edit-preserving evals
matter), `2604.11006` (emission textures), `2511.19437` (LumiTex),
`2506.05449` (3D environment survey), `2605.01783` (runtime PCG evaluation).
