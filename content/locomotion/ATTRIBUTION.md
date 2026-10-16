# CT3 locomotion set — attribution

All eight clips are synthetic stand-ins authored in code in Quaternius style
(`crates/anim/src/loco_bake.rs`, `HERO_SET`). No external bytes were fetched,
so there are no donor SHAs to record; the bake digests in `bake.json` are the
content hashes. Real source bytes enter later through the CT1 retarget cook
with an `EVIDENCE.md` row each (filed by the orchestrator).

| Clip | Source | URL | License | Frames | SHA-256 |
|---|---|---|---|---|---|
| idle_stand | Quaternius (synthetic) | https://quaternius.com/ | CC0-1.0 | 60 | n/a (synthetic) |
| walk_forward | Quaternius (synthetic) | https://quaternius.com/ | CC0-1.0 | 60 | n/a (synthetic) |
| run_forward | Quaternius (synthetic) | https://quaternius.com/ | CC0-1.0 | 60 | n/a (synthetic) |
| start_walk | 100STYLE (synthetic) | https://ianxmason.github.io/100style/ | CC-BY-4.0 | 30 | n/a (synthetic) |
| stop_walk | 100STYLE (synthetic) | https://ianxmason.github.io/100style/ | CC-BY-4.0 | 30 | n/a (synthetic) |
| pivot_turn | 100STYLE (synthetic) | https://ianxmason.github.io/100style/ | CC-BY-4.0 | 30 | n/a (synthetic) |
| strafe_left | Quaternius (synthetic) | https://quaternius.com/ | CC0-1.0 | 60 | n/a (synthetic) |
| turn_in_place | Quaternius (synthetic) | https://quaternius.com/ | CC0-1.0 | 30 | n/a (synthetic) |

100STYLE mirror: https://zenodo.org/records/8127870 (record 8127870).

Exclusions (never ship): the `Geno.fbx` mesh in `orangeduck/100style-retarget`
is non-commercial; LAFAN1 (CC BY-NC 4.0); Epic Game Animation Sample and other
Unreal-only content; any non-commercial or no-derivatives data. The bake
refuses them with `BakeError::Unlicensed` (tested: `geno_mesh` with
`CC-BY-NC-4.0` is refused, unknown sources with `BakeError::UnknownSource`).
