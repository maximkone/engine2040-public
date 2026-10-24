# StarNinjas sword clashes, the steel-on-steel layer (fetched 2026-10-24)

Page: https://opengameart.org/content/20-sword-sound-effects-attacks-and-clashes — the
page states CC0 (https://creativecommons.org/publicdomain/zero/1.0/) and names
StarNinjas as the uploader. The pack's notice asks for a link to the author's profile;
that is a request, not a CC-BY condition, so the terms stay CC0.

Source zip: `sword_clash_-_starninjas_0.zip`, 144,575 bytes, md5
`3b82c7072f96dcec53d24072bf74a7b9`, fetched with
`node tools/fetch-content.mjs content/fetch-roman.list` (that list already carried
this row and its md5).

## What was taken

All ten clash takes, OGG Vorbis, 160,175 bytes total:

`sword_clash.1.ogg` .. `sword_clash.10.ogg`

Dropped: the sibling `sword_attacks` zip (ten attack whooshes) — the arena already has
attack tells from the Mixamo clip cue table, and the whoosh is a later row. The zip
file itself is not committed.

## Consumer

Wired 2026-10-24: `crates/audio/src/cue.rs` `ARENA_CUES` plays takes 1-4 as the
`parry` cue and 5-8 as the `clash` cue (the SN12 steel-bind row), so both stop
resolving to the synthesized voice; `decode::load_cue_bank` decodes these OGG files
with `decode_vorbis`. The parry and clash picks are proved by the test
`the_new_cc0_takes_decode_and_their_cues_pick_them` (engine-audio, `device` feature:
every file decoded, rate and length printed).
