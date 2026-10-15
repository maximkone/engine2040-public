# Rome arena spec (engine2040 side of the shared game, forge T1)

Goal: one game in two engines. Forge has the helmet-POV march
(`game/slice/rome/RomeScene.json`); engine2040 has the arena duel
(`crates/game`, `crates/combat`). This file plus `rome-spec.json`
is the merge point: where the numbers differ, the spec names which
side moves and why. Machine truth is the `.json`; this note is the
reasoning. Pillar: FOCUS Now-2/Now-1 (AI door + look) via the arena
duel proof; EMPIRE outcome "AI makes games".

## Map

- Arena half-extent **10 m**. Forge stays (`RomeScene.json:8`
  `halfExtentCm 1000`). Engine adopts 10 m for Rome bouts: its range
  5..100 m already admits it (`crates/game/src/lib.rs:128`,
  `crates/game/src/bench.rs:131` default 20.0, `:146` range check).
- Duel spawns **(-4,0,0) / (+4,0,0)**, facing each other, 8 m apart.
  Engine keeps its >= 1.5 m separation rule
  (`crates/game/src/window_bind.rs:28`); forge's march spawn
  (0,0,-400) and camp (0,0,800 r 200) stay as the team ingress option.

## Units

- Hero **1.8 m**, eye **1.70 m** (engine reference figure 1.8 m,
  `content/arena/README.md:21`; forge eye 170 cm `RomeScene.json:9-18`,
  inside the HeroGates band 150-185 cm / fov 55-65,
  `HeroGates.cs:64-73`). Neither side moves.
- HP **100** display. Forge stays (`RomeScene.json:9` hp 100). Engine
  internal 1000 permille = 100 x 10 (`crates/combat/src/fight.rs:30`):
  representation only, neither side moves.
- Speed **1.7 m/s** march. Forge stays (`speedCmS 170`). No fixed
  engine locomotion constant surfaced in this run's read budget, so
  the engine adopts 1.7 m/s; the first parity row verifies it.
- Sword **25 HP damage, 0.5 s cooldown, 200 cm range**. Range: forge
  200 cm meets the engine bound HURT_RADIUS 200 cm exactly
  (`crates/combat/src/sweep.rs:29-31`) — neither moves. Damage and
  cooldown land as a **Rome Tables preset beside V1**
  (`crates/combat/src/fight.rs:34-38`, shipped 120 permille / 4+8
  ticks untouched): the engine moves by addition, the core by nothing.

## Bouts and win rules

- **Duel** 1v1 (slot 0 human, slot 1 AI). **Team** 4-16, even sides,
  slot 0 human, rest AI.
- **Round** = 600 ticks; all opponents at 0 HP wins early, else higher
  total HP wins, exact tie replays the round. **Match** = best of 5,
  first to 3 round wins.
- Who moves: the engine adds the round loop (a session `wins` counter
  already exists, `crates/game/src/lib.rs:764`); forge adds duel mode
  (its march win `objective.reached` / lose `hp <= 0`,
  `RomeScene.json:71`, stays as ingress).

## AI fill

Slot 0 human, every other slot AI (`crates/game/src/lib.rs:103-111`
"one human vs `fighters - 1` AI brains"). Engine unchanged; forge
adopts; empty net slots fall back to AI, never idle dummies.

## Tick rate and seed rule

- **60 Hz, 600 ticks** (10 s bouts). Engine canonical
  (`crates/core/src/tick.rs:4`, `crates/core/src/config.rs:54-64`
  seed 2040). Forge moves 20 Hz -> 60 Hz (hash law + shared 16.7 ms
  frame bar); the 600-tick cap is already shared
  (`RomeScene.json:71`, `crates/core/src/config.rs:16`).
- Same map seed -> same map; same seed + same tape -> same bout
  result and hash. Both builds print one result line
  (seed, ticks, winner, hp, hash) for the T2 parity row.

## Camera

Hero-cam preset **eye 170 cm, fov 55** for parity captures. Forge
unchanged (`RomeScene.json:10-18`, `HeroGates.cs:71-73`). The engine
adopts the preset (no fixed duel camera today) beside its open cam.

## Shared assets (base: engine `content/arena` manifest)

CC0-1.0 original kit, generated (`content/arena/README.md:3-6`,
provenance hashes `content/arena/manifest.json`): sand_floor, stands,
walls, gate_tunnel, torch, banner (+crate, debris). Forge contributes
sword (`game/slice/fight/sword.glb`), legionary, galea, scutum —
provenance rows UNKNOWN until forge T1 names each CC0 source.

## Parity hook (for the ship seat splitter)

`ROME.n` piece: a test that loads `rome-spec.json` and fails when an
arena constant drifts (half-extent, tick_hz, max_ticks, damage,
cooldown, range, spawns, seed rule). T2 result parity rides
`e2040_play_run` vs the forge PIE log; forge numbers are never
invented — UNKNOWN stays UNKNOWN with the reason.

## Open parity gaps (digest of docs/archive/2026-10-10/plans/rome-parity.md, 2026-10-10)

The run log is `git show a0e13976:docs/plan/rome-parity.md` (archived under `docs/archive/2026-10-10/plans/`). One line per open gap; rows with no gap are dropped.

- Units (2026-10-05): range exact match (200 cm = 2000 mm bound) and HP (100 x 10) do not move; damage and cooldown differ by design (V1 120 permille / 12 ticks stays; Rome 250 permille / 30 ticks lands as a Tables preset); speed 1.7 m/s adopted.
- Bouts and win rules (2026-10-05): tick base differs (engine 60 Hz, forge 20 Hz; the spec says forge moves to 60); round loop missing on engine; duel mode missing on forge.
- Look (2026-10-05): hue matches exactly; fov differs by design (duel 60, helmet 55 in the parity preset); engine has no head-bob, helmet vignette or flicker constant.
- Seed rule (2026-10-05): engine seed slot fixed (2040); forge has no seed slot; tick base still differs.
- AI fill and tick (2026-10-08): forge has no slot or AI-fill concept and runs 20 Hz; the engine side is complete for ai-fill and tick.

