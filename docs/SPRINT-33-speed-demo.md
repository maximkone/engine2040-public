# Speed demo: fly, drive, cut (owner 2026-10-22)

Owner: "want game to fly also always faster option, and cut with sword handing in side
giant one to cut building, and also car able to do same to drive on generated terrain and
with time always faster and feel the speed; we can add this as for demo in arena".

Decision: one demo mode in the arena window, `arena --speed-demo`. Presentation only: it
never touches the fight sim, the rules or the golden `d57b5c8ae3096614`. All code lives in
the window binary under `crates/game/src/bin/arena_frame/speed_demo/` (one file per row),
so the pinned `lib.rs` and the fight look never grow. Units: meters, seconds, radians.

The picture: a generated terrain (seeded, hills and valleys) with procedural towers on it.
You fly (WASD + mouse, Space up, Ctrl down) or drive (key `C` toggles), a giant sword held
at your right side; a swing (left click) or simply passing a tower with the blade cuts it
along the blade's plane, and the top half slides off and falls. With "always faster" on
(key `F`, on by default) the speed keeps climbing; the camera widens, shakes and streaks so
you feel it.

- [x] SD1 terrain: the generated ground. `world::terrain::bake_terrain(seed, size)` heights
  become a render mesh (grid, normals, sand/grass/rock by slope) and a `height_at(x, z)`
  query with bilinear interpolation, over a 2 km square. File: `speed_demo/terrain.rs`.
  Accept: tests: same seed same mesh bytes; `height_at` matches the grid at vertices and is
  continuous between them; normals are unit and point up-ish; the mesh fits the 65k-vertex
  upload limit (tiles if not).
- [x] SD2 flight: the flying body and "always faster". Velocity follows the look direction;
  base speed 30 m/s, with always-faster on the speed grows 4% per second up to 900 m/s,
  off holds it; a speed-feel block gives FOV (75 to 110 deg by speed), camera shake and a
  streak rate. Never below the terrain (`height_at` + 2 m). File: `speed_demo/flight.rs`.
  Accept: tests: speed after 10 s on vs off; the cap holds; FOV rises with speed and stays in
  range; the body never goes under ground on a dive.
- [x] SD3 city: towers to cut. A seeded generator places 40 to 120 towers (8 to 120 m tall,
  boxes with floors and a roof cap) on the terrain's flat spots, with a colour per tower.
  File: `speed_demo/city.rs`. Accept: tests: same seed same towers; no two overlap; every
  base sits on the ground (`height_at`); total triangles under the budget.
- [x] SD4 cut: the blade cuts a tower in two. The blade's swept plane splits the tower mesh
  (`engine_destruction::csg::knife`) with a cap on each face, the bottom stays, the top half
  becomes a physfx body that slides along the cut plane and falls, with dust (FEEL5 pool).
  File: `speed_demo/cut.rs`. Accept: tests: a horizontal cut halves a box into two closed
  shells whose volumes sum to the box; a cut that misses changes nothing; the top half moves
  along the plane, not through it.
- [x] SD5 sword: the giant blade at the side. A 12 m blade (the gladius proportions scaled)
  rides the body's right side; left click swings it in a 0.35 s arc; its plane and reach are
  what SD4 cuts with. File: `speed_demo/sword.rs`. Accept: tests: the blade tip traces the
  arc; the plane at mid-swing is the arc's plane; a tower inside reach is cut once per swing.
- [x] SD6 car: drive on the terrain. Four raycast wheels with spring and damper on
  `height_at`, throttle, brake and steering, grip falling with slip; always-faster raises
  the top speed over time like SD2; the same blade rides the car's side and cuts.
  File: `speed_demo/car.rs`. Accept: tests: the car rests level on flat ground; it climbs
  a gentle slope and slides on a steep one; a full lock turns it; top speed after 10 s on vs
  off.
- [x] SD7 window: the mode in the arena. `--speed-demo` builds the scene (SD1, SD3), the
  controls switch fly and drive, the chase camera reads the speed feel, the cut pieces and
  dust draw, a HUD line prints speed. Files: `crates/game/src/bin/arena.rs`,
  `arena_frame/mod.rs`, `speed_demo/mod.rs`. Accept: a capture run
  `arena --speed-demo --frames 900 --exit --capture-dir target/clips/speed --capture-every 60 --sha <hex>` exits 0 with
  `view=ok`, the frames show terrain, towers, a cut tower, and the exit line
  `arena speed demo: top_mps=<n> cuts=<n> mode=fly|car` names what happened.

Landed 2026-10-22: `arena --speed-demo` exits 0 with `view=ok`; the autopilot run
(900 frames) printed `arena speed demo: top_mps=53 cuts=67 swings=9 pieces=48 towers=40
mode=fly autopilot=true`, frames in `target/clips/speed`. Shared vector math and named
conversions live in `speed_demo/num.rs`. Short of the rows as written, left open below:

- [ ] SD8 ground colour: sand/grass/rock by slope (SD1). `MeshData` has no colour channel,
  so the terrain is one sand material; needs a vertex colour or a slope-blend material.
- [ ] SD9 falling halves as physfx bodies (SD4). The cut top is a kinematic slide-then-tumble
  (`cut::Slide`), not a physfx body; it does not hit the ground or other towers. Progress 2026-10-25: Piece 1 landed 99e762856 (a cut top is a physfx stone body that lands and is stopped by towers). Left: scene.rs still draws Slide; Piece.half and SpeedDemo.falls.
- [ ] SD10 demo HUD: the arena's 2D health bars still draw over the demo; hide them and show
  the speed line on screen (SD7 asks for a HUD speed line). Progress 2026-10-24: Landed 8dd1b29bc: speed_demo/hud.rs stages the demo's two rows through push_text and clears on drop; 117 tests run 40127336988. Left: a frame of the HUD from a rebuilt arena.exe.
- [ ] SD11 drive capture: the autopilot only flies; add a drive leg so a capture proves the
  car (SD6 is proven by its tests only). Progress 2026-10-24: Landed 8dd1b29bc: dunes_autopilot drives one 7 s leg from 5 s, line() prints drove_s/drive_legs; 117 tests run 40127336988. Left: the 900-frame capture receipt.

