# fixtures/clips — baked motion clips

`stagger.*` is the AM27 motion-bake fixture: a seed-7 synthetic SOMA-77
stagger, retargeted onto the fighter-65 rig and packed to AM5P by
`tools/motion_bake`. It is authored data, not Kimodo model output (no model was
run); the NPZ uses Kimodo's array layout so a real teacher clip drops in.

| file | what | sha256 |
|---|---|---|
| `stagger.npz` | teacher source: 36 frames at 30 fps, `local_rot_mats`, `root_positions`, `posed_joints`, `fps`, `seed` | `2bc933c4175af98d2a39303b93ef97809ee8b04361d059c91321daf857d01fa1` |
| `stagger.am5p` | baked pack: AM5P v1, 61 ticks at 60 Hz, tracks `leftfoot`+`rightfoot`, 820 bytes | `f255f453a46d47f2327aead292b31197a7d179af714cd169243a3ba2c9e75d4e` |
| `stagger.receipt.json` | the bake receipt: `qclip_hash 0x34f72e4f8b72217a`, `ticks 61`, `avg_slip` 0.004975 / 0.102805, `plant_windows` 2/3, `retarget_digest 0x592af8cffb61ff84` | — |

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
