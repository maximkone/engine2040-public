# content/reference/

The machine-eyes reference set: a tier's real captures, the frames the look
gate scores a capture against.

## What is here

- `index.json`: the manifest. `rows` holds the **real** captures, ten per
  delivery tier: the shot name, the PNG path, the size, the seed, the camera,
  the sim tick and the **pixel digest** (FNV-1a hex over the decoded pixels),
  plus a `source` naming the binary or browser, the adapter and the command
  line that drew it.
- `<tier>/*.png`: those captures, one file per row, and the `fragment.json`
  the shooter wrote beside them (canvas, adapter, driver, command, one entry
  per shot with the digest over the PNG bytes).
- `recipe`: three painted stand-in rows, kept as the **second source**. They
  are not captures: `reference::rgba` paints them from the canvas and the
  seed, integer only, so the digest is reproducible offline on any machine.

## The tiers

| Tier | Canvas | Leg | Rows |
|---|---|---|---|
| `native` | 1280x720 | the desktop window | 10 real |
| `web` | 640x360 | the browser page canvas | 10 real |
| `potato` | 320x180 | the low-end fallback | 10 real |

A tier is either absent or complete: ten real rows. `reference_set` in
`crates/playtest/src/look/reference.rs` refuses a tier that holds any other
count.

## The scene

All ten native and potato shots are the shipped arena on camera `arena600`
(seed `2040`) at ticks `0, 60, 120, ... 540`, so the orbit spreads across the
ten frames. Native draws at 1280x720 with the presenter's default quality;
potato draws at 320x180 with the low tier's shadow, particle, decal and
texture caps. The web tier is the real page's own 640x360 canvas and
`procedural-arena` camera. The recipe rows are the `arena-duel` stand-in: a
stands band over textured sand with two fighters at their staged duel
positions.

## Shooting a tier

```
cargo build --release -p engine-game --features app --bin arena
target/release/arena.exe --reference-set --reference-tier native --reference-dir content/reference
target/release/arena.exe --reference-set --reference-tier potato --reference-dir content/reference
node tools/reference-captures.mjs --tier native  # verify, digest, merge, print
node tools/reference-captures.mjs --tier potato
```

The bin draws one tier headless on the offscreen twin and refuses loudly when
no GPU adapter is present: a real tier is never painted. The tool reads the
fragment, checks each PNG's canvas, seed, tick set and byte digest, computes
the **pixel** digest, prints one line per shot and merges `index.json`. Two
passes over two captures must print the same `pixel_fnv` per shot; that is the
determinism proof for the tier. The web tier is shot from the real page by
`tools/reference-web.mjs`.

## Reading the set

```rust
let set = ReferenceSet::load(&content/reference())?;   // reads and digest-checks every PNG
let refs = set.references("native")?;                  // the tier's captures
```

- A real row whose PNG is missing, the wrong canvas, or a different digest is
  a **named refusal** (`MissingCapture`, `SizeMismatch`, `DigestMismatch`).
  Re-shoot the tier; never edit a digest by hand.
- A tier the set does not hold is `ReferenceError::UnknownTier`, never a
  silent pass.
- The judge pairs a capture with the reference of the tier the capture ships
  on, and reports L8 (the full-frame SSIM against that reference) beside the
  second metric, SSIMULACRA2 v2.1.

## Why the digest is over pixels

The digest pins the decoded pixels, not the PNG bytes. The bytes depend on
the encoder (its version, its filters, its zlib settings); the pixels are the
capture. The bin can only see its own PNG bytes, so its fragment files
`png_fnv` over the bytes and the tool files `pixel_fnv` over the decode: two
names, two gauges, no ambiguity.
