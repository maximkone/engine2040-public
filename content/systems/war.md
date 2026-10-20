# Bannerfall — Statue War

Data lives in `content/systems/war.json`. This page is the rule book behind
those numbers. If the two disagree, the JSON wins and this page is wrong.

Clean-room from `docs/research/ryl1-2026-10-20/dossier.md` §5. The loop is the
old one. Every name, number and place here is ours.

## The loop

Two factions, one contested middle, three statues. Rivenmarch holds them. The
wardens come from Wrenport, the ashkin from Cinderhold, and everybody who
logs in before the bell knows the shape of the next hour: take a statue, hold
it, earn the buff, then take the next one before the field resets.

One window, in order.

1. **Call.** At the window's start hour all three statues go neutral
   (`faction: "neutral"`, meter 0). Forts do not reset. The HUD shows the
   clock, three bars, two point counters.
2. **Take.** A faction standing on a statue with no enemy on it drives the
   capture meter. The meter's sign says who is taking it (positive wardens,
   negative ashkin); its magnitude is progress toward flipping the statue.
3. **Hold.** The owner of a statue gets the buff: +10% XP, +10% drop. The buff
   is live while your faction owns the statue and fades `duration_min` = 2
   minutes after you lose it — the grace that lets a losing team fall back
   without the reward vanishing mid-fight. Any player of the owning faction
   standing in the statue's radius earns 1 honour per minute.
4. **Score.** Every 10 seconds a statue gives its owner 1 point. Points come
   from statues, never from players.
5. **Bell.** After 60 minutes the window ends. Hold 2 of the 3 statues at the
   bell and you win the window. A 1-1-1 split is broken by points. A tie is a
   tie: nobody wins, everybody keeps its honour.

Fame comes only from inside a window (`war_kill`, `node_capture`). Nothing
outside the war gives fame. That is what makes the hour worth the hour.

Death is 30 seconds back in the fight. Default respawn is the faction spawn
hall; a guild holding a fort with the war banner respawns at its own fort.

## The day: four windows, one hour each

| window | starts (server local) | ends |
|---|---|---|
| 1 | 10:00 | 11:00 |
| 2 | 14:00 | 15:00 |
| 3 | 18:00 | 19:00 |
| 4 | 22:00 | 23:00 |

Why these hours: every 4 hours, so a player can plan the day around one of
them; no window crosses midnight (the last ends at 23:00), so "a day" is
unambiguous for the reset and the scheduler never needs a wrap-around case;
3 hours between windows, so the field is never always-on; 18:00 and 22:00 land
after work. `local_hours` is the server's local clock, not UTC — the
deployment picks the zone and the file changes with it.

## Temporary and persistent: the asymmetry is the point

| | statues | forts |
|---|---|---|
| lives | one window | forever |
| owner at window start | neutral | the guild that built it |
| pays | points, honour, stars, fame | respawn, market, mine, land |
| costs | an hour of your night | materials, every day |

If the statues persisted, the war would be won before the bell and the map
would calcify: the same ten guilds would hold the field and a newcomer would
never touch a statue. If the forts reset too, the war would be a sandbox with
no stakes — an hour of fighting for numbers nobody keeps. So the
**scoreboard resets and the land does not**. A fort is what a guild builds
over weeks; a statue is what a guild wins in an hour. The war is the reason to
care about the fort, and the fort is the reason the war still matters on a
Tuesday.

Forts: a guild master buys a starter kit (30 materials) and the fort starts
at level 1. Upgrades cost 60 and 100 materials. Upkeep is 3 / 6 / 10
materials a day; a fort that cannot pay drops a level, and level 0 is rubble
with no flag. The mine works once a week and yields 60 / 130 / 220 materials
by level (net 39 / 88 / 150 a week after upkeep). A fort may not be built
inside another guild's fort claim.

## Numbers

| thing | value | where it lives |
|---|---|---|
| windows a day | 4 x 60 min | `windows[0].count_per_day`, `duration_min` |
| window start hours | 10, 14, 18, 22 local | `windows[0].local_hours` |
| node tick | 10 s | `scoring.node_tick_seconds` |
| points per node per tick | 1 (6/min, 360/hour per node) | `scoring.points_per_node_tick` |
| win condition | hold 2 of 3 statues at the bell | `scoring.win_nodes` |
| points tiebreak | only used in a 1-1-1 split | `scoring.points_per_node_tick` |
| respawn | 30 s | `scoring.respawn_seconds` |
| capture meter | 30 to flip a statue | `nodes[].capture_progress` |
| capture rate | 0.5 progress/s per player, 5 players max per faction (2.5/s) | derived |
| meter decay | 0.5 progress/s while nobody is taking the statue | `nodes[].decay_per_second` |
| contested | meter frozen: no gain, no decay, owner unchanged | derived |
| solo flip | 60 s (one player, 6 ticks) | derived |
| squad flip | 20 s (5 players, 2 ticks) | derived |
| claim bleed-out | 60 s from full meter to zero | derived |
| capture buff | +10% XP, +10% drop, live while owned, 2 min grace after a loss | `nodes[].buff` |
| honour | 1 per minute standing in your own statue | `honour.per_minute` |
| stars | 5 max, 10 honour each, +5% lucky drop each (max +25%) | `honour.stars`, `honour.lucky_per_star` |
| fame | 2 per war kill, 5 per node capture, nowhere else | `fame.from` |
| fort | kit 30 materials, levels 30/60/100, hp 20k/50k/100k, upkeep 3/6/10 a day | `forts.*` |
| fort flags | war banner (respawn), market stall (5% tax), mine (weekly) | `forts.flags` |
| guild cap | 50 members | `limits.guild_cap` |
| field | 64 players per window, 40 max per faction | `limits.max_players_per_window` |
| friendly fire | off | `no_friendly_fire` |

Capture meter, exactly as the tick computes it. Per statue the state is one
owner (`neutral` / `wardens` / `ashkin`) and one signed integer (the
challenger's progress, -30..30). Each 10-second tick:

- both factions present -> contested: meter frozen, owner unchanged.
- only one faction present, and it is not the owner -> that faction
  challenges. A challenger change resets the meter to 0. Meter gains 5 x
  players (max 5 players) per tick. At 30 the statue flips to the challenger
  and the meter returns to 0.
- only one faction present, and it is the owner -> nobody is challenging, the
  meter decays 5 per tick toward 0.
- nobody present -> the meter decays 5 per tick toward 0.

Ownership is sticky inside a window: only a flip or the window reset changes
it. Decay bleeds a half-made claim, never a held statue.

## Three rules that keep a 64-player window readable

1. **The score reads statues, not players.** One owner and one signed integer
   per statue; the tick awards points from statues. Sixty-four players cost
   the same to score as six, and the snapshot carries three integers, not
   sixty-four truths.
2. **A statue pays its faction, not its crowd.** The buff is owner-wide (+10%
   XP, +10% drop), and the capture rate is capped at five contributing players
   per faction per statue. A zerg cannot flip faster than a squad, and no
   player can stack a private multiplier.
3. **Contested means frozen.** With both factions on a statue the bar stops:
   no progress, no decay, no owner change. A fight for the third statue always
   reads as a fight, never as a coin flip. Friendly fire is off, so the only
   damage in the field comes from the other faction.

## What a headless test can check

- `windows[0].local_hours` has 4 entries, all in 0..23, all at least 2 hours
  apart, and `local_hours.length === count_per_day`; the last window ends
  before midnight.
- 1 warden alone on a neutral statue: owner flips to wardens after 60 s
  (6 ticks x 5 progress), meter back to 0.
- 5 wardens: flip after 20 s (2 ticks x 25 progress).
- A challenger that leaves: meter 30 -> 0 in 60 s (6 ticks x 5 decay).
- Both factions on the statue: meter and owner unchanged over any number of
  ticks.
- One statue held by wardens for 60 minutes: 360 points. Two statues: 720.
- Window ends with 2 statues wardens, 1 ashkin: wardens win.
- Window ends 1-1-1: more points wins; equal points, no winner.
- Honour for 60 minutes standing in your own statue: 60, which buys 5 stars
  at 10 honour each and +25% lucky drop.
- Fort level 1 pays 3 upkeep a day and yields 60 a week: net 39 a week.
- `no_friendly_fire` is true and `fame.from_other_sources` is false.

## Numbers I am not sure about

- The buff is 2 minutes, not the 2 hours the source game gave. Short buff,
  long ledger: honour and stars persist, the +10% does not. If the owner
  wants the old generosity, `nodes[].buff.duration_min` is 120.
- Capture rate and the 5-player cap are derived, not sourced. They are the
  reason rule 2 holds.
- Fort hp is written as 20k/50k/100k at an assumed 100 damage a hit. The
  invariant to keep is the hit count: 200 / 500 / 1000 clean hits. Rescale
  hp when the class sheets land.
- The weekly mine yields (60/130/220) and the daily upkeep (3/6/10) are
  balance guesses against a material economy that does not exist yet.
- Local hours are the server's local clock; the deployment timezone is not
  in the file.
- Fame here is war-only (`from_other_sources: false`). The economy page
  lists boss kills and bounties as fame sources too: one of the two has to
  give.
