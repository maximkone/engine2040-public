# KEMAR measured HRIR sphere — provenance (2026-10-24, AAC7)

First measured ears in the engine. The 8-octant synthetic filter in
`crates/audio/src/hrtf.rs` stays the shipped path (flag off, bit for bit);
a 48-direction measured sphere sits beside it behind
`HrtfProcessor::set_measured_sphere`.

## Licence read live from the download (2026-10-24)

- Source page: `https://sound.media.mit.edu/resources/KEMAR.html`
  (HTTP 200, saved from the same download).
- Files read: `https://sound.media.mit.edu/resources/KEMAR/compact.zip`
  (HTTP 200, 231461 bytes, sha256
  `0bcd69f8e8760cf8eacff4a89594ceca7d44fb94f28ecf0bf19970003bbfb3e0`),
  plus the page text itself and its archive listing.
- Licence text, verbatim from that page (line 78-81):

  > This data is Copyright 1994 by the MIT Media Laboratory. It is
  > provided free with no restrictions on use, provided the authors
  > are cited when the data is used in any research or commercial
  > application.

  So: **redistribution is allowed, attribution is required**. That is the
  licence of record here. The "CC BY 3.0" label that sometimes travels with
  KEMAR does not appear on the page the bytes came from, so it is not
  claimed.
- Citation: Bill Gardner and Keith Martin, "HRTF Measurements of a KEMAR
  Dummy-Head Microphone", MIT Media Lab, Perceptual Computing Technical
  Report #280, May 1994 (revised 2000).
- SADIE II (CC BY 4.0) was the first pick; it needs an HDF5/SOFA reader on
  this box and no download-to-licence-check loop finished inside the box,
  so the already-local, plainly readable KEMAR WAV set was used instead.
  CIPIC (research-only) was not used.

## The subset taken

- Grid: **16 azimuth columns** (22.5 deg apart, azimuth 0 = straight ahead,
  clockwise = right) x **3 elevation rows** (-40, 0, +40 deg) = **48
  directions**. Azimuth wraps; elevation clamps at the poles.
- Each cell is the measured HRIR nearest to the grid node: exact at
  elevation 0 for every column it shares with the compact set, and within
  3.5 deg of the node at +-40 deg (the compact set's step is 6.4 deg
  there). Left half-sphere cells with no measurement of their own are the
  same measurement with the two ears swapped (KEMAR compact covers
  0..180 deg only).
- Per ear: **16 taps**, starting 3 samples before that ear's peak (the
  peak is the stable arrival marker in a 128-sample compact IR; a level
  threshold is not). Both ears of a direction are scaled by one common
  factor, so the ILD is the measured one. The ITD is the measured arrival
  difference in samples, never a delay law.
- Rate: **44.1 kHz** (the capture rate).
- Format: `kemar-hrir-sphere.bin`, 3136 bytes, sha256
  `4368d04b97ac49c5da3c76e681cb80e7897a1af4011216a75d05e6abce4ceaba`:
  one ASCII header line `HRIRSPH1 44100 16 16 3 -40 0 40`, then 16 `u16`
  azimuth columns in tenths of a degree, then the taps as little-endian
  `i16` in `[elevation][azimuth][ear][tap]` order (48 x 2 x 16 values).
  Re-readable with `readU16LE`/`readI16LE` and no new dependency.
- The same numbers are compiled into `hrtf.rs` as `SPHERE_TAPS` /
  `SPHERE_ONSETS` const tables (no file IO on the audio thread), and the
  test `dataset_bin_matches_compiled_table` parses the committed `.bin`
  and fails if the two ever drift.

## Measured, on the tree that ships (44.1 kHz, 16 taps)

- Held-out direction error (every 4th azimuth column dropped, rebuilt as
  the midpoint of its neighbours, RMS energy error over 16 taps per ear):
  **6.29 dB RMS, 10.92 dB worst, 24 ears**.
- ITD error against the measured arrivals, same held-out set: **3.67
  samples mean, 12.00 worst**.
- 8 kHz band (one bin of the 16-tap DFT), held-out directions: **10.75 dB
  worst off**. The row's bar is 6 dB, so that bar is **not met** at this
  grid density; it needs a denser azimuth grid (or a magnitude/phase
  split), which is the follow-on.
- Query cost: **0.35 us/query** (mean of 20 000, measured on this PC).

## What is not SOFA

The row says "authored as SOFA". These bytes are not a `.sofa` file:
SOFA is HDF5, which needs a reader (and a writer) this lane does not have.
The subset is a plain deterministic `.bin` plus a Rust const table. A SOFA
export is open, not hidden.

## Files

| Path | Bytes | What |
|---|---|---|
| `content/audio/kemar-hrir-sphere.bin` | 3136 | 48 directions, 16 taps, 44.1 kHz, header + i16 |
| `content/audio/PROVENANCE.md` | this file | licence read, subset, numbers |
| `crates/audio/src/hrtf.rs` | | `measured_hrir_at`, `set_measured_sphere`, const table, tests |
