# content/reference/

The machine-eyes reference set: one capture per delivery tier, the frame the
look gate scores a capture against.

## What is here

- `index.json`: the manifest. One row per tier, with the shot name, the
  relative path of the capture, the size, the seed, the camera and the
  **pixel digest** (FNV-1a hex over the decoded pixels).
- No PNG bytes ride in git. The capture is a *recipe*: the same canvas plus
  the same seed paints the same frame twice, integer only, no wall clock and
  no RNG, so the digest is reproducible offline on any machine and the set
  cannot rot behind a stale renderer.

## The tiers

The tier axis is the delivery leg the capture ships on. The same scene, the
same camera and the same seed are painted at that leg's canvas:

| Tier | Canvas | Leg |
|---|---|---|
| `native` | 1280x720 | the desktop window |
| `web` | 640x360 | the browser page canvas |
| `potato` | 320x180 | the low-end fallback |

## The scene

`arena-duel` on camera `arena600` with seed `2040`: a stands band over
textured sand, with two fighters at their staged duel positions. The recipe
lives in `crates/playtest/src/look/reference.rs` (`reference::rgba`), the
same module the judge reads the set through.

## Reading the set

```rust
let set = ReferenceSet::load(&content/reference())?;   // index.json plus the digests
let refs = set.references("native")?;                  // the tier's captures
```

- A tier the set does not hold is a **named refusal**
  (`ReferenceError::UnknownTier`), never a silent pass.
- A row whose rendered capture no longer matches its filed digest is
  `ReferenceError::DigestMismatch`: re-render the row and file the new
  digest.
- The judge pairs a capture with the reference of the tier the capture ships
  on, and reports L8 (the full-frame SSIM against that reference) beside the
  second metric, SSIMULACRA2 v2.1.

## Why the digest is over pixels

The digest pins the decoded pixels, not the PNG bytes. The bytes depend on
the encoder (its version, its filters, its zlib settings), the pixels are the
capture. A digest over the pixels stays reproducible offline in any language
and never moves because a PNG was re-encoded.

## Materializing the captures

An operator that wants the PNGs on disk writes them from the manifest:

```rust
set.materialize(&target/reference/)?;   // one PNG per row, at the manifest paths
```

The judge never needs them on disk: it renders the row and compares pixels.
