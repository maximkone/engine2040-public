# Armour movement: chain and plate rattle (fetched 2026-10-24, Freesound API token lane)

Mail and plate moving with the body: one bed for continuous movement and
short rattles for a step, a turn or a shove. The two `chain_step` takes are
the same footstep recordings the source made over gravel, so the chain is
the loud layer and the ground is a soft bed under it.

Licence: every source page states **Creative Commons 0** (CC0 1.0,
https://creativecommons.org/publicdomain/zero/1.0/): no attribution is required
and the uploader is named below anyway.

## What these bytes are

The sources are the **HQ preview OGG** the API serves for each sound
(`previews.preview-hq-ogg`, about 128 kbps), fetched with the account token.
Original uploads need OAuth2 download, which this lane does not have; the ids
stay the same. Each file was converted once with the PC's ffmpeg 9.0.1 to
**OGG Vorbis, mono, 22,050 Hz, quality 4** — the format `decode_vorbis` reads —
and a one-shot longer than 3 s was cut at 3 s with a 40 ms fade. Converting
again from the same preview gives the same file (md5 below). Total 4 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_chain_move_797655.ogg` | https://freesound.org/s/797655/ | CC0 | SecureSubset | 7.55 | 40228 | e2541fde7c4399c452ef319e865c9290 | chain mail moving, bed for armour movement |
| `fs_chain_rattle_369321.ogg` | https://freesound.org/s/369321/ | CC0 | Sheyvan | 3.4 | 24855 | afe68baa40804bb1203b4b877897056e | chain rattle picked up and shaken |
| `fs_chain_step_1_384887.ogg` | https://freesound.org/s/384887/ | CC0 | Ali_6868 | 0.5 | 6402 | 7c120a2d539e54fda8dcc034bd65b5a4 | chainmail footstep, gravel underneath |
| `fs_chain_step_2_384884.ogg` | https://freesound.org/s/384884/ | CC0 | Ali_6868 | 0.5 | 7112 | 89e346965df2acc88a93bcb8fed6d619 | chainmail footstep, other foot, gravel underneath |

Total in this folder: 78597 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
