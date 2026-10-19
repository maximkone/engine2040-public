// Regenerates fixtures/scope-delta/trace.txt: the seeded room-scope walk the
// scope-view delta proof replays. The owning recipe lives in
// crates/net/src/interest.rs tests (trace_mask) and the Rust test refuses any
// drift between this file and that recipe, so a stale trace fails a test.
//
//   node fixtures/scope-delta/generate.mjs [--check]
//
// ASCII, integer only, no clock: the same bytes on every run and machine.
import { readFileSync, writeFileSync } from "node:fs";

const SEED = 2040;
const STEPS = 100;
const ROOMS = 8;
const CROSS_EVERY = 5;
const OUT = new URL("./trace.txt", import.meta.url);

/** Scope mask after `step`: three rooms from the walker's room round the ring. */
export const traceMask = (seed, step) => {
  const room = ((seed % ROOMS) + Math.floor(step / CROSS_EVERY)) % ROOMS;
  let mask = 0n;
  for (const k of [0, 1, 2]) mask |= 1n << BigInt((room + k) % ROOMS);
  return mask;
};

/** The whole fixture text, byte for byte. */
export const traceText = () => {
  const lines = [
    "# scope-delta: seeded room-scope trace, one row per snapshot step.",
    `# seed ${SEED}, ${STEPS} steps, ${ROOMS} rooms, a crossing every ${CROSS_EVERY} steps.`,
    `# rule: room(step) = (seed mod ${ROOMS} + step / ${CROSS_EVERY}) mod ${ROOMS}`,
    "#       mask(step) = room | room+1 | room+2 (mod the ring)",
    "# step<TAB>mask (16 hex digits)",
  ];
  for (let step = 0; step < STEPS; step++) {
    lines.push(`${step}\t${traceMask(SEED, step).toString(16).padStart(16, "0")}`);
  }
  return `${lines.join("\n")}\n`;
};

const text = traceText();
if (process.argv.includes("--check")) {
  const now = readFileSync(OUT, "utf8");
  if (now !== text) {
    console.error("fixtures/scope-delta/trace.txt is stale: run node fixtures/scope-delta/generate.mjs");
    process.exit(1);
  }
  console.log(`scope-delta trace.txt up to date (${STEPS} steps)`);
} else {
  writeFileSync(OUT, text);
  console.log(`wrote fixtures/scope-delta/trace.txt (${STEPS} steps)`);
}
