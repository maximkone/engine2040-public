# CT63 combat sound list

Every fight cue in the staged fight (the `FightCue` enum in `crates/audio/src/crowd.rs`) is either voiced by a CC0 file
below or marked silent. All picks come from the Vol0 procedural pack already in `content/audio` (no download). They are
placeholders: recorded clips replace them row by row with their own URL + SPDX + author + SHA.
`node tools/hit-list.mjs` checks the table against the files and the enum; `--json` is what the SN10 log bridge reads.

<!-- gen:hit-list -->
| cue | when | file (under content/audio) | URL | SPDX | author | SHA-256 | bytes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Hit | damage < 120 | `impacts/flesh-small.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `5773762453a5fe7e04a97d591291b0e0114a0fef8c1e3934ad8cb69bdfa7255c` | 38444 |
| Hit | damage < 120 (variant) | `impacts/flesh-medium.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `997c67e7624d4f3d78fa5d02565cd144a8423b883c41f97e5f9f90db3957e0fc` | 62444 |
| Hit | damage >= 120 | `impacts/flesh-large.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `f17352c9bab53a97f085b55692c26fe0057befd7638872ff3107dc7d047d86f8` | 96044 |
| Blocked | chip damage on a held block | `impacts/wood-medium.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `951d7b2dea345a6d3c49d44eb7d7184f4c267e0019a625ccf1c90f9904d6c896` | 62444 |
| Parried | block inside the parry window | `impacts/metal-medium.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `6443f285711eb456f35043cd8f2be9f2e8258efbbf53892047ea8b630445f33a` | 62444 |
| Knockdown | victim at zero hp | `impacts/flesh-large.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `f17352c9bab53a97f085b55692c26fe0057befd7638872ff3107dc7d047d86f8` | 96044 |
| Knockdown | victim hits the ground | `impacts/stone-medium.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `3f86d6faf843508042869a4bdbc93758e271b61cec9029c945c6fa005107ff52` | 62444 |
| Dodged | i-frames negate the hit (shares the footstep voice) | `footsteps/gravel-01.wav` | https://github.com/maximkone/engine2040-public/blob/master/content/audio/engine2040-vol0-manifest.json | CC0-1.0 | engine2040 Vol0 pack (procedural, seed 7, order O-003) | `6ea7d7c748b5899d12392da9d48e85b3eb47d740eb50feddee983643de1bd5d3` | 51592 |

Silent cues:

- `StaminaOut`: silent, a drained bar starts nothing; the coherence bridge expects silence.
<!-- /gen:hit-list -->
