# Feel: weight, impact, particles and the game-building door (owner 2026-10-21)

Spec: `docs/plan/melee-feel.md`. Sim rows go under a new rules version: rules v1 and the
golden `d57b5c8ae3096614` never move. Physics rows prove their laws with `engine_core::laws`.

- [ ] FEEL1 combat: weapons weigh what they are. `content/design/weapons.rows` (damage, speed,
  stagger, recovery) plus a mass per weapon in grams feed the live attack tables through the
  class sheet loader, under a new rules version: a heavier weapon starts later, recovers longer
  and staggers harder. Files: `crates/combat/src/tables.rs`, `tables_text.rs`, `fight.rs`,
  `crates/game/src/class_sheet.rs`, `content/design/weapons.rows`. Accept: a live-duel test
  pits a dagger row against a mace row and prints startup, recovery and stagger for each; the
  mace numbers are larger; rules v1 replays unchanged (`golden_hash_untouched`).
- [ ] FEEL2 arena: a blow moves a body by its momentum. The victim takes a knockback velocity
  and a balance impulse from weapon mass times swing speed (FEEL1), not the constant
  `BALANCE_HIT_Q8`; the walker carries the velocity and friction stops it. Files:
  `crates/arena/src/authority.rs`, `lib.rs`, `collide.rs`, `crates/physics/src/balance.rs`.
  Accept: law tests: a heavier blow moves the victim farther; mirrored blows give mirrored
  knockback (`laws::mirror`); a knocked-back body comes to exact rest (`laws::rest_stays_rest`).
- [ ] FEEL3 game: one hitstop truth. The look mirror and the feel stop read the live fight's
  hitstop count for the strike kind instead of their own constants. Files:
  `crates/game/src/hitstop.rs`, `lib.rs`, `fighter_look_acts.rs`, `crates/combat/src/feel_flow.rs`.
  Accept: a test feeds a light, a heavy and a parry and asserts the look holds exactly the
  sim's 3, 6 and 8 ticks.
- [ ] FEEL4 arena: armour deflects. Per-slot worn armour from the fighter kit feeds
  `resolve_swing` (today 0); a blow on plate deflects with a spark cue instead of cutting.
  Files: `crates/arena/src/authority.rs`, `lib.rs`, `crates/combat/src/cut.rs`, `anatomy.rs`.
  Accept: a test swings the same blow at an unarmoured and a plated torso: the first cuts,
  the second deflects and emits the deflect cue; rules v1 unchanged.
- [ ] FEEL5 render: a hit throws sparks, blood and dust where it lands. The arena frame folds
  each cue into the GPU particle pool at the contact point (sparks on metal, block and clash;
  blood on a flesh hit; dust on a fall), the pool steps and draws every frame, and a blood
  decal lands on the floor. Files: `crates/game/src/bin/arena_frame/mod.rs`,
  `crates/render/src/particles_gpu.rs`, `present/gpu_present/live.rs`, `submit.rs`, `decals.rs`.
  Accept: a scripted duel capture shows particles at the hit point (counted spawns per cue in
  the frame stats) and a decal under the victim; potato tier caps the count.
- [ ] FEEL6 combat: a clash binds and recoils. Blade on blade throws both weapons back
  (a held recoil pose for both fighters), sparks (FEEL5) and a metal clash cue. Files:
  `crates/combat/src/exchange.rs`, `crates/arena/src/match_wire.rs`,
  `crates/game/src/fighter_look_acts.rs`, `crates/audio/src/cue.rs`. Accept: a clash test
  shows both attackers in recoil for the same ticks and the clash cue once.
- [ ] FEEL7 game: landing has weight. Jump and fall play in the default duel (not only behind
  `--height`); a landing squashes the body, dips the camera, plays a land sound by speed and
  raises dust; a long fall costs health by a fall table. Files: `crates/game/src/lib.rs`,
  `bin/arena.rs`, `crates/arena/src/authority.rs`, `fighter_look_cam.rs`,
  `window_audio_space.rs`, `fighter_look_feet.rs`. Accept: a scripted jump shows the squash,
  dip and land cue in one tick; a fall from 4 m costs the table's damage; rules v1 unchanged.
- [ ] FEEL8 audio: a hit sounds like what hit what. Gain and pitch follow damage and weapon
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
- [ ] FEEL11 game: the pad rumbles on a hit. gilrs force feedback, strength by the eye trauma
  the hit gave (`fps_view::EyeTrauma`), off by a setting. Files: `crates/game/src/windowed.rs`,
  `fps_view.rs`. Accept: a test maps a hit taken and a knockdown to two rumble strengths; no
  pad means no error.
- [ ] DOOR1 endpoint: the door builds games, not rooms. The level grammar and the scene kinds
  gain player, enemy, goal, floor, wall and spawn, plus rules that act on them (win on goal,
  lose on death, enemy chases player), all listed in the tool description. Files:
  `crates/endpoint/src/scene_tools.rs`, `crates/scene`, `crates/rules`. Accept: a scripted MCP
  session (the runbook's driver) builds floor, player, enemy and goal, plays it and reads a
  win or a loss.
- [ ] DOOR2 endpoint: a built level persists. `scene.save` and `scene.load` (and an export)
  write the scene to the project folder; a fresh process loads it at the saved revision.
  Accept: save, restart the server, load: same revision and same scene digest.
- [ ] DOOR3 endpoint: the authored scene is what plays. `play.run` and `play.step` take a
  scene revision and run that scene; the default frame is readable (at least 320x180). Accept:
  a level built over MCP shows its own props in the first `play.step` frame.
- [ ] DOOR4 endpoint: every refusal says what to fix. Overlap names the pair, kinds are listed
  in the tool description, the `mx` schema states the range the handler takes, `api.load`
  refuses a pack that would break the tool budget instead of reporting loaded. Accept: one
  test per refusal asserts the named culprit or the stated range.
- [x] DOOR5 tools: the installed bridge is never stale. `tools/mcp-install.mjs --check` fails Landed da72ff8.
  when `.opencode/bin/e2040-mcp.exe` is older than `target/release/e2040-mcp.exe`, and the
  install renames a locked copy aside instead of failing. Accept: node tests for both cases.
