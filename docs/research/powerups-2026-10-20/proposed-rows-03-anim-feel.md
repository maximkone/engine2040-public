# Proposed rows — animation and feel (lane 03)

From `03-anim-feel-steals.md`. The board is not edited here: the lead merges `docs/SPRINT*.md`.
Row ids use the `AMX-` prefix so they cannot clash with a live board id.

Two checks ran before these rows were written (full text further down):
(a) NS19 BVH intake — no C++ dependency, a Rust-native parser in `crates/assets` instead of assimp;
(b) AM23 rotation-additive — the exact quaternion math, named against the live `crates/anim` API.

Every "already in the tree" rejection was verified by grep on 2026-10-20 with file and line, so the
anim gaps in any comparison table are editor-UX gaps (door verbs, graph preview, node editor), not
engine gaps. The four real code gaps left are NS19 (BVH intake), EG40 and LF12 (sockets), AM23 with
C14 (rotation additive) and LAI3 (live lip sync source).

## Rows

### AMX-1 — anim: BVH motion intake as a std-only parser in assets

**What it is.** A hand-written BVH text parser in `crates/assets` that reads HIERARCHY, JOINT, End
Site, CHANNEL and MOTION into a joint tree with per-frame local transforms, then hands the clip to
the existing retarget cook. No C++ and no new dependency; assimp is read as the format spec only.

**Donor.** assimp/assimp, `code/AssetLib/BVH/BVHLoader.cpp`,
https://github.com/assimp/assimp/blob/master/code/AssetLib/BVH/BVHLoader.cpp — BSD-3-Clause
(`LICENSE` read live 2026-10-20) — read 2026-10-20. Ideas only: no assimp bytes, no assimp crate,
no build.rs, no FFI.

**Closes.** NS19 (motion intake as BVH through the CT4 loop).

**Owned paths.** `crates/assets/src/bvh.rs` (new), `crates/assets/src/clip_intake.rs` (intake hunk),
`crates/assets/src/lib.rs` (module line), `crates/assets/Cargo.toml` (none: no new dependency).
The board row says "intake hunks beside `crates/anim/src/clip.rs`"; `crates/anim` is std-only and
IO-free, so the parse function stays pure text in assets and only the rotation-lane clip type, if
the lead wants it, touches `crates/anim/src/clip.rs`.

**Accept.** `cargo test -p engine-assets --lib bvh` prints
`test bvh_intake_clip_retargets_and_plays_x2_hashed ... ok` and the run ends with
`test result: ok. 5 passed; 0 failed`. Heavy run on the runner:
`node tools/gh-check.mjs cargo test -p engine-assets --lib bvh`.

**Kill line.** Kill if the parser needs any dependency, any `unsafe`, or any float in the tick path,
or if one intake clip does not retarget and play twice with the same hash.

**Cost.** M.

### AMX-2 — anim: rotation-additive hit flinch on the real skeleton

**What it is.** The existing additive layer applies position offsets only and leaves rotations on
the base (`crates/anim/src/layers.rs:215`, rotation line `:242`). This row adds a rotation lane:
per-joint delta quaternions on the base rotation, blended by mask times gain, with the clip data
untouched.

**Donor.** mrdoob/three.js, `src/animation/AnimationUtils.js`, `makeClipAdditive`,
https://github.com/mrdoob/three.js/blob/dev/src/animation/AnimationUtils.js — MIT (`LICENSE` read
live 2026-10-20) — read 2026-10-20. Second reading: nfrechette/acl,
`includes/acl/core/additive_utils.h` (MIT), https://github.com/nfrechette/acl/blob/main/includes/acl/core/additive_utils.h
(read 2026-10-20). Both shape only.

**Closes.** AM23 (rotation-additive hit flinch) and the rotation-additive half of C14.

**Owned paths.** `crates/anim/src/layers.rs`, `crates/anim/src/hit_react.rs`,
`crates/anim/src/graph.rs` (quaternion helpers next to `nlerp` at `graph.rs:773`),
`crates/anim/src/additive.rs` (caller check only).

**Accept.** `cargo test -p engine-anim --lib hit_react_rot` prints
`test hit_react_rot_weight_zero_equals_base_bytes ... ok`,
`test hit_react_rot_two_strikes_sum_within_1e5 ... ok` and the run ends with
`test result: ok. 4 passed; 0 failed`. Then `node tools/product-check.mjs` prints an `arena view:`
line with `react_head_cm_max` at 8 or more and `pops` not above the value before the row.

**Kill line.** Kill if weight 0 does not return the base bytes exactly, if two strikes do not sum
within 1e-5, or if the golden hash moves.

**Cost.** S.

### AMX-3 — game: weapon and shield sockets on hand bones

**What it is.** A socket is a joint plus a local offset. The weapon and shield meshes leave the
body mesh at cook time and ride the hand joint each frame; a dropped weapon leaves its socket and
gets exactly one physics body.

**Donor.** mrdoob/three.js, `src/core/Object3D.js`, `attach( object )`,
https://github.com/mrdoob/three.js/blob/dev/src/core/Object3D.js — MIT (`LICENSE` read live
2026-10-20) — read 2026-10-20. Shape only: the re-parent that keeps the world transform.

**Closes.** EG40 (weapons and shields on sockets) and the grip half of LF12 (the weapon hand
reaches the grip socket).

**Owned paths.** `crates/game/src/fighter_look.rs` (attachment hunk), `crates/game/src/fighter_bind.rs`,
`crates/assets/src/body_import.rs` (mesh split hunk), `crates/anim/src/fbik.rs` (grip target hunk
only).

**Accept.** `cargo test -p engine-game --lib attach_socket` prints
`test attach_socket_sword_follows_the_hand_within_1mm ... ok` and
`test attach_socket_drop_leaves_one_physics_body ... ok` and the run ends with
`test result: ok. 3 passed; 0 failed`. The IK half: `cargo test -p engine-anim --lib grip` prints
`test grip_two_hand_target_is_reached_within_1mm ... ok`.

**Kill line.** Kill if the sword is not within 1 mm of the hand joint for 60 frames, if a dropped
sword does not leave its socket for exactly one physics body, or if the golden hash moves.

**Cost.** M for the row (the socket hunk alone is S; the mesh split at the cook is the rest).

### AMX-4 — anim: live lip sync from phoneme segments to viseme curves

**What it is.** Phoneme segments with start and end times become a per-frame viseme weight matrix,
deterministic and model-free, ready to drive the existing viseme clip. This is the live curve
source LAI3 needs; `crates/anim/src/lipsync.rs` today only bakes offline.

**Donor.** OpenFaceFX/OpenFaceFX, `src/openfacefx/coarticulation.py` (`build_viseme_curves`),
https://github.com/OpenFaceFX/OpenFaceFX/blob/main/src/openfacefx/coarticulation.py and
`src/openfacefx/data/jali_rules.json`,
https://github.com/OpenFaceFX/OpenFaceFX/blob/main/src/openfacefx/data/jali_rules.json — MIT
(`LICENSE` read live 2026-10-20) — read 2026-10-20. Shape only: the phoneme to weight matrix math
and the plain-data rule table.

**Closes.** LAI3 (live lip-sync).

**Owned paths.** `crates/anim/src/lipsync.rs`, `crates/anim/src/viseme.rs`.

**Accept.** `cargo test -p engine-anim --lib lipsync` prints
`test lipsync_viseme_timing_within_60ms_of_the_reference_track ... ok` and the run ends with
`test result: ok. 3 passed; 0 failed`.

**Kill line.** Kill if viseme timing is not within 60 ms of the reference track, or if the bake
needs a float in the tick or a model download.

**Cost.** M.

### AMX-5 — anim: mouth-shape rule table per phoneme

**What it is.** A phoneme maps to one mouth shape with an open and a closed variant, plus the
per-rule timing, written as our own table. Paired with AMX-4 it replaces the missing curve source
with a table we own.

**Donor.** DanielSWolf/rhubarb-lip-sync, `rhubarb/src/animation/animationRules.cpp`,
`rhubarb/src/animation/targetShapeSet.cpp`, `rhubarb/src/exporters/JsonExporter.cpp`,
https://github.com/DanielSWolf/rhubarb-lip-sync/tree/master/rhubarb/src/animation — MIT
(`LICENSE.md` read live 2026-10-20: "Rhubarb Lip Sync is released under the MIT license") — read
2026-10-20. Shape only: the phoneme to shape rule shape and the shape plus time wire shape.

**Closes.** LAI3 (second donor for the same row).

**Owned paths.** `crates/anim/src/viseme.rs`.

**Accept.** `cargo test -p engine-anim --lib viseme` prints
`test viseme_same_phoneme_gives_the_same_shape_twice ... ok` and
`test viseme_shape_table_has_no_duplicate_phonemes ... ok` and the run ends with
`test result: ok. 2 passed; 0 failed`.

**Kill line.** Kill if one phoneme maps to two shapes in two bakes, or if the table needs a
hand-tuned value per fighter.

**Cost.** S.

### AMX-6 — anim: state machine and blend tree as JSON data

**What it is.** The shape of a state machine definition and a blend tree written as data the door
can read and reload, so the editor can draw nodes without a Rust rebuild. The door verbs
`anim.graph.set|get|undo` and `anim.preview` already exist in `crates/endpoint/src/packs/anim.rs:34-40`.

**Donor.** PistonDevelopers/skeletal_animation, `src/blend_tree.rs` and `src/controller.rs`,
https://github.com/PistonDevelopers/skeletal_animation/blob/master/src/blend_tree.rs — MIT
(`LICENSE` read live 2026-10-20) — read 2026-10-20. Shape only, Rust-native and JSON-based.

**Closes.** EG18b (one `StateMachineDef` per slot via `Playback`).

**Owned paths.** `crates/anim/src/state_machine.rs`, `crates/endpoint/src/packs/anim.rs`.

**Accept.** `cargo test -p engine-anim --lib state_machine` prints
`test state_machine_def_from_json_round_trips ... ok` and
`cargo test -p engine-endpoint --lib anim` prints
`test anim_graph_round_trips_through_the_door_with_the_same_bytes ... ok`.

**Kill line.** Kill if the JSON graph cannot round-trip through `anim.graph.set` and
`anim.graph.get` with the same bytes, or if it forces a schema version bump.

**Cost.** S.

### AMX-7 — anim: additive reference-pose helper as a second opinion (research only)

**What it is.** A second reading of the additive delta shape (delta against a reference frame) so
AMX-2 has one cross-check. No row is proposed for the board: read it, port later if AMX-2 stalls.

**Donor.** nfrechette/acl, `includes/acl/core/additive_utils.h`,
https://github.com/nfrechette/acl/blob/main/includes/acl/core/additive_utils.h — MIT (`LICENSE`
read live 2026-10-20, SHA 4767cb746dd76ac401e38c7621d2efa5d5d8e99e) — read 2026-10-20.

**Closes.** AM23 and C14, as a second opinion only (no board row).

**Owned paths.** none.

**Accept.** Not run. The check is that AMX-2's delta math agrees with the acl reference-pose shape
before AMX-2 ticks.

**Kill line.** Kill if the acl shape differs from the ozz shape cited on the AM23 row; then one
donor is enough and this row dies.

**Cost.** S.

### AMX-8 — anim: crowd LOD that shares one animation texture (research only)

**What it is.** The idea of one baked animation texture shared by several LOD tiers, plus socket
attachments on crowd instances and stride-matched playback. No board row proposed: the licence is
unknown, so no code may be ported.

**Donor.** natepac/crowdbake, `README.md`,
https://github.com/natepac/crowdbake — licence UNKNOWN: the repo has no LICENSE file and
`package.json` says `"private": true` with no licence field (both read live 2026-10-20) — read
2026-10-20. Ideas only.

**Closes.** AM13 (animation LOD), AM18 (thousand-agent crowd budget), AM22 (crowd readability at
LOD) as ideas only (no board row).

**Owned paths.** none.

**Accept.** Not run. AM13 still needs its own 64-fighter timing receipt, which is a measurement.

**Kill line.** Kill if the licence stays unknown; then the ideas die with it and our own `lod.rs`
and `crowd_scale.rs` stay the only paths.

**Cost.** M.

### AMX-9 — anim: live pose tracker to servo targets (research only)

**What it is.** The idea of a live landmark list mapped to named joint targets with a confidence
gate, feeding the PX1 servos. No board row proposed: a model download and a foreign runtime are
not a crate dependency.

**Donor.** google-ai-edge/mediapipe, `LICENSE`,
https://github.com/google-ai-edge/mediapipe — Apache-2.0 (`LICENSE` read live 2026-10-20, SHA
0e03e3911e0ac02c72ed527b34efe250f0fb9bef) — read 2026-10-20. Ideas only, never a dependency.

**Closes.** NS8 (live motion tracker driving PX1 servos) as ideas only (no board row).

**Owned paths.** none.

**Accept.** Not run. NS8 stays open until the offline LMM1 path is measured.

**Kill line.** Kill if the tracker needs a model download or a float in the tick.

**Cost.** L.

## Patch-ready spec per row

Code is blocked while `crates/anim` is held by the AM24 landing agent and `crates/game` and
`crates/assets` by the WEB6 landing agent. Each spec below is ready to apply the minute the crate
frees.

### AMX-1 (crates/assets)

- New file: `crates/assets/src/bvh.rs`.
- Structs: `pub struct BvhClip { pub names: Vec<String>, pub parents: Vec<i32>, pub frames: u32,
  pub frame_time_s: f32, pub tracks: Vec<BvhTrack> }`,
  `pub struct BvhTrack { pub joint: usize, pub kind: BvhChannel, pub times: Vec<f32>,
  pub values: Vec<[f32; 4]> }`, `pub enum BvhChannel { Position, Rotation }`,
  `pub enum BvhError { BadHeader, BadHierarchy, BadChannel, BadMotion, TooManyJoints,
  TooManyFrames, NonFinite }`.
- Functions: `pub fn parse_bvh(text: &str) -> Result<BvhClip, BvhError>` (pure, no IO),
  `pub fn bvh_to_qclip(clip: &BvhClip, map: &BoneMap) -> Result<QClip, BvhError>`.
- Module line in `crates/assets/src/lib.rs`: `pub mod bvh;` (std-only, no feature; the file read
  stays in the import lane caller so the headless default graph stays std-only).
- Tests: `bvh_parses_a_single_root_hierarchy`, `bvh_reads_frame_count_and_frame_time`,
  `bvh_rotation_channels_become_quaternion_tracks`,
  `bvh_intake_clip_retargets_and_plays_x2_hashed`, `bvh_bad_input_is_refused_and_names_the_cause`.
- Proof: `cargo test -p engine-assets --lib bvh` (runner: `node tools/gh-check.mjs cargo test -p
  engine-assets --lib bvh`).
- Golden: `d57b5c8ae3096614` must stay. `cargo run -p engine2040 --release` prints the same hash
  twice. The parser is cook-time only and never reaches the tick.

### AMX-2 (crates/anim)

- New file: none. New items:
  `crates/anim/src/graph.rs`: `pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4]`,
  `pub fn quat_conj(q: [f32; 4]) -> [f32; 4]`, `pub fn quat_normalize(q: [f32; 4]) -> [f32; 4]`
  (next to `nlerp` at `graph.rs:777`).
  `crates/anim/src/layers.rs`: `pub fn apply_additive_rotation(base: &Pose, add: &Pose,
  mask: &MaskTable, gain_q8: u16, out: &mut Pose) -> Result<(), GraphError>` and the rotation lane
  inside `apply_additive` (`layers.rs:215`), which stays identity-safe for today's callers because
  they write identity rotations (`crates/anim/src/hit_react.rs:276`).
  `crates/anim/src/hit_react.rs`: `fn delta_rotation(dir: HitDir, joint: usize, scale: f32)
  -> [f32; 4]` and the rotation write inside `write_additive` (`hit_react.rs:261`), plus
  `pub const FLINCH_ROT_PEAK_DEG: f32`.
- The math, written out:
  1. The flinch delta is authored against a reference pose (the bind pose). For a quaternion
     reference `r` and a target `t`, the donor builds the delta as `delta = t * conj(r)`: the
     reference is conjugated and multiplied onto the target (`AnimationUtils.js`, quaternion
     branch). That is the order to use: target on the left, conjugate of the reference on the
     right.
  2. The delta is applied to the live base rotation by right-multiplying, `base * delta`, because
     the delta lives in the bone's own local frame and the skeleton composes parent to child.
  3. The weight is the same scale the position path already uses: `s = mask_weight * gain_q8 /
     65280` (`crates/anim/src/layers.rs:245`), and the result is
     `out_rot = nlerp(base_r, base_r * delta, s)` using the shortest-arc `graph::nlerp` at
     `crates/anim/src/graph.rs:777`, the same one the override path uses (`layers.rs:170`).
  4. Sign and scale: normalize `delta` and negate it when `delta.w < 0`, so the arc is the short
     way; a zero delta must return the base bytes exactly.
  5. Blend-in, hold and blend-out stay with the existing envelope `envelope(elapsed)`
     (`crates/anim/src/hit_react.rs:228`), which the game layer already calls
     (`crates/game/src/fighter_look.rs:24`). The sim window stays 6 ticks (`HIT_REACT_TICKS`);
     the about 450 ms readable window is a view-side envelope, not a tick change.
  6. Clip data is never touched: the flinch is a delta pose built per tick from
     `(HitDir, impulse_q8)`, applied through the layers additive path. Clips stay immutable and the
     golden hash does not move.
- Tests: `hit_react_rot_weight_zero_equals_base_bytes`,
  `hit_react_rot_two_strikes_sum_within_1e5`, `hit_react_rot_delta_is_shortest_arc`,
  `hit_react_rot_replay_is_identical_twice`.
- Proof: `cargo test -p engine-anim --lib hit_react_rot` (runner: `node tools/gh-check.mjs cargo
  test -p engine-anim --lib hit_react_rot`), then `node tools/product-check.mjs` for the
  `arena view:` line.
- Golden: `d57b5c8ae3096614` must stay. View-only path; `cargo run -p engine2040 --release` prints
  the same hash twice.

### AMX-3 (crates/game, crates/assets)

- New file: none.
  `crates/game/src/fighter_look.rs`: `pub struct Attachment { pub joint: usize, pub offset_m:
  [f32; 3] }`, `pub enum AttachError { NoSuchJoint, NoSocket, Dropped }`, the field
  `attachments: Vec<Attachment>` on `FighterLook`, and
  `pub fn attach_socket(&mut self, slot: usize, joint: usize) -> Result<(), AttachError>` plus
  `pub fn socket_world_pose(&self, slot: usize) -> Option<[f32; 12]>`.
  `crates/assets/src/body_import.rs`: `pub fn split_weapon_meshes(body: &MeshAsset) ->
  Result<Vec<WeaponMesh>, BodyImportError>` and `pub struct WeaponMesh { pub name: String,
  pub joint: usize, pub offset_m: [f32; 3] }`.
  `crates/game/src/fighter_bind.rs`: bind the split meshes to the hand joints and hand the drop to
  one physics body.
  `crates/anim/src/fbik.rs`: the grip target reads the socket pose (existing `GripMode::TwoHand`
  at `fbik.rs:399-407`).
- Tests: `attach_socket_sword_follows_the_hand_within_1mm`,
  `attach_socket_drop_leaves_one_physics_body`,
  `attach_socket_missing_joint_is_refused_and_names_the_joint`, and in anim
  `grip_two_hand_target_is_reached_within_1mm`.
- Proof: `cargo test -p engine-game --lib attach_socket` and `cargo test -p engine-anim --lib grip`
  (runner: `node tools/gh-check.mjs cargo test -p engine-game --lib attach_socket`).
- Golden: `d57b5c8ae3096614` must stay. Presentation only; the drop body is a view-side physics
  body, never a hashed event.

### AMX-4 (crates/anim)

- New file: none.
  `crates/anim/src/lipsync.rs`: `pub struct PhonemeSegment { pub phoneme: String, pub t0_s: f32,
  pub t1_s: f32 }`, `pub fn build_viseme_curves(segments: &[PhonemeSegment], fps: f32) ->
  Result<VisemeClip, LipsyncError>`, `pub fn phoneme_to_viseme(phoneme: &str) -> Option<Shape>`.
  `crates/anim/src/viseme.rs`: the shape enum stays the owner; the table lands with AMX-5.
- Tests: `lipsync_viseme_timing_within_60ms_of_the_reference_track`,
  `lipsync_phoneme_maps_to_one_shape`, `lipsync_bake_is_replay_identical_twice`.
- Proof: `cargo test -p engine-anim --lib lipsync` (runner: `node tools/gh-check.mjs cargo test -p
  engine-anim --lib lipsync`).
- Golden: `d57b5c8ae3096614` must stay. Bake only; the runtime inference path stays refused as it
  is today (`crates/anim/src/lipsync.rs:1`).

### AMX-5 (crates/anim)

- New file: none.
  `crates/anim/src/viseme.rs`: `pub fn mouth_shape_table() -> &'static [(&'static str, Shape, Shape)]`
  (phoneme, open shape, closed shape) and
  `pub fn shape_for_phoneme(phoneme: &str, closed: bool) -> Option<Shape>`.
- Tests: `viseme_same_phoneme_gives_the_same_shape_twice`,
  `viseme_shape_table_has_no_duplicate_phonemes`.
- Proof: `cargo test -p engine-anim --lib viseme` (runner: `node tools/gh-check.mjs cargo test -p
  engine-anim --lib viseme`).
- Golden: `d57b5c8ae3096614` must stay. Data table only.

### AMX-6 (crates/anim, crates/endpoint)

- New file: none.
  `crates/anim/src/state_machine.rs`: `pub fn state_machine_def_from_json(text: &str) ->
  Result<StateMachineDef, StateMachineError>` behind the existing `json` feature (serde_json is
  already an optional dependency of `engine-anim`), and the blend node data shape
  `pub struct BlendNodeDef { pub clip: String, pub param: Option<String> }`.
  `crates/endpoint/src/packs/anim.rs`: the state machine verb next to `anim.graph.set`
  (`packs/anim.rs:34`), plus the round-trip test.
- Tests: `state_machine_def_from_json_round_trips` (anim),
  `anim_graph_round_trips_through_the_door_with_the_same_bytes` (endpoint).
- Proof: `cargo test -p engine-anim --lib state_machine` and `cargo test -p engine-endpoint --lib
  anim` (runner: `node tools/gh-check.mjs cargo test -p engine-anim --lib state_machine`).
- Golden: `d57b5c8ae3096614` must stay. Data shape only; the door is not in the tick.

### AMX-7, AMX-8, AMX-9

No code proposed. Read-only rows. Nothing to land, no owned paths, no test, no proof command. The
golden `d57b5c8ae3096614` stays by definition because nothing changes.

## Extra check (a): NS19 BVH intake, no C++ dependency

Result: this tree cannot take a C++ dependency for this row, so assimp is a format spec and the
implementation is a Rust-native parser.

Why, from the tree:
- `deny.toml` governs Cargo dependency licences only and would pass assimp (BSD-3-Clause is
  allowed), but assimp is not a Cargo dependency at all: binding it needs a `-sys` crate, a C++
  toolchain and FFI. The workspace lint `unsafe_code = "deny"` (`Cargo.toml`, workspace lints)
  allows only five named sites, none of them a new FFI wrapper.
- The headless default graph must stay std-only (`tools/workspace-check.mjs`, and the `import`
  feature comment in `crates/assets/Cargo.toml`). A new crate dependency in `crates/assets` would
  change that graph.
- The only C++ in the tree is vendored xatlas inside `crates/geo`, behind the off-by-default
  `uv` feature with an optional `cc` build dependency (`crates/geo/Cargo.toml:42-43`). That is a
  cook-time geometry lane, not an anim lane, and it is not a model for intake.
- The house style for intake text is already a hand-written parser: `crates/assets/src/retarget.rs`
  has a std-only sidecar JSON parser (`retarget.rs:541-546`, `enum Json`) with no dependency.
- `crates/anim` is std-only and IO-free, and the NS19 row wants intake hunks beside
  `crates/anim/src/clip.rs`; the parser text function stays pure (`parse_bvh(&str)`) and the file
  read stays in the assets import lane.

Conclusion: name the Rust-native parser (AMX-1) as the implementation, and keep assimp's
`code/AssetLib/BVH/BVHLoader.cpp` (BSD-3-Clause, read live 2026-10-20) as the format spec to read.
No dependency is added, nothing is vendored, and the row costs M either way but M instead of L.

## Extra check (b): AM23 rotation-additive, the exact math

Result: the tree has the quaternion lanes and the additive layer, but the layer carries positions
only, so the row is a rotation lane plus the delta math, not a new system.

What is there today (grep, 2026-10-20):
- `crates/anim/src/layers.rs:215` `apply_additive(base, add, mask, gain_q8, out)`: position offsets
  scaled by mask times Q8 gain; the rotation line `out.rotations[joint] = base.rotations[joint];`
  at `layers.rs:242` keeps the base rotation, and the scale line `let s = f32::from(m) *
  f32::from(gain_q8) / 65280.0;` is at `layers.rs:245`. The doc comment at `layers.rs:212-214` says
  so in words: "rotations stay on the base".
- `crates/anim/src/layers.rs:281` `compose_stack` does the same for `LayerMode::Additive`
  (`layers.rs:263`, scale at `layers.rs:323`).
- `crates/anim/src/layers.rs:150` `override_joint` is the masked override with the shortest-arc
  `nlerp(base_r, over_r, t)` at `layers.rs:170`, and that `nlerp` is `graph::nlerp`
  (`crates/anim/src/graph.rs:777`, imported at `layers.rs:14`). Trap: `pose::nlerp`
  (`crates/anim/src/pose.rs:123`) differs on the exact antipodal case (it returns `a`, graph's
  returns identity), so the additive lane must use the same `graph::nlerp` as the override path.
- `crates/anim/src/hit_react.rs:276` writes `out.rotations[joint] = IDENTITY;`, so the flinch has
  no rotation at all today.
- `crates/anim/src/pack.rs` already stores rotation lanes: `PACK_VERSION_ROT` at `pack.rs:24` and
  smallest-three u16 quaternions (`pack.rs:7`).

The math to implement:
1. Delta against a reference: `delta = t * conj(r)`. The reference quaternion is conjugated and
   then multiplied onto the target, in that order (three.js `AnimationUtils.js`, quaternion
   branch: `Quaternion.multiplyQuaternionsFlat(target, valueStart, referenceValue, 0, target,
   valueStart)` where `referenceValue` is the conjugated reference). The reverse order
   (`conj(r) * t`) is wrong for a joint-local delta.
2. Apply to the live base: `base * delta` (right-multiply; the delta is in the bone's local frame).
3. Weight: `s = mask_weight * gain_q8 / 65280`, the same scale the position path uses
   (`layers.rs:243`), then `out_rot = nlerp(base_r, base_r * delta, s)`.
4. Normalize `delta`, and negate it when `delta.w < 0` so the turn is the short way.
5. Identity delta must return the base bytes exactly (weight 0 test).
6. Composition with the existing blend: the flinch stays a delta pose fed to the layers additive
   path, so it composes with the base, with the masked upper-body override and with the one
   inertializer, in that order (`crates/anim/src/additive.rs:8-9` names the stage order). The
   existing additive callers write identity rotations, so extending `apply_additive` with the
   rotation lane is a no-op for them and no existing test changes meaning.
7. Clip data untouched: the delta is computed per tick from `(HitDir, impulse_q8)`; clips stay
   immutable, the golden hash does not move, and the view envelope
   (`envelope(elapsed)`, `hit_react.rs:228`) keeps blend-in, hold and blend-out.

## Rejections (kept so nobody re-mines them)

- Motion matching: `crates/anim/src/motion_match.rs:1` ("motion_match: motion matching v2") plus
  `crates/anim/src/motiondb.rs`, `crates/anim/src/motion_style.rs`, `crates/anim/src/motion_net.rs`.
  LM1 is ticked. Not a gap.
- Spring bones and secondary motion: `crates/anim/src/springs.rs:1-2` ("springs: secondary
  motion", fixed-point spring bones, Q12). AM11 landed. Not a gap.
- Spring arm: `crates/game/src/camera_rig.rs:759` already carries a PORT line for the Godot spring
  arm; Godot is on the mined list. No new donor.
- Inertialization and inertial blending: `crates/anim/src/inertial.rs:1` ("inertial: inertialized
  clip switches over joint positions"). LF9 is ticked. Not a gap.
- Angular-speed limiter: `crates/anim/src/inertialize.rs:1` ("inertialize: foot-lock + angular
  limiter") holds `AngularLimiter`; `crates/game/src/fighter_look.rs:26` imports it. AM24's
  residual is root and translation motion plus frame cadence, which is our own root-motion code.
  No donor found that is not the mined ozz or the row's own Godot cite.
- Two-bone IK, FABRIK and foot planting: `crates/anim/src/ik.rs` (`solve_limb`, `limb_positions`),
  `crates/anim/src/fbik.rs:7-9` (`FABRIK_ITERS = 8`, `GripMode::TwoHand` at `fbik.rs:399-407`),
  `crates/anim/src/feet.rs`. LF12's only open half is the socket, which is AMX-3.
- Rotation-additive flinch: `crates/anim/src/additive.rs:1` and `crates/anim/src/hit_react.rs:1`
  exist, but the rotation lane is missing (`layers.rs:242`, `hit_react.rs:276`). AMX-2 is the row.
- Clip trim, retime, mirror and cleanup as callable operations: `crates/anim/src/edit.rs:77`
  `trim`, `:111` `retime`, `:141` `cleanup`, `:197` `strip`, `:217` `mirror`. `git grep "edit::"
  crates/` returns only `crates/anim/src/state_machine.rs:31`, which is a different module, so the
  verbs have no caller: the open work is the door verbs, our own code, no donor needed.
- Quaternion packed clip lanes and clip compression: `crates/anim/src/pack.rs:7` (smallest-three
  u16 quaternion lanes) and `crates/anim/src/pack.rs:24` (`PACK_VERSION_ROT`), plus
  `crates/assets/src/anim_compress.rs`. C14's first half is done.
- Procedural hit reactions and stagger: `crates/anim/src/hit_react.rs` and
  `crates/combat/src/depth.rs` (poise) exist; only the rotation delta is missing, which is AMX-2.
- Crowd animation LOD and animation sharing: `crates/anim/src/lod.rs:1-10` (tier table, 8 nearest
  at full rate) and `crates/anim/src/crowd_scale.rs:1-8` (1000-agent tape). AM13's missing part is
  the 64-fighter timing receipt, a measurement, not a donor. AMX-8 is the only sharing idea found.
- Facial and lip sync: `crates/anim/src/lipsync.rs:1-2` (offline phoneme bake) and
  `crates/anim/src/viseme.rs`. LAI3 needs a live curve source, which is AMX-4 and AMX-5.
- BVH motion intake: nothing in `crates/` reads BVH. `git grep -i bvh crates/` returns only
  `crates/audio/src/trace.rs:22` (a ray-tracing BVH) and the vendored `crates/geo/vendor/xatlas`.
  AMX-1 is the gap.
- Gait and locomotion state machines: `crates/anim/src/locomotion.rs` and
  `crates/anim/src/state_machine.rs` exist; AMX-6 only shows the JSON shape.
- Anim graph node editors in data: `crates/anim/src/graph.rs` and the JSON door
  `crates/endpoint/src/packs/anim.rs:34-40` (`anim.graph.set`, `anim.graph.undo`,
  `anim.graph.get`, `anim.preview`) exist, with tests `pack_registers_four_annotated_verbs` and
  `set_undo_preview_through_the_door`. This is an editor-UX gap, not an engine gap. AMX-6 is the
  shape donor for EG18b.
- BX5 injured locomotion (hop, crawl, fall, one-arm guard): no MIT-class code donor found. The
  masks are already there (`crates/anim/src/injured.rs:1`, `crates/anim/src/injured_double.rs:1`);
  the missing part is clip content plus our own mask work. CMU-style mocap data was not proposed:
  AGENTS cuts mocap bytes in git.
- EG42 baked per-move hit tracks: no donor needed. `crates/anim/src/loco_bake.rs` and
  `crates/anim/src/hit_track.rs` exist.
- LF9, LF13, LF14, EG18b, EG19c: all our own code in this tree (`crates/anim/src/inertial.rs`,
  `crates/anim/src/fbik.rs` look-at, `crates/anim/src/trails.rs`,
  `crates/anim/src/state_machine.rs`, `crates/game/src/fighter_look.rs:24`). No external donor.
- orangeduck/Motion-Matching: already cited by board rows LF12 and LF13. Not a new find.
- Unity-Technologies/Animation-Instancing: Unity is on the mined list and the repo carries its own
  licence. Rejected.
- DragonBones (slots and attachments) and amethyst/lumberjack: the repos returned 404 on the
  GitHub API on 2026-10-20, so no file could be read. No row written.
- natepac/crowdbake has no LICENSE file, so its code is off limits; only the README ideas are read
  (AMX-8).
