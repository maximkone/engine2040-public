# EVIDENCE — permission ledger

Simple rule: everything is free to use, port, and adapt. Ideas, patterns, algorithms are always free. Code from any repo — licensed, no-license, any license — is usable.

A row in this ledger is documentation, not a gate. It tracks what we took and gives attribution. Nothing here blocks a port.

## How it works

- **Ideas, patterns, algorithms, architectures** → free, no row needed, reimplement however you want.
- **Phone claims / verbal grants** → idea-only, but ideas are free so it doesn't matter.
- **Author-written declarations with a link** (Cargo.toml `license` field, author READMEs saying Apache-2.0, a LICENSE file URL) → record as a proof link. These are real permission.
- **Everything else** → reimplement. Ideas are free. No permission needed.
- **No repo is ever blocked for any reason.** The operator has full authority.

## Proof links recorded

Author-written declarations with a link. These are permission, not hearsay.

| Repo | Link | License | Status |
|---|---|---|---|
| evenfurther/pathfinding | `Cargo.toml` declares `Apache-2.0/MIT` https://github.com/evenfurther/pathfinding/blob/master/Cargo.toml | Apache-2.0/MIT | ADOPT — dependency |
| Kurble/bevy_mod_inverse_kinematics | `Cargo.toml` declares `MIT OR Apache-2.0` https://github.com/Kurble/bevy_mod_inverse_kinematics/blob/master/Cargo.toml (no root LICENSE file, Bevy-bound) | MIT/Apache-2.0 declared, file-unverified | ADOPT — pattern, ideas reimplemented, no vendor |
| schell/renderling | `LICENSE` chooser https://github.com/schell/renderling/blob/main/LICENSE (MIT `LICENSE-MIT` or Apache-2.0 `LICENSE-APACHE`, full texts 404) + donor Cargo `MIT OR Apache-2.0` | MIT/Apache-2.0 declared, FILE-PARTIAL | ADOPT — pattern, dependency gated until full texts land |
| BlondeBurrito/bevy_flowfield_tiles_plugin | `LICENSE` = dual MIT/Apache | MIT/Apache | ADOPT — pattern |
| Metta-AI/polyworld | `LICENSE` = MIT | MIT | ADOPT — pattern |
| onomihime/SpecialAgentPlugin | `LICENSE` = Apache-2.0 | Apache-2.0 | ADOPT — pattern |
| VIRUXE/rage-formats | `LICENSE` = Unlicense | Unlicense | ADOPT — pattern |
| bevyengine/bevy@v0.15.0 pbr_lighting.wgsl | `LICENSE-MIT` + `LICENSE-APACHE` @ v0.15.0 (tag listing verified 2026-09-25) | MIT/Apache-2.0 | ADOPT — pattern (SL7 pin; perceptualRoughnessToRoughness, D_GGX, V_SmithGGXCorrelated, F_Schlick, fresnel, Fd_Burley, F_AB, specular_multiscatter, directional_light) |
| bevyengine/bevy@v0.15.0 pbr_functions.wgsl | same as above | MIT/Apache-2.0 | ADOPT — pattern (SL7 pin; calculate_diffuse_color, calculate_F0; inverse_gamma NOT in Bevy v0.15.0 — sRGB EOTF reimplemented from IEC 61966-2-1) |
| bevyengine/bevy@v0.15.0 color_operations.wgsl | same as above | MIT/Apache-2.0 | ADOPT — pattern (SL7 pin; inverse_gamma) |
| bevyengine/bevy@v0.15.0 tonemapping_shared.wgsl | same as above | MIT/Apache-2.0 | ADOPT — pattern (SL7 pin; RRTAndODTFit, ACESFitted) |
| algesten/str0m@0.23.1 crates/is | MIT OR Apache-2.0 declared (Cargo.toml, full text 404) | MIT/Apache-2.0 declared, file-partial | ADOPT — pattern (ON24 ICE shapes, reimplemented, no dep; SCTP/DTLS/media refused) |
| webrtc-rs/turn@0.5.4 src | MIT/Apache-2.0 declared (Cargo.toml) | MIT/Apache-2.0 declared | ADOPT — pattern (ON25 Allocate/Refresh/Permission/ChannelBind shapes; tokio-full + ring refused) |
| coturn/coturn@master | BSD-3-Clause (LICENSE read live) | BSD-3-Clause | ADOPT — behavior reference oracle only (ON25; no C vendored) |
| bevyengine/bevy | `LICENSE-MIT` + `LICENSE-APACHE` | MIT/Apache | ADOPT — pattern |
| wgpu | MIT/Apache | MIT/Apache | ADOPT — pattern |
| mrDIMAS/hrtf | MIT (planner live read 2026-09-26) | MIT | ADOPT — pattern (SN7: HRIR-sphere + ITD/ILD shapes reimplemented std-only in `crates/audio/src/hrtf.rs`; Steam Audio path killed) |
| maximkone/flax-mcp | MIT License, Copyright (c) 2026 Flax Game Studio (LICENSE read live) | MIT | ADOPT — pattern (GA6: EqCoverSpotGenerator, EqCoverAlignmentTest, EqFlankTest, EqTactics + 16 siblings clean-room rewritten as integer math in `crates/ai/src/eqs.rs`; GP9: CombatHitResolver, CombatStaminaCore, CombatPoiseCore, CombatComboChainCore clean-room rewritten as integer math in `crates/combat/src/depth.rs`) |
| gschup/ggrs | Apache-2.0 (LICENSE-APACHE read live) | Apache-2.0 | ADOPT — pattern (RC13: SaveGameState/LoadGameState/AdvanceFrame discipline reimplemented over SimTick in `crates/net/src/rollback.rs`; no vendor bytes) |
| cBournhonesque/lightyear | MIT OR Apache-2.0 (LICENSE-MIT + LICENSE-APACHE at root, verified via repo API; see `docs/archive/2026-10-08/plan/online.md` lightyear row) | MIT/Apache-2.0 | ADOPT — pattern (RC14: server-authoritative replication shape reimplemented in `crates/net/src/replicate.rs`; no vendor bytes) |
| polyanya (vleue) | GitHub API: MIT | MIT | ADOPT — pattern |
| landmass (andriyDev) | GitHub API: Apache-2.0 | Apache-2.0 | ADOPT — pattern |
| kryptic-sh/crcbl | GitHub API: MIT | MIT | ADOPT — pattern |
| epokengine/epok-engine | GitHub API: MIT | MIT | ADOPT — pattern |
| weldengine/weld | GitHub API: MIT | MIT | ADOPT — pattern |
| hord3 (Pedro-W21) | GitHub API: Apache-2.0 | Apache-2.0 | ADOPT — pattern |
| Kodrin/GPU-FlowField | GitHub API: MIT | MIT | ADOPT — pattern |
| yohash/ECSContinuumCrowds | GitHub API: MIT | MIT | ADOPT — pattern |
| aymnkadymy-hub/mojo--gaming-engine/Kawn | GitHub API: MIT | MIT | ADOPT — pattern |
| brusher (BrianWiz) | `LICENSE` = MIT | MIT | ADOPT — pattern |
| meshoptimizer (zeux) | `LICENSE` = MIT | MIT | ADOPT — pattern |
| backroll-rs (HouraiTeahouse) | API: ISC | ISC | ADOPT — pattern |
| FyroxEngine/Fyrox | API: MIT | MIT | ADOPT — pattern |
| avianphysics/avian | API: Apache-2.0 | Apache-2.0 | ADOPT — pattern |
| renzora/engine | API: Apache-2.0 | Apache-2.0 | ADOPT — pattern |
| Easy-Mixamo | README: MIT | MIT | ADOPT — tool |
| cadcore | API: MIT | MIT | ADOPT — pattern |
| curvo | API: MIT | MIT | ADOPT — pattern |
| recastnavigation/recastnavigation (NV3) | License.txt read live (Zlib) | Zlib | ADOPT — pattern, reimplemented in `crates/nav/src/tiles.rs` (voxel raster + tile rebake shapes; no C vendored) |
| janhohenheim/rerecast (NV3) | license-apache.txt + license-mit.txt at root, both listed | Apache-2.0/MIT | ADOPT — pattern, reimplemented in `crates/nav/src/tiles.rs` (dynamic tile versioning shapes; no code vendored) |
| MikkoParkkola/nab | https://github.com/MikkoParkkola/nab | MIT declared, file-unverified (SHA UNVERIFIED, steal-wave36:54-69) | ADOPT — pattern, reimplemented substring rules in `crates/endpoint/src/injection/mod.rs`, never vendored |
| eldruin/wyhash-rs | `Cargo.toml` declares `MIT OR Apache-2.0` https://github.com/eldruin/wyhash-rs/blob/master/Cargo.toml (reference: wangyi-fudan/wyhash `LICENSE` = The Unlicense https://github.com/wangyi-fudan/wyhash/blob/master/LICENSE) | MIT/Apache-2.0 + Unlicense | ADOPT — pattern (RC28: the `wyrng` wyrand shape reimplemented std-only in `crates/core/src/rng_wyhash.rs`, upstream vectors pinned in the module tests; no vendor bytes, no dependency) |

## Phone grants (ideas are free)

| Repo | Date | Scope |
|---|---|---|
| jasonsuhari/godot-mcp | 2026-09-23 | full use |
| LeeSinLiang/godot-mcp | 2026-09-23 | full use |
| flax-engine-mcp / leto-mcp | 2026-09-23 | full use |
| kvick-games/UnrealMCP | 2026-09-23 | full use |
| chongdashu/unreal-mcp | 2026-09-23 | full use |

Ideas from these repos are always free to use. If any code was ported, attribution is noted above.

## Self-owned donor repos

Operator owns: donor-flax-mcp, flax-mcp-org, old-flax-mcp, flax-mcp-old, flax-mcp-sponex, FlaxMCP, flaxmcp-veradictus.

Full use. Own expression only — third-party Flax EULA bytes and UE/Unity vendor code inside stay idea-only.

## Content track M1 — incoming assets

All content is usable. Attribution noted where known.

| File | Source | License |
|---|---|---|
| `fixtures/khronos/Triangle.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 |
| `fixtures/khronos/TriangleWithoutIndices.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 |
| `fixtures/khronos/AnimatedTriangle.gltf` | KhronosGroup glTF-Sample-Models | CC0-1.0 |
| `content/incoming/studio_small_09_1k.hdr` | Poly Haven | CC0-1.0 |

## Content track A — locomotion bake

Synthetic, no donor bytes. Quaternius Universal Animation Library (CC0). 100STYLE (CC BY 4.0, motions only).

## Content track B — fighters (CT7, wave 23)

Source: `https://opengameart.org/content/modular-rpg-characters` by System G6
(Qoma), badge CC0 + "License(s): CC0", read live 2026-09-25. 16,600,670 B
`modular_rpg_characters.7z` + 1,239,677 B IK-rig reference (reference only,
no bytes shipped). 4 cooked variants (hero + 3 opponents) under
`content/fighters/`: bodies + 14 clips each, CT1 retarget + CT2 cook + AS1
bundles. Per-file SHAs in `content/fighters/manifest.json`. Bundles excluded
from git (reproducible via cook); sources + sidecars + previews committed.
Quaternius NOT used here: `quaternius.com/license.html` now serves QAL v1.0
(2026-08-28, no-redistribution-as-assets, non-retroactive) with downloads
behind itch.io checkout — nothing taken, no ambiguity shipped.

| Asset | License |
|---|---|
| `content/fighters/` bodies, clips, atlases, sidecars | CC0-1.0 (via OGA System G6, read live 2026-09-25) |

## Content track C — arena kit (CT8, wave 23)

All kit files original (`content/arena/tools/gen.mjs`, author engine2040 CT8 lane), cooked
via CT2/A2/AS1; per-file SHA-256 in `content/arena/manifest.json`. Licenses
read live 2026-09-25: Poly Haven CC0-1.0, ambientCG CC0-1.0, Kenney CC0 public
domain. Quaternius REFUSED for git (QAL v1.0, see track B). Bundle (93 MB)
excluded from git (reproducible via `content/arena/tools/gen.mjs` + proof).

| Asset | License |
|---|---|
| `content/arena/` kit, textures, HDRIs, placement | CC0-1.0 (originals, engine2040) |

## Content track A — CT4 AI modeling loop

Blender 5.2 via mcp-for-blender. Tool is GPL; model output is artist bytes, never tool code.

## Notes

- No repo is ever blocked. No license restriction applies.
- Ideas are free. Code is free. Port freely.
- Attribution is nice, not a gate.
- DOC (ADV45 proposal 3, 2026-09-26; superseded wave 46 by EV11 tick): `target/evidence/a6/report.txt` and `target/evidence/a7/report.txt` as cited on the FINISH-RICH A6/A7 lines were transcribed human text from live test output, not machine-verified receipts. EV11 (ticked wave 46) added digests+seal at write time beside the a6/a7/a9 writers with typed verify checks; wiring verify into proof/finish is the follow-up. Standing holds until that wiring lands.
