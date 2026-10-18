STEAL-SCOUT block 6 card, read 2026-10-18. Donor: gage-marshall/lp-engine (Rust engine + Python tools; runs LovePlus+ (NDS, 2010) from the player's own dump; 7 stars).
Donor sha 0c73f43f4fea2fb722f479f05161ba90354da3f9 (main, 2026-10-18). Read: file bodies fetched live today (README.md, docs/SAVE_RECORD.md, docs/SAVE_FORMAT.md, docs/LABELING.md, docs/UI_PROVENANCE_AUDIT.md, docs/RENDER_PRIMITIVES.md, docs/SCRIPT_VM.md, sandbox/flow/verify_all.py, tools/autolabel.py, engine/examples/save_diff.rs, engine/src/flow/code.rs, plus the tree). No build, no run. No bytes vendored: ideas only.
Gates: golden d57b5c8ae3096614 is P2P on every row below; nothing here touches a tick or a hash.
Related: docs/research/steals-2026-10-18/06-new-engines.md (block index), 00-combined.md.

## What it is

An engine built around one retail game: the game's own logic runs unchanged (its SBIN script VM), and the hardware around it is reimplemented (graphics, sound, touch, clock, saves). The interesting part is the proof method. Every claim about the port is anchored to something recorded from the real machine -- a call log, a save byte table, a regenerated frame -- and the game dump is never vendored.

## Licence

MIT, read live today from `LICENSE`: "MIT License, Copyright (c) 2026 gage-marshall". GitHub's licence API agrees (SPDX `MIT`). No game code or assets are distributed. Voice playback uses an ITU-T G.722.1 Annex C decoder licensed from Polycom (README:121-123); irrelevant to any idea we take.

## What we have (checked, not guessed)

- A crash-safe per-tick journal: crates/proof/src/journal.rs:1-15 (one self-delimiting record per tick, flushed after each; a truncated trailing record is dropped, a damaged earlier record fails hard).
- A versioned save envelope: crates/save/src/persist.rs:1-7, :64, :117 (seal/open/migrate; the same payload bytes twice).
- A provenance gate with stable failure codes: crates/assets/src/provenance.rs:1-8, :50, :258, :299, :355, :380.
- Replay is a marker grid, not per-tick state: crates/net/src/replay.rs (12-byte marker every 600 ticks, per the 2026-10-12 scout).

## Steals (ranked by value per hour)

1. **A recorded host-call log replayed against the port, frame by frame, with state coverage** | lp-engine@0c73f43f4fea:sandbox/flow/verify_all.py:1-3 (usage: "Check every recorded call log against the port and merge the state coverage"), :12-40 (per log: frame count and a status; a divergence prints `MISMATCH: <detail>` and counts `bad`; the end line is "%d logs, %d diverged; %d frames, %d calls, %d loads, %d stores checked"), :41-46 (states seen, each having run at least one frame) | MIT | our files: crates/proof/src/tape.rs, crates/proof/src/bin/arena_line.rs, crates/arena/src/recorder.rs |
   Our proof compares our sim to our own frozen tape. This donor compares two implementations of the same logic (retail vs port) through a log of the calls/loads/stores between them, and reports coverage of the states touched. Map: record the sim's per-tick read/write set (which subsystem read or wrote which field family) as a log; replay a candidate build's log against the frozen one; report per-log OK/MISMATCH plus which states no run has ever exercised. Moves: "the hash differs at tick 412" becomes "load #3 in system `stamina` first differed at frame 412, and 5 of 61 states were never exercised". Kill: if the log costs more bytes than the freeze it replaces, or needs a tape/WIRE bump, keep it test-only.

2. **A whole frame regenerated bit-exactly from game data, with every input traced to a source** | lp-engine@0c73f43f4fea:docs/RENDER_PRIMITIVES.md:1 ("evidence-driven"), :13-27 (method and tools, all in sandbox/), :28 ("Verified: the whole frame generated from game data (bit-exact)"), :89-98 (a table of every frame input -- heroine, outfit label, accessory, hair label, time mode, backdrop time of day, camera preset, motion key -- each resolved from a named game source/offset) | MIT | our files: crates/proof/src/arena_matrix.rs, crates/playtest/src/frame_check.rs, crates/render |
   We judge frames by SSIM/FLIP thresholds (the 2026-10-12 card's LF15). This donor instead regenerates the frame from data alone and compares bit for bit, and its doc lists where each visual input comes from, so a mismatch names an input rather than an image. Map: for one frozen scene, drive the renderer from extracted scene data (no live sim) and compare pixels exactly; list each visual input's source row. Kill: if the GPU path cannot be bit-stable across vendors, keep bit-exactness for the CPU twin only and thresholds for the GPU capture -- do not claim bit-exact where you cannot hold it.

3. **A provenance audit row for every visible value, with a result column** | lp-engine@0c73f43f4fea:docs/UI_PROVENANCE_AUDIT.md:1, :26-37 (a table: module | value | kind (`table` or `code`) | source (module + address + accessor) | result `ok`) | MIT | our files: crates/assets/src/provenance.rs:1-8, :50, :258, :355, :380 |
   Our provenance gate covers shipped assets; the donor audits the drawn UI the same way, so an unexplained pixel is caught like an unlicensed asset. Map: extend the provenance rows to on-screen readouts (hp bar, timer, damage numbers) -- the field family, the code that derives it, a result -- so a HUD element with no source fails the release audit. Kill: if a row needs runtime state rather than a static source, mark it `runtime:<system>` and keep the gate to static sources only.

4. **A crash-only session recording: the starting save plus inputs, kept only when the game dies** | lp-engine@0c73f43f4fea:README.md:87-89 ("Each play session is recorded quietly: the save it started from and your inputs, a few kilobytes. If the game's code ever stops, the recording is kept and the app tells you where. Attaching it to an issue lets the exact session be replayed, step for step.") | MIT | our files: crates/proof/src/journal.rs:1-15, crates/proof/src/tape.rs, crates/net/src/replay.rs |
   Our journal flushes every tick and stays on disk; the donor keeps a small ring and preserves it only on a crash, with the path in the report. Map: a bounded in-memory ring of (seed, tick, input), written out only by the crash hook, with the path in the crash message; the full journal stays opt-in (`Keep Session Recordings`). Moves: a crash report arrives attached to the exact session. Kill: if the ring cannot reproduce the crash tick from its first kept tick, widen it or keep the full journal for that run.

Not worth it: the SBIN VM reimplementation and the NDS asset extractors (one retail game's formats). The tier-A/B/C function-labeling registry (docs/LABELING.md:3-23, tools/autolabel.py) is a reverse-engineering aid with no counterpart in our tree. The save byte table (docs/SAVE_RECORD.md:1-13) records a layout derived from accessors; our save layout is authored, not recovered.
