# Melee feel and the game-building door (owner 2026-10-21)

Owner, 2026-10-21: "physics like real in games: if hit with a sword then it feels hit for real
and lands properly, feel weight of stuff, weapons, melee, real close collision with real
physics; we also lack particles"; "there's nothing playable, just a demo scene"; "I want to
build games soon with weak models there".

Two measured inventories back every row here (both read the tree on 2026-10-21, nothing
guessed). Rows live in `docs/SPRINT-30-feel.md`; FOCUS "## Now" items 3 and 5 name them.

## What the live duel does today

The window runs `App::into_duel` (combat tables v2, arena rules v2): swept blades against body
capsules, clash and riposte poise, severing, balance gait (stagger, stumble, fall), a
knockdown ragdoll, additive flinch poses, a per-kind sim hitstop, camera shake and a lens punch.

What it does not do, by measured gap, most important first:

1. **Weapons weigh nothing.** One attack table for every weapon (`tables_text.rs` V2: startup 15,
   recovery 8). `content/design/weapons.rows` has damage, speed, stagger and recovery per weapon
   but only a test reads it; `props.json` has no mass.
2. **A blow pushes the same whatever hits.** The victim takes a fixed balance step
   (`BALANCE_HIT_Q8`); no knockback velocity exists, so a mace and a dagger move a body alike.
3. **Three hitstop truths.** The sim freezes 3/6/8 ticks by kind, the look mirror holds 4, the
   feel layer stops 4 frames at 10%: the freeze reads differently on screen than in the sim.
4. **Armour never applies.** `resolve_swing` gets worn armour 0 (`arena/src/authority.rs`).
5. **No hit particles.** Nothing calls the GPU particle pool's `try_spawn`; its emitters sit at
   the world origin and it draws on probe frames only; decals and the weapon trail are unwired.
6. **A clash has no physical answer.** Poise only: no blade recoil, no spark, no held bind.
7. **No landing.** Jump and fall run only behind `--height`; no squash, camera dip, land sound,
   dust or fall damage.
8. **Every hit sounds the same.** One HIT cue at a fixed gain; the App mirror hard-codes Flesh.
9. **Hits flinch the look only.** `PhysFx::hit_ragdoll`, the powered ragdoll (stable since
   061269529) and the stagger-memory limp have no presentation caller.
10. **Close contact is kinematic.** Push-apart only; clinch runs in physics tests only.

## The game-building door (weak models)

The MCP audit drove `e2040-mcp` as a weak model would (report in this session's commit
741b19648 runbook). Gaps, most important first:

1. **No game objects.** The level grammar knows ten furniture props; no player, enemy, goal,
   floor, spawn or rule. A weak model can stage a room, never a game.
2. **Nothing persists.** Edits live in memory; a fresh process starts at revision 0.
3. **The authored scene is not what plays.** `play.step` takes no scene revision and shows the
   default duel.
4. **Refusals do not say what to fix.** `spatial_rejected: overlap` names no pair; the kinds
   list is not in the tool description; the `mx` schema says int8 while the handler takes ±100.
5. **`api.load` breaks the tool budget** and still reports loaded.
6. **A stale installed bridge** (2026-09-26) failed every apply (a JSON hash above 2^53); the
   10/20 build is now installed; the install must refuse a stale copy.

## Donors (research 2026-10-22, `docs/research/finish-line-2026-10-22/02-feel.md`)

Open melee code is AngelScript, Go or float physics, so donors give the mechanism and we write the integer version. FEEL1 landed (`1843106`): each weapon row has its own startup, recovery, damage, stagger and mass.

- **FEEL2 knockback** (Veloren `common/src/combat.rs`, read by the lead): impulse = base x strength x target resistance (1/2 when resistant) x attacker multiplier x a direction blended toward up (1/2 for away and towards, 0.85 for towards-up). Our strength is weapon mass x swing speed; the blend is integer weights out of 1024 with an integer square root to renormalise. A capture-point choice picks stagger, step or fall (ARX01).
- **FEEL3 one hitstop:** derived hit fields computed once at parse time (Ikemen HitDef), and the latest hit replaces a freeze, never adds to it.
- **FEEL4 armour order** (Veloren): flat block first, then reduction scaled by penetration; a crushing blow turns the blocked part into poise damage. All permille.
- **FEEL5 and FEEL6 trigger:** an event only above an approach-speed threshold, carrying point and normal (Box2D `hitEventThreshold`, `b2ContactHitEvent`); sparks and the clash recoil read it. Particle speed capped at 1 m/s with a fixed spawn offset (Overgrowth).
- **FEEL7 landing:** the land event is the air-to-ground tick after a step-down snap, not a speed test (Jolt character controller).
- **FEEL8 and FEEL11 loudness and rumble:** the sum of contact impulse magnitudes in the tick drives gain, pitch and rumble.
- **FEEL9 hit reaction:** one hit record (direction, attacker, both multipliers) picks the reaction; two designer strength knobs map to per-joint drive gains on the integer powered servo.
- **FEEL10 contact:** a restitution speed floor so a resting body never bounces (Jolt `mMinVelocityForRestitution`, 1 m/s), and a capped push-out per tick with damping 230/256.
- Still to read: Overgrowth's AngelScript character scripts (`Data/Scripts/`), where its melee and active-ragdoll blend live; the report read only the C++ side.

## Rules for these rows

- Sim changes (weight, impulse, armour) go under a new rules version; rules v1 and the golden
  `d57b5c8ae3096614` never move. Presentation rows never touch the hash.
- Every physics row proves its laws with `engine_core::laws` (skill `physics-bodies` 3b):
  momentum into the target, energy never rising after a blow, mirror symmetry of a mirrored hit.
- Every look row lands a capture or a counted proof (spawned particles per hit, frames held),
  not "looks better".
