---
name: anim-combat-rules
description: Use when you change clip playback, pose blending, root motion, foot IK, the ragdoll handoff, character movement, joint limits or limb cuts in crates anim, physics, combat or game, before you claim the body works.
version: 0.1.0
author: skillworks
---

# Animation and body rules

Use when you change clip playback, pose blending, root motion, foot IK, the ragdoll handoff, character movement, joint limits, or limb cuts.

1. Animate from sim state. Never feed an animated float pose into the hash. If an outcome must always happen one way, bake it as a table or animate it. [gearus 13.1.3]
2. Order each frame as anim clocks, then event dispatch, then blend. Any other order costs one lag frame at clip end. [geaen 15.7.9.2]
3. Cross-fade with a binary LERP node on the tree root, or with old group weights times (1 - lambda) and new weights times lambda. Rotations take the shortest arc, as `render-math-checklist` item 4 says. [geaen 11.10.4]
4. A looping clip of N frames stores N samples. Keeping the last sample too repeats the first, and the loop pops. [geaen 11.4.1.4]
5. An additive clip is S times inverse(R), where S is the pose and R the neutral reference. Keep shoulders and elbows neutral in R, or the limbs over-rotate. [geaen 11.6.5]
6. Extract root motion before you zero the root. Scale playback rate to speed. Apply foot IK only in contact windows. Player feel beats animation purity. [geaen 11.11.5.4]
7. Never LERP a live pose into a ragdoll pose. Drive bones with powered (PD) constraints, then release. Go ragdoll as soon as any bone touches solid. [geaen 12.4.8.8, 12.5.3.8]
8. Move characters with capsule casts and a manual slide, step and slope, not forces. Casts never change the world. [geaen 12.5.3.6, 12.3.7]
9. Limit ball joints by swing-twist: swing as an ellipse in quaternion space, twist as a clamped angle. Never use three Euler clamps. [gems3 ch.10 10.5]
10. Remove a limb by flattening its child bone matrices to the cut joint on the shared mesh. Reverse the flattening for the detached limb. [gems1 ch.19 19.3]

## Repo facts

- `crates/anim` and `crates/physics`: about 20k of 27k anim lines and 15k of 26k physics lines have no caller in the game or the arena (bodies review).
- `crates/game`: the running arena is a kinematic match with capsule fighters and a 4-state clip player. No Rapier, ragdoll, IK, anim graph or cut runs in it.

Search more: `node tools/books.mjs search "animation blending root motion"`
