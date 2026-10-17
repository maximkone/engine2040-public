# Browser physics: realistic and cheap (scout 03, 2026-10-17)

GUESS marks unverified claims. Row IDs come from `docs/SPRINT-22-web-realism.md`. Unreadable: the Jolt multicore PDF and the `@babylonjs/havok` npm page (403).

## Ideas

| Idea | Source | Licence | Do this | Row | Effort |
|---|---|---|---|---|---|
| Prove Rapier wasm matches native, with and without simd128 | [Rapier determinism](https://rapier.rs/docs/user_guides/rust/determinism/), [rapier3d features](https://docs.rs/crate/rapier3d/latest/features) | Apache-2.0 | `enhanced-determinism` enables `parry3d/enhanced-determinism` and `simba/libm_force`. Build the ragdoll tape twice, plain and with `RUSTFLAGS=-C target-feature=+simd128`. Compare digests. Default `std` pulls in `wide`, which uses wasm SIMD with that flag (whether Rapier's hot loops use it is GUESS). | WEB3 (add simd128 leg) | S |
| Powered ragdoll muscles as PD motors | [GenericJoint](https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.GenericJoint.html), [MotorModel](https://docs.rs/rapier3d/latest/rapier3d/dynamics/enum.MotorModel.html) | Apache-2.0 | `set_motor(axis, target_pos, target_vel, stiffness, damping)` plus `set_motor_max_force`. Keep default `AccelerationBased` (the page calls it recommended; springs scale with mass). Set `motor_axes` and `limit_axes` or nothing applies. If it oscillates, try stable PD ([2011 paper page](https://tml.stanford.edu/publications/2015/stable-proportional-derivative-controllers)). | RZ2 | M |
| Sleep settled debris; lifetime and cap per break | [RigidBodyActivation](https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.RigidBodyActivation.html), [Torque3D PhysicsDebris](https://reference.torque3d.org/scripting/class/classphysicsdebrisdata), [Unity sleeping](https://docs.unity.com/en-us/engine/6000.0/manual/physics-section/physics-overview/physics-optimization/cpu/rigidbody-sleeping) | Rapier Apache-2.0; Torque3D and Unity licence not read | Rapier defaults in physfx. Settled debris goes static or leaves the island. Each piece gets base lifetime plus random variance, one fragment ceiling per break, and pooling. Audit constant forces: they stop sleep (Unity). | RZ1, RZ4, RZ9 | S |
| Severed limb: joint break limits, authority decides the cut | [Unity CharacterJoint](https://docs.unity3d.com/400/Documentation/ScriptReference/CharacterJoint.html) (force and torque limits) | Unity docs | physfx may break its own joint at a force limit so the limb falls. The 16-byte cut event stays the truth. Stump cap mesh: GUESS, no source found. | RZ3 | M |
| Jolt wasm as a second backend | [JoltPhysics.js](https://github.com/jrouwe/JoltPhysics.js) | MIT | Use `wasm-compat` (single thread, base64). Soak the RZ1 8-ragdoll scene on Rapier wasm and Jolt wasm. Build Jolt with `CROSS_PLATFORM_DETERMINISTIC=ON`. Switch only if Rapier misses the bar. | NEW RZ10 | L |
| Wasm SIMD on/off in the browser | [Godot web article](https://godotengine.org/article/upcoming-serious-web-performance-boost/) | article | Time the physfx step with and without simd128 in the web receipt. Godot's Jolt test: 1.37x Chromium 134, 2x Firefox 138, one scene, 13900K and RTX 4090 Linux box. | WEB8 | S |
| Threads later, behind isolation | [wasm-bindgen-rayon](https://docs.rs/wasm-bindgen-rayon), [coi-serviceworker](https://github.com/gzuidhof/coi-serviceworker) | coi-serviceworker MIT; rayon not read | Needs nightly, atomics, bulk-memory and build-std, pinned (tested on nightly-2024-08-02). coi-serviceworker supplies the headers on GitHub Pages but reloads the page on first visit. Check `crossOriginIsolated` in WEB10. | WEB10; NEW WEB12 | L |

## Top 5 this week

1. WEB3: add the simd128 digest leg before any RZ row trusts Rapier on the web.
2. RZ2: motor PD on `AccelerationBased`, mask bits set, driven inside the fixed 60 Hz step (a Godot forum report says an active ragdoll goes random when the tick drops below 60).
3. RZ1: Rapier sleep defaults; debris goes sleep, then static, then lifetime.
4. RZ9: body caps from the budget numbers below, then the soak sets the real values.
5. RZ3: cut is authority; joint break limits are presentation only.

## Traps

- rapier.js standard build "does not guarantee cross-platform determinism"; its deterministic build is "less optimized" with no numbers. The repo was archived 2026-07-12 and merged into `dimforge/rapier`. Use the Rust crate.
- Rapier says IEEE 754 is met by "WASM targets" but does not say it tests WASM. WEB3 is the test.
- Do not enable relaxed-simd. A search summary of the wasm design debate says relaxed fused multiply-add may round once or twice per engine (GUESS on final spec text).
- JoltPhysics.js README: "only been compiled under Linux"; objects need `Jolt.destroy`; its SIMD browser note is stale (2023 wording).
- Havok: README has no licence; npm 403; Unity's Havok package says Microsoft owns the copyright under a limited-term agreement (search summary). Do not ship until read.
- Box3D is alpha per Catto (search summary); its joint grid went unstable in the Godot bench. Watch only.
- Godot bench numbers are native, hardware not reported, body counts not given. Do not copy them as browser numbers.

## Budget numbers

- Rapier sleep: linear 0.05 units/s, angular 0.5 rad/s, sleep after 0.5 s ([docs.rs](https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.RigidBodyActivation.html)).
- Ragdoll: Jolt's ragdoll scenes list 3680 bodies for 160 ragdolls, so about 23 bodies each (derived; [Jolt PerformanceTest.md](https://github.com/jrouwe/JoltPhysics/blob/master/Docs/PerformanceTest.md)). 8 ragdolls ≈ 184 bodies.
- physfx total: 8 ragdolls + 200 debris ≈ 384 bodies (derived from the RZ1 bar).
- Tick: 4 ms (RZ1 bar, our board) is 24% of a 16.7 ms frame at 60 Hz (arithmetic).
- Native Rapier3D 0.35, ms (cores): pyramid 14.6 (2.8), smash 19.1 (2.8), mixed pile 11.3 (2.1), query storm 5.9 (1.0); Box3D v0.1 pyramid 13.5, smash 9.6 ([Apps in a Cup bench](https://godot.rapier.rs/blog/v0-35-0)). Body counts and hardware not given.
- Web bars (WEB11, our board, not externally sourced): wasm under 25 MB, first bundle under 60 MB compressed, peak memory under 1.5 GB, first frame under 8 s.
- Contacts: no source found. GUESS: set the contact cap from the RZ9 soak.
