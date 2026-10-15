// Idea-only, no copied code. Donor: rust-random/rand (MIT OR Apache-2.0) https://github.com/rust-random/rand . Mirrors crates/core/src/rng.rs Rng new/next_u64/next_u32/next_range (xorshift64 13/7/17, seed|1).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
const VECTORS_URL = new URL("../crates/core/testdata/rng_vectors.json", import.meta.url);
const MASK = (1n << 64n) - 1n;
function nextU64Word(state) {
let x = state & MASK;
x = (x ^ ((x << 13n) & MASK)) & MASK;
x = (x ^ (x >> 7n)) & MASK;
x = (x ^ ((x << 17n) & MASK)) & MASK;
return x & MASK;
}
function stream(seed, n) {
let s = (BigInt(seed) | 1n) & MASK;
const out = [];
for (let i = 0; i < n; i++) { s = nextU64Word(s); out.push(s.toString()); }
return out;
}
function loadVectors() {
const raw = readFileSync(VECTORS_URL, "utf8");
return JSON.parse(raw);
}
test("rng vectors pin same-seed same-stream", () => {
const doc = loadVectors();
const count = doc.vectors.length;
console.log("rng vectors=" + count + " same-seed same-stream pinned");
assert.ok(count >= 3, "at least 3 vectors, got " + count);
for (const v of doc.vectors) {
const made = stream(v.seed, v.next_u64.length);
assert.deepEqual(made, v.next_u64, "next_u64 matches seed " + v.seed);
const made32 = made.slice(0, v.next_u32.length).map((s) => (BigInt(s) >> 32n).toString());
assert.deepEqual(made32, v.next_u32, "next_u32 matches seed " + v.seed);
}
const a = stream(42, 8);
const b = stream(42, 8);
assert.deepEqual(a, b, "same seed gives same stream");
const s0 = stream(0, 8);
const s1 = stream(1, 8);
assert.deepEqual(s0, s1, "seeds 0 and 1 alias to state 1");
const r = doc.range_example;
let s = (BigInt(r.seed) | 1n) & MASK;
const span = BigInt(r.hi) - BigInt(r.lo);
const got = [];
for (let i = 0; i < r.values.length; i++) { s = nextU64Word(s); got.push((BigInt(r.lo) + (s % span)).toString()); }
assert.deepEqual(got, r.values, "next_range matches");
});
