# Horn fanfare (fetched 2026-10-24, Freesound API token lane)

The arena's call to order: a short war horn and a horn-and-timpani fanfare
for the round start and the verdict.

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
again from the same preview gives the same file (md5 below). Total 2 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_fanfare_803823.ogg` | https://freesound.org/s/803823/ | CC0 | marlocarlo | 5.25 | 28396 | 99908a15d9f01517a588d99886ab03ed | horn and timpani fanfare, 5 s |
| `fs_war_horn_539956.ogg` | https://freesound.org/s/539956/ | CC0 | adharca | 3 | 19387 | 45449acef7c6d8f1156fe08a269ece10 | war horn blast, bone/brass, short |

Total in this folder: 47783 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
