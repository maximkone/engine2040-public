# Fighter voice: effort and pain, wordless (fetched 2026-10-24, Freesound API token lane)

Male effort and pain vocalisations with no words. They carry the attack
effort, the hit reaction and the death cry. Pitch and gain are set per
fighter in the mix, not in the file.

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
again from the same preview gives the same file (md5 below). Total 6 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_grunt_1_557131.ogg` | https://freesound.org/s/557131/ | CC0 | melisandepope | 1.55 | 11190 | b9ae36b2ec55ac79ba89ce962b9d80f3 | male effort grunt, short, wordless |
| `fs_grunt_2_667697.ogg` | https://freesound.org/s/667697/ | CC0 | EvilOldScratch | 1.05 | 9051 | d53efbd49e06a6e1cef2ebbec0f8bc85 | male effort grunt, heavier |
| `fs_pain_1_547205.ogg` | https://freesound.org/s/547205/ | CC0 | MrFossy | 0.45 | 6251 | 40c3623639b1d3f4266822862297b671 | male pain grunt, wordless |
| `fs_pain_2_547193.ogg` | https://freesound.org/s/547193/ | CC0 | MrFossy | 0.47 | 6418 | d34ab7dcd9363631d0d58b26d1cebf8b | male pain grunt, wordless, second timbre |
| `fs_pain_3_547192.ogg` | https://freesound.org/s/547192/ | CC0 | MrFossy | 0.31 | 5637 | be0319a2f4a61e004cf642a41b3ae9ec | male pain grunt, short and blunt |
| `fs_pain_death_547189.ogg` | https://freesound.org/s/547189/ | CC0 | MrFossy | 1.14 | 10173 | 0bfd4c758d7a944299b694ad9a2ce456 | male death cry, wordless |

Total in this folder: 48720 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
