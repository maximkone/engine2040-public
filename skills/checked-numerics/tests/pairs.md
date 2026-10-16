# Checked numerics pairs

## Case 1: narrowing packet length (net)
Input: `let n = buf.len() as u16;` on an untrusted network buffer length.
With skill: flag check 1 (core, net, save); require `u16::try_from(buf.len())` with an error on overflow, plus a unit test at 65535 and 65536.
Without skill: a reviewer accepts the `as` as harmless; lengths above 65535 truncate and the parser reads a short frame.

## Case 2: float speed into Fixed ticks (core)
Input: `let v = (speed_f32 * 65536.0) as i64;` inside the tick, then `Fixed(v)`.
With skill: flag checks 4 and 5 (core, physics); require truncation plus clamp review, NaN guard, and a checked multiply before narrowing, proven by the golden hash and a Fixed rounding test.
Without skill: a reviewer checks the happy path only; NaN becomes zero and huge values clamp, so replays diverge from the expected ticks.

## Case 3: index cast from save (ecs)
Input: `let e = entities[count as usize];` where `count: u32` comes from a save file.
With skill: flag check 6 (ecs, arena, render); require a bounds check against `entities.len()` before indexing, with a test for `count == len`.
Without skill: a reviewer sees a widening-looking cast and passes it; a crafted save panics or reads the wrong entity.
