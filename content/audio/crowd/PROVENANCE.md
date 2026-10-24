# Crowd audio, fetched 2026-10-24 (GS23 rows, no token needed)

Four crowd reactions and two Colosseum field recordings, CC0, chosen for the arena
crowd bed and the "the crowd answers the fight" cues.

Licence: every Freesound page states "Creative Commons 0" (CC0 1.0,
https://creativecommons.org/publicdomain/zero/1.0/) and names its uploader below.

## What these bytes are, and are not

The sources are the **public `-hq.mp3` previews** the sound page serves, not the
original WAV upload (that comes from the Freesound API with `FREESOUND_TOKEN`, which
this seat does not have; the ids stay the same). Each preview was converted once with
the PC's ffmpeg 9.0.1 to **OGG Vorbis, mono, 22,050 Hz, quality 4** — the format the
crate decodes (`decode_vorbis`, lewton) — so the arena loads them through
`decode::load_cue_bank` with no MP3 decoder. The MP3 previews are not kept; the rows
below follow the converted files. Converting again from the same preview gives the
same file.

Rows marked *skipped* were fetched and then left out of git: a single 12.4 MB and a
7 MB file is a new tracked non-LFS blob over the 5 MB `workspace-check` bar. They stay
in `content/_incoming/` until the original WAVs arrive through the token.

| file | source page | licence | author | bytes | md5 | what |
|---|---|---|---|---|---|---|
| `fs_crowd_small_264377.ogg` | https://freesound.org/s/264377/ | CC0 | HowardV | 160,283 | 83bc0e17cb883435246c34106e7171da | small crowd cheering, bed |
| `fs_crowd_cheer_397434.ogg` | https://freesound.org/s/397434/ | CC0 | FoolBoyMedia | 149,444 | 25726c4d76fd23c8aa23849c0bcdb7f9 | crowd cheer (a hit lands) |
| `fs_crowd_yahoo_511788.ogg` | https://freesound.org/s/511788/ | CC0 | Kinoton | 34,572 | 62e665ed1929166e22cbe44364a08320 | "yahoos" (a knockdown) |
| `fs_crowd_exciting_365132.ogg` | https://freesound.org/s/365132/ | CC0 | SoundsExciting | 80,352 | 3f13b40beec479531cc67f06dd142b20 | excitement rise (round end) |
| *(skipped)* `fs_rome_labicana_341684` | https://freesound.org/s/341684/ | CC0 | alesonica | 12,411,168 | 7bea50927c12d7359e850175b6c06b41 | Rome, Colosseum, Labicana ambience: over the 5 MB commit bar |
| *(skipped)* `fs_rome_fori_341687` | https://freesound.org/s/341687/ | CC0 | alesonica | see `_incoming` | 45a0cbe00dd31d9cf1507672857a6ac9 | Rome, Colosseum, Fori ambience: same bar |

## Added 2026-10-24 through the API token: boo, gasp, roar, chant

Six reactions the fight still lacked (a bad decision booed, a gasp at a killing
blow, a stadium roar, a chant, an indoor roar). These came through the Freesound
API with the account token, so the source is `previews.preview-hq-ogg` (HQ OGG,
about 128 kbps) rather than the page's MP3 preview; the conversion is the same
one (ffmpeg 9.0.1, OGG Vorbis, mono, 22,050 Hz, quality 4). A source longer than
30 s was not taken. All six are 14 s or less, so each is a bed or a long
reaction, not a loop.

| file | source page | licence | author | bytes | md5 | what |
|---|---|---|---|---|---|---|
| `fs_crowd_boo_764142.ogg` | https://freesound.org/s/764142/ | CC0 | ShangusBurger | 78,077 | c65bf674e4d18f3419207be89e70b9cd | stadium crowd booing, angry (13 s) |
| `fs_crowd_boo_2_333390.ogg` | https://freesound.org/s/333390/ | CC0 | jayfrosting | 45,420 | 3379d5d01dc2caa9c29f643dcd2afd6a | crowd booing, disgusted (7 s) |
| `fs_crowd_gasp_763993.ogg` | https://freesound.org/s/763993/ | CC0 | ShangusBurger | 25,306 | 3999af0b21483aea3e9233108d3b8f62 | crowd gasp in surprise, voiced (4 s) |
| `fs_crowd_roar_649156.ogg` | https://freesound.org/s/649156/ | CC0 | tylermyers1992 | 146,864 | 5b1e2b8090c3c9f61527f21edba53d4a | full stadium roar, cheer bed (26 s) |
| `fs_crowd_chant_629884.ogg` | https://freesound.org/s/629884/ | CC0 | kyles | 147,629 | e9ad64ba837802c84fed8de7cea447e6 | arena crowd chanting and applauding (24 s) |
| `fs_crowd_roar_2_210959.ogg` | https://freesound.org/s/210959/ | CC0 | felix.blume | 88,313 | 25307041117a7f37e1b10eb73801262d | indoor crowd screaming and yelling (14 s) |

## Consumer

RL55 (`game: the crowd answers the fight — reactions are audio only`) picks the cue
table; the four kept files are one cue each (bed, hit, knockdown, round end). The mix
path already exists (`crates/game/src/audio_mix.rs`, crowd bed gain in
`TickOutcome::bed_gain`), so RL55 is a data row, not a decoder.
