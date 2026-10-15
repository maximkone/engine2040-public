// Run: node --test tools/cast-safe.test.mjs
// Checked cast discipline: valid conversions pass, silent truncations throw.
import assert from "node:assert/strict";
import test from "node:test";
import { cast, tryCast, isCastable } from "./cast-safe.mjs";
test("in-range integers pass through", () => {
  assert.equal(cast(0, "u8"), 0);
  assert.equal(cast(255, "u8"), 255);
  assert.equal(cast(-128, "i8"), -128);
  assert.equal(cast(32767, "i16"), 32767);
  assert.equal(cast(65535, "u16"), 65535);
  assert.equal(cast(2147483647, "i32"), 2147483647);
  assert.equal(cast(4294967295, "u32"), 4294967295);
});
test("64-bit targets return exact BigInt", () => {
  assert.equal(cast(0, "i64"), 0n);
  assert.equal(cast(9223372036854775807n, "i64"), 9223372036854775807n);
  assert.equal(cast(-9223372036854775808n, "i64"), -9223372036854775808n);
  assert.equal(cast(18446744073709551615n, "u64"), 18446744073709551615n);
  assert.equal(cast(42, "u64"), 42n);
});
test("overflow throws loudly instead of wrapping", () => {
  assert.throws(() => cast(256, "u8"), RangeError);
  assert.throws(() => cast(300, "u8"), RangeError);
  assert.throws(() => cast(-1, "u8"), RangeError);
  assert.throws(() => cast(128, "i8"), RangeError);
  assert.throws(() => cast(4294967296, "u32"), RangeError);
  assert.throws(() => cast(9223372036854775808n, "i64"), RangeError);
  assert.throws(() => cast(-1n, "u64"), RangeError);
});
test("fraction would truncate, so it throws", () => {
  assert.throws(() => cast(1.5, "u8"), RangeError);
  assert.throws(() => cast(1.5, "i32"), RangeError);
  assert.throws(() => cast(0.1, "u64"), RangeError);
  assert.equal(cast(1.5, "f64"), 1.5);
});
test("non-finite floats never cast to int", () => {
  assert.throws(() => cast(NaN, "u8"), RangeError);
  assert.throws(() => cast(Infinity, "i32"), RangeError);
  assert.throws(() => cast(-Infinity, "f32"), RangeError);
  assert.throws(() => cast(NaN, "f64"), RangeError);
});
test("wrong input types fail loudly", () => {
  assert.throws(() => cast("42", "u8"), TypeError);
  assert.throws(() => cast(true, "u8"), TypeError);
  assert.throws(() => cast(null, "i32"), TypeError);
  assert.throws(() => cast(1, "u7"), TypeError);
});
test("f32 guards overflow, f64 passes finite through", () => {
  assert.equal(cast(1.5, "f32"), Math.fround(1.5));
  assert.throws(() => cast(1e40, "f32"), RangeError);
  assert.equal(cast(1e40, "f64"), 1e40);
});
test("tryCast returns null and isCastable is false on value failure", () => {
  assert.equal(tryCast(256, "u8"), null);
  assert.equal(tryCast(5, "u8"), 5);
  assert.equal(tryCast(1.5, "u8"), null);
  assert.equal(isCastable(255, "u8"), true);
  assert.equal(isCastable(256, "u8"), false);
  assert.equal(isCastable(NaN, "i32"), false);
});
test("silent truncation matrix: 8 of 8 loud", () => {
  const vectors = [[256, "u8"], [300, "u8"], [-1, "u8"], [1.9, "i32"], [NaN, "u8"], [Infinity, "u32"], [9223372036854775808n, "i64"], [1e40, "f32"]];
  let caught = 0;
  for (const pair of vectors) {
    try { cast(pair[0], pair[1]); } catch (err) { caught += 1; }
  }
  assert.equal(caught, 8);
});
