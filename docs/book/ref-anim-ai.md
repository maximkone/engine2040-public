# Reference: anim, ai, nav, nn

Details only. Signatures are copied from source (file named in each section).
Units: meters, seconds, radians, unit quaternions `[x, y, z, w]`, fixed 60 Hz ticks, integer
permille where a row says so.

Read first: `docs/HASH-LAW.md`. Root motion and gameplay AI are in the authoritative
simulation (tier 3): integer or `Fixed` math, `+ - * /` and `sqrt` only. Pose blending,
inertialization, the feet view and everything in `render` are presentation: floats there
never enter a hash (`inertial.rs`, `warp_ground.rs`, `pose.rs` say so in their headers).

## 0. Rules across this group

| Rule | Where it applies |
|---|---|
| Root (`RootDelta`, `RootTable`, `ContactTable`) is authoritative. Pose (`LocalPose`) is presentation. | `locomotion.rs`, `contract.rs`, `pose.rs` |
| Brains emit validated `FighterInput` once per decision period; the arena applies the wish. | `ai::Brain`, `ai::DepthBrain`, `arena` |
| Network outputs are proposals. Authority validates them. Replay never calls a model again. | `nn` docs, `HASH-LAW.md` "AI and asynchronous services" |
| Invalid input is refused with a typed error and changes nothing. | every module below |

Tests (heavy cargo runs on GitHub): `node tools/gh-check.mjs cargo test -p <crate> --lib`.

## 1. `engine-anim` (`crates/anim`)

Skeleton, clips, pose sampling, blending, IK, locomotion, state machines. Std-only. Feature
`json` (`serde_json`) adds the state-machine JSON codec and door layer. The crate depends on
`engine-nn` for two motion-matching paths (`motion_match.rs`, `motion_net.rs`).

### 1.1 Contract types (`contract.rs`)

| Item | Definition and range |
|---|---|
| `MAX_JOINTS` 256; `NAME_MAX_LEN` 64 | Joint name charset `[a-z0-9_]`, no leading digit. |
| `Joint` (name, parent index), `Skeleton` (joints in parent-first order) | `Skeleton::len()`, `is_empty()`. |
| `LocalPose` | Per-joint local transform lanes (presentation). |
| `RootDelta` | Planar plus yaw intent for one tick (authoritative). |
| `RootTable` | Per-tick root deltas for a clip. |
| `Contact`, `ContactTable` | Foot plant and lift samples per tick (authoritative). |
| `validate_skeleton(&Skeleton)`, `validate_pose(&Skeleton, &LocalPose)`, `validate_root(&RootTable)`, `validate_contacts(&ContactTable)` | Each returns `Result<(), AnimError>`. |
| `skinning_matrices(...)` | Render-side matrices from a pose. |
| `BoneRole` (17 values), `REQUIRED_BONE_ROLES`, `REQUIRED_BONE_COUNT` 17, `bone_role_for_name(&str) -> Result<BoneRole, AnimError>`, `bone_coverage(&[BoneMapping]) -> Result<u16, AnimError>`, `FULL_COVERAGE_PERMILLE` 1000 | Asset import maps joints to roles; coverage is permille. |

`AnimError`: typed refusal for skeleton, pose, root and contact shape problems. Assets import into these
types; anim never imports `assets` or `arena`.

### 1.2 Clips and sampling (`clip.rs`, `pose.rs`)

| Function | Signature and behaviour |
|---|---|
| `TICK_DT` | `engine_core::DT` (one fixed step, seconds). |
| `bake_track(times: Vec<f32>, values: Vec<[f32;3]>) -> QTrack`; `bake_clip(Vec<QTrack>) -> QClip` | Quantized baked clip. |
| `sample_track(&QTrack, t: f32, hint: &mut usize) -> [f32;3]`; `sample_all(&QClip, t, &mut Cursor, out: &mut [[f32;3]])` | Clamp at ends, lerp between keys. `Cursor::new(n)`, `reset()`. |
| `qclip_hash(&QClip) -> u64`; `traj_hash(&QClip, joint, ticks) -> u64`; `slip(a, b, dt) -> f32`; `avg_slip(&QClip, foot, ticks) -> f32` | FNV checks and foot slip. |
| `sample_pose(&Skeleton, &Clip, time: f32) -> Result<LocalPose, AnimError>` | Emits identity rotations: `Clip` has no rotation lanes. |
| `sample_pose_with_rots(...)`, `sample_pose_with_rots_into(...)` with `QuatTrack` slices | A joint keeps its quaternion when a valid track exists, else identity. |
| `blend_poses(&LocalPose, &LocalPose, t: f32) -> LocalPose` | Presentation blend. |
| `extract_root(...) -> RootTable` | Root motion from a clip. Test: `root_deltas_sum_to_full_loop_travel` (pose.rs). |

Edit verbs over clips (`edit.rs`): trim, retime, cleanup over `QClip`/`QTrack`.
Pack (`pack.rs`): quantized ACL-shape pack with exact decode.

### 1.3 Locomotion (`locomotion.rs`)

Authoritative gait machine. Arena consumes `LocoOutput::root` and `contacts`; render consumes `pose`.

| Item | Signature and behaviour |
|---|---|
| `Locomotion::new() -> Self` (Clone, PartialEq: clone snapshots everything) | |
| `update(&mut self, wish_speed: f32, wish_dir: [f32;2], tick: u32, interrupt: Option<CombatInterrupt>) -> Result<LocoOutput, LocoError>` | One fixed tick. Returns root, pose and contacts. |
| `update_root(...same...) -> Result<RootDelta, LocoError>` | Root only, for the authoritative path. |
| `set_airborne(bool)`; `phase() -> f32` (`[0,1)`); `facing() -> [f32;2]`; `speed()`; `is_airborne()`; `interrupt_active()`; `interrupt_weight()` | |
| `LocoOutput { root: RootDelta, pose: LocalPose, contacts: ContactTable }` | Pose is presentation. |
| `loco_skeleton() -> Skeleton`; `contacts_for_phase(phase, tick)`; `contacts_planted(tick)`; `contacts_airborne(tick)` | Seven joints; legs at `LOCO_LEG_JOINTS` = `[3, 4]`. |
| `drive_readable(...)` | Overlay for the facing arrow (`readable.rs`) without touching authority. |

Gait windows: left foot planted on phase `[0.0, 0.6)`; right foot on `[0.5, 1.0)` and `[0.0, 0.1)`;
double support `[0.5, 0.6)`. Constants: `IDLE_EPS` 0.05, `WALK_MAX` 2.0, `RUN_MAX` 6.0, `STRAFE_MAX` 2.0,
`MAX_WISH_SPEED` 12.0, `START_TICKS` 10, `STOP_TICKS` 12, `PIVOT_TICKS` 10, `LAND_TICKS` 10,
`WALK_CYCLE` 0.6, `RUN_CYCLE` 0.38, `HIT_LIGHT_TICKS` 14, `HIT_HEAVY_TICKS` 24.

Test: `transition_map_idle_walk_run_stop_idle`, `drive_readable_wires_overlay_without_touching_authority` (locomotion.rs).

### 1.4 Inverse kinematics (`ik.rs`, `fbik.rs`, `feet.rs`)

| Function | Signature and behaviour |
|---|---|
| `two_bone(root, mid, end, target) -> ([f32;3], [f32;3])` | Never fails: falls back to `(mid, end)` when `two_bone_pole` refuses. |
| `two_bone_pole(root, mid, end, target, pole) -> Result<([f32;3], [f32;3]), IkError>` | Pole vector names the bend side. |
| `turn_between(from, to) -> [f32;4]` | Shortest-arc quaternion. |
| `Limb::new(&Skeleton, root, mid, end) -> Result<Self, IkError>`; `solve_limb(&Skeleton, &mut LocalPose, Limb, target, pole) -> Result<(), IkError>` | A refused call writes nothing. |
| `fabrik(joints: &mut [[f32;3]], target, max_iter: u32, tol: f32) -> u32` | Iterations used. |
| `foot_lock(current, locked, lock_radius) -> [f32;3]` | Holds the planted point inside the radius. |
| `MIN_BONE_M` = 1.0e-6 | Shorter bones are refused. |

`IkError`: `DegenerateBone(u8)` (0 root-to-mid, 1 mid-to-end), `NonFinite(&'static str)`, `ZeroPole`, `BadJoint(usize)`,
`NotAChain`, `PoseLength { skeleton, pose }`. Solves allocate nothing.

Example, adapted from `two_bone_reaches` (`crates/anim/src/ik.rs`):

```rust
let (mid, end) = two_bone([0.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, -2.0, 0.0], [1.0, -1.0, 0.0]);
// end lands within 0.05 m of the target; mid stays 1.0 m from the root.
```

### 1.5 Inertialization and foot locks (`inertial.rs`, `inertialize.rs`)

| Item | Behaviour |
|---|---|
| `Inertial::new(joints: usize, halflife_s: f32) -> Result<Self, InertialError>` | Halflife in `MIN_HALFLIFE_S` 0.001..=`MAX_HALFLIFE_S` 1.0. `MAX_JOINTS` 256. |
| `switch(&mut self, shown, shown_vel, arriving, arriving_vel) -> Result<(), InertialError>` | Offsets = shown minus arriving; a refused call changes nothing. Coordinates bounded by `MAX_COORD_M` 1000, speeds by `MAX_SPEED_M_S` 1000. |
| `step(&mut self, arriving, dt_s: f32, out) -> Result<(), InertialError>` | `dt_s` in `0.0..=MAX_DT_S` (1.0). A zero step writes the shown pose. |
| `offsets_after(&self, age_s, out) -> Result<(), InertialError>`; `max_offset() -> f32` | Critically damped decay. |
| `contact(height_y, speed, h_max, v_max) -> bool`; `slide_dist(a, b) -> f32` | Planted when low and slow. |
| `FootLock::new(break_r)`; `update(cur, contact) -> [f32;3]`; `is_locked()`; `slide()`; `reset()` | Pins while in contact; breaks past `break_r`. |
| `AngularLimiter::new(joints, limit_deg_per_sec)`; `step(wanted: &[[f32;4]], dt, out: &mut [[f32;4]])`; `limit_in_place`; `DEFAULT_ANG_LIMIT_DEG_PER_SEC` 1080 | Caps joint turn rate. |

Example, adapted from `switch_frame_shows_the_old_pose` (`crates/anim/src/inertial.rs`, halflife `HALF` = 0.03):

```rust
let mut iz = Inertial::new(2, 0.03)?;
iz.switch(&shown, &shown_vel, &arriving, &arriving_vel)?;
let mut out = [[0.0; 3]; 2];
iz.step(&arriving, 0.0, &mut out)?;   // zero step: the old pose is still on screen
```

Tests: `offset_decays_to_under_a_millimeter_in_five_halflives`, `sixty_and_thirty_hertz_stepping_agree`,
`hostile_input_is_refused_and_changes_nothing`, `replay_is_bit_exact` (inertial.rs).

### 1.6 Blending, crossfade, graphs (`blend.rs`, `crossfade.rs`, `graph.rs`, `layers.rs`)

| Item | Behaviour |
|---|---|
| `fade_weight(fade_left: u32, total: u32) -> f32` | New-clip weight. A zero total cuts at once (1.0). |
| `SlotFade::{retarget(prev_clip: u8, prev_phase_ticks: u32), tick(), blending(), weight(), digest()}` | One per fighter slot. Integer ticks. |
| `blend_pair(&LocalPose, &LocalPose, fade_left: u32) -> LocalPose` | Presentation blend of leaving and arriving pose. |
| `blend::Machine::new()`; `add_state(clip: u32, looped: bool, speed: f32) -> u32`; `add_trans(from, to, after, dur)`; `reset()`; `tick(&[QClip], &mut [Cursor], out: &mut [[f32;3]])` | Fixed-tick state machine over baked clips. `traj_hash_mc`, `machine_slip` for checks. |
| `graph.rs`: animation graph as data; nodes compile to a flat eval order | `layers.rs`: masked body layers over the graph pose. |
| `events.rs`: frame-exact combat and audio callbacks over the graph | Presentation side. |

### 1.7 State machine (`state_machine/`)

| Item | Behaviour |
|---|---|
| `playback.rs`: `fighter_crossfade_ticks() -> u32`; `Playback::new(Arc<CompiledMachine>)`; `state()`; `state_name()`; rate handling | Integer tick, fade and travel. Test: `rate_double_advances_two_per_tick`. |
| `edit.rs`: `new(StateMachineDef) -> Result<_, StateMachineError>`; `apply(Edit)`; `replace(def)`; `undo()`; `redo()`; `undo_depth()` | Validated, undoable edits. Removing the start state with no transitions is refused. |
| `door.rs`: `graph_set(&mut Document, text) -> Result<SetReceipt, StateMachineError>`; `preview(...)`; `parse_script(text) -> Result<PreviewScript, PreviewError>` | Door entry points (endpoint `anim.graph.set`, `anim.preview`). |
| `json.rs` (feature `json`): `parse(&str) -> Res<StateMachineDef>`; `from_value`; `to_value`; `to_json(&StateMachineDef) -> String` | Codec. |

### 1.8 Ground warp, hit reaction, motion matching

| Item | Value or signature |
|---|---|
| `warp_ground.rs`: `MIN_WARP_SPEED_M_S` 0.5; `MAX_WARP_SPEED_M_S` 7.0; `SLIDE_BUDGET_MM` 10; `RATE_MIN` 0.25; `RATE_MAX` 4.0; `GroundWarp::warp_output` post-processes a `LocoOutput` | GASP-class feel over the root; root passes through bit-identical. Errors `WarpError`; a rejected call mutates nothing. |
| `hit_react.rs`: procedural additive flinch from hit direction and impulse | Presentation. |
| `motion_match.rs` (v2 database query), `motion_net.rs` (learned pose path over `engine_nn::Model`), `net_pose.rs` (pick index plus phase replication), `traversal.rs` (authoritative traversal chooser and edge warp) | `MatchError::Net(engine_nn::NnErr)` for net faults. |

Test names in these modules: `phase_wrap_past_clip_end`, `invalid_skeleton_and_track_rejection` (pose.rs);
`solving_a_limb_allocates_nothing_and_repeats_bit_for_bit` (ik.rs).

Tests: `node tools/gh-check.mjs cargo test -p engine-anim --lib` (add `--features json` for the codec).

## 2. `engine-ai` (`crates/ai`)

Brains and decision tools. Gameplay brains emit `FighterInput` (validated, the arena's input type).
The arena fills slots that have no tape input with `Brain` or `DepthBrain`.

### 2.1 Duel brain (`gladiator.rs`)

| Item | Signature and behaviour |
|---|---|
| `Percept { self_pos: [f32;3], self_yaw: f32, foe_pos: [f32;3], foe_attacking: bool, dist_m: f32, tick: u32, melee: MeleeView }` | Meters and radians. Features are quantized before any net (2.3). |
| `Intent`: `Attack`, `Block`, `Dodge`, `Advance`, `Retreat`, `Reposition` | `Reposition` is the anti-stuck wander when the foe is far (`FAR_STREAK_TICKS` 180, `FAR_MM` 8000). |
| `pick_intent(scores: [i32; 5]) -> Intent` | Argmax over the five utility lanes in order (attack, block, dodge, advance, retreat). Strict `>` keeps the earlier index on ties. |
| `Brain::new(fighter: FighterId, period_ticks: u32)`; `with_config(fighter, period_ticks, AiConfig)`; `with_rows_text(...)` | Decides every `period_ticks`; holds intent between decisions. The arena defines `AI_PERIOD_TICKS` = 6. |
| `Brain::MORALE_NEUTRAL` = 500; `set_morale(i32) -> i32` | |
| `decide(&mut self, &Percept) -> FighterInput`; `decide_with_bias(&Percept, [i32;5])`; `decide_drained(_into)` | Always passes `FighterInput::validate`. Depth bias composes with the core. |
| `features_from(&Percept) -> [i8; 6]` | Lanes: `LANE_DX` 0, `LANE_DZ` 1, `LANE_DY` 2, `LANE_DIST` 3, `LANE_FOE_ATTACK` 4, `LANE_YAW` 5. Scales `FEATURE_FULL_GROUND_M` 16, `FEATURE_FULL_HEIGHT_M` 8, `FEATURE_FULL_DIST_M` 16; `FEATURE_ATTACK_ON` 127. |
| `FEATURE_DIM` = 6; `INTENT_OUT_DIM` = 5 | Net shape. |

Example, adapted from `scripted_60s_identical_twice` (`crates/ai/src/gladiator.rs`, which runs 3600 ticks):

```rust
let mut brain = Brain::new(FighterId::new(0).unwrap(), AI_PERIOD_TICKS);
let percept = script_percept(tick);            // any Percept for this tick
let input = brain.decide(&percept);
input.validate().expect("every brain output validates");
```

### 2.2 Policies and nets (`gladiator.rs`)

| Item | Behaviour |
|---|---|
| `TickPolicy` trait (`decide`, `intent`); `Policy` enum; `Policy::load_or_scripted(fighter, Option<&[u8]>) -> Self` | Scripted fallback when no net is given. |
| `NetPolicy::load(fighter, bytes: &[u8]) -> Result<Self, NetPolicyError>`; `intent_for(&Percept)`; `decide` | Loads an `.e2nn` model whose input dim must be `FEATURE_DIM`. `NetPolicyError::{Nn(NnErr), BadInputDim{..}, ..}`. |
| `MarchPolicy` | Second `Policy` variant with its own `decide(&Percept)`; see `gladiator.rs` for its rule. |
| `compare(&mut Policy, &mut Policy, &[Percept]) -> Comparison`; `compare_curves(&AiConfig, &AiConfig, &[Percept])` | Same script, two policies. |
| `self_play_bout`, `self_play_batch`, `SelfPlayConfig`, `SelfPlayReport` | `MAX_SELF_PLAY_BOUTS` 256, `MAX_SELF_PLAY_TICKS` 3600; `SelfPlayError`. |

Consumers: `NetPolicy` is used by `crates/distill` (offline trainer) only. No tick crate uses it.

### 2.3 Depth brain and arena policy (`depth.rs`)

| Item | Behaviour |
|---|---|
| `DepthBrain::new(fighter, period_ticks)`; `with_config(...)`; `set_personality(Personality)`; `set_phase(Phase)`; `set_role(Role)`; `set_tier(TierRow)` | Arena-facing brain with stamina, morale and feint state. Snapshotted by `MatchSnapshot`. |
| `ArenaPolicy`: `set_phase`, `set_role`, `set_stamina(i32) -> i32`, `update_feint(foe_attacking, foe_was_attacking, dist_mm)`, `end_round(Phase)`, `bias() -> [i32;5]` | Integer bias added to the utility lanes. `stamina_cost(Intent)` and `stamina_gate_bias(stamina)` give the stamina gate. |
| `Phase::from_u8(u8) -> Result<Self, DepthError>`; `Phase::for_tick(tick, round_ticks)`; `Role::from_u8` | Round thirds drive the bias order (test `phase_from_round_thirds_with_goldens`). Unknown values refuse loudly. |
| `TierRow`; `SHIPPED_TIERS: [TierRow; 3]`; `PRIMUS_TIER`; `primus_phase(hp_permille) -> u8`; `primus_counter(phase) -> u8`; `parse_tiers(&str) -> Result<[TierRow; 4], TierError>` | Duel enemy tier rows, shipped as constants and parsed from row text (`tier.rs` header: tiers T1 to T4). |
| `FeintState::update(foe_attacking, foe_was_attacking, dist_mm)` | Feint on close release, then expires. |
| Constants | `STAMINA_MIN` 0, `STAMINA_MAX` 1000, `STAMINA_COLLAPSE_AT` 100, `MORALE_CAESAR` 750, `MORALE_HANNIBAL` 650, `MORALE_FABIUS` 500, `TIER_RANGE_MM` 1200. |

### 2.4 Other AI modules

| Module | Contents (from the module header) |
|---|---|
| `utility.rs` | `pick`, `pick_gated`, `response_curve`, `Action`, `Gate`, `Gated`, `Scorer`. Pure decision, no NN, no rand. |
| `behavior.rs`, `bt.rs`, `blackboard.rs` | Stateless behaviour tree (`tick`, `Behavior`, `Status`); serializable integer trees (`bt`); `goap_plan`, `Blackboard`, `TaskStatus`. |
| `bout.rs` | Deterministic bout harness over integer trees (records tapes and reports). |
| `director.rs` | Pacing as data: `pick_spawn`, `Budget`, `SpawnRow`; tension scalar. |
| `curves.rs`, `curves_text.rs` | `AiConfig { curves: AiCurves, morale_default: i32, .. }`, `AiConfig::scripted_defaults()`, `validate_config`, `validate_curves -> AiCurveError`; `DEFAULT_CURVES_VERSION`, `DEFAULT_AGGRESSION`, `DEFAULT_CAUTION`, `DEFAULT_MORALE`, `DEFAULT_ENGAGE_RANGE_CM`, `ENGAGE_RANGE_MIN_CM`, `ENGAGE_RANGE_MAX_CM`, `BIAS_NEUTRAL`, `BIAS_MAX`, `CURVE_LANES`. Designer rows parsed offline. |
| `personality.rs` | `Personality::new(reaction_ticks: u8, preferred_range_mm: u16, aggression_permille: u16)`: seeded, integer temperament. |
| `reaction_delay.rs` | Integer observation delay for brains. |
| `eqs.rs`, `hearing.rs`, `alert.rs`, `memory.rs`, `mind.rs`, `goalstack.rs` | Cover, flank and ambush queries; hearing table; five-state alert ladder; bounded NPC memory; NPC mind contract (memory, goals, relations, async asks); bounded goal memory. |
| `melee.rs` | `MeleeView`, `FoeView`, `StrikePhase`, `NEAR_FOES`, `WAIT_RING_MM`: quantized close-quarters perception. |
| `guard.rs` | Validators in three rails (input, topical, execution). Runtime sees `Verdict` pass, fix or refuse. |
| `legion.rs` | Squad-scale brain (formations) for hundreds of soldiers. |
| `apply.rs`, `provenance.rs`, `dialogue.rs` | Tick-boundary effects for mind actions; provenance records; scripted talk compiled offline to tables. |

Tests: `node tools/gh-check.mjs cargo test -p engine-ai --lib` and `--test general-presets`, `--test crowd_orca_bench`.
Named: `percept_features_quantize_with_goldens`, `perception_clock_holds_intent_between_ticks` (gladiator.rs),
`unknown_phase_and_role_refuse_loudly`, `feint_triggers_on_close_release_then_expires` (depth.rs).

## 3. `engine-nav` (`crates/nav`)

Tiled dynamic grid, flow fields, traversal links, navigation mesh bake. Integer BFS; no hash-map order.

### 3.1 Constants

| Constant | Value |
|---|---|
| `tiles::MAX_TILES` / `MAX_BREAK_TILES` / `MAX_GRID_CELLS` / `MAX_FLOW_FIELDS` | 65 536 / 1024 / 1 048 576 / 8 |
| `tiles::MAX_OBSTACLE_REQUESTS` / `MAX_TOUCHED_TILES` / `TICKS_PER_SECOND` | 64 / 8 / 60 |
| `mesh::MESH_VERSION` / `MAX_TRIS` / `MAX_SIDE` / `MAX_CELLS` / `MAX_BYTES` | 1 / 65 536 / 1024 / 1 048 576 / 16 MiB |
| `mesh::MAX_COORD_M` / `MAX_RADIUS_M` / `MESH_TILE_EDGE` | 4096.0 / 1024.0 / 8 |
| `links::MAX_LINKS` / `EDGE_RANGE_M` | 4096 / 1000.0 |

### 3.2 Tiled grid (`tiles.rs`)

| Item | Signature and behaviour |
|---|---|
| `TiledNav::bake(blocked: &[bool], w, h, edge) -> Result<Self, TileError>`; `bake_seeded(seed, w, h, edge, density_permille)` | Tile-versioned grid. |
| `set_blocked(x, y, blocked) -> Result<TileId, TileError>` | Marks a cell and its tile dirty. |
| `apply_breaks(&BreakSet) -> Result<Vec<TileId>, TileError>`; `rebake_budget(max_tiles) -> usize`; `rebake_one()`; `update_budget(&mut ObstacleCache, max_tiles)` | Dirty tiles rebake in ascending order. |
| `bake_goal(goal: (usize, usize)) -> Result<(), TileError>`; `direction(goal, pos) -> Result<Option<(usize, usize)>, TileError>`; `steer_legion(goal, &[(usize, usize)], out) -> Result<usize, TileError>` | Flow-field steering; O(agents) reads. |
| `find_path(start, goal) -> Result<Option<StampedPath>, TileError>`; `path_open(goal, pos) -> Result<bool, TileError>`; `needs_repath(tile, walking_version)`; `path_is_stale`; `stale_paths(&[StampedPath]) -> Vec<usize>`; `refresh_path` | A `StampedPath` records tile versions it was planned on. |
| `BreakSet::new(tick, &[TileId], tile_count) -> Result<Self, TileError>`; `BreakSet::from_chunks(...)` | Sorted break set from destruction. |
| `ObstacleCache::add_box_obstacle(...)`, `remove_obstacle(ObstacleRef)`; `TileRegistry::{new, version, is_dirty, dirty_tiles, invalidate, rebake}` | |
| `TileRegistry::{to_bytes, from_bytes}`; `MeshTiles::{to_bytes, from_bytes}` | Wire forms refuse bad input and never half-apply. |

Test names (tests/tiles.rs): `broken_wall_opens_a_path_within_one_second_of_ticks`,
`shuffled_break_order_gives_identical_bytes_twice`, `legion_of_500_steps_on_one_shared_field`,
`obstacle_rebuild_touches_only_its_tiles`, `obstacle_requests_refuse_loudly`.

Example, adapted from `broken_wall_opens_a_path_within_one_second_of_ticks`:

```rust
let mut nav = TiledNav::bake(&walled(24, 16, 12, None), 24, 16, 4).expect("bakes");
nav.bake_goal((23, 8)).expect("goal bakes");
assert_eq!(nav.path_open((23, 8), (0, 8)), Ok(false));
nav.set_blocked(12, 8, false).expect("breach flips");
nav.rebake_budget(usize::MAX);
nav.bake_goal((23, 8)).expect("field refreshes");
assert_eq!(nav.path_open((23, 8), (0, 8)), Ok(true));
```

### 3.3 Flow fields (`flow.rs`) and bake cache (`bake.rs`)

| Item | Behaviour |
|---|---|
| `seeded_blocked(seed, w, h, density_permille) -> Vec<bool>` | Seeded obstacle layout. |
| `bake_flow(blocked, w, h, goal: (usize, usize)) -> Vec<i32>`; `follow(flow, w, h, pos) -> Option<(usize, usize)>`; `flow_hash(&[i32]) -> u64` | Integer distance field and next-step lookup. |
| `bake::BakedNav::bake(blocked, w, h, &TileEdge) -> Self`; `bake_goal(&Goal) -> &[i32]`; `direction`; `bake_hash()`; `validate() -> Vec<String>`; `FlowCache::{new(cap), get, insert}` | Bounded flow cache with eviction. Tests: `tiles_cover_grid_and_flag_occupancy`, `bake_hash_stable_x2`. |

### 3.4 Navigation mesh (`mesh.rs`) and traversal links (`links.rs`)

| Item | Signature and behaviour |
|---|---|
| `BakeConfig { cell_size: f32 (0.05..=5.0 m), walkable_normal_y: f32 (in (0.0, 1.0]) }`; `Default`: 0.25 m, 0.7 | |
| `mesh::bake(tris: &[[[f32;3];3]], &BakeConfig) -> Result<NavMesh, NavError>` | Voxel bake of triangles. |
| `NavMesh::{polygon_count, polygons, polygon(PolyId), cell_size_m, grid_dims, walkable_cells, polygon_at(point), query(from, to) -> Result<Vec<[f32;3]>, NavError>, query_area(center, radius_m), to_bytes, from_bytes, digest}` | `PolyId` dense, ordered by minimum cell. |
| `StackedNav::bake_stacked(...)`, `query_stacked`, `levels()`; `MeshTiles::{from_mesh(&NavMesh, edge), version, bump_exactly, to_bytes, from_bytes}`; `stitch_shared_edge(&MeshTiles, &MeshTiles) -> bool` | Multi-level and tiled forms. |
| `links::TraversalLink { id: LinkId, kind: LinkKind, edge_m: [f32;2], tile: TileId }`; `rebake_links(&[TraversalLink], dirty: &[TileId]) -> Result<Vec<LinkId>, LinkError>`; `rebake_for_set(links, &BreakSet)` | A break set rebakes exactly the links on its tiles. |
| `rebake::rebake_mesh_tile(...)` | Per-tile mesh rebake. |

Tests: `node tools/gh-check.mjs cargo test -p engine-nav --lib` and `--test tiles`.

## 4. `engine-nn` (`crates/nn`)

Integer-only neural runtime for `.e2nn` files. Status (crate docs): offline and tooling consumers
only. No arena or game tick binds a net yet (EG24(d) open). Anim motion-matching nets and
`ai::NetPolicy` load it.

### 4.1 File layout

| Field | Encoding |
|---|---|
| Header | `MAGIC` `b"E2NN"`, `version` u8 = 1, `layer_count` u8 (1..=16), `input_dim` u16 (1..=1024). |
| Layer tag 1, dense | `out_dim` u16, `mult` i32, `shift` u8 (0..=31), `act` u8 (0..=4), `bias: [i32; out]`, `weights: [i8; out*in]` row-major, then `lut: [i8; 256]` only when `act == 4`. |
| Layer tag 2, embedding | `vocab` u16 (1..=4096), `emb_dim` u16, `mult`, `shift`, `act`, table, optional LUT. |
| Layer tag 3, GRU | `hidden` u16 (1..=512), `mult`, `shift`, then gates z, r, n each with bias and `w_ih`, `w_hh`. |

Activations: `ACT_NONE` 0, `ACT_RELU` 1, `ACT_TANH` 2, `ACT_SIGMOID` 3, `ACT_LUT` 4. Each layer
saturates to `[-128, 127]`. Requantization is an integer multiplier plus arithmetic shift, round-half-away-from-zero.

### 4.2 API (`crates/nn/src/lib.rs`, `quant.rs`)

| Item | Signature and behaviour |
|---|---|
| `load(&[u8]) -> Result<Model, NnErr>` | Malformed bytes refuse. Caps: `MAX_LAYERS` 16, `MAX_DIM` 1024, `MAX_VOCAB` 4096, `MAX_HIDDEN` 512, `MAX_PARAMS` 4 194 304. |
| `infer(&Model, &[i8]) -> Vec<i32>` | Allocating convenience. Short inputs are zero-padded, long ones truncated. |
| `infer_into(&Model, &[i8], &mut Scratch, out: &mut [i32]) -> Result<(), NnErr>`; `Scratch::for_model(&Model)`; `Scratch::width()` | In-tick entry; zero heap per call. `OutputLen` if `out` length differs from `output_dim()`. |
| `Model::{input_dim, output_dim, layers, max_width, digest}`; per-layer accessors (`out_dim`, `mult`, `shift`, `act`, `bias`, `weights`, `lut`, `vocab`, `emb_dim`, `table`, `hidden`) | `digest()` is FNV-1a 64 of the file bytes: a tape can pin the net that decided. |
| `NnErr`: `BadMagic`, `BadVersion`, `Truncated`, `BadLayerCount`, `BadDim`, `BadLayerTag`, `BadShift`, `BadActivation`, `TooLarge`, `TrailingBytes`, `OutputLen`, `ScratchTooSmall` | Typed; never a panic. |
| `quantize_lane(value: f32, steps_per_unit: f32) -> i8`; `quantize_lanes(&[f32], &[f32], out: &mut [i8]) -> usize` | The one float-to-int edge. NaN gives `QUANT_NAN` 0; clamps to `QUANT_MIN` -127 and `QUANT_MAX` 127. |
| `requant_params(ratio: f64) -> Option<(i32, u8)>`; `weight_scale(&[f32]) -> f32`; `SIGMOID_TABLE`, `TANH_TABLE: [i8; 256]` | Export-side helpers. `QAT_INPUT_STEPS` 16.0 (`qat.rs`). |

Example, adapted from `dense_single_golden` (`crates/nn/src/lib.rs`):

```rust
let m = load(&dense_golden_bytes()).expect("golden loads");
assert_eq!((m.input_dim(), m.output_dim()), (2, 2));
assert_eq!(infer(&m, &[7, 8]), vec![-10, 81]);
```

Hand-computed FPS duel golden (`fps_duel_shape_golden_and_tick`): 6 lanes in, 5 intents out, identity
requant, no activation; `[1,2,3,4,5,6]` gives `[31, -31, 1, 12, 5]`.

Tests: `node tools/gh-check.mjs cargo test -p engine-nn --lib` (`weight_scale_is_peak_over_127_and_one_for_zeros`,
`requant_params_pick_the_largest_fitting_shift`, `saturating_narrow_clamps_both_edges`).

## Where the code is

- `crates/anim/src/contract.rs`: `Skeleton`, `LocalPose`, `RootDelta`, `RootTable`, `ContactTable`, `AnimError`, bone roles.
- `crates/anim/src/clip.rs`, `pose.rs`, `pack.rs`, `edit.rs`: baked clips and cursors, pose sampling, root extraction, pack, edit verbs.
- `crates/anim/src/locomotion.rs`, `readable.rs`, `loco_bake.rs`: gait machine, facing overlay, baked locomotion set.
- `crates/anim/src/ik.rs`, `fbik.rs`, `feet.rs`: two-bone, FABRIK, full-body, foot lock.
- `crates/anim/src/inertial.rs`, `inertialize.rs`, `crossfade.rs`, `springs.rs`: switches, foot lock and angular limit, fades, secondary motion.
- `crates/anim/src/blend.rs`, `graph.rs`, `layers.rs`, `events.rs`, `space.rs`: blend machine, graph, masked layers, frame events, 2D blend space.
- `crates/anim/src/state_machine/`: `playback.rs`, `edit.rs`, `door.rs`, `json.rs` (feature `json`), `anim_door.rs`.
- `crates/anim/src/warp_ground.rs`, `hit_react.rs`, `traversal.rs`, `exert.rs`, `injured.rs`, `lod.rs`: ground warp, flinch, traversal, exertion, limb-loss masks, LOD tiers.
- `crates/anim/src/motion_match.rs`, `motion_net.rs`, `motiondb.rs`, `net_pose.rs`, `face.rs`, `viseme.rs`, `lipsync.rs`, `trails.rs`: motion matching, learned pose path, replication, face and speech presentation, trails.
- `crates/ai/src/gladiator.rs`: `Brain`, `Percept`, `Intent`, `pick_intent`, `features_from`, `NetPolicy`, `self_play_*`.
- `crates/ai/src/depth.rs`: `DepthBrain`, `ArenaPolicy`, `Phase`, `Role`, tier rows, feint.
- `crates/ai/src/utility.rs`, `behavior.rs`, `blackboard.rs`, `bt.rs`, `bout.rs`, `director.rs`: utility scorers, behaviour trees, GOAP, bouts, pacing.
- `crates/ai/src/curves.rs`, `curves_text.rs`, `tier.rs`, `personality.rs`, `reaction_delay.rs`: tunable curves, duel tiers, temperament, observation delay.
- `crates/ai/src/eqs.rs`, `hearing.rs`, `alert.rs`, `memory.rs`, `mind.rs`, `goalstack.rs`, `melee.rs`, `legion.rs`, `guard.rs`, `apply.rs`, `provenance.rs`, `dialogue.rs`: NPC cognition modules.
- `crates/nav/src/tiles.rs`: `TiledNav`, `BreakSet`, `ObstacleCache`, `TileRegistry`, `StampedPath`.
- `crates/nav/src/flow.rs`, `bake.rs`: flow fields, `BakedNav`, `FlowCache`.
- `crates/nav/src/mesh.rs`, `links.rs`, `rebake.rs`: navigation mesh bake, traversal links, rebake.
- `crates/nav/tests/tiles.rs`: tile and break tests.
- `crates/nn/src/lib.rs`: `.e2nn` format, `load`, `infer`, `infer_into`, `Model`, `NnErr`, activation codes.
- `crates/nn/src/quant.rs`, `qat.rs`: quantization edge, QAT input steps.
- `docs/HASH-LAW.md`: determinism tiers and the AI-services rule.
