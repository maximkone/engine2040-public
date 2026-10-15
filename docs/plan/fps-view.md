# FPS view — plan to the VISION.md bar

Plain summary: a first-person eye that sits on a fighter's body and sees
the arena through the same render graph as everything else. The engine
today has a third-person fight director only (the boom sits behind and
above on `+z`); no eye-level pose, no look axis in input, and no wiring
from an eye pose into the render camera exist. This plan adds the eye
pose as presentation-only view code, a look channel beside the hashed
input, the eye-to-matrix feed into the render camera, and the no-clip
body rule its consumer (UW11) already demands.

Owner lanes: `engine-game` (pose, feed), `engine-input` (look channel),
`engine-render` (camera, submit). Prefix: FV (new, proposed by this
plan; no FV rows are on the board). Base commit:
`2e12baf12299a6c6d1f536c3021d84398b48faef`. No PLN row for FPS or
first-person exists on the board (Grep `PLN.*fps|fps-view|FPS` over
`docs/SPRINT.md` and `docs/` is empty except the UW11 anim row); this
file is written from the code as-is. Index note for the keeper: this
file needs its one line in `docs/plan/README.md` plus its VISION row in
the same landing commit (`tools/plan-index.test.mjs` checks it); that
edit is outside this packet's owned path.

## 1. Where it stands

The table is what the code confirms today. Ticked rows are dropped (git
history and the board keep them).

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Third-person director (lock-on, boom, trauma decay, shake) | LANDED | `crates/game/src/camera.rs:1-19`, `DirectorParams` `:138-158`, `FRAME_BYTES` 46 `:24` |
| Staged rig (follow -> lock -> shake, bypassable) | LANDED | `crates/game/src/camera_rig.rs:32-58`, `StagedRig::update` `:177-197` |
| Look intent plus damped sync (raw vs smoothed split) | LANDED | `LookIntent` `:203-219`, `DampedSync::sync` `:276-291` in `crates/game/src/camera_rig.rs` |
| Fight-camera spring (halflife yaw/dist/look, whip cap, exact ramp) | LANDED | `FightCamSpring::step` `:610-702`, duel test `:843-918` in `crates/game/src/camera_rig.rs` |
| Smooth trauma shake (value noise, squared gain, roll) | LANDED | `smooth_trauma_offset` `:471-506`, test `:977-1025` in `crates/game/src/camera_rig.rs` |
| Presentation feed (presented bytes, receipts, manifests) | LANDED | `crates/game/src/camera_feed.rs:1-24`, `PresentationFeed::new` `:120` |
| Framing oracle (unframed count over a frame) | LANDED | `count_unframed` in `crates/game/src/camera.rs:661-668`, used `:873-904` in `camera_rig.rs` tests |
| Deterministic fighter input (wish, facing, buttons, tape) | LANDED | `FighterInput` in `crates/input/src/fighter.rs:68-81`, frozen 10-byte layout `:95-96` |
| Bindings, deadzone, gestures, action map | LANDED | `crates/input/src/bindings.rs:38-311`, `map.rs:163-249`, `gestures.rs:27-80` |
| Render camera (eye/target/up, validated view plus proj matrices) | LANDED | `Camera::view_matrix` `:389-396`, `proj_matrix` `:404-405` in `crates/render/src/v1.rs` |
| Frame graph (compiled order, aliasing, barriers, CPU receipt) | LANDED | `FrameGraph::compile` `:431-696`, `CompiledGraph::execute` `:820-862` in `crates/render/src/graph.rs` |
| Window GPU submit over the compiled graph | LANDED | `CompiledGraph::submit_gpu` `:1191-1197` in `crates/render/src/graph.rs`, budget 16 ms `:892` |
| First-person eye pose from a body | MISSING | no eye-height, head-offset or eye-level pose in `crates/game/src/camera.rs` or `camera_rig.rs`; every pose uses `boom_up`/`boom_back` (`camera.rs:526-531`, `camera_rig.rs:708-716`) |
| Look (yaw/pitch) axis in input | MISSING | `FighterInput` carries absolute `facing` centidegrees only (`fighter.rs:77`); no pitch, no look-delta channel anywhere in `crates/input/src/` |
| Eye-to-camera feed (pose into `v1::Camera`, submit path) | MISSING | the GX1 look uniform carries frame index, viewport, exposure and sun dir only (`graph.rs:1042-1053`); no eye, no view-proj |
| Body no-clip for an eye inside its own head | MISSING | consumer UW11 (`docs/SPRINT-16-urban-war.md:66-71`) demands "the first-person camera never clips the body"; no near-clip, hide-own-head or fade rule exists |
| Eye-space shake and FOV punch | MISSING | `smooth_trauma_offset` returns world-space offset plus roll (`camera_rig.rs:455-463`); nothing maps it into eye space, nothing touches FOV |
| Pillar-aware boom pull-in (third-person precedent) | LANDED, third-person only | `crates/game/src/camera_arm.rs:1-3` clamps the boom eye against pillars, presentation-only; no eye-level equivalent exists |
| Render draw-camera validation (fov, aspect, clip) | LANDED | `crates/render/src/draw.rs:339-368` refuses bad `fov_y_rad` before any matrix; the eye feed must pass the same gate |
| Debug camera, pause and step in the arena binary | OPEN, boarded (GP42) | only `--camera-path` in the game binary today (`docs/plan/gameplay.md` section 1); FV rows do not duplicate GP42, the eye-mode switch is one flag beside it |

Module notes: the director, rig, spring and trauma helpers are
presentation-only and never feed simulation state (`camera_rig.rs:8-14`);
`FighterInput::append_bytes` is the frozen hashed layout (a per-fighter
record stays 10 bytes); the render `Camera` refuses bad input loudly
(`v1.rs:352-386`) and its matrices are bit-identical for identical
input; the graph runs compiled order as compiled (`graph.rs:883`).

## 2. Finish line

- **Tier A (alpha, one eye on the sand):** a scripted tape drives a
  first-person eye through the arena for 600 ticks; the eye sits at head
  height over the focus fighter, turns with its facing plus a
  presentation-only look channel, and feeds the render camera; the
  window build shows the arena from eye level with 0 validation errors;
  the eye never sees the inside of its own head; all bytes replay x2.
- **Tier B (v1, a view template any mode can use):** the pose, look,
  feed, no-clip and shake rows below are landed; a second consumer
  (the UW11 infantry body, or a debug eye-mode in the arena binary)
  reads the same `FpsEye` type with no fork; eye travel stays inside a
  per-frame budget at 30 fps; the golden hash
  `d57b5c8ae3096614` never moves (every new byte is presentation-side).
- **Tier C:** none. Weapon view-models, ADS magnification curves,
  leaning, and eye-tracked foveation stay CUT (see section 6).

CUT (never): a second camera stack beside `v1::Camera`; hashed look
input inside `FighterInput`; float in the tick; a forked trauma curve
for first-person; imported FPS-controller code as a gate.

## 3. Target design

Data flow per presented frame:

```
fighter track (pos_m, facing) + look deltas (presentation channel)
  -> FpsEye { eye_m, yaw_rad, pitch_rad } (FV1, FV2)
  -> v1::Camera { eye, target, up, fov_y } (FV3)
  -> view-proj matrices -> graph submit (GX1 path)
  -> pixels; eye-space shake added last, never hashed (FV5)
```

Ownership: `crates/game` owns the eye pose (`fps_view.rs`, beside the
frozen `camera.rs`, never inside it); `crates/input` owns the look
channel (beside `FighterInput`, never inside its hashed bytes);
`crates/render` owns the camera and the submit (the eye arrives as a
validated `Camera`, the same type every other caller uses). The arena
binary owns the debug eye-mode switch; UW11 owns the body that the eye
sits on. No row crosses those edges: game never edits hashed input
bytes, input never builds matrices, render never reads fighters.

Determinism follows `../HASH-LAW.md`: the sim keeps integer math plus
stable orders; the eye pose, look smoothing and shake may use `sqrt`,
`sin`, `exp` and noise because they are presentation-only and never
enter a checkpoint hash. Same tape plus same look-delta track gives the
same eye bytes twice (FV6 proves it).

Budgets: eye pose at or under 0.02 ms per frame, zero heap bytes per
call (same bar as the director's 0.05 ms and zero-heap rule);
per-frame eye travel inside the framing budget the spring test uses
(peak at or under 25 cm at 30 fps, `camera_rig.rs:872`); submit inside
the posted `GPU_SUBMIT_BUDGET_MS` 16 ms at test sizes. API sketch:

```rust
pub struct FpsEye { pub eye_m: Vec3, pub yaw_rad: f32, pub pitch_rad: f32 }
pub fn eye_for_fighter(pos_m: Vec3, facing_rad: f32, look: &LookState) -> Result<FpsEye, CameraError>;
pub fn camera_for_eye(eye: &FpsEye, fov_y_rad: f32, aspect: f32) -> Result<v1::Camera, CameraError>;
```

### Starting tables (tuned only by fixtures, never by hand in Rust)

One table in `content/design/fps-view.rows` (same `table <name> version
<n>` format GP27 defines, whole numbers, units in the key), parsed by
the same validated loader path:

| Key | Starting value | Why |
|---|---|---|
| `eye_height_mm` | 1600 | standing eye over feet `pos_m`, below a 1.8 m figure's crown |
| `yaw_rate_permille_per_tick` | 40 | full turn in about 150 ticks at full stick; whips stay readable |
| `pitch_clamp_centideg` | 8000 | about +/-80 deg: look up and down, never behind the neck |
| `own_head_hide_mm` | 250 | inside this distance the own head hides (FV4 rule) |
| `near_clip_mm` | 50 | matches the render `near = 0.1` order without eating the nose |
| `fov_punch_permille` | 60 | trauma FOV kick cap, about 6% of base FOV (FV5) |

The parsed digest joins the rules version the same way HR3 tables do;
a bad row refuses with its line number. The golden proof scene never
reads this table, so v1 bytes cannot drift through it.

## 4. Hard problems

- **An eye inside its own head.** Problem: at eye level the near clip
  cuts the skull and the camera sees backfaces. Technique: head-height
  eye plus a no-clip rule (own-head hidden or faded inside a table
  distance, near clip from the same table), owned by FV4 with the UW11
  body as the live consumer. Why: one rule, one table, proven by a
  fixture that orbits the eye through the body. Reference: UW11 accept
  line (`docs/SPRINT-16-urban-war.md:68-71`).
- **Look input without moving the golden.** Problem: a pitch axis does
  not fit the frozen 10-byte `FighterInput` record. Technique: the look
  channel rides beside the tape as presentation-only deltas (like
  `DampedSync`, which keeps smoothing apart from raw intent,
  `camera_rig.rs:221-231`); the sim keeps absolute `facing`. Why:
  HASH-LAW keeps `d57b5c8ae3096614`; the eye still turns. Reference:
  `../HASH-LAW.md`, `crates/input/src/fighter.rs:95-96`.
- **Shake that reads at eye level.** Problem: world-space shake moves
  the world sideways instead of kicking the view. Technique: map the
  existing `smooth_trauma_offset` (squared gain, bounded,
  `camera_rig.rs:471-506`) into eye space (yaw/pitch/roll about the
  eye) and keep the cap in the same table as the no-clip distance. Why:
  one trauma curve for both views, no fork. Reference: GP21
  trauma-curve bar (jab at or under 1/4 super).
- **Two views, one submit path.** Problem: the GX1 uniform has no eye
  and the graph knows no camera. Technique: the eye arrives as a plain
  validated `v1::Camera`; the submit path is untouched (FV3 only builds
  the camera, never a second submit). Why: the slim law, one owner per
  matrix. Reference: `crates/render/src/graph.rs:1186-1197`.

- **Look rates that do not whip the read.** Problem: a fast look
  flick plus a trauma kick on the same frame hides the tell the kick is
  meant to sell. Technique: the look table caps per-tick yaw/pitch
  rates, and FV5 applies shake after the look pose is final, so the
  kick is always the last and smallest motion. Why: readability is the
  gameplay plan's second pillar (section 0 of `gameplay.md`).
  Reference: the spring's 35 deg/s whip cap precedent
  (`camera_rig.rs:513-513`, `MAX_PAN_RAD_S`).

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4.

## 6. Slim

- Delete: no new trauma curve, no new smoothing core, no new submit
  path; FV rows call `smooth_trauma_offset`, `DampedSync`-style gain
  and `CompiledGraph::submit_gpu` as they stand.
- Merge: FV3 lands the eye-to-camera feed for both consumers (the
  scripted tape fixture and UW11) in one row; no per-consumer feed.
- Freeze: `camera.rs` (director), `FighterInput` bytes, `v1::Camera`
  shape and the graph compile order; new code sits beside them, never
  forks them.
- What we will not build here: weapon view-models, ADS zoom, lean,
  head-bob as a sim feature (a presentation bob table may ride FV5
  only if UW11 asks), third-person shoulder cams (GP14 owns that
  idea), networked eye sync (multiplayer owns it), eye tracking.

## 7. Board tasks

Rows live here until the orchestrator boards them; slice any row with
`node tools/board-slice.mjs <ID>` once boarded. Prefix FV is proposed
by this plan; no FV rows are on the board, so there are no Row changes.
`needs:` names same-plan rows or the boarded UW11 only.

- [ ] FV1 fps-view: eye pose from the fighter body. Tier A. Size S.
  Owns `crates/game/src/fps_view.rs` (new file beside `camera.rs`).
  Accept: `eye_for_fighter` puts the eye at table head height over
  `pos_m` with yaw from `facing`; bad height, facing or table refuses
  with `CameraError::BadParam` naming the row; same track replays
  byte-exact twice; zero heap bytes per call; `cargo test -p
  engine-game --lib fps_view` green. needs: none (FV6 proves it last;
  the old "needs: FV6" made a cycle with FV2, FV3, FV6).
- [ ] FV2 input: presentation-only look channel beside the tape. Tier
  A. Size S. Owns `crates/input/src/look.rs` (new file beside
  `fighter.rs`).
  Accept: yaw/pitch deltas accumulate into a validated `LookState`
  with table clamp rates; `FighterInput` bytes unchanged (10-byte
  record test stays green); same delta track replays exactly twice;
  golden hash unchanged; `cargo test -p engine-input --lib` green.
  needs: FV1.
- [ ] FV3 render: eye-to-camera feed into the submit path. Tier A.
  Size M. Owns `crates/game/src/fps_view.rs` feed hunk plus a
  `v1.rs` test-only constructor hunk (no `Camera` shape change).
  Accept: `camera_for_eye` builds a validated `v1::Camera` whose
  view-proj is bit-identical for identical eyes; the window submit
  runs the eye camera with 0 validation errors at test sizes inside
  `GPU_SUBMIT_BUDGET_MS`; `cargo test -p engine-render --lib camera`
  green. needs: FV1.
- [ ] FV4 view: no-clip body rule at eye level. Tier A. Size M. Owns
  `crates/game/src/fps_view.rs` no-clip hunk (table: hide distance,
  near clip).
  Accept: scripted eye orbit through the focus body never shows
  inside-of-head pixels (near-clip plus own-head rule from one
  table); UW11 accept line ("never clips the body") holds on the
  infantry body; replays twice. needs: FV1, UW11.
- [ ] FV5 feel: eye-space trauma kick plus FOV punch. Tier B. Size S.
  Owns `crates/game/src/fps_view.rs` shake hunk (calls
  `smooth_trauma_offset`, no new curve).
  Accept: jab kick at or under 1/4 super kick about the eye (GP21
  bar in eye space); roll bounded by 0.10 rad times gain; FOV punch
  from the same gain inside a table cap; calm reports zero; replays
  twice. needs: FV1.
- [ ] FV6 proof: scripted 600-tick eye tape with receipt. Tier B.
  Size S. Owns `crates/game/tests/it/fps_view_tape.rs` (new file,
  wired as a `mod` line in its `main.rs`).
  Accept: one tape drives fighter plus look deltas for 600 ticks;
  eye bytes replay exactly twice; per-frame eye travel peaks at or
  under 25 cm at 30 fps; the receipt prints eye travel, shake peak
  and submit ms; golden hash unchanged. needs: FV2, FV3.

## 8. Quality gates

- Every row: same inputs replay byte-exact twice; a shuffled fighter
  order that changes an eye pose fails the row (stable id order, the
  director's tiebreak rule).
- Golden: `d57b5c8ae3096614` unchanged under every FV row; a look or
  eye byte that leaks into the tick hash fails the row.
- Input honesty: `FighterInput` stays a 10-byte record; the look
  channel never enters `append_bytes`; an unknown look rate refuses
  loudly with its name.
- Camera honesty: every eye builds a validated `v1::Camera`; a NaN,
  degenerate view or bad FOV refuses before any matrix; 0 wgpu
  validation errors on the eye submit.
- No-clip: the FV4 orbit fixture plus the UW11 body hold with no
  inside-of-head frame; a table change without the fixture rerun fails
  the row.
- Shake parity: eye-space kick keeps the GP21 ratio (jab at or under
  1/4 super); calm is exactly zero; caps from the same table as FV4.
- Budgets: eye pose at or under 0.02 ms per frame with zero heap per
  call (counting-allocator gate where wired); eye travel peak at or
  under 25 cm at 30 fps; submit inside 16 ms at test sizes.
- Slim: no second camera type, no second trauma curve, no second
  submit path; a mirrored type fails review.
- Table gate: `content/design/fps-view.rows` parses with zero
  refusals; its digest joins the rules version; a float, a missing
  unit or an unknown key refuses with its line number (GP27 bar).
- Cross-consumer gate: the scripted tape fixture and UW11 read the
  same `FpsEye` type; a per-consumer eye struct fails review.

## 9. Session estimate

Tier A FV1-FV4 about 6 sessions (each S or M, FV3 the M); Tier B FV5,
FV6 about 2 sessions (each S). Total about 8 sessions. No Tier C.
Assumes UW11 lands the body the eye sits on; if UW11 slips, FV4 proves
against the scripted capsule fixture instead and stays S.

Per-row shape: FV1 S (one new file plus its tests), FV2 S (one new
input file, frozen-bytes test stays green), FV3 M (feed hunk plus
submit-path proof on the reference GPU), FV4 M (orbit fixture plus the
UW11 body bind), FV5 S (eye-space map over the landed curve), FV6 S
(one tape test plus its receipt line). Split anything that grows past
M; no row lands without its Accept proof command green.
