# Realistic characters and real mocap (owner 2026-10-22: "I need realistic chars", "fight and sword animations, feeling real with weight")

Decision: characters are realistic, built on Microsoft Rocketbox avatars (MIT,
115 rigged humans, ~7.4k tris, Biped rig), with Roman kit made by us on top.
Core moves are real mocap from Mixamo (Sword And Shield packs, 101 clips in
`C:\forge-media\mixamo\`); Kimodo text-to-motion (`tools/motion/clip.mjs`) adds
variety. Weight comes from the engine (FEEL rows): weapon mass, hitstop,
knockback. The stylised MPFB lane stays as a fallback only.

Tools that exist (2026-10-22): `tools/asset-gen/fighter.mjs` (the game path:
one avatar to a fighter in the arena, RB7), `tools/asset-gen/character.mjs --from rocketbox`
(the look study: LODs, gates), `tools/blender/retarget_mixamo.py` (a Mixamo clip on a Rocketbox
body, exact world-rotation transfer, renders checked), `tools/motion/mixamo.mjs`
(a pack baked for the engine: 50 of 50), `crates/assets/src/rocketbox_map.rs`
(Biped to fighter-rig names).

- [x] RB1 game: a Rocketbox body is a fighter body in the arena window.
  Landed 600c16994: `tools/blender/rocketbox_fighter.py` cooks Military_Male_01
  into its own family tree `content/fighters/rocketbox/` (body + idle, run,
  slash, attack, slash (2), death on the body's own skeleton, CT7 joint names);
  `E2040_BODIES=rocketbox` loads it (`FighterLook::load_rocketbox`). The engine
  refuses a scaled joint (`BodyImportError::ScaledJoint`) and a pelvis outside
  0.6-1.4 m (`check_body_units`). Capture `target/clips/rb_legionary`, view=ok.
- [x] RB2 tools: the character gates fit realistic bodies. The 30 m fill band
  (0.40-0.70) was set on blockouts; an arms-out body reads at 0.30-0.35 (measure
  on a guard-pose frame instead of the T-pose, or a band per lane). The page
  budget counts the 1024 px PNGs; count the KTX2 the rig cook makes (256 px).
  Files: `tools/asset-gen/character.mjs`, `tools/blender/mesh_silhouette.py`.
  Accept: both Rocketbox avatars pass, a blockout that read as a rectangle
  (0.7443) still fails; node tests.
  Landed 2026-10-22: the rocketbox lane has its own fill band 0.25-0.70
  (`ROCKETBOX_BAND`; arms-out avatars read 0.30/0.35, a 0.7443 rectangle still
  fails) and its budget counts what ships (`rocketboxShippedBytes`: the GLB
  without its PNGs plus the 256 px BC1/BC5 KTX2 per map). Male_Adult_08:
  gate pass, budget pass, 621 KB of 2.04 MB. 27 node tests.
- [x] RB3 assets: the weapon rides the hand. `tools/blender/roman_kit.py`
  (gladius 642 tris, scutum 660) grafted to `Bip01 R Hand` / `Bip01 L Hand`
  by the cook (600c16994); the RB1 capture shows sword and shield.
- [x] RB7 core: one command to a game fighter, and the AI notices a broken
  view. `node tools/asset-gen/fighter.mjs --avatar <G>/<Name> --name <v> --prove`
  (fetch, kit, cook, manifest check, real-window proof needing exit 0 and
  `view=ok`); the family manifest is the game's only variant list
  (`fighter_bind::rocketbox_variants`), so a second avatar needs no code edit
  (proven: rb_male08 cooked beside rb_legionary in target/fighters_try).
  The capture line carries `view=ok|FAIL(...)` (`ViewLaws::verdict`) and an
  armed capture of staged boxes exits 1. Skills `assets-content` and
  `door-and-headless-runbook` name the path.
- [ ] RB4 assets: the Roman kit on a Rocketbox body. Tunic, lorica, galea and
  caligae made in Blender (`tools/blender/armour_pass.py`) and skinned to the
  Biped rig by nearest-bone weights; one legionary and one Ashkin warrior.
  Accept: hero renders of both, under the potato budget at LOD2.
- [ ] RB5 motion: the Kimodo prompt library. 150 combat prompts (cuts,
  thrusts, blocks, parries, shield bashes, staggers, deaths, get-ups; light and
  heavy) encoded once on Kaggle (`tools/motion/kaggle_encode.py`), so
  `tools/motion/clip.mjs` runs any of them locally. Accept: the cache holds
  them all; three new clips bake under the 0.08 slip gate.
  Progress 2026-10-22: the library is data, `content/motion/combat_prompts.txt`
  (168 prompts, every category, the 16 cached texts kept verbatim);
  `kaggle_encode.py --library <file>` encodes it in one run (`--check` validates
  on the PC); `node tools/motion/clip.mjs --list-library` prints cached vs missing
  (16/168 today). Left: the Kaggle encode run and three new clips under the slip gate.
- [ ] RB6 anim: attack clips enter and leave without a pop. The look's
  fighter_look proofs count 551 pops with the teacher packs in the attack
  slots against 431 without (clip_switches_do_not_pop_in_a_real_duel and six
  more red, run 39747341111). Inertial blend in and out of every attack slot,
  root travel handed to the sim (in-place clip plus the travel as a curve).
  Accept: those seven tests green with real attack clips in the slots.
