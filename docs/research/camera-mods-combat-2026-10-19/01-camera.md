# Camera tricks and plugins — 2026-10-19

Sources read live unless noted. Structure is stolen; no code is.

## A. Tricks that are just math (cheap, land in `camera_rig.rs`)

### A1. Directional shake, not one noise ball
- Source: "What Features Influence Impact Feel?" (arXiv 2208.06155, §B1.3): a
  knockdown shakes vertical, a transverse slash shakes horizontal, in Guilty Gear
  Xrd: Revelator. The paper names **camera control** as one of the three features
  that decide impact feel (with hit stop and sound coherence).
- We have: `smooth_trauma_offset` — one value-noise vector, three channels, plus
  roll. Omnidirectional.
- Steal: keep the noise, add an **event axis**. A hit carries a small ordered
  axis (up on a knockdown, along the swing on a cut, back on a parry). The noise
  keeps the texture; the axis gives the meaning.
- Home: `camera_rig.rs` (add `axis: Vec3` beside `trauma_permille`), caller
  `camera_feed.rs`. Row QL8.
- Kill rule: if a capture shows the shake direction is unreadable at 30 fps, drop
  the axis and keep the noise.

### A2. Dead-zone framed follow
- Source: Phantom Camera (ramokz/phantom-camera, Godot 4, MIT, read 2026-10-19),
  `Framed` follow mode: the camera stays still until the target leaves a
  horizontal/vertical dead zone.
- Steal: one rectangle test on the target's screen position. No new solver.
- Home: `camera_rig.rs` beside `DampedSync`. Row GP25.
- Why: a follow camera that holds still reads as calm, and it stops the "camera
  swims on every step" failure the GDC talk names.

### A3. Whiskers and DOF priority, not pull-in alone
- Source: John Nesky, "50 Camera Mistakes" (GDC 2015, read 2026-10-19). Two
  named ideas: (a) **whiskers** — side raycasts that detect a looming obstacle
  and *pre-emptively swing the camera out*; (b) a **7-degree-of-freedom
  constraint list** (yaw, pitch, roll, X/Y/Z offset, FOV) where player intent
  wins on yaw, so when the player swings toward the obstacle the camera pulls in
  instead.
- We have: `camera_arm.rs` pull-in only (Godot SpringArm3D shape).
- Steal: a whisker pass that yaws out early, then the priority rule (player yaw
  beats avoidance, then pull in). This is the difference between "the camera
  jerks when the pillar arrives" and "it glides".
- Home: `camera_arm.rs` (whisker distances) + rig stage order. GP14.
- Kill rule: if `count_unframed` never improves on the pillar fixture, reject.

### A4. FOV as a function of pitch
- Source: same GDC talk, mistakes 14 and 15: use a *shorter* boom and a *wider*
  FOV looking up at the sky, and pull *out* looking down at the floor, so it does
  not feel claustrophobic.
- We have: a constant `fov_y_rad`.
- Steal: a small pitch -> (distance, fov) curve in `DirectorParams`.
- Home: `camera.rs` (`DirectorParams`) + `camera_rig.rs`. GP14.

### A5. Roll is the best cheap juice axis
- Source: "Crafting Gun Feel" thesis (Mourath 2025) and "Math for Game
  Programmers: Juicing Your Cameras with Math" (Eiserloh, GDC 2016): roll affects
  the screen edges far more than the centre, so a little roll reads as a lot.
  Eiserloh also gives the canonical rule: **trauma in [0,1], shake = trauma^2 (or
  ^3), decay linearly, and use Perlin/value noise, never random** — which is
  exactly what we built.
- We have: `roll_rad` in `smooth_trauma_offset`.
- Steal: only tuning. Author roll per event type (a parry rolls less than a
  knockdown). Ties to A1.
- Note: we already follow the accepted recipe (trauma^2, smoothed noise). Good.

### A6. Shake inside a holder so it never turns the aim
- Source: "Kinesthetic in a classic video game" (2018): a holder object carries
  the camera's true orientation; shake moves the camera inside it.
- We have: `unshaken_eye()` and the rig's "shake on the presented eye only".
- Verdict: already done. Listed so nobody re-adds a second shake path.

### A7. Dolly zoom / exposure tricks (later)
- Source: gun-feel thesis: one very bright frame triggers automatic exposure and
  darkens the rest; a blur or a negative frame accentuates an impact frame.
- Steal: an "impact frame" is 1-2 frames of post, not an animation. Cheap,
  frighteningly effective. Home: `crates/render` post (RL rows), not the tick.

## B. Camera plugins worth reading (ideas, not vendoring)

| Plugin | What it is | What we take |
|---|---|---|
| Cinemachine (Unity, docs read 2026-10-19) | Virtual cameras with priority, blend styles (Cut/Ease In Out/Hard In/Hard Out/Linear/Custom curve), custom blenders, Impulse system, ClearShot group, deoccluder | The **virtual-camera + priority + blend-curve** shape; **Impulse channels** (see B1); **ClearShot** (see 9); "ignore transparent objects" list is our `Pillar()` trick already |
| Phantom Camera (ramokz/phantom-camera, Godot 4) | PCam/PCamHost: follow modes Glued, Simple, Group, Path, Framed; priority switch with tween | `Framed` (A2), `Group` (9), `Path` (11/12) |
| GDExtension_CineCam (LumiToad, MIT) | Godot camera blending/shaking, thesis project | Same shape as Cinemachine, smaller. Blending + shake as separate concerns |
| MrMinimal/godot-camera-shake (AGPL-3.0) | **Handshake presets recorded in the real world**, from a Blender addon | The **preset library** idea: named shakes (footstep, handheld, engine) instead of one noise. AGPL — ideas only anyway |
| nikableh/shaky-camera-3d (MIT) | Camera3D with "filming while in motion" shake | Handheld-as-a-default-layer, separate from event shake |
| filipbasara/trauma-gd | Trauma-based shake in Godot | Confirms the trauma model; check its curve against ours |

### B1. Cinemachine Impulse, in full (the strongest single steal here)
- Impulse **Source** broadcasts on **channels**; **Listener** filters by channel
  mask and applies a **gain**. Impulse types: **Uniform** (infinite speed),
  **Dissipating** (weaker with distance), **Propagating** (finite speed, default
  343 m/s) with a dissipation-distance curve. Shapes: Recoil, Bump, Explosion,
  Rumble, or a hand-drawn curve. Listener can add a secondary "on springs" shake.
- Why it fits us: our camera is presentation-only and already has trauma. This is
  a data model for *what caused* the shake: channel, shape, distance falloff. A
  door/ replay consumer could drive it, and `observe` already emits typed events.
- Home: `camera_rig.rs` + `camera_feed.rs`; consume `engine-observe` events so
  sim never sees it. Row QL8.

## C. Capture and photo mode (the bridge's home)

### C1. Photo mode (a checklist we can copy)
- Sources: Unity Technologies `PhotoMode` (URP, Cinemachine, free: view roll,
  view distance, focus distance, aperture, exposure, contrast, saturation,
  vignette, filters, frames, stickers, camera height/rotation, rule-of-thirds
  grid, hide UI); UE `DarknessFX/PhotoModeFX` (21 filters, high-res buffer
  screenshots, 360/stereo panorama, cubemap export, 3D photo, no tick);
  UE `egementon/PhotoModeProject` (character move up/down/N/S/E/W, hide player,
  FOV, roll, filters, grid); BG3 community "screenarchery" tool list (camera
  tweaks, light placement, pose player, object mover, freecam, hotsampling via
  SRWE, DOF shaders).
- Steal, in order of value to us: pause and free the camera; roll; FOV; a
  rule-of-thirds grid; hide UI; a filter ladder; then hotsampling (render above
  the window size) and 360/cubemap. Hotsampling is the one that makes captures
  look like art.
- Home: forge `bridge` (a `capture.photo` verb beside `CaptureCamera.cs`), plus
  the engine's `playtest`/`render` offline path. Never the fixed tick.

### C2. Replay camera with keyframes and tripods
- Source: ReplayMod (Minecraft, GPL-3.0) docs: record sessions, replay from any
  perspective, a **keyframe timeline** (position keyframes of x/y/z/yaw/pitch/
  roll, interpolation, path preview), camera roll keys (J/K/L), **tripod**
  cameras set from the hotbar and switched, event markers (`M`) during play, and
  in-mod rendering to video (including 360 and 3D).
- We have: `--camera-path arena600` and `Q1`/`RL` capture rows.
- Steal: (a) named tripod cameras as a small data file; (b) **event markers** on
  the replay timeline so a capture is aimed at a parry or a knockout instead of a
  frame number; (c) keyframe roll.
- Home: `crates/playtest` (captures) + `crates/endpoint` (a `capture.path` verb).
- Note: Modrinth's camera collection adds `Motion Capture` (record and replay
  player movement) and `Creator Overlays` (rule-of-thirds and vertical-framing
  guides). The overlay idea is one line in the capture path.

### C3. Free camera and tripod in-game
- Source: MinecraftFreecam (MIT): freecam with tripod mode (F4 + hotbar = save a
  camera, switch at will), ignore-collision toggles, initial-perspective modes
  (first person / behind / mirror / inside), full-brightness, disable-on-damage.
- Steal: the tripod + `initial perspective` enums; "disable on damage" is a
  nice anti-cheat-flavoured guard if any player-facing camera tool ships.
- Home: `camera.rs` free mode (row GP42) + GS41.

## D. Camera direction from a sentence (arxiv, see `04-arxiv.md`)

The four that matter: Camera Diffusion Model (text -> trajectory, keyframes,
latent blending), ChatCam (an LLM agent + CineGPT + an anchor determinator that
picks which object a shot is about), LensCraft (volume-aware, real-time, open
code and weights), and example-driven camera behaviours (learn a style from film
clips as a low-dim cinematic feature space with a Mixture-of-Experts gate).

Steal the *interfaces*, not the networks: a shot request names a subject, a
framing (rule of thirds), a motion and an emotion; the bridge already has
`CameraShot.Fit` and `CameraPlacer` to solve it. The papers tell us which knobs
(distance, angle, screen composition, actor configuration) the request should
carry.

## E. Our own gaps this hunt confirms

- `crates/spatial/src/compose.rs` **scores** a shot and never proposes one
  (SPR10). Everything in A3, 9 and D is a *proposer*.
- forge `CameraPlacer` already does scene-aware search with a visibility score;
  it is the closest thing we own to Cinemachine ClearShot. Cinemachine's
  **shot-quality assessment** hook is the generalisation.
- No dolly-zoom, no impact frame, no photo mode, no event markers.
