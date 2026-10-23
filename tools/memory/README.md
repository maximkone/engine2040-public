# Project memory

A JSONL store of the lessons a build leaves behind: one decision, one failed
fix or one tuned value per row, each carrying the tape hash (or golden) it was
taken under, the source path it is about and a UTC stamp.

```sh
node tools/memory/memory.mjs record --kind failed-fix \
    --hash d57b5c8ae3096614 --path crates/anim/src/motion_bake.rs \
    --text "the local foot offset is not slip; the planted-foot meter is the world one" \
    --evidence be75d2c32
node tools/memory/memory.mjs query --path crates/anim
node tools/memory/memory.mjs query --kind tuned-value --hash d57b5c8ae3096614
node tools/memory/memory.mjs --selftest
```

- `record` refuses a row with no hash, a hash that is not 16 hex digits, an
  empty text or an absolute path. A lesson that cannot be re-checked under the
  hash it was learned on is not memory.
- `query` filters by `--kind`, `--path` (substring) and `--hash`, and prints one
  JSON row per line sorted by UTC stamp then id. The row count goes to stderr,
  so stdout is only rows.
- `--selftest` replays a fixed four-row fixture twice and compares both answers
  byte for byte, then proves a no-hash row is refused.

The store is `tools/memory/memory.jsonl`, seeded with this run's lessons: the
arena.rs pin, the shared-index commit that carried a foreign console removal,
the world-frame slip bug and the AM22 draw-chord tuning. `--store <file>` points
at another store, which the selftest and tests use.
