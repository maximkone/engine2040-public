# Log-before-pop journaling — append-then-checkpoint WAL with torn-tail recovery

Owner crate: `engine-editor` (`crates/editor`). Consumer: the editor
shell save flow. Units: file offsets and lengths are bytes; stamps
are fixed ticks plus monotonic editor revisions. Determinism:
little-endian frames in append order, hyphenated ids, CRC32 per
frame; replay is a pure fold, so the same files always reopen to
the same document and journal. Errors: [`WalError`]; torn tails
truncate to the last good frame, damaged snapshots refuse loudly.

## Pattern

The WAL (write-ahead log) follows a log-before-pop contract:
every append writes the log entry *before* mutating in-memory state,
and every group end writes an END marker *after* the operations.
If the END marker write fails (e.g., the log file becomes read-only),
the journal must come back bit-identical — no partial state mutation.

The key invariant (the "log-before-pop contract") is tested by
`wal_end_group_failed_append_leaves_journal_untouched`: when the
log file handle is swapped for a read-only one, the END marker write
fails, and the journal state is unchanged from before the group began.

This pattern is ED8 (crash-safe journal) on the board. The WAL
provides: one writer, append-then-checkpoint, readers pinned to an
end mark, a torn tail recovered on open, each entry stamped with
the LW6 tick and revision.

## Why it is reusable

Any system that needs crash-safe durable state can use this pattern.
The log-before-pop contract ensures that a failed write never leaves
partial state. The torn-tail recovery ensures that a crashed process
reopens to the last good state. The CRC32 per frame ensures integrity.

**Transferable to:** any crate that writes durable state and needs
crash safety. The recipe: (1) append to the log before mutating state,
(2) write a checkpoint after the log is durable, (3) pin readers to
an end mark, (4) recover torn tails on open by replaying to the last
good frame.

## Test

The `engine-editor` crate owns the contract test:

```sh
cargo test --locked -p engine-editor wal_end_group_failed_append_leaves_journal_untouched
```

This test swaps the log file for a read-only handle, attempts the
END marker write (which must fail), and asserts that the journal
state is bit-identical to before the group began. It proves the
log-before-pop contract: a failed append leaves nothing mutated.

Additional tests cover frame size bounds and checkpoint sealing:

```sh
cargo test --locked -p engine-editor wal_
```

## Reuse counter

This entry starts at **0** reuses. It becomes an asset at **2**
reuses and archives at **0** reuses across 10 waves.

| Wave | Packet | How it reuses |
|---|---|---|
| 31 | `ED8` | `crates/editor/src/wal.rs` — crash-safe journal with append-then-checkpoint, torn-tail recovery, CRC32 per frame |
| future | any durable state system | Log-before-pop contract: append before mutate, checkpoint after, recover torn tails on open |

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-editor --no-deps`);
the file that owns this page: `crates/editor/src/wal.rs`.
Key items: `Wal`, `Wal::open`, `Wal::checkpoint`, `wal_begin_group`,
`wal_end_group`, `wal_apply`, `wal_undo`, `wal_redo`.
The contract test: `wal_end_group_failed_append_leaves_journal_untouched`
(line 1720).
