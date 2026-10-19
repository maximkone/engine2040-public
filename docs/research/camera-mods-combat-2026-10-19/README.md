# Camera tricks, game mods, PCG and dueling mechanics — hunt 2026-10-19

Owner ask: "for camera what tricks or plugins can we play with? Hunt mods and
plugins on GitHub and online (ModDB, PCG, open stuff) for cool ideas to add to
our factory or bridge. In parallel: the game Blade Symphony — similar mechanics,
easy for AI to author and 100x deeper. Research that and arXiv too."

Read date: 2026-10-19 (all sources). Method: websearch, then primary pages
(README, docs, paper abstract + one section). Zero bytes vendored. Every
suggestion is an idea with a named home in our tree.

## What we already have (so the steals are additive)

- `crates/game/src/camera.rs` — the frozen fight director: `DirectorParams`,
  `CameraMode`, `frame_pair`/`fit_pair`, `select_lock_target`, `shake_offset`,
  `count_unframed`, `free_cam_step`, `ARENA_PILLAR_BOX`/`arena_boom_obstacles`.
- `crates/game/src/camera_rig.rs` — staged rig (follow -> lock -> shake),
  `DampedSync`, trauma squared gain, value-noise `smooth_trauma_offset` (offset +
  roll), `FightCamSpring` (velocity-compensated halflife, `MAX_PAN_RAD_S` whip
  cap), `fight_targets` (side view of the pair).
- `crates/game/src/camera_arm.rs` — `arm_length` sphere sweep at pillars/walls.
- `crates/game/src/camera_wish.rs` — WASD relative to the camera's floor heading.
- `crates/game/src/camera_feed.rs` — `PresentationFeed`/`SmoothedFeed` under the
  window loop.
- `crates/spatial/src/compose.rs` — scores a camera shot (never proposes one).
- `crates/combat/src/cut.rs` — joint-targeted cuts (`BladeSwing`, `JointTarget`,
  sharpness/energy/wear, `try_resolve_swing`).
- `crates/combat/src/exchange.rs` — parry window (4 ticks), clash bind (6),
  riposte (6 ticks, 140 permille), poise pools. Clash and parry already exist.
- forge `bridge/CameraPlacer.cs` — scene-aware placement (yaw/pitch/spring pull
  search, 27-sample visibility, floor/solid classify, `Pillar()` for skinned
  rows), `CameraFraming.cs` `CameraShot.Fit`, `CaptureCamera.cs`,
  `CaptureVisibility.cs`.
- Rows already open or planned: GP14 (third-person melee framing), GP25
  (play-step follow camera), GP42 (free camera), GS41 (raw mouse free camera),
  QL8 (feel camera: rotation-only trauma, hard cap), FV3 (first-person view),
  SPR10/SPF2 (bridge camera fit, closed-form placement).

## The hunt, ranked (one line each; detail in the topic files)

Tier A — small, math-only, straight into a file we own:

1. **Directional shake per event, not one noise ball.** A knockdown shakes
   vertical, a cross slash shakes horizontal (Guilty Gear Xrd, cited in the
   impact-feel paper). We have omnidirectional `smooth_trauma_offset`; add an
   event axis. `camera_rig.rs`, row QL8.
2. **Dead-zone "framed" follow.** The camera holds still until the target
   leaves a rectangle. One AABB test, big calm win. Phantom Camera `Framed`.
   `camera_rig.rs`, GP25.
3. **Whiskers + DOF priority, not pull-in alone.** Swing yaw out of an obstacle
   early (Journey/GDC "50 camera mistakes"); when the player fights the swing,
   pull in instead. Order the 7 DOF by priority. `camera_arm.rs` + rig, GP14.
4. **FOV as a function of pitch.** Pull out and widen when looking down, narrow
   when looking up (GDC 14/15). `DirectorParams` curve, GP14.
5. **Charge tell = pose.** Every tier/stance must be readable from the pose.
   `anim_bridge.rs` clip pick, combat rows.
6. **String counter (the Blade Symphony crown jewel).** One small integer per
   stance, shared across stances; attacks advance it, dash advances, shuffle
   steps it back. A combo becomes a path through a shared automaton. Table data,
   AI-authorable. `crates/combat`, `crates/rules`.
7. **Damage vs. preparation-time curve.** Quick / optimal / late zones; a rushed
   strike is weak, a held one is strong, an over-held one loses power
   (Overgrowth design doc). One curve, huge feel.
8. **Startup interrupt amplifies damage.** Hit a swing during windup and it hurts
   far more (Movie Battles II: 1.6x bare, 1.2x swingblocked). Punishes mashing.

Tier B — a system, needs a caller:

9. **Group reframe / clear-shot.** Score several yaws/distances so all live
   fighters fit, pick the best, blend on change. Cinemachine ClearShot +
   Phantom Camera Group; our `select_lock_target` is the seed. GP14.
10. **Impulse channels with travel and falloff.** Cinemachine Impulse: impulses
    per channel, dissipating over distance, propagating at a finite speed, an
    authored shape curve (Recoil/Bump/Explosion/Rumble). Fits our presentation
    layer, never the tick.
11. **Photo mode in the bridge.** Free camera while paused, roll, FOV, aperture,
    filters, rule-of-thirds grid, hide UI, high-res hotsampling, 360/stereo,
    cubemap export (Unity PhotoMode, UE PhotoModeFX, BG3 screenarchery list).
    New door verb + `CaptureCamera`.
12. **Replay keyframe path with event markers.** ReplayMod: record the session,
    replay from any angle, keyframe timeline, camera roll, tripod cameras named
    from the hotbar, render to video. We have `--camera-path`; steal the marker
    overlay (mark hits/parries so a capture lands on the right tick).
13. **An LLM/tool path from a sentence to camera keyframes.** LensCraft (open
    code + weights), ChatCam (CineGPT + anchor determinator), Camera Diffusion
    (text -> camera motion, keyframe constraints). Offline, in the bridge, beside
    `CameraShot.Fit`. P5, and exactly what the bridge already does for shots.
14. **WFC for level tiles.** `mxgmn/WaveFunctionCollapse` (used by Bad North,
    Townscaper), `krychu/wfc` single-file C (fast, measured solves). Bake-time
    only: the tick hash needs a fixed seed.
15. **Constraint-first layout.** Fix entrances/exits first, dig a tunneler
    backbone backward to them, add optional barriers (Cogmind devlog). Directly
    useful for `crates/world` bake and `nav`.

The one donor from outside games and engines: the virtual-cinematography papers
(9, 13) and the film-grammar feature space of example-driven camera control.
Structure, never prose.

## Files

- `01-camera.md` — camera tricks and plugins, with our home for each.
- `02-mods-and-pcg.md` — ModDB and GitHub mods, and PCG ideas.
- `03-blade-symphony.md` — the dueling mechanics deconstructed, and what to steal.
- `04-arxiv.md` — papers, IDs, and what each one gives us.

## Ladder

Fifteen `CARD` lines land in `docs/research/LADDER.md` with this dive.
Nothing is spiked yet: no numbers, so no ADOPT/REJECT. The kill rule for each is
in its topic file.
