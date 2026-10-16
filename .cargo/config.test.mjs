import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const loadConfig = () => readFileSync(join(dir, "config.toml"), "utf8");

test("no global rustc-wrapper: every build runs rustc directly, sccache is opt-in per run", () => {
  const lines = loadConfig().split(/\r?\n/);
  const active = lines.filter((l) => /^\s*rustc-wrapper\s*=/.test(l));
  assert.equal(active.length, 0, `expected no active rustc-wrapper line (a .cmd wrapper runs through cmd.exe, 8191-char limit), found ${active.length}`);
});

test("warm rebuilds stay incremental: the config never disables it globally", () => {
  assert.ok(!/^\s*incremental\s*=\s*false/m.test(loadConfig()));
});
