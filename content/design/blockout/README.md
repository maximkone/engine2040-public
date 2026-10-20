# Body archetypes — blockout specs v0

Six JSON specs, one per body archetype. They are the single source of the
proportions for `tools/blender/blockout.py`. Numbers only, no mesh data: the
generator reads these files and builds primitives from them.

- `wardens_heavy.json` — human shield line. Broad but upright. A wall.
- `wardens_light.json` — human rogue and archer line. Slim, tall, thin.
- `wardens_caster.json` — human mage and cleric line. Robe flare, staff.
- `ashkin_heavy.json` — Ashkin templar and attacker line. Huge shoulders,
  short thick legs.
- `ashkin_gunner.json` — Ashkin gunner. Arm cannon right, smaller off arm
  left, widest stance.
- `ashkin_caster.json` — Ashkin officiator line. Medium frame, cape, staff.

Humans are light, upright and cool. Ashkin are dark, top-heavy and warm.
The humans read as a slim vertical line, the Ashkin as a dark triangle on
two legs. The two silhouettes must never overlap.

## Proportion table

| archetype | heads | height m | shoulder span m | leg len m | hand m | stance deg |
|---|---|---|---|---|---|---|
| wardens_heavy | 7.5 | 1.82 | 0.50 | 0.88 | 0.19 | 4 |
| wardens_light | 7.5 | 1.86 | 0.40 | 0.90 | 0.17 | 6 |
| wardens_caster | 7.5 | 1.78 | 0.35 | 0.86 | 0.17 | 5 |
| ashkin_heavy | 6.0 | 2.05 | 0.98 | 0.86 | 0.24 | 7 |
| ashkin_gunner | 6.0 | 2.00 | 0.95 | 0.95 | 0.22 | 9 |
| ashkin_caster | 6.0 | 1.95 | 0.72 | 0.94 | 0.21 | 6 |

The head count is the contract: humans are 7.5 heads tall, Ashkin 6.0.
Head unit is `height_m / heads`. `head_size` is the head box height in
metres, about 0.9 of one head unit. `shoulder_span` is the full outer
shoulder width in metres, across both pauldrons: Ashkin are roughly three
head widths, humans about two. `leg_len` is hip joint to floor, `arm_len`
is shoulder to wrist, `hand` is hand length, `bulk` scales the limb and
torso thickness (1.0 is the gunner blockout baseline), `stance_deg` is the
foot spread.

## Field contract

Every file has exactly these keys, in this order, with these names:
`name`, `height_m`, `heads`, `shoulder_span`, `torso_depth`, `chest_w`,
`waist_w`, `hip_w`, `leg_len`, `arm_len`, `hand`, `head_size`, `bulk`,
`stance_deg`, `attachments`. All lengths are metres, all angles degrees.

Each attachment is `kind`, `side`, `length`, `radius`, `angle_deg`.
Allowed kinds: `arm_cannon`, `shield`, `staff`, `bow`, `quiver`, `cape`,
`blade`, `none`. Allowed sides: `l` (left), `r` (right), `b` (back, for a
cape or a quiver worn on the back).

- `arm_cannon` — `length` is the barrel length, `radius` the barrel
  radius, `angle_deg` the tilt of the barrel from vertical. The gunner
  uses 66, the value that already works in the blockout. A second
  `arm_cannon` with side `l` is the smaller off arm.
- `shield` — `length` is the shield height, `radius` its half width,
  `angle_deg` the tilt of the shield face from vertical.
- `blade` — `length` is the blade length, `radius` its half width,
  `angle_deg` the tilt from vertical in the hand.
- `staff` — `length` is the staff length, `radius` the shaft radius.
- `bow` — `length` is the bow arc length, `radius` the limb thickness.
- `quiver` — `length` is the quiver length, `radius` the tube radius.
- `cape` — `length` is the cape drop from the shoulders, `radius` its
  half width at the hem.

## The 90 pixel rule

Every archetype is rendered as a flat black silhouette on white at 90
pixels tall, side by side, with no colour and no texture. At that size:

1. Faction reads first. Human: a slim vertical line. Ashkin: a triangle
   on two legs, top-heavy. If the Ashkin does not read as a triangle, the
   proportions are wrong.
2. Class reads second. The gunner shows one long diagonal barrel
   crossing the body. The shield lines show the shield edge. The casters
   show the staff line. The archer shows the bow arc.
3. Nothing new may appear that is not in the spec. A silhouette that
   needs a texture to be understood is a failed silhouette.

A silhouette that fails at 90 pixels but passes at 512 pixels has failed.
Scale the frame, not the model.

## Notes

- The gunner is the reference build. Its numbers come straight from
  `tools/blender/gunner_blockout.py`, which already renders.
- The heavy and light Wardens share one head count, so the only real
  difference between the human lines is width: 0.50 shoulder span
  against 0.40.
- The rogue branch of `wardens_light` swaps the bow for a pair of short
  blades. The body numbers do not change.
- Robe flare on `wardens_caster` is carried by `hip_w` (0.48) sitting
  well above `waist_w` (0.28). The generator flares a skirt from the
  waist down.
