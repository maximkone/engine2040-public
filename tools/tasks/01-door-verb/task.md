# Task 1: add a door verb

The door serves `scene.read` and `tx.preview`. A preview needs a way to put the
camera on the item it is about to change, so add one verb, `scene.frame`.

A verb is wired in two places, and a half-wired verb is the classic red:

- `verbs.json` lists it: `name`, `handler` (equal to the name) and a `doc` line.
- `handlers.mjs` exports a function under the same key. It receives
  `{ name }` and answers `{ ok: true, framed: name }`.

## Start

```sh
node tools/tasks/01-door-verb/referee.mjs --apply start --dest target/task-01
```

## Accept

```sh
node tools/tasks/01-door-verb/referee.mjs --tree target/task-01
```

The referee prints PASS and exits 0 only when the manifest, the handler and the
doc agree and the handler actually frames the item it is given. It does not read
your commit message; it calls the handler.

The committed mutation is the half-wired verb: the manifest lists `scene.frame`,
`handlers.mjs` has no such key. That tree must stay red.
