# Arena ambience beds: torch, fire, city, wind (fetched 2026-10-24, Freesound API token lane)

Long beds for the arena itself, all under 30 s and mono 22.05 kHz like the
crowd files: a torch brazier, a fire pit, the distant city beyond the walls,
and wind over the open ground. These are previews of field recordings, not
synthesized loops; the mix loops them with a crossfade.

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
| `fs_city_407154.ogg` | https://freesound.org/s/407154/ | CC0 | kyles | 22.36 | 129596 | 5514f708c86279f4e1b733775bcd1125 | distant city: busy market street and light traffic, 22 s bed |
| `fs_fire_676385.ogg` | https://freesound.org/s/676385/ | CC0 | StevenMyat_ | 11.6 | 79737 | 7b9bc95290af0889fe05dabc18735b12 | fire crackling, brazier-scale, 12 s bed |
| `fs_torch_848457.ogg` | https://freesound.org/s/848457/ | CC0 | lfyaudio | 19.79 | 173660 | 5fdae66306b9852ce55c773d4536ef17 | torch flame close, crackle and flare, 20 s bed |
| `fs_wind_327771.ogg` | https://freesound.org/s/327771/ | CC0 | Jeffreys2 | 24.91 | 132986 | dae7bc334c89e454da794c91fbd0c7be | wind howling over open ground, 25 s bed |

Total in this folder: 515979 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).

## The two Rome beds (sounds lane, 2026-10-24)

The Colosseum field recordings under the crowd, from the same CC0 page family
(https://freesound.org/s/341684/ and https://freesound.org/s/341687/, alesonica).
The fetched bytes were the public `-hq.mp3` previews (12,411,168 B md5
`7bea50927c12d7359e850175b6c06b41` and 8,769,120 B md5
`45a0cbe00dd31d9cf1507672857a6ac9`, recorded in `content/fetch-roman.list`),
converted once with ffmpeg 9.0.1 to **OGG Vorbis, mono, 22,050 Hz, quality 3**
and trimmed to 120 s: the raw recordings are 186 s and the long one ran past
`decode::MAX_PCM_FRAMES` (4,000,000 frames, 181 s at 22.05 kHz), so the loader
refused it. 120 s is 2,646,000 frames, inside the bound, and longer than a round.
The MP3 originals stay untracked in `content/_incoming/url/`.

| file | source page | licence | author | source bytes | land bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_rome_labicana_341684.ogg` | https://freesound.org/s/341684/ | CC0 | alesonica | 12,411,168 | 564,194 | af506a4a7aa26ceaee637423eec0436b | Colosseum stands, distant crowd murmur, 120 s bed |
| `fs_rome_fori_341687.ogg` | https://freesound.org/s/341687/ | CC0 | alesonica | 8,769,120 | 562,307 | 0dac9df186a8ed53fbd76788b68bf55c | Roman forum, open-air room tone, 120 s bed |

Wired: `crates/audio/src/cue.rs` `names::AMBIENCE_ROME` (`ambience_rome`),
looped, Group::Ambient, priority 150, gain 0.45; two takes, decoded by
`decode_vorbis` through `load_cue_bank`. Nothing starts the bed yet.
