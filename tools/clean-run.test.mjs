import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import test from "node:test";

// clean-run.mjs is the GM3 entry point: a thin shim that forwards every argv
// to `ship.mjs run` and propagates its exit code. These tests lock that
// delegation on fast paths only (--help and a Usage fast-fail); a full run
// stages dist and plays frames, so it never runs here.
const SHIM = fileURLToPath(new URL("./clean-run.mjs", import.meta.url));
const SHIP = fileURLToPath(new URL("./ship.mjs", import.meta.url));

const run = (bin, args, { cwd } = {}) => {
  try {
    const out = execFileSync(process.execPath, [bin, ...args], {
      encoding: "utf8",
      timeout: 120_000,
      cwd: cwd ?? process.cwd(),
    });
    return { status: 0, out };
  } catch (error) {
    return { status: error.status ?? 1, out: `${error.stdout ?? ""}${error.stderr ?? ""}` };
  }
};

test("--help exits 0 and names the GM3 clean-run contract", () => {
  const r = run(SHIM, ["--help"]);
  assert.equal(r.status, 0);
  assert.match(r.out, /clean-run\.mjs/);
  assert.match(r.out, /packaged Windows build runs clean/);
});

test("--help output is byte-identical to ship.mjs run --help", () => {
  const via = run(SHIM, ["--help"]);
  const direct = run(SHIP, ["run", "--help"]);
  assert.equal(direct.status, 0, "delegate side stays green");
  assert.equal(via.status, 0);
  assert.equal(via.out, direct.out, "shim adds nothing and drops nothing");
});

test("a Usage fast-fail passes argv through with the same exit code", () => {
  const via = run(SHIM, ["--frames", "0"]);
  const direct = run(SHIP, ["run", "--frames", "0"]);
  assert.equal(direct.status, 2, "delegate rejects --frames 0 with Usage exit 2");
  assert.equal(via.status, direct.status, "shim propagates the delegate exit code");
  assert.equal(via.out, direct.out, "shim forwards argv untouched");
});

test("the shim pins its delegate in source: ship.mjs, run verb, argv forward", () => {
  const src = readFileSync(SHIM, "utf8");
  assert.match(src, /ship\.mjs/, "delegates to ship.mjs in its own dir");
  assert.match(src, /"run"/, "passes the run verb");
  assert.match(src, /process\.argv\.slice\(2\)/, "forwards every caller argv");
  assert.match(src, /process\.exit\(r\.status/, "propagates the delegate exit code");
});

test("--help works from any cwd: the delegate resolves beside the shim", () => {
  const here = run(SHIM, ["--help"]);
  const away = run(SHIM, ["--help"], { cwd: tmpdir() });
  assert.equal(away.status, 0);
  assert.equal(away.out, here.out, "cwd never changes the delegate lookup");
});
