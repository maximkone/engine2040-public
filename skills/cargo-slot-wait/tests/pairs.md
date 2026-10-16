# Pairs for cargo-slot-wait: with skill waits once instead of looping

Each case shows the WITHOUT answer (no skill) and the WITH answer
(skill applied). Winner named per case.

## Case 1: coder hits 3 of 3 slots busy before a scoped test

Situation: builder owns `crates/core/src/lib.rs` and sees
`cargo wait: 3 of 3 slots busy (perf ...; runner ...)` before
`cargo test -p engine-core --lib hash`.

- WITHOUT: retry the same test line every 10 s in a loop.
  Answer: three more `cargo wait` lines, then `cargo slots full: 3 of 3 busy for 5 min. Do non-cargo work now...`, step wasted, second identical retry ends the step.
- WITH: stop, `Start-Sleep -Seconds 60`, read `target/queue/batch.md` and owned `crates/core/src/lib.rs`, then run once `cargo test -p engine-core --lib hash`.
  Answer: one wait, scoped test green, report `RESULT: DONE - scoped test | proof: cargo test output for engine-core`.
- Winner: WITH (one wait plus signal, no slots-full error).

## Case 2: judge sees Blocking waiting for file lock on target/

Situation: judge runs `cargo test -p engine-spatial --lib place_on_top`
and gets `Blocking waiting for file lock on target/`.

- WITHOUT: run `cargo clean` to force the lock free, then retry.
  Answer: clean deletes shared `target/`, blocks other builds, still waits, plus the guard never allows clean; work lost, step wasted.
- WITH: wait, keep keeper timeout, read `target/queue/claims.txt -Tail 20` for the holder, then run once `cargo test -p engine-spatial --lib place_on_top`.
  Answer: lock frees, scoped test green, report cites the wait line and the test output.
- Winner: WITH (waits out the holder, no shared damage).

## Case 3: skill-hunter sees cargo wait and is tempted to build

Situation: skill-hunter/write seat sees `cargo wait` and wants signal
for `crates/render/src/sky.rs`.

- WITHOUT: run the cargo line as skill-hunter to get signal fast.
  Answer: blocked `this seat never calls cargo: only coders, judges and the lead build; file a packet instead` plus the wait, two failures for one call, step wasted.
- WITH: never build, file `target/queue/ready/cargo-sky-skyrs.md` naming `crates/render/src/sky.rs` and `cargo test -p engine-render --lib sky`, run `node tools/board-slice.mjs --ready` while waiting.
  Answer: no cargo call, packet filed, report `RESULT: BLOCKED - waiting for a cargo slot, packet filed | proof: ready packet plus queue-cap check`.
- Winner: WITH (avoids the seat block and routes to the owner).

Score: WITH wins 3 of 3. Install.
