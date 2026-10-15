# Animation feel steals for engine2040 (research, 2026-10-11)

Scope: web research only, no repo edits. Each URL was opened with WebFetch. UNVERIFIED means search snippet or memory only. "(path UNVERIFIED)" means the repo is verified but the file path is not.

Determinism rule for every row: these steals are float code. They live in the view layer (render-side pose, IK, springs, camera) and are rebuilt from hashed state each frame. Anything that changes gameplay (hit pause) must be an integer tick counter in the hashed sim.

## Top 12 in build order

Est. lines are estimates.

| # | Steal | Repo + exact URL(s) | What to port | Est. lines | What the player sees | Depends on |
|---|---|---|---|---|---|---|
| 1 | Critically damped spring | [Spring-It-On common.h](https://github.com/orangeduck/Spring-It-On/blob/master/common.h) | halflife_to_damping, critical_spring_damper_exact, damper_exact, fast_negexp | 60 | Camera and IK targets settle, no overshoot | none |
| 2 | Inertialization | [inertialization.c](https://github.com/orangeduck/Spring-It-On/blob/master/inertialization.c) | inertialize_transition, inertialize_update (per-bone offset decays) | 50 | Clip switches (attack, stance, recover) stop popping | 1 |
| 3 | Trauma camera shake | [bevy 2d_screen_shake.rs](https://github.com/bevyengine/bevy/blob/latest/examples/camera/2d_screen_shake.rs); talk: [Eiserloh, GDC 2016](https://www.gamedeveloper.com/programming/video-sprucing-up-cameras-with-math) | shake_camera, increase_trauma, Perlin generate; decay 0.5/s, exponent 2 | 90 | Hits and kills jolt the frame, fade in about 2 s | none |
| 4 | Hit pause | [saltmire-juice](https://github.com/saltmire/saltmire-juice) (MIT; Juice.hitstop; path UNVERIFIED) | Frame counter that freezes a fighter's tick for N ticks | 40 | 2-4 frame freeze on contact | integer tick in sim |
| 5 | Two-bone IK | [ozz ik_two_bone_job.h](https://github.com/guillaumeblanc/ozz-animation/blob/master/include/ozz/animation/runtime/ik_two_bone_job.h); [Motion-Matching controller.cpp](https://github.com/orangeduck/Motion-Matching/blob/master/controller.cpp) (ik_two_bone) | Law-of-cosines solve, pole vector, soften, weight | 120 | Hands grip the sword, knees bend | pose hook |
| 6 | Look-at and aim IK | controller.cpp (ik_look_at); [ozz ik_aim_job.h](https://github.com/guillaumeblanc/ozz-animation/blob/master/include/ozz/animation/runtime/ik_aim_job.h) | Aim forward axis at target, up vs pole, weight | 60 | Head and torso track the opponent | 5 |
| 7 | Procedural hit reaction | [ProcHitReact](https://github.com/Vaei/ProcHitReact) (MIT, UE C++; path UNVERIFIED) | One spring per bone near impact, falloff by distance | 100 | Struck torso or limb flinches and recovers | 1, 5 |
| 8 | Foot placement | [Vulkan docs: foot placement](https://docs.vulkan.org/tutorial/latest/Advanced_glTF/Procedural_Animation_IK/04_foot_placement.html) | Down-ray from height, pelvis drop = largest gap, align to normal, lerp factor 8-12 | 120 | Feet plant on ramps and steps | 5, physics raycast |
| 9 | Dead blending (upgrade of 2) | [Spring-It-On deadblending.c](https://github.com/orangeduck/Spring-It-On/blob/main/deadblending.c) | dead_blending_transition, dead_blending_update (extrapolate, smoothstep) | 60 | Very different poses (guard to lunge) blend cleaner | 2 (same API) |
| 10 | Motion matching search | [Motion-Matching database.h](https://github.com/orangeduck/Motion-Matching/blob/master/database.h) | normalize_feature, build matching features, database_search with AABB pruning (64/16) | 250 (3 packets) | Next clip chosen by heading, fewer state-machine pops | 2, 8, 11 |
| 11 | Trajectory prediction | controller.cpp (trajectory_*_predict, simulation_positions_update) | Predicted positions and rotations; damper-based desired velocity | 150 | Lean into turns, weighted stops | 10 |
| 12 | Spring bones | [VRM springBone spec](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0); [tenMinutePhysics 14-cloth.html](https://github.com/matthias-research/pages/blob/master/tenMinutePhysics/14-cloth.html) (XPBD) | Per-joint stiffness, drag, gravity, sphere collider; XPBD distance constraints with substeps | 150 | Strips, capes, hair lag and settle | 2; client-only |

Packet plan: 1+2, 3, 4, 5+6, 7+8, 9, then 10-12 in packets of at most 200 lines. The learned variant of motion matching ([theorangeduck.com](https://www.theorangeduck.com/page/learned-motion-matching)) is skipped.

## Per-item notes

### 1. Critically damped spring
- Take: halflife_to_damping, critical_spring_damper_exact (no over/under branches, y = d/2), damper_exact, fast_negexp (rational approximation of e^-x, so no exp on this path). All float.
- Deterministic: float, view-only. Never written back to hashed state.
- Test: drive a spring from 0 to 1 at 60 Hz and at 30 Hz for 2 s. Max x stays <= 1 + 1e-5 (no overshoot), and the two end values agree within 1e-4.

### 2. Inertialization
- Take: inertialize_transition stores (src - dst) as offset for position and velocity per bone; inertialize_update returns input + offset with the offset decaying by halflife. Rotations need a quaternion difference (write it).
- Deterministic: view-only. Clip id and frame index stay integers in sim state; offsets are rebuilt in render.
- Test: switch clip A to B at frame t. The first output equals A's last pose within 1e-5. After 5 halflives the output equals B within 1e-3.

### 3. Trauma camera shake
- Take: shake_camera (applied in PostUpdate, reset next frame), increase_trauma (clamp 0..1), perlin_noise::generate. Constants: decay 0.5/s, exponent 2, noise speed 20, max roll 10 deg, max offset 20 (retune for 3D).
- Deterministic: render-only offset from the frame clock. The fight camera state is not written. Trigger from hit events, not wall time.
- Test: add trauma 1.0, step 120 frames at 60 Hz. Trauma reaches 0 at 2.0 s +/- 1 frame, and the offset is exactly zero after reset.

### 4. Hit pause
- Take the concept from the saltmire-juice README (Juice.hitstop). Its code path is UNVERIFIED. Port as hitstop_ticks (u8) in fighter state. While > 0, anim time, attack timers and movement do not advance. Render keeps the last pose.
- Deterministic: must be in sim as an integer counter, hashed. Never wall clock.
- Test: replay one input log twice, state hashes match each frame. With a 3-tick hitstop, the anim frame index ends exactly 3 behind the no-hitstop run.

### 5. Two-bone IK
- Take: ozz IKTwoBoneJob inputs (target, pole_vector, mid_axis, soften, weight), and the ik_two_bone function in Motion-Matching controller.cpp. The function name is confirmed; its body was not read (UNVERIFIED). Port the law-of-cosines solve, reach clamp, and pole side.
- Deterministic: view-only, applied after the matched or blended pose. Hand target = weapon grip socket transform.
- Test: put the target at 0.9 of chain length. After the solve, the end joint is within 1e-3 of the target, and the mid joint lies on the pole side (dot > 0).

### 6. Look-at and aim
- Take: ik_look_at (controller.cpp) and ozz IKAimJob (forward, up, pole_vector, weight; outputs a joint_correction quaternion).
- Deterministic: view-only.
- Test: target 90 deg to the right, weight 1: forward axis within 1 deg of target. Weight 0.5: within 1 deg of the half-angle slerp.

### 7. Procedural hit reaction
- Take the pattern from ProcHitReact (MIT, UE C++; file paths UNVERIFIED): impulse direction times distance falloff drives one spring per bone (item 1). The spring target is 0, so the bone returns.
- Deterministic: view-only. Impact point and strength are integers from the sim hit event, converted to float at render.
- Test: impulse at the spine. Bones within 0.3 m reach |offset| < 1e-3 within 0.5 s. Bones beyond 1.0 m stay exactly unchanged.

### 8. Foot placement
- Take the Vulkan algorithm: ray from character height, length 2x height. Pelvis drops by the largest foot gap and never rises. Foot up-axis aligns to the smoothed terrain normal. Lerp factor 8-12. Code is C++/glm/Jolt; port the math only.
- Deterministic: view-only ray against the static arena collider. Never writes back.
- Test: flat ground, offsets < 1e-4. A 0.1 m step under one foot: pelvis drops 0.1 m and that foot ends within 1 cm of step height after 0.5 s.

### 9. Dead blending
- Take dead_blending_transition and dead_blending_update (plus smoothstep). Use it where inertialization looks too soft on big pose changes; keep item 2 as the default. Skip the learned extrapolator on the Motion-Matching dead_blending branch.
- Deterministic: view-only.
- Test: after the switch, max frame-to-frame joint delta over 0.3 s stays under 2x the source clip's max per-frame delta.

### 10. Motion matching search
- Take: normalize_feature, database_build_matching_features (foot pos and vel, hip vel, trajectory pos and dir weights), database_search with 64/16 AABB pruning. It is brute force with bounds, not a KD-tree.
- Deterministic: view-only in v1. If it ever drives gameplay, store the chosen (clip, frame) as integers and build features in fixed point.
- Test: a query equal to database row k returns k. Bounded search equals brute-force search on 1000 random queries.

### 11. Trajectory prediction
- Take: trajectory_desired_velocities_predict, trajectory_positions_predict, trajectory_rotations_predict, query_compute_trajectory_position_feature, and simulation_positions_update (damper halflife).
- Deterministic: view-only. The sim position stays the authority. The predicted trajectory only feeds the pose query.
- Test: desired velocity 3 m/s held for 2 s. In steady state, the spacing between predicted points k-1 and k is 3*dt within 1e-3.

### 12. Spring bones
- Take the VRM model (per-joint stiffness, gravityPower, dragForce, hitRadius, sphere colliders) and the XPBD loop from 14-cloth.html (distance constraints with compliance, 15 substeps). Verlet per joint is enough for strips. Unity reference: [vgospringbone](https://github.com/izayoijiichan/vgospringbone) (MIT).
- Deterministic: client-only float, same rule as the Rapier ragdoll.
- Test: pin the root, move it 1 m, then stop. Segment lengths stay within 1% after 2 s, and tip speed < 1e-3 m/s by 3 s.

## Camera framing (2+ fighters)
- [GatitoMimoso00/3D-Fighting-Camera](https://github.com/GatitoMimoso00/3D-Fighting-Camera) (MIT, Godot 4): forward, back, side-step and side-swap handling with eased follow. Script paths and zoom are UNVERIFIED (the page lists folders only).
- Midpoint framing with distance zoom: no verified open-source code. Build it: center = midpoint of fighters, distance from spread through the item 1 spring, clamped to arena bounds.

## Ragdoll and powered blend (not in top 12)
- [sergioabreu-g/active-ragdolls](https://github.com/sergioabreu-g/active-ragdolls) (Apache-2.0, Unity): animated body plus physics body; joints track the animation.
- [ashleve/ActiveRagdoll](https://github.com/ashleve/ActiveRagdoll) (MIT): PID joint drive. Its links point to AnimationFollowing.cs, MasterController.cs, SlaveController.cs in another repo; paths here UNVERIFIED.
- Per-bone blend weight from physics to animation: general technique, no verified implementation (UNVERIFIED).
- Getting up: start from the rest pose and blend with items 2 or 9.

## Compression, masks, layers
- [ACL](https://github.com/nfrechette/acl) (MIT); decompression API in [includes/acl/decompression](https://github.com/nfrechette/acl/tree/develop/includes/acl/decompression). Skip until clip memory matters: 8 fighters, few clips.
- Upper/lower masks and additive layers: no verified source. Write a per-bone weight array inside the blend (about 40 lines).

## Rollback
- [GGRS](https://github.com/gschup/ggrs) (MIT or Apache-2.0). Its examples folder holds ex_game; box_game and sync_test are not in the listing.
- Rule: clip id, frame index, hitstop ticks and attack timers are integers in the hashed state. Inertial offsets, springs, IK and trauma are rebuilt from history and never hashed.
- Test: one input log run twice from the same seed produces identical hashes every frame.

## Weapon trails, slow-mo, impact frames
- Trails: [julcst/bevy_trail](https://github.com/julcst/bevy_trail) (GPU ring-buffer trail; no license stated). Technique: ring buffer of blade base and tip per frame, u along trail, v age, alpha fade.
- Slow-mo on kill and impact frames: no verified open-source code. Implement as a view time scale, or as an integer tick skip if it must affect gameplay (UNVERIFIED).

## Datasets and retargeting
- [LAFAN1](https://github.com/ubisoft/ubisoft-laforge-animation-dataset): BVH, 30 fps, 77 sequences, 5 subjects. Themes include "Fight" and "Fight and sports" (verified). Best sword-and-shield starting point, but no weapons. The repo states CC BY-NC-ND 4.0.
- [100STYLE](https://www.ianxmason.com/100style/): 100 locomotion styles, over 4 million frames at 60 fps, CC BY 4.0 per the page. BVH zip (1.5 GB) on [Zenodo](https://zenodo.org/record/8127870). No melee.
- Mixamo (Adobe, FBX): URL and clip content UNVERIFIED. Check Adobe's terms before training models on it. CMU mocap (http://mocap.cs.cmu.edu): fetch failed with a certificate error (UNVERIFIED).
- Retarget code: [omimo/PyMO](https://github.com/omimo/PyMO) (MIT) gives a BVH parser and preprocessing (the README imports pymo/parsers.py and pymo/preprocessing.py); it has no retarget. Rule-based option: joint-name map plus FK with scaled target offsets (method, UNVERIFIED as a library). Learned options: [pan-motion-retargeting](https://github.com/hlcdyy/pan-motion-retargeting) (BSD-2-Clause, Mixamo scripts) and [R2ET](https://github.com/Kebii/R2ET) (inference_bvh.py).

## Caveats
- The trauma model (item 3) comes from the Bevy example; the talk page confirms only title, speaker, year.
- UNVERIFIED list: ozz two-bone body, ProcHitReact and saltmire-juice file paths, UniVRM spring code, VgoSpringBone scripts, CMU and Mixamo URLs, slow-mo and impact-frame code, Godot Active Ragdolls (search only).
