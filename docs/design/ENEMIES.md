# ENEMIES — arena duel enemies (design)

Seat: enemy designer. Date: 2026-10-12.
Goal: every enemy has one reason to exist and one counter from the player's own kit.
Rule: no enemy attack without a tell and a counter.

## Code kinds read (invent none)

Read 2026-10-12, byte-exact:

- `crates/ai/src/gladiator.rs`: `Intent` (Attack, Block, Dodge, Advance, Retreat, Reposition). One `Percept` in, one `FighterInput` out.
- `crates/ai/src/depth.rs`: `Role` (Holder pins the center and trades blows; Flanker circles for side arcs). `Phase` (Open, Pressure, Finish) is pacing only.
- `crates/ai/src/melee.rs`: `StrikePhase` (Idle, Windup, Active, Recovery). `is_threat` is Windup or Active. Not a kind. A clock.
- `crates/ai/src/personality.rs`: `Personality` (reaction 0-5 ticks, range 1000-1500 mm, aggression 250-750). Not a kind. A temperament.
- `crates/ai/src/legion.rs`: `Formation` (Line, Column, Wedge, ShieldWall) and `Order` (Hold, Advance, Charge, Retreat, FlankLeft, FlankRight). Squad shapes for hundreds of soldiers. Not single-duel kinds.

Tier check (re-checked 2026-10-17): `rg -n "grunt|veteran|elite|boss" crates/ai/src` hits `crates/ai/src/depth.rs:460` only — one `elite` doc comment. The ladder is the tier *number*: `SHIPPED_TIERS` seats tiers 1-3 (`depth.rs:433`) and `PRIMUS_TIER` seats tier 4 (`depth.rs:463`), `parse_tiers` returns `[TierRow; 4]` (`depth.rs:585`), and `crates/ai/src/tier.rs:14` re-exports the ladder for the non-AI crates.
Tiers 1-4 are shipped in code and the tier_4 file row landed: `content/design/enemies.rows` carries tier_1..tier_4 and `crates/game/tests/it/design_enemy.rs::header_holds_four_tiers` seats four rows.
Wiring check (re-checked 2026-10-17): the rows live in `depth.rs` and `DepthBrain` reads them; the install path out of the crate is `ForkProposal::SetTier` in `crates/arena/src/fork.rs:83`, applied through `DepthBrain::set_tier` at `fork.rs:262`, and the caller is the tournament ladder (`crates/playtest/src/tournament.rs:1073` puts `SHIPPED_TIERS[index]` on every brain of the duel). That caller is landed at HEAD (row GA13) and it posts 995 permille per adjacent pair on the real duel — see "Curve and bar numbers" below. `crates/endpoint/src/assistant.rs:1108` only narrates `SetTier`. The rung that clears the bar without freezing the ladder is a packet for the AI builder (`target/queue/ready/0-des-enemy-tier-ladder-20261017.md`), never an edit here.

## Global numbers both cards use

Combat is v2 (`content/design/combat.rows`, `crates/combat/src/tables.rs`):

- Tell: `attack_startup_ticks=15` (250 ms at 60 Hz). Recovery 8. V1 startup 4 is replay-only.
- Parry window 7 (below startup, so a held block ages out). Parry stagger 12.
- Dodge duration 10, i-frames 6. Dodge cost 20, attack cost 25, regen 2 per tick.
- Damage 120 permille. Chip 1/5. Hit react 6, hitstop 4, knockdown 30.
- Every attack reads Windup (15) then Active (1) then Recovery (8) from `StrikePhase`.

Bodies and space:

- Size: capsule radius 0.4 m, height 1.8 m (`crates/arena/src/lib.rs`). Both cards use the same body as the player. Fair duel.
- Reach: blade 200-1500 mm (`crates/arena/src/match_wire.rs`). Preferred swing 1000-1500 mm (`personality.rs`).
- Speed: top 5.0 m/s (`TOP_SPEED_MPS`). Dodge covers about 0.83 m (5 m/s over 10 ticks).
- Wait ring 3000 mm, edge 500 mm (`melee.rs`). Player spawns 8 m apart (`docs/design/CORE-LOOP.md`).

## House numbers

Two numbers below are house numbers, not measured facts:

- The 60% tier bar (a tier beats the tier below in 60% or more of seeded duels). Stated on each tier row. Keep until the tournament proves another number.
- The 2-4 tick human noise (33-66 ms) plus one action per 8 ticks in `crates/playtest/src/winrates.rs`. The code marks it as a house number itself. Calibrate it on recorded human tapes when any exist. No *recorded* human tape exists today (`content/` carries none; `winrates::human_tape_for` builds a seeded noise tape out of the perfect brain, so it can never calibrate its own number), so the house number stays.

## Card 1 — Holder duelist (`Role::Holder`)

- Size: capsule 0.4 m radius, 1.8 m tall. Same as the player.
- Archetype: center holder. Pins the middle and trades blows. Block bias +75, attack +25 (`depth.rs`).
- Speed: top 5.0 m/s, same as the player. Advance bias -25. It does not chase. It holds.
- Moves: Attack, Block, Dodge, Advance, Retreat (plus Reposition when far). Same `Intent` set as the player.
- Tell in ticks: 15. Windup 15, Active 1, Recovery 8. Read from `StrikePhase`.
- Counter: parry. A fresh block press inside the 7-tick window negates the hit and staggers the holder 12 ticks. Dodge (6 i-frames) also works, but parry is the intended answer.
- Reason for HP: 1000 permille (100 display). Same as the player. HP tests answers, not grind. Stats stay in the narrow 900-1100 band per `docs/design/matchups.csv`. Edge comes from hit rate, not from HP.
- Silhouette note: square stance, shield forward, feet planted. Reads as a wall. Spectacle serves information, never hides the tell.
- Question it asks the player:
  - "Can you avoid my damage?" Answer: parry. Fresh block inside 7 ticks.
  - "Can you hit me?" Answer: attack in the punish window. Its Recovery is 8 ticks. Strike then, as `melee.rs` rewards with +250 attack bias on a recovering target.
- Mobility check: reach 1.5 m. From the 3 m wait ring it closes in about 36 ticks at 5 m/s. Player dodge covers 0.83 m per 10 ticks. The holder is not faster than the player, but it does not need to be. It holds the center and forces the player to come. It is a threat because it matches escape speed and cuts the ring, not because it outruns. An enemy slower than the player's escape is never a threat. This one is not slower.

## Card 2 — Flanker duelist (`Role::Flanker`)

- Size: capsule 0.4 m radius, 1.8 m tall. Same as the player.
- Archetype: circler. Goes for side arcs. Advance bias +75, dodge bias +50 (`depth.rs`).
- Speed: top 5.0 m/s, same as the player. It uses speed sideways, not head-on. It wraps and then closes.
- Moves: Attack, Block, Dodge, Advance, Retreat (plus Reposition when far). Same `Intent` set as the player. It prefers Dodge and Advance.
- Tell in ticks: 15. Windup 15, Active 1, Recovery 8. Same clock as Holder. Its arc reads as a sidestep before the swing.
- Counter: dodge. Sidestep the arc. The 6 i-frame ticks cover the 1 Active tick with margin. Block is the backup answer. Parry still works but the window is harder on a moving foe.
- Reason for HP: 1000 permille (100 display). Same as the player, same reason as Holder. Skill first. Veterans in `matchups.csv` sit at 110 HP only to set the odd cycle, still inside the narrow band.
- Silhouette note: bladed stance, body sideways, always moving. Reads as a knife circling. Distinct from the Holder wall at a glance.
- Question it asks the player:
  - "Can you avoid my damage?" Answer: dodge. Move with the arc, not into it.
  - "Can you hit me?" Answer: cut it off, then attack. Advance to close the 3 m ring, then strike when it commits. When it recovers (8 ticks), punish as with Holder.
- Mobility check: reach 1.5 m. It closes the 3 m ring in about 36 ticks at 5 m/s, same math as Holder. Player dodge is 0.83 m per 10 ticks. The flanker matches escape speed and adds lateral angle, so it stays a threat. A straight-line runner slower than 5 m/s would never catch a dodging player. This one does not run straight.

## Why no other cards

- `StrikePhase` is the shared clock, not an enemy.
- `Personality` (reaction, range, nerve) tunes both cards. Bold (750) presses, shy (250) guards. Both stay inside the same two cards.
- `Formation` and `Order` (Line, Wedge, ShieldWall, Charge, Flank) are squad shapes for the siege field, not single duelists. One melee enemy at a time per encounter. Squads wait for the legion seat.

## Tier plan (names PROPOSED; T1-T4 rows shipped in `depth.rs` and in the designer file)

The four names — grunt, veteran, elite, boss — are PROPOSED designer labels for
the code's `tier_1..tier_4` rows: `rg -n "grunt|veteran|elite|boss" crates/ai/src`
hits one doc comment only (`depth.rs:460`), so the ladder is the tier *number*
and the name is this card's word for it. The `tier_4` file row landed:
`parse_tiers` seats four rows (`crates/ai/src/depth.rs:585`),
`content/design/enemies.rows` carries tier_1..tier_4, and
`crates/game/tests/it/design_enemy.rs` seats four rows, checks the 15-tick tell
against the v2 table and runs the 60% bar over its stat model. What is still
PROPOSED is the rung that *holds*: the row file carries HP, poise, tell and
counter only, while `TierRow::role()`, `reaction_ticks()` and
`aggression_permille()` derive one role and one temperament per tier number, and
that derived rung posts 20 permille on the real duel ("Curve and bar numbers"
below, now 995 with the rung landed and the ladder frozen). So the bar needs a
rung that wins inside the P3 bands, which is the packet
`target/queue/ready/0-des-enemy-tier-ladder-20261017.md` for the AI builder.
The boss itself needs no new ask: the owner answered with `EB-2026-10-13-S2` and
both asks closed.

| tier | name | card | change from below | bar |
|---|---|---|---|---|
| T1 | grunt | Holder, calm (nerve 400, reaction 4) | first foe, slow to notice | beats nothing yet, teaches parry |
| T2 | veteran | Flanker, even (nerve 500, reaction 2) | adds the arc, must dodge | beats T1 in 60%+ of seeded duels (house number) |
| T3 | elite | Holder, bold (nerve 700, reaction 1); a Flanker-kit mix stays a card proposal | stamina pressure, riposte | beats T2 in 60%+ of seeded duels (house number) |
| T4 | boss (row landed) | Holder body, Flanker feet, phase 2 | phase answer, see below | beats T3 in 60%+ of seeded duels (house number) |

Noise for all tiers: perfect player first (a variant nobody can finish is skipped), then the seeded 2-4 tick human error, default off. House number until human tapes calibrate it.

Tier rows scale HP inside the narrow band only: `hp_permille` 950/1000/1050/1100 with `stagger_permille` 400/500/600/700 (`content/design/enemies.rows`), so HP stays a test of answers, never a wall. The tier rows name the counter which the tables allow (`counter` 1 parry, 2 dodge, 3 block); the first two tiers teach parry then dodge.

Tier composition (PROPOSED, and it adds no kind): T1 and T3 hold the Holder card, T2 the Flanker card, T4 a Holder body with Flanker feet. `Role` still has its two kinds, so no card below was invented. The rung is derived, not authored: `TierRow::role()` maps tier 2 to `Role::Flanker` and every other tier to `Role::Holder`, and `reaction_ticks()`/`aggression_permille()` climb 4/2/1/1 and 400/500/700/700 (`crates/ai/src/depth.rs:281,289,298`). The row file still carries no role column, so a designer cannot retune a rung without a Rust edit — which is what the packet changes.

Boss: owner answered with `EB-2026-10-13-S2` [BOSS-primus-t4] (both asks CLOSED 2026-10-12/13): build the T4 primus boss as these cards propose it. The answer is recorded by the earlier runs of this seat (`target/queue/archive/builder-design-enemy-r10.md`, `r13.md`); today's `node C:/empire/center/empire.mjs inbox engine2040 list` does not list it, so no second ask was filed. Code and file landed: `PRIMUS_TIER` in `crates/ai/src/depth.rs:463` (1100 HP inside the narrow band, 15-tick tell, parry answer, phase 2 at or below 500 HP) and the tier_4 row in `content/design/enemies.rows`. No new boss stats beyond that answer; what the boss still needs is the skill rung and a sweep that counts its phase-2 answer, not the stat model alone.

## Curve and bar numbers (measured, not proposed)

The sweep (E07-12, `crates/playtest/src/tournament.rs`): one knob — `match_ticks`, the ONE `BalanceConfig` field the sweep varies — at 300/600/1200 ticks, 200 matches per difficulty per value (1800 duels), the perfect player first, then the seeded 2-4 tick human error when enabled (default off). It writes `target/reports/difficulty-<UTC>.csv` with `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials` plus `# skipped ...` lines for a variant nobody finishes. A number per difficulty tier is the win rate in permille on that tier's row.

- Newest curve (2026-10-17T0511Z, the dry-fallback re-run): `target/reports/difficulty-20261017T0511Z.csv` (sha256 `7997DE3027D7D5D3A86607C1B13F502E2A1BA7851F1D50B64D3A18FF41B1CF40`), 3 data rows, 6 skips. Pressure finishes at 57/2/0 permille at 300/600/1200 ticks with `2:1000` kill share and stuck 146/382/396 of 200; even and cauldron are no-finisher at all three values. A second full sweep (`difficulty-20261017T0512Z.csv`) wrote a byte-identical body, so the curve replays exactly.
- The curve before it (`difficulty-20261017T0233Z.csv`, sha256 `2D1EDAA349CCDF1E997AA94D8088DCDE8F2E63A61B488C763C1EDBE2F992541A`) had the same 3 rows and 6 skips with pressure at 482/472/420 and stuck 4/12/56; the one before that (`difficulty-20261016T2354Z.csv`, sha256 `687803EEB19466E69CE1037F8AB18494387ED70DABFF077F0510E832EAFC68AB`) finished 7 rows inside the 400-600 band. Two collapses in one day, both measured.
- The tier bar on the real duel is green today: `cargo test -p engine-playtest --lib tournament::tests::tier_bar_posts -- --nocapture` prints `tier pair tier_2 vs tier_1: 995 permille of 200 duels` and `tier pair tier_3 vs tier_2: 995 permille` (was 20 permille at 01:55Z), and the inverted install still refuses at 0 permille. 995 permille is a blowout, not a rung: the same install freezes the ladder (57/2/0, stuck 396).
- Bands (house numbers): win rate 400-600 permille, kill-share cap 350. The newest curve violates both (57/2/0; `2:1000`), the same failure the P3 sim pins report.
- `cargo test -p engine-game --test it design_enemy::` 4 passed: the 60% bar holds in the rows' own stat model (HP plus poise plus seeded jitter). The real ladder contradicts its side effects, so that model is a floor, never the proof.
- `cargo test -p engine-playtest --lib tournament` is RED today (23 passed, 2 failed): `p3_standard_posts_numbers_with_known_stuck_edge` (left `[(0,191),(1,191)]`, right `[(0,3),(1,3)]`) and `proof_gate_reproduces_byte_identical_with_five_named_stuck_legs` (left 6, right 2). Both pins are stale receipts of the pre-install ladder and are deliberately NOT re-blessed, because the reached numbers breach the bands. Cause is outside this seat: `crates/ai` landed `tier_wish_to_world` at 04:22Z (commit `20d57419`). Named in `docs/design/balance-log.md`.

The 60% tier bar and the 2-4 tick noise are house numbers (stated at the top). The noise cannot be calibrated yet: no recorded human tape exists, and `winrates::human_tape_for` builds a noise tape out of the perfect brain, so it cannot calibrate its own number.

## What the pilot checks

- Every swing shows its 15-tick Windup. No instant hit.
- Holder parry works in the 7-tick window and staggers 12.
- Flanker arc is dodged with the 6 i-frame ticks.
- Both cards at 1000 HP feel fair. Wins come from answers, not from HP.
- Open: boss kit, phase 2, the calibrated noise number, and the skill rung that makes the 60% bar true on the real ladder (packet `des-enemy-tier-skill-rung-20261017`).
