# Pairs for hearing-cull-checks

## 1. Far footstep heard over near gunshot

Input: the new `heard_by` compares loudness first, so a footstep at 2500cm hears while a gunshot at 3900cm is dropped; asked whether GS28 range holds.

Answer with the skill: fail checks 1 and 2. Read kind, range and life from the table and test distance before strength; the 2500cm footstep never hears, the 3900cm gunshot hears at its start tick.

What a skill-less answer misses: it trusts the louder label, so far-quiet beats near-loud and stealth breaks.

## 2. Fresh list each tick flaps

Input: the brain rebuilds the audible set from scratch every tick with no drop flags, and mixes two live noises by averaging; asked whether the heard voice is stable.

Answer with the skill: fail checks 5 and 7. Keep the prior set, mark all for drop, unmark still-audible ones; a louder live noise replaces the current one by integer compare, never mixes.

What a skill-less answer misses: it accepts the rebuild because one frame sounds right, leaving flap between frames and a mixed strength no table predicts.

## 3. Float strength drifts across runs

Input: `heard_by` returns an f32 level with a linear fade after tick 48 of 120, and the same ticks twice give different bytes; asked whether x2 equal holds.

Answer with the skill: fail checks 6 and 8. Return `Option<u16>` integers in `0..=1000` scale like `crates/ai/src/eqs.rs::SCORE_MAX`, refuse bad ids like `crates/ai/src/eqs.rs::EqError`, and prove identical bytes twice with `cargo test -p engine-ai --lib` green.

What a skill-less answer misses: it likes the smooth float fade, so replays drift and the 130-tick gunshot cutoff moves with no receipt.
