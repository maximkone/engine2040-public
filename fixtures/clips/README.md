# fixtures/clips — baked motion clips

Two kinds of clip live here, and the difference matters:

- `stagger.*` is the AM27 motion-bake fixture: a seed-7 **synthetic** SOMA-77
  stagger, retargeted onto the fighter-65 rig and packed to AM5P by
  `tools/motion_bake`. It is authored data, not Kimodo model output (no model
  was run); the NPZ uses Kimodo's array layout so a real teacher clip drops in.
- `combat/` holds four **real teacher clips** (an overhead strike, a spear
  thrust, a parry and a horizontal slash), baked by the same tool through the
  same name table. Their provenance and licence are in
  `combat/PROVENANCE.md`.

| file | what | sha256 |
|---|---|---|
| `stagger.npz` | teacher source: 36 frames at 30 fps, `local_rot_mats`, `root_positions`, `posed_joints`, `fps`, `seed` | `2bc933c4175af98d2a39303b93ef97809ee8b04361d059c91321daf857d01fa1` |
| `stagger.am5p` | baked pack: AM5P v2, 61 ticks at 60 Hz, one track per fighter joint (65) plus one rotation track per joint, 54813 bytes | `a9204ea95e00f9c0b2339cbea3674446c9797b5a17b2052f0e7cf4d9d16acbdc` |
| `stagger.receipt.json` | the bake receipt: `qclip_hash 0xf5f4de57dd5a7280`, `ticks 61`, `track_count 65`, `avg_slip` 0.024264 / 0.496259, `plant_windows` 0/0, `retarget_digest 0x0f326636a24f4f30` | — |

Regenerate (same bytes):

```
cargo run --manifest-path tools/motion_bake/Cargo.toml -- synth --seed 7 \
    --out fixtures/clips/stagger.npz
cargo run --manifest-path tools/motion_bake/Cargo.toml -- bake \
    --source fixtures/clips/stagger.npz --name stagger \
    --out fixtures/clips/stagger.am5p \
    --receipt fixtures/clips/stagger.receipt.json
```

The teacher's last pinned frame is tick 60 of 70, so the bake is 61 ticks: the
10-tick trailing tail is cut by the endpoint rule, never by a fixed duration.

## The pack carries the whole body, and the rotations

A pack holds one track per fighter joint (65), in the rig's own joint order,
and the receipt names them. A version-2 pack also carries one rotation track
per joint, so a combat clip shows its swing: the arm turns instead of sliding
on the rig's bind rotation. The pack's byte budget is 2 MiB per fighter; the
combat packs are 84–104 KB.
