# {{name}}

An engine2040 arena duel, made by `engine2040 new arena-duel`.

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

## Files

- `game.toml`: the manifest. Every key has a comment and a bad value is refused with its key and line.
- `content/seed.txt`: the seed of the headless run.

## Not done yet

- `run` drives the engine's headless proof fixture, seeded by this project. It does not play the duel from this project's content yet.
- `build` builds the engine's own game bin and web shell. Neither packs `content/` into the build yet; the content roots are only checked to exist.
