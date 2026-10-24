# Falls onto the arena floor (fetched 2026-10-24, Freesound API token lane)

A body hitting dirt and sand. Used for the knock-down and death moments
(`knockdown` cue), beside the armour rattle as the fighter goes down.

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
again from the same preview gives the same file (md5 below). Total 3 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_body_fall_1_504626.ogg` | https://freesound.org/s/504626/ | CC0 | leonelmail | 1.63 | 13498 | 44ea701c1b5b6c49da0196dcb5a6745f | heavy body fall onto dirt and dust |
| `fs_body_fall_2_325270.ogg` | https://freesound.org/s/325270/ | CC0 | deleted_user_2104797 | 2.54 | 12351 | 50e1db67a03fea975404f936c14860c4 | body fall onto ground, body-first |
| `fs_body_fall_3_325269.ogg` | https://freesound.org/s/325269/ | CC0 | deleted_user_2104797 | 1.89 | 10084 | 2ce700035a894e258c0e453d0d8a518d | body fall onto ground, shorter knock |

Total in this folder: 35933 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
