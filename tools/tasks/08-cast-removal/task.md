# Task 8: remove a cast without deleting the work

`tick_count` converts a `u8` with `n as f32`. The engine's rule is no numeric
cast in sim code: use `f32::from(n)`, which cannot silently truncate.

The trap is deleting the arithmetic along with the cast. The referee needs the
function, the conversion and the arithmetic line, so `todo!()` is the committed
mutation and stays red.

## Start

```sh
node tools/tasks/08-cast-removal/referee.mjs --apply start --dest target/task-08
```

## Accept

```sh
node tools/tasks/08-cast-removal/referee.mjs --tree target/task-08
```

Exit 0 needs the `tick_count` function, no `as f32` (or any numeric `as`), the
`f32::from(n)` conversion and the `scaled * 0.5` arithmetic intact.
