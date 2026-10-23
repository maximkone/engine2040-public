# Open-source games as donors, not engines

Read 2026-10-23. Eight shipped or playable games with readable source, picked for what
they have that a demo does not: people with memory, a camp with steps, a body that keeps
its wounds. Nothing is copied as code. Each steal is a data shape or a cheap mechanic.

Read: `git clone --depth 1 --filter=blob:none` plus `sparse-checkout` of the dirs named
below, then a newline count per file by extension. Every row names its scope, because a
whole-repo count for a game with art would measure art. No game was run or played: every
claim comes from the source.

**We are not behind on barks or formations.** `crates/game/src/announcer.rs` (bout lines
as data), `crates/ai/src/dialogue.rs` (baked barks), `crates/audio/src/voice.rs`,
`crates/audio/src/crowd.rs` (reactions with cooldowns), `crates/ai/src/legion.rs` (500
soldiers, formation brain, thinking LOD) and `crates/arena/src/legion_bind.rs` (the
contubernium of 8) already cover the cheap tricks people usually list. What is missing is
the layer above them: men with names and a record, a place with steps, a body that carries
damage past the fight.

## The sources

| Game | Shipped / license | Counted size | Take this | Not there |
|---|---|---|---|---|
| 0 A.D. | free download, no Steam; GPL-2.0+ code, art CC-BY-SA 3.0 | sim data 2,090 files / 110,127 lines (677 unit + 531 structure XML, JS); sim C++ 233 / 51,364 | one small file per fact; a formation as a table of slots | no first-person, no feel, no soldier memory |
| OpenXcom | free, no Steam; GPL-3.0 | 646 files / 175,397 C++ (incl. 15,749 scaler + 6,245 lodepng); 32 YAML rulesets | the soldier record: missions, kills, recovery days, diary | no real time, no formation, squad is player-driven |
| OpenJK (Jedi Academy) | shipped 2003, Steam 6020; GPL-2.0 | code/ 434 / 423,133; game/ 149 / 191,366; cgame 54 / 51,576 | blade segments and parry windows read from one text anim file | no squad brain, no roster, no crafting |
| Veloren | open-source MMO, no Steam; GPL-3.0 | 374 files / 141,672 Rust (client, common, server, utils, tools) | recipe with a non-consumed tool input; modular kit components | no line combat, no siege, no camp |
| OpenArena | free, GPL-2.0 game code | code/ 248 / 170,226 C; game/ 64 / 58,948 | 11 bot-chat hooks with a per-character dial and a cooldown | no career, no persistence, no crafting |
| Shattered PD | shipped, Steam 1769170; GPL-3.0 | core/src 1,221 / 220,175 Java | level feeling line, badges, a journal that fills in | no real time, no 3D, no formation |
| Pixel Dungeon (2014) | shipped on mobile; GPL-3.0 | src 366 / 44,297 Java | 44k lines that read rich: feelings, curses, badges | same as above, less of it |
| CDDA | free, no Steam; CC-BY-SA 3.0 code and content | src 1,177 / 787,932 C++ (incl. third-party); recipes 197 files / 117,992 lines, 486 recipes | body-part wounds; recipes with tools, stations, skills | no real time, no formation, no crowd |

## What each one has

**0 A.D.** (Wildfire Games, Rome-era RTS). Systems: 5 damage types as one JSON each
(hack, pierce, crush, fire, poison), armour multipliers per unit, 2 status effects,
15 formation templates, a 6,530-line `UnitAI.js` order FSM, an 18,668-line Petra AI.
Layout: one XML per unit (677) and per structure (531), each with `parent=` inheritance,
`Identity` name and icon, `Promotion` to the next rank entity, a `Builder` list, and
`Sound` groups naming one XML per event (`attack_melee` -> `attack/weapon/sword_attack.xml`).
Comrades/formations: `testudo.xml` is 16 members, 8 columns, and a `row..row,col..col: anim`
grid that gives every cell its own animation; that is a formation as data, not code.
Lacks: no first person, no per-soldier memory, no feel; a unit is a template, not a person.

**OpenXcom** (GPL clone of X-COM). Systems: base, research, manufacture, geoscape, turn
battles with 11 soldier stats. Layout: 32 YAML rulesets, one per domain (`soldiers.rul`,
`items.rul`, `armors.rul`, `manufacture.rul`), mods as extra files over the same loader.
Comrades: none, the squad is the player's. What it does have is a *record*: `Soldier` holds
id, missions, kills, `_recovery` (days in hospital), rank, and a `SoldierDiary` of
commendations, a kill list and mission ids, saved with the game.
Lacks: no real time, no formations, no melee geometry.

**OpenJK** (the shipped Jedi Academy code, GPL). Systems: saber combat, NPC AI, animation
events. Layout: one file per weapon/behaviour (`wp_saber.cpp` 14,452 lines, `bg_panimate`
7,258, `AI_Jedi` 7,669), animation ranges read from a text `animation.cfg` beside the model.
Melee: each blade is two traced points (`muzzlePoint`, `muzzleDir`, `length`, old pose kept
for the sweep), with `WP_SaberBlockNonRandom`/`Random`, broken-parry and knockaway moves,
six styles and kata. Lacks: no squad formation, no roster, no crafting; the ally NPC is a
scripted follower.

**Veloren** (Rust, GPL). Systems: ECS world, inventory, crafting, NPC agents. Layout:
workspace dirs (`common`, `server`, `client`), 66,231 lines in `common/src`, 38,900 in
`server/src`, and a 13,239-line `server/agent` crate of NPC action nodes that runs the
crowd offscreen. Crafting: `Recipe` is output + inputs, and an input can be "not consumed"
(a craftsman's hammer), so a tool is a recipe input with no special case; equipment is
`modular.rs`, a weapon assembled from components that carry the stats.
Lacks: no line combat, no siege, no camp; combat is action, not formation.

**OpenArena** (GPL Q3 code). Systems: arena modes, bots, announcer. Layout: `code/game`
58,948 lines in 64 files, one per system (`ai_main`, `ai_dmq3`, `ai_chat`, `g_combat`).
Comrades: team bots. The cheap part: `ai_chat.c` has 11 event hooks (`EnterGame`,
`StartLevel`, `Death`, `Kill`, ...), each picks a line, and whether it speaks at all is a
per-character trait (`CHARACTERISTIC_CHAT_KILL` 0..1) against a `TIME_BETWEENCHATTING`
cooldown, silenced in tournament mode.
Lacks: no career, nothing persists after the map, no crafting.

**Shattered Pixel Dungeon** (shipped, Steam). Systems: 1,221 files, statuses as buffs,
item identification, talents, badges, a journal (Bestiary, Catalog, Document). Rich for the
price: `Level.Feeling` is an enum of 7 lines, rolled with a 50% chance at level entry and
~7.15% each, so a level "feels" like something before a single monster is seen; badges are
one class in `Badges.java`; the journal fills in as you meet things.
Lacks: no real time, no 3D, no formation.

**Pixel Dungeon (2014, 44,297 lines).** The small one that looks great for its size: the
same bones as SPD (feelings, curses, badges, one file per actor) in 366 files. Proof that
perceived richness is mostly *names, states and short lines*, not geometry. Lacks: same as
SPD.

**Cataclysm: DDA** (CC-BY-SA). Systems: 486 recipes over 197 JSON files, body-part health,
bleeding, bandaging, frostbite, morale, skills. Layout: JSON with `"type"`, so a recipe is
data with `time`, `tools` (with charges), `using` (a toolset or station), `skills_required`,
`proficiencies` and batch time factors; the 12 body parts (`bp_head`, `bp_arm_l`,
`bp_leg_r`, ...) each hold hp, bandaged damage and timers. Lacks: no real time, no
formation, no crowd; every system is turn-based.

## Three cheapest wins, ranked

**P1. Seven named men with a diary line each** (OpenXcom record + 0 A.D. Identity).
Port: one record per comrade - name, fights, kills, wounds, watches, rank step - plus one
diary line appended per fight ("took the first man at the ditch"), and the name over the
HP bar. Names come from a table with a `ref` to an inscription; counters come from play,
so no invented number enters content.
Home: new `crates/game/src/roster.rs` (record, encode, fold), `content/rome/contubernium.toml`
(8 rows), `crates/arena/src/legion_bind.rs` (a `Man` carries the name beside his slot),
`crates/ui/src/play_hud.rs` (name line), save payload through `crates/save` (game owns bytes).
Repro: 2 scripted fights over the existing `FightEvent` stream -> 7 names, 7 kill counts, 7
diary lines; the same script twice gives the same payload bytes; a name with no `ref` is
refused; a save round-trip keeps the record. 4 tests, plus one HUD capture.
Kill: if it needs a new crate edge (note `crates/game/src/march.rs` records that game does
not depend on rome) or if it moves the hash. If so, keep the record in `crates/game` only.
Moves: named men on screen 0 of 7 -> 7 of 7; 5 counters per man; payload grows by about 120
bytes; hash unchanged because it is presentation until P3 makes wounds hashed.

**P2. A camp day that is a table of works** (0 A.D. build steps + CDDA basecamp recipes).
Port: `content/rome/works.toml` with 8 sections (tent row, vallum, fossa, porta, tribunal,
ara, stake line, latrine), each with men-ticks, crew cap, a `ref`, and the ditch it feeds;
each fixed tick adds `crew` to a work, a work completes at its men-ticks, and the ditch
dimensions still come from `crates/rome/src/camp.rs` (fossa depth by threat).
Home: new `crates/game/src/camp_day.rs` (state plus its own strict table reader, the way
`march.rs` does it), `content/rome/works.toml`, progress cells in `crates/ui/src/play_hud.rs`,
and the day opened by `crates/game/tests/it/legionary_slice.rs`'s camp phase.
Repro: 7 men, 3 works, 240 ticks -> one receipt with works done, men-ticks spent and the
ditch equal to the `Camp` table; twice -> same digest; crew 0 and a number with no `ref`
refused. 2 tests, 1 capture.
Kill: if a work needs dug ground in the tick (that is ROM5's geometry) or nav changes; the
row must stay a table plus counters. Also kill if a work cannot be an integer men-tick.
Moves: buildable works 0 -> 8; the camp chapter's player steps 0 -> 1 loop; the slice test
gains a third phase.

**P3. Wounds that outlive the fight** (CDDA body parts + OpenXcom hospital days).
Port: 4 parts (head, torso, arm, leg) x 3 severities (light, deep, crippling) with speed and
accuracy permille, bleed ticks and heal days; set from the existing `engine_combat::cut`
events (joint -> part), read as a plain struct by `crates/ai/src/capability.rs` (already
observes hands, legs and blood), healed one step per camp day, and written into the P1 diary.
Home: new `crates/game/src/wounds.rs`, `content/rome/wound.toml`, the cut event tap in
`crates/arena/src/live.rs`, a HUD pip in `crates/ui/src/play_hud.rs`, payload in `crates/save`.
Repro: a scripted fight with 3 cuts -> 3 ledger rows, speed down N permille, healed after 3
camp days; twice -> same bytes; severity 3 refused; a save round-trip keeps the ledger. 3
tests; one golden re-pin with a receipt, because speed changes the sim.
Kill: if the ledger changes the golden without that receipt, if `ai` would need a new edge
to `game` (pass the plain struct instead), or if a wound cannot heal in whole camp days.
Moves: persisted injury states 0 -> 12 (4 parts x 3 severities); 7 men can each carry one;
1 explicit golden re-pin.

Next in line, not ranked: 0 A.D.'s per-cell formation animation (`row..row,col..col: anim`,
8 columns) feeds the open LEG12 testudo as data, with no code, once LEG12 is picked up.

## Not verified

- Nothing was played. Sizes are newline counts of the checked-out files, 2026-10-23, and
  include third-party code where the row says so.
- On Steam: Shattered Pixel Dungeon (1769170) and Jedi Academy (6020), checked with the
  Steam appdetails API today. 0 A.D., OpenXcom, Veloren, OpenArena and CDDA are free
  downloads, not Steam titles.
- 0 A.D.'s GitHub mirror is archived (moved to gitea.wildfiregames.com), so the code read
  is that mirror's last state (last push 2024-08-17).
- Not covered: The Dark Mod (no public source repo found under that name) and Xonotic (its
  game code sits in a separate data repo that was not fetched).
