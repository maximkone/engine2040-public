# STEAL CARD: 8b-is-engine (wave 2026-10-18/2, position 22)

Donor: `peterlodri-sec/8b-is-engine` @ `main`, pushed 2026-10-18T12:08:44Z, created 2026-09-08, 0 stars, 40,371 KB. Rust 2024 + Go 1.26, NATS actor mesh, Vulkan/wgpu, wasm; crates include `world-core`, `ternary`, `spherepop`, `qdecorators`, `pipeline`, `mesh-node`, `mesh-relay`, `ultrawild`, `admissibility`, plus `entheai/` and `6502-is/` trees. Read live 2026-10-18 with `gh api` and `raw.githubusercontent.com`, read-only: no clone, no build, no bytes vendored.
Licence: **EOS-CLA v1.0** — "Eternal Open Substrate & Contributor Mutual Trust Agreement", read live from the root `LICENSE`: 12 sections, clauses for AI/agent contributions, a "Capability-Graph Binding" clause and an "Absolute Disclaimer of Liability" ("The Crash-and-Burn Clause"). Custom and not OSI-approved; `gh api license.spdx_id` reports `NOASSERTION`. Treat as **ideas only**, take no bytes.

## What it is

A very large, self-described "world-model sim-MMO + game engine" in the middle of a wide design document: a fixed-step world (`world-core`, its own `SimWorld` with needs/position/keepers), a ternary-quantized model lane, a Gray-code wire, slab arenas, a compression lane, a NATS mesh, a browser client, and a 6502 machine for a side world. Most of it is not our shape. Four pieces are real, standalone and portable.

## Steals

**1. Ternary (1.58-bit) weight lane: 4 trits per byte, 2 bits each** | `crates/ternary/src/pack.rs:12-35` (`PACK_PER_BYTE = 4`, `packed_len(n) = n.div_ceil(4)`, encoding `0b00→0`, `0b01→+1`, `0b10→-1`, `0b11 reserved` never written, and `pack_trits` panics on anything outside `{-1, 0, +1}`), `:37-41` (`field()` with strict/lenient unpack — strict refuses `0b11`, lenient maps it to 0), and `crates/ternary/src/lib.rs:1-31` (weights in `{-1,0,+1}`, "the forward pass is an integer GEMM that is bit-exact across architectures ... the same i32 sum on an AMD Zen CPU (AVX2), an Apple/ARM Neoverse core (NEON), a WASM sandbox (simd128) ... because integer addition does not care about the order you add in, and the single float scale is applied once, at the end") | EOS-CLA (ideas only) | SIM-adjacent (a shipped net, not the tick) | lands on `crates/nn` (`lib.rs`, `quant.rs`, `qat.rs` = int8 today) and the AI door's offline eval
Idea: a third precision lane under int8. Two bits per weight instead of eight is a 4x smaller artifact, and the integer-sum contract means the same numbers on every target, so a ternary policy net passes the same golden proof an int8 one does. Section `{.tern}` = "sections + sha256 trailer" (`crates/ternary/src/lib.rs:15-16`) is the self-check shape.
Moves: a shipped policy net's bytes and its load path shrink 4x; the pack/unpack round trip is exact and testable in isolation (`pack` → `unpack` for every value, plus the reserved field refused).
Kill: kill the lane if the ternary-quantized policy loses the eval/win-rate number the int8 net holds, or if a 4x smaller file moves no budget (web bytes, VRAM) — the quantize step is otherwise pure cost.

**2. Gray-code delta wire, and a changed-bits cost meter to judge it** | `crates/world-core/src/gray.rs:16-29` (`gray_encode(v) = v ^ (v >> 1)`, `gray_decode` in five XOR-shifts), `:31-36` (`changed_bits(a, b) = (a ^ b).count_ones()`, documented as "the hot path's cost meter: `+1` steps toggle exactly 1 bit, so a well-behaved delta costs one changed bit; a scramble shows its face as a wide spread"), `:38-39` (the delta wire) | EOS-CLA (ideas only) | SIM (wire) | lands on `crates/net/src/delta.rs` (changed fields carry full i32 mm, no residual coding; MP9 gate is >= 20% saving)
Idea: two things. One, send a slowly-changing integer Gray-coded, so a ±1 step is a single-bit toggle. Two, keep `changed_bits` as a *measurement* over a recorded bout, so a coding change is judged by a number instead of by feel.
Moves: MP9's byte budget gets a second, cheaper lever beside residual coding; the meter gives the before/after in changed bits per tick per field.
Kill: kill if the MP9 gate is already met and the meter shows the same spread as plain little-endian — Gray pays only on runs of ±1 and costs a decode per field.

**3. `Pow2Slabs<T>`: an append-mostly arena whose ids never move** | `crates/world-core/src/pow2arena.rs:1-17` (elements in slabs sized 1, 2, 4, …; "an id never moves once handed out"; "iteration is slab order then insertion order"; worst-case slack <= 2x the live count), `:28-40` (`next_pow2`, saturating growth), and the stated contrast with a swap-remove register ("`Events` register's swap-remove stays for the game-loop arena; this lane is the append-mostly arena — spawn queues, ledger staging, render batches") | EOS-CLA (ideas only) | SIM-adjacent (storage) | lands on `crates/arena` (checkpoint vector, `spine_step`) and the net spawn/debris ledger
Idea: where a swap-remove renumbers a live element, a slab arena keeps every id valid to the end of the bout while still growing amortized. Two properties make it hash-safe: ids are handed out in push order, and iteration is slab order then insertion order.
Moves: spawn queues and render batches keep stable handles across a bout; a despawn costs a tombstone instead of moving the last element into the hole.
Kill: kill if the ledger is bounded and preallocated (a fixed ring is smaller and faster); the slab pays only where the count grows unbounded within a bout.

**4. A model file that carries its own digest** | `crates/ternary/src/lib.rs:15-16` (".tern format, sections + sha256 trailer"), `crates/ternary/src/format.rs` (`Checkpoint`, `SectionKind`, `TernLayer`, `load_checkpoint`), `crates/ternary/src/lib.rs:48-50` (the committed base model is embedded once and shared by the golden, the sampler and the wasm ABI, "one data segment in the wasm binary") | EOS-CLA (ideas only) | TOOLING / SIM-adjacent | lands on `crates/nn` and the content cook
Idea: a weight artifact is named sections plus a trailing digest over those bytes, loaded by one function that refuses a truncated or swapped file loudly. Same artifact serves the native build and the wasm build, so the golden covers both.
Moves: a corrupt or stale net fails at load with a named error instead of producing different logits and a slow-drifting eval.
Kill: kill if the cook already writes a digest that an existing gate verifies; then only keep named sections (a section index is useful for a partial load, a trailer is not).

## Not worth it (read, rejected)

- `crates/world-core/src/tick.rs:22-46`: the fixed-step accumulator is f64 (`Clock{fixed, accumulator, last: f64}`, `max_steps = 8`, wall clock fed in as `now: f64`). We are stricter with an integer tick; nothing to take. Worth keeping only its two guards as text: a non-finite or negative `dt` returns 0, and the accumulator is clamped before the step count is taken.
- `crates/world-core/src/gaia.rs`, `kompress.rs`, `ultrawild`, the NATS mesh and `6502-is`: a different product (world-memory, mesh, emulation), not a duel engine.
- The licence: an "eternal open substrate" CLA with agent clauses is not a licence we can follow into a dependency; ideas only, never a dependency edge.

DONE 4 steals, 7 donor files read (ternary/pack.rs, ternary/lib.rs, world-core/gray.rs, world-core/pow2arena.rs, world-core/tick.rs, world-core/tests/lockstep.rs, LICENSE), plus a tree listing.
