# Gates and portcullis (fetched 2026-10-24, Freesound API token lane)

The gate cue: a wooden gate swinging shut, its chain and wire creaking, and
a heavy gate opening and closing for the portcullis weight at the arena mouth.

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
| `fs_gate_chain_861827.ogg` | https://freesound.org/s/861827/ | CC0 | qubodup | 2.67 | 22244 | 9fcede6555b7cc9fd2b65e9531025003 | creaking metal wire and chain of a wooden gate |
| `fs_gate_wood_788015.ogg` | https://freesound.org/s/788015/ | CC0 | RealSquink | 3 | 19160 | fb8c01b9684ac924d9d935332d7bfd91 | wooden gate creaking and swinging shut |
| `fs_portcullis_336888.ogg` | https://freesound.org/s/336888/ | CC0 | Omnisis | 9.81 | 64676 | 0a04ddcd5e16bb48ccd1e012d3ebf987 | heavy gate opening and closing, portcullis weight, 10 s |

Total in this folder: 106080 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
