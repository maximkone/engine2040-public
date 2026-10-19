# Blade Symphony and the dueling family — 2026-10-19

Owner ask: "similar mechanics, easy af for AI to make, deeper by 100x." So the
test for every mechanic below is: **can an AI author it as a table, and does it
still create depth for a player later?**

Sources read live 2026-10-19: blade-symphony.com official guide and press kit;
ModDB game and news pages (MotY post 4 "Stances", MotY post 7 "Battlefield
Manoeuvres"); the Steam community "Everything about Blade Symphony" guide; the
Puny Human balance GitHub note; plus the wider family: Die by the Sword (1998)
manual and reviews, Exanima dev history and quickstart, Overgrowth's 2009
fighting-system design doc, Movie Battles II dueling compendium and 2023/2024
releases, Mordhau/Chivalry 2 combat pages, For Honor's terminology guide and the
Gamasutra "Rock, Paper, Guard Breaks" analysis, Hellish Quart's official tutorial
lines and an academic review of its realism.

## 1. What Blade Symphony actually is (the inventory)

- **Three stances** (Fast/Reverse, Balanced, Heavy) plus an **Air** stance while
  airborne; four in later builds. Each character has **40+ moves** across the
  stances. Stance changes what every attack is.
- **String counter**: one counter per stance, incremented by *every* attack
  regardless of stance, so a combo is a path through a shared automaton. Dash
  +1, shuffle -1, so movement resets strings. Side attacks do not touch it. The
  UI shows it in the corner.
- **Charge tiers**: hold the attack to charge; tier 1 (no charge), tier 2 (flashes
  mid-charge, the mixup tier), tier 3 (full, knocks down and goes through block).
  The mid-charge flash is a deliberate tell.
- **Stance parry matrix**: the official guide says Heavy beats Balanced, Balanced
  beats Fast; Air always loses tracer parries. The community Steam guide says
  "Balanced beats fast, Heavy beats balanced, Air beats Heavy, Balanced and Fast
  beat Air" — a **contradiction between two official-ish sources**. That is a
  design lesson in itself: a matrix players must read has to be single-sourced
  and shown in the UI, or the community invents a wrong version.
- **Tracer parry (the deep part)**: sword paths are traced in 3D as you swing, so
  a parry is a real blade-vs-blade collision. Side attacks sweep wide for
  intercepting; thrusts go straight. You choose how flat or vertical the swing
  is and can angle it *while it is happening*.
- **Throws**: attack + defend together. No counter. Leaves the victim in a
  **Down state** with three options (roll, desperate stab, jump up). Anti-turtle.
- **Movement**: dash/roll/backflip, air attacks, shuffle. Movement also drives the
  string counter.
- **Swords**: katana (multi-hit, block), foil (feints, thrusts), jian
  (intercept, combos), etc. Weapon and character are separate axes.
- **No stat unlocks**: all unlocks cosmetic. Skill only. Balance was even done in
  the open on a GitHub repo with community pull requests.

## 2. The 100x-depth ideas, ranked by (depth / authoring cost)

### 2.1 String counter — the crown jewel
- Cost to author: one integer per fighter, one move table per (character, stance),
  a "reset rule" per movement action. All data.
- Depth: you must remember which string position each stance is on, choose when
  to switch, and decide whether to spend a dash/shuffle to reset. Consistency
  (same input, same move) makes it learnable; the shared counter across stances
  makes it expressive.
- Our home: `crates/combat` (a `string: u8` per fighter on the sim side, so it
  hashes) + `crates/rules` (an AI authors the move table as text).
- Note: this is the single best example of "easy for AI, deep for humans". It is
  a table plus a cursor.

### 2.2 Charge tiers with one visible tell
- Cost: a charge timer quantised to 3 tiers; the mid-charge flash is one clip
  tag. The table maps (stance, tier, string position) -> move.
- Depth: tier 2 is the mixup; the tell gives the opponent one read.
- Our home: `crates/combat` (tier in the frame data) + `anim_bridge.rs` (the
  flash pose). Blends with our hit stop and cancel windows (already present).
- Rule to steal: **tier 3 beats block**. One flag gives a charged attack a job.

### 2.3 Stance rock-paper-scissors, on the parry only
- Cost: a 3x3 outcome table (win / draw / lose). Three bits.
- Depth: attack choice stays situation-driven (range, speed, damage), so the
  matrix only decides blade clashes. That is why it is deep without being a
  guessing game.
- Our home: `crates/combat/src/exchange.rs` — we already have `resolve_clash`,
  `ClashOut`, `RIPOSTE_WINDOW_TICKS`, poise, and a parry window. The steal is a
  `Stance` field in the clash input plus the table.
- Lesson from the sources: publish **one** matrix, and show both stances in the
  world-space UI (For Honor's lesson: the matrix must be visible, because
  animations alone are not readable).

### 2.4 Tracer parry — wide sweep vs. thrust
- Cost: tag each swing `{wide, narrow}`, and let the clash resolver test the two
  swept segments. We already resolve a `BladeSwing` into joint cuts with
  sharpness/energy/wear.
- Depth: "swing wide to intercept, thrust to go through the gap" is a real,
  spatial decision that reads on screen.
- Our home: `crates/combat/src/cut.rs` (`BladeSwing`, `MAX_CUTS_PER_SWING`) +
  `exchange.rs`. This is closer to done than it looks.

### 2.5 Down state as a mini-game
- Cost: one state, three actions, a timer.
- Depth: the moment after a tier-3 hit is its own puzzle, and it gives the loser
  agency instead of a cutscene.
- Our home: `crates/combat` state machine + `crates/anim` clips.

### 2.6 Throws and the anti-turtle answer
- Cost: one input (both attack buttons), one unblockable, one down state.
- Depth: it punishes pure defense, which is the failure mode every sword game
  falls into.
- The family agrees, from four directions: Blade Symphony's throw; Mordhau's
  **stamina cost on every parry** ("losing a fight is usually a stamina loss
  before it is a health loss"); For Honor's guard break (counterable) and the
  Gamasutra analysis of the **defense-first meta** ("if the optimal way to play is
  reactionary, the game gets boring; the fix is a mechanic, not a number");
  Hellish Quart's **automatic block that fails**: the guard is free while you do
  nothing, but if you are too close the blade goes through it, so there is no
  turtle to hide behind.
- Our home: `exchange.rs` (`POISE_MAX`, `PARRY_POISE_DAMAGE`,
  `CLASH_POISE_DAMAGE` already drain poise on a parry — the Mordhau rule is
  already half-built). Add an unblockable throw + down state.
- This is the strongest *design* steal in the whole hunt.

### 2.7 Startup interrupt amplification
- Source: Movie Battles II compendium: hit during swing start-up takes **1.6x**
  damage bare, **1.2x** if you hold block during the swing ("swingblock"), 1.0x
  normal. Disarms and knockdowns are prevented by swingblocking.
- Cost: one comparison (was the victim in windup?) and a small multiplier table.
- Depth: it punishes mashing and rewards a "safety" input (hold block while
  attacking) that costs you your own offense. One boolean, real decisions.
- Our home: `crates/combat` frame data. Blends with our cancel/parry windows.

### 2.8 A momentum counter (ACM) instead of just health
- Source: Movie Battles II: ACC/ACM grow with landed hits and grant a damage
  bonus at 4 ACM, by style (light 1.08x, medium 1.10x, heavy 1.12x); partial ACM
  on some combo states; it drains on defense.
- Cost: one integer per fighter, thresholds as data.
- Depth: it makes long strings matter and gives the string counter (2.1) a
  payoff, without a second health bar.
- Our home: `crates/combat` (a `momentum: u8` beside poise).

### 2.9 Right of way and auto-block (Hellish Quart, the minimal-input deep duel)
- Source: Hellish Quart official basics (read 2026-10-19): characters **block
  automatically if you do not attack**; any attack can be cancelled by pressing
  Backward, which also turns on parrying; stamina drains on whiff and on tired
  guards, not on a landed hit; "tempo" is "the moment to go"; and a **Right of
  Way** rule decides double-kill scoring (first to move forward, lost by moving
  back, lost on a failed attack).
- Cost: one state ("am I attacking?" -> block else auto-guard), one cancel input,
  one stamina pool, one boolean for priority.
- Depth: near-zero inputs, and yet real fencing rhythm: attack, get parried,
  prepare to parry the riposte, counter-riposte. It is turn-taking with swords.
- Our home: `exchange.rs` (riposte exists: `RIPOSTE_WINDOW_TICKS`) + a stamina
  pool in `crates/combat`; ROW is a scoring rule (duplicate-kill tiebreak), so it
  belongs to the match rules, not the tick.

### 2.10 Damage vs. preparation time (Overgrowth)
- Source: Overgrowth 2009 design doc: `Attack = F(distance, prep_time, position)`
  -> (animation, damage); prep time has a **quick zone, an optimal zone and a late
  zone**, so a rushed strike is weak and an over-held strike loses power (and you
  get tired). Damage splits into **permanent** and **temporary** (recovers over
  time, so 5 hits in 5 seconds hurt more than 5 hits in 5 hours).
- Cost: one curve per move, one second pool ("temporary damage") per fighter.
- Depth: charging is not strictly better; it is a curve with a sweet spot, so
  timing beats holding the button.
- Our home: `crates/combat` frame data. Ties to 2.2 (Blade Symphony tiers choose
  *which move*, Overgrowth curves choose *how hard*).

### 2.11 VSIM: direct sword-arm control, and the Move Editor (Die by the Sword)
- Source: Die by the Sword (Treyarch, 1998) manual and reviews: full control of the
  sword arm (mouse or keypad), no pre-recorded animations, a physics model per
  avatar and weapon, damage from forces/momentum, **locational damage and limb
  loss** (a head hit can behead; a lost sword arm ends the fight), and a **Move
  Editor** where moves are recorded in three tracks (body, arm, off arm), tuned
  frame by frame, and four at a time are bound to keys per character.
- Two lessons, one good and one bad:
  - **Good**: the Move Editor is an AI-native authoring tool. A move is data: a
    short tape over three channels, with begin/end frame trims. Our
    `engine-anim` is a SoA clip sampler, and we already have input tapes. An
    "author a move" verb that writes a tape and a bound key is exactly P5.
  - **Bad**: VSIM's raw control was too hard to learn; the game shipped an
    "arcade mode" as the real interface. So: keep the deep table (Blade
    Symphony's), do not make the *input* the depth.
- Our home: `crates/endpoint` (a `combat.move.record` / `combat.move.bind` verb),
  `crates/anim` (the clip), `crates/combat` (the move table).

### 2.12 Exanima's principle: the AI plays by exactly the same rules
- Source: Exanima quickstart and dev diaries (read 2026-10-19): characters are
  driven by simulated muscles, not clips; "Non player characters are subject to
  the same exact rules as players and even simulate human reaction times";
  every nuance of input matters; hits depend on the weapon's contact point
  (axe head vs. shaft), momentum, and locational armour.
- Steal the **principle**, not the solver: an NPC top-level would use the same
  move table, the same windows, the same poise, plus a reaction-time delay. That
  is what makes an AI opponent feel fair *and* is trivially data-driven. It also
  means our `combat` fixture (AI vs AI) is the real balance harness.
- Do **not** steal: the muscle/ragdoll sim. It is float, unbounded, and cannot sit
  inside a fixed-tick hash; `physfx` is the crate that exists for that layer, and
  it is explicitly outside the hash.
- Home: `crates/ai` (an NPC policy over the move table) + `crates/combat`.

### 2.13 Manual block zones (the optional deep layer)
- Source: Movie Battles II: perfect block (PB) is directional — a swing hits from
  a direction, you block the matching zone; manual block (Mblock) adds a movement
  input plus timing; swingblock covers both. This is the layer above auto-parry.
- Cost: eight zones as data, one direction per incoming swing.
- Depth: the highest-skill layer; keep it *optional* so the game works without it.
- Our home: `exchange.rs` (add an optional zone to `TimedAnswer`).

### 2.14 The modding surface (Blade Symphony's balance on GitHub)
- Source: ModDB Blade Symphony news (Oct 2014): balance changes were taken as
  community pull requests against a public repo; cosmetics went to the Steam
  Workshop (589 player-made pieces).
- Steal: **balance is data in a repo, cosmetics are content**. If our move tables
  live in `crates/rules`/`content`, a human or an AI can retune balance as a
  one-line diff and we can prove it replay-exact.
- Home: `crates/rules` + `content/`.

## 3. The single best build order (cheap -> deep)

1. String counter + move table (2.1) — data only; unlocks everything else.
2. Charge tiers with a visible flash (2.2).
3. Stance matrix in `resolve_clash` (2.3) — three bits on what already exists.
4. Startup interrupt multiplier (2.7) — one boolean.
5. Momentum counter (2.8) — one integer.
6. Wide-vs-narrow tracer clash (2.4).
7. Down state + throw (2.5, 2.6) — the anti-turtle pair.
8. Optional manual block zones (2.13) for the top skill layer.

Every step is a table or a flag, so an AI can author it; every step adds a
decision, so a player can go deeper. That is the 100x.

## 4. What NOT to take

- Deriving animation from data (mocap) instead of clips: we are a hashed
  fixed-tick engine with an SoA clip sampler. Data-driven motion is a
  `physfx`-layer idea, and it has to stay out of the hash.
- Raw mouse-to-joint physics as the *only* interface (Die by the Sword's
  mistake): unlearnable, and unhashable in the tick.
- A stat-based progress system: Blade Symphony deliberately refuses it, and so
  should we.
