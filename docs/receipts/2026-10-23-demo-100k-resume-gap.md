# Demo-length 100k proof and the resume gap (findings, 2026-10-23)

## The length gate

`tests/proof_cli.rs::demo_100k` (ignored by default) plays the shipped demo
match twice from fresh state: seed 2040, 100,000 fixed ticks, 2 fighters,
rules v2 on the kit walls, one gladius per slot, spear on. Slot 0 is a fixed
script (the demo's human seat); slot 1 is an arena brain.

Command:
`node tools/gh-check.mjs --budget 1700 cargo test -p engine2040 --release --test proof_cli demo_100k -- --ignored --exact --nocapture`

Receipt (run 40017917792, ubuntu-latest, x86-64):
`demo_100k: ticks=100000 hash_a=c600e309490b8095 hash_b=c600e309490b8095 wall_s=0.802`

The 0.802 s covers both 100,000-tick runs. No nightly leg is needed.

## What the save carries today

- `App::save_progress` (`crates/game/src/lib.rs:1912-1932`) seals one payload:
  `self.progression().encode()` through
  `engine_save::save_generation(dir, slot, config.seed, cursor, &payload)`.
- `GameProgress` (`crates/game/src/progress.rs:44-95`) is 25 fixed bytes:
  `matches_completed`, `wins`, `losses`, `match_seed`, `match_ticks`.
- `engine_save` adds a 29-byte envelope: magic, version, seed, tick, payload
  length, payload, CRC32 (`crates/save/src/lib.rs:89-109`).
- `App::load_progress` (`crates/game/src/lib.rs:1938-1949`) returns the same
  counters plus the envelope seed and tick, and nothing else.
- No production code calls save or load: a repo-wide grep finds the pair only
  in `crates/game/src/lib.rs` (definition plus tests). The demo window has no
  save key, no autosave and no load path.

So a save today remembers the score and a coordinate. It cannot rebuild the
match: it has no fighter count, arena size, table/rules version, spear/height
flags, weapon rows, world identity or inputs.

## Why `MatchSnapshot` cannot fill the hole

`LiveMatch::snapshot` (`crates/arena/src/live.rs:817-852`) returns `None` when
any of these holds:

- the match has a non-empty obstacle world (the demo's kit walls),
- `rules != ArenaRules::V1` (the demo runs v2),
- the spear is armed (the demo passes `--spear`),
- the height channel is armed.

`MatchSnapshot` (`crates/arena/src/lib.rs:571-609`) carries the config, tick,
fighter states, wishes, buttons, fight state, match cursor/version/phased
flags, brains, locos and destruction. It carries no `MatchWorld`, no
`ArenaRules`, no spear slots, and none of the v2 balance, stagger or limb
state. Those all ride the checkpoint hash, so a resume from it would silently
drop them. `from_snapshot` (`crates/arena/src/live.rs:854-902`) therefore
resumes the legacy open arena only. Under the arena `sim` feature the method
does not exist at all (`#[cfg(not(feature = "sim"))]`).

This is why the demo match, which is exactly world + v2 + spear, gets `None`.

## What a save that carries the match needs

1. The match identity, not just the seed: fighter count, half-extent, move
   table / rules version, spear, height, per-slot weapon rows and the input
   delay. `MatchSpec::identity` (`crates/arena/src/spec.rs:177-212`) already
   folds exactly these; the kit world is deterministic content, so its digest
   is enough to rebuild it.
2. The human input tape. `App` already keeps it: `tape: FighterTape`
   (`crates/game/src/lib.rs:795`) and `rebuild_live` (`:1487-1498`) rebuilds
   `spec().live()` and replays `0..ticks` from it. The arena AI slots are
   deterministic and refill themselves.
3. A version bump and a migration. The payload is a fixed 25 bytes at version
   1; new fields need `PROGRESS_VERSION = 2` and a v1 loader
   (`GameProgress::decode` already refuses a future version).
4. A size decision. `FighterInput` encodes to 10 bytes
   (`crates/input/src/fighter.rs:105-114`); a 100,000-tick one-slot tape is
   about 1,000,000 bytes, inside the 1 MiB `MAX_PAYLOAD_BYTES` by about 48 KiB
   before any header. Two human slots double it over the cap. Sidecar file or
   a larger payload cap is a decision, not a detail.
5. A caller. Nothing in `crates/game/src/bin/arena.rs` or `windowed.rs` calls
   save or load today, so even the counters are only exercised by tests.

`engine_net::replay::ReplayTape` (`crates/net/src/replay.rs:69-87`) is the
ready-made shape: config, `spec_digest`, inputs, one checkpoint per tick and a
keyframe every 600 ticks; `record_tape_under` / `play_tape_under` already
refuse another world or rules by name and report the first divergent tick. It
resimulates from tick 0, so it verifies a resume but does not shrink the tape.

## Smallest honest next step

Do not grow `MatchSnapshot` a world. Extend the progression payload to v2 with
the `MatchSpec` identity fields plus the input tape, and on load build
`spec().live()` and replay to the saved cursor. Reuse `ReplayTape` as the
verifier. Prove it with one save at tick 90,000 of a 100,000-tick demo run,
load, replay the last 10,000 ticks, and require the same final hash
`c600e309490b8095` as the unbroken run. That is the resume leg, at the one
length the demo ships.
