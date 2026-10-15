import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

// Gap test: tools/drift-check.mjs catch blocks stay loud.
// EXPECT-SILENT 0: no empty catch sites remain; loud sites return,
// record, or say() instead of swallowing.

test("drift-check catch blocks pin: silent count unchanged (lead lands fix)", () => {
  const src = readFileSync(fileURLToPath(new URL("./drift-check.mjs", import.meta.url)), "utf8");
  const blocks = [...src.matchAll(/catch\s*(\([^)]*\))?\s*\{([^}]*)\}/g)];
  const total = blocks.length;
  // Loud: body returns, records, or reports. Silent: empty body.
  const silent = blocks.filter((m) => m[2].trim() === "");
  const loud = total - silent.length;
  const EXPECT_SILENT = 0;
  console.log(`catch sites: total=${total} loud=${loud} silent=${silent.length} EXPECT-SILENT ${EXPECT_SILENT}`);
  assert.equal(
    silent.length,
    EXPECT_SILENT,
    `gap moved: ${silent.length} silent (EXPECT-SILENT ${EXPECT_SILENT}); fix landed or new silent catch added`,
  );
});
