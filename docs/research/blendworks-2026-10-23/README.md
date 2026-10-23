# blendworks pre-alpha — a Half-Life built out of Geometry Nodes

Read 2026-10-23 from the free download linked under Malc's video "My 6 Month
Journey to Recreate Half-Life" (2026-08-02). One person, six months, no engine.

- Source: <https://www.youtube.com/watch?v=rwQ8p_J5KhI> (description and pinned
  comment carry the link and the pre-alpha release).
- Artifact: `blendworks_pre-alpha.zip`, 11,303,964 bytes, SHA-256
  `4EE739432FB3F476EDAF01B49C5989822EFC9CB3E38111D47AE507600AF7D113`.
  It holds two files: `blendworks prealpha 5_2.blend` (13,405,759 bytes, read
  here) and `blendworks prealpha.blend` (13,829,259 bytes).
- Read headless with the pinned tool: `"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"`
  (`bpy` 5.2.0 LTS), `-b <file> --factory-startup -P dump.py`. The .blend is
  not vendored; a reader reruns the script at the bottom of this file.
- GPL process, states and ideas only, never linked and never in the tick
  (`docs/FOCUS.md` D5, `docs/plan/content.md` line 157). No code copied.

## What it is

One scene, 75 objects (53 mesh, 11 font, 5 light, 4 empty, 1 camera),
34 materials, 11 collections, 52 Geometry Nodes groups. There is exactly one
text datablock in the file: `Input Handling`, 281 lines of Python, a modal
operator that writes raw key and mouse events into the Geometry Nodes
modifier's interface sockets. Everything else is nodes.

| group | nodes | links |
|---|---|---|
| Player Controller | 149 | 313 |
| Headcrab Controller | 133 | 228 |
| Main Player Controller | 69 | 145 |
| Interface (HUD) | 64 | 85 |
| Impacts | 63 | 102 |
| Elevator System | 51 | 82 |

Collections are the map format: `Collision`, `Collision Invisible`,
`Collision - Ceiling Only`, `Non-Collision`, `Hidden`, and one zone collection
per mechanic (`Zones - Ladder`, `Points - Headcrab`, `Points - Ammo`,
`Points - Health Station`, `Points - Grates`, `Elevator`). A zone is a brush
shape and a point is an entity. That is the HL1 `.bsp` split rebuilt as
collections, and it is why a new mechanic is a node group plus one collection.

The controller returns **one record** of 22 named fields per tick (Firing
Audio, Headcrab Attack, Headcrab Dead, Player Fall Death, Crowbar Hit, ...).
Audio, HUD and death overlay all read that record. Audio itself is absent from
the release: the sequencer handler is inside a `""" AUDIO COMING SOON """`
block.

## The numbers that make it feel like Half-Life

| what | value | where |
|---|---|---|
| scene | 60 fps, EEVEE, 1920x1080 at 200% | scene render |
| walk | +0.05 m per frame (key sets 0.01, group multiplies by 5) | Input Processor |
| run | x3 default | Input Processor, Running Speed 3.0 |
| gravity | -0.1 per frame | Main Player Controller, Apply Gravity |
| jump | +0.1 per frame, held at most 0.15 s | Jump Conditions, Max Jump Time |
| apex test | 9.4 | Compare "Reached Apex" |
| fall | -0.1 rising to -0.3 over 1.9 s | Fall Velocity map range, then "Scale Velocity" |
| fall damage | below -0.15 | Compare "Minimum Velocity For Damage" |
| slope limit | 30 deg default, 50 deg at the call site | Slope Angle Calculator |
| collision | 7 raycasts, fan of +-30 deg in yaw and pitch, 0.1 to 0.5 m | Player Collision System |
| body | eye 1.1 m, crouch -0.2 m | Main Player Controller |
| headcrab | vision 10 m, jumps at 2.7 m, damage at 1.2 m, gravity -0.06, hitbox 0.4x0.3x0.35 | Headcrab Controller |

## Counterevidence: it is frame-rate dependent, and that is the whole point

The stored state is a **per-frame delta**, not a velocity. Walk is 0.05 m per
frame: 3 m/s at 60 fps, 7.2 m/s at 144 fps. Animation divides by the measured
frame rate (labels "Calculate Frame Rate" and "Set to Framerate" appear in
`Animation Logic`, `Animation Single` and `Blicky`), so the hands are corrected
and the feet are not. The game feels right because the demo is played at
60 fps in a viewport that holds 60 fps.

Nothing here can produce our golden hash, a replay, a rollback or a browser
tick. One map, no audio, one enemy, no culling, no LOD, 34 materials. It is a
demo that reads as a game, not a host.

## What we take

**S1 - a feel oracle from a shipped game (P4).** Our movement numbers are
chosen (`AIN6`, `AirTuning::shaped`: 0.30 air control, half gravity near the
apex, 0.12 s coyote). His are tuned by hand against a game every player
already knows, which is why they land. Half-Life's own numbers are public in
the Source SDK `pm_shared.c` (`PM_Accelerate`, `PM_Friction`, and the
`sv_`-side constants) - read the file, do not trust memory.
- Home: `crates/physics` (`crates/physics/src/controller.rs`), rows AIN6 and
  FEEL7.
- Reproduction: one input tape, our controller and the `PM_Accelerate` /
  `PM_Friction` model fed the SDK constants, speed and height printed per tick
  side by side. Adopt a number only when our curve is closer to the oracle
  than the current one.
- Kill: if the curve cannot be reproduced from the published constants without
  the engine's hundreds of other tunables, the oracle is not usable. Record it
  and drop it.

**S2 - edit-to-picture as a number (P3).** He pays zero compile: change a
group, press play, see it move. We pay `gh-check` minutes and no one prints the
number, so no row can move it. The metric is seconds from saving a scene file
to a moving arena frame with the same hash, printed beside build time.
- Home: `tools/`, the OPT rows, whatever receipt prints build time today.
- Kill: if the number is printed for a week and no row improves it, it is
  vanity and goes.

**Already owned, no new row:** named tunables in one table (AIN6), one record
per tick to hash, the browser page as the artifact (`WEB6`, `WEB17`), and the
receipt as the pitch.

## The oracle, pinned from source (read 2026-10-23)

`ValveSoftware/halflife` `pm_shared/pm_shared.c`, fetched from the default
branch (`raw.githubusercontent.com/ValveSoftware/halflife/master/pm_shared/pm_shared.c`).
The file carries a Valve licence header that forbids reuse: this is a spec,
read once, and our code is our own. Line numbers are that fetch.

| what | source | value |
|---|---|---|
| ground accel | `PM_Accelerate` :990 | `accelspeed = accel * frametime * wishspeed * friction`; capped at `wishspeed - dot(vel, wishdir)` |
| friction | `PM_Friction` :1202 | `drop = max(speed, stopspeed) * friction * frametime`; newspeed floors at 0; edge friction multiplies when no ground trace hits |
| air accel | `PM_AirAccelerate` :1279 | `wishspd` capped at **30** for the addspeed test only; `accelspeed` uses the **full** wishspeed - that asymmetry is the strafe gain |
| jump | `PM_Jump` :2474 | `velocity[2] = sqrt(2 * 800 * 45.0)` = **268.328** units/s |
| long jump | `PM_Jump` :2474 | `sqrt(2 * 800 * 56.0)` = **299.33**, forward speed `PLAYER_LONGJUMP_SPEED 350` x 1.6 |
| bunnyhop crop | `PM_PreventMegaBunnyJumping` :2444 | cap `1.7 * maxspeed`; over it `velocity *= (cap / speed) * 0.65` |
| hull | header | `VEC_HULL_MIN/MAX -36/36`, duck `-18/18`, view **28**, duck view **12**, `TIME_TO_DUCK 0.4` |
| falls | header | safe **580**, fatal **1024**, damage `100 / (1024 - 580)` per unit, punch at 350 |
| duck speed | header | `PLAYER_DUCKING_MULTIPLIER 0.333` |
| ladder | header | `MAX_CLIMB_SPEED 200` |
| stop | header | `STOP_EPSILON 0.1` |

Not in the SDK: `maxspeed`, `accelerate`, `airaccelerate`, `friction`,
`stopspeed`, `edgefriction`, `maxvelocity` are engine cvars, and `pmove->maxspeed`
is set by the client. They are **parameters of the fixture**, not constants of
the model, so the fixture carries them in a table and a reader can change them
without touching code. Do not quote their defaults from memory.

The model needs float math. Our sim is integer (`engine-core` is std-only and
the tick is fixed), so the oracle is an **offline f64 reference** that emits a
target curve; the test compares our integer curve to it inside a stated band.
The oracle never enters the tick, the tape or the hash.

## Reproduce the read

```python
import bpy, collections
for g in bpy.data.node_groups:
    print("==", g.name, len(g.nodes), len(g.links))
    for n in g.nodes:
        vals = [(s.name, getattr(s, "default_value", None)) for s in n.inputs]
        print("  ", n.name, n.bl_idname, n.label, vals)
```

Run it with `-b "<file>.blend" --factory-startup -P dump.py`; the first dump
above is that script, the rest add the modifier inputs, the object list, the
scene fps and the `Simulation` zone links.
