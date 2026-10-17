# 05 Ragdoll get-up and limb overlap (research, 2026-10-17)

Scope: four gaps in the ragdoll stack: (1) limb interpenetration, (2) getting up from a
ragdoll, (3) partial/active ragdoll and hit reactions, (4) dismemberment and gore caps.
Repo pins rapier3d 0.36 (Cargo.toml:193).

Evidence tags: [F] fetched and read this session. [S] search snippet only (page not read or empty). [U] unverified.

## What I could not verify

- DeepMimic (arXiv 1804.02717): the PDF came back as compressed bytes. No per-joint gains. [U]
- Bollo, Gears of War inertialization (GDC session and SIGGRAPH 2017 PDF): PDF unreadable.
  The "60% cheaper" figure is a snippet only. [S]
- CEDEC 2010 ragdoll deck: the URL redirected to a 2018 index. Not read. [S]
- Shrimple Ragdolls and the Godot forum thread: near-empty fetches. Snippets only. [S]
- docs.rs and rapier.rs describe the latest Rapier, not 0.36. The defaults below may
  differ on 0.36. Confirm against the 0.36 source before tuning anything.
- No verified Rapier mass-ratio number and no verified humanoid stiffness number.
- Human joint ranges (knee, elbow) are from memory, not fetched. [U]

## Repo facts checked (for the mapping)

- crates/physics/src/ragdoll.rs:1120-1150 `insert_joints`: every bone is a SphericalJoint
  to its parent, `contacts_enabled(false)`, and AngX/AngY/AngZ limits of +-swing_limit(depth).
  physfx calls this (crates/physfx/src/fx.rs:223), so physfx knees and elbows use the same
  cone. There is no hinge on this path.
- ragdoll.rs:2053-2058 (tests): swing_limit(1) = 0.75 rad, swing_limit(2) = 0.6 rad.
  SWING_ROOT 0.9, SWING_STEP 0.15, SWING_MIN 0.2 (lines 56-60). Derived, not run: with the
  pelvis as root, depth 3 gives 0.45 rad and depth 4 gives 0.3 rad. The floor applies from depth 5.
- ragdoll.rs: no motor calls (grep). A hit is a center-of-mass impulse on one bone (lines 17-19).
  `blend_to_ragdoll` weight runs 0 (animated) to 1 (sim); COUPLING_GETUP_BUDGET_TICKS = 60 (line 69).
- crates/physfx/src/caps.rs:51-86: Potato/Web/Comfort/High give solver_iterations 2/3/4/6,
  ccd_substeps 0/1/1/2, contacts 256/512/1024/2048. fx.rs:115-116 applies these.
- crates/physfx/src/fx.rs:218 capsule colliders with no groups set. fx.rs:263 already has
  `GroupId::Limb`.
- crates/game/src/ragdoll_look.rs: 15 bones (line 10), MIN_LYING_TICKS 45 (line 59),
  GETUP_HALFLIFE_S 0.25 (line 64), GETUP_TICKS 150 (line 67), NS_PER_PERMILLE 0.1 and
  MAX_DAMAGE_PERMILLE 600 (lines 48, 54), so the root impulse tops out at 60 N.s.
- Not checked: whether ragdoll_look picks the get-up clip by lying orientation. Grep first.

Note on hashing: crates/physfx is float-only and never hashed. Rows that touch only physfx
leave the golden alone. Any change to crates/physics/src/ragdoll.rs changes hashed state.

## Techniques

### 1. Solver iterations and CCD budget

- Source: https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.IntegrationParameters.html [F]
  Defaults stated: num_solver_iterations 4, num_internal_pgs_iterations 1,
  num_internal_stabilization_iterations 1, normalized_max_corrective_velocity 3.0,
  max_ccd_substeps 1, min_ccd_dt 1/6000 s, dt 1/60 s.
- Licence: Rapier is Apache-2.0 / MIT per its repo. [U, from memory]
- Our numbers: Potato runs 2 solver iterations and CCD off. Two iterations on a 15-capsule
  chain is the likeliest overlap source on Potato. Not measured.
- Map: caps.rs:51-86, fx.rs:115-116.
- Cost: S. Measure penetration at 2/3/4/6 iterations. Row QR1.

### 2. Neighbour contacts: what is already off, and what is left

- Source: https://rapier.rs/docs/user_guides/rust/colliders [F]: collision_groups filter
  before the narrow phase, solver_groups after it. Both masks must pass. Default is all bits set.
- Our state: every joined pair has contacts off (ragdoll.rs:1143), so parent-child pairs
  never collide. Not covered: grandparent pairs (forearm vs torso, hand vs head) and
  left/right thigh pairs. Those are the pairs that interpenetrate.
- Warning: a Godot forum poster reports that self-colliding every bone gave jitter and
  stopped bodies sleeping. [S, page broken] Filter only pairs that overlap in the standing pose.
- Map: fx.rs:218 capsules, add membership bits per bone in physfx only.
- Cost: S. Row QR2.

### 3. Swing limits: one depth cone for every joint (finding from code)

- Source: https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.GenericJoint.html [F]:
  `set_limits(axis, [lo, hi])`, and "the limit must also be explicitly enabled by the
  limit_axes bitmask".
- Our state: knee and elbow get the same +-limit cone as the hips (ragdoll.rs:1140-1146).
  By the formula, a depth-2 knee gets +-0.6 rad (about 34 deg). A human knee flexes about
  140 deg. [U] Consequence: legs cannot fold, and limbs may pass straight through each
  other instead of folding. Verify with a folded-leg spawn test before changing anything.
- Map: ragdoll.rs:1197-1290 already has a hinge JointSpec on the Ragdoll path. physfx skips
  it because it calls `insert_joints`.
- Cost: M. Row QR3.

### 4. Mass ratio and damping

- Sources: none verified. Our values: LINEAR_DAMPING 0.05, ANGULAR_DAMPING 0.05
  (ragdoll.rs:62-64), FRICTION 0.9 (line 66).
- Forum figures of 3:1 to 5:1 for parent/child mass ratio [U]. Not checked.
- Map: physfx density via `flesh.density()` (fx.rs, capsule_mass).
- Plan: log per-bone mass and the parent/child ratio first. Tune only after measuring.
- Cost: S. Row QR4.

### 5. Spawn from a non-overlapping pose

- Source: https://pulsegeek.com/articles/ragdoll-setup-and-stability-tips-for-reliable-collisions
  [S] Enable the ragdoll on a frame where colliders do not overlap. Switching on while
  interpenetrating makes the solver blast limbs apart. Generic guide, not Rapier-specific.
- Map: fx.rs:167 `spawn_ragdoll` takes the standing pose unchecked. A wide-stance pose
  with leg overlaps would blast on spawn.
- Cost: S. Row QR5.

### 6. Fast limbs tunnel: soft CCD and our truncate pass

- Source: Rapier changelog, mirror https://git.nea.moe/github/rapier.git/plain/CHANGELOG.md?h=v0.21.0
  [S] Soft CCD (predictive contacts) added in v0.19.0, described as generally cheaper than
  time-of-impact CCD. Present in 0.36 is [U].
- Source: min_ccd_dt 1/6000 s [F] (item 1).
- Our state: ragdoll.rs:31-37 Divide-and-Truncate (idea from arXiv 2604.15513) already
  guards coupling. physfx sets CCD substeps only.
- Cost: M. Row QR6.

### 7. Get-up blend: inertialization (already in place; test it)

- Source: https://www.theorangeduck.com/page/spring-roll-call [F]: inertialization offsets
  decay with no second animation evaluated. Decay: y = halflife_to_damping(halflife)/2, with
  halflife_to_damping = 4 ln2 / halflife. The article gives no halflife values.
- Gears of War: GDC Vault https://gdcvault.com/play/1025165/Inertialization-High-Performance-Animation-Transitions
  [S, listed as GDC 2018]. SIGGRAPH 2017 PDF https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf [S, "60% cheaper" snippet].
- Our numbers: GETUP_HALFLIFE_S 0.25 s, GETUP_TICKS 150 = 10 halflives = 2.5 s. Residual
  2^-10 (about 0.1%) matches the code comment. Clip switch 0.08 s (ragdoll_look.rs:61).
- Map: ragdoll_look.rs lines 7-8 and 36 (`apply_offsets`, `begin_switch`).
- Cost: S. Row QR7.

### 8. Pick the get-up clip by lying pose (front/back, nearest start frame)

- Unity write-up https://www.moddb.com/news/blending-ragdoll-physics-and-animation-in-unity
  [S] compares root up vector to world up. Front and back only.
- CEDEC 2010 deck https://2018.cedec.cesa.or.jp:443/2010/archive/files/C10_I0052/C10_I0052.pdf
  [S, not read] compares orientation with several get-up start frames, drives toward the
  closest, switches to the clip near it.
- CryEngine "Fall and Play" https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1638401/pages/1048781
  [S] nearest stand-up clip by pose.
- Numbers: none fetched.
- Map: ragdoll_look.rs `ragdoll_fold_gait` (around line 741), MIN_LYING_TICKS 45.
- Cost: M. Row QR8.

### 9. Fade the sim into the animated pose before switching

- Source: https://sbox.game/fish/shrimple_ragdolls [S, page near-empty] `StartLerpBodiesToAnimation(duration, targetMode)`
  lerps bodies to the current pose, then switches mode.
- Map: ragdoll.rs blend weight and COUPLING_GETUP_BUDGET_TICKS 60 (1 s at 60 Hz). physfx uses
  inertial offsets instead (item 7). Compare the two on one fall.
- Cost: S. Row QR9.

### 10. PD motors with a strength multiplier for partial ragdoll and hit recovery

- Rapier joints guide https://rapier.rs/docs/user_guides/rust/joints [F]: motors are PD
  (stiffness toward target position, damping toward target velocity), via
  `configure_motor_position(target, stiffness, damping)` and `configure_motor_model`.
- GenericJoint docs [F]: `set_motor`, `set_motor_max_force` ("the maximum force the motor
  can deliver"), `set_motor_model`. Motor axes must be enabled.
- Unreal PhysicalAnimationComponent https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UPhysicalAnimationComponent
  [S] strength multiplier that can be blended 0 to 1.
- Numbers: Rapier gives no default stiffness. Residual Force Control paper
  https://arxiv.org/pdf/2006.07364 [S snippet] reports kp 200-1000 and kd = 0.2 kp for a
  humanoid sim. Different model and units. Use only as a starting scale.
- Map: physics/src/ragdoll.rs has no motor calls. Build this in physfx first (float, not hashed).
  A hit sets a strength spike that fades.
- Cost: M. Row QR10.

### 11. Partial ragdoll and hit reaction spread

- Source: Shrimple Ragdolls [S] partial ragdoll on chosen bones. Procedural hit reaction
  with radius, duration and strength. No numbers read.
- Map: ragdoll.rs:17-19 applies one COM impulse. Our scale: NS_PER_PERMILLE 0.1 and
  MAX_DAMAGE_PERMILLE 600 cap the root impulse at 60 N.s (ragdoll_look.rs:48, 54).
- Proposal: spread the same total impulse to bones inside a radius, mass-weighted, so no
  energy is added.
- Cost: S. Row QR11.

### 12. Dismemberment: break the joint, spawn the limb, cap the stump

- Unity CharacterJoint https://docs.unity3d.com/Manual/class-CharacterJoint.html [F]: Break
  Force and Break Torque ("the force that needs to be applied for this joint to break").
  No default and no behaviour stated.
- Rapier GenericJoint docs [F]: no break threshold. Breaking is user code reading impulses.
- L4D2-style approach (shader clipping, gore caps, gibs): https://luzemrik.itch.io/gutbox/devlog/475023/some-info-on-the-gore-tech
  [S, dev post, not primary]. Commercial Unreal plugin listing (not used):
  https://www.unrealengine.com/marketplace/en-US/product/dismemberment-and-bone-break [S].
- Map: fx.rs:263 `GroupId::Limb` exists for blast limbs. A severed limb means: remove the
  joint, move that bone's capsule into a Limb group, spawn a stump cap on the look
  (ragdoll_look.rs, game-side).
- Cost: L. Row QR12.

## Proposed board rows (not added to the board)

- [ ] QR1 physics: measure ragdoll penetration per tier (solver 2/3/4/6, CCD 0/1/1/2) from a fixed 15-bone drop. Diff: probe and receipt field in physfx, no tuning change. Tier A. Size S. Owns crates/physfx/src/caps.rs, crates/physfx/src/tests/contacts.rs. Accept: receipt prints max penetration per tier. F2P: new contacts test that the probe field is finite. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR2 physfx: filter only grandparent pairs that overlap in the standing pose, via per-bone membership groups. Diff: group bits in physfx capsules. Tier A. Size S. Owns crates/physfx/src/fx.rs. Accept: standing-pose overlap count is 0 for the 15-bone rig. F2P: new test on that count. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR3 physfx: knee and elbow hinge limits instead of the depth cone. Diff: physfx builds hinge joints from the existing JointSpec for knee and elbow. Tier A. Size M. Owns crates/physfx/src/fx.rs, crates/physfx/src/rig.rs. Accept: a folded-leg spawn reaches the target knee angle. F2P: new test that a knee bends past 0.6 rad. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR4 physfx: log per-bone mass and parent/child mass ratio in the receipt. Diff: receipt fields only. Tier A. Size S. Owns crates/physfx/src/fx.rs. Accept: receipt has one ratio per joint. F2P: new test reads the ratios. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR5 physfx: log overlap count of non-joined pairs at spawn. Diff: spawn-time count only, no nudge. Tier A. Size S. Owns crates/physfx/src/fx.rs. Accept: wide-stance pose logs a nonzero count. F2P: new test on that case. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR6 physfx: soft CCD on the High tier only, after a feature check on rapier 0.36. Diff: flag in caps.rs for High. Tier A. Size M. Owns crates/physfx/src/caps.rs, crates/physfx/Cargo.toml. Accept: fast-limb drop does not tunnel at High, Comfort unchanged. F2P: new tunnelling test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR7 game: first-frame get-up error test at halflife 0.25 s. Diff: test only. Tier A. Size S. Owns crates/game/src/ragdoll_look.rs. Accept: first-frame joint error is under the 10 cm the code comment states. F2P: new test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR8 game: pick the get-up clip by lying orientation (front/back, then nearest start frame). Diff: scoring over the clip set. Tier A. Size M. Owns crates/game/src/ragdoll_look.rs. Accept: the clip matches the lying side in a table of four poses. F2P: new table test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR9 game: A/B the 60-tick sim-to-animation blend against the inertial path on one fall. Diff: test and receipt line. Tier A. Size S. Owns crates/game/src/ragdoll_look.rs. Accept: both pops measured and written to the receipt. F2P: new test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR10 physfx: per-joint PD motor with a strength multiplier that fades after a hit. Diff: motor setup and fade curve in physfx. Tier A. Size M. Owns crates/physfx/src/fx.rs, crates/physfx/src/rig.rs. Accept: strength goes 1 to 0 over 0.3 s after a hit. F2P: new test on the curve. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR11 physfx: spread the hit impulse over bones inside a radius, total unchanged. Diff: impulse split in fx.rs. Tier A. Size S. Owns crates/physfx/src/fx.rs. Accept: summed impulse equals the input to 1e-4 Ns. F2P: new test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] QR12 physfx and game: sever a limb (remove joint, Limb group, stump cap). Diff: fx.rs remove and look-side cap. Tier A. Size L. Owns crates/physfx/src/fx.rs, crates/game/src/ragdoll_look.rs. Accept: the severed limb falls as its own group and the parent keeps 14 bones. F2P: new test. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

Order: QR1 and QR3 first (measure overlap per tier, test folding limits); QR7 and QR8 next for the get-up; QR10 after the physfx side is measured.
