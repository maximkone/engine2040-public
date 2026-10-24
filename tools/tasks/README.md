# Build-task suite

Ten real engine jobs, each judged by a programmatic referee. **No model and no
LLM judge:** a referee reads the tree, runs its checks and exits 0 (PASS) or 1
(FAIL). The same referee is red on that task's committed known-bad mutation.

```sh
node tools/tasks/run.mjs --selftest    # all referees on start/fixed/mutation + the pass@k table + both solve arms
node tools/tasks/run.mjs --list        # the ten tasks
node tools/tasks/run.mjs --log tools/tasks/logs/canned.jsonl --k 3
node tools/tasks/run.mjs --solve --arm scripted   # solve the suite, print its real pass@1
```

## The ten tasks

| folder | job | what the referee refuses |
| --- | --- | --- |
| `01-door-verb` | add a door verb | a manifest verb with no handler, or a handler that does not frame the item |
| `02-red-capture` | fix a red capture | a clean receipt over a black frame |
| `03-web-build-size` | make the web build smaller | a small total with the wasm artifact dropped |
| `04-board-row` | add a board row | a row that needs an id nobody has, or accepts with a missing tool |
| `05-golden-pin` | fix a determinism red | two receipts agreeing on a hash that is not the golden |
| `06-slip-meter` | measure a planted foot | the local foot offset instead of the world one, or a made-up number |
| `07-lod-budget` | fit the LOD eval budget | the nearest fighters frozen to fit the budget |
| `08-cast-removal` | remove a numeric cast | the cast gone because the work is gone |
| `09-doc-index` | add a doc | a page with no index line |
| `10-wasm-gate` | gate a desktop clock | the wasm gate on the wrong side |

## Shape of a task

Each folder holds `task.md` (the job for a solver) and `referee.mjs` (the
referee). The referee carries three trees and one set of checks:

```sh
node tools/tasks/<task>/referee.mjs --apply start|fixed|mutation --dest <dir>
node tools/tasks/<task>/referee.mjs --tree <dir>
```

- `start` is the tree handed to a solver (it must be red).
- `fixed` is the reference solution (green).
- `mutation` is a committed known-bad attempt: plausible, and red. It is the
  guard that the referee is not just checking a filename.

`--apply` is the patch hook: no solver tree is stored in the repo, the tree is
written on demand. `lib.mjs` holds the CLI, the tree reader and the checker
wiring.

## Scoring a run log

A run log is JSONL, one recorded attempt per line:

```json
{"model":"deepseek-v4.1-flash","task":"06-slip-meter","attempt":1,"passed":true}
```

`pass@1` is the share of tasks solved on attempt 1. `pass@k` is the share solved
on any of the first k attempts. The scorer groups by model and prints the table;
`logs/canned.jsonl` is a recorded example (3 models x 10 tasks).

The table it prints:

```
model                 tasks  attempts  pass@1  pass@3
deepseek-v4.1-flash      10        13   0.800   1.000
baseline-agent           10        15   0.500   0.700
no-memory-agent          10        17   0.200   0.200
```

## Solving the suite for real (`--solve`)

A run log is only as good as the referee behind it. `--solve --arm <name>` writes
each task's `start` tree, lays one arm's patch over it the way a solver's edit
lands, and asks the referee to judge the tree that results. It then prints the
pass@1 it actually measured.

| arm | what it lays | expected pass@1 |
| --- | --- | --- |
| `scripted` | each task's own `fixed` tree | `1.000` |
| `mutation` | each task's committed known-bad tree | `0.000` |

```
task               patched  verdict  the referee
01-door-verb       2        solved   referee 01-door-verb: PASS
02-red-capture     2        solved   referee 02-red-capture: PASS
...
model     tasks  attempts  pass@1
scripted  10     10        1.000
solve scripted: pass@1 1.000 (10/10 tasks solved)
```

`patched` counts the files the arm changed over the start tree, so an arm that
laid nothing is visible as one row of zeros. The arms are **oracles, not
models**: they say what the referee can tell apart, which is the floor a real
model is scored against. `--selftest` runs both arms and fails the suite when an
arm misses its expected pass@1, so a referee that stops judging is caught here
rather than in someone's benchmark number.
