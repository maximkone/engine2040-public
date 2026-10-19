# Shady Knight port: mechanics and camera

The owner's own game (`Shady Knight`, Unity Mono) is the donor for movement,
camera and grapple feel. The reverse-engineering bench and every graded number
live outside this repo, in `C:\rea-arsenal`
(`docs/findings/shady-knight-mechanics.md`, `docs/plans/shady-knight-port.md`);
this file records only what enters engine2040, in which crate, and what each
landing is allowed to touch.

Boundary: mechanics and tuning numbers only. No game code, mesh, clip, sound or
level data is copied or redistributed.

## Ported so far

| What | Source numbers | Lands in | Hash | Row |
|---|---|---|---|---|
| Walk bob on the fight camera | `CameraBob bobSpeed=1`, `mgtSpeed=2`, `xAmp=0.08`, `yAmp=0.12`; stride assumed 0.75 m | `crates/game/src/fighter_look_cam.rs` (`CamFollow::walk_bob`) | none, presentation only | SG6 |
| Fighter height channel | gravity -40 m/s^2 read at the 60 Hz tick = 11 mm/tick^2 | `crates/arena/src/lib.rs` (`Sim::fall`), `MatchSpec::with_height` | folds `0xB4` only when armed | SG5 |
| Rope swing on the wall tether | take-in 60 mm/tick, damping `v -= v/64`, rope floor 400 mm (tuning, graded I) | `crates/arena/src/spear.rs` (`swing`, `project`) | folds only under `height` | SG4 |

The bob is driven by the hero's own floor speed (slot 0), so a standing hero
holds the eye still and a cut or a respawn never spins it. Phase advances with
travelled distance, so the same walk bobs the same at 30 and 144 fps. The
source's units for `xAmp`/`yAmp` are not in its assemblies: they are read as
metres of eye travel, and only that reading can be wrong.

## How the rope swing landed

The first attempts failed because the wall tether's only force is a reel
pointing at the anchor: on a floor-plane body a taut rope has nothing to turn
into an arc, and any rope strict enough to bite also killed the straight zip
the demo uses. Height is the missing ingredient (SG5), and with it the swing is
a sphere constraint: the hook fixes the 3D radius, the reel takes rope in, and
the body is projected back onto the sphere after it moves. A run along the wall
then slides around the anchor instead of pulling it off the rope, the take-in
shortens the arc, and a release keeps the tangential drift while gravity lands
the body. The `height`-off path is untouched, byte for byte, so the golden
`d57b5c8ae3096614` and the old zip both hold.

Still open: SG7 arms the channel for the window so `play-arena.bat` shows the
swing; the jump button and the movement numbers are a later row.

## Next

- SG7: `--height` on `arena.exe` so the demo plays the swing.
- Movement numbers (`maxSpeed=10.5`, `airControl=1`, `PlayerDash speed=30`) need
  a golden receipt of their own; open a row when SG7 lands.
- Camera sway (`CameraSway count=10`) and kinematic FOV: presentation only.

Unknowns that stay unknown until a scene or prefab read: `jumpForce`,
`maxSlideVel`, `slideBreak`, `focusScale`, `defaultFOV`, and every curve.
