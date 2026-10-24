# Grapple / hook-and-pull in games: what to steal for a Roman melee duel

Date: 2026-10-24. Scope: how shipped games do "hook an enemy and pull them" (or pull yourself to them), what it is called, the numbers where I could verify them, the counters, and the feel details.

## How to read this file

- **Verified** = I fetched the page or the source states it directly.
- **Snippet** = the number came from a search-result summary of a wiki page I could not open. Treat as a lead, not a fact.
- **Not found** = I searched and could not find a reliable number or source. Nothing is invented.
- **Proposed** = my starting value for our game, not sourced. Tune it in play.

Blocked or unreachable: Dustloop (MK), Overwatch fandom and wiki.gg (Roadhog), Liquipedia, Dota fandom, Halopedia, Zelda wiki, Arkham fandom (403, 402, 401, 404 responses). GDC Vault session pages are members-only, so no talk transcripts or slides were read. Most feel numbers below are from reviews, interviews, and wiki snippets, not developer postmortems.

## The generic name (plain answer)

- Game design has no single standard term for "hook a target and pull them." Common usages:
  - **Grapple** is the umbrella term for attach-and-travel mechanics (Halo, Sekiro, Batman, Titanfall, Apex).
  - **Hook** is the common name for the projectile version that pulls a target to the caster (Overwatch Chain Hook, Dota Meat Hook, Doom Meat Hook, Thresh Death Sentence in spirit).
  - **Pull** (target to caster) vs **yank / reel** (caster to a surface or target) is how designers and players split the directions. I did not find a single authoritative source for these labels.
  - **Tether** is used when two things are linked (Just Cause retractors, JC3 tether two objects together).
  - **Forced movement / displacement** is the umbrella term used in MOBA and fighting-game design writing for any effect that moves a target. I did not verify this from a specific source, so use it as a working term.
- For our docs: **"hook-pull"** for the target-to-caster version, **"grapple"** for self-traversal, and **"tether"** for the rope-between-two-things version.

## Per-game findings

### 1. Mortal Kombat Scorpion "Spear" ("Get over here")
- **Official name:** Spear (MK1 B1; MK11 spear). Genre: projectile hook-pull, pull-to-melee.
- **Play:** Spear is thrown and lodges in the opponent, then pulls them toward Scorpion. The MK X enhanced version can stun the opponent in place instead of pulling (Prima guide, search snippet).
- **Frame data:** Not found. Dustloop was blocked and the search only returned other games' move data. Do not use any numbers from this section without checking Dustloop directly.
- **Counters / balance:** Not found in sources I could read beyond the enhanced-version variation.
- **Feel:** The taunt yell is part of the identity (Wikipedia notes the origin, search snippet). Recommended reading: Dustloop frame data pages.

### 2. Overwatch Roadhog "Chain Hook"
- **Official name:** Chain Hook. Genre: projectile hook-pull.
- **Play (Snippet, conflicting sources):** Range about 20 m, projectile speed about 62 m/s, pull speed about 40 m/s. Movement penalty during hook. These come from a fandom search summary I could not open, so treat as unverified.
- **Patch history (Snippet, secondhand):**
  - PTR 2018 "Chain Hook 2.0": redesigned pull so the target is dragged in front of Roadhog, not onto his position, except when he rotates hard. Line-of-sight checks were improved and aiming is based on Roadhog's view, not the hook's. Targets leaving line of sight before the pull are released (Kotaku, quoted by search).
  - A later PTR build reportedly weakened the hook: cooldown 6 to 8 s, and targets pulled from 3.5 m instead of 2 m (Mein-MMO, undated in search).
  - January 2023 (Blizzard forum patch notes, search snippet): impact damage cut from 30 to 5, and the final pull distance raised from 3 m to 4 m. Blizzard's stated goal was fewer deaths from being hooked and then one-shot.
  - Cooldown starts immediately rather than after the pull lands; pull delay shortened from 0.5 s to 0.3 s (forum summary, undated).
- **Counters:** Line-of-sight break, the hook cooldown, and the hook's damage being low so the pull is the payoff.
- **Feel:** Not found beyond the pull-direction change.
- Sources: [Kotaku, Chain Hook 2.0](https://kotaku.com/overwatch-s-roadhog-improves-with-hook-2-0-1791612838), [Mein-MMO PTR notes](https://mein-mmo.de/en/overwatch-ptr-roadhog-nerf-winston-buff,138944), [Invenglobal Chain Hook 2.0](https://www.invenglobal.com/articles/744/ptr-introduces-chain-hook-20-along-with-other-adjustments-and-features). Official patch notes not opened (blocked).

### 3. Dota 2 Pudge "Meat Hook"
- **Official name:** Meat Hook. Genre: projectile hook-pull, first-unit collision.
- **Play (Snippet):** Cast range 1300, damage 150 / 220 / 290 / 360 by level, pure damage. The hook grabs the first unit it meets and drags it back to Pudge. Talents: +120 damage at level 15, -4 s cooldown at level 20.
- **Not found:** hook travel speed, hook width, cooldown by level, mana cost.
- **Counters:** Predictable skill shot (per the wiki snippet), so skill is in aiming and the first-unit rule.
- Source: [wiki.cs.money Pudge](https://wiki.cs.money/dota/heroes/pudge), [Liquipedia Pudge changelog](https://www.liquipedia.net/dota2/Pudge/Old) (only the index was readable).

### 4. League of Legends Blitzcrank "Rocket Grab"
- **Official name:** Rocket Grab. Genre: projectile hook-pull.
- **Play (Snippet, LoL wiki search summary; not fetched):** Range 1115 from center / 1020 from edge (intended 1150, bug halves a check). Projectile speed 1800. Cast time 0.25 s. Width 140 from edge / 60 from center.
- **Stun (Snippet):** Stun 0.65 s. Blitzcrank is locked out of moving or attacking while the hook is in flight and for 0.25 s after it hits.
- **Patch note (Snippet):** V9.19 raised range from 1050 to 1100 units.
- **Pull distance:** Not separately given. The target ends up next to Blitzcrank.
- Sources: [LoL wiki Blitzcrank](https://wiki.leagueoflegends.com/en-us/Blitzcrank), [Template: Data Blitzcrank/Rocket Grab](https://wiki.leagueoflegends.com/en-us/Template:Data_Blitzcrank/Rocket_Grab).

### 5. League of Legends Thresh "Death Sentence" (Verified: fetched LoL template page)
- **Official name:** Death Sentence (the scythe throw plus the "Deathly Leap" pull). Genre: hook-then-tug, a two-stage pull.
- **Play (Verified):** Cast range 1100 (center) / 1040 (edge). Speed listed as 1900 and 1400 without saying which is scythe vs pull. Scythe hit: stun 1.5 s, airborne 0.4 s (removed if the first pull is interrupted). Each tug pulls the target "a short distance" toward Thresh; the page gives no number.
- **Cooldown (Verified):** 19 / 16.5 / 14 / 11.5 / 9 s by rank; Arena 11 / 10.5 / 10 / 9.5 / 9 s. Hitting an enemy cuts the current cooldown by 2 s.
- **Counters (Verified):** Tugs only happen while the target stays stunned and Thresh has not cast Deathly Leap. Cleanse removes the mark. Spell shield blocks everything, including the cooldown reduction. A target too close to Thresh is not pulled by the tugs. Deathly Leap has a max cast range (about 3000 units); if the target goes beyond it while marked, the dash fails.
- **Feel:** Not found.
- Source: [LoL wiki Template: Data Thresh/Death_Sentence](https://wiki.leagueoflegends.com/en-us/Template:Data_Thresh/Death_Sentence).

### 6. Doom Eternal "Meat Hook"
- **Official name:** Meat Hook (mod on the Super Shotgun). Genre: enemy-only grapple, self-pull.
- **Design (Verified: Wikipedia):** The hook originally pulled enemies toward the player. It was reversed to pull the player toward enemies to maximize mobility. A proposal to hook any surface was rejected because the team felt it would trivialize other traversal mechanics. The hook grabs enemies and slingshots the player toward them.
- **Design rationale (Search snippet):** Developers said the hook only attaches to demons, to keep it from becoming a general grapple. Level designers used it on a Pain Elemental to reach upper levels instead of two or three double-jump ledge grabs.
- **Counters (Snippet, DoomWiki):** The hook disengages on knockback, dash, or double-jump. It does not work against shielded enemies (marauders, riot soldiers with shields up). Acceleration is frame-rate based, not tick based, which allowed a speed exploit ("yeethook") when opening the weapon wheel mid-pull.
- **Feel:** Not found beyond the design notes.
- Sources: [Wikipedia Doom Eternal](https://en.wikipedia.org/wiki/Doom_Eternal), [DoomWiki Meat hook](https://doomwiki.org/wiki/Meat_hook) (blocked, snippet only), [Game Informer QuakeCon preview](https://gameinformer.com/preview/2018/08/10/a-bigger-badder-bloodier-demon-fest).
- The GDC 2018 talk on push-forward combat is about the 2016 DOOM, not the hook. Do not cite it for the hook.

### 7. Just Cause 2 / 3 / 4 grapple and tether retractors
- **Official name:** Grappling hook (JC2/JC3); retractor (JC4). Genre: tether, grapple, pull-together.
- **Play (Verified: Wikipedia):** The hook can attach to nearly every object and NPC. Multiple objects can be tethered together. Extra tethers come from unlockable mods.
- **Tension and retraction (Snippet, reviews):** Retraction on the trigger, adjustable tension: low tension drags slowly, high tension flings. Heavier objects are harder to pull (JC4 fan wiki says the same of earlier tethers, so this is an inference for JC3). A reviewer says some objects need several tethers. Up to 6 tethers in the world at once (search summary).
- **Counters:** Mass. Tethers can be cut or their line strength limits pulling.
- **Feel:** Rope tension and fling force are the feel. Designers said they built retraction so objects smash into each other.
- Sources: [Wikipedia Just Cause 3](https://en.wikipedia.org/wiki/Just_Cause_3), [Den of Geek review](https://www.denofgeek.com/games/just-cause-3-playstation-4-review), [Just Cause fandom Retractor mods](https://justcause.fandom.com/wiki/Retractor_mods) (not opened).

### 8. Sekiro: Shadows Die Twice "Grappling Hook" (Shinobi Prosthetic)
- **Official name:** Grappling Hook (prosthetic tool). Genre: self-traversal grapple to fixed points.
- **Play (Verified: Fextralife):** Grapple points are circular markers; the marker must turn green to be in reach. Most markers pull Sekiro to the point; some launch him forward in his facing direction. Input L2/LT. Breaks stealth if crouched when landing. Certain bosses become grapple targets under special conditions.
- **Not found:** range, travel speed, whether enemies are pulled or launched, posture/vitality damage.
- **Design (Snippet, Game Informer):** Miyazaki said the traversal makes boss arenas more dynamic than Dark Souls' leg-circling. Not every boss should allow grappling, because grappling everywhere is monotonous. Position chosen with the hook matters in combat.
- **Feel:** Not found.
- Sources: [Fextralife Grappling Hook](https://sekiroshadowsdietwice.wiki.fextralife.com/Grappling+Hook) (verified), [Game Informer Miyazaki interview](https://www.gameinformer.com/interview/2018/08/21/hidetaka-miyazaki-on-sekiro-shadows-die-twice) (snippet).

### 9. Halo Infinite "Grappleshot"
- **Official name:** Grappleshot. Genre: self-traversal grapple.
- **Play (Snippet, Halopedia and GameSpot):** Cooldown does not start on a miss, stagger, or finishing blow. Quickshot upgrade cuts cooldown by 40%. Multiplayer: 5 grapples per pickup. Grappling a player rockets you to them and performs an automatic melee strike on arrival. Firing or meleeing mid-pull cancels the pull and leaves you in the air.
- **Design (Snippet, PC Gamer / NME):** The hook started as a multiplayer experiment. NME describes tuned momentum (heavy, momentum-driven, not acrobatic), a metallic attach sound and a reeling whine. Pure Xbox reports 343 rethought level design around it. Numbers for range and travel speed: not found.
- Sources: [Halopedia Grappleshot](https://www.halopedia.org/Grappleshot) (snippet), [GameSpot equipment guide](https://gamespot.com/articles/halo-infinite-equipment-guide-all-the-best-tools-and-how-to-use-them/1100-6498176/), [PC Gamer on the grapple](https://www.pcgamer.com/uk/halo-infinites-grappling-hook-wont-turn-master-chief-into-spider-man), [NME feature](https://www.nme.com/features/how-halo-infinites-grapple-shot-makes-it-the-best-sandbox-shooter-in-the-series-3273314).

### 10. Titanfall 2 grapple (slingshot)
- **Official name:** Grappling hook (Titanfall 2). Genre: self-traversal grapple.
- **Play (Snippet, 2016 interviews):** Used for verticality, speed, hitching onto Titans or friendly hitches, and damaging enemies. The team said getting the slingshot movement to feel natural took a while.
- **Not found:** the GDC talk, numbers.
- Sources: [PC Gamer Titanfall 2 hands-on](https://www.pcgamer.com/titanfall-2-pc-footage/), [PC World hands-on](https://www.pcworld.com/article/415196/titanfall-2-hands-on-grappling-hooks-make-everything-better.html).

### 11. Apex Legends Pathfinder "Grapple Hook"
- **Official name:** Grapple Hook (Tactical). Genre: self-traversal grapple with scaled cooldown.
- **Play (Snippet, Apex wiki):** Max length 21 m. Max travel time capped at 5 s (Nov 2020 revision). Cooldown scales with distance: short grapples cost little (minimum about 10 s), long grapples cost up to 30 s (the Oct 2020 notes said 35 s, sources conflict).
- **History (Snippet):** Season 6 nerf took cooldown from 15 to 35 s and dropped pick rate. Oct 2020 made cooldown distance-based. Season 25 restored a variable 15 to 30 s cooldown, with a Legend Upgrade for two charges.
- **Design point:** The developers said short grapples should cost little, which opens room for skill expression.
- Sources: [Apex wiki Pathfinder](https://apexlegends.wiki.gg:443/wiki/Pathfinder) (snippet), [Apex wiki Oct 6 2020 patch](https://apexlegends.fandom.com/wiki/October_06,_2020_Patch) (snippet), [GG Recon Pathfinder](https://www.ggrecon.com/articles/pathfinder-reverse-nerf-gives-hope-to-path-mains).

### 12. Batman Arkham "Batclaw" / Line Launcher
- **Official name:** Batclaw (Arkham series). Genre: enemy pull plus takedown setup.
- **Play (Snippet, Gamer Guides):** Pulls thugs off ledges. In combat it drags them toward Batman until they fall. Upgrades: disarm, pull to Batman for a stun attack, combo takedown. Ultra Batclaw (Arkham Asylum) fires three claws. Arkham Knight extends the takedown range.
- **Not found:** pull speed, stop distance, range numbers.
- Sources: [Gamer Guides gameplay basics](https://www.gamerguides.com/batman-arkham-knight/gameplay-basics) (snippet), Arkham fandom Batclaw (blocked).

### 13. Insomniac Spider-Man web swing and web yank
- **Official name:** Web swing; web yank. Genre: self-traversal swing; enemy-yank is the enemy-pull variant.
- **Play:** The "web yank" was not documented in any source I could read. The 2019 GDC talk "Concrete Jungle Gym: Building Traversal in 'Marvel's Spider-Man'" (Doug Sheahan) covers swing design and camera-for-speed. The GDC Vault page is members-only.
- **Not found:** numbers or a verified web-yank description.
- Sources: [GDC Vault Concrete Jungle Gym](https://gdcvault.com/play/1026422/Concrete-Jungle-Gym-Building-Traversal) (session abstract).

### 14. God of War
- **Official name:** Blades of Chaos pull (God of War Ragnarok), Serpent's Snare (Leviathan Axe, Ragnarok). Genre: pull-to-Kratos and launch-self variants.
- **Play (Snippet):** Ragnarok's Blades of Chaos can pull an enemy close or launch Kratos toward one. Serpent's Snare throws enemies across the arena. The 2018 game's axe is thrown and recalled; I found no 2018 pull or grab for either weapon.
- Sources: [Wikipedia God of War (2018)](https://en.wikipedia.org/wiki/God_of_War_(2018_video_game)) (not checked for pull mechanics), [God of War fandom Leviathan](https://godofwar.fandom.com/wiki/Leviathan) (not opened).

### 15. Dying Light grapple / Dying Light 2 rope
- **Official name:** Grappling hook (DL1), rope/grapple (DL2). Genre: self-traversal.
- **Play (Snippet, Techland interviews):** DL1 hook worked like an instant pull to a high vantage point. DL2 made it physics-based, "Tarzan rope" style, and requires something above the player to swing from so it cannot be abused.
- **Talk:** Bartosz Kulon's GDC talk on Dying Light movement. Slides and recording not read.
- Sources: [GamingBolt DL2 physics hook](https://gamingbolt.com/dying-light-2s-grappling-hook-is-more-physics-based-like-spider-man-techland/amp) (snippet), [80.lv GDC parkour production guide](https://80.lv/articles/gdc-parcour-production-guide).

### 16. Ghostrunner grapple
- **Official name:** Grappling hook. Genre: self-traversal grapple.
- **Play (Snippet, PAX East 2020):** Designer Dariusz Kulak said the goal was the most responsive controls possible, and anything outside player control gets tuned. Grapple is one of the starting-kit movement tools alongside dash and wall-run. The team started with a prototype whose only fun was the movement.
- **Not found:** numbers.
- Source: [Outerhaven PAX East 2020 interview](https://www.theouterhaven.net/pax-east-2020-a-conversation-with-ghostrunner-game-designer/).

### 17. Zelda Hookshot / Longshot
- **Official name:** Hookshot; Longshot (Ocarina of Time, twice the reach). Genre: hook, pull to self or pull target.
- **Play (Snippet, zelda wiki search):** Longshot pulls Link to some enemies (ReDead) and pulls certain enemies to Link (Morpha's nucleus). Hookshot stuns some enemies and instantly kills a few. Nice Hookshot (A Link Between Worlds) has longer length and damage. Base Hookshot range and speed: not found. BotW and TotK: not checked.
- Sources: [zeldawiki.wiki Hookshot](https://zeldawiki.wiki/wiki/Hookshot) (search snippet only).

### 18. Bionic Commando (2008 GRIN remake, arm swing)
- **Official name:** Bionic arm (swing). Genre: swing-grip, not a hook.
- **Play (Snippet, press reviews):** Aim the arm at a surface. Release timing sets momentum; letting go at the apex gets you nowhere. A blue U-shaped indicator shows the best release point. Jump retracts the claw.
- **Development (Snippet):** Capcom picked GRIN partly for its physics expertise. The team retooled the swing and focus-tested it.
- Sources: [Worth Playing developer interview](https://worthplaying.com/article/2008/12/26/interviews/57634-bionic-commando-psx360pc-developer-interview/) (snippet), [Gamasutra/Game Developer GRIN interview](https://www.gamedeveloper.com/game-platforms/interview-grin-s-viklund-on-rearming-capcom-s-i-bionic-commando-i-) (not opened).

### 19. Attack on Titan ODM games (Omega Force)
- **Official name:** ODM Gear. Genre: self-traversal grapple with fuel.
- **Play (Snippet, reviews):** Lock onto a titan body part with the stick; the ODM reels toward the target or dashes in for an attack. Fuel limits swinging. Reviewers praise the swing feel, but not with numbers.
- **Not found:** Omega Force developer material on the physics. One fan source claims a dedicated physics engine; unverified, do not use.
- Sources: [GamingTrend AoT review](https://gamingtrend.com/reviews/youre-dead-if-you-run-out-of-gas-attack-on-titan-review) (snippet), [Waytoomany AoT2 review](https://waytoomany.games/2018/07/01/review-attack-on-titan-2-switch/) (snippet).

### 20. Adjacent: For Honor, Dead Island, Hades
- **For Honor:** I searched for the grab mechanic. Nothing specific was found. The relevant design write-up is a Game Developer mechanics deep dive on rock-paper-guard breaks (blogger, not a GDC talk). Useful for duel readability, not for a hook.
- **Dead Island, Hades:** Not researched. Dead Island's and Hades' mechanics are not relevant as grapples as far as I know, but I did not verify this.

### 21. Physics sandbox mods and tools
- **Bonelab / Boneworks:** A search found a generic Bonelab grapple listing that I could not verify. Not usable as a source.
- **Teardown:** No grapple tool found in the press release tool list (17 tools, none grapple). Physics praised by critics. Not usable as a source for a grapple.
- **Garry's Mod:** Not found in search. The rope tool is well known, but I did not find a source to cite.
- **Skyrim / Mount & Blade:** No grapple mod found. HIGGS (Skyrim VR) is a grab/gravity-glove mod, not a hook.
- **Minecraft grapple mods (usable as design reference only):** Advanced Hook Launchers makes relative mass affect the pull, caps capacity, and dismounts riders. Grappling Hooke models the hook as a spring that pulls toward a resting distance. Sources: [Advanced Hook Launchers](https://www.curseforge.com/projects/244849), [Grappling Hooke](https://www.modpackindex.com/mod/123159/grappling-hooke).

## Cross-cutting feel: hit-pause, juice, and screen shake

- **Hitlag / hit-pause (Snippet, SmashWiki):** On contact both fighters freeze for a set number of frames. Frames scale with damage and knockback; an attack with zero knockback causes no hitlag. Electric attacks multiply the frame count by 1.5 (rounded down). Example: a 15% move gives 10 frames in Smash 64, 8 in Melee, 9 in Brawl and Smash 4, 15 in Ultimate. The wiki warns that "hitstun" means different things in different games, so check the term before comparing.
- **Juice (Snippet, 2012 talk by Martin Jonasson and Petri Purho, "Juice it or lose it"):** Screen shake for impact; bigger or smaller scaling on events; easing for animation; particles in moderation. Jan Willem Nijman (Vlambeer) gave a separate talk on screen shake. Folmer Kelly warned against juice for its own sake.
- **Game feel framework (Snippet):** Steve Swink's book is the usual reference for input-to-perception loops.
- **Not verified:** Specific hook screen-shake values, hook-attach sound timing, camera behavior, and rope rendering for any game in this list.
- Sources: [SmashWiki Hitlag](https://www.ssbwiki.com/Hitlag) (snippet), [Juice talk write-up](https://rpgplayground.com/research-making-a-juicy-game/) (snippet).

## Counters and balance patterns that recur

- **Line-of-sight release:** Overwatch releases the target when they leave line of sight before the pull.
- **Disengage on your own action:** Halo cancels the pull if you fire or melee; Doom disengages on knockback, dash, or double-jump.
- **Cooldown rules:** Halo does not start the cooldown on a miss; Pathfinder scales cooldown with distance; Thresh cuts cooldown on hit by 2 s.
- **Stun then tug:** Thresh tugs only while the target stays stunned, and a spell shield blocks it.
- **Too close = no pull:** Thresh does not tug a target that is already in melee range.
- **Target restrictions:** Doom only attaches to demons; Sekiro only allows some bosses as grapple targets.
- **Keep damage low:** Overwatch cut Chain Hook impact damage from 30 to 5 (Jan 2023, snippet) so the pull is the payoff and a hook is not a one-shot.
- **Final position:** Overwatch changed the stop distance after the pull (2 m, then 3 m, then 3.5 to 4 m across patches; sources conflict).

## Steal list: 12 ideas for a Roman melee duel, ranked by feel impact

Numbers marked (source) come from the sections above. Numbers marked (proposed) are my starting values, not facts. Tune them in play.

1. **Hit-pause on hook contact.** Freeze both fighters 6 to 10 frames at 60 fps on hook contact, scaled by damage and knockback, and zero knockback means zero pause (SmashWiki pattern). (proposed, based on SmashWiki range of 8 to 15 frames for mid-damage hits.)
2. **Pull the victim in front of you, not onto you.** Stop them about 2 to 4 m in front, so the pull lands them at sword reach and does not put the caster inside their hitbox. Overwatch moved to "in front" with stop distances around 3 to 4 m (Overwatch snippets). Tune the stop distance to your weapon reach. (proposed)
3. **Readable stun-then-tug.** Hook hit = 0.4 s airborne, 1.5 s stun, pull only while stunned (Thresh). The airborne window is the readable tell. (source: Thresh verified; numbers may need adjustment for gladiator weights.)
4. **Auto-melee on arrival.** When a hook-pull lands you next to the victim, fire an automatic strike, as Halo does on enemy grapple arrival. This is the payoff that makes the pull feel earned. (source: Halo snippet; damage value proposed.)
5. **Keep momentum on the self-pull.** Let the player keep velocity when they release at the apex, as Bionic and Halo do, so the self-pull feels like a swing and not a stop. Release timing near the apex should matter. (source: Bionic and Halo snippets.)
6. **Visible tether with tension.** Draw a rope or chain with visible sag that tightens as the pull starts. JC3 tension (drag slowly or fling hard) is the model. Tension ramp is proposed at about 0.3 s from slack to taut. (proposed)
7. **Cooldown scales with distance.** Short hooks are cheap, long ones are expensive. Pathfinder: short grapple about 10 s, long grapple up to 30 s. For a duel that is 4 to 8 s for a short hook and 12 to 15 s for a full-arena hook. (source: Pathfinder for pattern; numbers proposed.)
8. **Pull cancels on the victim's own input.** Dodge, block, or knockback breaks the tether (Doom disengage pattern; Halo cancel). Victims need an escape, or hook-pull becomes a free kill. (source: Doom and Halo snippets.)
9. **Line-of-sight release.** If the victim steps out of your line of sight during the pull flight, release the hook. This handles corner cheese. (source: Overwatch 2018 change.)
10. **Lockout after the hook lands.** Lock the caster out of attacking for about 0.25 s after the hook hits, so the pull has a cost (Blitzcrank: 0.25 s lockout). (source: Blitzcrank snippet.)
11. **Mass-dependent pull.** Heavier targets (armoured gladiators) get a shorter pull or need a second hook. Advanced Hook Launchers and JC3 show weight as a factor. (source: JC3 and Minecraft mods, pattern only.)
12. **Low hook damage.** Hook damage 5 or lower (Overwatch 2023 cut from 30 to 5) so the pull, not the hook, is the payoff, and a hook cannot one-shot a victim. (source: Overwatch 2023 snippet.)

## Open items for follow-up

- Dustloop frame data for MK Scorpion Spear (blocked).
- Overwatch official patch notes for Chain Hook numbers (blocked).
- Halo Grappleshot range and travel speed (not found).
- Sekiro Grappling Hook pull vs launch enemy rules and posture damage (not found).
- GDC Vault sessions: Concrete Jungle Gym (2019), Spider-Man 2 traversal (2024), Kulon Dying Light talk, Claves For Honor (2016). Members-only, so transcripts would need a members account.

## Sources (used)

- Wikipedia Doom Eternal: https://en.wikipedia.org/wiki/Doom_Eternal
- Wikipedia Just Cause 3: https://en.wikipedia.org/wiki/Just_Cause_3
- LoL wiki Thresh Death Sentence template: https://wiki.leagueoflegends.com/en-us/Template:Data_Thresh/Death_Sentence
- LoL wiki Blitzcrank Rocket Grab template: https://wiki.leagueoflegends.com/en-us/Template:Data_Blitzcrank/Rocket_Grab
- Fextralife Sekiro Grappling Hook: https://sekiroshadowsdietwice.wiki.fextralife.com/Grappling+Hook
- Kotaku Overwatch Chain Hook 2.0: https://kotaku.com/overwatch-s-roadhog-improves-with-hook-2-0-1791612838
- Mein-MMO Overwatch PTR: https://mein-mmo.de/en/overwatch-ptr-roadhog-nerf-winston-buff,138944
- Invenglobal Chain Hook 2.0: https://www.invenglobal.com/articles/744/ptr-introduces-chain-hook-20-along-with-other-adjustments-and-features
- Apex wiki Pathfinder: https://apexlegends.wiki.gg:443/wiki/Pathfinder
- Apex wiki Oct 2020 patch: https://apexlegends.fandom.com/wiki/October_06,_2020_Patch
- GG Recon Pathfinder: https://www.ggrecon.com/articles/pathfinder-reverse-nerf-gives-hope-to-path-mains
- Halopedia Grappleshot: https://www.halopedia.org/Grappleshot
- GameSpot Halo Infinite equipment: https://gamespot.com/articles/halo-infinite-equipment-guide-all-the-best-tools-and-how-to-use-them/1100-6498176/
- NME Halo grapple: https://www.nme.com/features/how-halo-infinites-grapple-shot-makes-it-the-best-sandbox-shooter-in-the-series-3273314
- PC Gamer Halo grapple: https://www.pcgamer.com/uk/halo-infinites-grappling-hook-wont-turn-master-chief-into-spider-man
- SmashWiki Hitlag: https://www.ssbwiki.com/Hitlag
- Juice talk write-up: https://rpgplayground.com/research-making-a-juicy-game/
- Game Informer QuakeCon 2018 Doom: https://gameinformer.com/preview/2018/08/10/a-bigger-badder-bloodier-demon-fest
- Game Informer Miyazaki Sekiro interview: https://www.gameinformer.com/interview/2018/08/21/hidetaka-miyazaki-on-sekiro-shadows-die-twice
- Den of Geek JC3 review: https://www.denofgeek.com/games/just-cause-3-playstation-4-review
- Worth Playing Bionic Commando interview: https://worthplaying.com/article/2008/12/26/interviews/57634-bionic-commando-psx360pc-developer-interview/
- PC World Titanfall 2 hands-on: https://www.pcworld.com/article/415196/titanfall-2-hands-on-grappling-hooks-make-everything-better.html
- Gamer Guides Batman Arkham Knight basics: https://www.gamerguides.com/batman-arkham-knight/gameplay-basics
- GDC Vault Concrete Jungle Gym: https://gdcvault.com/play/1026422/Concrete-Jungle-Gym-Building-Traversal
- 80.lv GDC parkour: https://80.lv/articles/gdc-parcour-production-guide
- Outerhaven Ghostrunner PAX East 2020: https://www.theouterhaven.net/pax-east-2020-a-conversation-with-ghostrunner-game-designer/
- Wiki.cs.money Pudge: https://wiki.cs.money/dota/heroes/pudge
- GamingTrend AoT review: https://gamingtrend.com/reviews/youre-dead-if-you-run-out-of-gas-attack-on-titan-review
- Advanced Hook Launchers: https://www.curseforge.com/projects/244849
- Grappling Hooke: https://www.modpackindex.com/mod/123159/grappling-hooke
