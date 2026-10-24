# Footsteps on sand (fetched 2026-10-24, Freesound API token lane)

Single steps on fine sand (the source recorded fine snow, which is the same
granular surface at foley distance). The mixer alternates and slightly
re-pitches them for walk and run; no loop file is committed.

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
| `fs_sand_step_1_778568.ogg` | https://freesound.org/s/778568/ | CC0 | BlondPanda | 0.66 | 7281 | fe9734c3e10bdb10ea8cbbdfc2e6f1e3 | single step on fine sand, firm |
| `fs_sand_step_2_778564.ogg` | https://freesound.org/s/778564/ | CC0 | BlondPanda | 0.78 | 7749 | 3e2808fa93bf9dcae44ffbf040acd935 | single step on fine sand |
| `fs_sand_step_3_778556.ogg` | https://freesound.org/s/778556/ | CC0 | BlondPanda | 0.82 | 7925 | e29edc37151a8497a2c69a3d955e84c7 | single step on fine sand, softer |
| `fs_sand_step_4_778565.ogg` | https://freesound.org/s/778565/ | CC0 | BlondPanda | 0.78 | 8001 | c150fbdc87ed9d3fdf5dd219be122de3 | single step on fine sand, scuffier |

Total in this folder: 30956 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
