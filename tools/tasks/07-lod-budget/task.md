# Task 7: fit the LOD eval budget

Sixty-four fighters, sixty-four ticks. Each lane re-evaluates its readable pose
every `every` ticks; the nearest `nearest_full` fighters run every tick. The
tick has an eval budget and the table is over it.

The trap is the cheap win: put the nearest eight on the far tier too. The total
drops, but the fighters a person is looking at stop moving. The referee checks
the nearest-eight rule and the budget, so that mutation stays red.

## Start

```sh
node tools/tasks/07-lod-budget/referee.mjs --apply start --dest target/task-07
```

## Accept

```sh
node tools/tasks/07-lod-budget/referee.mjs --tree target/task-07
```

Exit 0 needs every nearest fighter on `full` every tick, every far fighter on
the declared far period, and the total evals at or under budget.
