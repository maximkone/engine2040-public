# LF10 pose-hold rethink — why the stall, and a different way through

LF10 (board `docs/SPRINT-18-gaps.md:1506`, FOCUS Feel) says poses hold
through the sim's hitstop. The code is on disk. Six helper runs tried the
same verify and got no green. This file names the cause, picks a different
proof seam, and gives the writer one exact proof command. It is a rethink,
not a subsystem plan: only sections 1-5 apply.

Status: code landed-but-unproven (`crates/game/src/pose_hold.rs`,
`crates/game/src/hitstop.rs`, hold hunks in
`crates/game/src/fighter_look_acts.rs:239-252,366-367`); board row still
open; link wall `arena-link-anon319-destruction-x2`
(`docs/BLOCKERS.md:139`) still stands.

## 1. Root cause

The code is done and the proof path is broken. LF10's Accept runs
`cargo test -p engine-game --features app --lib hitstop_hold`: the heaviest
link in the tree (game lib plus the app feature pulls render, net, arena
and destruction rlibs). Every run dies inside `rust-lld.exe` with undefined
`anon.*.llvm` symbols, the same wall that holds LF11, LF14, GA8 and EG21
(`docs/BLOCKERS.md:138-139,141`). Meanwhile `cargo check` stays green,
queue-cap PASSes, and zero-diff verifies confirm the files are untouched.
So each retry re-proves the toolchain is red, not the code. Six identical
verifies burned 72 h on an environment fault. AGENTS says the second
identical failure ends the step; this one ran four more times past that.

## 2. Different approach

No seventh identical verify. Split the row into a landed-first slice and a
deferred re-verify, on a seam the linker can cross today.

Slice A (land today, link-free): prove the two LF10 laws where linking
works. The sim authority (per-kind hitstop counts) lives in
`crates/combat`, which links fine (257 passed proven,
`docs/BLOCKERS.md:108`). The presentation wiring (hold mask, slot pick,
bridge freeze) compiles without any link step under `cargo check -p
engine-game --features app` (check-green proven on every BLOCKERS row).
Land the wiring gated on check, not on link.

Slice B (deferred, not the writer's gate): the exact LF10 Accept
(`cargo test -p engine-game --features app --lib hitstop_hold` x2) runs
once, off-peak, after the owner clears the wall
(`cargo clean -p engine-destruction` on a quiet tree). It is its own row
(LF19) that waits on the link wall instead of hammering it.

This is genuinely different: a different test seam (combat lib, not game
app lib), a smaller slice (check-gated landing), and deferred proof
(re-verify filed behind the wall, never retried in-run).

## 3. Proof command

The writer proves slice A with commands that never link the game lib.
Both must exit 0; run the test twice.

```
cargo test -p engine-combat --lib hitstop
cargo check -p engine-game --features app
```

Expected: test binary green x2 (exit 0, hitstop count and kind tests);
check green (exit 0, no link step, immune to the anon319 wall). Judge
replays the same two commands and adds
`node tools/workspace-check.mjs` (expect no new FAIL line). Forbidden as
writer proof: `cargo test -p engine-game --features app --lib hitstop_hold`
— that command belongs to LF19 only, off-peak, after the owner clean.

Done-when: combat hitstop tests green x2, game app check green, zero
game-link invocations, workspace-check with no new FAIL.

## 4. Writer's packet

Name: `0-lf10-pose-hold-slice-a`. Paths: `crates/game/src/pose_hold.rs`,
`crates/game/src/hitstop.rs`, and the hold hunks of
`crates/game/src/fighter_look_acts.rs` only; no new files; never touch
`camera.rs`, net, render, or destruction files from this scope. Proof
command: section 3, verbatim, with the expected exits. Stop rule: the
first link-env failure stops the run — file BLOCKED under fingerprint
`arena-link-anon319-destruction-x2`, no in-run retry; a second identical
failure ends the step per AGENTS and becomes a BLOCKERS row, never a
seventh verify.

## 5. Proposed SPRINT rows

- [ ] LF18 game: pose-hold slice A, land the wiring check-gated — keep the
  hold mask, slot pick and bridge freeze as they read on disk
  (`crates/game/src/pose_hold.rs`, `hitstop.rs`, `fighter_look_acts.rs`
  hold hunks). Tier A. Size S. Owns the three hold files only. Accept:
  `cargo test -p engine-combat --lib hitstop` x2 green plus `cargo check
  -p engine-game --features app` green; no game-link step. needs: none.
- [ ] LF19 game: pose-hold slice B, off-peak re-verify of the LF10 Accept —
  run `cargo test -p engine-game --features app --lib hitstop_hold` x2
  after the owner clears the link wall (quiet tree or
  `cargo clean -p engine-destruction`). Tier A. Size S. Owns nothing new.
  Accept: held slots frozen while others advance x2 green; golden
  unchanged. needs: LF18.
