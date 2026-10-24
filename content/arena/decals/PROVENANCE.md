# Sand decals: one CC0 blood splatter (fetched 2026-10-24)

Page: https://opengameart.org/content/blood-splatter — "Author: ExileGL", "License(s):
CC0", "File(s): blood.png 2.6 Mb". Licence line the page states: CC0 1.0
(https://creativecommons.org/publicdomain/zero/1.0/), author ExileGL.

The served file is `blood_0.png` (2,625,982 bytes, the 2.6 MB the page names); the
`blood.png` name on that host is a different, 43 KB older upload, so the fetch row
points at `blood_0.png` and the landed copy keeps the source's content under a name
that says what it is: `blood_splatter.png`.

| file | source URL | licence | author | bytes | md5 | pixels |
|---|---|---|---|---|---|---|
| `blood_splatter.png` | https://opengameart.org/sites/default/files/blood_0.png | CC0 1.0 | ExileGL | 2,625,982 | 90e858f396e5b327ce7623ff0c888dc6 | 2048x2048 RGBA (single splatter, alpha) |

Fetched with `node tools/fetch-content.mjs content/fetch-potato.list`, md5 verified by
the tool and re-checked on disk after the copy. png is a Git LFS path, so the tracked
blob is a pointer and the working tree keeps the bytes.

## Not cooked yet

The decal path is `crates/render/src/decals.rs` (LF7 landed deferred blood decals;
RL49 "the sand remembers the fight" and QL11 "clustered projected decals" are the open
rows). This file is the source art only: the cook step (1024 px BC7/ASTC + a
single-channel alpha mask) and the cluster cap belong to those rows. Until a caller
loads it, it is OPEN.
