# motion_bake — teacher clip in, `.am5p` out (AM27)

Offline bake, never in the tick. Reads a Kimodo NPZ teacher clip, retargets it
onto the fighter-65 rig through `engine-assets` (`soma_pairs()`, `BoneMap`,
`retarget_cook`), takes the clip length from the last constrained frame
(the PartiGen endpoint rule, `engine-anim::motion_bake::bake_len`) and packs
the two foot lanes to AM5P through the one pack home (`bake_pack`).

Not a workspace member: this folder has its own `[workspace]`, so
`cargo test -p engine-anim` and every tick gate never build it. It is a desk
tool that writes a committed fixture.

## Commands

```
# the seed-7 synthetic SOMA-77 stagger in Kimodo's NPZ layout
cargo run --manifest-path tools/motion_bake/Cargo.toml -- synth \
    --seed 7 --out fixtures/clips/stagger.npz

# bake it: retarget, endpoint rule, pack, receipt
cargo run --manifest-path tools/motion_bake/Cargo.toml -- bake \
    --source fixtures/clips/stagger.npz --name stagger \
    --out fixtures/clips/stagger.am5p \
    --receipt fixtures/clips/stagger.receipt.json
```

Both print one line with every number the gate reads (`qclip_hash`, `ticks`,
`avg_slip`, `retarget_digest`, sha256 values). `synth` and `bake` are
deterministic: the same seed and source give the same bytes on every run.

## Input: Kimodo NPZ

`numpy.savez_compressed` arrays, read by `src/npz.rs` (stored and deflated zip
entries, little-endian `f4/f8/i4/i8/u1/b1`, C order):

| array | shape | meaning |
|---|---|---|
| `local_rot_mats` | `(N, 77, 3, 3)` or `(1, N, 77, 3, 3)` | parent-relative rotation matrices |
| `root_positions` | `(N, 3)` or `(1, N, 3)` | root world positions, meters |
| `posed_joints` | `(N, 77, 3)` or `(1, N, 77, 3)` | world joint positions, meters |
| `fps` | `()` | teacher frame rate (30 for Kimodo) |
| `seed` | `()` | recorded seed (optional; goes into the receipt) |

Names are SOMA-77 (`engine_assets::soma::SOMA77`). Unknown entries (a Kimodo
`prompt`, `global_rot_mats`, `smooth_root_pos`) are ignored.

Translation convention: the teacher's positions are **world** joint positions,
so the retarget transfers the world delta onto each fighter joint's local bind
— the lane the contact and slip meters read. Rotations ride as bind-relative
local deltas, which is what the retarget correction expects. This is written
down in `src/bake.rs`.

## Output

`<name>.am5p` (AM5P version 1, two position tracks: `leftfoot`, `rightfoot`,
sampled at 60 Hz for `ticks` frames) plus the receipt JSON: `qclip_hash`,
`ticks`, `dur_s`, `track_count`, `avg_slip` per foot, `plant_windows`,
`retarget_digest`, `seed`, `source_sha256`, `bytes_sha256`, `packed_bytes`.

## Follow-ups (honest gaps)

- SOMA BVH input (`somaskel77` hierarchy, centimetre root) is not read yet; the
  NPZ path is the primary one. A BVH carries translation only on the root, so
  its foot lanes would be constant and the contact table degenerate — the BVH
  reader needs the world-joint conversion the NPZ path already does.
- The fighter rig is this tool's profile (`src/rig.rs`): SOMA proportions with
  the ankle bone stretched 1.2x so the bind-scale path is exercised. A real
  fighter rig profile in `engine-assets` replaces it.
- Full-pose lanes (65 joints, rotation tracks) are not packed; the clip carries
  the two foot lanes the slip meter and the sampler gate read.
