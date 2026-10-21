### Rome: one legionary's war (owner 2026-10-21)

Boarded 2026-10-21 from the owner's ask: "roman arena to focus to roman soldier simulator with all
engineering", "plan it deeper and write and combine.. realistic af with making tools ... real rome
docs". Plans: `docs/plan/rome.md` (the game: ROM, LEG, MK) and `docs/plan/rome-codex.md` (the
sources: CDX). Look bar: `docs/design/legionary/`. Rules v1 and the golden `d57b5c8ae3096614`
never move for these rows; the simulator runs on rules v2 and later. Chapter rows carry `LATER:`
until the slice (LEG10) lands.

- [x] ROM1 docs: the old Rome docs agree with their data. Fix the 15 contradictions the 2026-10-21 survey listed (ladder rungs and ranks, loss penalty, sword damage and reach, dodge, tier names and HP, reaction delay, hit stop, zone radius, entrance bar, camera recording, rope damping and spear gravity, the first-fight bar, stale MISSING lines in `gameplay.md`, wrong line cites) by one rule: the `content/design/*.rows` file wins and the prose changes; where no data file exists, the newest owner decision wins. Each fix cites the data line it now matches. Tier A. Size S. Owns `docs/design/`, `docs/plan/gameplay.md`, `docs/plan/rome-spec.md`, `docs/plan/spear-grapple.md`. Landed 51d424e7. The 15-item table (before, after, winning data line) is at the end of `docs/plan/rome.md` section 8. `node --test tools/beat-lint.test.mjs` 12 passed 0 failed; `node tools/workspace-check.mjs` PASS; `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` 1 passed 0 failed on the runner (gh run 39561611917), golden d57b5c8ae3096614 x2; no data file changed.
  Accept: a table at the end of `docs/plan/rome.md` section 8 lists the 15 items with before, after and the winning data line; `node tools/workspace-check.mjs` PASS; `node --test tools/beat-lint.test.mjs` passes.
  F2P: the 2026-10-21 survey finds 15 disagreements.
  P2P: no data file changes; golden d57b5c8ae3096614 x2.
  needs: none.
- [x] ROM2 rome: the Roman game crate. New std-only `crates/rome` (package `engine-rome`, deps `engine-core` only) that loads `content/rome/*.toml` tables into typed integer structs (lengths in mm, times in ticks) and refuses a table entry with a number and no `ref`. First tables: `content/rome/camp.toml` (ditch and rampart by threat, street widths) and `content/rome/line.toml` (frontage, rank gap) with the values and refs from `rome.md` section 5. Document the new crate in `docs/CRATE-MAP.md` and its edge in `docs/ARCHITECTURE.md`. Tier A. Size S. Owns `crates/rome/`, `content/rome/camp.toml`, `content/rome/line.toml`, `docs/CRATE-MAP.md`, `docs/ARCHITECTURE.md`. Landed 294a0f63. `cargo test -p engine-rome --lib` 9 passed 0 failed (gh run 39543893672); one-night ditch 1480 mm x 888 mm, light 2664 x 2072, serious 3552 x 2664, rampart 2368 x 1776, streets 17760 / 11840, line 888 / 1776; entry with numbers and no `ref` refused as `TableError::MissingRef { line: 5, entry: "ditch.light" }` whose Display names `ditch.light`; two loads of each table hash the same and camp != line; clippy -D warnings pass (gh run 39544248144); golden d57b5c8ae3096614 x2 (gh run 39544248095); `workspace-check.mjs` FAILs only on the pre-existing genre ledger rows outside these owned paths (`ai/src/plan_trace.rs`, `physics/src/cloth/collide.rs`, `physics/src/cloth/tests.rs`, from a2531f3b).
  Accept: `cargo test -p engine-rome --lib` passes: the two tables load, a one-night ditch reads 1480 mm wide and 888 mm deep (5 x 3 ft at 296 mm), an entry without `ref` is refused with the key named, and two loads hash the same; `node tools/workspace-check.mjs` PASS.
  F2P: no Rome crate and no Rome tables.
  P2P: golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] ROM3 game: the career replaces the gladiator ranks. The ladder's rank names become Tiro, Miles, Immunis, Optio, Centurio at the same glory thresholds (0, 300, 900, 1800, 3600) in `content/design/loop.rows` and `crates/game/src/ladder.rs`; glory reads as the service record in the HUD. Tier A. Size S. Owns `crates/game/src/ladder.rs`, `content/design/loop.rows`, `content/design/ladder.rows`.
  Accept: `cargo test -p engine-game --lib ladder` passes with the new names at the same thresholds; `node --test tools/beat-lint.test.mjs` passes.
  F2P: the ranks read Gladiator, Champion, Legend.
  P2P: golden d57b5c8ae3096614 x2 (names are not hashed; a test pins it).
  needs: ROM1.
- [ ] ROM4 game: armatura at the post. Rung 0 of the ladder becomes drill at the palus: a fixed post with three marked targets (head, body, legs) and a wooden sword at double weight (Vegetius 1.11), scored by clean thrusts in 30 s; the duel rungs follow. Tier B. Size S. Owns `crates/game/src/ladder.rs`, `crates/arena/src/kit.rs`.
  Accept: `cargo test -p engine-game --lib palus` passes: a scripted 30 s of thrusts scores the same twice, a cut scores less than a thrust, and the rung advances at the bar.
  F2P: rung 0 is a duel.
  P2P: golden x2.
  needs: ROM3.
- [ ] ROM5 endpoint: Rome door verbs. `rome.camp { site, threat }` lays out a marching camp from `camp.toml` on the dig grid and returns its works list; `rome.road { from, to }` lays a road line on the height grid; `rome.scene { card }` stages a set piece from a codex scene card. Each returns a capture and a receipt and is one undoable change. Tier B. Size M. Owns `crates/endpoint/src/packs/rome.rs`, `crates/endpoint/src/packs/mod.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib packs::rome` passes: `rome.camp` on a flat 200 m site with threat `light` returns four ditch runs whose sizes match `camp.toml`, undo restores the grid byte-equal, and a bad threat is a typed refusal.
  F2P: no Rome verbs.
  P2P: golden x2.
  needs: ROM2, LEG7.
- [ ] LEG1 game: legionary mode. `arena.exe --legionary` starts slot 0 in first person (FV3 feed) with the galea, gladius and scutum kit on rules v2; third person stays one key away for the replay. Tier A. Size S. Owns `crates/game/src/bin/arena.rs`, `crates/game/src/fps_view.rs`.
  Accept: a 600-tick `--legionary --ticks 600` run prints `legionary: eye=fps kit=galea,gladius,scutum` and a capture from the eye is opened; replay twice equal.
  F2P: no first-person play mode.
  P2P: golden d57b5c8ae3096614 x2 (v1 untouched).
  needs: FV3.
- [ ] LEG2 render: the helmet in view. A lit 3D rim mesh built from an aperture outline table (presets: galea with cheek guards, face mask with two eye holes and a nose bar), attached to the eye, drawn in its own depth range so the world never clips it, taking the scene light and dirt and blood decals. Look bar: `docs/design/legionary/*.jpg`. Tier A. Size M. Owns `crates/game/src/fps_view.rs`, `crates/render/src/viewmodel.rs`.
  Accept: `cargo test -p engine-render --lib viewmodel` passes (the outline table builds a closed mesh with finite normals; the depth range never lets a wall 5 cm in front of the eye cover the rim); a capture beside each reference picture is opened.
  F2P: no helmet in view.
  P2P: golden x2; `validation_errors=0`.
  needs: POV1.
- [ ] LEG3 ai: the contubernium falls in. `crates/ai/src/legion.rs` gets its first caller: in rules v2, 7 AI comrades take formation slots beside slot 0 (nearest free slot, keep row and file, the slowest sets the pace; frontage 3 ft, rank gap 6 ft from `line.toml` once ROM2 lands, constants until then) and obey `fall in`, `hold` and `advance` from `crates/ai/src/orders.rs`. Donors: 0 A.D. Formation.js and Total War group formations (ideas only), read 2026-10-21. Tier A. Size M. Owns `crates/arena/src/live.rs`, `crates/arena/src/legion_bind.rs`.
  Accept: `cargo test -p engine-arena --lib legion_bind` passes: after `fall in`, 8 bodies hold their slots within 15 cm while the line advances 50 m; `hold` stops them within 1 s; two runs hash the same; `node tools/unwired-code-gate.mjs` no longer lists `legion`.
  F2P: `legion.rs` has 0 callers outside `crates/ai`.
  P2P: golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] LEG4 combat: the shield line fights. A boss push becomes a v2 action (a short shove with the scutum that feeds the target's balance like TC3), the thrust under the shield rim gains reach and damage over the cut (Vegetius 1.12), and two lines in contact press through TC5. Tier A. Size M. Owns `crates/combat/src/fight.rs`, `content/design/weapons.rows`.
  Accept: `cargo test -p engine-combat --lib shield_line` passes: a boss push staggers a guarded foe, a thrust beats a cut on damage at the same reach, the pilum row in `weapons.rows` is a throw (no two-hand thrust), replay twice equal.
  F2P: no shield push; the cut and thrust are equal.
  P2P: golden d57b5c8ae3096614 x2 (v1 tables unchanged).
  needs: TC3.
- [x] LEG5 arena: pila. Each legionary carries two pila; on the horn they are thrown (built on `spear.rs`); effective 15-25 m, max about 30 m; a pilum that sticks in a shield weighs it down (guard slower) until the bearer drops the shield or pulls the pilum. Tier A. Size M. Owns `crates/arena/src/spear.rs`, `crates/arena/src/pilum.rs`. Landed 05275e78. `cargo test -p engine-arena --lib pilum` 7 passed 0 failed (gh run 39569688432): a volley of 8 pila at 20 m sticks in the 8 scuta of a line of 8, a stuck scutum holds the guard 24 ticks late, a throw at a target 35 m away falls short at the 30 m reach, the same match hashes the same twice, the pilum pulled out and the scutum dropped free the guard again, the channel off by default throws nothing. `cargo test -p engine-arena --lib` 262 passed 0 failed (gh run 39570054818); `cargo test -p engine-arena --lib spear` 16 passed 0 failed (gh run 39570898271); `cargo clippy -p engine-arena --all-targets --no-deps -- -D warnings` pass (gh run 39570522806); `cargo test -p engine-proof --features arena,spine --lib arena_` 9 passed, the arena digest stays 2fbd56040fd091a7 (gh run 39571287261); golden d57b5c8ae3096614 unchanged x2 (gh runs 39571758242 and 39572114339). The volley lives in `crates/arena/src/pilum.rs` and reuses the flight, the swept cast and the geometry march of `crates/arena/src/spear.rs`; the horn is the arena's press-tick call cue and the calling side looses. The channel is off by default, so the sim hash, the arena digest and the golden do not move. The arena Sim grew the field, so `crates/arena/src/fork.rs` carries the matching line.
  Accept: `cargo test -p engine-arena --lib pilum` passes: a volley of 8 at 20 m lands on a line of shields, stuck shields guard slower, a throw past 30 m falls short, replay twice equal.
  F2P: no pilum, only the tether spear.
  P2P: golden x2; the spear tests stay green.
  needs: none.
- [x] LEG6 world: the dig layer. `crates/world/src/dig.rs`: an integer height grid in mm (cell 0.5 m) owned by the tick; `dig` and `pile` move earth and conserve its volume; walkers read height and slope with the slope limit and ground snap ported from `crates/physics/src/kcc.rs` to integer, after which `kcc.rs` is deleted; nav reads blocked cells. Tier A. Size M. Owns `crates/world/src/dig.rs`, `crates/world/src/lib.rs`, `crates/physics/src/kcc.rs`, `crates/physics/src/lib.rs`. Landed 87463faa. `cargo test -p engine-world --lib dig` 8 passed 0 failed (gh run 39562315663); a 10 m ditch 1480 mm wide and 888 mm deep moves 53 280 cell-mm (13 320 000 000 mm3) into the rampart beside it and the ground total is unchanged; the walker cannot climb the ditch side (1776 per mille over the 1000 limit) nor climb out; the grid hashes the same twice; nav reads the blocked cells with a one-cell margin; `kcc.rs` is gone and `cargo check -p engine-physics` is green (gh run 39562484902); clippy -D warnings pass (gh run 39562686678); golden d57b5c8ae3096614 x2 (gh run 39562892763); `workspace-check.mjs` PASS; `rust-quality.mjs --owned` PASS.
  Accept: `cargo test -p engine-world --lib dig` passes: a 10 m ditch 1480 mm wide and 888 mm deep moves the same volume into a rampart beside it, a walker cannot climb the ditch side above the slope limit, the grid hashes the same twice; `kcc.rs` is gone and `cargo check -p engine-physics` is green.
  F2P: no dig layer; `kcc.rs` has 0 callers.
  P2P: golden d57b5c8ae3096614 x2.
  needs: none.
- [ ] LEG7 world: works. `crates/world/src/works.rs`: a build job is data (footprint, material, crew, progress per stroke, snap points); unfinished work is weak; more hands go faster; deconstruct refunds. A camp plan from `crates/rome` becomes ditch, rampart, stake line and gate jobs; the camp is done when the perimeter closes. Donor: Holdfast sapper builds (wiki, ideas), PlanBuild ghost and snap (WTFPL). Tier A. Size M. Owns `crates/world/src/works.rs`, `crates/rome/src/camp.rs`.
  Accept: `cargo test -p engine-world --lib works` and `cargo test -p engine-rome --lib camp` pass: 8 diggers close a 50 x 50 m one-night camp perimeter in fewer ticks than 4 diggers, an unfinished rampart blocks less than a finished one, replay twice equal.
  F2P: no build jobs.
  P2P: golden x2.
  needs: LEG6, ROM2, TC2.
- [ ] LEG8 game: the march. Column order along a road line, pace from the regular and full step, stamina from the pack load (60 lb), a time skip to the next set piece, and the objective line from the mode's rules file ("March to the camp"). Tier B. Size M. Owns `crates/game/src/march.rs`, `content/rome/march.toml`.
  Accept: `cargo test -p engine-game --lib march` passes: the column holds order over 500 m of road, full step drains stamina faster than regular step, the skip lands at the set piece with the same state twice.
  F2P: no march.
  P2P: golden x2.
  needs: LEG3, ROM2.
- [ ] LEG9 ui: the HUD of the pictures. HP and stamina bars bottom left, the weapon icon bottom right, one objective line top right, nothing else in legionary mode. Tier B. Size S. Owns `crates/ui/src/lib.rs`.
  Accept: `cargo test -p engine-ui --lib legionary_hud` passes (the three elements in their corners at 1080p and 720p, no other element drawn); a capture beside `helmet-pov-battle.jpg` is opened.
  F2P: the arena HUD in every mode.
  P2P: the arena HUD tests stay green.
  needs: LEG1.
- [ ] LEG10 proof: the slice. A scripted 3-minute run: march 60 s, dig the camp 60 s (time-scaled), hold the line 60 s against a warband; replays twice equal, writes `target/reports/legionary-slice-<stamp>.md` with digests, four captures beside the two reference pictures and the machine look score. Tier A. Size S. Owns `crates/game/tests/it/legionary_slice.rs`, `crates/game/tests/it/main.rs`.
  Accept: `cargo test -p engine-game --features app --test it legionary_slice` passes twice with equal digests; the report lists the captures.
  F2P: no slice run.
  P2P: golden x2.
  needs: LEG1, LEG2, LEG3, LEG4, LEG5, LEG6, LEG7, LEG8, LEG9.
- [ ] LEG11 arena: line relief on the horn. LATER: after LEG10. The front rank's stamina sets the call; on the horn the ranks pass through each other and the fresh rank takes the front. Tier B. Size M. Owns `crates/arena/src/legion_bind.rs`.
  Accept: `cargo test -p engine-arena --lib relief` passes: the swap completes inside 3 s without a gap wider than one man, replay twice equal.
  F2P: no relief.
  P2P: golden x2.
  needs: LEG10.
- [ ] LEG12 ai: testudo and wedge. LATER: after LEG10. Two formations with their own contact rules (testudo: shields up, slow, missiles blocked; wedge: point at the front). Tier B. Size M. Owns `crates/ai/src/legion.rs`, `crates/arena/src/legion_bind.rs`.
  Accept: `cargo test -p engine-arena --lib testudo` passes: arrows on a testudo hit shields not bodies, the wedge splits a line in a fixture, replay twice equal.
  F2P: no testudo.
  P2P: golden x2.
  needs: LEG10.
- [ ] LEG13 game: the night watch. LATER: after LEG10. Four watches, a daily password carried by the tesserarius, a sentry check on the rampart; sleeping on watch is punished. Tier B. Size S. Owns `crates/rome/src/watch.rs`.
  Accept: `cargo test -p engine-rome --lib watch` passes: four watches split the night, a wrong password raises the alarm, replay twice equal.
  F2P: no watch.
  P2P: golden x2.
  needs: LEG10.
- [ ] LEG14 world: the bridge chapter. LATER: after LEG10 and MK6. Piles, beams and deck as works over a river cell, with a pile driver and the crane; piles in pairs 2 ft apart as in Caesar BG 4.17. Tier B. Size L. Owns `crates/world/src/works.rs`, `crates/rome/src/bridge.rs`.
  Accept: `cargo test -p engine-rome --lib bridge` passes: a 40 m span closes with the right pile count, a walker crosses, replay twice equal.
  F2P: no bridge works.
  P2P: golden x2.
  needs: LEG10, MK6.
- [ ] LEG15 game: siege works and engines. LATER: after LEG10. A ring of ditch, rampart and towers, traps (cippi, lilia, stimuli from Caesar BG 7.73), and engine crews for scorpio and ballista on the siege ledger in `crates/game/src/siege.rs`, sized by Vitruvius 10.10-11. Tier B. Size L. Owns `crates/rome/src/siege.rs`, `crates/game/src/siege.rs`.
  Accept: `cargo test -p engine-rome --lib siege` passes: a ballista's spring hole follows the Vitruvius table for its shot weight, a lilium pit slows a charge, replay twice equal.
  F2P: no siege works.
  P2P: golden x2.
  needs: LEG10.
- [ ] LEG16 world: roads. LATER: after LEG10. A road as layered works on the march route (bed, rubble, core, paving) with the camber unknown until the codex finds a source. Tier B. Size M. Owns `crates/rome/src/road.rs`.
  Accept: `cargo test -p engine-rome --lib road` passes: a 100 m road needs its layers in order, a finished road lifts march pace, replay twice equal.
  F2P: no road works.
  P2P: golden x2.
  needs: LEG10.
- [ ] MK1 world: timed processes and the bloomery. `crates/world/src/process.rs` (genre-neutral: inputs, tool, ticks, outputs, a failure rule) runs the first loop: smelt in a shaft furnace (time-skipped, 10-20 h of game time), break out the bloom, reheat and forge a bar in 6-10 anvil beats. Recipe in `content/rome/tech/iron.toml` with refs. Tier A. Size M. Owns `crates/world/src/process.rs`, `crates/world/src/lib.rs`, `content/rome/tech/iron.toml`.
  Accept: `cargo test -p engine-world --lib process` passes: the iron recipe turns ore and charcoal into a bloom and a bar, a missing input refuses with its name, the same beats give the same bar twice.
  F2P: no process engine.
  P2P: golden x2.
  needs: ROM2.
- [ ] MK2 rome: wear and mend. Tools and kit wear per stroke and per hit (gladius edge, dolabra, lorica rivets, hobnails); a mend process restores them (regrind, rivet a lobate hinge, replace nails). Tier A. Size S. Owns `crates/rome/src/kit.rs`, `content/rome/tech/mend.toml`.
  Accept: `cargo test -p engine-rome --lib kit` passes: 200 dig strokes blunt a dolabra below its bar, a regrind restores it, a broken hinge mends with 5 rivets per half, replay twice equal.
  F2P: kit never wears.
  P2P: golden x2.
  needs: MK1.
- [ ] MK3 rome: timber. Felling is season-gated (early autumn to spring, Vitruvius 2.9), a log becomes stakes or pila shafts, oak for ground work and alder for marsh piles. Tier B. Size S. Owns `crates/rome/src/timber.rs`, `content/rome/tech/timber.toml`.
  Accept: `cargo test -p engine-rome --lib timber` passes: a spring felling is refused with the reason, one log yields its stake count, replay twice equal.
  F2P: no timber loop.
  P2P: golden x2.
  needs: MK1.
- [ ] MK4 rome: lime and mortar. Burn stone (a third of its weight leaves), slake hot, mix 1 lime : 3 pit sand or 1 : 2 river sand (Vitruvius 2.5); a wrong mix sets weaker. Tier B. Size S. Owns `crates/rome/src/lime.rs`, `content/rome/tech/lime.toml`.
  Accept: `cargo test -p engine-rome --lib lime` passes: the burn loses one third of the mass, the right mix gives the full strength and a wrong mix less, replay twice equal.
  F2P: no lime loop.
  P2P: golden x2.
  needs: MK1.
- [ ] MK5 rome: the scutum. Build and re-cover: two glued plank layers, linen, calfskin, a rim (Polybius 6.23); hits wear the cover and the boards, and the drawn shield shows the damage. Tier B. Size S. Owns `crates/rome/src/scutum.rs`, `content/rome/tech/scutum.toml`.
  Accept: `cargo test -p engine-rome --lib scutum` passes: a shield takes its hit budget before the boards split, a re-cover restores it, replay twice equal.
  F2P: shields never wear.
  P2P: golden x2.
  needs: MK1.
- [ ] MK6 rome: cranes. Trispastos (3 pulleys) and pentaspastos (5) with three gangs on the ropes and a treadwheel (Vitruvius 10.2); the crew count sets the lift speed. Tier B. Size S. Owns `crates/rome/src/crane.rs`, `content/rome/tech/crane.toml`.
  Accept: `cargo test -p engine-rome --lib crane` passes: a pentaspastos lifts the same load faster than a trispastos with the same crew, a missing gang halves the speed, replay twice equal.
  F2P: no lifting machine.
  P2P: golden x2.
  needs: MK1.
- [x] CDX1 tools: the codex fetch tool. `node tools/codex/fetch.mjs <source>` reads `tools/codex/sources.json` (the twelve sources in `rome-codex.md` section 2: URL pattern, rate limit, size cap), writes to `content/local/codex/<source>/` with a `.src.json` sidecar (URL, time, sha256, the stated licence), prints the bytes before it starts, and needs `--yes` above 500 MB. `--dry-run` prints the plan only. Tier A. Size S. Owns `tools/codex/fetch.mjs`, `tools/codex/sources.json`, `tools/codex/fetch.test.mjs`. Landed 1fcb9e1a. `node --test tools/codex/fetch.test.mjs` 8 tests pass (dry run prints twelve PLAN lines, sidecar written, size cap leaves no half file, rate window honoured, --yes asked above 500 MB, bad table refused); every source url probed live; `node tools/workspace-check.mjs` PASS. Fix 2026-10-21: the wikidata query asked for wd:Q11303, which is the skyscraper, so the pull answered with towers; it now asks wd:Q163323 (Roman legion) and wd:Q178561 (battle), the two queries CDX3 fetched, so a fresh pull reproduces `content/rome/places.json`. `node --test tools/codex/fetch.test.mjs` 8 tests pass; the dry run plans two wikidata units.
  Accept: `node --test tools/codex/fetch.test.mjs` passes against a local fixture server (sidecar written, cap enforced, rate limit honoured, a dry run writes nothing); a real `--dry-run` over all sources prints twelve lines.
  F2P: no codex tool.
  P2P: workspace-check PASS.
  needs: none.
- [x] CDX2 tools: the passage index. `node tools/codex/index.mjs` cuts each fetched text into passages by book, chapter and section, tags each with topics (march, camp, tool, bridge, road, siege, craft, place, year) and the numbers with units it holds, and writes `content/local/codex/index.jsonl`. Tier A. Size S. Owns `tools/codex/index.mjs`, `tools/codex/index.test.mjs`. Landed 3940897b. `node --test tools/codex/index.test.mjs` 12 tests pass (the fixture of Vegetius 3.8 and Caesar BG 4.17 gives the refs `vegetius.3.8` and `caesar.bg.4.17`; the camp passage tags `camp` and holds `5 ft` and `3 ft`); the real run over the fetched gutenberg and perseus texts wrote 808 passages and the ref `caesar.bg.4.17` is there; `node tools/workspace-check.mjs` PASS.
  Accept: `node --test tools/codex/index.test.mjs` passes on a fixture of Vegetius 3.8 and Caesar BG 4.17: the passages carry the refs `vegetius.3.8` and `caesar.bg.4.17`, the camp passage tags `camp` and holds `5 ft` and `3 ft`.
  F2P: no index.
  P2P: workspace-check PASS.
  needs: CDX1.
- [x] CDX3 tools: places, roads and the lexicon. Extract Pleiades and Wikidata forts and battles for the six chapter areas into `content/rome/places.json`, Itiner-e roads for the same boxes into `content/rome/roads.json`, and Latin terms with meaning and command use into `content/rome/lexicon.json`; every entry keeps its source id. Tier B. Size M. Owns `tools/codex/places.mjs`, `content/rome/places.json`, `content/rome/roads.json`, `content/rome/lexicon.json`. Landed 33fd4d7. `node --test tools/codex/places.test.mjs` 9 tests pass (a place in a box keeps its id, a place out of every box is dropped, a road in a box is kept, a lexicon term keeps its passage ref, two runs give the same bytes); the real run over the fetched Pleiades, Wikidata and Itin-e files lists `PLACE I Bonna pleiades:108896 51.666461,6.443716`, `PLACE III Mogontiacum pleiades:109169 49.998912,8.273869`, `PLACE IV Drobeta pleiades:207100 44.629271,22.657173` and `PLACE V Sarmizegetusa pleiades:207425 45.669615,23.323961`, writes 73 places, 90 roads and the five orders of `docs/plan/rome.md` 4.5; `node tools/workspace-check.mjs` PASS.
  Accept: `node --test tools/codex/places.test.mjs` passes on fixtures; the real run lists Bonna, Mogontiacum, Drobeta and Sarmizegetusa with coordinates and ids; the lexicon holds the orders in `rome.md` 4.5.
  F2P: no place data.
  P2P: workspace-check PASS.
  needs: CDX2.
- [x] CDX4 tools: tech tables from the texts. Pull the numbers with units and refs from tagged passages into draft `content/rome/tech/*.draft.toml`; a person or the lead promotes a draft line into the real table. Tier B. Size M. Owns `tools/codex/tech.mjs`, `tools/codex/tech.test.mjs`. Landed c1b5c31. `node --test tools/codex/tech.test.mjs` 10 tests pass (the Vegetius 3.8 fixture yields the camp draft with the 5 ft and 3 ft ditch and the ref `vegetius.3.8`; nothing writes a non-draft file); `node tools/workspace-check.mjs` PASS.
  Accept: `node --test tools/codex/tech.test.mjs` passes: the Vegetius 3.8 fixture yields a camp draft with the 5 ft and 3 ft ditch and its ref; nothing writes a non-draft file.
  F2P: tables are typed by hand with no link to the passage.
  P2P: workspace-check PASS.
  needs: CDX2.
- [ ] CDX5 tools: the Column storyboard and reference boards. List the Trajan's Column scenes (Cichorius numbers) with a public-domain image ref each and a one-line caption into `content/rome/column.json`, and build reference boards (helmet, gladius, scutum, lorica, dolabra) from CC0 museum records into `content/rome/boards/*.json`. Tier B. Size M. Owns `tools/codex/boards.mjs`, `content/rome/column.json`, `content/rome/boards/`.
  Accept: `node --test tools/codex/boards.test.mjs` passes on fixtures; the real run writes at least 100 scenes and 5 boards, each image with its source URL.
  F2P: no storyboard or boards.
  P2P: workspace-check PASS.
  needs: CDX1.
- [ ] CDX6 tools: a Rome number needs a source. `tools/workspace-check.mjs` fails a `content/rome/**/*.toml` entry that holds a number and no `ref`, and a `ref` that is not in the codex ref list. Tier A. Size S. Owns `tools/workspace-check.mjs`, `tools/workspace-check.test.mjs`.
  Accept: `node --test tools/workspace-check.test.mjs` passes with a fixture entry without `ref` reported by path and key; the tree run PASS.
  F2P: unsourced numbers pass.
  P2P: workspace-check PASS on the tree.
  needs: ROM2.
- [ ] CDX7 endpoint: `codex.search` and `codex.cite`. `codex.search { query, topic?, limit }` returns passages with refs from the index; `codex.cite { file, key, ref }` adds a ref to a Rome table entry as one undoable change. Tier B. Size S. Owns `crates/endpoint/src/packs/codex.rs`, `crates/endpoint/src/packs/mod.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib packs::codex` passes on a fixture index: a "camp ditch" query returns the Vegetius passage first, a cite then undo leaves the file byte-equal, a missing index is a typed refusal.
  F2P: the door knows no sources.
  P2P: golden x2.
  needs: CDX2.
- [ ] CDX8 tools: credits from the sidecars. `node tools/codex/credits.mjs` writes `content/rome/CREDITS.md` from every sidecar used by a shipped Rome file (source, URL, the stated licence). Tier B. Size S. Owns `tools/codex/credits.mjs`, `content/rome/CREDITS.md`.
  Accept: `node --test tools/codex/credits.test.mjs` passes; the real run lists every source a Rome table cites.
  F2P: no credits.
  P2P: workspace-check PASS.
  needs: CDX4, CDX5.
- [x] KIT1 rome: kit says how sure we are. `content/rome/kit.toml` lists the period kit from `rome.md` section 3 (Corbridge lorica, Weisenau helmet, Pompeii gladius, pilum, scutum, pugio, dolabra, caligae) and the section 4.9 table, each with `certainty` (attested, reconstructed, contested, invented), refs, and `found` (site, year found, object date) when an object exists; `crates/rome/src/certainty.rs` loads it and refuses an item without a certainty or a ref. Tier B. Size S. Owns `crates/rome/src/certainty.rs`, `crates/rome/src/lib.rs`, `content/rome/kit.toml`. Landed e2928a7. `node tools/gh-check.mjs cargo test -p engine-rome --lib certainty` 9 passed (18 in the whole lib); the Xanten-Wardt manuballista reads `attested`, found 1999, its hand-held reading `debated`; an item with no certainty is refused with its key `kit.lorica` named; two loads hash the same; `node tools/gh-check.mjs cargo clippy -p engine-rome --all-targets -- -D warnings` PASS; `node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli default_and_max_seed` PASS, golden d57b5c8ae3096614 x2; `node tools/workspace-check.mjs` PASS; `node tools/rust-quality.mjs --owned crates/rome,content/rome/kit.toml` PASS.
  Accept: `cargo test -p engine-rome --lib certainty` passes: the table loads, the Xanten-Wardt manuballista reads `attested` with `found` 1999 and its hand-held reading marked debated, an item with no certainty is refused with its key named, and two loads hash the same.
  F2P: kit has no certainty and no finds.
  P2P: golden d57b5c8ae3096614 x2.
  needs: ROM2.
- [ ] KIT2 combat: falx against arm guard and helmet. LATER: after LEG10. The Dacian falx hooks over the shield rim (a reach past the rim the gladius lacks), the manica cuts arm-hit damage on the sword arm, the cross-braced helmet cuts head-cut damage; values are `reconstructed` in `content/rome/kit.toml` with refs (Adamklisi metope XVII, Trajan's Column). Tier B. Size S. Owns `crates/combat/src/design.rs`, `content/rome/kit.toml`.
  Accept: `cargo test -p engine-combat --lib falx` passes: a falx cut lands on a shielded man where a gladius cut does not, and the manica and braced helmet each lower the damage they cover; rules v2 only.
  F2P: no falx and no arm guard.
  P2P: rules v1 and golden d57b5c8ae3096614 x2.
  needs: KIT1, LEG10.
- [ ] KIT3 arena: the manuballista (cheiroballistra). The ballistarius's one-man torsion bolt-shooter: wind the two springs (ticks per turn from the table), load an 18 cm iron bolt, shoot flat and fast (start value 31 m/s, the reconstruction figure), reload; a bolt pierces a shield or sticks in it by the table, and a hit on an unshielded man wounds through the body path. Direct fire from the rampart and over a work party; ammo is counted. Start numbers carry refs (the owner's reconstruction sources) in `content/rome/kit.toml` and are rechecked when the codex holds Schalles 2010. Tier B. Size S. Owns `crates/arena/src/bolt.rs`, `crates/arena/src/lib.rs`, `content/rome/kit.toml`.
  Accept: `cargo test -p engine-arena --lib manuballista` passes: wind, load and shoot take the table's ticks; a bolt's flight and hit replay byte-equal twice; at 30 m a bolt drops less than a thrown pilum at the same range; a bolt into a scutum sticks or pierces as the table says; rules v2 only.
  F2P: no manuballista.
  P2P: rules v1 and golden d57b5c8ae3096614 x2.
  needs: KIT1, LEG5.
- [ ] KIT4 rome: name your kit. Punch letters into the shield boss or helmet with a punch, one letter per stroke, as on the Tyne boss of Junius Dubitatus (legio VIII Augusta); the text lives in the save and shows on the drawn shield. Tier C. Size S. Owns `crates/rome/src/scutum.rs`, `content/rome/tech/scutum.toml`.
  Accept: `cargo test -p engine-rome --lib punch` passes: 12 strokes punch "IVL DVBITATI" style text of up to 24 letters, the save round-trips byte-equal, and a letter outside the Latin capitals is refused.
  F2P: kit has no owner marks.
  P2P: golden x2.
  needs: MK5.
- [ ] KIT5 game: slingers and caltrops. LATER: after LEG10. Auxiliary slingers throw lead glandes (some inscribed: a found one is a codex card, `contested` date); iron caltrops join Caesar's lilia and stimuli as camp field traps that wound and slow. Tier C. Size S. Owns `crates/rome/src/siege.rs`, `content/rome/kit.toml`.
  Accept: `cargo test -p engine-rome --lib caltrop` passes: a walker crossing a caltrop field slows and takes the listed wound, a cleared field does not; a sling volley replays byte-equal.
  F2P: no slingers or caltrops.
  P2P: golden x2.
  needs: KIT1, LEG15.
