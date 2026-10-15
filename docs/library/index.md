# Library — reusable skills, patterns, and recipes

Every reusable piece gets a test plus a reuse counter. Packets
reuse-before-invent; a piece used twice becomes an asset, never
reused in 10 waves gets archived. Plans index their reusable pieces
here (index pointers, not copies).

## How to read this index

| Column | Meaning |
|---|---|
| **Entry** | Name of the reusable piece |
| **Pattern** | What it solves, in one line |
| **Owner** | Crate that proves it works |
| **Test** | Command that runs the test |
| **Reuse** | How many wave packets have reused it (starts 0) |
| **Waves** | Which waves will or have reused it |

## Rules (CODELAW §9, EMPIRE level 4)

1. **Index pointers, not copies.** A plan that needs a pattern
   links to this entry; it does not re-describe the pattern.
2. **Test proves it.** Every entry has a runnable test that fails
   on a plausible defect.
3. **Counter tracks value.** `Reuse` increments when a packet cites
   this entry. Reach 2 → asset; stall at 0 for 10 waves → archive.
4. **New entries go here first.** Before writing anything reusable,
   check this index. If it does not exist, add it.

## Entries

| # | Entry | Pattern | Owner | Test | Reuse | Waves |
|---|---|---|---|---|---|---|
| 1 | [Counting-allocator floors](./counting-allocator-floors.md) | Prove zero steady-state allocations per tick with a thread-local counting allocator | `engine-perf` | `cargo test --locked -p engine-perf alloc::tests` | 5 | Waves 28–31 (SL6, SL18–SL20, SL21); future: every quality row |
| 2 | [Buffer reuse with counting-allocator floors](./buffer-reuse.md) | Reserve buffers once, reuse across ticks; floor drops proven by counting allocator | `engine-anim`, `engine-destruction`, `engine-combat` | `cargo test --locked -p engine-anim steady_state_reuses_buffers` | 3 | Waves 28–29 (SL18 locomotion, SL19 support-refresh, SL20 combat event-buffer) |
| 3 | [Log-before-pop journaling](./log-before-pop-journaling.md) | Append-then-checkpoint WAL; readers pinned to end mark; torn tail recovers to last good | `engine-editor` | `cargo test --locked -p engine-editor wal_end_group_failed_append_leaves_journal_untouched` | 1 | Wave 31 (ED8 crash-safe journal); future: every editor write path |
| 4 | [Contract-test patterns](./contract-test-patterns.md) | Typed cross-crate invariants proved by contract tests with alloc probes and roundtrip checks | `engine-ecs`, `engine-combat` | `cargo test --locked -p engine-ecs wire_roundtrip` | 2 | Waves 21–29 (SL10 bridge roundtrip, SL20 alloc_probe); future: every cross-crate edge |
| 5 | [Cross-crate harness wiring](./cross-crate-harness-wiring.md) | `node tools/proof.mjs` wires verifier gates by name; removed gates fail the build | `engine-proof` | `node tools/proof.mjs --skip-build` | 5 | Waves 28–32 (all proof rows); future: every new verifier gate |

## Adding a new entry

1. Write `docs/library/<kebab-name>.md` using the template below.
2. Add a row to the table above.
3. Add an index pointer in the plan that owns the pattern
   (`docs/plan/quality.md`, `docs/plan/README.md`, etc.).
4. Run `cargo doc --no-deps` and confirm no new warnings.

## Template

```markdown
# <Entry name> — <one-line summary>

Public summary. Owner crate. Units. Determinism scope.

## Pattern

What it solves and how, in plain language plus the precise detail
for coders. Cite the source file and line.

## Why it is reusable

What makes this a pattern rather than a one-off. What other crates
or systems can take from it.

## Test

The command that proves the pattern works and fails on a plausible
defect. Copy it verbatim.

```sh
<command>
```

## Reuse counter

This entry starts at **0** reuses. It becomes an asset at 2
reuses and archives at 0 reuses across 10 waves. Increment the
counter when a packet cites this entry.

## Waves that reuse or will reuse

| Wave | Packet | How it reuses |
|---|---|---|
| N | `<id>` | `<description>` |

## Reference

Full signatures live in rustdoc (`cargo doc -p <crate> --no-deps`);
the file that owns this page: `<path>`.
```

## See also

- [SPRINT.md](../SPRINT.md) — the lane board; LIB1 owns this index
- [CODELAW.md](../CODELAW.md) — the slim law; §9 covers free copying
- [EMPIRE.md](../EMPIRE.md) — level-4 changes; this index is a level-4 asset
- [plan/quality.md](./../plan/quality.md) — indexes the counting-allocator and buffer-reuse rows
- [book/verifiers.md](../book/verifiers.md) — indexes the proof harness wiring
