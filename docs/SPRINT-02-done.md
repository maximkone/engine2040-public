### Done

- [ ] CB3 combat: delta-battle design patterns — hit-stop timing, cancel
  windows and reaction timing reimplemented (MIT, PORT cite + EVIDENCE
  row; Deltarune IP never touched, patterns only). Tier B. Size L.
  Owns `crates/combat/src/design.rs`. Accept: PQ2 tournament balance
  within tolerance; input latency ≤ 50ms. Kill: float math in the tick;
  divergence. needs: CB1, PQ2. (RES-STEAL wave 35 Dive 11, reviewed
  2026-09-26; remapped from colliding CB1.)
- [x] PT2 playtest: readable fight captures — duel `play.step` captures Landed 1d01a410. pt2_* tests, run 39053276085.
  draw CT7 fighter silhouettes (or stamp ABSTRACT-DOTS + legend), kp01
  jab flash visible; same inputs twice give same digests. Tier A.
  Size S. Owns capture-draw hunks beside `crates/endpoint/src/play.rs`.
  Accept: seed95 capture shows silhouettes/legend + jab flash; digests
  x2 identical. needs: CT7, CT15, VP1. (PLAYTEST56 B2 wave 56.)
- [ ] AU3 audio: hit-synced combat numbers — 10 combat scenarios carry
  hit-synced audio with per-tick 0-alloc proof surfaced as
  capture-adjacent numbers. Tier B. Size S. Owns hunks beside
  `crates/audio/src/coherence.rs`. Accept: 10 scenarios synced, 0
  audio-thread allocs. needs: PQ8, CB1. (PLAYTEST56 B-row 8 wave 56.)
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [x] SC2 scene: run pin-or-refuse — every `play.step`/`play.run` pins
  (build, scene, adapter) or refuses pre-start with a named error; an
  empty revision-0 arena never plays silently. Tier A. Size S. Owns pin
  hunks beside `crates/endpoint/src/scene_tools.rs`. Accept: unpinned
  run refused with named error; pinned run carries the pin in its
  manifest. needs: MCP8. (PLAYTEST56 B7 + EB-89 EVAL-PIN wave 56.) Landed 60d36c53. play.run refuses unpinned_run / empty_arena / stale_scene_rev before any job, status and manifest carry scene_digest; lib scene_tools/play/explain 94 passed, tests/it scene_drives_match rules_door session_chaos mcp_stdio 14 passed (run 39073069726). Limits: play.step plays no scene so it is unchanged; build and adapter are not pinned yet.
