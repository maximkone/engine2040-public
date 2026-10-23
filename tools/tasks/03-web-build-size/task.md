# Task 3: make the web build smaller

The web build is over its budget: `engine.js` plus `engine_bg.wasm` exceed
`budget_bytes`. Shrink it without dropping the wasm artifact.

The catch: a build that reports a small number because the wasm half is missing
is not smaller, it is broken. The referee checks the wasm magic (`0061736d`),
so a placeholder cannot pass.

## Start

```sh
node tools/tasks/03-web-build-size/referee.mjs --apply start --dest target/task-03
```

## Accept

```sh
node tools/tasks/03-web-build-size/referee.mjs --tree target/task-03
```

Exit 0 needs both entries present and non-empty, the total at or under budget,
the wasm magic intact and the `wasm32` flag still set.
