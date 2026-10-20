# Reference: arena, combat, rules, input

Details only. Signatures are copied from source (file named in each section).
Units: meters, radians, centidegrees, permille, fixed 60 Hz ticks unless a row says otherwise.
Read first: `docs/HASH-LAW.md` (arena simulation is held to determinism tier 3) and
`docs/ARCHITECTURE.md` (`arena` may depend on `combat`, `ai`, `anim`, `input`, `destruction`,
`physics`; `combat`, `ai`, `anim` never import `arena`).

## 0. Rules across this group

| Rule | Where it applies |
|---|---|
| Authoritative math is `+ - * /`, `sqrt`, comparisons, integers and `Fixed`. No platform `sin`/`cos`/`exp`. | arena hashed state, `combat`, `rules`, `input` |
| Floats enter the arena checkpoint only as `to_bits`. | `arena::run` |
| v1 is frozen and is the default for every runner, tape, snapshot and `game::App`. | `FIGHT_RULES_V1`, `TABLES_V1`, `ArenaRules::V1` |
| v2 folds its version and team ids into the digest, so v1 and v2 hashes never compare equal. | `ArenaRules::v2_*` |
| Presentation never feeds the hash. | `run_presentation`, `run_loco_poses`, `presentation_pose`, cues, `trauma_shake`, `MoveData`, `LookState` |
| Invalid input never panics. Entry points return empty, `None`, or a typed error. | every entry point below |

Tests (heavy cargo runs on GitHub): `node tools/gh-check.mjs cargo test -p <crate> --lib`; combat integration: `... -p engine-combat --test it`.

## 1. `engine-arena` (`crates/arena`)

Headless deterministic match: fixed-tick root intent, combat, brains, anim presentation, FNV
checkpoint hash. Default build is std-only. Feature `sim` adds Rapier controllers. Feature
`spine` adds the `engine-ecs` spine step and parity harness.

### 1.1 Core types

| Type | Definition and range | Determinism | Errors |
|---|---|---|---|
| `ArenaConfig` | `{ seed: u64, ticks: u32, fighters: u8, half_extent_m: f32, table_version: u32 }`, Copy, PartialEq. `ticks` 1..=`MAX_TICKS` (100 000); `fighters` 2..=32; `half_extent_m` 5.0..=100.0; `table_version` 1 (startup 4) or 2 (startup 15, parry 7). | Validated before any step. | `ArenaError::{BadTicks(u32), BadFighters(u8), BadExtent, BadTableVersion(u32)}` from `validate(self)`, in that order. |
| `ArenaConfig::try_new(seed, ticks, fighters, half_extent_m)` | Sets `table_version` 1. | | `ArenaError` |
| `Checkpoint` | `{ tick: u32, hash: u64 }`, Copy, Hash. One per tick. | the hash | none |
| `TickEvents` / `TickBreaks` / `TickMatchEvents` | `{ tick, events: Vec<FightEvent> }` / `{ tick, events: Vec<BreakEvent> }` / `{ tick, exchange, sweep }`. Fighter-ID order; breaks are bond snaps then freed chunks. | events feed the hash; match events are observed only | none |
| `FighterState` | `{ id, pos: [f32;3] (feet, m), yaw: f32 (rad) }` | presentation view | none |
| `MatchSnapshot` | Opaque full restore state (tick, poses, wishes, buttons, combat, match track, brains, locomotion, destruction; controllers under `sim`). Single-use. | restore resimulates identically | consumed by `resume` |
| `ArenaRules` | `{ version, teams: [u8;16], collision: bool, pillars: [Option<Pillar>;8], balance: bool }` | v2 digest adds tags (1.3) | `is_known()` |
| `FighterInput`, `FighterTape`, `FighterId`, `Buttons` | Re-exported from `engine-input` (section 4). | canonical bytes | `FighterError` |

Re-exports: `BodyId`, `BodyInput`, `BodyTape` (genre-neutral aliases), `MAX_PARTICIPANTS` (32),
`TICK_HZ`, `TOP_SPEED_MPS` (5.0), `FIGHTER_RADIUS_M` (0.4), `FIGHTER_HEIGHT_M` (1.8),
`AI_PERIOD_TICKS` (6), `MAX_TICKS`. Under `sim`: `SIM_TRACK` (true), `CAPSULE_CYL_H_M` (1.0),
`CAPSULE_REST_Y_M` (0.9).

### 1.2 Run functions (`crates/arena/src/lib.rs`)

Invalid config or tape returns empty (or `None`), never a partial result. Each call rebuilds from
tick 0 (O(n) per call). For per-tick stepping use `LiveMatch` (1.4).

| Function | Returns | Notes |
|---|---|---|
| `run(&ArenaConfig, &FighterTape) -> Vec<Checkpoint>` | one per tick, or empty | Legacy v1. Brains fill slots with no tape input. |
| `run_controlled(&cfg, &tape, &mut dyn SlotController) -> Vec<Checkpoint>` | as `run` | Controller overwrites its slots after tape inputs win. |
| `run_events(&cfg, &tape) -> Vec<TickEvents>` | one row per tick | Event count and bytes feed the hash. |
| `run_breaks(&cfg, &tape) -> Vec<TickBreaks>` | one row per tick | A `FightEvent::Hit` fractures the bonded crate that tick. |
| `run_match_events(&cfg, &tape)` (V0 intake), `run_match_events_v1`, `run_match_events_versioned(&cfg, &tape, version: u32)` | `Vec<TickMatchEvents>` | Observes exchange and sweep; never feeds `run`. Versions: `MATCH_WIRE_VERSION_V0` (0, legacy), `_V1` (1, phased). |
| `run_match_bytes(&cfg, &tape)` (V0), `run_match_bytes_v1`, `run_match_bytes_versioned(&cfg, &tape, version)` | `Vec<Vec<u8>>` | Canonical match bytes per tick. |
| `run_presentation(&cfg, &tape) -> Vec<RootTable>` | root motion per fighter | Presentation only. |
| `run_loco_poses(&cfg, &tape) -> Vec<Vec<LocalPose>>` | poses per tick | Presentation only. |
| `presentation_pose(skeleton: &Skeleton, clip: &Clip, tick: u32) -> Result<LocalPose, AnimError>` | pose at `tick / 60` s | Presentation only. |
| `run_dataset(&cfg, &tape) -> Option<Dataset>` | rows = ticks x fighters | `recorder::{Dataset, Row, DamageLedger}`. |
| `run_prefix(&cfg, &tape, ticks) -> Option<(MatchSnapshot, Vec<Checkpoint>)>` | snapshot at `ticks` | `None` if `ticks > cfg.ticks`, bad input or unknown version. V0 intake. |
| `run_prefix_versioned(&cfg, &tape, ticks, version)` | as above | Unknown version gives `None`. |
| `resume(MatchSnapshot, &cfg, &tape) -> Vec<Checkpoint>` | suffix checkpoints | Config mismatch gives empty. Consumes the snapshot. |

Example, adapted from `same_tape_twice_gives_identical_checkpoints` (`crates/arena/src/lib.rs`):

```rust
let config = ArenaConfig { seed: 2040, ticks: 32, fighters: 2, half_extent_m: 20.0,
    table_version: TABLES_VERSION_V1 };
assert_eq!(run(&config, &tape), run(&config, &tape));
```

Example, adapted from `scripted_attack_tape_produces_hit_events`: an attack press at tick 0 lands a
`Hit` on tick 4 (startup 4) against an idle foe.

```rust
let events = run_events(&config, &attack_tape(&config));
let first = events.iter().find(|t| t.events.iter()
    .any(|e| matches!(e, FightEvent::Hit { .. }))).map(|t| t.tick);
assert_eq!(first, Some(4));
```

Tests: `node tools/gh-check.mjs cargo test -p engine-arena --lib`. Named: `neighbouring_seeds_differ`,
`invalid_config_or_tape_returns_empty`, `snapshot_restore_mid_match_resimulates_identical`.

### 1.3 Fight rules `ArenaRules`

| Item | Behaviour |
|---|---|
| `ArenaRules::V1` | `version = FIGHT_RULES_V1`, solo teams, `collision = false`, no pillars, `balance = false`. |
| `v2_free_for_all()` (= `v2_teams(solo_teams())`); `v2_teams(teams: [u8;16])`; `with_pillar(self, Pillar) -> Option<Self>` | v2 sets `collision = true`, `balance = true`. Equal team ids never strike. `None` when 8 pillar slots are used. |
| v2 rules (`FIGHT_RULES_V2` docs) | Impacts need the sweep-capsule test over integer-mm poses and `Fixed` facing. Teammates are not struck. Block and parry need the shield cone to face the attacker. Simultaneous impacts resolve from pre-resolution state. Brain wishes act in the foe frame. |
| v2 digest (`ArenaRules::fold`) | version u32, team ids, then `0xC0` plus pillar footprints (when collision runs), then `0xB1` plus each slot's `BalanceState::state_hash`, `0xB2` plus `StaggerMemory::state_hash`, `0xB3` plus `LimbPool::state_hash` (when balance is on). V1 with no collision and no balance returns the hash unchanged. |
| `collides()` | `collision && !cfg!(feature = "sim")`. Under `sim` the controller resolves motion. |

### 1.4 Live stepping (`crates/arena/src/live.rs`)

| Method | Signature and behaviour |
|---|---|
| `LiveMatch::new(&ArenaConfig) -> Option<Self>`; `with_rules(&cfg, ArenaRules) -> Option<Self>` | v1 by default. |
| `with_rules_world(config: &ArenaConfig, rules: ArenaRules, world: MatchWorld) -> Option<Self>` | Adds static obstacles. |
| `step(&mut self, &[FighterInput]) -> Option<LiveTick<'_>>` | One tick. `None` on a rejected slice, nothing steps. A step failure poisons the match: rebuild from tick 0. |
| `snapshot(&self) -> Option<MatchSnapshot>` (non-consuming); `from_snapshot(MatchSnapshot) -> Option<Self>` | Resumes bit-identically. `stamp`, `recorded_tape` also exist. |
| `balance(&self, slot: u8) -> Option<&BalanceState>`; `cues(&self) -> &[AttackStarted]` | Read-only. Cues come from press ticks under v2 rows. |

Example, adapted from `live_snapshot_resumes_bit_identical_without_consuming_the_match`:

```rust
let mut live = LiveMatch::new(&config).expect("live match");
for tick in 0..300 { live.step(tick_inputs(&tape, tick)).expect("live step"); }
let second = LiveMatch::from_snapshot(live.snapshot().expect("snapshot")).expect("resume");
assert_eq!(second.tick(), 300);
```

Tests: `live_match_equals_whole_match_runners_tick_for_tick`, `v2_live_match_digest_twice_equal_and_apart_from_v1`.

### 1.5 Gym (`crates/arena/src/gym.rs`)

| Item | Signature and behaviour |
|---|---|
| `GYM_NUM_ACTIONS` 3: `GYM_ACTION_ATTACK` 0, `GYM_ACTION_BLOCK` 1, `GYM_ACTION_DODGE` 2 | Legal-action mask per fighter. |
| `GymPhase` | out, knocked down, staggered, attacking, dodging, blocking, idle (first match wins). Derived, never stored. |
| `GymLive::new(&cfg) -> Option<Self>`; `step(&mut self, &[FighterInput]) -> Option<&[GymEvent]>`; `snapshot(&self) -> GymSnapshot` | Digest is 0 before the first step. |
| `observe_state(&cfg, &tape) -> Option<GymSnapshot>`; `initial_state(seed, fighters, half_extent_m) -> Option<Vec<GymFighterState>>` | |
| `reward_of`; `legal_mask_of`; `BatchEnv`; `vec_step(&[BatchEnv]) -> Vec<Option<GymSnapshot>>` | Derived views. `vec_step` keeps input order; a bad env yields `None` alone. |

Tests: `live_env_matches_prefix_observe_tick_for_tick` (gym.rs), `vec_step_keeps_input_order_and_isolates_bad_envs` (lib.rs).

### 1.6 Controllers (`crates/arena/src/controller.rs`)

`trait SlotController { fn controls(&self, slot: u8) -> bool; fn control(&mut self, view: &ControlView<'_>, row: &mut [FighterInput]); fn take_effects(&mut self) -> Vec<ScriptEffect> { /* empty */ } }`.
`apply_script_effects(&mut FightState, &[ScriptEffect])` applies effects in order. Claimed slots are clamped;
`SlotView` is the per-slot read (meters converted to mm internally). Test: `only_claimed_slots_change_and_values_are_clamped`.

### 1.7 Fight ring (`crates/arena/src/zones.rs`)

Pure classifier: no heap, no IO. The balance tournament and `game`'s ring module re-export these.

| Item | Value or signature |
|---|---|
| `CENTRE_R_M` 3.0; `RING_R_M` 8.0; `ArenaZone::{Centre, Ring, Wall, Gate}`; `index()` gives the `ZoneTally::shares` slot | |
| `zone_at(x_m, z_m, gate: [f32;2], gate_half_m: f32) -> ArenaZone` | Gate square tested first (`|x-gx| <= half` and `|z-gz| <= half`). Else radius from origin: `< 3` Centre, `<= 8` Ring, beyond Wall. |
| `ZoneTally`: `add`, `total`, `permille(zone)`, `entrance_share()`, `shares() -> [u32;4]` | Shares sum to exactly 1000. |
| `ZONES_ROWS` = `content/design/arena-zones.rows`; `GATE_ANCHOR_FALLBACK_M` = `[12.0, 0.0, 2.0]` | Fallback when the row is absent or malformed. |
| `gate_anchor(rows: &str) -> Result<[f32;3], ZoneError>`; `shipped_gate_anchor() -> [f32;3]` | `ZoneError::{MissingRow, BadField { line: usize (1-based), field: &'static str }}`. |

### 1.8 Static world, kit, colliders

`Obstacle { center: [f32;2], half: [f32;2], axis: [f32;2] }`: oriented box on the floor (meters, half > 0,
axis `[cos yaw, sin yaw]`). Fighters are circles. Resolve is a push-out with a fixed pass count and a
rollback if a corner pinch cannot be solved.

| Item | Signature and behaviour |
|---|---|
| `MAX_OBSTACLES` 64; `MAX_OBSTACLE_HALF_M` 100.0; `MatchWorld::empty()` (legacy match, digest 0) | |
| `MatchWorld::try_new(Vec<Obstacle>) -> Result<Self, WorldError>` | Sorts by `to_bits`, so order never matters. `WorldError::{TooMany(usize), NonFinite(usize), BadHalf(usize), BadAxis(usize)}`. |
| `resolve(&self, from, to, radius, limit) -> [f32;2]`; `settle(&self, at, radius, limit) -> Option<[f32;2]>`; `overlaps`; `digest`; `obstacles`; `is_empty` | `settle` returns `None` when a spawn cannot be moved clear. |
| `with_kit_zone(self, KitZone)`; `kit_zone()`; `MatchWorld::fps_template() -> Result<kit::FpsMap, kit::KitError>` | |
| `kit::KIT_JSON` (`content/arena/placement.json`); `kit_match_world()`; `kit_match_world_from_json(&str)`; `fps_test_map()` | 11 actors in the file, 6 physical boxes collide (`kit.rs`). `FpsMap { world, spawns: [[f32;2];2], yaws: [f32;2] }`; `FPS_LIMIT_M` 40.0. |
| `KitError::{Parse(&'static str), World(WorldError)}` | e.g. `"kit fps spawn blocked"`. |
| `KitZone::from_cooked(&[u8]) -> Result<Self, KitZoneError>`; `try_new(Vec<Collider>)`; `push_out`; `overlaps`; `footprint_count`; `digest` | Built once at load. `STEP_M` 0.5, `MAX_FOOTPRINTS` 8192. Only blocking layers make footprints. |
| `collide::Pillar::new(a: [i32;2], b: [i32;2])` (mm); `MAX_PILLARS` 8; `COLLIDE_RADIUS_MM` | Integer footprint, grown by the capsule radius. |

Tests: `kit_collision_a_walk_stops_at_the_pillar_and_the_open_world_walks_through` (kit_zone.rs),
`settle_moves_a_spawn_out_or_refuses` (world.rs).

### 1.9 The `sim` feature and scenarios

| Fact | Source |
|---|---|
| `sim = ["engine-physics/rapier"]`: one physics controller per fighter, `move_and_slide` once per tick. Controller hash folds behind the `sim` tag byte. No workspace manifest enables it; `game`'s `app` deliberately does not. | `crates/arena/Cargo.toml`, `crates/game/Cargo.toml` |
| Snapshot restore under `sim` moves live controllers (Rapier contact state is not in the public snapshot). | `MatchSnapshot` docs |

Run: `node tools/gh-check.mjs cargo test -p engine-arena --features sim --lib`
(`sim_tag_byte_separates_configurations`, `sim_controllers_settle_and_rest`).

### 1.10 Forks, probes, game spike

| Item | Behaviour |
|---|---|
| `fork::Fork::from_snapshot(MatchSnapshot, tick) -> Option<Self>`; `apply_proposal(&mut self, ForkProposal) -> Result<(), ForkError>`; `run_n(&self, ticks) -> ForkMetrics`; `try_run_n -> Result<_, ForkCrashed>`; `run_recorded -> (ForkMetrics, Dataset)` | Counterfactual play from a snapshot. `STUCK_SPEED_MPS` 0.2. |
| `gameworld::GameWorld::reset(game, seed)`, `step(action)`, `digest()`; `run_headless(game, seed, steps) -> Option<(i32, u64)>` | 10 integer games (`GAME_COUNT` 10, `GW1_STEPS` 10 000). Not in the golden. |

## 2. `engine-combat` (`crates/combat`)

Frame-data combat: hits, hit-stop, move tables, sweeps, cuts, exchanges. Std-only, integer, each
module a pure resolver that the arena calls.

### 2.1 Step functions and events (`fight.rs`)

| Item | Signature and behaviour |
|---|---|
| `FightState::new(count: u8) -> Result<Self, CombatError>` | `count` 1..=16. Fields `fighters: [FighterState; 16]`, `fighter_count`, `tick`, `hitstop`. |
| `FighterState` | `hp_permille` (0..=1000), `stamina` (0..=100), `attack_age`, `is_attacking`, `attack_kind`, `attack_buffer: StreamBuffer`, `block_age` (`u32::MAX` when not blocking), `is_blocking`, `dodge_age`, dodging, stagger, knockdown. Methods: `can_act`, `invulnerable[_with]`, `parry_open[_with]`. |
| `try_step(&[FighterInput], &mut FightState) -> Result<Vec<FightEvent>, CombatError>` | Strict, V1 rows. Validates first; a rejected tick leaves state untouched. |
| `try_step_into(inputs, state, &mut Vec<FightEvent>) -> Result<(), CombatError>` | Reused buffer; no heap per tick. `step`, `step_into` are lenient. |
| `try_step_with_tables[_into]`, `step_with_tables[_into]` (explicit `&Tables`) | |
| `try_step_v2_with_cues_into(inputs, poses: &[StrikePose], state, tables: &Tables, out: &mut Vec<FightEvent>, cues: &mut Vec<AttackStarted>) -> Result<(), CombatError>` | Rules v2 strike geometry; cues empty under V1 rows; `BadStrikePose(u8)`. |
| `validate_inputs(&[FighterInput], u8) -> Result<(), CombatError>`; `hitstop_for_impact(&Tables, kind: u8, parried: bool) -> u32`; `block_chip(damage: u16) -> u16` | Chip is `damage * num / den`. |
| `FightEvent` | `Hit{attacker,victim,damage}`, `Blocked{attacker,victim,chip}`, `Parried{attacker,victim}`, `Dodged{attacker,victim}`, `StaminaOut{fighter}`, `Knockdown{attacker,victim}`. Fighter-ID order. |
| `CombatError` | `BadFighterCount(u8)`, `TooManyInputs(usize)`, `BadInput{index, source: FighterError}`, `InactiveFighter{index, fighter}`, `DuplicateInput{index, fighter}`, `BadTableVersion(u32)`, `BadTableValue{field: &'static str, value: u32}`, `UnknownTableVersion(u32)`, `BadStrikePose(u8)`. |
| Limits; versions | `MAX_INPUTS_PER_TICK` 32; `MAX_EVENTS_PER_TICK` 512; `MAX_ATTACK_STARTED_PER_TICK` 16; `FIGHT_RULES_V1` 1, `FIGHT_RULES_V2` 2. |

Example, adapted from `v2_foe_ten_metres_away_is_not_hit` (`crates/combat/src/fight.rs`, which uses test-local helpers):

```rust
let mut state = FightState::new(2)?;
let input = |id: u8, buttons: Buttons| FighterInput { tick: 0,
    fighter: FighterId::new(id).unwrap(), mx: 0, mz: 0, facing: 0, buttons };
let mut events = try_step(&[input(0, Buttons(ATTACK)), input(1, Buttons::NONE)], &mut state)?;
for _ in 0..TABLES_V1.attack_startup_ticks {
    events.extend(try_step(&[input(0, Buttons::NONE), input(1, Buttons::NONE)], &mut state)?);
}
assert_eq!(events, vec![FightEvent::Hit { attacker: 0, victim: 1, damage: ATTACK_DAMAGE }]);
```

Tests: `node tools/gh-check.mjs cargo test -p engine-combat --lib`. Named: `commit_lock_refuses_late_dodge_under_v2_only`,
`recovery_chain_restarts_and_cues_once_under_v2`, `v2_teammate_is_not_hit` (fight.rs).

### 2.2 Frame data and move tables

| Item | Value (frames at 60 Hz unless stated) |
|---|---|
| `TABLES_V1` (`Tables::v1()`, 16 fields) | damage 120 permille; startup 4; recovery 8; attack stamina 25; dodge stamina 20; block hold 1/tick; regen 2/tick; dodge 10 with i-frames 6; chip 1/5; parry window 4; parry stagger 12; hit react 6; hit-stop 4; knockdown 30. |
| `TABLES_V2` (`v2_rows()`) | `TABLES_V1` with startup 15 and parry window 7. Input buffer 8 (`INPUT_BUFFER_TICKS_V2`). Light/heavy poise 60/100; stamina 15/30; poise regen delay 30; hit-stop light 3, heavy 6, parry 8. |
| `MOVE_JAB`, `MOVE_CROSS`, `MOVE_UPPERCUT` (`lib.rs`) | startup/active/recovery 4/2/8 (cancel `cross`), 6/3/12 (cancel `uppercut`), 9/3/16 (none). Test-pinned presentation rows, no production caller per source comment. |
| `MoveData { name, startup, active, recovery, cancel }`; `total()`; `can_cancel(into: &str, frame: u32)`; `move_row(&str)`; `MOVES_V1` | `can_cancel` needs `frame >= startup` and the name listed. |
| `parry_open(frame, start, end) -> bool`; `Trauma::from_permille(u16)`; `trauma_shake(seed, trauma, tick) -> (f32, f32)` | Parry window is inclusive. Shake is presentation only. |
| `tables::validate_tables(&MoveTableSet) -> Result<(), CombatError>`; `MoveTableSet::{v1, v2, new(version, Tables), validate}` | Version 0 refused. Bounds: startup <= 60, recovery and stagger <= 120, parry window <= 60, hit-stop <= 60, knockdown <= 600, chip den <= 16, damage <= 1000. First failure wins. |
| `SwapInput::new(version, Tables) -> Result<Self, CombatError>`; `tables_for_version(u32) -> Option<&'static Tables>`; `step_for_version`; `try_step_for_version` | Swap is applied only at recorded tick commands. `TABLES_VERSION_V1` 1, `_V2` 2. |

### 2.3 Sweep, cut, exchange, depth, body model

| Item | Signature and behaviour |
|---|---|
| `sweep` bounds: `MAX_SWEEPS_PER_TICK` 8; `MAX_SWEEP_EVENTS_PER_TICK` 24; `SWEEP_Q` 8192; `POS_BOUND_MM` 200 000; `HURT_HALF_MAX_MM` 2000; `SHIELD_HALF_MAX_DEG` 90; `BLADE_HALF_MAX_MM` 500; `CONTACT_SLACK_MM_V1` 4; `MAX_SWEEP_LEN_MM_V1` 6000 | `validate_sweep`, `validate_fighter` refuse with `SweepError`. |
| `WeaponSweep { attacker: u8, p0_mm: [i32;3], p1_mm: [i32;3], blade_half_mm: u32 }`; `SweepParticipant { id, pos_mm, facing_x, facing_z, hurt_half_height_mm, hurt_radius_mm, .. }` | Facing is a scale-free integer vector. |
| `sweep_one(&attacker, &defender, &sweep, &SweepTables) -> SweepOutcome`; `try_resolve_tick(fighters, sweeps, &SweepTables, &mut [SweepEvent; 24]) -> Result<usize, SweepError>` | `Miss(SweepMiss)` names its gate. `SwingGate::fresh(swing, attacker)` and `claim(victim)` give one hit per victim per swing. |
| `cut::resolve_swing(swing: &BladeSwing, targets: &[JointTarget], anatomy: &AnatomyTable, worn: u8, tables: &CutTables, out: &mut [Cut; MAX_CUTS_PER_SWING]) -> usize`; `cut::try_resolve_swing(...same...) -> Result<usize, CutError>` | Lenient returns cut count. Strict validates `CUT_VERSION_V1` tables and anatomy first. `cut::digest_cuts(&[Cut]) -> u64`. |
| `exchange::resolve_tick(state: &mut ExchangeState, intents: &[ExchangeIntent; 16], tables: &ExchangeTables, out: &mut [ExchangeEvent; exchange::MAX_EVENTS_PER_TICK]) -> usize` | Parry, riposte, clash. `exchange::MAX_EVENTS_PER_TICK` is 8, not the fight cap. |
| `exchange::try_riposte(state, tables, attacker: u8, victim: u8) -> Result<ExchangeEvent, ExchangeError>`; `validate_phased(&PhasedTableSet)`; `phased_for_version(u32)` | Phased versions gate the match track. |
| `depth::PoiseMeter::apply_hit(u16) -> bool`; `depth::StaminaPool::try_drain(u16) -> bool`; `DepthSet::try_spend(slot, amount) -> Result<bool, DepthError>`; `record_strike(kind) -> Result<Option<u8>, DepthError>`; `observe(&[FightEvent])` | Integer meters. |
| `anatomy::validate(table: AnatomyTable) -> Result<(), AnatomyError>` (version 0 and unknown refused); `limbloss::BodyState::apply_cut(&mut self, &WoundTable, &AnatomyTable, &Cut) -> bool`; `step(&mut self, &WoundTable) -> Option<LimbEvent>`; `hitsim::simulate(anatomy, wound, spec: &HitSpec) -> Result<HitReport, HitSimError>` | Integer bleed by `BleedClass`; seeded distribution over clean hits. |

Tests: `steel_live_only_during_active`, `out_of_reach_sweep_misses` (sweep.rs).

## 3. `engine-rules` (`crates/rules`)

Game rules as sandboxed integer-only Rhai. Off `default-members`. Not a dependency of any tick crate.
The arena reaches it only through `arena::RulesController` (feature `arena`).

### 3.1 Invariants and budgets

| Item | Value (source) |
|---|---|
| Rhai pin; ABI | `RHAI_VERSION` = `"1.26.1"` (op counts are a replay contract). `RULES_ABI_VERSION` = 3; `rules_abi(&Limits) -> u64` hashes limits and `HOST_FNS`. |
| `Limits::DEFAULT` | `max_ops` 2000; `max_call_levels` 8; `max_expr_depth` 32; `max_fn_expr_depth` 16; `max_source_bytes` 16 384; `max_actions_per_call` 8; `max_variables` 32; `max_functions` 16. |
| `MAX_FIGHTERS` 64; `MEM_SLOTS` 8; host ranges | `MAX_AMOUNT` 1000; `MAX_TICKS` 600; `MAX_PUSH_MM` 5000; `MAX_WISH` 100; `MAX_FACING_CDEG` 18000; `BTN_ATTACK` 0x01, `BTN_BLOCK` 0x02, `BTN_DODGE` 0x04. |

### 3.2 Script surface

Scripts define `fn on_tick(me)`, called for each living actor in slot order. Scripts never write state:
they emit `Action`s, which the arena applies in order.

| `Action` | Fields | Effect |
|---|---|---|
| `Damage` / `Heal` / `DrainStamina` | `target: u8, permille: u16` (drain: `amount`) | Remove or restore health, or drain stamina. Heal caps and never raises the dead; drain saturates at 0. |
| `Stun` / `Shield` | `target, ticks: u16` | Stagger keeps the longest. Shield is a rule shield. |
| `Push` | `target, dx_mm: i32, dz_mm: i32` | Host `push` range-checks both axes to ±5000 mm. |
| `Wish`, `Face`, `Press` | `mx, mz` (±100) / `cdeg` (±18000) / `buttons` (0..=7) | Input actions. The reference arena ignores them. |

`HOST_FNS` (name, arity): `tick` 0, `hp_of` 1, `stamina_of` 1, `team_of` 1, `alive` 1, `x_of` 1, `z_of` 1, `dist` 2,
`nearest_foe` 1, `foes_within` 2, `roll` 1, `clamp` 3, `mem_get` 1, `mem_set` 2, `damage` 2, `heal` 2, `stun` 2, `push` 3,
`drain_stamina` 2, `shield` 2. `INPUT_FNS` (not in `rules_abi`): `walk` 2, `face` 1, `press` 1.
`PROPS`: `id`, `team`, `x`, `z`, `facing`, `hp`, `stamina`, `attack_age`, `attacking`, `blocking`, `dodging`,
`react`, `knockdown`, `shielded`, `mx`, `mz`, `buttons`, `attack`, `block`, `dodge`.

### 3.3 `Rules` (`crates/rules/src/rules.rs`)

| Method | Behaviour |
|---|---|
| `Rules::new(Limits)`; `set(&mut self, source: &str, tick: u32) -> Result<SetReceipt, RulesError>` | `set` compiles, vets, smoke-runs, installs. Failure keeps the last good script. Receipt `{ rev, hash, replaced, cases, max_ops }`. |
| `load_exact(&mut self, source, tick)`; `test(&self, source, &[World]) -> TestReport` | `load_exact` compiles and vets only (replay). `test` installs nothing. |
| `explain(&self, &World)`; `explain_slots(&self, &World, slots: u64)`; `tick(&mut self, &World) -> Result<TickReport, RulesError>`; `tick_slots(world, slots: u64)` | `explain` logs every host call, state untouched. `tick` failing calls become `Fault`s, never panics. Empty report if no script. |
| `snapshot() -> MemSnapshot`; `restore(&MemSnapshot)`; `state_digest() -> u64`; `reset_memory()`; `script_hash() -> Option<u64>`; `history()`; `refusals()`; `last_refusal()` | Memory only for snapshot. |

`World { tick: u32, seed: u64, fighters: Vec<ActorView> }`. `ActorView` fields: `id`, `team`, `alive`, `x_mm`, `z_mm`,
`facing_cdeg`, `hp_permille`, `stamina`, `attack_age`, `attacking`, `blocking`, `dodging`, `react`, `knockdown`,
`shield`, `mx`, `mz`, `buttons`.

`RulesError`: `TooLarge{bytes,max}`, `Float{line}`, `Syntax{line,message}`, `UnknownVariable`, `UnknownHostCall`,
`Forbidden`, `UnboundedLoop`, `Limit` (`LimitKind`), `Host(HostFault)` (`BadTarget`, `BadArg`, `ActionCap`, `MemSlot`),
`Arithmetic`, `Thrown`, `Runtime`, `BadWorld`, `NoScript`.

Example, adapted from `shield_bash_installs_and_acts` (`crates/rules/src/tests.rs`):

```rust
let mut rules = Rules::default();
let receipt = rules.set(SHIELD_BASH, 0).expect("installs");
assert_eq!(receipt.rev, 1);
assert!(rules.tick(&bash_world(100)).expect("tick").faults.is_empty());
```

### 3.4 Tapes and replay (`crates/rules/src/tape.rs`)

| Item | Behaviour |
|---|---|
| `TAPE_MAGIC` `b"E2RL"`; `TAPE_VERSION` 1; `MAX_TAPE_TICKS` 200 000; `MAX_TAPE_INPUTS` 2 000 000; `MAX_TAPE_SCRIPTS` 256 | Header holds the ABI id and script hashes. |
| `ScriptLibrary::{new, add(&str) -> u64, get(u64), len, is_empty}`; `RulesTape::{encode, decode}` | `TapeError::{BadMagic, Version(u16), Truncated, Trailing, Invalid(&str)}`. |
| `record(&RecordSpec, Vec<ScriptInput>, &mut ScriptLibrary) -> Result<RulesTape, RulesError>`; `replay(&RulesTape, &ScriptLibrary, Limits) -> Result<u64, ReplayError>` | `ReplayError`: `Tape`, `AbiMismatch{tape,engine}`, `MissingScript(u64)`, `LibraryCorrupt(u64)`, `Script`, `Tick`, `Divergence{tick,..}`. |

Tests: `node tools/gh-check.mjs cargo test -p engine-rules --lib` (`replay_10000_ticks_equal_twice`,
`replay_detects_divergence_and_abi_change`, `tape_decoder_distrusts_everything`). Arena binding:
`cargo test -p engine-rules --features arena --lib` plus `tests/arena_drive.rs`, `tests/arena_budget.rs`.

## 4. `engine-input` (`crates/input`)

Deterministic action map for the fixed tick. `FighterInput` is one validated command per fighter per tick.

### 4.1 Fighter contract (`fighter.rs`)

| Item | Value or signature |
|---|---|
| `FighterInput { tick: u32, fighter: FighterId, mx: i8, mz: i8, facing: i16, buttons: Buttons }` | Copy, Hash. |
| `WISH_RANGE` 100 (`mx`, `mz` in ±100 percent of top speed); `FACING_RANGE` 18 000 (centidegrees) | |
| `MAX_PARTICIPANTS` 32; `FighterId::new(u8) -> Option<Self>`; `Buttons(pub u8)`: `ATTACK` 0x01, `BLOCK` 0x02, `DODGE` 0x04, `KNOWN_BUTTONS` 0x07 | Slots 0..=31. |
| `FighterInput::validate(self) -> Result<(), FighterError>`; `append_bytes(self, &mut Vec<u8>)` | Canonical little-endian bytes for tick hashing. |
| `FighterTape(pub Vec<FighterInput>)`; `validate(&self)` | Sorted by tick then fighter; no duplicates. |
| `FighterError`: `UnknownFighter(u8)`, `WishOutOfRange(i8)`, `FacingOutOfRange(i16)`, `UnknownButtons(u8)`, `UnsortedTape(usize)`, `DuplicateInput(usize)`; `range_detail()` | Neutral aliases: `BoutTape`, `ParticipantInput`, `ParticipantId`, `ParticipantError`. |

Example, adapted from `valid_input_passes_and_hashes_stably` (`crates/input/src/fighter.rs`):

```rust
let input = FighterInput { tick: 3, fighter: FighterId::new(0).unwrap(),
    mx: 100, mz: 0, facing: 0, buttons: Buttons(ATTACK) };
input.validate().unwrap();
let mut bytes = Vec::new();
input.append_bytes(&mut bytes);   // 4 + 1 + 1 + 1 + 2 + 1 = 10 bytes
```

Tests: `node tools/gh-check.mjs cargo test -p engine-input --lib` (`tape_order_and_duplicates_rejected`,
`wish_and_facing_ranges_rejected`, `sixteen_slot_boundary_validates`).

### 4.2 Bindings (`bindings.rs`)

| Item | Behaviour |
|---|---|
| `BoundAction`: `Attack`, `Block`, `Dodge`, `MoveForward`, `MoveBack`, `MoveLeft`, `MoveRight`; `ACTION_COUNT` 7 | |
| `PhysicalButton::from_parts(kind, code) -> Option<Self>` (`KIND_KEY` 0, `KIND_MOUSE` 1, `KIND_PAD` 2); `Bindings::new()`; `button(action)`; `rebind(action, button)`; `set_deadzone(u8)`; `DEFAULT_DEADZONE` 8 | |
| `button_bits(&[PhysicalButton]) -> Buttons`; `wish_from_pressed(&[PhysicalButton]) -> (i8, i8)` | Forward 100, back -100, right 100, left -100. Opposed pairs cancel. Move keys never set bits. |
| `to_fighter_input(tick, fighter, &[PhysicalButton], wish: (i8,i8), facing: i16) -> Result<FighterInput, FighterError>` | Applies `apply_deadzone(mx, mz, deadzone)`, then validates. |
| `ActionSet`: `Duel`, `Menu`, `Pause`; `ActionSetStack::{push, pop, active, is_duel_active}`; `InputConfig::{from_bindings, to_bindings}` | Menu presses never reach duel. Unknown config kinds fall back to defaults. |

Example, adapted from `movement_presses_build_full_wish` (`crates/input/src/bindings.rs`):

```rust
let bindings = Bindings::new();
let fwd = bindings.button(BoundAction::MoveForward);
let back = bindings.button(BoundAction::MoveBack);
assert_eq!(bindings.wish_from_pressed(&[fwd]), (100, 0));
assert_eq!(bindings.wish_from_pressed(&[fwd, back]), (0, 0));
```

### 4.3 Gestures, frames, look

| Item | Behaviour |
|---|---|
| `TAP_MIN_HELD_TICKS` 2, `TAP_MAX_HELD_TICKS` 12, `DOUBLE_TAP_MAX_GAP_TICKS` 18, `HOLD_MIN_HELD_TICKS` 30; `GestureDetector::{new, reset, tick(pressed) -> Option<Gesture>}`; `Gesture::{Tap, DoubleTap, Hold}` | Input-prep detector. |
| `Frame`; `InputMap<A>::{bind, pressed, just_pressed, just_released, action_bits, hash_frame}`; `ActionState::{sample(digital, analog), pressed, held, bits, digest}` | Generic action map and held state. `MAX_ACTIONS` 64; `MAX_ANALOG` 8. |
| `apply_deadzone(mx: i8, mz: i8, deadzone: u8) -> (i8, i8)` | Snaps small wishes to zero. |
| `LOOK_RATES` (`yaw_permille_per_tick` 40, `pitch_clamp_centideg` 8000); `LookState::{zero, new(rates), apply_deltas(dx_milli_deg, dy_milli_deg) -> Result<(), LookError>, digest}` | Presentation only: `look.rs` says it never enters the fighter record, the tape or a tick hash. |

## Where the code is

- `crates/arena/src/lib.rs`: `ArenaConfig`, `ArenaRules`, `run*`, `run_prefix`, `resume`, `MatchSnapshot`, `Sim`.
- `crates/arena/src/live.rs`, `gym.rs`: `LiveMatch`, `LivePresentation`; `GymLive`, `BatchEnv`, `vec_step`.
- `crates/arena/src/controller.rs`: `SlotController`, `ControlView`, `SlotView`, `apply_script_effects`.
- `crates/arena/src/zones.rs`: fight ring classifier, gate anchor.
- `crates/arena/src/world.rs`, `kit.rs`: `Obstacle`, `MatchWorld`; embedded placement and FPS template.
- `crates/arena/src/kit_zone.rs`, `collide.rs`: cooked collider zone (`KitZone`); `Pillar` round-wall collision.
- `crates/arena/src/recorder.rs`: `Dataset`, `Row`, `DamageLedger`.
- `crates/arena/src/fork.rs`, `probe.rs`, `sync.rs`, `parity.rs`, `spine_step.rs`: forks, metrics, system hashes, spine.
- `crates/arena/src/gameworld.rs`, `seed.rs`, `examples/fk1_hashdump.rs`, `benches/headless_ticks.rs`: GW1 spike, seeds, FK1 scenario, bench.
- `crates/combat/src/fight.rs`: `Tables`, `FighterState`, `FightState`, `FightEvent`, `try_step*`.
- `crates/combat/src/tables.rs`: move-table versions, `SwapInput`, bounds.
- `crates/combat/src/lib.rs`: `MoveData`, `MOVE_*`, `Trauma`, `trauma_shake`, `parry_open`.
- `crates/combat/src/sweep.rs`, `cut.rs`: swept hit detection; blade cuts.
- `crates/combat/src/exchange.rs`, `depth.rs`: parry, riposte, clash; stamina, poise, combo chains.
- `crates/combat/src/hitstop_kind.rs`, `design.rs`, `anatomy.rs`, `limbloss.rs`, `hitsim.rs`, `gore.rs`, `projectile.rs`: hit-stop rows, timing, body model, outputs.
- `crates/combat/tests/it/main.rs`: integration tests (`--test it`).
- `crates/rules/src/rules.rs`: `Rules`, `set`, `tick`, `test`, `explain`, `RHAI_VERSION`, `RULES_ABI_VERSION`.
- `crates/rules/src/host.rs`, `limits.rs`, `error.rs`: `Action`, `World`, `ActorView`, `HOST_FNS`, `Limits::DEFAULT`, `RulesError`.
- `crates/rules/src/tape.rs`, `sim.rs`, `arena.rs`: tape format and replay; reference arena and FPS rules; `RulesController`.
- `crates/rules/scripts/*.rhai`: shipped rule scripts.
- `crates/input/src/fighter.rs`: `FighterInput`, `Buttons`, `FighterTape`, `FighterError`.
- `crates/input/src/bindings.rs`: `Bindings`, `BoundAction`, `PhysicalButton`, `ActionSetStack`, `InputConfig`.
- `crates/input/src/gestures.rs`, `map.rs`, `action_state.rs`: gestures, action map, held state.
- `crates/input/src/look.rs`: presentation look state. `docs/HASH-LAW.md`: the determinism contract.
