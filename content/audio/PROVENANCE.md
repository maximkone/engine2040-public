# KEMAR measured HRIR sphere — provenance (2026-10-24, AAC7)

First measured ears in the engine. The 8-octant synthetic filter in
`crates/audio/src/hrtf.rs` stays the shipped path (flag off, bit for bit);
a 108-direction measured sphere sits beside it behind
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

- Grid: **36 azimuth columns** (10 deg apart, azimuth 0 = straight ahead,
  clockwise = right) x **3 elevation rows** (-40, 0, +40 deg) = **108
  directions**. Azimuth wraps; elevation clamps at the poles.
- Each cell is the measured HRIR nearest to the grid node: within 2.5 deg at
  elevation 0 (the compact set steps 5 deg there) and within 3.2 deg at
  +-40 deg (the compact set steps 6.4 deg there). Left half-sphere cells with
  no measurement of their own are the same measurement with the two ears
  swapped (KEMAR compact covers 0..180 deg only).
- Per ear: **16 taps**, starting 3 samples before that ear's peak (the
  peak is the stable arrival marker in a 128-sample compact IR; a level
  threshold is not). Both ears of a direction are scaled by one common
  factor, so the ILD is the measured one. The ITD is the measured arrival
  difference in samples, never a delay law.
- Rate: **44.1 kHz** (the capture rate).
- Format: `kemar-hrir-sphere.bin`, **7016 bytes**, sha256
  `c2df052d4002eb3065d6cf1bb15a0ba8f06d6f73b7a3a6e6e23cb6b18222e522`:
  one ASCII header line `HRIRSPH1 44100 16 36 3 -40 0 40`, then 36 `u16`
  azimuth columns in tenths of a degree, then the taps as little-endian
  `i16` in `[elevation][azimuth][ear][tap]` order (108 x 2 x 16 values).
  Re-readable with `readU16LE`/`readI16LE` and no new dependency.
- Same licence as the 48-direction subset it replaces (same page, same
  download): no new bytes from any other source, so no new licence line is
  needed; the file above is the whole dataset.
- The same numbers are compiled into `hrtf.rs` as `SPHERE_TAPS` /
  `SPHERE_ONSETS` const tables (no file IO on the audio thread), and the
  test `dataset_bin_matches_compiled_table` parses the committed `.bin`
  and fails if the two ever drift.

## Measured, on the tree that ships (44.1 kHz, 16 taps)

Held-out protocol (unchanged from the 48-direction run): drop every 4th
azimuth column, rebuild it as the midpoint of its neighbours, score against
the measurement that was dropped. "err/sig" is
`10 log10(err energy / signal energy)`: negative is good, 0 dB means the
error is as loud as the signal.

| grid | ears | err/sig signed rms | worst err/sig | 8 kHz worst | ITD mean | ITD worst |
|---|---|---|---|---|---|---|
| 16 az x 3 el (48 dirs, 22.5 deg) | 24 | +6.29 dB | +4.16 dB | 10.75 dB | 3.67 | 12.00 |
| **36 az x 3 el (108 dirs, 10 deg)** | **54** | **+11.67 dB** | **-0.12 dB** | **18.13 dB** | **0.96** | **4.00** |
| 72 az x 3 el (216 dirs, 5 deg) | 108 | +17.40 dB | +5.93 dB | 12.77 dB | 0.48 | 4.50 |

Read the signed rms as an error *ratio*: at 16 columns it is above the
signal, at 36 every single rebuild is quieter than the measurement (worst
-0.12 dB), and at 72 it climbs again only because the dropped columns now
land 5 deg from a measurement the 6.4-degree compact set repeats.

- **The 6 dB 8 kHz bar is still not met.** 16 columns: 10.75 dB (a loud
  band rebuilt too quietly); 36 columns: 18.13 dB; 72 columns: 12.77 dB.
  Density does not fix it, because at 8 kHz the pinna response flips phase
  across a 10-20 degree neighbour gap, so a linear tap blend cancels a band
  the measured direction carries. Which columns are dropped changes with
  density, so the three worst values are not the same cell. The remaining
  routes are magnitude/phase (log-spectral) interpolation or a finer
  measured set in elevation; neither is done here.
- Baseline ITD (16 columns) is the figure the row recorded at landing;
  the re-run on the same table reads 4.17 mean / 14.50 worst, so the
  improvement is measured on both readings.
- Query cost: **0.19 us/query** (mean of 20 000, measured on the CI runner).
- State: `HrtfProcessor` stays under 4 KB (test asserts it); the table is
  code, not processor state.

## What is not SOFA

The row says "authored as SOFA". These bytes are not a `.sofa` file:
SOFA is HDF5, which needs a reader (and a writer) this lane does not have.
The subset is a plain deterministic `.bin` plus a Rust const table. A SOFA
export is open, not hidden.

## Files

| Path | Bytes | What |
|---|---|---|
| `content/audio/kemar-hrir-sphere.bin` | 7016 | 108 directions, 16 taps, 44.1 kHz, header + i16 |
| `content/audio/PROVENANCE.md` | this file | licence read, subset, numbers |
| `crates/audio/src/hrtf.rs` | | `measured_hrir_at`, `set_measured_sphere`, const table, tests |
