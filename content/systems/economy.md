# Bannerfall — economy

Data lives in `content/systems/economy.json`. This page is the rule book behind
those numbers. If the two disagree, the JSON wins and this page is wrong.

**Hard rules.** No pay to win. No cash shop, no bought currency, no real money
in any shape. No auction house and no player-to-player market: every trade goes
through a fixed-price shop, so no one can outbid, underpay or farm a price.
Stats are never sold. The only thing a real wallet buys is nothing.

## The loop

1. **Start in your valley.** Wardens in Frostmarch Valley, Ashkin on Emberstrand
   Coast. Keep, market and smithy on both sides, same prices, same quests. The
   mirror is the point.
2. **Work the valley.** Mobs, elites, one boss, bounties and the quest board,
   levels 1 to 20. This is where coin and level come from. Nothing here is
   contested.
3. **Spend on the gear.** Repair what killed you at the smithy, buy metal, step
   a piece up to +5, then socket it. This is where coin leaves.
4. **Walk into the middle.** The dusty bowl, levels 21 to 60: the ruined keep,
   the caravan posts and three statues. Four war windows a day. Statues go
   neutral, then are taken and held. War pays honour for being there and fame
   for winning.
5. **Build a home.** Forts persist between wars; statues reset. A guild that
   keeps a fort through the week earns the map. Outfitting that fort is the
   long-tail sink that keeps coin moving after the cap.

Forts persist. Statues reset. That asymmetry is the whole reason the middle is
worth taking. A fort's market stall takes 5 percent of every sale made there,
so the guild that holds ground also taxes it.

## Currencies

| Currency | Earned from | Spent on | Pace |
|---|---|---|---|
| Coin | kills, elites, bosses, quests, selling loot, war windows | the five sinks below | ~1,110 per hour |
| Honour Medals | war presence (60/hour), war kills, objectives, guild wars | war respawn, camp supply, war gear, fort flags | ~260 per war hour |
| Fame | war kills and node captures, and nothing else | titles, guild hall slots, banner wrapping, dyes | one title every ~4 war hours |
| Ember Marks (seasonal) | season goal tiers, war attendance, faction war wins | seasonal titles, sets, craft kits, cache, free season track | ~4,440 a season, 555 a week |

Honour is pay for showing up: one medal a minute in a window, whatever
happens. Fame is pay for winning: kills and captures only. Ember Marks are the
only currency that dies at the season boundary. Everything else is permanent.
That is what makes a season worth playing without ever asking a player for
money.

## The five sinks

Coin has five kinds of drain. Nothing else eats coin, so a player always has
a choice about what to lose.

| Sink | Cost | How often |
|---|---|---|
| Repair | 30 per piece, full condition | 3 per hour |
| Upgrades | 180 / 270 / 396 for the smith's work by band, 300 per socket drill | one attempt every 3 hours |
| Materials | 10-320 per unit of metal or gem | 2 units per hour |
| Respawn items | 120 field token, 30 honour war pass | 1-2 per hour |
| Travel | 25 per scroll | 4 per hour |

The flask charge (40) and the ration (15) take another 110 an hour. They are
small, but they are why you stop at the shop between fights.

A typical hour: ~1,110 coin in, ~995 coin out. The surplus is ~115 an hour,
which is one upgrade attempt every three hours, plus the drill and the metal
on top.

An upgrade attempt costs the smith's work plus one unit of the item's own
metal, and the metal is burned whether the attempt works or not. A failure
costs the coin and the condition and can drop a step; the +5 cap is what
keeps it bounded, and the money goes into materials, sockets and better base
gear instead of an infinite refine treadmill.

The two valley smithies are mirror images. The caravan post in the middle is
where both sides trade, and where the rare metal and the big gems live, so the
contested ground is also the economic crossroads. A shop buys back at
`sell_rate` (0.25 of the listed value) and the shop price is `price` times
`buy_rate` (0.5). Round-tripping always loses, so there is no buy/sell loop to
exploit.

## The flask rule

One flask. Three charges. Healing and mana draw from the same pool, so a
healing draught and a mana draught fight over the same three charges.

- The flask holds **3 charges**. Any draught spends one charge.
- **2 second** cooldown between charges. You cannot dump all three at once.
- A healing draught restores **150 health**. A mana draught restores **90 mana**.
- A charge refills every **45 seconds out of combat**, or instantly for **40
  coin** at any shop.
- The priest class carries a chant that refills the flask faster. That is a
  reason to bring a priest, not a reason to bring ten flasks.

This is the fix for 2005 potion chugging. You cannot win a duel by drinking
faster than your opponent; you win it by arriving with charges and a plan.
The old game gave you a belt of potions and made the belt the skill. Now the
three charges are the constraint and the priest's psalm is the counter.

## Gear endurance

Every equipped piece carries condition. Death is the only thing that spends it.

- The reference piece runs **0 to 100**. The item system scales the max per
  slot and per metal, so these numbers are the reference piece.
- **Death costs 10** on the reference piece (10 percent of the piece's max
  endurance, at least 1) on every equipped piece.
- At or below **20 condition** the piece starts eating its own **max
  endurance**: every further death costs **1 point of max endurance,
  permanently**. A repair never brings it back.
- A repair costs **30 coin** and restores condition to the piece's current
  max. It does not restore max endurance.
- A **restoration** costs **600 coin** and gives back **10 points of max
  endurance** on one piece. This is the only way to undo the permanent loss,
  and it costs more than the upgrade it protects.
- A piece at **0 condition is broken** and gives no stats until repaired.

Death costs gear, not the evening. XP is never lost (`death_xp_percent` 0) and
coin is never lost on death. You pay for your mistakes in condition and
repairs, in the currency you can earn back, and never in progress.

## Levels

Level cap **60**. The table is explicit in `economy.json`: `xp` is the total XP
needed to *reach* that level, level 1 is 0, level 60 is 316,730. No formula, no
guesswork, no auto-level.

- Mob 40 XP, elite 300 XP, boss 2,500 XP, quest 150-800 XP, war window 600 XP.
- No XP loss on death. No coin loss on death. A bad night is a long night, not
  a reset.
- About 30 hours of mixed play to the cap. Party XP is split between the
  members, so a party of three earns less each but clears the valley three
  times faster.

## Three ways to earn, three ways to lose

**Earn**

1. **The valley.** Mobs, elites, bosses, quest turn-ins, bounty boards. Coin
   and level. Unglamorous, always there, and the reason a new player has
   money on day one.
2. **The war.** Statue war windows four times a day, guild war on its own
   schedule. Honour for presence, fame for kills and captures, and a chunk of
   XP. This is the best money per hour in the game, and it is also the only
   money that requires other people.
3. **The season.** Goal tiers, war attendance streaks, faction war wins. Ember
   Marks, and the only currency that resets. It is a reason to come back
   tomorrow.

**Lose**

1. **Death.** Ten condition per piece, and one point of permanent max
   endurance for every piece already under 20. It is the only tax the game
   charges you for being bad.
2. **The smith.** 180-396 coin for the work, 10-320 for the metal it burns,
   300 for a drill. The +5 cap means the money has to go into materials,
   sockets and better base gear instead. There is no infinite refine treadmill
   and no cap raise that turns old gear into new gear.
3. **The map.** Travel scrolls, respawn tokens, rations and flask charges. The
   cost of being somewhere you should not be.

Nobody loses XP for dying. Nobody loses a coin they earned. The three losses
are gear, gear-adjacent money, and travel. That is the whole economy.
