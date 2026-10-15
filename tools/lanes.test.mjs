// Split from tools/lanes.test.mjs (22 tests, 94s) into warm/cold halves.
// See tools/lanes-warm.test.mjs (10 tests) + tools/lanes-cold.test.mjs (9 tests).
// `create` is retired (owner 2026-10-10): the warm half asserts the refusal; stray lanes are built by hand.
// This file keeps 0 tests to avoid double runs.
