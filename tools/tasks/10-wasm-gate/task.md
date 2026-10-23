# Task 10: gate a desktop clock for the wasm build

`now_millis` reads `std::time::Instant`, which does not exist for `wasm32`.
Gate the desktop body with `#[cfg(not(target_arch = "wasm32"))]` and give the
browser a twin behind `#[cfg(target_arch = "wasm32")]` that takes its clock from
the shim, never from `std::time`.

The trap is the attribute in the wrong place, or with the wrong polarity. Both
are the committed mutation and stay red, as does a red `wasm-check` leg.

## Start

```sh
node tools/tasks/10-wasm-gate/referee.mjs --apply start --dest target/task-10
```

## Accept

```sh
node tools/tasks/10-wasm-gate/referee.mjs --tree target/task-10
```

Exit 0 needs the `std::time` body under a `not(target_arch = "wasm32")` gate, a
wasm twin with no `std::time`, and the `wasm-check` leg green.
