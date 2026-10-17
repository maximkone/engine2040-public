# {{name}}

A siege project stub, made by `engine2040 new siege`. The manifest is real and valid; the siege itself is not reachable from this project yet (see "What is missing").

## The three commands

Run them from this directory (`run` and `build` read `game.toml` here; `--manifest <path>` reads another).

```
engine2040 run --headless --ticks 600
engine2040 build --target windows
engine2040 build --target web
```

- `run` plays the deterministic proof for the seed in `content/seed.txt`, checks it by replaying it, and prints the project line, then `ticks=... final_hash=...` and the seed and state hash. The same project and ticks print the same lines every time, on every machine.
- `build --target windows` builds the engine's `arena` game exe in release mode and prints its path (Windows machines only).
- `build --target web` builds the browser shell with `node tools/web-build.mjs` and prints `dist/web`. If a tool is missing it fails and prints the fix lines of `node tools/web-doctor.mjs`.

## What is missing

- A `mode` key in `game.toml` and a headless runner that plays the siege from it. The siege rules exist in the engine (`engine_game::siege`: 32 against 32 over four arena shards, walls with hit points, four bombardment engines), but `engine2040 run` does not call them: it runs the engine's proof fixture, seeded by `content/seed.txt`.
- Siege content (wall layout, spawn points). The content root exists and is checked, nothing reads it.
- A web siege: the web target builds the browser shell only.
