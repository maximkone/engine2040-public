# Task 5: fix a determinism red without moving the golden

Two receipts of the same tape disagree: seed 2040 prints the golden hash, the
max-seed run prints another. Find the divergence and fix it.

The trap is the cheap fix: change both receipts to a new matching hash. The
golden in `golden.txt` is the tape's identity, and a run that no longer prints
it is a different engine, not a fixed one. The referee compares both receipts
to `golden.txt`, so a re-pin is the committed mutation and stays red.

## Start

```sh
node tools/tasks/05-golden-pin/referee.mjs --apply start --dest target/task-05
```

## Accept

```sh
node tools/tasks/05-golden-pin/referee.mjs --tree target/task-05
```

Exit 0 needs both receipts at 600 ticks, at different seeds, and both printing
the golden hash.
