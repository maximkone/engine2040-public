STEAL CARD, wave 2026-10-18/5, position 35/50. Read only, no bytes vendored.
Donor: kpreid/all-is-cubes @ 7db4f7580af5 (main, 2026-10-16T22:00Z, 238 stars) https://github.com/kpreid/all-is-cubes
Source list: github.com/topics/game-engine?l=rust&o=desc&s=updated (position 35).
Every reader below was fetched live on 2026-10-18. Golden d57b5c8ae3096614 is P2P on every row.

== 1. WHAT IT IS ==
A block/voxel game where the blocks are made of blocks, running natively and in browsers on
WebGPU + WASM. Split into twelve crates: base (math, raycast, time), mesh (block meshing incl. a planar
triangulator), content, port, gpu, render (a software ray tracer AND a wgpu renderer), ui, desktop,
server, wasm. Two things stand out for us: it renders the SAME scenes through SEVERAL renderers through
one test harness, and its math is built from no-NaN range newtypes.

== 2. LICENCE ==
MIT OR Apache-2.0. LICENSE-MIT line 1 reads "MIT License / Copyright 2020-2026 Kevin Reid" and
LICENSE-APACHE is present in the tree (both read 2026-10-18). Per-crate copies exist
(all-is-cubes-base/LICENSE-MIT, all-is-cubes-mesh/LICENSE-MIT, ...). Dual-licensed, take either.

== 3. STEALS (ranked by value to the duel per hour) ==
1. A cross-renderer golden-image harness with a structured comparison record and an HTML report | test-renderers/types/src/comparison.rs:19-58,70,87,103,123 ; test-renderers/runner/src/harness.rs:28-45 ; test-renderers/expected/renderers/<case>-<renderer>.png (tree read 2026-10-18) | MIT OR Apache-2.0 | PRESENTATION (pixels; the tick never sees an image) | crates/proof/src/matrix.rs, crates/proof/src/gpu_live.rs, crates/proof/src/fb_ship.rs, crates/proof/src/arena_matrix.rs, crates/playtest/src/look/{manifest,metric,judge}.rs, tools/frame-moved.mjs | M
   Idea: one scene is rendered by every renderer (the expected names encode both axes:
   `<case>-<renderer>.png`, renderer in {all, ray, wgpu, gltf, surf, vol}), and each run writes a
   ComparisonRecord (:21) holding the expected image, the actual image, an optional DIFF image, a
   256-long diff histogram (:25) and a ComparisonOutcome (:45): Equal, Different{amount: u8}, NoExpected,
   or Flawed(String). is_flawed (:73), could_be_overwritten (:87) and describe_failure (:123) make the
   result queryable by a machine, and the runner emits a human report (report.rs + report.template.html
   + report.css in the tree) beside a results JSON. The harness takes HarnessArgs (:36: --format pretty
   or json, --ignored).
   Maps onto us: proof/src/matrix.rs and arena_matrix.rs already hold golden matrices and gpu_live.rs /
   fb_ship.rs drive GPU facts; playtest/src/look/ has manifest/metric/judge. The steal is the RECORD
   SHAPE — expected/actual/diff + a 256-bin histogram + a small outcome enum — instead of a bare
   pass/fail, so a failing gate tells you HOW different (amount) and shows the diff, and a "no expected
   yet" state is not a failure. That is test-and-proof's "pixels not manifests" with a receipt a judge
   can read. The per-renderer file naming is also the pattern for our tiers: one case, one expected file
   per tier (potato / base / ultra) instead of one number for all.
   Moves: a pixel gate answers pass/fail -> + diff amount, histogram and diff image; expected-missing
   is an error -> its own outcome; tier differences visible -> one expected file per tier. Proof:
   `cargo test -p engine-proof --lib image_comparison` x2 on the arena fixture: identical renders are
   Equal, a one-pixel change is Different{amount=1}, a missing file is NoExpected; arena PNG hashes
   unchanged.
   Kill: if a diff histogram needs a new image dependency in the fixed-tick path, keep it in proof
   (offline) only; never in crates/ecs or crates/game.

2. A Flaws type: a renderer declares its known deficiencies, and a difference it explains counts as
   Flawed, not Different | all-is-cubes-render/src/flaws.rs:1-45,93-101 ; test-renderers/types/src/comparison.rs:45-60 (read 2026-10-18) | MIT OR Apache-2.0 | PRESENTATION | crates/render/src/tier.rs, crates/render/src/tier_rules.rs, crates/render/src/webgpu_limits.rs, crates/render/src/dynres_gov.rs | S
   Idea: Flaws (:20) is a u16 bitflags set of ways a rendering may fail to match the scene: OTHER,
   TOO_COMPLEX, UNFINISHED (:33), UNSUPPORTED (:38) and more, with contains_resource_limitation (:98) as
   the one that "shouldn't happen". The comparison consumes it: when the images differ but the renderer
   signalled a flaw, the outcome is Flawed(String) rather than Different (:55-58), and a flaw that claims
   a resource limitation is called out specifically (:60).
   Maps onto us: our tiers already admit differences (potato vs base vs ultra; webgpu_limits.rs is the
   same "what the device cannot do" idea) but the harness has nowhere to SAY it, so a tier difference and
   a regression look the same. The steal is the explicit, named insufficiency that the gate can consume:
   a renderer returns the set of things it knows it did not do, the gate accepts exactly those, and any
   difference outside the declared set still fails. In a receipt, "flawed: UNSUPPORTED(msaa4x)" is a
   fact; a silent mismatch is not.
   Moves: a tier difference is a fail or a hand-waved pass -> Flawed(named set); a device limit named in
   code only -> named in the gate outcome. Proof: `cargo test -p engine-render --lib flaws_declared` x2:
   a difference in a declared flaw is Flawed, the same difference with an empty flaw set is Different.
   Kill: a Flawed outcome must still be listed in the receipt and may never silence a regression — if
   two runs of the SAME tier and the same declared flaws differ, it is a fail.

3. No-NaN and range-bound numeric newtypes for math | all-is-cubes-base/src/math/restricted_number.rs:13-35,37-45,58,123,200-203,278 (read 2026-10-18) | MIT OR Apache-2.0 | SIM (types only; no behaviour change without a receipt) | crates/core math, crates/anim (blend weights), crates/render (colour/alpha) | S
   Idea: PositiveSign<T> (:35) is a float that is not NaN and whose sign bit is positive, so it is Eq
   (safe as a cache/interning key, :16-19) AND closed under multiplication and addition (:20-21, mul
   :278), unlike NotNan which can panic. ZeroOne<T> (:45) is a float forced into +0..=+1, for alpha,
   reflectance and lerps; it has ZERO/ONE consts (:200-203), clamp_01 (:123) and a complement (:189).
   Both expose into_inner (:78) and a checked constructor, with new_unchecked marked unsafe (:58).
   Maps onto us: our hash law needs finite, byte-stable numbers — one NaN in the tick and two runs
   differ; our anim blend weights live in 0..1 and our render reads alpha and colour. The steal is the
   TYPE-level guarantee instead of a runtime assert: a value that cannot be NaN and cannot leave its
   range, so the invariant is enforced where it is created, and Eq is honest enough to key a cache.
   Moves: a NaN or out-of-range lerp weight is a runtime assert (or nothing) -> a type that cannot hold
   one; NaN as a hash input -> impossible for these fields. Proof: `cargo test -p engine-core --lib
   restricted_number` x2 plus a golden x2 identical run.
   Kill: if the newtype changes any value the tick currently produces (the donor's "0 * inf = 0" rule is
   NOT identical to f32, restricted_number.rs:27-33), it does not land — adopt it for NEW fields only
   until a receipt shows the tick numbers are unchanged.

== 4. NOT WORTH IT ==
- The wgpu/WebGPU backend and the software ray tracer: donors for a voxel game, not a duel renderer;
  our crates/render already owns that ground.
- The planar triangulator / mesh outliner (all-is-cubes-mesh/src/planar/*): clever, but our meshes are
  cooked offline from kit geometry (assets/src/mesh_cook.rs), and there is no runtime polygon problem.
- Its UI crate and the desktop/server split: a different architecture, no duel use.
DONE 3 steals, 6 donor files read.
