import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chain, tape, compare } from "./tape-checksum.mjs";

// Vectors mirror crates/core/src/hash.rs fnv_matches_published_vectors.
describe("tape-checksum FNV chain", () => {
  it("foobar chains to 85944171f73967e8", () => {
    assert.equal(chain(null, Buffer.from("foobar", "utf8")), "85944171f73967e8");
    assert.equal(tape(["foobar"])[0], "85944171f73967e8");
  });

  it("a chains to af63dc4c8601ec8c", () => {
    assert.equal(chain(null, Buffer.from("a", "utf8")), "af63dc4c8601ec8c");
    assert.equal(tape(["a"])[0], "af63dc4c8601ec8c");
  });

  it("two temp tapes differing at index 7 report first-divergence 7 with both hexes", () => {
    const base = Array.from({ length: 10 }, (_, i) => `tick-${i}`);
    const aTicks = [...base];
    const bTicks = [...base];
    bTicks[7] = "tick-7-diverged";
    const aHex = tape(aTicks);
    const bHex = tape(bTicks);
    const r = compare(aHex, bHex);
    assert.equal(r.equal, false);
    assert.equal(r.tick, 7);
    assert.notEqual(aHex[7], bHex[7]);
    const dir = mkdtempSync(join(tmpdir(), "tape-checksum-"));
    try {
      const aPath = join(dir, "a.json");
      const bPath = join(dir, "b.json");
      writeFileSync(aPath, JSON.stringify(aTicks));
      writeFileSync(bPath, JSON.stringify(bTicks));
      const script = fileURLToPath(new URL("./tape-checksum.mjs", import.meta.url));
      const out = spawnSync(process.execPath, [script, "--tapes", aPath, bPath], { encoding: "utf8" });
      assert.equal(out.status, 0);
      assert.match(out.stdout, /first-divergence: 7/);
      assert.match(out.stdout, new RegExp(`a\\[7\\]=${aHex[7]} b\\[7\\]=${bHex[7]}`));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
