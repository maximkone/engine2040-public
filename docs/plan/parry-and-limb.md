# The duel reads like a fight, and a lost guard costs a limb

Plan for row WEB16 (presentation) with QR12 (the severing payoff). It takes one
cool mechanic the plans already carry — the v2 exchange: parry window, locked
riposte, clash over `crates/combat/src/exchange.rs` — and joins it to the owner's
recent asks: play it in the browser, make it read, destruction and dismembering,
"the arena looked meh".

## The idea

The page already runs these rules: `App::into_duel()` moves a fresh session to
`ArenaRules::v2_free_for_all()`, and the step emits `FightCue`s for `Hit`,
`Blocked`, `Parried`, `Dodged`. None of it reaches the picture: the page calls
`live_fight_dots(&self.app, &[])` with an empty cue slice, so a perfect parry and
a plain hit look identical, and there is no stamina or poise on screen.

The combination: a **guard break**. Land the parry (6-8 tick window), take the
riposte (locked, 140 permille, 8 tick stagger), or break the opponent's poise
(a heavy or two lights inside 60 ticks) — and the loser does not just lose hp,
they lose an arm. The arm leaves the body as its own group (QR12), falls into the
debris layer the arena already paints (RZ4), and the fighter keeps fighting one
handed, which changes what it may block with.

Skill -> read -> spectacle: parry, freeze, riposte, limb, debris. All of it from
the sim's own events, none of it scripted.

## Steps

- [ ] 1. Cues reach the picture (WEB16, `crates/web` + `crates/render`): pass the
      tick's `FightCue`s into the dots, flash per kind (parry bright ring, block
      dim spark, hit red, dodge after-image), and freeze the body for the
      cue's hit-stop ticks.
- [ ] 2. HUD (WEB16): hp bars, stamina bar, poise pips, and a short word on the
      exchange (`PARRY`, `RIPOSTE`, `CLASH`, `BREAK`), names, best-of-5 pips.
- [ ] 3. The break severs (QR12): a parry or poise break raises a limb-loss event;
      `crates/physfx/src/fx.rs` detaches that limb as its own group with a stump
      cap; the parent keeps its 14 bones.
- [ ] 4. The page paints it: the severed limb draws as debris, the stump reads,
      and the hurt seat's move set loses the guard that arm carried.
- [ ] 5. Proof: golden `d57b5c8ae3096614` unchanged (both runs); `cargo test -p
      engine-combat --lib exchange`; `cargo test -p engine-physfx --lib`; the
      page at 60 fps with a capture that shows a parry flash, a stamina bar and a
      severed limb beside its owner.

## Where it lives

- `crates/combat/src/exchange.rs` — parry window, riposte, clash (already landed; read only unless the break rule belongs here).
- `crates/physfx/src/fx.rs` — detach the limb, stump cap, debris group (QR12).
- `crates/game/src/ragdoll_look.rs` — the look side of the severed limb (QR12).
- `crates/web/src/fight.rs`, `crates/render/src/procedural_arena.rs` — cues, HUD, camera shake, debris draw (WEB16).
- `crates/arena` — the rule that turns a break into a limb event; arena stays the step owner, combat the resolver.

## Out of scope (named, not hidden)

Character meshes: the fighters are still procedural boxes until the cooked
character assets land (QW6 mesh compression, WEB6 fetch). The mechanic, the
read and the debris are real; the art is stand-in.
