STEAL-SCOUT block 9 card, read 2026-10-18. Donor: rei-kurzweil/mittens (Vulkan + OpenXR engine in Rust; avatar / vtuber / VR focus, 1.6k files).
Donor sha 774c51d61303771c7806cd0083dc56114b74d56c (main, 2026-10-18). Read: file bodies fetched live today (component/mirror.rs, docs/spec/mirror_system.md, docs/spec/secondary_motion_system.md, docs/spec/t_pipeline-spring_bones.md, examples/camera-constraint.mms, tree, LICENSE). No build, no run. No bytes vendored: ideas only.
Gates: golden d57b5c8ae3096614 / 2aca9338670aa0cb are P2P on every row below; every row is presentation or camera, never the hashed tick.
Skill match: `render-lighting` (mirror is a light path), `anim-combat` (secondary motion, camera boom).

## What it is

A document-heavy engine: a real runtime plus a serious `docs/spec`, `docs/analysis` and `docs/bugs` set that records why each system is shaped the way it is. The code is avatar-centric (bisket bones, IK, spring chains, VR), but three of its systems are general and cheap: planar mirrors, secondary motion, and a transform pipeline that can fork and drop a channel.

## Licence

`LICENSE` root, line 1 (read live): `MIT License`, `Copyright (c) 2026 Rei Evans <ultr7a@gmail.com>`. GitHub licence API: SPDX `MIT`.

## What we have (checked, not guessed)

- No planar mirror or reflection probe in `crates/render/src` (grep `mirror|planar|reflection_probe` -> prose in comments only).
- A verlet rope exists in the float layer: `crates/physfx/src/tether.rs:1-5` (rope between two points, spring-damper pull), `lib.rs:35-39` (spear tether, carry ragdoll).
- No spring *bone* chain that follows a skeleton (grep `spring|tassel|cape|hair|chain` in `crates/physfx/src` -> the tether only).
- A single frame camera per draw list: `crates/render/src/present.rs:449-457`.

## Steals (ranked by value per hour)

1. **Planar mirror as one component plus one system** | mittens@774c51d61303:src/engine/ecs/component/mirror.rs:4-29 (`MirrorComponent { quality }`, plane = the nearest ancestor transform's XY plane with +Z as the reflection normal, `quality` clamped `64..=2048`), docs/spec/mirror_system.md:34-38 (`MirrorSystem` every frame: find mirrors, resolve transform and surface, derive plane, build the reflected camera and an off-axis projection, publish a texture, hand the renderer a mirror capture), docs/bugs/mirror-*.md (their own trap list: orientation, tracking, occlusion behind the surface) | MIT | our file: `crates/render/src/present/gpu_present/` beside steal 1 of `fyrox.md` (a camera render target is the mechanism; the mirror is the first consumer) |
   The arena has polished surfaces (blade, bronze shield, wet stone) and a sun. A mirror is the cheapest way to make metal read as metal without a reflection probe. Build it on the render-target camera from `fyrox.md`, not as a separate device path. Reuse their bug list as our acceptance cases. Kill: if the off-axis projection plus texture publication needs more than ~300 lines and no shipped scene has a reflecting surface, drop it and keep only the render-target camera.

2. **Secondary motion: spring chains after IK, with an explicit frame order** | mittens@774c51d61303:docs/spec/t_pipeline-spring_bones.md:3 (chain-level `SecondaryMotionSystem`, modeled on VRM 1.0 SpringBone: ordered joint references, fixed-length Verlet steps, writes joint rotations *after* AvatarControl and IK, a virtual endpoint so the last bone rotates without changing the glTF topology), :20 (deliberately excluded at first: colliders, angular limits, stretch, grabbing, parameter curves), docs/spec/secondary_motion_system.md:12-13 (continuously simulated; must not depend on an unrelated transform update to become visible) and :41-49 (required frame order: primary pose writes -> propagate -> secondary motion -> propagate only the dirty chain roots -> refresh skinned palettes -> render) | MIT | our file: `crates/anim` (a layer after the sampler) or `crates/physfx` (float presentation layer) |
   Tassels, cloth strips and a sash that trails a step are pure look and pure presentation: safe next to the hash law. The value is not the Verlet loop (we have that in `tether.rs`) but the *order rule* and the *dirty-root propagation*: simulate after the pose, then re-propagate only the subtree the chain moved. Kill: if the chain must run inside the hashed tick to look right, keep it presentation-only and off in the duel camera; never let it reach the golden.

3. **Fork the transform pipeline and drop one channel** | mittens@774c51d61303:examples/camera-constraint.mms:33-48 (`TOP.fork_trs().drop("rotation")` between input and the camera: position and scale pass through, rotation is stripped, so the eye follows without mouse-look), with docs/how_to/avc-head-neck-constraint-diagram.md as the fuller case | MIT | our file: `crates/anim` camera boom / `crates/render` duel camera |
   Our duel camera is a boom with a yaw/pitch pair. The general idea is one operator: a fork that can drop a named channel of a TRS stream. It gives "follow the fighter's position, hold my own yaw", "keep the boom's pitch while the target jumps", and a head/neck constraint for free. Small and data-shaped. Kill: if the boom already carries the needed channel split as two fields, skip the operator; do not build a pipeline for one caller.

Not worth it: their `mittens-query` crate (a CSS-like and an mmq selector language over the component tree, `crates/mittens-query/src/{css,mmq}/parser.rs`) overlaps our `scene.query` verbs and their kinds; and their `meow-meow-script` VM is a whole language we do not need. The valuable part of the VM is its `block_effect_analyzer.rs` (which blocks are pure vs impure) — file it as a later read for the day a mod script reaches the door.
