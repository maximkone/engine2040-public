# Spear grapple - throw a spear, tether to the enemy, fly with physics

Owner request (Maxim, 2026-10-18): "make spear throwable and he connects to
enemy and fly with him toward where spear flew but with physics, Spiderman
grapples". This file is the contract: the authority half (arena, input, net)
is built first and is described here as built; the presentation half (window,
render, audio, HUD) reads only what is listed under "What presentation reads".

## One-screen summary

- New input bit `THROW_SPEAR = 0x10` in the fighter `buttons` byte (the 10-byte
  wire/tape record is unchanged; `Buttons::is_legal` now accepts
  `KNOWN_BUTTONS | THROW_SPEAR`; `KNOWN_BUTTONS` stays `0x07`). AIN7 later adds
  `JUMP = 0x20` to `ALL_BUTTONS`; the mask consumers here are unchanged.
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
| `HIT_DAMAGE` | 5 | permille at impact on an enemy (stick); low on purpose, the pull is the payoff (steal #12: Overwatch cut its hook from 30 to 5) |
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
EndWhy = Arrived | Wall | Timeout | Released | Guard | Parry | Down | Cut
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
   turns the fall into an arc. The swing falls with `FIGHTER_GRAVITY` (11 mm/tick²,
   `crates/arena/src/lib.rs:171`); the spear in flight falls with `SPEAR_GRAVITY`
   (3 mm/tick², `crates/arena/src/spear.rs:38`).
3. Damping is a constant fraction, not a multiplier chain: `v -= v / 64` per tick
   (`crates/arena/src/spear.rs:82`), so a release keeps most of the swing speed.
4. Release (`TetherEnd(Released)`) keeps the current velocity, as SG1 already does. The
   new part is that the velocity is now tangential, so it is worth more.
5. Enemy tethers keep the existing reel. Only the wall tether swings.

Accept: `rope_swing_release_lands_further_than_a_flat_jump` in `crate::spear::tests` - the
same seed and tape with the swing on lands at least 300 mm further than the jump-only
control. Golden `d57b5c8ae3096614` x2 unchanged: the fixture does not enable
`MatchSpec::spear`, so the rope never runs in the hashed baseline.

Status 2026-10-19: landed with the height channel (SG5). `MatchSpec::with_height` gives a
fighter a vertical speed and gravity, so the wall anchor can be a rope: the hook sets the
3D radius to the anchor, the reel takes 60 mm of rope in per tick, `settle` puts the body
back on the sphere after the move (which is what turns the run and the take-in into an
arc), the drift keeps only its tangential part, and the release keeps it. Enemy tethers
and the floor-plane path (`height` off) are untouched, byte for byte. Numbers: 60 mm/tick
take-in, damping `v -= v/64`, rope floor 400 mm, gravity `FIGHTER_GRAVITY` = 11 mm/tick^2.
The window does not arm the channel yet (row SG7), so `play-arena.bat` still zips.

## Yank (row SG9)

Owner 2026-10-24: "grapple ppl and pull them and good af felt". The carry drags
both fighters away along the throw. A yank pulls the enemy to the thrower,
"get over here" style. The research in `docs/research/grapple-2026-10-24/` agrees
on what makes a hook pull feel good: a freeze on the catch, a pull that ends in
front of the hooker (not on top of them), and a stun long enough for the free hit
(Thresh tugs only while the target is stunned; Halo's grappleshot melees on arrival).

- Choose at the hit: the thrower's move wish against the flight direction
  (`mx*dir.x + mz*dir.z < 0`, S held while facing the throw) means yank, anything
  else is the old carry. No new button bit.
- On the catch: `HIT_DAMAGE` as before, fight hit-stop `max(hitstop, 5)`, victim
  `react = max(react, 20)` (a stunned fighter drops its guard, so neither `Guard`
  nor `Parry` can cut a yank). Both drifts are zeroed.
- Each pulled tick: the thrower plants (drift 0). The victim's speed is
  `clamp((gap - 1200) / 4, 30, 400)`, no more than the gap left, set straight
  toward the thrower (no ramp: the first tick is the snap, the quarter-gap
  ease is the soft landing). The stun is refreshed to 20 each tick.
- Ends: `Arrived` when the gap is inside `1200 + 30` mm, 20 permille, victim drift
  zeroed (they stop in front of the blade with about 20 ticks of stun left);
  a slam on the way (`victim_blocked`) keeps the carry's slam damage; `Timeout`
  and `Down` as before.
- Counters stay before the catch: a facing guard deflects the spear, a parry
  staggers the thrower, dodge i-frames make the spear pass through, and the
  throw cooldown. SG13 (a blade cuts the rope) is the counter once caught.
- Hash: a yank tether folds one extra byte `0xA7`; a carry folds as before.

## Rope cut (row SG13)

Steal #8 of `docs/research/grapple-2026-10-24/02-games.md` (the victim needs an
escape, or a hook is a free kill) and the "universal counter" of the Rome rows. A
blade that crosses the live rope ends the tether with `EndWhy::Cut`.

- Check: in `pull`, after the `Down` and `Timeout` ends and before the guard and
  parry checks, so a yank (whose victim is stunned and cannot guard) is cut too.
- The rope: the planar segment from the thrower to the victim (a carry; a yank:
  the planted thrower and the reeled victim) or to the world anchor (`anchor_mm`).
- The blade: the arena's own mapped sweep, `match_wire::sweep_for_attacker` of the
  swinger's position and `facing_vector` (base 200 mm to reach 1500 mm along the
  octant sign), planar. Live while `is_attacking` with `attack_age` from the impact
  tick (`heft::startup`) for `CUT_LIVE_TICKS` (4) ticks, and only for a swinger that
  is up (hp above zero, no `react`: the fight cancels the swing of a stunned fighter).
- Who: every fighter but the thrower (the victim escaping a carry, or a third
  fighter; a yank victim is stunned, so only a bystander cuts a yank). The thrower
  has the release press.
- Geometry: integer segment-to-segment gap, a cut when it is at most the blade half
  thickness plus 60 mm (`ROPE_HALF_MM`).
- On the cut: `TetherEnd { why: Cut, damage 0 }`, the spear drops, the throw cooldown
  restarts, bodies keep their drift (a yank victim stops dead, as on any end).
- Presentation: `window_audio_space::spear_cue` plays `sever`; `spear_look` drops the
  spear on any `TetherEnd`; `feel_view` shows nothing for a zero-damage end.
- Hash: no new bytes (the phase just goes idle). Spear off runs none of this; the
  golden and every spear-off digest are untouched.
- Proofs in `crate::spear::tests`: `a_victims_swing_across_the_rope_cuts_a_carry`,
  `a_third_fighter_cuts_a_yank`, `a_swing_that_misses_the_rope_does_not_cut_it`,
  `the_throwers_own_swing_does_not_cut_its_rope`,
  `rope_geometry_crossing_touching_and_missing`.

## Laquearius noose (row SG11)

Isidore (Etymologies 18.56, read in the Latin in `docs/research/grapple-2026-10-24/03-rome.md`)
has the laquearii throw a noose over men fleeing in the arena, tangle them, then chase them
down and knock them over with the shield boss. Whether it was a full bout class or a comic
type is debated; the game takes the text as written. The noose is the second rope weapon of the
arena: a cord that holds a man in place instead of carrying him, with the fight's own boss push
as the finisher.

- Opt in: `MatchSpec::noose` (`with_noose()`), independent of `MatchSpec::spear`; the spec
  identity folds byte `0x4E` only when it is on. Input bit `THROW_NOOSE = 0x80` in the same
  buttons byte (`0x08` stays unknown: many fixtures use it as the illegal bit). A match without
  the noose masks the bit out of the latched buttons (`leg_pool`), so a stray press cannot reach
  the hash, and every digest from before is byte-identical (golden `d57b5c8ae3096614`, the
  spear-on and spear-off tapes). Window: `Ctrl` throws the noose when the grapple is on
  (`arena_frame/grapple.rs`); `live_session` arms it with the spear (`App::into_noose`).
  `Sim::set_noose` refuses past tick 0 and on the Rapier `sim` leg, like the spear.
- It rides the spear's machinery: one `SpearState`, one phase per slot, one cooldown (90, then
  60 after an end), the same events, the same `rope_cut`. A loop in the air is a `Flight` with
  `noose` set; a snare is a `Tether` with `noose` set; a new `EndWhy::Toppled`. A noose-only
  match (`with_noose()` without `with_spear()`) throws no spears. A second press of the button
  that threw releases that cord (`Released`).
- Fold: `0x4E` after a noose flight or snare (a snare adds its cord length), plus one noose block
  `0x4F` (per-slot press edge and pending push) only while the noose is armed. A spear-only
  match folds exactly what it did.

Constants (mm, ticks, mm per tick):

| name | value | meaning |
| --- | --- | --- |
| `NOOSE_SPEED` / `NOOSE_MAX_TICKS` | 500 / 12 | the loop flies 6 m (spawns 400 ahead, same drop as a spear) |
| `LOOP_MM` | 150 | the open loop adds this to the body radius (400) when it closes; spear tip 80 on top |
| `CHASE_MM` | 250 | more on a man whose wish points away from the thrower (wish dot offset > 0) |
| `SNARE_TICKS` | 90 | ticks before the cord slips off (`Timeout`) |
| `WALK_PERCENT` | 40 | share of the walk wish a snared man keeps |
| `SLACK_MM` / `MIN_ROPE_MM` | 100 / 900 | cord = catch gap + slack; never tightens below the floor (outside the 700 mm push reach) |
| `TUG_MM` / `DRAG_MAX_MM` | 12 / 160 | the pull on a man at the end of the cord; most in one tick |
| `CATCH_HITSTOP` | 3 | freeze on the catch, ticks |

Rules, per tick, in slot order:

1. Press (rising edge of `THROW_NOOSE`), idle, cooldown 0 (or airborne), can act: a loop leaves
   the hand along the facing (`Thrown`).
2. Flight: candidates are the spear's (living, not the thrower, not a teammate, no dodge
   i-frames) minus any man a cord already holds. Each is cast as the body capsule widened by
   `LOOP_MM`, and by `CHASE_MM` more when the man is running from the thrower. The nearest wins.
   A guard facing the thrower turns the loop (`Deflected`, a parry staggers the thrower) through
   the spear's own check. Geometry stops the loop (`Missed`, nothing anchors); range ends it.
3. Catch: no damage, hit-stop 3, `Hit` is emitted, the cord is the gap plus 100 mm, and the man
   keeps his drift at zero.
4. Before the fight step each tick (`SpearState::prepare`, in the merged row so the hash carries
   it): a snared man's `DODGE` bit is stripped and his wish scaled to 40 percent. He may block,
   swing and walk; a guard or a parry does not end a snare (the cord stays on him).
5. Snare tick (after the down, timeout and cut checks of `tether_ends`): the cord tightens to
   `min(cord, gap + slack)`, floored at 900. A man whose gap reaches the cord is hauled toward
   the thrower by what he pulled out plus the tug, at most 160 a tick (`TetherPull` every sixth
   hauling tick). The thrower is never moved: it walks, closes and fights.
6. Ends: `Timeout` at 90 snare ticks, `Down` when either side is out, `Released` by a second
   press, `Cut` when a blade crosses the cord (every fighter but the thrower, the victim too:
   SG13, the one-slash counter), `Toppled` below. The man stops dead, the cooldown restarts.
7. A shield push on a snared man: the boss push (rules v2: `ATTACK` + `BLOCK` with a weapon row)
   reports its landing on `FightState::balance_impulse_q8`, a channel that clears when the next
   fight step starts, so `prepare` reads it first and remembers it through the push's freeze.
   When the freeze ends the man is knocked down (`knockdown` set from the tables, attack, dodge
   and block cleared), the cord comes off (`Toppled`) and `Sim::step_with` appends a real
   `FightEvent::Knockdown { attacker: thrower, victim }` to the tick's events: the log, the hash,
   the balance fall and the window's knockdown path treat him like any other. The same push on a
   free man only staggers him.

Counters: any slash across the cord (SG13), a second press, or close the distance and fight (the
cord is slack inside its length; the thrower is as free as the victim). A shield held up turns the
loop before the catch, not after.

Not here: the Rapier `sim` leg refuses; the spine step does not arm it; a LAN room
(`NetRules::DuelSpear`) has no noose rules yet, so the key is inert there (the sim masks it).
The cord is one rope per thrower: a slot in a snare cannot throw a spear.

What presentation reads: the same `spear_events()` and `spear_view()`; `Flight::noose` and
`Tether::noose` pick the look. `spear_look::noose` draws the cord alone, hand to chest, as long
as the authority's `Tether::rope_mm` (slack while the man is inside it, straight at its end), no
spear mesh and no towed ragdoll. The toppled man is an ordinary `FightEvent::Knockdown`.

Proofs: `crate::spear::noose::tests`:
`the_loop_is_wider_than_a_spear_and_wider_again_on_a_man_running_away`,
`the_loop_flies_six_metres`, `a_catch_snares_and_the_cord_comes_off_after_ninety_ticks`,
`a_snared_man_cannot_dodge`, `a_snared_man_walks_at_two_fifths_of_a_free_mans_speed`,
`the_cord_drags_a_man_back_when_he_pulls_away`,
`the_cord_tightens_as_the_thrower_closes_and_never_lets_out`,
`a_blocking_man_cannot_shrug_the_cord_off`, `a_shield_push_on_a_snared_man_is_a_knockdown`,
`a_man_the_push_topples_is_a_knockdown_event_for_the_log_and_the_balance`,
`a_slash_across_the_cord_cuts_it`, `a_second_press_lets_the_man_go`,
`a_guard_facing_the_thrower_turns_the_loop_away`, `each_button_throws_its_own_weapon`,
`noose_off_ignores_the_button_and_latches_nothing`; `live::noose_tests`:
`noose_duel_600_ticks_digest_identical_twice_and_across_tape_replay`,
`noose_off_ignores_the_button_and_keeps_the_old_digest`,
`noose_matches_have_no_snapshot_and_report_their_view`; `spec::tests`:
`noose_spec_identity_moves_only_when_on`; input `fighter::tests::throw_spear_bit_is_legal`
(the noose bit is legal); game `arena_frame::grapple::tests`:
`ctrl_throws_the_noose_and_only_with_the_grapple_on`; `spear_look::noose::tests`.

Next: SG12 retiarius net (its cord reuses the same cut check and the same
`SpearState` ride; rows in `docs/SPRINT-08-hard-stuff.md`).

