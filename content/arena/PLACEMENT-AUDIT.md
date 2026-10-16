# CT81 placement + dressing joint SP5 audit

Row CT81. Joint audit of the staged arena set: 11 placement actors plus
6 dressing actors, checked as one set through the spatial offline gate.

## Inputs (frozen)

- `content/arena/placement.json` (11 actors, ids 1-11)
  SHA256 `E7982F89395368DBDC507F46C31E91AB85501C624E189315624A86813D5CD372`
- `content/arena/dressing.json` (6 actors, ids 101-106, CT53 layer)
  SHA256 `EA4886E3EAB80424ED7602C72098CD72C49B134CB5586C1305F6E29ADF4DA970`
- No new bytes staged. No engine hunks. Both JSON files frozen by this audit.

## Actors (meters, center x/y/z with full-extent sx/sy/sz)

Placement: sand_floor(-60,0.1,0 30x0.2x30), stands(-20,1.5,0 12x3x6),
wall_north(-2,1.5,-8 8x3x1), wall_south(-2,1.5,8 8x3x1),
gate_tunnel(12,2.5,0 6x5x3), torch_west(20,1,-3 0.5x2x0.5),
torch_east(20,1,3 0.5x2x0.5), banner_west(24,1.5,-3 1.6x3x0.6),
banner_east(24,1.5,3 1.6x3x0.6), crate_a(29,0.5,0 1x1x1),
debris_field(34,0.25,0 2.5x0.5x2.5).
Dressing: torch_approach_w(5.5,1,-2 0.5x2x0.5),
torch_approach_e(5.5,1,2 0.5x2x0.5), banner_court_w(17,1.5,-2 1.6x3x0.6),
banner_court_e(17,1.5,2 1.6x3x0.6), banner_far_w(26,1.5,-2 1.6x3x0.6),
banner_far_e(26,1.5,2 1.6x3x0.6).
Every actor rests on its footprint (y equals sy/2). Footprints use
absolute meters (x +/- sx/2, z +/- sz/2), all inside +/-99 m.

## Gate

Command (lane `builder-content-ct81`, pool flags from `lanes.mjs create`):

`cargo test --manifest-path .lanes/builder-content-ct81/Cargo.toml
--target-dir target/pool-1 -j 4 --locked -p engine-spatial
--test ct81_probe -- --nocapture`

The probe built the 17-actor joint set with the values above, ran the
spatial offline gate `run()` on the forward order and the reversed order,
and asserted an empty finding list.

Result:

`actors=17 findings=0 verdict=clean hash=cbf29ce484222325`
`test ct81_joint_audit_probe ... ok`

Order check: forward and reversed input give the same hash (x2 in one run).
The probe was transient and removed after the run; it ships no code.
XZ footprints are pairwise disjoint (no shared interior area), confirmed
by the zero-`overlap` count inside the zero-finding report.

## Budgets

0 new tris, 0 new textures, 0 new materials. Dressing reuses torch (300
position verts) and banner (335) from the 7472-tri CT8 kit. Props stay
under the 5k LOD0 cap each. No VRAM change.

## Bytes x2

This file is the only owned write. Two consecutive reads hash identically
(values in the packet report, not embedded here so the bytes stay stable).
The two input JSON files were not modified.

## Golden

Unchanged. No simulation bytes touched (single new doc file). No pixels
claimed; the joint log==pixels capture rides CT60 on the reference GPU.
