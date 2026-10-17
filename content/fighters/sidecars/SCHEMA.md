# Clip sidecar schema (CT33)

One JSON file per canonical clip in this folder, named `<short>.json`
(`idle.json`, `attack_1h_1.json`, ...). The parser is
`engine_assets::parse_sidecar` (`crates/assets/src/retarget.rs`); this page is
the consumer contract and `crates/assets/tests/sidecar_conformance.rs` fails
when the files, the parser or this page drift apart. Unknown fields are
ignored by the parser, so older cooks read newer sidecars; every known field
is validated strictly. `null` reads as missing.

## Fields

| Field | Type | Meaning | Consumer |
|---|---|---|---|
| `clip` | string | Clip short name, equals the file stem | all |
| `loop` | bool | Clip tiles seamlessly (default false) | LM2 locomotion |
| `root_motion` | bool | Clip drives root motion (default false) | LM2 locomotion |
| `tags` | array of token | Locomotion and combat tags (`idle`, `run`, `attack`, `melee`, ...) | LM2, CB4 |
| `ground_y` | number | Foot contact threshold in meters | LM2 contacts |
| `feet` | object `{left, right}` | Foot joint names for the contact cook | LM2 contacts |
| `hit_windows` | array of `{socket, start_s, end_s}` | Combat windows: socket `SOCKET_<name>`, `0 <= start < end <= clip duration` | CB4 combat |
| `traversal` | array of `{edge, time_s}` | Traversal edge markers; edge is `vault`, `hurdle`, `mantle`, `climb`, `drop` or `finisher`; `time_s >= 0` | CT24 traversal link |

Tokens are ASCII letters, digits and `_`, at most 64 chars. Foot names are
tokens that do not start with a digit. Tags are unique within one file.

## Limits (the parser constants)

| Constant | Value |
|---|---|
| `MAX_SIDECAR_BYTES` | 65536 |
| `MAX_TAGS` | 32 |
| `MAX_TAG_CHARS` | 64 |
| `MAX_HIT_WINDOWS` | 32 |
| `MAX_TRAVERSAL` | 32 |
| `MAX_JSON_DEPTH` | 32 |

## Rules the shipped files obey (checked)

- Every clip has `feet` and `ground_y` (LM2 reads contacts from every clip).
- Every clip tagged `attack` has at least one hit window (CB4).
- Hit windows end inside the clip duration from `manifest.json`
  (`node tools/fighter-report.mjs`).
- Traversal edges stay inside the vocabulary above; the 14 shipped clips carry
  none yet, CT24 adds them with its clips.
