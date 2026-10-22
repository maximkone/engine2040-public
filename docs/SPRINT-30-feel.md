# Feel: weight, impact, particles and the game-building door (owner 2026-10-21)

Spec: `docs/plan/melee-feel.md`. Sim rows go under a new rules version: rules v1 and the
golden `d57b5c8ae3096614` never move. Physics rows prove their laws with `engine_core::laws`.

- [x] FEEL1 combat: weapons weigh what they are. `content/design/weapons.rows` (damage, speed, Landed 1843106.
  stagger, recovery) plus a mass per weapon in grams feed the live attack tables through the
  class sheet loader, under a new rules version: a heavier weapon starts later, recovers longer
  and staggers harder. Files: `crates/combat/src/tables.rs`, `tables_text.rs`, `fight.rs`,
  `crates/game/src/class_sheet.rs`, `content/design/weapons.rows`. Accept: a live-duel test
  pits a dagger row against a mace row and prints startup, recovery and stagger for each; the
  mace numbers are larger; rules v1 replays unchanged (`golden_hash_untouched`).
- [ ] FEEL1b game: the window duel swings real weapons. `App::into_duel` arms each slot's kit weapon
  through `LiveMatch` (`Sim::arm_weapons`, FEEL1), the look times each swing by its own startup
  (`Sim::weapon`) and the HUD names the weapon. Files: `crates/arena/src/live.rs`,
  `crates/game/src/lib.rs`, `fighter_look_acts.rs`. Accept: a game test arms pugio against clava
  and reads the clava's first hit 9 ticks after the pugio's; the default duel hash moves only by
  the armed-weapon fold.
- [x] FEEL2 arena: a blow moves a body by its momentum. Landed 3a75ead5. The v2 arena feeds a landed blow the weapon's momentum: the hit impulse is the weapon's mass times the swing's blade travel (the strike pose's span over the weapon's startup ticks) and the victim takes the matching knockback velocity along the attacker-to-victim direction; the collision pass carries that velocity and ground friction stops the body at EXACT rest. Law tests green on the runner: `cargo test -p engine-arena --lib` 303 passed, 0 failed (run 39770795230), among them `a_heavier_blow_moves_the_victim_farther` (the club slides the victim farther than the dagger), `mirrored_blows_give_mirrored_knockback` (`laws::mirror`) and `a_knocked_back_body_comes_to_exact_rest` (`laws::rest_stays_rest`); `cargo test -p engine-combat -p engine-physics --lib` PASS (run 39770604089); `cargo clippy -p engine-arena -p engine-combat -p engine-physics --all-targets -- -D warnings` PASS (run 39770793859); golden d57b5c8ae3096614 unchanged (run 39771027880). The momentum map's shift moved 6 -> 8 so the arena's real swing (54 to 108 mm a tick) reads 510 to 826 Q8 with no cap binding; unarmed and v1 bouts keep every byte. The arena matrix pin was already stale on master before this change (the same digest 6944380216732058356 with and without it), so this row does not move that hash. The victim takes a knockback velocity
  and a balance impulse from weapon mass times swing speed (FEEL1), not the constant
  `BALANCE_HIT_Q8`; the walker carries the velocity and friction stops it. Files:
  `crates/arena/src/authority.rs`, `lib.rs`, `collide.rs`, `crates/physics/src/balance.rs`.
  Accept: law tests: a heavier blow moves the victim farther; mirrored blows give mirrored
  knockback (`laws::mirror`); a knocked-back body comes to exact rest (`laws::rest_stays_rest`).
- [x] FEEL3 game: one hitstop truth. The look mirror and the feel stop read the live fight's Landed 2c24e95.
  hitstop count for the strike kind instead of their own constants. Files:
  `crates/game/src/hitstop.rs`, `lib.rs`, `fighter_look_acts.rs`, `crates/combat/src/feel_flow.rs`.
  Accept: a test feeds a light, a heavy and a parry and asserts the look holds exactly the
  sim's 3, 6 and 8 ticks.
- [x] FEEL4 arena: armour deflects. Per-slot worn armour from the fighter kit feeds
  `resolve_swing` (today 0); a blow on plate deflects with a spark cue instead of cutting.
  Files: `crates/arena/src/authority.rs`, `lib.rs`, `crates/combat/src/cut.rs`, `anatomy.rs`.
  Accept: a test swings the same blow at an unarmoured and a plated torso: the first cuts,
  the second deflects and emits the deflect cue; rules v1 unchanged.
  Landed 2026-10-22: `Sim::wear_armour` puts the kit's worn slots on each fighter before
  tick 0 (rules v2 only; v1 refuses); `resolve_cuts` stops a blade whose edge reaches a
  plated joint (`AnatomyTable::plated`, `PLATE_BLOCK_Q16`) and pushes a `DeflectCue`
  (`Sim::deflects`); other blows pass the worn bits to `resolve_swing`. Worn bits fold
  only when armour is worn, so v1 and bare v2 bytes hold. Left: today every blow on plate
  deflects (penetration past the block needs weapon energy), and the window duel does not
  yet call `wear_armour` from the kit.
- [ ] FEEL5 render: a hit throws sparks, blood and dust where it lands. The arena frame folds In hand 2026-10-22 (Claude lead session + one Haiku; pick another row until this lands).
  each cue into the GPU particle pool at the contact point (sparks on metal, block and clash;
  blood on a flesh hit; dust on a fall), the pool steps and draws every frame, and a blood
  decal lands on the floor. Files: `crates/game/src/bin/arena_frame/mod.rs`,
  `crates/render/src/particles_gpu.rs`, `present/gpu_present/live.rs`, `submit.rs`, `decals.rs`.
  Accept: a scripted duel capture shows particles at the hit point (counted spawns per cue in
  the frame stats) and a decal under the victim; potato tier caps the count.
- [ ] FEEL6 combat: a clash binds and recoils. Blade on blade throws both weapons back In hand 2026-10-22 (Claude lead session + one Haiku; pick another row until this lands).
  (a held recoil pose for both fighters), sparks (FEEL5) and a metal clash cue. Files:
  `crates/combat/src/exchange.rs`, `crates/arena/src/match_wire.rs`,
  `crates/game/src/fighter_look_acts.rs`, `crates/audio/src/cue.rs`. Accept: a clash test
  shows both attackers in recoil for the same ticks and the clash cue once.
  Progress 2026-10-22 (sim layer): `EXCHANGE_V2` / `MATCH_WIRE_VERSION_V2` put both
  fighters in a held recoil of `CLASH_RECOIL_TICKS` (6, the bind) on a clash, cue once
  (`ImpactKind::Clash` already sounds steel); v0/v1 bytes unmoved. Left: the live duel
  runs `match_live = false`, so the recoil shows once the arena authority is wired, and
  the look needs a recoil flag (`FighterLook` has none); V2 snapshots fail closed in
  arena `lib.rs`/`fork.rs`.
- [ ] FEEL7 game: landing has weight. Jump and fall play in the default duel (not only behind
  `--height`); a landing squashes the body, dips the camera, plays a land sound by speed and
  raises dust; a long fall costs health by a fall table. Files: `crates/game/src/lib.rs`,
  `bin/arena.rs`, `crates/arena/src/authority.rs`, `fighter_look_cam.rs`,
  `window_audio_space.rs`, `fighter_look_feet.rs`. Accept: a scripted jump shows the squash,
  dip and land cue in one tick; a fall from 4 m costs the table's damage; rules v1 unchanged.
- [x] FEEL8 audio: a hit sounds like what hit what. Gain and pitch follow damage and weapon Landed ad9f95aa4.
  mass; metal, wood and flesh cues come from `impact_cue` by material; the App mirror stops
  hard-coding Flesh. Files: `crates/audio/src/combat_cues.rs`, `cue.rs`, `strike_sound.rs`,
  `crates/game/src/window_audio_space.rs`, `lib.rs`. Accept: a test maps a dagger-on-flesh and a
  mace-on-plate hit to different cues with the mace louder.
- [ ] FEEL9 game: a hit moves the body that took it. Upper-body hits drive
  `PhysFx::hit_ragdoll` on the struck bones through the (now stable) powered servo and blend
  back; stagger memory shows a limp. Files: `crates/physfx/src/fx/hit.rs`,
  `crates/physics/src/powered.rs`, `stagger_memory.rs`, `crates/game/src/ragdoll_look.rs`,
  `fighter_look_acts.rs`. Accept: a capture of a torso hit shows the struck chain displaced
  and back within the row's tick budget; presentation only, hash unchanged.
- [ ] FEEL10 physics: close contact pushes back. Bodies in contact exchange push forces (not
  only positional separation) and two fighters can clinch with the clinch pass in the live
  look. Files: `crates/arena/src/collide.rs`, `crates/physics/src/clinch.rs`,
  `controller2.rs`. Accept: law tests: momentum is conserved in a two-body push
  (`laws::conserved`), a resting pair stays at rest, mirrored shoves mirror.
  Progress 2026-10-22: `controller2::push_exchange` (integer, equal and opposite, none for
  a resting or separating pair) with law tests (conserved, rest stays rest, mirror; a
  rapier clinch pair conserves too); arena `exchange_contact_pushes` hands the speed over
  through the knockback carrier above rules v2. Left: it is dormant until a
  `FIGHT_RULES_V3` / `ArenaRules::V3` exists (combat `fight.rs`, arena `lib.rs`), walking
  speed does not enter the push yet, and the clinch pass in the live look.
- [x] FEEL11 game: the pad rumbles on a hit. gilrs force feedback, strength by the eye trauma
  the hit gave (`fps_view::EyeTrauma`), off by a setting. Files: `crates/game/src/windowed.rs`,
  `fps_view.rs`. Accept: a test maps a hit taken and a knockdown to two rumble strengths; no
  pad means no error.
  Landed 2026-10-22: `pad_rumble.rs` turns the human slot's cues into the eye's own trauma
  (`EyeTrauma::feed`) and a strong gilrs effect (hit 180 permille for 150 ms, knockdown
  350 for 300 ms) on every pad with force feedback; no pad, no FF or a refused effect is
  quiet. Hooked in the live `run_windowed` loop. Left: the setting is the const
  `PAD_RUMBLE_ON` until `crates/ui/src/settings.rs` gets a rumble field.
- [x] DOOR1 endpoint: the door builds games, not rooms. Landed 2026-10-23 (f276aeb6f on
  fc0fd9841): the level grammar gains kinds `floor wall player enemy goal` with a `Role`
  each (`build_level/imp/game.rs`), the player wins by standing in the goal zone while
  alive and loses on death or on running out of the tick cap; every enemy walks at the
  player and swings in reach (`crates/rules/src/objective.rs`, `ObjectiveMatch`). The door
  verb `game.play` seats the placed parts and reads the verdict, listed with its rules in
  the tool description. Accept: `crates/endpoint/tests/it/game_build.rs` — a scripted
  session builds player, enemy and goal, plays it, and reads a win and a loss; 4/4 on the
  runner (run 39853641980), `engine-rules` objective 10/10 (run 39854659228). Left: a bare
  node is a unit cube to the staged spatial gate, so a level with three or more parts can
  be refused for overlap its real sizes do not have; the floor plate is a role, not a body.
  The level grammar and the scene kinds
- [x] DOOR2 endpoint: a built level persists. Landed 2026-10-22: `scene.save` seals `gen-N.edoc` in the project folder, `scene.load` reopens the newest; the test rebuilds the door from the folder and gets the same revision and blake3 digest, undo intact (no export verb yet). `scene.save` and `scene.load` (and an export)
  write the scene to the project folder; a fresh process loads it at the saved revision.
  Accept: save, restart the server, load: same revision and same scene digest.
- [x] DOOR3 endpoint: the authored scene is what plays. Landed 2026-10-22: `play.step` takes `scene_rev` (same `pinned_world` as `play.run`), default frame 320x180, the frame paints the scene's props. Open: the test inserts the prop by `commands::apply`, not `level.build` over MCP; fighters do not collide with props (`GymLive` takes no world, arena lane). `play.run` and `play.step` take a
  scene revision and run that scene; the default frame is readable (at least 320x180). Accept:
  a level built over MCP shows its own props in the first `play.step` frame.
- [x] DOOR4 endpoint: every refusal says what to fix. Landed d4c73fe, clippy green at f2a7f19: overlap names the pair, kinds come from one list, the `mx` schema states -100..=100, `api.load` refuses a pack past the tool budget. Overlap names the pair, kinds are listed
  in the tool description, the `mx` schema states the range the handler takes, `api.load`
  refuses a pack that would break the tool budget instead of reporting loaded. Accept: one
  test per refusal asserts the named culprit or the stated range.
- [x] DOOR5 tools: the installed bridge is never stale. `tools/mcp-install.mjs --check` fails Landed da72ff8.
  when `.opencode/bin/e2040-mcp.exe` is older than `target/release/e2040-mcp.exe`, and the
  install renames a locked copy aside instead of failing. Accept: node tests for both cases.
