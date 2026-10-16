---
name: checked-numerics
description: Use when a diff adds or changes a numeric `as` cast, integer math, or float use in sim code and the reviewer must decide if the cast is safe, needs TryFrom, or needs checked math.
version: 0.1.0
author: skillworks
---

# Checked numerics

Eight checks for numeric `as` casts in sim code. Name the crate in each finding (core, physics, net, ecs, arena, render, save, rules). PASS needs the cast class plus a test or hash run, never a promise.

1. **Narrowing int cast.** Ask: can every source value fit the target, else TryFrom? Flag: `v as u8` keeping low bits on overflow (core, net, save).
2. **Sign change.** Ask: can a negative reach an unsigned target, or a large unsigned reach a signed one? Flag: a negative `as u32` becoming huge; use TryFrom or a range guard (core, physics).
3. **Safe widening.** Ask: is this a widening that keeps every value exact? Flag: rewriting `u8 as u32` or `i32 as i64` with extra machinery (assets, render).
4. **Float to int.** Ask: is truncation toward zero plus clamping at the edges the wanted result? Flag: out-of-range float clamping, NaN giving zero, fraction loss inside hashed state (core, physics).
5. **Fixed-point scaling.** Ask: does Fixed math scale by SCALE with a checked i64 step before narrowing? Flag: raw `as` skipping the scale, or divide-then-multiply losing the fraction (core).
6. **Index and word size.** Ask: is the index a validated usize, with usize width treated as platform-dependent? Flag: `len as u32` stored in a save, or `u32 as usize` indexing without a bounds check (ecs, arena, render).
7. **Overflow mode.** Ask: which overflow mode did the author choose on purpose? Flag: plain `+` where debug panics and release wraps differ; sim keeps checked, saturating, or wrapping with a reason (core, physics, net).
8. **Bool, char, enum.** Ask: does the cast go outward to an integer only? Flag: integer-to-bool or integer-to-char by `as`; use a comparison or a checked constructor instead (rules, save, net).

## Do / Don't
- Do: keep a safe widening `as`, convert a narrowing fallible value with TryFrom, and pick checked, saturating, or wrapping for sim math. Don't: leave a narrowing or sign-changing `as` on untrusted data.

source: Blandy et al., Programming Rust (3rd ed. early release), ch. 2
