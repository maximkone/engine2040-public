# Blade swings and thrusts (fetched 2026-10-24, Freesound API token lane)

Air noise for a sword cut and a thrust, so the swing cue stops resolving to
the synthesized whoosh in `cue.rs` (`ARENA_CUES` `swing`). Four whooshes of
different weight plus one thrust that carries its own impact.

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
again from the same preview gives the same file (md5 below). Total 5 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_swing_1_59992.ogg` | https://freesound.org/s/59992/ | CC0 | qubodup | 0.42 | 5982 | 424801550e305af4cf96ef13af66fc0f | short sword swing whoosh |
| `fs_swing_2_60004.ogg` | https://freesound.org/s/60004/ | CC0 | qubodup | 0.38 | 5616 | 6147e7a9e265bcea80e68f0a595e5ba7 | fast sword swing whoosh |
| `fs_swing_3_395266.ogg` | https://freesound.org/s/395266/ | CC0 | deleted_user_6479820 | 0.49 | 6034 | b87465c4cafe0a0404a910d71b5482b0 | single sword movement, blade cutting air |
| `fs_swing_4_395263.ogg` | https://freesound.org/s/395263/ | CC0 | deleted_user_6479820 | 0.52 | 5262 | ed7b97f33d5d6f8516611b58f462faba | single sword movement, lighter variant |
| `fs_thrust_395373.ogg` | https://freesound.org/s/395373/ | CC0 | ihitokage | 1.8 | 16609 | 1a98672e75cf2e80cce19b58ecf4e223 | sword stab: thrust whoosh plus impact (spear-thrust layer) |

Total in this folder: 39503 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
