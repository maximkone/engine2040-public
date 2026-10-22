# 02 Feel: what to steal for melee and physics feel (2026-10-22)

Scope: the melee-feel gaps in `docs/plan/melee-feel.md` (weight, knockback, hitstop, armour,
particles and decals, clash, landing, impact sound, hit reaction, close contact) and the
rows FEEL1-FEEL11 in `docs/SPRINT-30-feel.md`. New rows use the prefix FEELX.

Method: WebFetch on raw files where a raw path existed (read on 2026-10-22), GitHub pages for
repo facts and directory listings, WebSearch for the rest. Anything not read is marked
UNVERIFIED. Nothing was cloned. Donor star counts are as the page showed them that day.

## 1. Verdict

Melee feel is not short of ideas; it is short of formulas we can port to integers. Most open
melee code lives in AngelScript (Overgrowth), in Go or a 2D fighter (Ikemen), or in float
engines (Jolt, Rapier, Box2D, Godot plugins). The donors below give the mechanism; we write
the integer version.

Three honest gaps: no open donor we read has a weapon mass (Veloren's tool stats have none),
no donor we read has a clash recoil (we design it from the contact event), and no donor we
read has a material-to-sound table (we derive it from contact impulse, see FEELX5).

Also: the tree already holds pieces of several rows. `crates/combat/src/heft.rs` says it is
the FEEL1 weapon-weight reader; `crates/physics/src/material.rs` carries `restitution_pm` and
the cue per material; `crates/combat/src/hitstop_kind.rs` is the per-kind hitstop table;
`crates/physics/src/clinch.rs` exists; `crates/audio/src/combat_cues.rs` exists. FEEL1 and
FEEL8 boxes are still unticked or partly ticked on the board, so check each row's own
Accept before claiming a steal is new. The steals below are the formulas those files lack.

Ranked: the knockback impulse (FEELX1), the armour math (FEELX2), and the contact-event
threshold (FEELX4) are the three to do first. They make FEEL2, FEEL4 and FEEL6 real.

## 2. Comparison table

| # | Donor | How read today | Repo facts as read | What it gives feel | Rows |
|---|---|---|---|---|---|
| D1 | Veloren (`veloren/veloren`) | raw `common/src/combat.rs`, raw `common/src/comp/inventory/item/tool.rs` | ~7.6k stars, pushed 2026-10-17 (prior note) | Knockback impulse with direction kinds; damage pipeline with armour and penetration; crushing and slashing poise; weapon stat sheet with durability scaling | FEEL1, FEEL2, FEEL4 |
| D2 | Jolt Physics (`jrouwe/JoltPhysics`) | raw `Jolt/Physics/PhysicsSettings.h`, raw `Jolt/Physics/Character/CharacterVirtual.cpp` | ~11.7k stars (prior note) | Restitution speed floor; speculative contact and slop constants; character sweep, step-down and stairs; capped contact impulses | FEEL7, FEEL10 |
| D3 | Box2D v3 (`erincatto/box2d`) | raw `include/box2d/types.h` | not read | Contact hit event above an approach-speed threshold, with point and normal; soft contact by Hz and damping ratio; overlap push speed cap | FEEL5, FEEL6, FEEL10 |
| D4 | Rapier (`dimforge/rapier`) | raw `src/pipeline/event_handler.rs`; GitHub page of `src/geometry/collider.rs` | not read | Contact force event carries the sum of force magnitudes (not the vector sum); collider friction and restitution combine rules default to Average | FEEL8, FEEL11, FEEL10 |
| D5 | Overgrowth (`WolfireGames/overgrowth`) | raw `Source/Objects/movementobject.cpp`; directory listings of `Source/Objects`, `Source/Physics`, `Source/Sound` | 2.9k stars, 193 commits, no date shown | Hit call carries direction and multipliers; material particle speed capped; blood surface sleeps; physics folder is Bullet, not Jolt | FEEL5, FEEL9 |
| D6 | Ikemen-GO (`ikemen-engine/Ikemen-GO`) | raw `src/char.go` (first chunk only); `src/` listing | ~1.6k stars, Go, MIT per README | HitDef derived fields (guard pause from hit pause; air-guard speed as fixed multiples); rollback fighter (`src/rollback.go`) | FEEL3 |
| D7 | saltmire-juice (`saltmire/saltmire-juice`) | GitHub page and README | 5 stars, 11 commits | `Juice.hitstop(seconds)` via time scale; nested calls: latest wins; trauma shake stacks | FEEL3 |
| D8 | Juicee (`Kelpekk/Juicee`) | GitHub page | 157 stars, 35 commits | Effect list: Hit Stop, Shake, Squash Land, Stretch, Burst (names only; no formulas read) | FEEL5, FEEL7 (names) |
| D9 | Active-Ragdoll-Character (`matieme/Active-Ragdoll-Character`) | GitHub page and README | 25 stars, 20 commits | Two global strength knobs (balance, core) feed the joint angular drive | FEEL9 |
| D10 | Godot-Active-Ragdolls (`R3X-G1L6AME5H/Godot-Active-Ragdolls`) | search snippet only | Godot 3.4 plugin | Torque-driven ragdoll on Generic6DOFJoint | FEEL9 (UNVERIFIED body) |
| D11 | Fxll3n's Fight Engine (Godot asset library) | search snippet only | Godot 4.5, 2D fighting plugin | Unknown; not read | none (UNVERIFIED) |
| D12 | OpenBOR, Paintown, OpenMUGEN | search snippets only | sprite beat-em-up engines | Not read; hit and knockdown code not checked | none (UNVERIFIED) |
| D13 | ashleve/ActiveRagdoll, sergioabreu-g/active-ragdolls | prior research (2026-10-11) | see prior file | PD joint drive; animation-following | already in prior steals |
| D14 | bevy_hanabi | prior research (2026-10-18) | ~1.4k stars | GPU particle spawn from events | already in prior steals |

Not found: "Castagne". A WebSearch for it returned no repository. Do not cite it.

Already mined, not repeated here: Veloren poise ladder and flank constants (steals-2026-10-18),
Tnua knockback shove (steals-2026-10-18), trauma shake (steals-2026-10-11), ozz two-bone IK and
look-at, motion matching (steals-2026-10-11).

## 3. Ranked steals

Every row below states: donor file, mechanism, FEEL row, the integer port, the target crate,
and a board line in the house style. Sim changes go under a new rules version; rules v1 and
golden `d57b5c8ae3096614` never move. Presentation rows never touch the hash.

### FEELX1 (rank 1). Knockback impulse by direction kind. Maps to FEEL2

- Donor: D1, `common/src/combat.rs`, `Knockback::calculate_impulse` (read 2026-10-22).
- Mechanism: `impulse = 50 * strength * f_resist * m_attacker * D(dir, kind)`. `f_resist` is
  0.5 when the target is knockback-resistant, else 1.0. `m_attacker` is the attacker's
  knockback multiplier. `D` is a direction chosen by kind: Away = slerp(dir, up, 0.5);
  Towards = slerp(-dir, up, 0.5); Up = (0,0,1); TowardsUp = slerp(-dir, up, 0.85). A zero
  impulse is dropped before it is emitted. The 50 is marked temporary in the source.
- Notes from the read: no mass term and no clamp in this function. Veloren has no weapon
  weight, so FEEL2's mass must come from `heft.rs` (weapon row) times swing speed.
- Integer port: store the four kinds as data (a small enum). Replace slerp by a linear blend
  of two integer vectors with integer weights out of 1024 (0.5 = 512; 0.85 = 870), then
  renormalise with an integer square root. Check `crates/core/src/det_math.rs` for an isqrt before adding one.
  Scale 50 becomes a named constant in the rules version.
- Target: `crates/arena/src/authority.rs` (replaces the constant `BALANCE_HIT_Q8` use),
  `crates/physics/src/balance.rs` (the impulse goes in), `crates/combat/src/heft.rs` (mass
  input).
- Board row: FEELX1 arena: a blow's knockback follows its direction kind and the struck
  body's resistance, not one constant. Accept: `laws::mirror` on a mirrored blow; the Up kind
  moves only on z; a resistant target moves exactly half of an ordinary one for the same
  blow; rules v1 golden unchanged.

### FEELX2 (rank 2). Armour order of operations: block flat first, reduction scaled by penetration, crushing turns reduction into poise. Maps to FEEL4

- Donor: D1, `common/src/combat.rs` `apply_attack` and `calculate_health_change` (read
  2026-10-22).
- Mechanism (as read): `d = value * damage_modifier`; precision adds `d * precision_mult *
  (precision_power - 1)`; the block decrement is subtracted flat, floored at 0; then
  `d = d * (1 - damage_reduction)`. Penetration: if the raw reduction is below 1,
  `reduction = (1 - attacker_penetration) * raw`. Crushing: `reduced = applied * dr / (1 - dr)`,
  then poise `= reduced * poise_damage_modifier`. A stunned target turns the poise loss into
  health damage.
- Integer port: all in permille. The division `dr / (1 - dr)` is integer division with the
  denominator clamped to at least 1 permille. Penetration is a permille too.
- Target: `crates/arena/src/authority.rs` `resolve_swing` (it takes worn armour 0 today),
  `crates/combat/src/cut.rs`, `crates/combat/src/anatomy.rs`.
- Board row: FEELX2 arena: worn armour changes the damage a blow does and the poise it
  costs, by one fixed order of operations. Accept: the same blow at an unarmoured and a
  plated torso: the plate takes less health and more poise; a penetrating blow recovers the
  lost share; rules v1 unchanged.

### FEELX3 (rank 3). Contact event above an approach-speed threshold, with point and normal. Maps to FEEL6 and FEEL5

- Donor: D3, `include/box2d/types.h`: `b2WorldDef.hitEventThreshold` ("threshold speed for
  hit events", m/s) and `b2ContactHitEvent` (`shapeIdA`, `shapeIdB`, `point`, `normal`,
  `approachSpeed`, always positive). The docs warn the point can trail the impact by up to
  one step when both bodies move.
- Mechanism: an event is emitted only when the approach speed passes the threshold. The
  event is the trigger; sparks, the metal clash cue and the FEEL6 recoil read it. A slow
  touch makes no event.
- Integer port: the threshold is an integer in mm/s. The event is computed in the tick's
  fixed order and queued, so the one-step lag is deterministic (name it in the test, do not
  hide it).
- Target: `crates/arena/src/collide.rs` (the swept blade and body contact),
  `crates/combat/src/exchange.rs` (clash), `crates/combat/src/feel.rs` (cue choice).
- Board row: FEELX3 combat: a blade or body contact raises one hit event only when it arrives
  faster than a threshold, with its point and normal. Accept: a slow touch raises none; a
  fast clash raises one event with the approach speed and normal; two runs give the same
  event tick.

### FEELX4 (rank 4). Restitution speed floor, so a resting body never bounces. Maps to FEEL10 and FEEL7

- Donor: D2, `Jolt/Physics/PhysicsSettings.h`: `mMinVelocityForRestitution = 1.0f` ("below
  this relative normal velocity, restitution is zero so objects settle sooner"); also
  `mSpeculativeContactDistance = 0.02`, `mPenetrationSlop = 0.02`, `mBaumgarte = 0.2`,
  `mNumVelocitySteps = 10` (the comment says at least 2 for friction to work).
- Mechanism: bounce applies only above a normal-speed floor. Below it the restitution is zero.
- Our tree: `crates/physics/src/material.rs` already carries `restitution_pm` (450, 200), and
  `crates/physfx/src/fx.rs` builds colliders with restitution. There is no speed floor.
- Integer port: the floor is an integer in mm/s applied in the hashed layer. Physfx (float)
  takes the same floor as a float constant only.
- Target: `crates/physics/src/material.rs`, `crates/physics/src/controller2.rs` (landing bounce),
  `crates/physfx/src/fx.rs`.
- Board row: FEELX4 physics: a body with normal speed under the floor takes no bounce, so a
  dropped weapon settles. Accept: `laws::rest_stays_rest` on a 0.5 m/s drop; a 3 m/s drop
  bounces with the material's restitution.

### FEELX5 (rank 5). Impact loudness and rumble from the sum of contact impulse magnitudes. Maps to FEEL8 and FEEL11

- Donor: D4, `src/pipeline/event_handler.rs`: the contact force event is built with
  `total_force_magnitude`, documented as "sum of magnitudes of all contact forces (not vector
  sum!)", with the example [0,100,0] and [0,-100,0] giving 200, not 0.
- Mechanism: the scalar that drives gain, pitch and rumble is the sum of magnitudes, so two
  equal and opposite pushes still read as hard hits.
- Integer port: per contact pair, sum of `|impulse|` in the tick's contact list (integer
  abs and add). The float physfx layer may sum floats for its own cue; the hashed layer does
  not need it.
- Target: `crates/audio/src/combat_cues.rs` (cue gain), `crates/game/src/windowed.rs` (pad),
  `crates/combat/src/feel.rs`.
- Board row: FEELX5 audio: the impact cue's gain and the pad strength read the summed contact
  impulse, not the speed of one blade. Accept: two equal opposite impulses read as their
  sum; a grab with a single push reads the same as before.
- Material note: no donor read gives a material-to-sound table. The cue choice stays
  `physics/material.rs` (`Material::cue`), the loudness comes from this sum.

### FEELX6 (rank 6). Landing is the air-to-ground transition, after a step-down snap. Maps to FEEL7

- Donor: D2, `Jolt/Physics/Character/CharacterVirtual.cpp` `ExtendedUpdate`: cancel velocity
  toward steep slopes, run `Update`, then when the character goes from supported to
  unsupported and is not moving up, `StickToFloor` sweeps down by `mStickToFloorStepDown`.
  `WalkStairs` sweeps up by the step height, moves, then sweeps down. `ground_state` and
  `ground_normal` drive the state.
- Mechanism: the land event is the transition tick, not a speed threshold. A step down the
  stairs is not a landing because the snap keeps the body supported.
- Integer port: the step-down distance is an integer in mm. The state is a two-value enum
  compared against last tick's value.
- Target: `crates/physics/src/controller2.rs` (the walker already reads the step height of
  the ground it stands on), `crates/game/src/lib.rs` (the land cue), `crates/arena/src/authority.rs`
  (fall damage table, FEEL7).
- Board row: FEELX6 game: one land event fires on the tick a falling fighter first touches
  the ground, after a snap down, and never on a step down. Accept: a 4 m drop fires one
  land; a walk off a 20 cm step fires none.

### FEELX7 (rank 7). Capped push-out and capped contact impulse. Maps to FEEL10

- Donor D3 `types.h`: `contactSpeed` ("controls how fast overlap is resolved, m/s; caps
  resolution speed"), `contactHertz` (stiffness) and `contactDampingRatio` (bounce of the
  correction). Donor D2 `CharacterVirtual.cpp` `HandleContact`: impulses to dynamic bodies
  are damped by 0.9 and clamped by `mMaxStrength`.
- Mechanism: contact correction has a speed cap and a damping; a shove never adds more
  speed than the cap.
- Our tree: `crates/arena/src/collide.rs` push-apart is positional only (melee-feel gap 10).
  `crates/physics/src/clinch.rs` exists, check its test status before adding.
- Integer port: the cap is an integer in mm per tick. Damping 0.9 is 230/256 in integer.
- Target: `crates/arena/src/collide.rs`, `crates/physics/src/clinch.rs`.
- Board row: FEELX7 physics: two fighters in contact exchange a capped push; a shove never
  adds speed beyond the cap. Accept: `laws::conserved` on a two-body push; the per-tick push
  never exceeds the cap; a resting pair stays at rest.

### FEELX8 (rank 8). Derived hitstop fields computed once at parse time. Maps to FEEL3

- Donor D6, `src/char.go` `HitDef.finalizeParams` (read 2026-10-22, first chunk): guard
  pause = hit pause when unset; air-guard velocity = air velocity times (1.5, 0.5, 1.5);
  down velocity inherits air velocity; guard X and Z inherit ground velocity. Defaults are
  set in `HitDef.reset`.
- Mechanism: one hit record, with the derived values filled once, so the step reads plain
  fields and never recomputes.
- Our tree: `crates/combat/src/hitstop_kind.rs` holds the kind table (3/6/8). The look mirror
  holds 4 (melee-feel gap 3). Make the look read the sim's derived value.
- Integer port: already integer. The 1.5 and 0.5 multiples become permille constants.
- Target: `crates/combat/src/hitstop_kind.rs`, `crates/game/src/hitstop.rs`,
  `crates/game/src/fighter_look_acts.rs`.
- Board row: FEELX8 game: the look's freeze for a hit is the sim's freeze for that hit,
  read from the same derived number. Accept: the test in FEEL3 (light 3, heavy 6, parry 8)
  plus a guard hit whose freeze equals the hit's freeze.
- Unverified: which side freezes (attacker, victim or both) in Ikemen's HitDef. The code
  read did not reach that path.

### FEELX9 (rank 9). Latest hit wins a freeze; nested freezes do not stack. Maps to FEEL3

- Donor D7, saltmire-juice README: "nested calls are safe, with the latest call taking effect"
  for `Juice.hitstop(seconds)`, which uses `Engine.time_scale`. Its shake note says repeated
  calls stack trauma (already mined as prior steal 3).
- Mechanism: one freeze at a time; a second hit replaces it. This is a rule choice, not math.
- Integer port: the freeze is a tick counter; a new hit sets it, never adds. Do not copy the
  time-scale method: the sim is fixed-tick, so the freeze is a count of ticks.
- Target: `crates/game/src/hitstop.rs`, `crates/combat/src/feel_flow.rs`.
- Board row: FEELX9 game: a second hit inside a freeze sets the freeze to its own length; the
  two never add. Accept: two hits two ticks apart give one freeze of the second hit's length;
  the look and sim agree (FEELX8).

### FEELX10 (rank 10). Hit reaction is data: one hit record with direction, attacker and both multipliers. Maps to FEEL9 and FEEL2

- Donor D5, `Source/Objects/movementobject.cpp` `ASWasHit`: the call passes the attack type,
  path, direction, position, attacker id, damage multiplier and knockback multiplier to the
  script `was_hit`. The formula is in AngelScript, not read (UNVERIFIED).
- Mechanism: one event record carries everything a reaction needs; the reaction is chosen
  from it, not from ad hoc reads of state.
- Integer port: a `HitRecord` struct of integers and an enum; stored in the sim's event list
  in tick order.
- Target: `crates/arena/src/live.rs` (the cues), `crates/combat/src/reaction.rs` (exists),
  `crates/physfx/src/fx/hit.rs`.
- Board row: FEELX10 combat: every hit leaves one record (direction, attacker, damage and
  knockback multipliers); the reaction and the knockback read that record. Accept: a replay
  of a duel gives the same records; `PhysFx::hit_ragdoll` gets its direction from the record.

### FEELX11 (rank 11). Two global strength knobs feed the joint drives. Maps to FEEL9

- Donor D9, `matieme/Active-Ragdoll-Character` README: "Balance Strength" and "Core Strength"
  apply to the Configurable Joint Angular Drive (README read 2026-10-22; C# paths and PD
  formula UNVERIFIED).
- Mechanism: two designer knobs for the whole body, mapped to per-joint drive gains.
- Integer port: the mapping is a table of integer gains per joint, scaled by the two knobs
  in permille. The per-joint table is data (`body.toml`, `crates/physfx/src/body.rs`).
- Target: `crates/physics/src/powered.rs`, `crates/physfx/src/body.rs`.
- Board row: FEELX11 door: a weak model tunes two numbers and the powered ragdoll's joint
  strength moves together. Accept: the two knobs change the per-joint gains by the table; a
  test sets both to 1000 permille and gets the table unchanged.
- Note: Godot-Active-Ragdolls (D10) is torque-driven on Generic6DOFJoint; body UNVERIFIED.
  Do not copy either PD law; our powered.rs is integer.

### FEELX12 (rank 12). Impact particles: speed capped at 1 m/s, spawn offset fixed below the bone. Maps to FEEL5

- Donor D5, `movementobject.cpp` `MaterialParticleAtBone`: particle velocity is
  `(0, min(1, |bone velocity|), 0)` and the spawn point sits 0.1 below on Y. Read 2026-10-22.
- Mechanism: a capped upward emission speed and a fixed offset so a fast limb does not fling
  particles.
- Integer port: `min(1000, |v|)` in mm/s; the offset 100 mm.
- Target: `crates/render/src/particles_gpu.rs` (the spawn is GPU; the count and cap come from
  the sim event, FEELX3), `crates/game/src/bin/arena_frame/mod.rs` (the fold, FEEL5).
- Board row: FEELX12 render: a hit's particles spawn at the contact point with the speed cap
  and count from the hit event. Accept: frame stats count spawns per hit; no particle moves
  faster than the cap.

### FEELX13 (rank 13). Blood decal sleeps after a time, instead of living forever. Maps to FEEL5

- Donor D5, `movementobject.cpp` `MovementObject::Update` (client branch): blood keeps
  updating while `game_time < blood_surface.sleep_time`. The `BloodSurface` body and the
  `MPCutPlane` cut were not read (UNVERIFIED).
- Mechanism: a decal has a sleep tick; after that it stops updating and stays fixed.
- Integer port: a decal stores a tick; stop updating at that tick. Presentation only, so the
  hash never sees it.
- Target: `crates/render/src/decals.rs` (named in the 2026-10-18 note; check it exists before
  wiring), `crates/game/src/bin/arena_frame/mod.rs`.
- Board row: FEELX13 render: a blood decal stops updating after its sleep tick and stays
  drawn; the frame stats count live decals. Accept: decal count rises on a flesh hit and
  holds flat after the sleep tick.

### FEELX14 (rank 14). Weapon durability scales speed and range down, equip time up. Maps to FEEL1

- Donor D1, `common/src/comp/inventory/item/tool.rs` `with_durability_mult` (read 2026-10-22):
  `less = dur * 0.5 + 0.5`; `equip_time /= less` (floor 0.01); `power`, `effect_power` and
  `buff_strength` multiply by the durability multiplier; `speed`, `range` and
  `energy_efficiency` multiply by `less`. The stat sheet has `equip_time_secs`, `power`,
  `effect_power`, `speed`, `range`, `energy_efficiency`, `buff_strength`.
- Mechanism: one stat sheet per item; wear acts on a named subset of its fields. Veloren has
  no mass, so this is a stat shape, not a weight.
- Integer port: `less` is permille: `dur_permille / 2 + 500`. Equip time divides by `less`
  with a floor.
- Target: `crates/combat/src/heft.rs` (weapon row reader), `content/design/weapons.rows`,
  `crates/game/src/class_sheet.rs`.
- Board row: FEELX14 combat: a worn weapon equips later and swings with less speed and range;
  a fresh weapon reads its row exactly. Accept: a test with durability 1000 gives the row
  values; a test with durability 0 gives the floor of each field.

## 4. What NOT to copy, and why

- Veloren's `50 * strength` knockback constant. The source calls it temporary. Keep a named
  constant under a rules version; do not carry the number.
- Veloren's parry line. The 2026-10-18 note already says the parry formula is probably a bug
  as written. Copy the intent only.
- Any float solver as the hashed layer. Jolt, Rapier and Box2D are float. Their constants
  (0.02 m slop, 0.2 Baumgarte, Hz stiffness) port as integer thresholds only; the solver
  stays in physfx, never in the hash.
- Juicee's 117 effects and Juice-style kits. They are float and time-scale driven. Take the
  effect names as a checklist; build the effects in our own event path.
- Godot `Engine.time_scale` freezes (saltmire-juice). Our sim is fixed-tick; a freeze is a
  count of ticks, not a wall-clock scale.
- Overgrowth's combat code as a model. Its physics folder is Bullet (`Source/Physics`:
  `bulletobject.cpp`, `bulletworld.cpp`), its hit and knockdown logic is in AngelScript, and
  the impulse formula was not reached. Take the call shape (FEELX10) and the particle cap
  (FEELX12); skip the rest.
- Ikemen's MUGEN HitDef as a data model. It is string-driven state scripting for a 2D
  fighter. Take the derived-field idea (FEELX8) only. Its `rollback.go` is a netplay answer,
  not a feel answer: not in scope for this wave.
- Active ragdolls with PD law in float (ashleve, sergioabreu-g, Godot-Active-Ragdolls). Our
  powered.rs is integer; take the knob layout (FEELX11), not the law.
- bevy_hanabi as the spawn source. Spawns come from the sim's hit event (FEELX3), counted in
  the frame stats; a GPU library does not decide when a hit happened.
- Anything with an unknown license or repo page we could not read (Fxll3n's, OpenBOR, Paintown,
  Castagne). Not cited, not copied.
- Shortcuts that turn the hit event into a counter the look reads on its own. The look must
  read the same record the sim wrote (FEELX8, FEELX10), or the hitstop truth gap stays open.

## 5. Board lines and order

Order: FEELX1 and FEELX2 (the knockback and armour numbers), FEELX3 (the event they both
need), then FEELX4 and FEELX6 (physics landing and restitution), FEELX8 and FEELX9 (one
hitstop truth), FEELX5 (sound and rumble from the sum), FEELX7 and FEELX10 (contact and the
record), FEELX11 to FEELX14 (look and door). FEELX14 pairs with FEEL1 because `heft.rs` is
already the reader.

Each row lands its own proof per the plan's rules: law tests from `engine_core::laws` for the
physics rows (momentum, rest, mirror); a counted frame stat for the look rows.

## 6. Sources read

- Veloren: `common/src/combat.rs`, `common/src/comp/inventory/item/tool.rs` (raw, 2026-10-22).
- Jolt: `Jolt/Physics/PhysicsSettings.h`, `Jolt/Physics/Character/CharacterVirtual.cpp` (raw).
- Box2D: `include/box2d/types.h` (raw, main branch).
- Rapier: `src/pipeline/event_handler.rs` (raw); `src/geometry/collider.rs` (GitHub page).
- Overgrowth: `Source/Objects/movementobject.cpp` (raw); directory pages `Source/Objects`,
  `Source/Physics`, `Source/Sound`, top-level `Source`.
- Ikemen-GO: `src/char.go` (raw, first chunk only); `src/` listing.
- saltmire-juice README; Juicee repo page; matieme Active-Ragdoll-Character README.
- Searches: Overgrowth ragdoll files, Castagne (not found), active ragdoll repos, game-feel
  libraries.
- Local: `docs/plan/melee-feel.md`, `docs/SPRINT-30-feel.md`, `docs/plan/finish-line.md`,
  `docs/research/steals-2026-10-11/anim-feel-steals.md`, `docs/research/steals-2026-10-18/`,
  `docs/research/hunt-2026-10-21/agent-3-physics-body.md`; crate files named above checked
  by existence (`crates/combat/src/heft.rs`, `crates/physics/src/material.rs`,
  `crates/combat/src/hitstop_kind.rs`, `crates/physics/src/clinch.rs`,
  `crates/physics/src/controller2.rs`, `crates/audio/src/combat_cues.rs`).
- Not in the tree any more: `crates/physics/src/kcc.rs` (named in the 2026-10-21 hunt; the
  walker is `controller2.rs` now).
