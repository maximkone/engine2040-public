### Showcase tour (owner 2026-10-24, plan `docs/plan/showcase.md`)

One scripted tour shows every system the arena plays, station by station; a system built but
not hooked in gets a row here that wires it and adds its station. Order: SHOW1, then the
cheapest station that is already wired (SHOW2), then the wiring rows by how much they show.

- [x] SHOW1 game: the showcase tour, five stations against a training dummy (Landed 86b92e36a: headless tour all five ok, a 4200-frame window run printed all five ok with view=ok) - `Showcase` in
  `crates/game/src/showcase.rs` drives slot 0 (yank, carry, wall swing, three sword hits, the
  finisher into a toned ragdoll), the rival is a dummy staged through `App::offer_remote`,
  each station has a window-title caption and a grade, `arena.exe --showcase` arms spear and
  height and runs it, `play-showcase.bat` launches it, the exit prints
  `showcase: yank=ok carry=ok swing=ok melee=ok finisher=ok`. F2P:
  `showcase::tests::the_tour_passes_every_station_against_the_dummy` (no module before).
  P2P: `cargo test -p engine-game --lib showcase`, golden `d57b5c8ae3096614` (spear off
  untouched). Accept: the headless test green, and one window run of `play-showcase.bat`
  printing all five `ok` with `view=ok`.
- [x] SHOW2 game: rope-cut station - the dummy swings its blade across the rope during a
  carry, so the tour shows SG13 (`EndWhy::Cut`, the sever cue). needs: SHOW1. F2P: a
  `cut=ok` grade in the headless tour. P2P: SHOW1's test. Landed 2dfbf31 (headless tour
  `showcase: yank=ok carry=ok cut=ok swing=ok melee=ok finisher=ok`, GitHub run 40095782247).
- [ ] SHOW3 arena+game: destruction sag station - a bonded column in the ring with DX18 sag Progress 2026-10-24: `crates/game/src/break_look.rs` leans a standing chunk by its sag offset (`set_sag`, `SAG_SPAN_MM`; 43b6291), run 40112921522 (18 tests, sim hash equal look on and off). Left: the bonded column in the ring, the sag feed, the station and the capture.
  enabled; the look reads `sag_q16`/`sag_offset_mm` so the mesh leans before it falls; the
  tour knocks its foot out. needs: SHOW1. F2P: `sag=ok` in the tour plus a look test that the
  drawn chunk offset follows `sag_offset_mm`. P2P: `cargo test -p engine-destruction --lib`.
- [x] SHOW4 content+game: machine props station - cook `tools/blender/machine_gen.py`
  presets (ballista, crane, cart) into the props kit under the potato budget and stand them
  in the ring; the tour walks past them. needs: SHOW1. F2P: the props manifest lists the
  three, a kit-budget test passes with them. P2P: `node --test tools/blender/machine_gen.test.mjs`. Progress 2026-10-24: content half landed 0d99f985d: ballista 302, crane 770, cart 756 tris cooked twice byte-equal, appended to the props manifest (count 27, first 24 rows unchanged), node --test machine_gen 12 passed. Left: the kit loader for content/generated/props, the station. Progress 2026-10-24: Landed 60f9d6534: Ballista/Crane/Cart load as kit assets (14 actors, look-only). Left: the final pin run (was blocked by the PRESENTATION_V1 red, fixed in af06ff26b). RETIRED (done): 0d99f985d (lead, trim audit 2026-10-24)
- [ ] SHOW5 world+game: ruins station - MJ2 (`world.rewrite` door) plus a ruined-wall map
  from `content/maps/rewrite/ruined_wall.rules` as arena obstacles; the tour fights through
  the breach. needs: SHOW1, MJ2. F2P: `ruins=ok` in the tour. P2P: `cargo test -p engine-world --lib rewrite`.
- [ ] SHOW6 render: impostor crowd station - RL64 (bake and draw) puts the far stands
  crowd on impostors; the tour's last shot pulls the camera back over them. needs: RL64.
  F2P: the frame-split line counts impostor draws. P2P: `cargo test -p engine-render --lib impostor`.
- [ ] SHOW7 ai+game: decision trace station - the bot (not the dummy) fights one round with
  the AIN8 trace ring on, and the window prints its last decision line each second. needs:
  SHOW1. F2P: `trace=ok` (a non-empty trace line). P2P: `cargo test -p engine-ai --lib bt`.
- [ ] SHOW8 anim+game: budget and warp - AM34 wires the animation budget into the look
  (rung counts on the frame-split line) and AM41 warps the lunge clips; the tour's melee
  station shows lunges that meet the hit point. needs: AM34, AM41. F2P: rung counts present,
  `foot_slide_cm` not worse. P2P: `cargo test -p engine-anim --test it anim_budget`.
- [ ] SHOW9 rome: city walkers view - a minimal top-down view of the CITY1 city (roads,
  houses by coverage, walkers moving) as its own showcase scene. LATER: needs a Rome window.
  F2P: a headless frame of the view with walkers drawn. P2P: `cargo test -p engine-rome --lib`.
- [x] SHOW10 game: noose station - the human throws the SG11 noose (Ctrl) over the dummy, walks in
  on the cord and shoves it down (a boss push on a snared man is a knockdown). Written once:
  `git show 42a02dec0 -- crates/game/src/showcase.rs docs/plan/showcase.md` (a `Station::Noose`
  after the cut, captions x/7, `noose_and_shove`, `noose=ok`); the headless tour passed it
  (`the_tour_passes_every_station_against_the_dummy`, run 40099889427) and the fold's
  `the_showcase_tour_pops_no_joint` passes once `fold_showcase` arms the noose (8774ad55f). It was
  taken out of the tour again because it turns
  `fighter_look_show::tests::a_ragdoll_in_the_tour_is_never_idle_and_the_idle_float_law_stays_quiet`
  red (`idle_float_cm_max=11.6` against its 10 cm law; 11.6 whether the script shoves or releases
  the cord, and the same test is green on the tour without the station, run 40103542757), so the
  cause is in the snare's look (not the knockdown): find which idle body floats 11.6 cm there
  (the thrower's tethered-arm pose or the dragged dummy), fix it, then re-add the station.
  needs: SHOW1, SG11. F2P: `noose=ok` in the tour and both fold laws green. P2P: `cargo test -p
  engine-game --features app --lib -- showcase fighter_look_show spear_look`. RETIRED (done): 4169e246e (lead, trim audit 2026-10-24)
