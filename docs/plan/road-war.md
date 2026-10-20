# Road War — the Auto Assault soul, rebuilt in this engine

Owner order 2026-10-20: "auto assault i want". Same door as `ryl1.md`: the
owner's own game, clean room, no Auto Assault code, art, names or maps.

Plain summary: cars with guns fighting for road. A highway map cut into nodes;
hold nodes to hold the road; the road wins the match. Third person, fast,
loud, browser playable.

Lane: owner. Prefix: RW. Base commit: `1a0cc6e0`.
Status: plan only. Rows are not on the board. Tagged LATER in the index.

## 1. Where it stands

| capability | status | evidence |
|---|---|---|
| hashed tick, replay, golden | proven | `docs/HASH-LAW.md`, `d57b5c8ae3096614` |
| aimed combat, damage events | implemented | `crates/combat` |
| capture point state | planned, not built | `ryl1.md` RY4, shared |
| vehicles, wheels, steering | missing | nothing in the tree |
| mounted weapons, turret aim | missing | `crates/combat` has no mount |
| large maps, streaming | partial | `crates/world`, cells |
| 6v6 net with AI seats | partial | duel proven, scale open |

The car is the whole bet. Everything else exists or is one shared row away.

## 2. Finish line

Tier A — one car, one gun, one road, one node. Two players, four AI drivers,
five minutes, Windows and browser, same hash, replay with no model.

Tier B — 4v4 on a three-node highway, chassis classes (fast, heavy, support),
a garage that changes a build, a season scoreboard.

Tier C — a road war map that remembers who holds what between matches, convoys
that move between nodes, weather that changes grip.

Measurable Tier A exit: car simulation under 0.5 ms per vehicle per tick,
6 vehicles under 2 ms total, 60 fps browser, snapshot under 1 KB per player.

## 3. Target design

Vehicle: one rigid body, four raycast wheels, fixed-step integration, integer
or fixed math in the tick. Presentation interpolates; physics predicts; the
tick decides. No float accumulation in position.

Weapons: mounted on the chassis, aim by turret yaw, fire a projectile that is
a sim entity (shared with `ryl1.md` RY3). Hit is an event.

Territory: the same capture state as the foot game. A node has owner, contest
count and progress. Hold nodes, score ticks, match ends. One system, two
skins: this is the point of building it in an engine, not a game.

Map: a highway loop, three nodes, two spawns. Cells for interest management.

Determinism: same seed, same inputs, same hash, on Windows and wasm.

## 4. Hard problems

| problem | technique | why | donor |
|---|---|---|---|
| car feels alive but stays deterministic | fixed-step, raycast suspension, integer state | replay must agree on every PC | box3d-rust (oracle) |
| turret aim readable at speed | aim assist on the presentation layer only | the tick must not cheat | own |
| 6 cars do not blow the snapshot | cell interest, delta by node | the duel budget does not scale | lightyear |
| AI drivers do not cheat | same inputs as a player, seat policy reads sim | a cheating AI breaks the proof | `crates/nn` |
| browser parity | one wasm path, one frame gate | no second game | `web-and-realism.md` |

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. No Auto Assault donor:
its client and assets are out of bounds. Numbers are re-derived and written
here, from measured play and public description only.

## 6. Slim

Will not build: open world, traffic, on-foot combat at launch, car crafting,
a garage economy, a launcher. Will not copy: Auto Assault chassis names, map
layout, UI, sounds, vehicle models, server files.

## 7. Board tasks

Rows join the board only when the owner says start.

- [ ] RW1 vehicle: one chassis, four raycast wheels, steering and throttle in
  the tick. Size L. Owns `crates/physfx` vehicle module. Accept: a scripted
  drive of 600 ticks gives the same position on two runs and on wasm; under
  0.5 ms per vehicle. needs: none.
- [ ] RW2 mount: a turret that aims and fires a projectile sim entity. Size M.
  Owns `crates/combat` mount module. Accept: a shot at a moving target hits on
  the same tick twice; the render thread reads the event. needs: RW1.
- [ ] RW3 map: a highway loop with three nodes and two spawns. Size M. Owns
  `content/road-war/`. Accept: loads headless, nodes report position and
  radius, a screenshot shows the loop. needs: none.
- [ ] RW4 capture: the shared point state from `ryl1.md` RY4, wired to the
  road map. Size S. Owns `crates/world` capture module. Accept: scripted
  occupancy ends with the expected score, same hash twice. needs: RW3, RY4.
- [ ] RW5 match: 4v4 with four AI drivers, chassis asymmetry, win by nodes.
  Size L. Owns `crates/game` road mode. Accept: a played match ends by score;
  AI drives without a model in replay; hash equal twice. needs: RW2, RW4.
- [ ] RW6 net: 6 vehicles on one server, interest by cell. Size M. Owns
  `crates/net` scale path. Accept: snapshot under 1 KB per player per second,
  no desync in a 10-minute soak. needs: RW5.
- [ ] RW7 browser: the same match in a tab at 60 fps. Size M. Owns
  `crates/web` road entry. Accept: browser frame beside native, hash equal.
  needs: RW5, RW6.
- [ ] RW8 look: one chassis per class readable at 50 m, team colour, dust and
  impact. Size M. Owns `content/road-war/` art. Accept: a capture where a
  stranger names the class; potato check green. needs: RW3.

## 8. Quality gates

Two-run hash equality, wasm equal to native, 0.5 ms per vehicle, snapshot
under 1 KB per player per second, potato tier playable, replay with no model.

## 9. Session estimate

Tier A: RW1-RW7, about 7-10 sessions; RW1 alone is 3 of them. Tier B: 20+.
Tier C: not estimated until a stranger plays Tier A.
