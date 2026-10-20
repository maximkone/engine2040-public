# RYL1 → engine2040: preparation note

Owner order 2026-10-20: "risk your life 1 not 2 — the second one was bleh".
Status: research only. No plan file, no board rows (D7 freezes new plan files).
Written by an agent; uncommitted.

## What RYL1 is (observed)

- 3D fantasy MMORPG, release 2005-01-05, publisher Youxiland Digital.
  Source: mmos.com review (read 2026-10-20).
- Two races at war: Humans and Ak'Kan.
- 15+ classes, branching at level 10 from Archer, Cleric, Sorcerer, Gunner,
  Rune, Shadow. Source: same review.
- Real-time combat with no auto-target. Open PvP and guild-vs-guild war,
  "waging war between camps, developing territory for PvP". Same review.
- Old-school grind, keyboard or mouse control, low population today.

## The correction to "died fully"

RYL1 has private servers. RaGEZONE hosts RYL1 server files and clients
(v557, v600 MY; MSSQL 2000, `DemonSetup.ini`, `RYLServerController`), and
"RYL 1 Nexus" runs a non-profit classic server on a v600 MY base.

So the gap is **quality and reach**, not existence. A recreation competes with
nostalgia servers. If it is only "the old game again", players stay where their
characters already are. It must keep the soul and be a better game to play in
2026.

## The soul — five things worth keeping

1. Two races that are not mirror images. Identity first, balance second.
2. No auto-target. Every hit is aimed.
3. The map is the endgame: camps, contested zones, guild claims.
4. Class branch at level 10. A build is a commitment.
5. Visible power gaps. Social hierarchy is the content.

## What to cut

Tab-target spam, 2004 UI, walking simulators, upgrade-failure RNG loops,
private-server pay-to-win, 300-hour grind walls. Compress the grind to days,
not months.

## Where it lands in this engine

| RYL1 system | engine2040 today | gap |
|---|---|---|
| aimed melee, parry, riposte | `crates/combat`, v2 exchange (parry/riposte/clash) | ranged and magic projectiles in the tick |
| classes and skills | `crates/rules` (Rhai sandbox, hashed) | class sheets as data, cooldown and range bands |
| territory war | `crates/world`, `scene`, `spatial` | capture point state machine, claim rules |
| 6v6 to 32v32 | `crates/net`, `engine-net` (QUIC, rollback, snapshot) | scale beyond the duel |
| characters that persist | `crates/save` | account and character records |
| AI filling slots | `crates/nn`, `ai`, `nav` | faction-aware legion behaviour |
| HUD and menus | parked (UI) | minimum: health, cooldown, capture bar |

## First slice — "Territory Duel"

Not a game. One proof that the soul works.

- Two factions, 6v6, four seats filled by AI.
- Three classes: Knight (melee), Archer (projectile), Sorcerer (magic bolt).
- One map, one capture point, ten minutes, hold to win.
- Windows and browser, same hash, replay with no model.

It reuses the arena proof machinery (hash law, replay, snapshot) and adds only
the three things RYL1 needs: aimed projectiles, class data, territory state.

Draft rows, for the owner to board later (not on `docs/SPRINT.md` yet):

- RY1 content: two faction kits, one weapon set each, one capture point prop.
- RY2 rules: three class sheets in data, cooldowns and range bands, hashed.
- RY3 combat: projectile and magic bolt through `physfx`, predicted, replayable.
- RY4 world: capture point state machine and claim tick.
- RY5 net: 6v6 snapshot budget with four AI seats.
- RY6 proof: replay hash equality, two runs, browser equal to native.

## The legal line

- No RYL1 assets, maps, textures, audio, class names or race names.
  "Ak'Kan" is theirs; rename it.
- No leaked server files, no decompiled client code. Clean room.
- D5 lets donor *code* be ported. It does not cover art or IP. Different rule.
- Publish the design, not the client.

## The decision only the owner can make

D7 freezes new plan files until the first real frame from `crates/render`
lands, and this would be game three. Three ways forward:

1. Keep it as this note. No rows.
2. Unfreeze one plan file for RYL-shaped work, one slice only.
3. Fold the first slice into the existing arena as a "faction mode" row.

Pick one and the work starts.
