# RYL1 dossier — ten-agent sweep, 2026-10-20

Sources were read, not remembered. Grades: [obs] = read in a source,
[inf] = reasoned from a source, [unk] = could not confirm. Full URLs sit with
each section. No code, no assets, no download links: this is a facts sheet for
a clean-room game.

## 1. The shape of the game

- Developer GamaSoft (KR); Youxiland (TW) licensed it region by region; Planetwide Games took North America.
  https://www.gamespot.com/articles/ryl-path-of-the-emperor-launches/1100-6127950/ [obs]
- Korea: open beta Oct 2002, commercial 2003 on Hangame, ended 2005; then GamaSoft self-published "REAL RYL". [obs]
- North America: retail 21 June 2005, closed September 2006 — the stated reasons were illegal private servers and a dispute with Youxiland.
  https://forums.mmorpg.com/discussion/95609/the-end-of-ryl-pote-i-think-so [obs]
- RYL2 shipped 21 Dec 2004 as a rework: three nations became two, gear and refine became the whole game. Players call it the worse game. [obs]
- RYL1 and RYL2 content were later merged into Return of Warrior (2008). Youxiland says it "owns ROW in full".
  https://www.4gamer.net/games/004/G000470/20080911053/ [obs]

## 2. IP status — read this before naming anything

- **ROW still runs officially.** rowplayon.com and roworigin.com were live on 2026-10-20, Youxiland copyright, selling cash items. [obs]
- Youxiland holds Taiwan trademark 01976785 (class 41, online games, filed 2018, expires 2029).
  https://tm.twincn.com/item.aspx?sn=7552883188 [obs]
- So "abandonware" is false. The name, the marks and the assets are owned and defended.
- Source was never released. Leaked server builds and DB dumps circulate on RaGEZONE since 2013. [obs]

Consequence: `ryl1.md` cannot ship the name "Risk Your Life", "RYL", "Return
of Warrior", the race names (Ak'Kan, Merkhadia), the zone names, or any art.
The loop is fair game. The skin is not.

## 3. Classes — 6 starters, 14 advanced

- Humans: Fighter → Defender (shield tank) or Warrior (dual-wield); Rogue →
  Assassin (stealth burst) or Archer (ranged); Mage → Sorcerer (nuker) or
  Enchanter (debuff); Acolyte → Cleric (heal) or Priest (party buff).
  https://www.ryl1.my/txt_guide_cls_h.html [obs]
- Ak'Kan: Combatant (male only) → Templar, Attacker or Gunner;
  Officiator (female only) → Rune, Life or Shadow Officiator.
  https://www.ryl1.my/txt_guide_cls_a.html [obs]
- Gender-locked starting classes on one race only. Weird, memorable, and a
  design choice worth stealing as a *faction trait*, not as a lock.
- Skills are learned from books that drop, with five named ranks and a point
  cap raised by quests. https://www.gamespot.com/reviews/ryl-path-of-the-emperor-review/1900-6128941/ [obs]
- Signature low-tier skills per class exist and are mostly utility, not damage:
  Net, Pain Sharing, Vampire Kiss, Psalm of Light, Weapon Potentiation. [obs]

## 4. Stats and gear

- Five stats: STR, DEX, CON, INT, WIS. 5 points at creation, **2 per level**;
  a primary stat costs 2 points per +1, a secondary 1. [obs]
  https://www.rylreturn.com/home/guide_09.html
- Each class auto-grows two primary stats per level. [obs]
- Max level 100 in the shipped XP table; servers hex-edit the gameserver to go
  higher (104→125 is the documented patch). [obs]
  https://forum.ragezone.com/threads/ryl-1-set-max-level-126.626637/
- Upgrade caps at **+5** in RYL1, then sockets open. Failure can drop a level
  and burn max endurance. Metals: Silvin < Mithril < Iternium < Soul Metal. [obs]
- 14 gem types in four sizes, one per socket. [obs]
- Item grades: F, D, C, B, A, AA, AAA, derived per stat from its distance
  above the F base. https://forum.ragezone.com/threads/ryl-grade-equation.981324/ [obs]
- Death costs equipped-gear endurance, and low endurance starts eating **max**
  endurance permanently. No XP loss is documented. [obs]

## 5. War — the actual endgame

- **Statue War**: four one-hour wars a day. Three statues in the Arena go
  neutral at start, then are destroyed and possessed. Each statue pays
  +10% XP and +10% drop for 2h, plus honour medals (1/minute) and up to five
  stars of +5% lucky-drop chance.
  http://rowplayon.com/ROW/Page/GameSystem/Main.jsp?page=StatueWar [obs]
- **Guild war**: declared in the guild panel (cautious war / enemy), one guild
  at a time, peace needs both sides. Wars run Sundays for one hour; joiners
  are locked in Caernarvon and ported to Edinburgh Castle. [obs]
- **Fortress**: a guild master buys a Starter Kit near Pirate Town, builds a
  fort outside other guilds' fields; it becomes respawn, taxed shop or mining
  machine. Forts **persist between wars** and mine weekly. [obs]
- Statues reset each war; forts do not. That is the asymmetry that made the
  map matter.
- Guild cap 50; one shared Caernarvon battlefield; no published headcount. [unk]

## 6. World

- Human start: Aldia Port on South Coast. Ak'Kan start: Opeiny City on Leihan.
  Advanced islands: Sainna (Human), Misery (Ak'Kan). Middle: **Caernarvon**,
  holding Edinburgh Castle.
  https://web.archive.org/web/2007/http://www.ryl.com.my/txt_map1.htm [obs]
- Mirror design: two starter valleys, one contested middle everyone fights
  over, bosses worth farming in the open world. No instanced dungeons found.
- Questing is thin: 8 quests on South Coast, 7 on Leihan, and the 2007 guide
  says "currently no quests" on both advanced islands. [obs]
- Grinding was camping fixed spawn spots (LeyHan Spot, GPV, spirit spots). [obs]

## 7. Tech stack (for protocol study only, never for reuse)

- Client v557 and v600 (v600 is the live RYL1 base). [obs]
- Six server binaries started in fixed order: Login → UID → DBAgent → Auth →
  Chat → one GameServer per zone, driven by `RYLServerController.exe`. [obs]
- MSSQL 2000, `DemonSetup.ini` holds the whole topology; client version and
  checksum must match or login refuses. [obs]
- `rylCoder 2.7.0` edits `.gsf`/`.mcf`; `rMap` edits zones; `rDec` decompiles
  scripts (2026). [obs]
- Formats: `.GSF` = LZO block + per-file XOR from a 40-byte table; `.MCF` =
  XOR 0x425529AC then VM bytecode; meshes `.Z3M`/`.Z3AM`; textures plain DDS.
  https://github.com/Ap0dexMe0/rDec [obs]
- Client is packed and hooked to nProtect GameGuard; no public RYL1 GSF
  decryptor exists. [obs]

## 8. The scene in 2026

- RYL1 is a handful of servers, not a scene. Of 200 toplist entries, every
  RYL1 domain checked was dead except a few. [obs]
- RYL Praetorian: ~35 concurrent (Akhan 20 / Human 15), cap 95, Season 2. [obs]
- RYL 1 MY: closed beta on Olympus/Hades/Zeus, GMT+8. [obs]
- RYL 1 Nexus and Torment: domains dead. [obs]
- Every live server is leaked official binaries, not a rewrite. Changes are
  uniform: 300-1600x exp, auto-level, books and metals sold by NPCs. [obs]
- Complaint that repeats: "don't make you get to 95 once you log in" — instant
  cap kills the game. [obs]

## 9. What players say the soul is

Keep: real-time PvP feel; class/race choice that matters; race war and guild
war as ritual; player shops and mats as currency; group grind as social life;
open-world PK with red names.

Fix: unmodernised 2005 client, mob grind, pot-chugging duels, RNG upgrade tax,
cash-shop power, servers that wipe.

## 10. Rules for Bannerfall

1. Two mirrored starter valleys + one contested middle. Copy the geography,
   not the names.
2. A timed, repeating, *scheduled* war (Statue War) is the heartbeat. Four
   windows a day beats "always on".
3. Forts persist, statues reset. Persistence is what makes a map a home.
4. Class branch at 10, permanent for a season. Six starters, fourteen ends.
5. Upgrade capped low (+5) and sockets, not infinite refine.
6. Death costs gear condition, not XP. Punish the gear, not the evening.
7. No auto-level. The complaint about every live server is instant cap.
8. Name, races, zones, art: all new. Mechanics only.
