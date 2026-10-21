# The class corpus

Twenty classes, ten per faction, in data. Clean room: the names are new, the
mechanics are re-derived from public descriptions, never from client files.

## What is here

- `schema.json` — the contract every class file obeys.
- `roster.json` — the twenty: id, faction, tier, branch, role, growth, and the
  signature that makes the class worth playing. The shape is fixed: Wardens
  four starters and eight advanced, Ashkin two starters and six advanced.
- `<class_id>.json` — one file per class: growth, loadout, abilities with
  their cooldowns, range bands, cast times, effects and telegraph.

## The rules live elsewhere

`content/systems/scaling.md` holds the numbers that every class obeys (Rule
S1 to S7: one damage rule, one health model, the cast floor, the growth
shape, the counters, the auras). `content/systems/stats.md` holds the
baselines and the effect vocabulary. A class file never invents a number: it
names a stat and the rule does the rest.

## The gates

    node tools/class-validate.mjs     -> RESULT class-validate ok files=20 errors=0
    node tools/content-cross.mjs      -> RESULT content-cross ok classes=20 maps=3 problems=0

`class-validate.mjs` checks each file against the schema and the roster
shape. `content-cross.mjs` checks the corpus against the maps, the war
rules, the economy and the items, so a class that names a stat nobody
defines fails here.

## The loader

`crates/game/src/class_sheet.rs` reads the corpus at runtime: a std-only
JSON reader, no serde, with errors that name the file and the field path
(`abilities[0].range`). It reads all twenty files, skips `roster.json` in
the directory loader and refuses it loudly when it is asked for by name.

Proof:

    cargo check -p engine-game
      -> Finished `dev` profile [optimized + debuginfo] target(s) in 31m 25s
    isolated harness: test result: ok. 19 passed; 0 failed
      (17 module tests + 2 real-content proofs)
    cargo clippy --all-targets -- -D warnings -> clean
    rustfmt --edition 2021 --check -> clean
    node tools/rust-quality.mjs --crate game --owned crates/game/src/class_sheet.rs
      -> RESULT PASS: score 96 (A), 0 ratchet regressions

The loader follows the content, not the other way round: the schema says an
aoe ability must carry a telegraph, so the loader refuses `telegraph: none`
on an aoe.
