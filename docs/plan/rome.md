# Rome: one legionary's war, from the Rhine forests to the Colosseum

The one plan for the Roman game. Owner 2026-10-21: "i want roman arena to focus to roman soldier
simulator with all engineering", then "plan it deeper and write and combine.. realistic af with
making tools. engineering and depth and real rome docs". It merges the gladiator duel (`CONCEPT.md`,
`CORE-LOOP.md`, `gameplay.md`, `rome-spec.md`), the FOCUS map list (arena, district, siege field,
ludus) and the legionary simulator into one game. Sources and the data pipeline: `rome-codex.md`.
Physical layer and helmet view: `touch-and-pov.md`. Rows: `docs/SPRINT-28-rome.md` (ROM, LEG, MK,
CDX). Decision D10 in `docs/FOCUS.md`.

## 0. Decisions (owner 2026-10-21: "u do all decide")

1. **Who and when.** One legionary of Legio I Minervia, AD 83-107. Checked 2026-10-21: Domitian
   raised the legion about AD 82 at Bonna (Livius; Wikipedia); it carried the title Pia Fidelis
   Domitiana from 89 to 96 (Livius); it served in both Dacian wars, and Hadrian commanded it in
   the second (Historia Augusta, Hadrian 3). Not in the sources: the legion's own part in the
   Chatti war of 83 and in the revolt of 89. So chapter II follows a work party of the legion
   on the new frontier, and chapter III is told as an invented loyalty story and says so in game.
1b. **Period names, real features (owner 2026-10-21).** Every historical weapon, tool or work uses its period name (Latin,
   with the Greek beside it: manuballista / cheiroballistra) and is built as a playable thing. Its evidence and any
   scholarly debate go on its codex card, never in place of the feature.
2. **View.** First person through the helmet is the game. Third person stays for replays, the
   spectator and the arena duel proof.
3. **The arena stays and gets a reason.** Legionary fortresses had amphitheatres for weapons drill.
   The existing duel, ladder and glory become the *armatura* (weapons training) in the fortress
   amphitheatre, and the Colosseum returns in the last chapter. The golden scene and rules v1 do
   not change.
4. **Ranks are a career, in Latin.** Tiro, Miles, Immunis, Optio, Centurio replace the mixed
   Tiro/Gladiator/Champion/Legend names. Glory becomes the service record.
5. **Squad.** A contubernium of 8 (the early Imperial tent group; Vegetius' 10 is the late army).
   Co-op is the contubernium: up to 8 players, AI fills the empty places.
6. **Realism is checkable.** Every number in Rome data cites a codex reference (`rome-codex.md`).
7. **Data beats prose.** Where `content/design/*.rows` and a doc disagree, the data file wins and
   the doc is fixed (ROM1 sweeps the 15 known contradictions).
8. **Code home.** Genre-neutral engine features (dig layer, build jobs, timed processes) go in
   engine crates. Roman rules and data (camp plan, recipes, career, chapters) go in a new
   `crates/rome` (std-only, hashed), like `kam2-sim`. `crates/game` does not grow for Rome.

## 1. The look

`docs/design/legionary/helmet-pov-battle.jpg`: through a galea, the lit scratched rim and cheek
guards frame the screen; gladius right, scutum low left; a shield line of red scuta against a
Germanic warband in a forest clearing; smoke, fire, blood spray, sparks off a boss, mud and bodies.
HUD: HP and stamina bottom left, the weapon bottom right. `helmet-pov-march.jpg`: through a
face-mask helmet's eye holes; a wheat field, the legion in column under the eagle and vexilla, a
fort; one objective line top right. These two frames are the look bar for this game (finish-line
B8 scores captures against them).

## 2. The game in one screen

You are one man in a tent group of eight, in a century of eighty, in a legion of about five
thousand. Every day has the same bones, and the chapters change the ground under them.

- **March.** Column order, 60 lb (about 20 kg) of kit, the standard ahead. Regular step 20 Roman
  miles in five summer hours. Time skips between set pieces.
- **Make camp.** Every evening each century digs its stretch of ditch, throws the earth up into the
  rampart and plants stakes. A dug ditch is real ground: walkers and the nav mesh read it.
- **Fight in the line.** 3 ft of front per man, 6 ft between ranks. Pila on the horn, close, shield
  to shield, stab under the rim, push with the boss, the rear rank relieves the front.
- **Make and mend.** In the fortress fabrica and on campaign: smelt, forge and mend iron; fell and
  shape timber; burn lime and mix mortar; build and re-cover a scutum.
- **Build big.** Roads, watchtowers and palisade on the frontier; a bridge over the Danube; a
  siege ring with engines.
- **Rise.** Tiro to Miles to Immunis (an engineer, smith or surveyor) to Optio. Rank opens jobs,
  tools and orders you can give.

## 3. Chapters (each a streamed map of at most 2 x 2 km; no seamless world)

| # | Chapter | Year | Ground | New play | Reuses |
|---|---|---|---|---|---|
| I | Bonna fortress | 83 | fortress, amphitheatre, fabrica, canabae | drill at the post (palus), armatura duels, first making loops | the arena duel, ladder, glory, HUD |
| II | Taunus forest | 83 | march road, clearing, camp site, frontier line | march, make camp, forest ambush (the battle picture), cut the road, build towers and palisade | legion brain, orders, dig, works |
| III | Mogontiacum | 89 | winter fortress on the Rhine | a mutiny story: hold the gate, choose a side (the legion stayed loyal) | fortress map, living-world minds |
| IV | Danube | 101-105 | river banks, Tapae pass | pitched battle in triplex acies; build a pier of the great bridge | line combat, works on water |
| V | Dacian mountains | 106 | a stone fortress on a ridge | siege works, engines (scorpio, ballista), the falx and the arm guard | siege ledger, destruction |
| VI | Rome | 107 | the triumph route, the district, the Colosseum | march in the triumph; the games | district plan, crowd, arena |

Chapter facts (checked 2026-10-21):

- II: the Chatti avoided pitched battle and fought from the forest; Domitian pushed the frontier
  forward "along a stretch of one hundred and twenty miles", cutting into the forest (Frontinus,
  Strategemata 1.3.10). Wooden watchtowers on a forest road in the Taunus and Wetterau are claimed
  for Domitian, not proven. So the forest fight is a raid on a road-cutting party, not a field
  battle. Germanic kit: spears (frameae), painted shields, few swords, few helmets or breastplates
  (Tacitus, Germania 6).
- IV: Trajan's bridge at Drobeta, built 103-105: 20 stone piers, 150 ft high, 60 ft wide, 170 ft
  apart, arched (Cassius Dio 68.13). The player works one pier.
- V: Sarmizegetusa Regia fell in 106 and Decebalus killed himself. The falx (a two-handed blade
  on a haft of about 0.9 m) met Roman helmets with iron cross straps and the arm guard (single
  source; verify before it drives numbers).
- VI: the games of 107 ran 123 days with about 11,000 animals and 10,000 gladiators (Cassius Dio
  68.15); Dio does not name the venue, so the Colosseum is our choice.
- Kit of the period: Corbridge-type lorica segmentata, Imperial Gallic (Weisenau) helmet, Pompeii
  gladius (blade 45-50 cm), pilum about 2 m with an iron shank of about 600 mm, hobnailed caligae.

The storyboard for II-V is Trajan's Column: 155 scenes (Cichorius I-CLV); camp and rampart work
in scenes XIII, XLIX, CXXVIII (log ramparts) and XCIV, CIII, CVII, CXIV, CXXXI, CXXXIV (camp
fortifications), a river crossing in XXVI (Richmond 1935). Plates: Commons category "Trajan's
Column - Cichorius Plates" (138 files). The codex holds the scene list (CDX5). Chapter I and the
slice come first; the rest are LATER rows.

## 4. Systems

### 4.1 Body and view

FV1, FV2, FV5 landed; FV3 (eye feed), FV4 (no clip) and the POV rows (lens, mount, viewmodel
weight, second cameras, flash, comfort) come from `touch-and-pov.md`. New here: LEG2, the helmet in
view (a lit 3D rim mesh from an aperture outline table: galea with cheek guards, face mask with eye
holes) and LEG9, the HUD of the pictures.

### 4.2 Fighting in the line

- Melee: `crates/combat/src/fight.rs` v2 (parry, riposte, limb cut) carries over. The gladius is a
  thrust weapon (Vegetius 1.12: two inches deep kills), so the thrust under the rim is the main
  attack and the cut is weak.
- Shield: the boss push is a v2 action (LEG4); body pushes feed balance (TC3); a packed line has
  pressure (TC5) and men lean and brace (TC7).
- Formation: `crates/ai/src/legion.rs` (formations, shield wall, orders, thinking LOD) gets its
  first caller (LEG3). Slots: each man takes the nearest free slot and keeps his row and file; the
  slowest man sets the pace.
- Pila: two per man, thrown on the horn (LEG5), built on `crates/arena/src/spear.rs`. A pilum stuck
  in a shield weighs it down until it is dropped. The data file's two-hand pilum thrust
  (`weapons.rows`) is wrong and goes.
- Relief: the front rank tires, the horn sounds, ranks pass through (LEG11, LATER).
- Enemies: Chatti warband (spear, shield, few swords; a charge that breaks or dies on the line);
  Dacians (the two-handed falx that reaches over a shield, answered by the arm guard and the
  reinforced helmet).

### 4.3 Engineering

- **Dig layer (LEG6).** An integer height grid in mm owned by the tick. Dig and pile conserve
  earth (the ditch becomes the rampart). Walkers read slope and height; nav reads it. The slope
  limit and ground snap come from `crates/physics/src/kcc.rs` (float) ported to integer, then
  `kcc.rs` is deleted.
- **Works (LEG7).** A build job is data: a footprint, a material, a crew, progress per stroke,
  a ghost preview, snap points. Unfinished work is weak and shows it; more hands go faster
  (Holdfast's progress circle). Stakes, towers and palisade are props with colliders (TC2) whose
  joints break through the bonded destruction path.
- **Survey.** The groma lays out right angles, the chorobates checks level (Vitruvius 8.5: a 20 ft
  rod with a water channel), the hodometer counts miles. In game: the surveyor (an Immunis) places
  the camp grid and the road line; a misplaced line costs hours.
- **Camp plan.** From Hyginus and Polybius: gate facing the enemy, via principalis 60 ft wide,
  via quintana 40 ft, the rampart 200 ft from the tents; ditch and rampart sizes by threat (table
  in section 5). `crates/rome/src/camp.rs` turns the plan into works.
- **Bridges, roads, siege works, engines** are works too, with their own tables (LEG14-LEG16).

### 4.4 Making (rows MK)

Real methods, short loops, each one a timed process (`crates/world/src/process.rs`, genre-neutral:
inputs, tool, ticks, outputs, a failure rule) fed by `content/rome/tech/*.toml` with refs.

| Loop | Steps (sourced) | First version |
|---|---|---|
| MK1 bloomery and forge | shaft furnace 1.2-1.5 m, smelt 10-20 h, break out the iron sponge, reheat and forge (experimental archaeology, EXARC 2020) | time-skipped smelt, then 6-10 hammer beats per bar on the anvil |
| MK2 mend kit | rivet a lorica plate (lobate hinges, 5 rivets per half), regrind a gladius edge, replace hobnails | tool wear per stroke and per hit; mend restores it |
| MK3 timber | fell only from early autumn, cut to the heart and let the sap drain, then fell (Vitruvius 2.9); oak for underground work, alder for marsh piles | season gate; stakes and pila shafts from a log |
| MK4 lime and mortar | burn stone (it loses a third of its weight), slake hot, mix 1 lime : 3 pit sand or 1 : 2 river sand (Vitruvius 2.5) | kiln time skip, mixing ratio as a choice with a strength result |
| MK5 scutum | two glued plank layers, linen, then calfskin, an iron or bronze rim (Polybius 6.23) | build and re-cover; damage shows on the shield |
| MK6 crane and lifting | trispastos (3 pulleys), pentaspastos (5), treadwheel; three gangs on the ropes (Vitruvius 10.2) | crew count sets the lift speed; used in the bridge chapter |

Tools wear and break. A legionary carries saw, basket, pickaxe, axe, strap, sickle-hook and three
days' food (Josephus, Jewish War 3.95); the dolabra digs and chops.

### 4.5 Orders, sound and words

Horns carry orders: the cornu moves the standards, the tuba calls attack and recall, both sound
together in battle (Vegetius 2.22). The centurion's voice and the standard's lean back them up.
The player hears Latin commands with a subtitle (lexicon, CDX3). As Optio the player gives four or
five orders (move, hold, form line, loose pila, relieve), never per-man micro (Arma's command menu).

### 4.6 Daily life and discipline (LATER)

Four night watches of up to three hours, four men per century; the password changes daily and
the tesserarius carries it (Vegetius 3.8, 2.9; Polybius 6.35). Sleeping on watch earns the
fustuarium (Polybius 6.37). Crowns reward deeds (Gellius 5.6). Pay and grain from Polybius 6.39.

### 4.7 Scale

Near ranks (up to 64 bodies) are full fighters; the next ranks are LOD bodies from the legion
brain's thinking LOD; the far legion is the GPU crowd (`crowd_vat.wgsl`). The potato tier keeps
16 near bodies.

### 4.8 Online

Co-op contubernium: up to 8 players in one tent group, AI fills empty places, one hash (the
multiplayer plan's server-authoritative model). The arena duel stays the PvP mode.

### 4.9 Rare and contested kit, for history nerds (rows KIT)

Owner 2026-10-21: "historic rare weapons ... for history nerds, like they had even 1 man xbow with
small bolts, discovered lately". Research pass the same day: Haiku web hunt, with every source
fetched and read unless marked.

**The rule: every piece of kit says how sure we are.** Each item in `content/rome/kit.toml` carries
`certainty`, which is one of four values:

- `attested`: an object was found and its date is fixed.
- `reconstructed`: built from texts and pictures.
- `contested`: scholars disagree.
- `invented`: ours.

Each item also carries its refs, and the find (site, year found, date of the object) when there is
one. The in-game codex card shows all of it. A contested item can be played, and it says so on
the card. That label is the history-nerd feature: the game shows its evidence instead of
pretending.

**The manuballista (Greek: cheiroballistra).** The Roman one-man bolt-shooter: "hand ballista".
It is a small torsion engine. Two twisted springs of sinew or hair sit in an iron frame, and it shoots
short iron bolts in a flat line. It is the ancestor of the crossbow, but its power comes from torsion,
not a bent bow.

- **Text:** Hero's *Cheiroballistra* gives the build, though its author is doubtful.
- **Find:** frame parts from Xanten-Wardt. They were found in 1999 in a Rhine side arm, published
  in 2010 (Schalles, Xantener Berichte 18), and are dated to the mid 1st century AD. That is just
  before our years, so the legion's armouries plausibly had it.
- **Reported in reconstructions:** bolts of about 7 in (18 cm), shot at over 70 mph (31 m/s) with
  sniper precision. Its use in Britain and Scotland is linked to the Burnswark siege (c. AD 140).
  Owner's sources: Smithsonian Channel; YouTube reconstructions; Wiktionary *manuballista*.

In game it is a real weapon: the ballistarius (an Immunis) carries it. Its jobs are direct fire
from the rampart, picking off chiefs and standard-bearers, and covering a work party (KIT3). Its
big brother is the carroballista on its cart. The codex card shows the Xanten frame, Hero's text,
and the one open debate, hand-held or on a stand. Start numbers come from the reconstructions and
are rechecked against Schalles 2010 and Baatz 1991 when the codex holds them; the weapon does not
wait for that. The crossbow-shaped brooches (Whitton, found 2017; Hadrianopolis, 4th-5th c.) are
codex cards beside it.

In our years (AD 83-107) and usable:

| Item | Evidence | Certainty | In game |
|---|---|---|---|
| Carroballista | cart-mounted torsion engine on Trajan's Column scene XL (Dacian war, carved c. 113) | reconstructed | chapter V engine crew (LEG15) |
| Falx and sica | two-handed falx; one-handed sica about 41 cm (Adamklisi; Trajan's Column) | attested (type) | Dacian enemy kit (KIT2) |
| Manica and reinforced helmet | metal arm guard; helmet with cross-braced bowl (Adamklisi metope XVII, c. 109) | reconstructed | the answer to the falx (KIT2) |
| Named shield boss | boss punched "Junius Dubitatus", legio VIII Augusta (River Tyne, early 2nd c.) | attested (no published reference found yet) | punch your own name on your kit (KIT4) |
| Inscribed sling bullets | lead glandes with taunts and unit names, about 1,000 casts catalogued (Berlin, 2021); no fixed date | contested for our years | auxiliary slingers; a found bullet is a codex card (KIT5) |
| Caltrops (tribuli) | iron caltrops from Tyras (2nd to early 3rd c.) and Caerleon (3rd c.) | contested for our years | camp field traps beside Caesar's lilia (KIT5) |
| Manuballista (cheiroballistra) | Xanten-Wardt frame (found 1999, mid 1st c.); Hero's *Cheiroballistra* | attested (hand-held or on a stand: debated) | the ballistarius's one-man bolt-shooter (KIT3) |

Outside our years. These are codex cards only, never kit:

- plumbata (3rd c. on)
- arcuballista (Vegetius, late 4th c.; no object)
- Harzhorn bolt cluster (AD 235)
- Burnswark drilled sling bullets (c. AD 140; the "whistling" is a guess)
- Judean cave spathae (AD 132-135)
- Kalkriese lorica and face mask (AD 9; the armour type fits, the find does not)
- Haltern inlaid pugio (early 1st c.)
- the Pompeii polybolos holes (89 BC)
- gastraphetes and corvus (Greek and Republican)

Leads to check before use. These come from memory and were not fetched:

- cheiroballistra (manuballista) arch pieces from Danube forts (Orșova, Gornea), which would sit in chapter IV's
  ground if their date fits;
- the original Xanten-Wardt and Baatz papers (the Germania review blocked the hunter).

## 5. Numbers (start values; each cites its source; code reads them from `content/rome/`)

Units: 1 Roman foot about 0.296 m; 1 Roman mile = 1,000 paces, about 1.48 km.

| Thing | Value | Source |
|---|---|---|
| Tent group | 8 men (early Imperial); Vegetius gives 10 for his day | modern consensus; Vegetius 2.13 |
| Century | 80 men under a centurion and an optio | modern consensus; Vegetius 2.14 gives 100 |
| Officers | centurio, optio, signifer, aquilifer, cornicen, tubicen, tesserarius | Vegetius 2.7-9, 2.13, 2.22 |
| Regular step | 20 Roman miles (about 29.6 km) in 5 summer hours | Vegetius 1.9 |
| Full step | 24 Roman miles (about 35.5 km) in 5 summer hours | Vegetius 1.9 |
| Pack load | 60 lb (about 20 kg) besides arms, recruit drill | Vegetius 1.19 |
| Frontage | 3 ft (about 0.9 m) per man | Vegetius 3.14 |
| Rank gap | 6 ft between ranks | Vegetius 3.14-15 |
| One-night camp ditch | 5 ft wide, 3 ft deep | Vegetius 3.8; Hyginus 49 |
| Ditch, light threat | 9 ft wide, 7 ft deep | Vegetius 1.24 |
| Ditch, serious threat | 12 ft wide, 9 ft deep | Vegetius 1.24 |
| Rampart | 8 ft wide, 6 ft high, turf | Hyginus 50; Vegetius 3.8 |
| Camp streets | via principalis 60 ft, via quintana 40 ft | Hyginus 14, 17 |
| Who digs | each century its own stretch | Vegetius 3.8 |
| Watches | 4 a night, up to 3 hours each | Vegetius 3.8; Polybius 6.35 |
| Gladius | thrust; two inches deep kills | Vegetius 1.12 |
| Pilum | shaft 5.5 ft, head 9 inches; effective throw 15-25 m, max about 30 m (modern tests) | Vegetius 2.15; modern tests |
| Rhine bridge (Caesar) | piles 1.5 ft thick, 2 ft apart in pairs, built in 10 days | Caesar BG 4.17 |
| Alesia works | 20 ft ditch, 12 ft rampart, towers every 80 ft, 5 rows of cippi, 8 rows of lilia 3 ft deep | Caesar BG 7.72-73 |
| Engines | 55 carroballistae and 10 onagri per legion (late army) | Vegetius 2.25 |
| Ballista spring hole | 2 lb shot: 5 digits; 10 lb: 8; 40 lb: 12 9/16; 80 lb: 15 | Vitruvius 10.11.3 |
| Scorpio | spring hole = 1/9 of the arrow length | Vitruvius 10.10.1 |
| Aqueduct fall | half a foot per 100 ft (1:200) | Vitruvius 8.6 |
| Mortar | 1 lime : 3 pit sand, or 1 : 2 river sand | Vitruvius 2.5 |
| Crane | trispastos 3 pulleys, pentaspastos 5; three gangs | Vitruvius 10.2 |
| Hodometer | 4 ft wheel, one pebble per mile | Vitruvius 10.9 |

Not found or unsure (do not use yet): stakes per man, camp build hours (modern guesses 2.5-4 h),
pilum volley timing, tool weights, gladius forging times, road thickness and camber.

## 6. Tools for us (authoring)

- The codex (`rome-codex.md`): fetch, index, extract, `codex.search` and `codex.cite`.
- Door verbs over the Rome crate: `rome.camp { site, threat }` lays out a camp from the Hyginus
  rules on the dig grid and returns the works list; `rome.road { from, to }` lays a road on the
  height grid; `rome.scene { column_scene }` stages a set piece from a Column scene card. Each
  returns a capture and a receipt, and each is undoable (ROM5).
- Reference boards per object (helmet, gladius, scutum, dolabra) from CC0 museum images (CDX5).

## 7. Steals

| Steal | Source | What we take |
|---|---|---|
| Formation slots | 0 A.D. `Formation.js` (GPL, idea only); Total War group formations doc | nearest free slot, keep row and file, the slowest sets the pace, a catch-all block |
| Build progress | Holdfast sapper builds (wiki) | progress per hit, unfinished is weak, more hands faster, deconstruct refunds |
| Ghost preview | PlanBuild, Valheim mod (WTFPL) | a ghost plan, real pieces snap to it |
| Smooth ditches | fast-surface-nets-rs, block-mesh-rs (MIT/Apache) | mesh the dig grid; dig subtracts, pile adds |
| Readable melee | For Honor deep dive; Chivalry 2 FAQ | committed swings, a clear tell for every action, death shows why |
| Directional hits | Unreal-Melee-Combat-System (MIT) | hit reaction by direction, blocked-hit variants |
| Orders | Arma 3 command menu | four or five orders, never micro |
| Camp loop | We Are Legion: Rome, Summa Expeditionis, Legatus (commercial references) | camp life between fights; the dusk camp |

## 8. What changes in the old Rome plans

| Old | Now |
|---|---|
| Gladiator hero in the Colosseum (`CONCEPT.md`) | Chapter I armatura and chapter VI games; the duel is training |
| Ranks Tiro, Gladiator, Primus, Champion, Legend | Tiro, Miles, Immunis, Optio, Centurio |
| Ludus hub | the fortress (fabrica, barracks, amphitheatre) |
| Rome district | chapter VI and leave days |
| Siege field | chapter V |
| Repair-craft LIGHT | making loops MK1-MK6 (owner 2026-10-21 lifts "crafting" to the soldier's real crafts; full economy stays CUT) |
| Third-person hero cam | first person through the helmet |
| Pilum as a two-hand thrust | a thrown javelin |
| No sources | every number cites the codex |

### 8.1 The 15 fixes (ROM1)

Rule: the `content/design/*.rows` file wins and the prose changes; where no data file
exists, the newest owner decision wins (decision 7 in section 0). No data file changed
in this pass: only prose. Each row names the line the prose now matches.

| # | Item | Before | After | Winning line |
|---|---|---|---|---|
| 1 | Ladder rungs and ranks | the rung table took tier and rank from the Rust constants: tier 4 on rung 7, names "T1 recruit" and "T3 champion", the last rung ranked Champion to Legend | the rows file seats tier 1,1,2,2,2,3,3,3 with the boss flag on rung 7 and rank 0,0,1,1,1,2,2,3; the code seat (tier 4) is named as the open gap | `content/design/ladder.rows:25-32`, `content/design/loop.rows:9` |
| 2 | Loss penalty | a lost bout drained "the penalty column", 25 | a lost bout drains a flat 50; the `penalty=25` column is lint-bounded only | `content/design/loop.rows:9`, `crates/game/src/ladder.rs:26` |
| 3 | Sword damage and reach | 25 HP damage, 0.5 s cooldown, 200 cm range, with a landed Rome Tables preset of 250 permille and 30 ticks | the engine does 120 permille damage, 15-tick startup, 8-tick recovery, blade reach 1500 mm; the forge 25 HP / 30 ticks / 2000 mm stays forge-parity data; the Rome Tables preset stays planned only | `content/design/combat.rows:9`, `content/design/weapons.rows:24`, `crates/arena/src/match_wire.rs:33` |
| 4 | Dodge | "dodge distance 2 m" | dodge 10 ticks with 6 i-frame ticks; no dodge impulse exists, so no distance is claimed | `content/design/combat.rows:9` |
| 5 | Tier names and HP | "T1 recruit, T2 veteran, T3 champion, T4 primus"; Holder and Flanker both at 1000 HP; band 900-1100 | grunt, veteran, elite, primus; Holder 950 HP, Flanker 1000 HP; band 900-1200 | `content/design/enemies.rows:10-13`, `content/design/enemies.rows:6`, `content/design/enemies.rows:23-24` |
| 6 | Reaction delay | scripted players carry "a reaction delay of 12-18 ticks" | no 12-18 tick delay exists: the tier reaction is 4/2/1/1 ticks, the scripted player carries 2-4 tick noise and acts once per 8 ticks | `crates/ai/src/depth.rs:295`, `crates/playtest/src/winrates.rs:45,49`, `content/design/enemies.rows:16` |
| 7 | Hit stop | "light 3, heavy 6, parry 8 ticks" only | the same per-kind numbers, plus the shipped flat `hitstop_ticks=4` of the v2 row | `content/design/combat.rows:9`, `crates/combat/src/tables.rs:107,116,125` |
| 8 | Zone radius | centre r under 3 m, ring 3-8 m, wall band over 8 m, from the code bands only | the layout rows seat the centre circle r 250 cm and the gate mouth r 200 cm; the 3 m and 8 m bands stay the code classifier | `content/design/arena-zones.rows:16`, `content/design/arena-zones.rows:9`, `crates/arena/src/zones.rs:17,19` |
| 9 | Entrance bar | "entrance_share under 25%" | the tournament holds the gate mouth under 250 permille of fighter ticks; the level check keeps its own floor-area band of 25..60 | `crates/playtest/src/tournament.rs:1980`, `docs/design/LEVELS.md:174` |
| 10 | Camera recording | the camera is "recorded in-sim" | the camera state is presentation only: the sim records the hits, the windowed leg feeds the trauma in, the shake never feeds back | `crates/game/src/camera.rs:1020` (no rows file; the code seat wins) |
| 11 | Rope damping | `v -= v / 32` per tick | `v -= v / 64` per tick, as the swing code and the landed status note already said | `crates/arena/src/spear.rs:82` |
| 12 | Spear gravity | the swing used "the same `SPEAR_GRAVITY` step the flight uses" | the swing falls with `FIGHTER_GRAVITY` 11 mm/tick², the spear in flight with `SPEAR_GRAVITY` 3 | `crates/arena/src/lib.rs:171`, `crates/arena/src/spear.rs:38` |
| 13 | First-fight bar | the first bout starts within 60 s of launch | 45 s target with a 60 s cap, owned by the ladder code; the cap is not a lint rule | `content/design/ladder.rows:13-15`, `crates/game/src/ladder.rs:49,51` |
| 14 | Stale MISSING lines in `gameplay.md` | the section 1 status table called tiers, the tier bar, the ladder, the debug camera and more MISSING, BUILT or dead | each row carries LANDED or PARTIAL with the code that proves it; GA14, GP33 and GA15 stay open | board rows GP27-GP42 and GA13-GA15 (`docs/SPRINT-20-game.md`) and the code each row cites |
| 15 | Wrong line cites | cites to moved code lines (`depth.rs:460,433,463,585,281,289,298`, `fork.rs:83,262,148`, `tournament.rs:1073`, `assistant.rs:1108`) | every cite points at the line that holds the symbol now (`depth.rs:449,440,453,571,287,295,304`, `fork.rs:85,366,271`, `tournament.rs:1096`, `assistant/fork.rs:274`) | those code lines |

## 9. Order

1. **M0 slice (now):** LEG3, LEG5, LEG6 and CDX1-CDX2 start at once; LEG1, LEG2, LEG9 after FV3
   and POV1; LEG4 after TC3; LEG7, LEG8 and LEG10 close it. Three minutes: march, dig, hold.
2. **M1 chapter I:** ROM2 (`crates/rome`), ROM3 career, MK1-MK2, the amphitheatre as drill.
3. **M2 chapter II:** the forest map, the frontier works, the ambush.
4. **M3-M5:** Danube bridge, siege, Rome (LATER rows lifted one at a time).

## 10. Not now

A seamless world, a campaign map, politics, an economy beyond the camp, naval chapters, horses
for the player, thousands of physical bodies.
