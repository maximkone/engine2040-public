# content/vfx — Kenney license inventory (RL52)

Date: 2026-10-15. Scope: exactly the 3 adopted Particle Pack descriptors
in `kenney-particle-pack.json` (1247 bytes, sha256
`f0d4c9dfb79a1e50fb0282841772dff46bca514f7bc7f72143c0f6e01ec3088f`).
Method: descriptors only, zero sprite bytes in `crates/`.

Ship rights exist only for rows with all four provenance fields
(source URL plus SPDX plus author plus SHA256). All rows below are
complete, so all three are shippable as descriptors.

| adopted file | source URL | SPDX | author | SHA256 (cooked row) |
|---|---|---|---|---|
| smoke_01.png | https://www.kenney.nl/assets/particle-pack | CC0-1.0 | Kenney | 84ee631f9f89383602f77fa48209e0fe4dcdf3e8ebb68940afef2d0fe94eb90e |
| fire_01.png | https://www.kenney.nl/assets/particle-pack | CC0-1.0 | Kenney | 401f98343195e7a31f1513dd878aab9fa981464fe8557b8059de01980f942235 |
| spark_01.png | https://www.kenney.nl/assets/particle-pack | CC0-1.0 | Kenney | 01d51ea849343f7c6483abe945dc4520b26d2f71323c5b7aea7a99e9c3b87965 |

Notes:
- Closes the `content/sprites/LICENSES.md` UNKNOWN-NEEDS-ROW gap for
exactly these adopted files; sprites placeholders stay NOT-SHIP.
- `kenney/smoke-particles` skipped: CC0 unverified on the live page,
so no byte adopted; packet passes on the Particle Pack alone.
- Unity ParticlePack v3.1 (Asset Store) is METHOD ONLY under the Unity
Companion License: two-frame flipbook blend plus velocity stretch
reimplemented over the seeded pool, zero bytes vendored.
