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

The bob is driven by the hero's own floor speed (slot 0), so a standing hero
holds the eye still and a cut or a respawn never spins it. Phase advances with
travelled distance, so the same walk bobs the same at 30 and 144 fps. The
source's units for `xAmp`/`yAmp` are not in its assemblies: they are read as
metres of eye travel, and only that reading can be wrong.

## Rope swing (row SG4): why it is not landed

The wall tether today is a winch: `WALL_REEL_ACCEL` pushes the thrower straight
at the anchor until `arrived`. Three designs were built and measured against
the existing wall test (`spear_hits_wall_and_zips_the_thrower`, which must keep
passing) and against a moving thrower:

1. Reel plus an outward-radius cap and `v -= v/32` damping.
2. A taut rope: remove the whole radial velocity, keep the tangent.
3. A rope that shortens a little each tick (a reel-driven spiral).

All three fail for one reason: in the horizontal plane the reel is the only
force, and it points at the anchor. A taut rope therefore has nothing to turn
into tangential speed, and the moment the rope is allowed to bite it stops the
old zip that the wall test (and `play-arena.bat`) depends on. A swing needs a
force that is not along the rope, which in the source game is gravity on a body
that can change height.

A match fighter in `crates/arena` is a floor-plane capsule: `pos[1]` is written
only at placement, there is no vertical velocity and no gravity, and
`move_body` takes an `[x, z]` drift. So the prerequisite is a fighter height
channel, opened as row SG5. SG4 then sits on top of it: constrain the anchor
distance, integrate height under `SPEAR_GRAVITY`, and release at the bottom of
the arc. The golden `d57b5c8ae3096614` cannot move in this step (the fixture has
`MatchSpec::spear` off); the vertical channel is the first part that touches
the hashed state, so SG5 carries the golden receipt.

## Next

- SG5: fighter vertical channel (height, gravity, landing) in `crates/arena`.
- SG4: rope swing on top of SG5.
- Movement numbers (`maxSpeed=10.5`, `gravity=-40`, `airControl=1`,
  `PlayerDash speed=30`) need a golden receipt of their own; open a row when
  SG5 lands, not before.
- Camera sway (`CameraSway count=10`) and kinematic FOV: presentation only,
  a later SG6 sibling.

Unknowns that stay unknown until a scene or prefab read: `jumpForce`,
`maxSlideVel`, `slideBreak`, `focusScale`, `defaultFOV`, and every curve.
