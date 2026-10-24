# Roman impact layer: sword on wood, sword on metal, shield (fetched 2026-10-24, Freesound API token lane)

The arena's scutum, lorica and helmet taking a blow. The wood row is the
gladius into a shield board; the metal rows are steel on steel and steel on a
plate; the shield rows are a flat board thump. These sit beside the Kenney
generic wood/metal/plate takes and the StarNinjas clash set; they do not
replace them.

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
again from the same preview gives the same file (md5 below). Total 7 file(s).

| file | source page | licence | author | sec | bytes | md5 | what |
|---|---|---|---|---|---|---|---|
| `fs_armor_hit_heavy_337856.ogg` | https://freesound.org/s/337856/ | CC0 | newagesoup | 2.99 | 19017 | 4578ee334757a5138796f512fcd5a018 | heavy metal hit, a blow landing on armour |
| `fs_armor_hit_light_337854.ogg` | https://freesound.org/s/337854/ | CC0 | newagesoup | 1.33 | 9998 | c0ac0d1000cab93b27e9e6467e2dd95e | light metal tick, armour or helmet grazed |
| `fs_shield_hit_636101.ogg` | https://freesound.org/s/636101/ | CC0 | fonografico | 0.82 | 7553 | ff636d3af2c1ab7ed068c73ac0b7e8a9 | shield taking a hit, dull board and metal thump |
| `fs_shield_knock_369710.ogg` | https://freesound.org/s/369710/ | CC0 | Mrguff | 0.93 | 8608 | c2cbb4a2aab6c9665c838b1b9bc13e69 | flat wood knock: shield rim, boss punch, gate plank |
| `fs_sword_impact_733887.ogg` | https://freesound.org/s/733887/ | CC0 | velcronator | 1.39 | 10962 | 62695ad5c13fa6dff98773f69304ddf7 | sword impact, steel biting metal |
| `fs_sword_shield_390765.ogg` | https://freesound.org/s/390765/ | CC0 | d00121058 | 0.38 | 5402 | b6dbdab233f6d717b174695f4e241556 | gladius edge into a wooden shield face, take 2 |
| `fs_sword_shield_390766.ogg` | https://freesound.org/s/390766/ | CC0 | d00121058 | 0.46 | 5592 | 9361172fe8ce376fb35cf5dd4005f97a | gladius edge into a wooden shield face, take 1 |

Total in this folder: 67132 bytes.

## Consumer

Not wired yet: `crates/audio/src/cue.rs` is owned by the sounds lane, so this
report names the cue each file should fill and that lane adds the path. No new
decoder: OGG Vorbis is the shipped path (`crates/audio` `decode_vorbis`).
