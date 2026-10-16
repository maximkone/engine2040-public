# pairs.md — coder-overlap-guard on-off test (3 cases)

Each case shows WITHOUT (no skill) and WITH (skill applied). Winner named per case.

## Case 1: 0-lf2-port-ibl-bind behind running render repair

Situation: `fail-ratchet-dup-render-repair` runs as builder on `crates/render/src/decals.rs` (claimed 12:03Z). Keeper batch offers `0-lf2-port-ibl-bind` on `crates/render/src/shaders/forward.wgsl`.

- WITHOUT: dispatch `0-lf2-port-ibl-bind` at once. Guard: `packet 0-lf2-port-ibl-bind held by the seat guard (paths overlap with running coder fail-ratchet-dup-render-repair: crates/render/src/shaders/forward.wgsl; give each coder...)`. Retry same packet: held again, coder slot wasted.
- WITH: read `Get-Content target/queue/claims.txt -Tail 20`, see render busy, run `node tools/board-slice.mjs --ready` for a non-render row (for example `0-eg41-perf-csv-rename` on `tools/...`), take that instead. No hold, work lands.
- Winner: WITH (one lookup avoids the hold).

## Case 2: cut-spot-torch-shadow behind the same repair

Situation: same running `fail-ratchet-dup-render-repair` on `crates/render/...`. Offer is `cut-spot-torch-shadow` on `crates/render/src/twin_shadow.rs`.

- WITHOUT: dispatch at once. Guard: `packet cut-spot-torch-shadow held by the seat guard (paths overlap with running coder fail-ratchet-dup-render-repair: crates/render/src/twin_shadow.rs; give each coder...)`. Round loses the second coder.
- WITH: see `crateOf` is the same (`render` equals `render`), park the cut packet in `ready/`, take `0-gs1-wiring-scan` on `tools/...` instead. Report `RESULT: BLOCKED - coder overlap cut-spot-torch-shadow held behind fail-ratchet-dup-render-repair crates/render` with the guard quoted.
- Winner: WITH (park plus a disjoint take beats a hold).

## Case 3: disjoint crate needs no guard ritual

Situation: running coder holds `crates/render/...`. Offer is a physics packet on `crates/physics/...` with no running physics coder.

- WITHOUT: dispatch at once. Guard: go (`crateOf` physics differs from render, no file match). Work starts in one round.
- WITH: same dispatch plus a full batch-plus-claims lookup and board slice before start. Same go, one extra round spent proving the seat already allowed it.
- Winner: WITHOUT (the ritual costs more than it saves when crates are already disjoint).

Score: WITH wins 2 of 3. Install.
