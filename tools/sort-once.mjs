// Sorted-unique helper for LAYER 3 hottest loop.
// Single owner for sort+dedup: sort a copy once, then drop neighbours
// in one linear pass. No Set, no second scan, input never mutated.
// Units: numbers in, sorted unique numbers out. Deterministic: same
// input order gives same output; numeric compare only.
export function naiveSortUnique(input) {
  const sorted = [...input].sort((a, b) => a - b);
  return [...new Set(sorted)];
}
export function sortOnce(input) {
  const sorted = [...input].sort((a, b) => a - b);
  const out = [];
  for (let i = 0; i < sorted.length; i += 1) {
    if (i === 0 || sorted[i] !== sorted[i - 1]) {
      out.push(sorted[i]);
    }
  }
  return out;
}
// Deterministic 10k fixture: LCG over a wide range so duplicates are
// rare and the sort cost is like-for-like for both paths.
export function makeFixture(size = 10000, seed = 2040) {
  const out = new Array(size);
  let state = seed >>> 0;
  for (let i = 0; i < size; i += 1) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    out[i] = state % 1000000;
  }
  return out;
}
// Paired benchmark lives in bench-compare.mjs --sort-demo (shared benchFns
// loop); this file owns only the sort+dedup functions and the fixture.
