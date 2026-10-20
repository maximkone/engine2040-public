# Faction War — the RYL1 soul, rebuilt in this engine

Owner order 2026-10-20: "risk your life 1 not 2 — the second one was bleh".
Owner door the same day: unfreeze one plan file for this and nothing else.

Plain summary: a third-person, aimed-combat, two-faction war game where the
map is the endgame. It keeps what made RYL1 (2005, Youxiland) feel like RYL1
and throws away everything that made it unplayable in 2026. No RYL1 art, code,
names or server files. Clean room.

Lane: owner. Prefix: RY. Base commit: `1a0cc6e0`.
Status: plan only. Rows are not on the board. Tagged LATER in the index.

## 1. Where it stands

| capability | status | evidence |
|---|---|---|
| aimed melee, parry, riposte, clash | implemented, unit-tested | `crates/combat`, v2 exchange; rows GP27-GP31 ticked |
| deterministic tick and replay | implemented, proven | golden `d57b5c8ae3096614`, `docs/HASH-LAW.md` |
| server-authoritative net, rollback, snapshots | integrated for the duel | `crates/net`, `engine-net`; row ON30 open |
| AI seats filling empty slots | implemented for the arena | `crates/game`, `crates/nn`; DP1/RB1 |
| projectiles and thrown weapons in the tick | partial | physics and `physfx`; spear row, no generic bolt |
| class data in a sandbox | partial | `crates/rules` Rhai, hashed; no class sheets |
| territory or capture state | missing | no owner in the tree |
| character persistence | partial | `crates/save`; no account model |
| HUD for capture, cooldown, party | missing | UI parked |

The engine already proves the hard half: a hashed fight, a replay without
models, and two players on one snapshot. What RYL1 needs on top is small and
specific: aimed projectiles, class sheets, capture points, persistence.

## 2. Finish line

Tier A — the slice. Two factions, three classes, 6v6 with four AI seats, one
map, three capture points, ten minutes, hold two to score. Windows and browser,
same hash, replay with no model. One sit-down match a stranger can play.

Tier B — the game. Three maps, gear that changes a build, guild claim on a
point, 12v12, a lobby, friends. A player can lose a night in it.

Tier C — the vision. 32v32 siege on a war map that remembers who holds what
between matches, seasons, and a faction identity strong enough that players
pick a side and stay.

Measurable Tier A exit: 10-minute match under 2 ms authoritative tick on the
reference PC, 60 fps in the browser at 1080p, snapshot under 900 bytes per
player per second, replay hash equal on two runs and on wasm.

## 3. Target design

Two factions, asymmetric, not mirrored. The Wardens: shield, line, slow.
The Ashkin: reach, burst, fragile. Same three roles, different numbers and
different silhouettes.

Three classes at Tier A: Knight (melee, gap closer), Archer (projectile,
range band), Sorcerer (slow bolt, area denial). Class sheets are data in
`crates/rules`, hashed with the match: cooldowns, damage, range bands, cast
time. No class logic in Rust beyond the generic verb.

Combat stays inside the tick. A projectile is a sim entity with position,
velocity and an owner; presentation interpolates, physics predicts, the tick
decides. A hit is an event, not a raycast on the render thread.

Capture is state, not UI. Each point has an owner, a contest count and a
progress integer. The score tick reads points, never players. This keeps the
hash small and the rule testable headless.

Ownership: `crates/combat` owns the exchange, `crates/rules` owns the sheets,
`crates/world` owns the point state, `crates/net` owns replication,
`crates/game` owns the match loop and the seats. No new engine crate for the
game itself; content and rules carry it.

Determinism: integers and fixed math only in the tick, stable iteration order,
no float accumulation in capture progress.

## 4. Hard problems

| problem | technique | why | reference | donor |
|---|---|---|---|---|
| projectile hits agree in replay | fixed-step integration, hit as event | a render-time ray gives two answers on two PCs | `docs/HASH-LAW.md` | own |
| class sheets stay data | Rhai tables, hashed into the match | lets an AI or a designer tune without a rebuild | `crates/rules` | own |
| capture stays readable at 32v32 | one integer per point, score from points | player-count scans blow the snapshot budget | `crates/world` | own |
| AI seats do not feel like bots | seat policy reads sim state, same verbs as a player | an AI that cheats breaks the proof | `crates/nn`, DP1 | own |
| 12v12 snapshot cost | interest management by cell | the duel budget does not scale linearly | `crates/net` | lightyear |
| browser parity | one wasm path, one frame gate | the page must not become a second game | `docs/plan/web-and-realism.md` | own |

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. No RYL1 donor: its
client, server files and assets are out of bounds. Mechanics are re-derived
from public descriptions and from play, written down here as numbers.

## 6. Slim

Will not build: an open world, mounts, crafting, housing, a cash shop, a
second combat system, a custom UI framework, a launcher, an account website.
Will not copy: RYL1 map layout, class names, race names ("Ak'Kan" is theirs),
item names, UI art, sounds, server files, decompiled client code.

Cut from RYL1 on purpose: auto-target, tab-target spam, upgrade-failure RNG,
300-hour grind, walking simulators, pay-to-win.

## 7. Board tasks

Rows join the board only when the owner says start. Until then this section is
the spec, not a queue.

- [ ] RY1 content: one map with three capture points and two spawn halls.
  Size M. Owns `content/faction-war/`. Accept: the map loads headless, the
  three points report their positions and radii, and a screenshot shows the
  ring markers. needs: none.
- [ ] RY2 rules: three class sheets in data (Knight, Archer, Sorcerer) with
  cooldowns, damage, range bands and cast time, hashed into the match.
  Size M. Owns `crates/rules` sheets and `content/faction-war/classes.rhai`.
  Accept: a headless test changes a sheet value and the match hash changes;
  the same sheet twice gives the same hash. needs: none.
- [ ] RY3 combat: a projectile entity that flies in the tick and resolves a
  hit as an event. Size M. Owns `crates/combat` projectile module. Accept:
  a scripted shot at a moving target gives the same hit tick on two runs and
  on wasm; the render thread reads the event, never decides it. needs: RY2.
- [ ] RY4 world: capture point state machine (owner, contest count, progress,
  score tick) with no player scan. Size S. Owns `crates/world` capture module.
  Accept: a headless 10-minute match with scripted occupancy ends with the
  expected score and the same hash twice. needs: RY1.
- [ ] RY5 game: 6v6 match loop with two AI seats per side, faction asymmetry
  and a win condition. Size L. Owns `crates/game` faction mode. Accept: a
  played match ends by score or time; four AI seats act without a model in
  replay; hash equal twice. needs: RY2, RY3, RY4.
- [ ] RY6 net: 6v6 snapshot budget with interest by cell. Size M. Owns
  `crates/net` scale path. Accept: six clients on one server, snapshot under
  900 bytes per player per second, no desync in a 10-minute soak. needs: RY5.
- [ ] RY7 ui: capture bar, class cooldowns, party frames, score and timer.
  Size M. Owns `crates/ui` faction HUD. Accept: a played match shows every
  state the tick knows; no HUD value is computed twice. needs: RY5.
- [ ] RY8 proof: the replay gate. Size S. Owns `tools/` faction check.
  Accept: two runs and the browser build produce one hash; a model-free
  replay plays the same match. needs: RY5, RY6.
- [ ] RY9 save: character record (faction, class, name, gear ids) written and
  read back. Size S. Owns `crates/save` account record. Accept: write, reload,
  same record; a corrupt record is refused, not crashed. needs: RY5.
- [ ] RY10 look: two faction silhouettes readable at 30 m, one weapon set
  each, team colour that survives the potato tier. Size M. Owns
  `content/faction-war/` art. Accept: a capture at 1080p where a stranger
  names the faction and the class; potato check green. needs: RY1.

## 8. Quality gates

Kept green forever once RY8 lands: two-run hash equality, wasm equal to
native, tick under 2 ms at 6v6, snapshot under 900 B per player per second,
potato tier playable, replay with no model. Every class number lives in one
sheet; no number twice.

## 9. Session estimate

Tier A: RY1-RY8, about 6-9 sessions with one agent, less with two in
parallel (RY1/RY10 art beside RY2/RY3 code). Tier B: 20-30 sessions. Tier C:
not estimated until Tier A is played by a stranger.
