# Skill-on vs skill-off pairs: camera-arm-checks
Three camera-arm reviews. Off is general habit; On names the checklist item and the proof. Pass = On cites the item, the failure, and the test.
## P1: thin ray clips the pillar edge (items 1, 7)
Input: `arm_length` in `crates/game/src/camera_arm.rs` casts a zero-radius ray against `Aabb` from `crates/physics/src/kcc.rs`. Corner passes, lens clips stone.
With skill: Item 1: inflate each box by radius and slab-test the segment, so edges push the arm in. Item 7: reject non-finite input with `CameraError` and prove radius 0.3 toward a face 3.0 m away returns 2.7 m, x2 equal.
Without skill misses: the corner clip and the missing radius term; approves the ray because center-line tests pass.
## P2: pivot starts inside the pillar (items 2, 7)
Input: fighter backed into a pillar, pivot inside its `Aabb`. Arm returns 0 and the view pops to the helmet.
With skill: Item 2: inside start returns the floor length, wired via `pub mod camera` in `crates/game/src/lib.rs`. Item 7: `is_finite` in `crates/game/src/camera.rs` keeps it finite and x2 equal.
Without skill misses: the inside-start case; tests only open ground, so no obstacle returns `max_len` passes and the trap ships.
## P3: side hit swings yaw and leaks into sim (items 4, 6)
Input: side brush swings yaw 40 degrees to dodge, and stores arm length in the fighter state hashed toward `d57b5c8ae3096614`.
With skill: Item 4: side hit only shortens the `Vec3` arm in `crates/game/src/camera.rs`, yaw stays for aim. Item 6: arm stays presentation-only per the header, golden unchanged.
Without skill misses: the control break and the hash move; calls the swing clever and misses that replay now diverges.
