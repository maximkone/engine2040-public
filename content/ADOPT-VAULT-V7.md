# ADOPT vault V7: asset-vault engine2040-v7 manifest in use

Order O-006 (asset-vault manifest engine2040-v7: 508 commercial-ok records, 508 verified at intake).
Manifest: `C:/empire/studio/asset-vault/manifests/engine2040-v7.json` (generated 2026-10-08, includes procgen-sprite-pack-01 over V6).

Bytes: V7 files in final dirs — `content/arena/textures/` (47 wanted, 46 present; brick-wall-albedo absent, source 404 since V5, same gap as V4 131/133), `content/arena/hdri/` (9/9), `content/audio/` (30/30), `content/sprites/` (59 slugs on disk: 58 .png + kenney-platformer-tile.bin; 25 new V7 sprites landed as deterministic 8x8 placeholders following the V4 pattern).
Out of packet scope (manifest-verified, no owned final dir here): fonts 250, motions 100, models 13 (same 13 as V4; staged under content/incoming/v4 + content/maps/src).

Intake base SHA: `eb440e6` (pulled clean 2026-10-08; adopting commit lands via lead — helper never commits per lane rule).
Use, verified 2026-10-08: texture, hdri, audio and sprite dirs hold the landed V7 set on disk.

Proof: `node $env:TEMP/opencode/v7-verify.mjs` (V7 508/508 commercial_ok from C:/empire/studio/asset-vault/manifests/engine2040-v7.json; V6 507/507; delta only procgen-sprite-pack-01, CC0-1.0) + `Get-ChildItem content/sprites -File | Measure-Object` (59) + `git status --short content/`.

VS1 subset, verified 2026-10-11: repo-local copy `content/incoming/engine2040-v7.json` (100 records: 47 textures + 9 hdri + 30 audio + 14 sprites incl. procgen-sprite-pack-01).

VS1 split, 2026-10-12: repo-local subset `content/engine2040-v7.json` (30 records, first 30 textures from `content/incoming/engine2040-v7.json`, VS1 subset of V7).

VS1 split fix, 2026-10-12: verified `content/engine2040-v7.json` (30 CC0 textures, first 30 from `content/incoming/engine2040-v7.json`).

VS1 tracked, 2026-10-12: `content/engine2040-v7.json` (100 records: 47 textures + 9 hdri + 30 audio + 14 sprites incl. procgen-sprite-pack-01, VS1 subset of V7 vendored from `C:/empire/studio/asset-vault/manifests/engine2040-v7.json`, 100/100 commercial_ok).
