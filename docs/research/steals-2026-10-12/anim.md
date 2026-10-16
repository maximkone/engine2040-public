STEAL SCOUT: ANIMATION AND COMBAT FEEL (engine2040, 2026-10-12, read-only pass)
Donor shas read 2026-10-12: godot 232e6b14abf6e590b41d60f11704e42de22bff72 (MIT), JoltPhysics 3c91764eb57c02c4c397598d507ef66ba12faf1c (MIT),
ozz-animation 744eb9d99f606eda849acb0b1204f7a3dc20bca1 (MIT), bevy c9c21c2b7af92a396b5f93ecfa1536a8a2afcc68 (MIT or Apache-2.0),
Spring-It-On 4d97b497a78e40c7b47d49e8a9da27e4aa5616d6 (MIT), Motion-Matching 57b7250e0d34a4e456a34d47e24c2f05fdcc711e (MIT).
Correction to the 2026-10-11 scout: Vaei/ProcHitReact is NOT "one spring per bone". Its README says it blends physics weight through Blend In, Hold,
Blend Out states and blends overlapping hits. Item 7 of anim-feel-steals.md needs that fix. No Bevy motion-matching crate exists (searched).

## 1. We have

1. Window pose path = 6 clips and a linear 9-tick crossfade: fighter_look.rs:73-80 (clips), :64 and :217-221 (FADE_TICKS 9, linear weight),
   :838-885 slot_pose. It imports only `contract` and `pose` from engine_anim (:21-22). Landed modules with no caller in crates/game/src or
   crates/render/src: hit_react.rs, inertialize.rs, ik.rs, fbik.rs, springs.rs, lod.rs (Grep engine_anim; only physics/balance.rs uses hit_react masks).
2. The swing starts on the OUTCOME cue and is back-dated 12 ticks: fighter_look.rs:50, :682-700, :786-801. Visible windup before a hit = 0 ticks.
   Board EG19b/EG19c (SPRINT-18-gaps.md:436-451) would fix it; `AttackStarted` has 0 hits in crates. v2 light startup = 15 ticks (tables.rs:53).
3. Hit react: 6 abstract joints (hit_react.rs:24-36), 0.10 m peak (:44), linear envelope welded to HIT_REACT_TICKS = 6 = 100 ms (fight.rs:65;
   hit_react.rs:228-234), positions only, rotations identity (:261-278). layers.rs:212-214 additive keeps base rotations.
4. inertialize.rs:25-95 is a position-only spring (no quaternions), unused. ik.rs has two_bone/fabrik/foot_lock (:20,:74,:140); fbik.rs has
   solve_look/solve_aim (:392,:400). Board LF9 says "new inertial.rs", LF12 says "new ik.rs": both files exist already (slim law, animation.md:36-37).
5. Balance lean/kneel/lie are constant pelvis offsets, not eased (fighter_look.rs:66-70, :870-883).
6. Three camera stacks: fighter_look::fight_camera (stateless `&self`, azimuth = atan2 of the pair axis, fov 50 deg fixed, :1133-1218),
   camera.rs Director/StagedRig fed by camera_feed.rs, and director.rs. `present_into` OVERWRITES list.camera with fight_camera (fighter_look.rs:1089),
   so camera_feed trauma never reaches the window picture. Shake = uncorrelated per-tick hash noise, translation only (camera.rs:755-779);
   DampedSync is a gain per call, frame-rate dependent (camera.rs:2487-2552).
7. Feel layers exist as integer data: feel.rs ContactLayers (:916-965), juice (:376-399), toggles never reach the sim (:994-1052). Presentation
   consumers: Grep hitstop/trail/slow-mo in crates/game/src finds none. Sim hit-stop freezes the whole step (fight.rs:717-719; set :918,:959; v1 = 4 ticks).
8. Sim blade is a fixed straight segment 200..1500 mm along facing (arena/src/lib.rs:298-324, match_wire.rs:31-35); the view plays 3 swing clips.
9. Ragdoll: one Rapier body per skeleton joint and ONE global blend scalar (ragdoll.rs:9-19, :448); powered.rs has per-bone drive and hit masks;
   callers are arena live.rs/limbs.rs only; the window never draws them.
10. Plan drift: animation.md:26 says warp_ground.rs does not exist, but crates/anim/src/warp_ground.rs exists and arena uses GroundWarp
    (arena/src/lib.rs:98). gameplay.md:141-149 says the camera is "recorded in-sim"; the window camera is stateless view code. animation.md section 4 has
    nothing on hit-react length, telegraph or camera.
11. Meters to prove with: `arena view:` line (view_truth.rs:356): pops, pop_cm_max (POP_M 0.10, :31), sunk_cm_max, idle_float_cm_max.
    tools/product-bar.json:24: pops 35-55 per 300 frames, up to 1 m, at attack and idle switches.

## 2. Steals (ranked by value for the duel look per hour)

1. Rotation-additive flinch with blend-in / hold / blend-out | ozz-animation@744eb9d9:src/animation/runtime/blending_job.cc AddLayers (read 2026-10-12);
   envelope idea only: Vaei/ProcHitReact@d4110b10 README (UE C++, source not read) | MIT / MIT | PRESENTATION | lands in crates/anim/src/layers.rs:212,
   hit_react.rs:228,261, fighter_look.rs slot_pose after the fade (:865) | M
   Idea: per joint the flinch is a delta quaternion nlerp(identity, delta, weight) multiplied onto the base rotation in local frame (ozz: out = base * delta;
   weight 0 skips; the mask weight scales it). Envelope in/hold/out; a second hit ADDS instead of restarting. Strike octant from drawn attacker/victim yaw.
   Number: window flinch today 0 (no caller); module flinch = 100 ms and no rotation. Target: head sway >= 8 cm at 1x impulse (new view field
   react_head_cm_max, 0 today), lasting >= 400 ms (3 in, 4 hold, 20 out ticks, start values), pops not above the pre-change count.
   Proof: `cargo test -p engine-anim --lib hit_react_rot` x2 (weight 0 = base bytes; two strikes 5 ticks apart sum within 1e-5). Risk: skin shear
   above ~25 deg per joint, so clamp. Kill if pops or sunk_cm_max rise. Sim cannot move: the module never reads back.

2. One stateful fight camera: exact springs, timed FOV punch, smooth-noise roll trauma | orangeduck/Spring-It-On@4d97b497:common.h (critical_spring_damper_exact,
   damper_exact, halflife_to_damping) + timedspring.c (timed_spring_damper_exact); bevyengine/bevy@c9c21c2b:examples/camera/2d_screen_shake.rs (noise speed 20,
   three offsets +0/+100/+200, shake = trauma^2) (all read 2026-10-12) | MIT; MIT or Apache-2.0 | PRESENTATION | crates/game/src/fighter_look.rs:1089,1133-1218,
   camera.rs:755-779,2487-2552 | M
   Idea: fight_camera becomes a thin caller of camera.rs with state: yaw (shortest arc), distance and look point on halflife springs (dt-exact, so 30 and 144
   fps agree); FOV punch (-3 deg on Hit) on a timed spring that lands in exactly N ticks; trauma noise = lattice value noise from the existing splitmix64 sampled
   at (tick + alpha) * 20/60 for x, y, z and roll, deterministic and smooth. Delete the hand-rolled orbit math once capture parity holds (slim: one stack).
   Number (computed, not measured): two fighters side-stepping at 1.7 m/s swing the pair-axis yaw at 49 deg/s at 4 m and about 240 deg/s at 0.8 m, unsmoothed.
   Target: yaw rate p99 <= 60 deg/s, eye step <= 25 cm per frame over 300 frames (new view fields cam_yaw_dps_p99, cam_eye_step_cm_max; start values).
   Proof: `cargo test -p engine-game --lib fight_cam_spring` x2 (30 fps vs 144 fps end within 1 cm) + product-check keeps framing_empty 0.
   Risk: lag hides a fighter; framing_empty and cover_max floors are the kill. Absorbs the camera hunk of LF11 (LF11 keeps the trauma value).

3. Last-stage joint angular-speed limiter | godotengine/godot@232e6b14:scene/3d/limit_angular_velocity_modifier_3d.cpp (read 2026-10-12) | MIT | PRESENTATION |
   crates/anim/src/inertialize.rs (deepen) + fighter_look.rs slot_pose end, before the pelvis pin (:868) | S
   Idea: keep the last output rotation per bone; each step slerp toward the wanted rotation by min(1, limit*dt/angle); chain list plus exclude flag; reload
   state on activate or rig change so no jump. Backstop for every stage after it (fade, flinch, look-at, IK, ragdoll blend), not only clip switches.
   Number: pops 35-55 per 300 frames, up to 1 m (product-bar.json:24) -> 0 pops and pop_cm_max <= 10 (POP_M). Start limits 1080 deg/s body, 2000 deg/s
   weapon arm (tune on captures).
   Proof: `cargo test -p engine-anim --lib ang_limit` x2 (180 deg step at 1080 deg/s takes 10 ticks +-1; in-limit input passes byte-equal); `arena view:` pops=0.
   Risk: slot_pose is documented pure (fighter_look.rs:833-836, test fighter_anim_same_tick_*); a limiter has state, so advance it in observe_tick at 60 Hz and
   evaluate at alpha. Kill if the attack impact-frame clip phase shifts more than 2 ticks.

4. Phase-warped attack from the sim's attack age, marker track, telegraph meter | IDEA-ONLY (no donor file; our events.rs track + EG19c; floors from GS32) | n/a |
   PRESENTATION | fighter_look.rs:786-831 (start_swing, clip_time), crates/anim/src/events.rs markers, view_truth.rs law | S-M
   Idea: clip time = piecewise-linear map of attack age through three markers cooked into the clip event track: windup peak, impact, recovery end. v2 light
   (startup 15): ticks 0-11 reach the windup peak (200 ms anticipation), ticks 12-14 snap to impact (50 ms), then the table's recovery. Any v2 row value works
   without re-authoring the clip. Needs the press-time cue (EG19b).
   Number: visible windup before impact 0 ticks today. Target: weapon hand >= 25 cm from guard by impact minus 3 ticks (new view law telegraph_hand_cm_min,
   start value); impact phase error <= 1 tick (EG19c asks 2).
   Proof: `cargo test -p engine-game --features app --lib telegraph_warp` x2. Risk: a 250 ms signal is under the 340 ms floor GS32 reports; warp cannot make
   reaction time, heavy rows (24-30 ticks) must carry it. Cap clip stretch at 2.0x or it looks rubbery.

5. Virtual-time scale for kill slow-mo | bevyengine/bevy@c9c21c2b:crates/bevy_time/src/virt.rs (read 2026-10-12) | MIT or Apache-2.0 | PRESENTATION (wall-to-tick
   pacing only; tick sequence and hash untouched) | crates/game/src/windowed.rs:906 TickAccumulator::push_elapsed_ms | S
   Idea: relative speed multiplies the wall delta before the accumulator; a change applies from the next frame; skip the multiply at 1.0 (no rounding);
   expose effective speed so camera springs, particles and audio pitch follow; a max-delta clamp bounds catch-up (ours WIN_MAX_STEPS_PER_FRAME). Ramp with
   the timed spring of steal 2. Local play and replay only; netgame forces 1.0.
   Number: 0 slow-mo today. Target: scale 0.25 for 12 sim ticks after a Knockdown cue (0.8 s wall); 120 wall frames at 0.25 give 30 ticks +-1 and the same
   digest as scale 1.0 at the same tick.
   Proof: `cargo test -p engine-game --lib time_scale` x2 + `node tools/proof.mjs --skip-build` prints golden d57b5c8ae3096614. Kill if any caller outside
   windowed.rs reads the scale.

6. Look-at with soft limit, target-switch easing, behind-the-back guard | godot@232e6b14:scene/3d/look_at_modifier_3d.cpp + aim_modifier_3d.cpp; orangeduck/
   Motion-Matching@57b7250e:controller.cpp ik_look_at (read 2026-10-12) | MIT | PRESENTATION | crates/anim/src/fbik.rs:392 solve_look + fighter_look.rs | S-M
   Idea: LF13 hard-clamps at 70/30 deg. Godot eases toward the limit on a quadratic curve above a damp threshold (head slows, never stops dead), starts a timed
   transition when the target changes (ease type, duration), and on a flip (sign change across the back) blends through the rest pose instead of spinning
   through the body. Each stage writes slerp(current, wanted, amount). Donor ik_look_at has no limits or weight, so it is only the core aim.
   Number: LF13 has a velocity jump at the clamp and at every lock-target swap. Target: second difference of head yaw <= 0.5 deg/tick^2 over a 0..180 deg
   sweep at 1 deg/tick; retarget ease 12 ticks (start values).
   Proof: `cargo test -p engine-anim --lib look_soft` x2. Kill if the head leaves the 70 deg cone.

7. Hit-stop shudder and frozen alpha | IDEA-ONLY for code (saltmire/saltmire-juice README + dev.to article read: hit-stop by engine time scale 0.05 for 0.08 s, 60-120 ms
   advised, MIT; unusable as code because ours is a sim counter) | MIT (README) | PRESENTATION | fighter_look.rs fold_cues (:682) and slot root offset | S
   Idea: while the sim counter is above 0 (fight.rs:717-719) hold sub-tick alpha (else bodies creep between two ticks), shudder the victim +-1.5 cm along the
   attack axis on alternate frames, lean the attacker in 2 cm, keep the flash. Strength by the per-kind stop (GP31: 3/6/8 ticks = 50/100/133 ms).
   Number: 0 hit-stop presentation code in crates/game/src. Target: victim root amplitude 1-2 cm during the stop, 0 outside (new view law hitstop_shudder_cm).
   Proof: `cargo test -p engine-game --features app --lib hitstop_shudder` x2; sim digest equal with the layer on and off. needs LF10.

8. Weapon trail from render-frame blade samples, smoothed | IDEA-ONLY for code (OBKF/Godot-Trail-System README read: Chaikin smoothing with iterations and ratio, lifetime in
   seconds, MIT; gamedev.net advice: Catmull-Rom through the samples) | MIT (README) | PRESENTATION | fighter_look.rs LF14 hunk + a render strip draw | S-M
   Idea: LF14 keeps 12 tick-rate samples. Sample base and tip from the CPU-skinned joints every render frame (we skin per frame, view_truth.rs:6-9), 2 Chaikin
   passes, fade by age in seconds, draw only from windup peak to impact + 4 ticks (steal 4 markers). A 90 deg swing of a 1.3 m reach in 4 ticks moves the tip
   about 0.5 m per tick (estimate), so a tick polyline shows corners.
   Number: no trail today. Target: no segment longer than 8 cm after smoothing; idle slots draw 0 vertices; under 0.2 ms at 1080p on the potato tier.
   Proof: view law trail_seg_cm_max <= 8 + render lib test. Kill if over the 0.2 ms bar.

9. Spring-bone chains with colliders and centre-space compensation | godot@232e6b14:scene/3d/spring_bone_simulator_3d.cpp + spring_bone_collision_capsule_3d.cpp
   (read 2026-10-12) | MIT | PRESENTATION | crates/anim/src/springs.rs (deepen) + fighter_look.rs | M
   Idea: per joint: inertia with drag, stiffness pull to the rest direction, gravity, length clamp, push-out of the joint sphere from capsule colliders
   (closest point on the capsule axis), rotation output by from-to; the chain is simulated in the character's centre frame so root motion does not whip it;
   params ride a curve along the chain. Do NOT port its time step: drag is applied once per call, so decay depends on frame rate (as read). Keep our Q12 tick.
   Number: springs.rs is single offset springs (:1-11), no length, no colliders, no window caller; 0 secondary bones drawn. Target: 8 chain joints per fighter
   (cape 4, crest plume 3, strap 1) within the 0.1 ms for 8 fighters bar (animation.md:69); 0 joints inside the body capsule.
   Proof: `cargo test -p engine-anim --lib spring_chain` x2 (root moves 1 m then stops: tip speed < 1e-3 m/s by 3 s; length error <= 1%). NEEDS CONTENT: do the
   CT7 bodies carry cape/plume bones? Unchecked. Kill if they do not (then a cook row first).

10. Blade-tip path baked from the clip into the v2 sweep (what you see is what hits) | IDEA-ONLY (no donor; our retarget_cook RootTable/ContactTable precedent,
    assets/src/retarget.rs:13-16) | n/a | SIM, v2 only: an integer mm table per move, read by attack age, hashed as content; v1 and golden untouched | crates/assets/
    src/retarget.rs (new BladeTable beside RootTable) + arena/src/lib.rs:298-324 | L
    Idea: arena strike_pose draws a straight stab; the view plays an arc. Cook the blade base/tip per tick of the active window from the authored clip, quantize
    to mm, and let strike_pose read it. Trails (steal 8) and sparks (LF6) then sit on the real hit volume.
    Number: sim blade vs drawn tip gap at the impact tick unmeasured; Target <= 15 cm. Proof: `cargo test -p engine-arena --lib blade_table` x2 + feel.rs
    balance_over_200 stays in the 400-600 permille band (feel.rs:36-40). Kill if golden moves or the band breaks.

11. Skeleton mapper and per-bone drive for the visible ragdoll | jrouwe/JoltPhysics@3c91764e:Jolt/Skeleton/SkeletonMapper.cpp + Jolt/Physics/Ragdoll/Ragdoll.cpp (read
    2026-10-12); per-bone blend weights and a blacklisted pelvis from ProcHitReact README | MIT | PRESENTATION (client-local ragdoll, never hashed, per FOCUS EG34 note) |
    crates/physics/src/ragdoll.rs:9-19,448 | L
    Idea: simulate a low-detail ragdoll (about 15 bodies) and map it to the full render skeleton: direct joints, chains whose middle joints follow by from-to
    rotation, unmapped joints follow their parent, locked translations. Drive with motors (position target plus velocity target from the pose delta over dt).
    Number: bodies per fighter = joint count today (not printed here, unmeasured) -> <= 16; post ms for 8 ragdolls at 60 Hz. Proof: `cargo test -p engine-physics
    --lib ragdoll_map` x2. Risk: big; only after steals 1-3. Kill if cost per fighter does not drop by half.

12. Bone-matrix (dual quaternion) animation texture for the spectator ring | IDEA-ONLY (GPU Gems 3 ch.2 idea; chengkehan/GPUSkinning README read, MIT, no shader read) |
    MIT (README) | PRESENTATION | crates/assets/src/vat.rs (GS20 alternative) + render crowd draw | M
    Idea: store per-bone dq for each frame of 6-8 loops (sit, cheer, clap, boo); the vertex shader blends two loops with nlerp, so crowd reacts to hits.
    Number: GS20 VAT: 5,000 verts x 30 frames x 8 B = 1.2 MB per loop. Bones: 40 bones x 30 frames x 16 B (f16 dq) = 19 KB, about 60x smaller (computed).
    Proof: bake bytes x2 + decode within 1e-3. Kill if the spectator mesh needs per-vertex deformation (cloth).

13. Dead blending for big pose jumps | orangeduck/Spring-It-On@4d97b497:deadblending.c (read 2026-10-12) | MIT | PRESENTATION | crates/anim/src/inertialize.rs | S
    Idea: at the switch freeze the source velocity and extrapolate the OLD motion for the blend time, then smoothstep into the new clip, instead of offsetting.
    Donor numbers (Motion-Matching controller.cpp): inertialization halflife 0.1 s (slider 0-0.3) = 6 ticks vs our FADE_TICKS 9. Do only if LF9 leaves pops
    at guard -> windup. Proof: max joint delta over 0.3 s after the switch <= 2x the source clip's max per-frame delta. Kill if LF9 reaches pops 0.

## 3. Board rows (top 6)

- [ ] XX1 anim: rotation-additive hit flinch on the real skeleton — hit_react.rs:261-278 writes identity rotations, layers.rs:212-214 keeps base rotations,
  the window imports no hit_react (fighter_look.rs:21-22) and the flinch window is 6 ticks (fight.rs:65). Diff: good. Tier A. Size M. Owns
  `crates/anim/src/layers.rs` additive hunk, `crates/anim/src/hit_react.rs`, `crates/game/src/fighter_look.rs` flinch hunk, `crates/game/src/view_truth.rs`
  react_head_cm_max. Accept: `cargo test -p engine-anim --lib hit_react_rot` x2 (weight 0 equals base bytes, two strikes sum within 1e-5) and
  `node tools/product-check.mjs` prints react_head_cm_max >= 8 with pops not above before. F2P: no rotation additive test; react_head_cm_max is 0.
  P2P: golden unchanged. needs: EG18.
- [ ] XX2 game: one stateful fight camera — `present_into` overwrites the director camera with a stateless fight_camera (fighter_look.rs:1089,1133-1218), trauma
  never shows, DampedSync is frame-rate dependent (camera.rs:2487-2552). Exact halflife springs on yaw, distance and look point, timed FOV punch, smooth-noise
  roll trauma, one stack. Diff: good. Tier A. Size M. Owns `crates/game/src/fighter_look.rs` camera hunk, `crates/game/src/camera.rs` spring and noise
  hunks, `crates/game/src/view_truth.rs` cam fields. Accept: `cargo test -p engine-game --lib fight_cam_spring` x2: yaw rate p99 <= 60 deg/s and eye step <= 25 cm
  over 300 frames; 30 fps and 144 fps stepping end within 1 cm. F2P: no spring state; stepping at two rates differs. P2P: golden unchanged; framing_empty 0.
  needs: none.
- [ ] XX3 anim: last-stage joint angular-speed limiter — pops 35-55 per 300 frames, up to 1 m (tools/product-bar.json:24); no stage guards the final pose
  (fighter_look.rs:838-885). Diff: regular. Tier A. Size S. Owns `crates/anim/src/inertialize.rs` limiter, `crates/game/src/fighter_look.rs` observe_tick state
  and slot_pose tail. Accept: `cargo test -p engine-anim --lib ang_limit` x2 (180 deg step at 1080 deg/s takes 10 ticks +-1) and `arena view:` pops=0 in
  product-check. F2P: no limiter; pops 35-55. P2P: golden unchanged; test fighter_anim_same_tick_* green. needs: none.
- [ ] XX4 game: phase-warped attack with a telegraph meter — the swing starts at the outcome cue, back-dated 12 ticks, so 0 ticks of windup show before impact
  (fighter_look.rs:50,786-801); v2 startup is 15 ticks (tables.rs:53). Clip time from attack age through windup-peak and impact markers. Diff: good. Tier A.
  Size M. Owns `crates/game/src/fighter_look.rs` clip_time and cue hunks, `crates/anim/src/events.rs` marker names, `crates/game/src/view_truth.rs`
  telegraph_hand_cm_min. Accept: `cargo test -p engine-game --features app --lib telegraph_warp` x2: hand >= 25 cm from guard by impact minus 3 ticks, impact
  phase error <= 1 tick, stretch <= 2.0x. F2P: windup is 0 ticks. P2P: golden unchanged. needs: EG19c.
- [ ] XX5 game: kill slow-mo through a virtual-time scale — TickAccumulator takes raw wall ms (windowed.rs:906), nothing can slow the picture. Scale the wall
  delta, ramp it, local and replay only. Diff: regular. Tier A. Size S. Owns `crates/game/src/windowed.rs` accumulator hunk, `crates/game/src/fighter_look.rs`
  cue hook. Accept: `cargo test -p engine-game --lib time_scale` x2: 120 wall frames at 0.25 give 30 ticks +-1 and equal state digest at tick 30; netgame
  forces 1.0. F2P: no scale input. P2P: `node tools/proof.mjs --skip-build` prints golden d57b5c8ae3096614 twice. needs: none.
- [ ] XX6 anim: look-at with soft limit, target-switch ease and behind-the-back guard — replaces the hard clamp LF13 asks for; fbik.rs:392 solve_look has no
  ease. Diff: regular. Tier A. Size S. Owns `crates/anim/src/fbik.rs` look hunk, `crates/game/src/fighter_look.rs` head hunk. Accept: `cargo test -p engine-anim
  --lib look_soft` x2: second difference of head yaw <= 0.5 deg/tick^2 over a 0..180 deg sweep, target swap eases in 12 ticks, head never leaves 70 deg.
  F2P: the clamp steps velocity. P2P: golden unchanged. needs: LF12.
Plan owner: XX1 XX3 XX6 = animation.md; XX2 XX4 XX5 = gameplay.md (camera.rs and combat rules table, section 3).

## 4. Plan text

animation.md section 1: fix the table (LM2 row: warp_ground.rs exists, arena uses GroundWarp at arena/src/lib.rs:98) and add "Landed, no window caller:
hit_react, inertialize, ik, fbik, springs, lod (Grep engine_anim in crates/game/src finds only contract and pose)".
animation.md section 3, new "Window pose stages (view only)": slot_pose runs clip sample + fade -> inertialize (LF9, deepen inertialize.rs, no inertial.rs)
-> rotation-additive flinch -> look-at -> hand/foot IK (ik.rs exists, LF12 wires it) -> spring chains -> angular limiter -> pelvis pin. Each stage writes
slerp(current, wanted, influence) (Godot modifier pattern). Stage state advances in observe_tick at 60 Hz and is evaluated at alpha, so slot_pose stays a pure
function of tick and alpha.
animation.md section 4, new hard problem "Hit react must outlast the sim window": sim lock is 6 ticks (100 ms), a readable flinch is about 450 ms; the view
envelope has its own in/hold/out constants and overlapping hits add. Proof: react_head_cm_max and pops in `arena view:`.
animation.md section 4, new hard problem "Telegraph": the sim gives attack age; markers in the clip event track map age to clip time; meter telegraph_hand_cm_min.
Heavy rows, not the warp, carry reaction time (GS32 floor 340 ms).
gameplay.md section 3: "One camera stack: fighter_look::fight_camera calls camera.rs; exact halflife springs; trauma noise is smooth, 3 axes plus roll; FOV punch
on a timed spring; slow-mo scales wall time at TickAccumulator, local only, never in netgame." Correct the "recorded in-sim" sentence (nothing is recorded).
gameplay.md hit-stop row: add "presentation holds alpha and shudders the victim 1-2 cm; the sim counter stays the only freeze". LF12 donor detail: Motion-Matching
ik_two_bone clamps reach by a 0.015 m buffer and uses fwd as pole; Godot TwoBoneIK3D solves a = (l^2 + r1^2 - r2^2) / 2l, picks the mid candidate nearer the pole.

## 5. Not worth it

1. Motion matching for attacks (Unreal Pose Search, Motion-Matching, no Bevy crate exists): needs hundreds of clips; we have 6 (fighter_look.rs:73-80); LM1 already serves locomotion.
2. ACL clip compression: 6 clips per rig and pack.rs exists; no memory problem to fix.
3. Ground-normal foot IK and pelvis drop (Vulkan tutorial, Godot TwoBoneIK3D on terrain): the arena is flat sand; foot lock (warp_ground.rs, LF12) is the value; revisit at siege stairs (TV3).
4. Root-motion-driven movement (Fyrox, Unreal): our capsule is integer and first (clips are in place, pelvis xz pinned at fighter_look.rs:866-869); extraction stays offline in retarget_cook.
5. Hit-stop by engine time scale (saltmire-juice): ours is a sim counter (fight.rs:717); scaling wall time for it would split replay and netplay.
6. Porting Bevy AnimationGraph/AnimationTransitions, Fyrox absm, Godot RetargetModifier3D, GenoView: we have graph.rs, state_machine.rs; retarget.rs:18-23 is the same bind-relative plus bone-length-ratio math; Bevy transitions are the same linear fade; GenoView is a viewer and `arena view:` already measures slide and sinking.

DONE 13 steals, 17 donor files read.
