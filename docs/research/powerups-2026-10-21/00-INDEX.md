# INDEX: the GitHub powerups wave (2026-10-21)

Thirteen lanes, each one file, each one row per find, every donor read live
and every licence named. This is the merge: the lane files carry the detail
(12 rows each, with the donor paths, the cost and the kill line), this file
carries the decisions.

Rules are in `../powerups-2026-10-20/RULES.md`. The short version: no
invented numbers, no unverified claims, a licence read live, a tree check
before calling something a gap, and the board is not touched by the lanes.
Nobody committed anything; the lead commits by path.

| lane | file | owns |
|---|---|---|
| GH-L look and beauty | `GH-L-look.md` | crates/render, crates/material |
| GH-A animation feel | `GH-A-anim-feel.md` | crates/anim |
| GH-V VFX | `GH-V-vfx.md` | crates/render, crates/physfx |
| GH-K art pipeline | `GH-K-art-pipeline.md` | content/design, tools/blender |
| GH-N NPC AI | `GH-N-npc-ai.md` | crates/ai, crates/nav |
| GH-M AI-native and MCP | `GH-M-ai-native.md` | crates/endpoint, crates/provider |
| GH-E engine architecture | `GH-E-engine-arch.md` | crates/ecs, crates/save, crates/proof |
| GH-C content generation | `GH-C-content-gen.md` | crates/geo, crates/world |
| GH-U procedural audio | `GH-U-audio.md` | crates/audio |
| GH-W web delivery | `GH-W-web.md` | crates/web, crates/net |
| GH-D destruction | `GH-D-destruction.md` | crates/destruction, crates/physfx |
| GH-S simulation | `GH-S-simulation.md` | crates/nav, crates/physics |
| GH-Q quality and proof | `GH-Q-quality-proof.md` | crates/proof, crates/playtest, tools/blender |

## The three finds that matter most

**MMD is the ancestor of the look we are chasing.** `togechiyo/MMD_modoki`
(MIT, WebGPU) holds the MMD Standard toon rule: the shadow band is **one
texel** of the toon texture, unfiltered, and the light and dark boundary is
`selfMask * occlusionMask`. It also holds an inverted-hull outline with
**constant width in pixels** (the view normal divided by the viewport, times
`clip.w`), which our renderer can do with no cooked outline normal. This is
the donor for the 2005 Korean MMO look, and it is a shader we can read line
by line. New row **TOON1**.

**The art gate can stop being my opinion.** `mapbox/pixelmatch` (ISC) moved
to **OKLab HyAB with the sqrt folded out of the threshold test**, so it
ports to integer math, and it has `windowSize` and `ignoreMask`.
`jaehl/blockhash` (MIT) is a 16x16 block-average fingerprint with a Hamming
distance: one number for "the silhouette still reads". New row **AT1**:
the 30 m silhouette gate compares against a golden PNG and fails the build.

**A deterministic agent door needs a log.** `Ladvien/bevy_debugger_mcp`
(MIT OR Apache-2.0) writes the agent's run as a file the engine replays one
step per frame, and refuses a step that is not a recorded command, so the
file cannot carry a read-only query. `Chrrxs/robloxstudio-mcp` (MIT) makes a
mutating call succeed only when its result list is clean **and** a separate
readback agrees: transport 200 is not success. Our tree has **no door-call
log at all**. Rows PF9 and HX3, plus **DOOR-READONLY**.

## The corrections the lanes made to our own record

- The particle CPU pool is **60 000**, not the 4 096 the 2026-10-20 physics
  and VFX file states (`crates/render/src/particles.rs:19`).
- `crates/render/src/lens.rs` **does** exist (49 KB): the RL39 correction
  that says it does not is wrong.
- `renzora/engine` is **Apache-2.0 OR MIT**, not UNVERIFIED
  (`docs/editor.md:164,176` still says otherwise).
- `bokuweb/pixelmatch-rs` has **no LICENSE file and no licence field**, so
  it is all-rights-reserved and rejected for vendoring, and it still uses
  the old YIQ metric.
- The tree cites `snape/RVO2@main:Agent.cpp`; the file today is
  `src/Agent.cc`, and the obstacle half was never ported.
- The tree cites `AizenKai1001/lunaris@56cc7e30:examples/flock/flock.rs`; at
  that commit the flock example is a scene file, not a Rust file.
- `cargo-fuzz`'s README says **libFuzzer is Unix-only**: the coverage leg
  belongs in the Linux CI and the Windows leg stays seeded.
- The engine2040 doc says the armour pass shrinkwraps the plates. It does
  not: it measures the limb radius and copies the body's weights. The doc
  is corrected in `content/design/asset-pipeline.md`.

## Dead code in our own tree, found by the lanes

These are not research findings, they are holes, and every lane that found
one says the same thing: the thing exists and nothing calls it.

| what | where | why it matters |
|---|---|---|
| ORCA is exported and never called | `crates/nav/src/orca.rs:12` | a crowd at a war gate has no near-field solver |
| the flock never gets a neighbour | `crates/ai/src/depth/steer.rs:235` | `flock_wish(mx, mz, &[])` |
| physics computes SoundClass and Loudness, audio ignores it | `crates/physics/src/material.rs:184` | the hit sound seam is already half built |
| no particle compute pass at all | `crates/render/src/particles.rs` | zero `ComputePipeline` in any particle file |
| the live frame draws no particles or decals | crates/render | the trail strip is only called from its own test |
| the parsers unwrap on short bytes | `crates/assets/src/archive.rs:1193` | a truncated file is a crash, not a refusal |
| the rebuild order breaks a cycle silently | `crates/ecs/src/schedule.rs:231` | a stage builder would refuse instead |
| the raw wasm module is 97.7% of the size cap | crates/web | 1.2 MB on the wire, 4.15 MB raw |
| BT, GOAP, blackboard and ORCA have no caller outside their own crate | crates/ai, crates/nav | a war brain needs a capture point, and nothing in `crates/` knows one exists |

## New rows the lanes propose

TOON1 (MMD toon + constant width outline) · AT1 (the art gate compares a
golden PNG) · DOOR-READONLY (the read door ships without the write verbs) ·
GH-E2 (a mid-match restore is proved) · WORLD-HYDRO (terrain hydrology) ·
WORLD-SITE (a site planner and scatter exclusions) · WORLD-DUNGEON ·
LF6b (a screen-space shockwave) · LF6c (only the hit flash blooms) · CHR1
(painted faces) · GEO12 (heat-diffused skin weights) · and the audio lanes'
three: a swing sound from swing parameters, an adaptive score that reacts to
the war state, and a sound from a parameter set rather than a clip.

Every one of them carries a donor, a licence, a cost and a kill line in the
lane file it came from. The board merge is the lead's next move; the lanes
do not touch the 598 KB board.
