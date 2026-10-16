---
name: camera-arm-checks
description: Use when a builder wires a third-person camera arm in crates/game that must shorten at walls and pillars before the frame ships.
---
# Camera arm checks
Eight checks for a shortening arm. Name the crate in each finding (game, physics). PASS needs a length test plus x2 equal, never a picture alone.
1. **Sweep with radius.** Ask: does boom distance in `crates/game/src/camera.rs::DirectorParams` (`base_dist_m` inside `min_dist_m`/`max_dist_m`, `boom_up`/`boom_back`) shorten against each `Aabb` from `crates/physics/src/kcc.rs::Aabb` by radius and slab-test the segment? Flag: zero-radius ray that clips pillar corners. (game)
2. **Floor on inside start.** Ask: does a pivot inside a box in `crates/game/src/camera.rs::DirectorParams::validate` return the floor length (`min_dist_m`), wired via `pub mod camera` in `crates/game/src/lib.rs`? Flag: zero or NaN that traps the lens in stone. (game)
3. **Front block has a rule.** Ask: when a pillar stands between pivot and fighter, does `crates/game/src/camera.rs` zoom in or fade the blocker, never pierce it? Flag: lens through the blocker or a wide swing around it. (game)
4. **Keep heading, shorten length.** Ask: does a side hit only shorten the `Vec3` arm in `crates/game/src/camera.rs`, keeping yaw outside battle aim? Flag: large yaw kick on light contact that breaks controls. (game)
5. **Cap the pitch.** Ask: does vertical adjust in `CameraFrame` in `crates/game/src/camera.rs` stay small so the horizon stays readable? Flag: top-down snap that shows only the helmet. (game)
6. **Look only, never sim.** Ask: is the arm kept out of the hash with golden `d57b5c8ae3096614` unchanged, per the header in `crates/game/src/camera.rs`? Flag: arm length in state or replay bytes that moves the hash. (game)
7. **Same inputs, same length.** Ask: does `DirectorParams::validate` reject non-finite input with `CameraError` via `is_finite` in `crates/game/src/camera.rs` and test x2 equal? Flag: NaN pivot with a new length each run. (game)
8. **Cheap and heap-free.** Ask: is cost O(boxes) with no alloc in the loop, inside the 0.05 ms budget in `crates/game/src/camera.rs`? Flag: per-box `Vec` or `format!` that grows with pillars. (game)
## Do / Don't
- Do: shorten the arm along its arc and ease back when clear. Don't: swing yaw or pitch to dodge stone.
source: Gregory, Game Engine Architecture (2nd ed.), ch. 12.5.3.7
