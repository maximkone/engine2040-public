// VISION.md promise guard: froxel volumetrics and knapping have
// 0 hits in crates/, the denoiser is one comment in
// crates/render/src/tier.rs, and GPU-driven virtual geometry has only the
// CPU meshlet build in crates/assets/src/meshlet.rs (row VG2), so the
// product bullet must park each with a dated note stating the true count,
// never promise them as working today.
// Fails on revert: restoring the old "with an engine-owned denoiser ... /
// GPU-driven virtual geometry (today 0 hits" bullet fails the look and
// virtual-geometry assertions; dropping the 2026-10-09 knapping or
// volumetrics parked notes fails the new assertions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VISION = join(dirname(fileURLToPath(import.meta.url)), "VISION.md");
const text = readFileSync(VISION, "utf8").replace(/\r\n/g, "\n");

test("unimplemented look promises stay parked, not promised", () => {
  assert.ok(
    text.includes("Parked until rows land it:"),
    "parked note missing from the 2040-look bullet",
  );
});

test("old unqualified denoiser promise stays cut", () => {
  assert.ok(
    !text.includes("with an engine-owned denoiser"),
    "unqualified engine-owned denoiser promise is back",
  );
});

test("old unqualified denoiser promise stays cut with true count", () => {
  assert.ok(
    text.includes("one comment in `crates/render/src/tier.rs`"),
    "denoiser parked note must state the true one-comment count",
  );
});

test("virtual geometry states its dated landed count, not 0 hits", () => {
  assert.ok(
    text.includes(
      "GPU-driven virtual geometry (2026-10-12: CPU meshlet build landed in",
    ),
    "virtual-geometry note must state the dated landed meshlet build",
  );
  assert.ok(
    text.includes("`crates/assets/src/meshlet.rs` under row VG2"),
    "virtual-geometry note must name the landed file and its row",
  );
  assert.ok(
    !text.includes("virtual geometry (today 0 hits in `crates/`"),
    "stale virtual-geometry 0-hits claim is back",
  );
});

test("knapping promise stays parked with date and true count", () => {
  assert.ok(
    text.includes("knapping (parked 2026-10-09: today 0 hits in `crates/`"),
    "knapping parked note must state the date and true 0-hits count",
  );
});

test("volumetrics promise stays parked with date and true count", () => {
  assert.ok(
    text.includes(
      "froxel volumetrics (parked 2026-10-09: today 0 hits in `crates/`",
    ),
    "volumetrics parked note must state the date and true 0-hits count",
  );
  assert.ok(
    text.includes("rows VX1-VX2 open"),
    "volumetrics parked note must point at the open VX rows",
  );
});
