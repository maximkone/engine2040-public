// Donor: mozilla/sccache (Apache-2.0) - idea only: warm compiler-cache plan
// (cold check, session-only cache, warm twice, hit-rate gate).
// All code below is written fresh for engine2040; no donor code is copied.
// SPDX-License-Identifier: Apache-2.0
// Ledger: mozilla/sccache | Apache-2.0 | warm-cache compile plan | tools/compile-cache.mjs | M

// Warm-cache compile plan: measurement only, no toolchain change.
// Run: node tools/compile-cache.mjs -> prints the plan, exit 0.

export const PLAN_LINES = [
  "compile-cache plan (sccache idea, Apache-2.0):",
  "1. record cold check time on a clean tree",
  "2. enable the cache for one session only",
  "3. run the same check twice to warm, then read hit rate",
  "4. record warm check time with a full cache",
  "5. drop the cache and repeat once to confirm the gap",
  "done-when: cold 300s -> warm 70s, hit rate >80%, two warm runs agree within 10%",
];

export function planText() {
  return `${PLAN_LINES.join("\n")}\n`;
}

export function helpText() {
  return [
    "Usage: node tools/compile-cache.mjs [--help]",
    "  prints the warm-cache compile plan (default) or this help with --help.",
    "Needs: clean tree for the cold check; same check repeated to warm.",
    "Output: plan steps plus done-when gate on stdout, exit 0.",
  ].join("\n") + "\n";
}

const invoked = process.argv[1] ?? "";
if (invoked.endsWith("compile-cache.mjs")) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) process.stdout.write(helpText());
  else process.stdout.write(planText());
}

