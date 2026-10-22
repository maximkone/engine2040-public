# PROVENANCE - fixtures/clips/combat

The four packs here are **real teacher output**, not authored data. They are
Kimodo (NVIDIA) motion clips, converted from the teacher's own SOMA BVH into
the teacher's NPZ layout, then baked by `tools/motion_bake` onto the
fighter-65 rig and packed to AM5P.

The source clips live in the empire monorepo, card
`lab/research/research/cards/kimodo-trial-2026-10-20/clips/`, with their own
provenance sidecars. This folder holds the baked packs and their receipts: the
engine's own artifacts, so the engine can play the motion without the teacher
or the empire repo.

## The clips

| pack | source clip | prompt | seed | steps | ticks | slip (left/right) |
|---|---|---|---|---|---|---|
| `slash_over.am5p` | `slash_over` | A person raises a sword and brings it down in a strong overhead strike. | 71235 | 100 | 119 | 0.003739 / 0.002573 |
| `spear_thrust.am5p` | `spear_thrust` | A person lunges forward and thrusts a spear. | 31236 | 100 | 95 | 0.015842 / 0.016747 |
| `parry.am5p` | `parry` | A person parries an attack with a sword and then strikes back. | 71234 | 100 | 119 | 0.387609 / 0.330258 |
| `slash_h.am5p` | `slash_h` | A person swings a sword in a fast horizontal slash. | 31234 | 100 | 95 | 0.646281 / 0.561439 |

Each pack is AM5P version 2: 65 position tracks (one per fighter joint) and 65
rotation tracks, so the arm turns over the swing instead of sliding on the
rig's bind rotation. `slash_over` and `parry` are 103823 bytes,
`spear_thrust` and `slash_h` are 83543 bytes, against the 2 MiB per-fighter
budget.

The slip numbers are the whole-clip average speed of the foot over the
ground in m/s, measured on the foot's **world** position from the retarget's
own world output, so a planted foot reads near zero and a sliding one reads
its real speed over the sand. They are also the numbers the empire card
attested for these clips, reproduced to the sixth decimal by the engine's
own bake: the card's receipts and the engine's receipts agree because both
came from the same teacher data through the same name table.

`slash_over` and `spear_thrust` slide under 8 cm/s. `parry` sits over the
engine's 0.08 gate and `slash_h` is further over it: those two are carried
with their numbers visible, not tuned into passing.

## Teacher

- Model: `nvidia/Kimodo-SOMA-RP-v1.2` (282 M parameters, SOMA-77 skeleton).
- Text encoder: LLM2Vec on Meta-Llama-3-8B-Instruct, run once into a cached
  embedding; the encoder never runs here.
- Frame rate: 30 fps in, baked at 60 Hz.

## Licence

- Weights: "Licensed by NVIDIA Corporation under the NVIDIA Open Model
  License". Commercial use is allowed; the agreement travels with the weights.
- Text encoder: Meta Llama 3 Community License. Built with Meta Llama 3.
- The SMPL-X variant is under the R&D licence, not the Open Model License, and
  is not used anywhere in this folder.

## Regenerate

The teacher clip's BVH goes to the teacher's NPZ layout, then the engine's own
bake writes the pack and the receipt:

```
python -m kimodo.scripts.motion_convert <clip>.bvh <clip>.npz --from soma-bvh --to kimodo
cargo run --manifest-path tools/motion_bake/Cargo.toml -- bake \
    --source <clip>.npz --name <clip> \
    --out fixtures/clips/combat/<clip>.am5p \
    --receipt fixtures/clips/combat/<clip>.receipt.json
```

The NPZ the tool reads carries `local_rot_mats`, `posed_joints` and
`root_positions` in C order; the teacher's own NPZ already is, and a converted
file is rewritten with C-contiguous arrays before the bake.
