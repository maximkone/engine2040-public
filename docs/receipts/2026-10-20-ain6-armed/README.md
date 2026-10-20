# AIN6 armed receipt (2026-10-20)

Owner order, in chat: **"ARM GOO ALL"** (2026-10-20). This is the written
receipt for the AIN6 movement and camera numbers: every number is named below,
with the first tick the armed controller tape differs. The root golden does not
move; the sim controller's hash tail is the only hashed change, and it is named.

## What is armed

| site | before | after |
|---|---|---|
| `crates/arena/src/lib.rs` `build_controllers` (sim leg) | `Controller::spawn` | `.with_air(AirTuning::shaped())` |
| `crates/game/src/bin/arena.rs` `new_cam_follow` | `CamFollow::new()` | `CamFollow::with_look_ahead()` |

The presentation (`app`) leg does not enable `engine-arena/sim`; it keeps its
own locomotion. The camera lead is live in the window. The air table is live on
the sim controller and in its snapshots.

## The numbers

| channel | number | unit | armed path |
|---|---|---|---|
| air control gain | 0.30 | scale on the wish | `AirTuning::shaped` |
| turn brake | 0.20 | gain on a full reversal, 1.0 on the line | linear in `dot(wish, facing)` |
| apex speed | 2.0 | m/s vertical | gravity halves under it |
| apex gravity | 0.5 | scale near the apex | under `apex speed` |
| coyote | 120 | ms | `AIR_COYOTE_MS` |
| jump hold | 6 | ticks | `AIR_JUMP_HOLD_TICKS` |
| hold cut | 0.5 | rise kept on release | `AirTuning::shaped` |
| camera horizon | 0.5 | s | `LookAhead` |
| camera lead cap | 1.2 | m | `LookAhead` |
| accel halflife | 0.2 | s | `LookAhead` |
| decel halflife | 0.1 | s | `LookAhead` |
| juggle budget | 15 | hits | **not built** (see open item) |

The default `AirTuning` remains today's flat table, so tests and callers that do
not arm the channel keep the old step. `Controller::state_hash` folds
`air_facing` and the hold deadline only when armed, so the sim match hash takes
the armed tail from tick 0; positions and speeds before the first air tick are
unchanged.

## First differing tick

The fixed tape in `crates/physics/src/controller.rs`
(`air_channel_shaped_tape_names_its_first_diff_tick`): one settle move, 12
grounded moves holding +x, a jump at 5.0 m/s, then 120 air moves alternating
+0.15 and -0.15. Position and vertical speed first differ on **air tick 6**
(tape index 5, the first reversed wish); the turn brake is the cause. The test
pins `first == 5`.

The camera lead peaks while moving and decays to zero when stopped:
`camera::tests::look_ahead_peaks_while_moving_and_decays_to_zero_when_stopped`
and `fighter_look_cam::tests::look_ahead_leads_a_walking_hero_and_decays_when_stopped`.

## Checks

- `cargo test -p engine-physics --lib --features rapier air_` -> 7 passed.
- `cargo test -p engine-physics --lib --features rapier apex_gravity` -> 1 passed.
- `cargo test -p engine-physics --lib --features rapier jump_hold_deadline` -> 1 passed.
- `cargo test -p engine-game --lib --features app look_ahead` -> 2 passed.
- `cargo test -p engine-arena --lib` -> 227 passed, 0 failed, 5 ignored.
- Root `target/release/engine2040.exe` x2 -> `final_hash=d57b5c8ae3096614`
  both times, `state_hash=2aca9338670aa0cb`; the root does not link arena/game.

## Open item

The juggle budget has no consumer: combat has no vertical launch or air-combo
meter, so a 15-hit budget would be a constant with nothing to count. It stays
open on AIN6; the air ramp, apex, coyote, hold and camera lead are armed and
proven above.
