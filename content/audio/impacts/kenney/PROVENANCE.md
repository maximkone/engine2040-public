# Kenney Impact Sounds, the arena's thud and clash layer (fetched 2026-10-24)

The wood, metal and plate impacts the arena lacks: a scutum taking a blow is a wood
thud, a gladius parry is a metal tick, a shield-boss shove is a plate knock.

Page: https://kenney.nl/assets/impact-sounds — "Files 130x", "License Creative
Commons CC0", download behind the page's own dialog. Licence line the page states:
Creative Commons CC0 (https://creativecommons.org/publicdomain/zero/1.0/). Author
Kenney (www.kenney.nl). `License.txt` in this folder is the pack's own notice,
extracted with the sounds.

## What was taken

Source zip: `kenney_impact-sounds.zip`, 800,850 bytes, md5
`064214e0544b18af545f9c2964d04f0c`, fetched with
`node tools/fetch-content.mjs content/fetch-potato.list`.

Only three surfaces were kept, each at light/medium/heavy, five takes a size, 45 OGG
files (354,593 bytes with `License.txt`):

| files | what it is | use |
|---|---|---|
| `impactWood_{light,medium,heavy}_000..004.ogg` | wood knock and splinter | scutum hit, wooden rack, gate |
| `impactMetal_{light,medium,heavy}_000..004.ogg` | metal tick to clang | gladius parry, steel on steel |
| `impactPlate_{light,medium,heavy}_000..004.ogg` | flat plate knock | shield boss, armour |

Dropped, deliberately: the `footstep_*` surfaces (carpet, concrete, grass, snow, wood —
sand footsteps are a later list row), `impactGlass_*`, `impactBell_*`, `impactMining`,
`impactPunch_*`, `impactSoft_*`, `impactTin_*`, `impactGeneric_light` and the zip
itself. Nothing was re-encoded: these are the pack's OGG bytes.

## Consumer

The impact table (`crates/game/src/lib.rs` bank cue names, `engine_audio::cue::impact_cue`)
names one cue per surface and size; these files fill the wood/metal/plate rows the
bank currently resolves to a placeholder. No new decoder: OGG Vorbis is the shipped
path (`crates/audio`).
