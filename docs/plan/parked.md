# Parked areas

Goal: keep parked areas small and say what un-parks each.
Acceptance:
1. Each area names the Core proof that un-parks it.
2. World: streaming with zero tick hitches. Audio: never in the hash, 0 audio-thread allocs. Editor: panel parity with the MCP verbs.
3. Golden unchanged: `node tools/proof.mjs` golden d57b5c8ae3096614.
Order: WD7 WD8 WD12 WD13 WD14 WD15 WD3 WD9 WD5 WD11; SN11 SN9 SN4 SN5 SN10; ED14 ED15 ED16 ED17 ED18 ED19; PLF9 PLF10 PLF4 PLF5 PLF6 PLF7 PLF8; UIX4 UIX6 UIX7 UIX8 UIX10 UIX11 UIX12 UIX9
Owner crates: world, audio, editor, ui.
Open board IDs: WD3 WD5 WD7 WD8 WD9 WD11 WD12 WD13 WD14 WD15 SN4 SN5 SN9 SN10 SN11 ED14 ED15 ED16 ED17 ED18 ED19 PLF4 PLF5 PLF6 PLF7 PLF8 PLF9 PLF10 UIX4 UIX6 UIX7 UIX8 UIX9 UIX10 UIX11 UIX12
Pre-merge text: git show d96091cd:docs/plan/world.md
Pre-merge text: git show d96091cd:docs/plan/audio.md
Pre-merge text: git show d96091cd:docs/plan/editor.md

## 1. Where it stands

### world

Goal: stream three Roman maps in cells, bind nav and physics per cell, drive day, night and weather, never touching the simulation hash. Owner `crates/world`; consumers `content/maps/`, `game`. Prefix WD.

| Landed | Code |
|---|---|
| Cell streaming (WD1, ticked) | `crates/world/src/cells.rs`: frame-boundary publish, halo-seam bake, typed `CellError` |
| Terrain bake, grid A*, blockout maps | `terrain.rs`, `lib.rs` `astar8`, `blockout.rs`; bake SHA x2 |
| Navmesh, crowd, traversal links (NV1, NV2, TV1) | `crates/nav/src/mesh.rs`, `crowd.rs`, `links.rs` |
| Asset streaming and ladder (AS2, GP4) | `crates/assets/src/stream.rs`; `crates/game/src/ladder.rs` |
| Ludus map kit (CT9, ticked) | `content/maps/ludus/`; district and siege maps not on disk |
| Siege ballistics | `crates/game/src/ballistics.rs`: fixed-point, sorted impacts; integrated |

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| WD3 | Rome district map: streets, rooftops and a market for TV1 traversal | C | CT9, WD1, TV1 |
| WD5 | Day/night and weather cycle driving VX2 and RT4 | C | VX2 |
| WD7 | Cell publish gate: dual ahead-agent check before any cell goes live | C | WD1 |
| WD8 | Versioned mesh tiles: per-cell NV1 bakes stitched on shared edges | C | WD1, NV1 |
| WD9 | District traversal bake: sidecar edge markers into NV2 registries | C | WD3, TV1 |
| WD11 | Weather presentation contract: rain, wind and fog clocked by WD5 | C | WD5, VX2 |
| WD12 | Map assembly manifests: versioned cell manifest with provenance gate | C | WD1, CT9 |
| WD13 | City grammar and map import: road graph rewritten in Rust | C | WD1 |
| WD14 | Node-constraint collapse for arena tilings (propagate and collapse) | B | WD1 |
| WD15 | Staged arena-ground bake: ordered seeded passes for courtyard dirt | B | WD1, NV3 |

Core proof: a scripted walk across 9 cells with zero tick hitches, memory flat over a 30-minute loop, every cell bake byte-identical twice.
- CUT: a seamless open world (cells only); runtime AI mesh generation in the shipped game.
- CUT: terrain editor, marketplace or UGC pipeline, server-fleet orchestration.
Gates: cell bakes identical x2, shuffled inputs identical; broken seams and unreachable islands refused before publish; golden hash with cycle and weather on equals off.

### audio

Goal: wire the soundscape into the played game in order (device wall, WIN1 select, operator listen), land one legal HRTF path, lock budgets. Presentation only: never hashed, never in the tick. Owner `crates/audio`; consumers `game` (WIN1), `arena`. Prefix SN.

| Landed | Code |
|---|---|
| Device mixer and decode (AU1), spatial mix and crowd reactions (AU2) | `device.rs`, `spatial.rs`, `crowd.rs`; cpal, lewton, hound |
| Device-loss heal (AH3) | `heal.rs`: 100/200/400 ms ladder, quarantine on the 4th |
| Reverb, occlusion, director (SN1-SN3) | `reverb.rs`, `occlude.rs`, `director.rs`; wired by SN6 |
| WIN1 wiring for sends and occlusion gains (SN6, ticked) | `crates/game/src/audio_director.rs`; Wave 36 |
| Seeded music, bake-only voice, bus DSP | `music.rs`, `voice.rs` (live TTS refused by type), `dsp.rs` |
| HRTF core (SN7) | `crates/audio/src/hrtf.rs`: 8/8 headless proxy, 0 audio-thread allocs; module proof, not a played mix |

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| SN4 | HRTF via Steam Audio behind the device wall | B | AU1; LATER |
| SN5 | Headphone 3D: an HRIR-sphere HRTF path as fixed tables | B | AU1; LATER |
| SN9 | Voice-bake vs voice-net boundary: typed tests proving bake-only refusal | B | ON8, AU1 |
| SN10 | Hit-sync numbers in the logs: per-hit loudness, clipping, sync delta | B | PQ8, AU1; LATER |
| SN11 | Repeat-last concealment over silent XRUN gaps instead of silence | B | CQ13 |

Not on the board: SN12 (device-wall stage) and SN13 (WIN1 headphone select). No open row carries the played HRTF path.

Core proof: WIN1 tunnel has tails and walls muffle in the played mix; zero audio-thread allocs; PF1 fails on a seeded audio regression.
- CUT: licensed soundtrack bundles; runtime TTS (refused by type); mic-captured claims without an operator listen.
- CUT: Steam Audio never vendored (source says SN4 killed with receipt; the board still lists SN4 open).
Gates: zero audio-thread allocs under the counting allocator; blind-listen fixtures per row (wall A/B 4/4, orbit 7/8); golden x2 on every row.

### editor

Goal: a stranger can inspect anything, find anything, play without leaving, and learn in one sitting. Owner `crates/editor`; consumer every human and AI session through the MCP verbs. Prefix ED.

| Landed | Code |
|---|---|
| Dock shell, picking, gizmo, save and reopen, child play (E1-E3) | unit-tested, integrated |
| Schema inspector with live edge (ED2, ED13 ticked) | `inspector.rs`: live `SchemaRegistry` and `Batch` edge |
| Asset browser and play-in-editor (ED3, ED4) | coded, unit-tested; `browser.rs:10` mirrors still TEMPORARY |
| Outliner and automatic inspector (ED6, ED9) | coded, unit-tested; live-panel wiring open |
| Provenance, capability tokens, undo, snapshots (PR1, CAP1, SU1, PL1) | integrated, ticked |
| Onboarding and templates (ED5, TPL1) | `onboard.rs`, `templates.rs`; re-measure open as ED18 |

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| ED14 | Browser real-edge wiring: AS1 EntryKind and SP5 place replace mirrors | B | ED3, AS1, SP5 |
| ED15 | Live-panel integration: hierarchy tab and inspector on one selection | B | ED1, ED2, ED6 |
| ED16 | Play parity on three tapes: child play and play_in match checkpoints | B | ED4, WIN1 |
| ED17 | Panel frame budgets: every panel at most 1 ms under PF2 | B | ED1, PF2 |
| ED18 | Newcomer re-measure: first fight within 15 minutes, live play-in-editor | B | ED4, ED5 |
| ED19 | Outliner at siege scale: 2,000 rows, search 50 ms, panel 1 ms | C | ED6 |

Not on the board: ED20-ED25 (source Tier B+ rows). Dropped; they stay in git.

Core proof: panel parity with the MCP verbs (each panel shows the command-service revision); play parity on three tapes; every panel at most 1 ms on the RTX 3050.
- CUT: XR/VR panels, mobile lanes, UGC marketplace, liveops.
- CUT: visual scripting and code mods (no rows ever).
Gates: every panel row has panel tests and an Xvfb smoke run; every edit goes through commands with undo, provenance and CAP1 coverage; one test asserts viewport, outliner and inspector share one selection.

## 2. Finish line

| Area | Core proof that un-parks it | Status |
|---|---|---|
| world | 9-cell walk with zero tick hitches; memory flat over 30 min; cell bakes identical x2 | Cell streaming landed (WD1). WD3 WD7 WD8 WD12 WD13 WD14 WD15 ready; WD5 and WD11 wait on VX2; WD9 waits WD3. |
| audio | WIN1 tunnel tails and wall muffle audible in the played mix; 0 audio-thread allocs; PF1 fails on a seeded regression | SN6 landed (ticked). HRTF is a headless core only. SN11 ready; SN4 SN5 LATER; SN9 waits ON8; SN12 and SN13 not boarded. |
| editor | Panel parity with the MCP verbs; play parity on three tapes; every panel at most 1 ms | ED13 ticked. ED14-ED19 open and all ready. ED20-ED25 not on the board. |

## 3. Target design

- world: Cells stream through AS2 async jobs and publish at frame boundaries; the tick reads only live cells, and a late cell makes the walker wait at the edge. The cycle clock is presentation only: it reads the tick and never writes sim state.
- audio: Buses stay; SN6 adds sends and occlusion gains; SN7 allows one HRTF path on the frozen `hrtf.rs` API, wired in order device wall, WIN1 select, operator listen. The game thread sets octant and mode; the audio thread only consumes.
- editor: Panels read the RC4 schema and the command service only; one `Selection` type feeds viewport, outliner and inspector. Play runs the WIN1 fixed-tick loop with PL1 snapshots; child-process play stays as the fallback.

**Steals 2026-10-12 (HUD, sound, input)** (board shard `../SPRINT-21-look-feel.md`, scout file `../research/steals-2026-10-12/audio-ui-matrix.md`). The UI and audio parks lift for the duel only. Audio: the played mix goes only through CueBank and AudioEngine; every combat event maps to a cue by a presentation-side (kind x surface) lookup (SN12); occlusion returns gain and cutoff, distance adds air absorption, reverb blends three zone presets (SN13); hit-to-speaker latency is measured, p95 <= 40 ms or cues are scheduled early (SN14); Steam Audio is studied, never vendored. UI: the HUD is a flat command list (rect, border, text, scissor) drawn by the hud pass with glyphon text; egui stays tools-only; menus pick Taffy or a Clay port after the HUD cost is measured (UIX13). Input: raw device -> tick-counted conditions -> FighterInput -> tape; action sets stack; the last device picks the prompt glyphs; a rumble table maps cue to motors (UIX14, GP46). Gaps no row owns yet: a frame-data timeline editor, localization beyond ASCII, a screen-reader tree.

## 4. Hard problems

Kept for open rows only: world seam bake and mesh-tile versions (WD7, WD8), audio HRTF choice by blind orbit fixture (SN4, SN5), editor mirror cutover and panel budgets (ED14, ED17); the rest is in docs/archive/2026-10-10/plans/ (docs/archive/2026-10-10/plans/world.md, docs/archive/2026-10-10/plans/audio.md, docs/archive/2026-10-10/plans/editor.md).

## 5. Donors

Donors: none kept; porting policy is AGENTS rule 4.

## 6. Slim

Slim: no new mixer, decode, thread, scene model or play path; one `Selection` type; freeze the W204/TVER layouts and the 1 cm warp bound.

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout file `../research/steals-2026-10-12/audio-ui-matrix.md`): UIX13 HUD pass with text, SN12 hit-kind cue table, SN13 occlusion lowpass, SN14 latency probe, UIX14 live gamepad and rumble, PF27 perf overlay and hitch catcher, GS37 BC7 and BC5 cook.

Open rows from `node tools/board-slice.mjs --ready --open` (prefixes WD, SN, ED, PLF, UIX); ticked rows dropped. No row is FOCUS Now, so none carries an Accept line here.
PLF rows are board-only tracking rows; their plan, docs/archive/2026-10-10/plans/platform-live-ai.md, is archived by another batch.

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| WD3 | Rome district map: streets, rooftops and a market for TV1 traversal | C | CT9, WD1, TV1 |
| WD5 | Day/night and weather cycle driving VX2 and RT4 | C | VX2 |
| WD7 | Cell publish gate: dual ahead-agent check before any cell goes live | C | WD1 |
| WD8 | Versioned mesh tiles: per-cell NV1 bakes stitched on shared edges | C | WD1, NV1 |
| WD9 | District traversal bake: sidecar edge markers into NV2 registries | C | WD3, TV1 |
| WD11 | Weather presentation contract: rain, wind and fog clocked by WD5 | C | WD5, VX2 |
| WD12 | Map assembly manifests: versioned cell manifest with provenance gate | C | WD1, CT9 |
| WD13 | City grammar and map import: road graph rewritten in Rust | C | WD1 |
| WD14 | Node-constraint collapse for arena tilings (propagate and collapse) | B | WD1 |
| WD15 | Staged arena-ground bake: ordered seeded passes for courtyard dirt | B | WD1, NV3 |
| SN4 | HRTF via Steam Audio behind the device wall | B | AU1; LATER |
| SN5 | Headphone 3D: an HRIR-sphere HRTF path as fixed tables | B | AU1; LATER |
| SN9 | Voice-bake vs voice-net boundary: typed tests proving bake-only refusal | B | ON8, AU1 |
| SN10 | Hit-sync numbers in the logs: per-hit loudness, clipping, sync delta | B | PQ8, AU1; LATER |
| SN11 | Repeat-last concealment over silent XRUN gaps instead of silence | B | CQ13 |
| ED14 | Browser real-edge wiring: AS1 EntryKind and SP5 place replace mirrors | B | ED3, AS1, SP5 |
| ED15 | Live-panel integration: hierarchy tab and inspector on one selection | B | ED1, ED2, ED6 |
| ED16 | Play parity on three tapes: child play and play_in match checkpoints | B | ED4, WIN1 |
| ED17 | Panel frame budgets: every panel at most 1 ms under PF2 | B | ED1, PF2 |
| ED18 | Newcomer re-measure: first fight within 15 minutes, live play-in-editor | B | ED4, ED5 |
| ED19 | Outliner at siege scale: 2,000 rows, search 50 ms, panel 1 ms | C | ED6 |
| PLF4 | Workshop data mods: publish and subscribe, schema and size checked | C | PLF2; LATER |
| PLF5 | Store build script: tools/ship.mjs with GM3 clean-folder check | B | GM3 (BLOCKED), OPT5 |
| PLF6 | Minidumps and a symbol store beside the WIN4 report | B | WIN4; LATER |
| PLF7 | Localization: Fluent string tables and a pseudo-locale | B | UI2; LATER |
| PLF8 | Cutscene sequencer: timeline tracks stored as data | B | ED1, AN1; LATER |
| PLF9 | Player benchmark: fixed fly-through reporting fps percentiles | B | OPT2 |
| PLF10 | Settings presets and safe start from RT0 plus a probe | B | RT0, UIX2 |
| UIX4 | Accessibility: subtitles-minimal, remap UI, contrast over UI1 palettes | B | UI1, CB1 |
| UIX6 | Settings persistence: versioned file with first-run adapter defaults | A | UIX2 |
| UIX7 | Settings dispatch wiring: OpenSettings rows into UIX2 staging | A | UIX1, UIX2 |
| UIX8 | Lobby scale proof: 32-slot list virtualized, 64-slot projection | B | UIX3, MP3 |
| UIX9 | Subtitle storm bound: drop-and-count queue under a full-roster (32) storm | B | UIX4, CB1; waits UIX4 |
| UIX10 | Remap-by-pad drill: full rebind with the keyboard unplugged | B | UIX2, WIN3 |
| UIX11 | Stale-beacon eviction: beacons older than K refresh periods | B | UIX3, MP3 |
| UIX12 | Tutorial-2 defense lesson: block, parry and riposte window | B | UIX5, GP1 |

## 8. Quality gates

1. Each area names the Core proof that un-parks it (section 1 and section 2).
2. World: zero tick hitches over the 9-cell walk. Audio: never in the hash, 0 audio-thread allocs. Editor: panel parity with the MCP verbs.
3. Golden unchanged: `node tools/proof.mjs` golden d57b5c8ae3096614, run twice on every row.

## 9. Session estimate

Session estimate: 61 size units over the 36 open rows (S=1, M=2, L=4, my weighting); 34 units are ready now. Planning figure, not measured.
