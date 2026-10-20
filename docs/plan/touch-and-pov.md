# Touch and POV: everything collides, bodies have weight, and a helmet-cam view

Owner 2026-10-20: "shouldnt we make all have collision in scenes by default? like now in arena cloth
no collide it goes through.. i want realistic hard to make games.. with crowds real touching,
leaning on ppl.. have center of gravity.. pov helmet view realistic and like the camera game swat
guys did with pov play and more tricks". Rows TC (touch) and POV (helmet cam) in
`docs/SPRINT-27-final-picture.md`. The engine half of the collision work (layer table, cloth
self-collision sets, swept test) is in `docs/research/powerups-2026-10-20/11-collisions-cloth-layers.md`.

## 1. The rule: collision is on unless a row says why not

- Every body, prop, cloth sheet, ragdoll bone, limb and debris piece collides with every other layer
  its mask accepts. The default mask is "everything that makes sense" (`layers::default_mask`).
  Opting out needs a named reason (a sensor, debris vs cloth, a cosmetic bit).
- Two worlds, one rule. What can change a match (fighter vs fighter, fighter vs wall, a shove) is
  integer and hashed, in the tick. What only paints (capes, ragdolls, debris, lens) is float and
  never hashed. A soft contact that matters for gameplay moves into the tick as its own row.
- A mesh with no collider is a content bug: the cook gives it a hull (TC2) unless the art says
  `NOCOL_`.

## 2. Where it stands (2026-10-21)

| Piece | Today | Gap |
|---|---|---|
| Cape vs bodies | fixed in this pass (TC1): the cape meets all 15 ragdoll bone capsules of its wearer plus every rival body within 1.6 m | world props and weapons do not push capes yet |
| Cloth solver | `crates/physics/src/cloth.rs`: many colliders, layers, swept test, self-collision sets, max-distance (landed 2026-10-20) | hair and rope (`Strand`) take no colliders |
| Fighter vs fighter | `crates/arena/src/collide.rs` pushes capsules apart, rules v2 only | a push never feeds balance; no lean |
| Centre of mass | `crates/physics/src/balance.rs`: CoM, capture point, stagger, stumble, fall, get-up; fed by landed blows | not fed by body pushes or crowd pressure |
| Crowd steering | `crates/nav/src/orca.rs` (integer ORCA, RVO2 port), 0 callers | no contact, no pressure, no lean |
| Ragdoll and debris | `crates/physfx` (Rapier float) | no group/mask table: layers do not reach Rapier |
| Mesh colliders | `crates/assets/src/collider_cook.rs` cooks `COL_` nodes only | an untagged mesh has no collider |
| First-person view | FV1 eye pose, FV2 look channel, FV5 trauma kick landed; FV3 feed, FV4 no-clip, FV6 tape open | no lens, no mount, no viewmodel, no second camera |

## 3. Touch (rows TC)

Steals, best first (read 2026-10-20, licences checked by the research agents):

| Steal | Source | Licence | What we take |
|---|---|---|---|
| Object-layer pair table | Jolt `Jolt/Physics/Collision/ObjectLayerPairFilterTable.h` | MIT | the profile table shape: one row per layer, default all-on |
| Interaction groups | Rapier `src/geometry/interaction_groups.rs` | Apache-2.0 | carry our group + mask into physfx as `InteractionGroups` (TC6) |
| Position-based crowds | Weiss et al., Computers and Graphics 78 (2019), DOI 10.1016/j.cag.2018.10.008 | paper | agents as discs with contact constraints solved by projection, the same loop as the cloth; ported to integer (TC5) |
| Social force overlap term | Helbing, PRE 51:4282 (1995); crush data PRE 75:046109 (2007) | paper | the body-compression force and the density where a crowd starts to shove |
| ORCA | RVO2 `src/Agent.cpp` (already ported in `orca.rs`) | Apache-2.0 | avoidance before contact (TC4) |
| Capture point | arXiv 1612.08034 (capture point x + xdot/omega) | paper | the stumble-step trigger from a push (TC3) |
| Overgrowth character | `WolfireGames/overgrowth` `Data/Scripts/aschar.as` | Apache-2.0 | tilt, target tilt and planted-foot flags for the lean look (TC7) |
| Jolt ragdoll motors | `Jolt/Physics/Ragdoll/Ragdoll.h` (`DriveToPoseUsingMotors`) | MIT | powered pose tracking under the lean, float layer only |

Rows:

- **TC1** cape meets every body (landed this pass, see the board for the proof).
- **TC2** the cook gives every untagged mesh a hull collider; `NOCOL_` opts out.
- **TC3** a body push becomes a shove: a push-apart displacement over a threshold in one tick feeds
  `BalanceState` like a light blow, so a packed crowd staggers and a hard shove can drop you. Rules
  v2 only; v1 and the golden stay.
- **TC4** wire ORCA: AI-driven bodies in rules v2 steer with `orca.rs` before contact.
- **TC5** crowd contact: integer position-based contact for 64 to 512 discs (one pass after ORCA),
  pressure per agent as a number, a crush threshold that triggers TC3.
- **TC6** physfx gets the layer table: ragdoll bones, limbs and debris carry group + mask as Rapier
  interaction groups, so a falling body lands on a crate and debris never pushes a cape.
- **TC7** lean and brace look: pressure and push direction tilt the upper body away and put a hand
  on the nearest shoulder (two-bone IK), presentation only.

Not now: a fully physical active ragdoll in the tick (Euphoria class). The integer balance plus
the float powered ragdoll is the line; a learned controller (MuJoCo, offline) is a frontier spike.

## 4. Helmet-cam POV (rows POV)

Target feel: Bodycam and Unrecord (lens and mount), Ready or Not and SWAT 4 (tactical tricks),
Arma and Tarkov (weight). No open-source bodycam shader was found; the parts below are standard and
the numbers start as a table tuned by fixtures.

| Steal | Source | Licence | What we take |
|---|---|---|---|
| RGB shift, vignette, film grain | three.js `examples/jsm/shaders/RGBShiftShader.js`, `VignetteShader.js`, `FilmShader.js` | MIT | the three formulas in one WGSL lens pass; CA starts at 0.005 |
| Damped spring | `Hyphinett/GodotSpringDamper` `addons/springdamper/` | MIT | the mount and viewmodel inertia spring; we already own halflife springs in the fight camera, reuse them |
| Sway split | Arma 3 OPREP "weapon sway and fatigue" (dev blog) | ideas | breath and grip as separate forces; breath hold cancels breath; prone < crouch < stand |
| Second camera to a texture | Godot `SubViewport` docs | ideas | optiwand under a door and teammate helmet cams, throttled per camera |
| Dynamic FOV restrictor | Fernandes and Feiner, IEEE VR 2016 | paper | narrow the FOV a little during fast motion, as an option |
| Stable aim, moving view | Bodycam design notes | ideas | the aim point stays steady while the mount shakes; only the view and the gun move |

Starting numbers (tune with fixtures, never by hand in Rust):

| Knob | Start | Note |
|---|---|---|
| `fov_deg` helmet / chest | 90 / 105 | eye view keeps FV's table |
| `barrel_k1` | 0.08 | 0 = off |
| `ca_amount` | 0.005 | uv offset |
| `vignette` | 0.35 | darkness at the corner |
| `grain` | 0.04 | luma noise |
| `mount_halflife_s` helmet / chest | 0.06 / 0.10 | chest shakes more |
| `sway_halflife_s` | 0.12 | viewmodel lag behind the look |
| `breath_amp_deg` | 0.15 | 0 while holding breath |
| `rtt_hz`, `rtt_px` | 15, 512 | per second camera |

Rows:

- **POV1** lens pass: barrel, CA, vignette, grain in one post pass with a lens table (eye, helmet,
  chest); off by default.
- **POV2** mount: the camera rides a spring on the head or chest bone; the aim ray does not.
- **POV3** viewmodel weight: spring sway from look speed, breath, grip; breath hold.
- **POV4** second camera: render-to-texture for an optiwand and teammate cams, throttled, with a
  budget line.
- **POV5** flashbang and suppression: white-out decay, low-pass plus ring in the mix, blur at the
  edges under fire.
- **POV6** comfort: FOV restrictor and sliders for shake, bob, CA, grain and FOV; defaults low.
- **POV7** proof: a 600-tick helmet-cam tape with lens on and off captures and a receipt.

Lean, peek and the gun pulling back near a wall belong to the infantry layer (UW11) and the no-clip
rule (FV4); POV rows wait for FV3 and do not duplicate them.

## 5. Decisions (owner 2026-10-21: "yes u do all decide on em")

- Default view is the helmet, for urban war and for the legionary sim (`rome.md`). The
  chest mount stays a lens preset.
- TC3 is armed in rules v2 like AIN6: a v2 hash may move, with a receipt under `docs/receipts/`;
  rules v1 and the golden `d57b5c8ae3096614` do not.
- `nav/orca.rs` is wired (TC4). `physics/kcc.rs` is f32 (43 `f32` uses), so it cannot drive the
  hashed tick: its slope limit and ground snap are ported into the integer controller for dug
  ground (LEG6), and then the float file is deleted.
