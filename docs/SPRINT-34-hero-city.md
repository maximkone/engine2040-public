# Sprint 34: hero city (a procedural New York to fly, swing and smash through)

Owner ask 2026-10-22: "pcg of newyork ... fast game like spiderman/superman to test physics
hard af ragdolls ... chars ... visually nice like full finished demo". This grows the speed
demo (docs/SPRINT-33-speed-demo.md) into its second world: `arena --speed-demo --nyc`.
The flyer, the car, the giant sword and the cut stay; the desert towers become Manhattan, a
web-swing joins fly and drive, the sidewalks fill with people who go full ragdoll when you hit
them, and the avenues carry traffic you can knock over. Presentation only (rules v1 and the
golden never move); deterministic per seed; potato floor holds (docs: potato target).

Methods, from the 2026-10-22 research (no code copied, ideas only):
- Grid, lots and heights: Parish and Mueller "Procedural modeling of cities" (SIGGRAPH 2001)
  grow roads then cut blocks into lots then massing; Manhattan needs no road growth, its grid
  is the 1811 Commissioners' Plan (avenues north-south, streets east-west), so roads are a rule
  and only lots and heights are generated. Lot slots after CityCrafter3D (Godot, MIT): blocks
  cut into 1x1/2x1/2x2 slots, a big tower takes a merged slot.
- Massing and setbacks: Mueller et al. "Procedural modeling of buildings" (CGA shape grammar,
  SIGGRAPH 2006): a mass splits into podium, tiers and crown by rules. New York's 1916 zoning
  is the rule set: a street wall, then tiers stepping back (the wedding cake), then a tower.
- Facades: Wonka et al. 2003 split grammars; here one tiling bay texture per style with
  repeat addressing (`Presenter::upload_material_set`, VR2), UVs in floors and bays.
- Web-swing: a rope that is a rigid rod while taut and slack otherwise (the Godot forum
  answer; Roblox rope constraint threads), anchor picked ahead and above on a building edge,
  release keeps the velocity and adds a small boost.
- Active ragdolls: PID-driven bones (ashleve/ActiveRagdoll, sergioabreu-g/active-ragdolls);
  the engine already has one: `engine_physics::ragdoll` (Rapier, joint table, hit impulses)
  and the window path `crates/game/src/ragdoll_look.rs` over `engine-physfx`.

Budgets: 256 draws per frame (`MAX_DRAWS_PER_FRAME`), so the city draws as merged chunk
meshes (one per chunk per facade style, each under the 65,535-vertex upload limit); city
under 600k triangles; ragdolls: at most 12 awake at once, the rest asleep or culled.

Files live under `crates/game/src/bin/arena_frame/speed_demo/` (`nyc/` for the city). Each
row is one file with its own tests; a row never edits another row's file or `mod.rs` (the
lead wires modules). Proofs run on GitHub: `node tools/gh-check.mjs --only <your file>
--budget 1500 cargo test -p engine-game --features app --bin arena speed_demo::<module>` and
the same `--only` with `cargo clippy -p engine-game --features app --all-targets -- -D
warnings`. Ratchet (`node tools/rust-quality.mjs --crate game`): no new `as` casts (use
`speed_demo/num.rs`), no `expect`/`unwrap` outside tests, no fn over 100 lines, no copied
helper (vector math lives in `num.rs`), every `allow` has a reason.

- [x] HC1 grid: Manhattan as data. `nyc/grid.rs`: `NycGrid::generate(seed)` over a 1.6 km
  square: avenues run north-south (z) every 260 m, 30 m wide; streets east-west every 80 m,
  18 m wide; one diagonal (Broadway) cuts across and leaves wedge lots; one park rectangle
  (3 x 10 blocks) holds no lots; 5 m sidewalks. Blocks split into lots 12 to 40 m wide along
  the long side, back to back across the block's middle. Height per lot from a density field
  with two peaks (Midtown in the north third, Downtown at the south end): 15 to 40 m between
  them, up to 380 m at a peak, seeded jitter; style per lot (Brick, Stone, Deco, Glass) by
  height and district. Also `roads()` (rects) and `is_road(x, z)`. Accept: tests: same seed
  same lots; no lot touches a road or the park; every lot is inside the square; peaks are
  taller than the valley between them (mean height); lot count 800 to 3000.
- [x] HC2 building: lot to massing. `nyc/building.rs`: `massing(&Lot) -> Building` with a
  podium up to the street wall (25 to 45 m), then tiers that each step in 3 to 6 m per side
  every 30 to 80 m (Stone and Deco: the wedding cake), Glass a straight shaft on the podium,
  Brick a box with a cornice; roofs: a water tank on Brick under 60 m, a spire on Deco over
  200 m. `Building::parts() -> Vec<(Style, MeshData-like positions/normals/uvs/indices)>`
  with UVs in floors (3.6 m) and bays (3 m), and `Building::boxes()` (axis boxes for cuts,
  swing anchors and hits). Accept: tests: every tier sits inside the one below; shells are
  closed and face out; UV scale is one bay by one floor; triangles per building under 400;
  same lot same bytes.
- [x] HC3 facade: the look. `nyc/facade.rs`: per style a 256 x 256 RGBA base tile (one bay by
  one floor: window glass with mullions and sill for Glass/Stone/Deco, brick courses and a
  sash window for Brick), a tangent-space normal tile (recessed windows, brick joints) and an
  ORM tile (glass smooth and a little metallic, brick rough), plus a lit-window mask the
  night look reads (a seeded share of windows lit, warm white). Pure byte generators, no
  GPU. Accept: tests: sizes are 256 x 256 x 4; normals are unit (decoded); the tile repeats
  seamlessly (left column matches right, top row matches bottom); lit share is 20 to 40%.
- [x] HC4 streets: the ground of the city. `nyc/streets.rs`: road meshes (asphalt with lane
  dashes and crosswalks from UVs), sidewalks raised 15 cm with a curb face, the park as grass,
  merged per chunk; street lamps every 30 m as low-poly posts (positions also returned as the
  night's light list). Accept: tests: every road rect is covered; the curb is 15 cm; meshes
  fit the upload limit; lamp spacing holds.
- [x] HC5 swing: the web. `swing.rs`: `Swing::attach(pos, vel, look, anchors)` picks an
  anchor ahead and above within 120 m (a building edge from HC2 boxes), the rope is a rigid
  rod while taut (velocity loses its outward part), slack while shorter; gravity, air drag,
  a pull that reels in 2 m/s while held; release keeps the velocity plus a 15% boost along it
  and a small upward kick. Accept: tests: a swing from rest at 60 deg reaches the other side
  near the same height (energy within 5%); a taut rope never stretches past 1%; release keeps
  speed; no anchor in range attaches nothing; deterministic.
- [x] HC6 crowd: people on the sidewalks who ragdoll. `crowd.rs`: up to 200 walkers on
  sidewalk loops (cheap capsule movers, culled past 150 m); the hero, the car, a swing or a
  falling cut piece that hits one at over 6 m/s wakes a full ragdoll (`engine-physfx` body
  set, the `ragdoll_look.rs` path) with the hit impulse; at most 12 awake, the oldest goes to
  sleep. Lead row (needs the physfx wiring). Accept: a capture where a flyby sends a walker
  tumbling with limbs that bend at the joints, never through the ground.
- [x] HC7 traffic: cars on the avenues. `traffic.rs`: lane followers (one lane per direction
  per avenue and street), stop at the light cycle of each crossing, keep 8 m gaps; a hit over
  6 m/s turns a car into a tumbling rigid box with the hit impulse and spin until it rests.
  Accept: tests: no two cars overlap; cars stop on red; a hit car comes to rest on the road
  within 8 s; deterministic.
- [ ] HC8 look: the city is beautiful. Lead row. Dusk and night presets with lit windows
  (HC3 mask), street lamps (HC4), distance haze scaled to 2 km (fog back on with a city
  density), bloom on; speed lines and FOV kick from SD2's speed feel; a hero body (Rocketbox
  character, fly and swing poses) in the chase cam. Accept: frames at day, dusk and night;
  potato tier (iGPU) holds 30 fps at 720p in the capture stats.
- [x] HC9 window: the mode. Lead row. `--nyc` builds HC1 to HC4 into merged chunks, Tab
  cycles fly, swing, drive; the sword cuts buildings (HC2 boxes through SD4); walkers and cars
  react; the autopilot flies, swings and cuts so a hands-off capture shows all of it. Accept:
  `arena --speed-demo --nyc --frames 3600 --exit --capture-dir target/clips/nyc
  --capture-every 120 --sha <hex>` exits 0 with `view=ok` and the exit line names cuts,
  ragdolls woken, cars hit and the top speed.

Progress 2026-10-22 (lead): HC6 crowd (`speed_demo/crowd.rs`: 14-bone mannequin walkers on
sidewalk loops, a hit over 6 m/s hands the walker's pose to physfx as a hinged ragdoll, nearby
buildings become static walls) and HC9 wiring (`speed_demo/stage.rs` dunes or New York, chunked
merged facade draws with VR2 material sets, `nyc/world.rs` walker loops, lanes, crossings,
`scene.rs` fly/swing/drive with a street-level autopilot, cabs from HC7) run in the real
window: `arena --speed-demo --nyc --frames 3600 --exit ...` printed `world=nyc top_mps=78
cuts=305 swings=172 buildings=1410 groups=42 ragdolls=45 walking=1555 cabs_hit=7 mode=car
autopilot=true`, capture view=ok, frames in target/clips/nyc. `--exit` runs are hands-off
(keys and clicks ignored). The arena HP bars are hidden while the demo owns the frame.
Proofs landed 2026-10-22: `node tools/gh-check.mjs --only <the speed_demo files> cargo test -p engine-game --features app --bin arena speed_demo` 113 passed on the runner (run 39832399544) and the same `cargo clippy -p engine-game --features app --all-targets -- -D warnings` green (run 39832406995); hands-off `arena --speed-demo --nyc --frames 3600 --exit --capture-dir target/clips/nyc3 --capture-every 240` printed `world=nyc top_mps=78 cuts=192 swings=172 buildings=1410 groups=42 ragdolls=42 walking=1558 cabs_hit=5 bumps=4 mode=car autopilot=true`, capture view=ok. HC8 stays open (night glow, Rocketbox hero, dusk preset). Two row fixes on the way: the autopilot's drive leg starts at the far end of the avenue facing the other end (a car cannot turn about at a wall), and a slow car always stops for a red (the stop line sits 14 m out, 2 m clear of the crossing, so a creeping car no longer counts as already inside). Known gaps, not yet rows:
- Night look: the forward pass never samples the emissive slot, so lit windows (HC3 `lit`
  mask) cannot glow; needs a render row (emissive in the forward shader) before HC8 night.
  Stopgap in: `E2040_NIGHT=1 arena --speed-demo --nyc` bakes the moonlit sky and the eight
  street lamps nearest the camera light the facades (`scene.rs` `torch_lights`); asphalt stays
  nearly black and no window glows.
- Lamp posts stand along the streets that cross the park.

Progress 2026-10-22 (lead, richer demo): the hero is drawn (`speed_demo/hero.rs`: a caped box
figure 3x life size, lying along the path with arms ahead while flying, hanging forward with one
arm up toward the web while swinging, three merged meshes), so HC8's body is in as boxes (the
Rocketbox swap stays open). Body and car stop at building walls (`speed_demo/collide.rs`: the body
goes back to the face it came through, loses the speed pointing into it, a flyer keeps half its
speed and the hit throws dust and sparks; the exit line prints `bumps=`). Chase camera sits 5.5 m up.

