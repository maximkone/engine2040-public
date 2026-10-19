# Spear grapple - throw a spear, tether to the enemy, fly with physics

Owner request (Maxim, 2026-10-18): "make spear throwable and he connects to
enemy and fly with him toward where spear flew but with physics, Spiderman
grapples". This file is the contract: the authority half (arena, input, net)
is built first and is described here as built; the presentation half (window,
render, audio, HUD) reads only what is listed under "What presentation reads".

## One-screen summary

- New input bit `THROW_SPEAR = 0x10` in the fighter `buttons` byte (the 10-byte
  wire/tape record is unchanged; `Buttons::is_legal` now accepts
  `KNOWN_BUTTONS | THROW_SPEAR`; `KNOWN_BUTTONS` stays `0x07`).
- New `MatchSpec::spear: bool` (default false; `MatchSpec::with_spear()` turns
  it on). Off: nothing runs, nothing is hashed, the input bit is masked out of
  the latched buttons, and every old digest is byte-identical. On: the match
  carries a `SpearState` that is folded into every checkpoint and the spec
  identity changes.
- Press `THROW_SPEAR` (rising edge): a spear flies along the thrower's facing.
  It hits one of three things:
  1. an enemy: a TETHER forms; the thrower is reeled to the enemy, then both
     are carried with momentum toward the spear's end point; damage on
     arrival or on a wall slam;
  2. world geometry (arena wall, kit obstacle, rule pillar): the thrower zips
     to the anchor (Spiderman);
  3. nothing (range, floor) or a guard: no tether, only a cooldown.
- A second press while tethered releases the tether (momentum is kept and
  drifts out with friction). A blocking victim cuts it; a parry cuts it and
  staggers the thrower.

## Where it lives

- `crates/input/src/fighter.rs`: `THROW_SPEAR`, `ALL_BUTTONS`, `Buttons::is_legal`.
- `crates/arena/src/spear.rs` (new): all spear authority: `SpearState`,
  `SpearEvent`, `SpearView`, the step, the hash fold. Integer only.
- `crates/arena/src/spec.rs`: `MatchSpec::spear`, `with_spear`, identity.
- `crates/arena/src/lib.rs` (`Sim`): one `spear: Option<SpearState>` field and
  one call in `step_with` between the movement loop and the collision pass.
- `crates/arena/src/live.rs`: `LiveMatch::from_spec` arms it; accessors
  `spear_events()` and `spear_view()`.
- `crates/net`: no wire change. `Server::from_spec` / `Client::from_spec` /
  `DuelHost::new` already build `spec.live()`; the bit rides the existing
  buttons byte and the flag rides the spec identity (lobby build id), so a peer
  with another `spear` value is refused before tick 0. Rollback replays from
  the spec and the tape (`LiveMatch::snapshot` returns `None` when spear is on,
  like for rules v2), so a resim equals a single run.
- Not supported on the Rapier `sim` leg: `LiveMatch::from_spec` returns `None`
  for a spear spec built with `--features sim` (its controllers own the poses).

## Units and conventions

- Positions in integer millimetres (`pos_mm`), same as `match_wire::pos_mm`;
  velocities in mm per tick (60 Hz); `y` up, feet at 0. Directions are Q16
  unit vectors from `facing_q16` (centidegree facing, 0 is +z, 9000 is +x).
- Sphere/capsule hit test: `engine_physics::query::swept_cast` (PX20, Q16
  `Fixed` metres), spear tip radius 80 mm against a fighter capsule from
  y 400 to y 1400 mm, radius 400 mm (`FIGHTER_RADIUS_M`). Hits sort by time of
  impact; the nearest legal one wins.
- World hit: the tick's segment is marched in 100 mm steps; a step is "in
  geometry" when it is past the arena wall inset (`wall_limit`), inside a rule
  pillar footprint, or `MatchWorld::overlaps(point, 0.05)`.
  The destruction crate prop is not an anchor (fighters walk through it).
- Only `+ - * /`, `isqrt`, shifts and comparisons; no float feeds the hash. The
  bodies move by adding the integer displacement onto the float pose and
  resolving it through the same wall clamp, `MatchWorld::resolve` and
  `collide()` pass the walking code uses (rules v2 pillars and push-apart).

## Constants (mm, ticks, mm per tick)

| name | value | meaning |
| --- | --- | --- |
| `SPEAR_SPEED` | 600 | muzzle speed along the facing (36 m/s) |
| `SPEAR_GRAVITY` | 3 | vertical drop per tick squared |
| `SPEAR_START` | (400 ahead, y 1300) | spawn point from the thrower |
| `SPEAR_RANGE` | 12000 | max flight length (20 ticks) |
| `THROW_COOLDOWN` | 90 | ticks from the throw until the next throw |
| `TETHER_COOLDOWN` | 60 | cooldown restarted when a tether ends |
| `HIT_DAMAGE` | 30 | permille at impact on an enemy (stick) |
| `REST` | 1600 | latch distance between the two bodies |
| `REEL_ACCEL` | 20 | thrower accel toward the enemy before the latch |
| `CARRY_ACCEL` | 12 | accel of the latched pair toward the goal |
| `MAX_SPEED` | 420 | speed cap (25 m/s) |
| `WALL_REEL_ACCEL` | 28 | thrower accel toward a wall anchor (cap 500) |
| `ARRIVE` | 700 | goal radius that ends a pull |
| `ARRIVE_DAMAGE` | 40 + speed/8 | permille to the victim on arrival |
| `SLAM_DAMAGE` | 60 + speed/6 | permille to the victim on a wall slam, cap 200 |
| `TETHER_MAX` | 90 | ticks before the tether times out |
| `FRICTION` | v -= v/16 (min 1) | body drift when no tether holds it |
| `PARRY_STAGGER` | combat `PARRY_STAGGER_TICKS` | thrower `react` after a parry cut |

## Authority rules, per tick, in slot order

State per slot: `phase` (`Idle`, `Flight`, `Tether`), `cooldown`, `held`
(previous `THROW_SPEAR` level, for the edge), `drift` (body velocity, mm per
tick, x and z). The step is skipped while combat `hitstop > 0` (the spear
freezes with everything else).

1. Press (rising edge of `THROW_SPEAR`):
   - `Idle`, `cooldown == 0`, hp above zero, `react == 0`, not knocked down:
     throw. Emits `Thrown`.
   - `Tether`: release. Emits `TetherEnd(Released)`; `drift` is kept.
   - `Flight`: ignored.
2. Flight: the spear moves `vel` for the tick (`vel.y -= SPEAR_GRAVITY` first).
   For the tick's segment: world hit distance by marching; fighter hit by
   `swept_cast` over the same segment shortened to the world hit. Candidate
   fighters: living, not the thrower, not a teammate (`rules.teams`), not
   inside dodge i-frames. Outcomes, nearest first:
   - enemy, victim `is_blocking` with the thrower in front half-plane of the
     victim's facing: deflected. `Deflected { parried }`, parried when
     `parry_open`; no tether. A parry also sets the thrower's `react` to the
     combat parry stagger.
   - enemy otherwise: victim takes `HIT_DAMAGE`, a tether forms
     (`Tether { victim, goal }`), `Hit` is emitted. Goal = throw origin plus
     facing times `max(SPEAR_RANGE, travelled + 2500)`: "where the spear was
     flying". It is not clamped: a goal beyond the arena wall means the wall
     stops the victim and the tether ends in a slam.
   - world: tether to the anchor, goal = hit point pulled back 700 mm along
     the flight so the thrower stops short of the geometry. `Hit` with
     `World` target.
   - floor (`y <= 0`) or range exhausted: `Missed`; no tether.
3. Tether, every tick, integer:
   - Enemy tether, gap > `REST` (reeling): thrower accel `REEL_ACCEL` toward the
     victim plus `CARRY_ACCEL` toward the goal; victim accel `CARRY_ACCEL`
     toward the goal only (it is dragged, not pulled back).
   - gap <= `REST` (latched): both velocities become their average (momentum
     is shared, equal mass), then both accelerate `CARRY_ACCEL` toward the goal.
   - Wall tether: thrower only, accel `WALL_REEL_ACCEL` toward the goal.
   - Speeds are capped at `MAX_SPEED` (500 for the wall reel); each body moves
     by its velocity through the wall clamp, world and collision resolution.
   - A body whose move comes up more than half short of its velocity is
     blocked: its velocity is zeroed. A blocked victim ends the tether
     (`Wall`, `SLAM_DAMAGE` to the victim); a blocked thrower ends it
     (`Wall`, no damage).
   - Ends with `Arrived` when the lead body is within `ARRIVE` of the goal or
     has passed it (dot product), damage `ARRIVE_DAMAGE` to the victim (none
     for a wall tether); with `Timeout` after `TETHER_MAX`; with `Guard` when
     the victim `is_blocking`; with `Parry` when the victim `parry_open` (plus
     the thrower stagger); with `Down` when either side reaches zero hp.
   - On every end the bodies keep their velocity as `drift`, the cooldown is
     set to `TETHER_COOLDOWN`, damage is applied (hp clamps at zero).
4. Idle bodies: `drift` moves them the same way and decays by `FRICTION`.
5. Fold (only when spear is on), after the rules fold and before the world
   fold: tag byte `0x5A`, then per slot: phase tag, ints of the projectile or
   tether, cooldown, held, drift. Fighter poses and hp already hash through
   the normal state, so a pull shows up there too.

## Events (per tick, in emission order, cleared every step)

```
SpearEvent::Thrown   { thrower, at_mm: [i32;3], vel_mm: [i32;3] }
SpearEvent::Hit      { thrower, victim: Option<u8>, at_mm: [i32;3] }   // None = world anchor
SpearEvent::Missed   { thrower, why: MissWhy }                         // Range | Floor
SpearEvent::Deflected{ thrower, victim, parried: bool }
SpearEvent::TetherPull { thrower, victim: Option<u8>, speed: u16 }     // every 6th tether tick
SpearEvent::TetherEnd  { thrower, victim: Option<u8>, why: EndWhy, damage_permille: u16 }
EndWhy = Arrived | Wall | Timeout | Released | Guard | Parry | Down
```

Presentation maps these onto audio `FightCue`s and VFX; there is no new
`FightCue` variant (the audio enum is exhaustive in several crates).

## What presentation reads (all read-only, never hashed by the reader)

- `LiveMatch::spear_events() -> &[SpearEvent]`: the events of the last stepped
  tick.
- `LiveMatch::spear_view() -> &[SpearSlot]` (one per fighter, empty without
  the flag): `SpearSlot { phase: Phase, cooldown, held, drift_mm }` with
  `Phase::Idle | Flight(Flight { pos_mm, vel_mm, age, origin_mm, dir }) |
  Tether(Tether { victim: Option<u8>, goal_mm: [i32;2], dir, age, latched,
  speed })`. The types are `engine_arena::spear::{SpearSlot, Phase, Flight,
  Tether}`; `SpearEvent`, `EndWhy`, `MissWhy`, `SpearSlot` are also re-exported
  at the crate root. Fighter positions come from the
  normal pose path (they are moved by the sim). Draw the rope between the
  thrower root and the victim root (or `goal_mm` for a wall anchor); the spear
  is `projectile.pos_mm` pointing along `vel_mm`; a carry arc can be drawn as a
  pure function of `tether.age` (no state needed).
- Input: set bit `engine_input::fighter::THROW_SPEAR` in `FighterInput.buttons`.
  Consumers that mask with `KNOWN_BUTTONS` (combat feel, `ui` play HUD, the
  endpoint play DTO) treat `0x10` as unknown: mask it off (`buttons &
  KNOWN_BUTTONS`) before feeding them.
- Spec: `MatchSpec::new(..).with_spear()`; `MatchSpec::spear` for the build id.
  The pose cursor must run the same flag: build it with
  `LivePresentation::from_spec(&spec)` (not `with_rules_world`), or the drawn
  fighters will not follow the tether.
- Key binding: not added (the `ACTION_COUNT == 7` bindings table is frozen);
  the game ORs `THROW_SPEAR` into `Buttons` itself.

## Proofs (tests that go red without the feature)

Arena, `cargo test -p engine-arena --lib spear` (`crate::spear::tests`):
`spear_hits_enemy_and_drags_both_toward_the_goal`,
`spear_hits_wall_and_zips_the_thrower`, `spear_miss_leaves_no_tether`,
`parry_cuts_the_tether_and_staggers_the_thrower`,
`block_cuts_the_tether_and_a_facing_guard_turns_the_spear`,
`release_press_ends_the_tether_and_keeps_momentum`,
`slam_into_an_obstacle_ends_the_tether_with_damage`,
`a_second_throw_waits_for_the_spear_and_the_cooldown`,
`spear_off_has_no_state_and_the_button_changes_nothing`; in `live::spear_tests`:
`spear_duel_600_ticks_digest_identical_twice_and_across_tape_replay`,
`spear_off_ignores_the_button_and_keeps_the_old_digest`,
`spear_matches_have_no_snapshot_and_report_their_view`; in `spec::tests`:
`spear_spec_identity_moves_only_when_on`. Input:
`fighter::tests::throw_spear_bit_is_legal`. Net (`cargo test -p engine-net --lib
spear`, `session::spear_tests`): `spear_resim_equals_a_single_run_and_x2_is_identical`,
`spear_on_plays_another_match_than_spear_off_on_the_same_tape`,
`spear_lobby_refuses_a_peer_without_the_flag`,
`spear_browser_duel_host_builds_from_the_kit_spec_with_its_own_build_id`.
Frozen and re-run unchanged: golden `d57b5c8ae3096614`
(`engine-proof --lib golden_hash_untouched`) and the `arena_line` digest
`d29b3b17bbd3b244`.

## Rope swing (row SG4)

Shady Knight's wall tether is a pendulum, not a winch. A reel pulls the thrower in a
straight line; a swing constrains the thrower to a circle of radius `REST` (1600 mm)
around the anchor and lets gravity accelerate along the tangent. Releasing at the bottom
of the arc then carries that tangential speed into the launch, which is the whole point
of the mechanic. Numbers and grades: `rea-arsenal docs/findings/shady-knight-mechanics.md`
(the source game runs 10.5 m/s run, -40 m/s² gravity, a 25 m/s throw).

Rules, per tick, integer, wall tether only:

1. `d` = distance from anchor to thrower, in mm. If `d > REST`, radial correction:
   move the thrower back toward the anchor by `d - REST` along the unit direction.
2. Gravity per tick pulls the thrower down before the correction; the correction is what
   turns the fall into an arc. Same `SPEAR_GRAVITY` step the flight uses.
3. Damping is a constant fraction, not a multiplier chain: `v -= v / 32` per tick, so a
   release keeps most of the swing speed.
4. Release (`TetherEnd(Released)`) keeps the current velocity, as SG1 already does. The
   new part is that the velocity is now tangential, so it is worth more.
5. Enemy tethers keep the existing reel. Only the wall tether swings.

Accept: `rope_swing_release_lands_further_than_a_flat_jump` in `crate::spear::tests` - the
same seed and tape with the swing on lands at least 300 mm further than the jump-only
control. Golden `d57b5c8ae3096614` x2 unchanged: the fixture does not enable
`MatchSpec::spear`, so the rope never runs in the hashed baseline.

Status 2026-10-19: built three ways and none of them holds. The reel is the only force on
a wall tether and it points at the anchor, so on a floor-plane body a taut rope has no
tangential force to turn into an arc; the versions that do constrain the radius also stop
the zip that `spear_hits_wall_and_zips_the_thrower` and `play-arena.bat` depend on. The
missing part is height: a match fighter has no vertical velocity and `move_body` takes
`[x, z]` only. Row SG5 opens that channel and SG4 follows it. Full diagnosis:
`docs/plan/shady-knight-port.md` (section Rope swing).

