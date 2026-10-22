// Run: node --test tools/license-gate-py.test.mjs
// The generator licence gate of content/ai-loop has its own python tests
// (content/ai-loop/test_license_gate.py). This mate runs them, so the row
// that owns the gate proves it green in CI instead of by hand.
//
// The gate's job, restated here so the test says it in one place: a
// generator whose licence is not on the approved list is refused BY NAME,
// and an approved one passes. The python suite covers every case; this mate
// checks the suite runs and passes, and then probes the gate itself, so a
// green suite cannot mean a gate that stopped refusing anything.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
const AI_LOOP = join(ROOT, "content", "ai-loop");
const SUITE = join(AI_LOOP, "test_license_gate.py");
const GATE = join(AI_LOOP, "license_gate.py");

// The python interpreter, in the order the CI image offers it.
function pythonBin() {
  for (const name of ["python", "python3"]) {
    const probe = spawnSync(name, ["--version"], { encoding: "utf8", windowsHide: true });
    if (!probe.error && probe.status === 0) return name;
  }
  return null;
}

const PY = pythonBin();

// Run one python program and read both streams: unittest reports on stderr.
function runPython(args) {
  const result = spawnSync(PY, args, { cwd: AI_LOOP, encoding: "utf8", windowsHide: true });
  return {
    status: result.status,
    out: result.stdout ?? "",
    err: result.stderr ?? "",
    report: `${result.stdout ?? ""}\n${result.stderr ?? ""}`,
  };
}

test("the licence gate and its python suite are in the tree", () => {
  assert.ok(existsSync(GATE), "content/ai-loop/license_gate.py is tracked");
  assert.ok(existsSync(SUITE), "content/ai-loop/test_license_gate.py is tracked");
});

test(
  "the generator licence gate suite is green",
  { skip: PY ? false : "no python interpreter on this machine" },
  () => {
    const { status, report } = runPython([SUITE]);
    assert.equal(status, 0, `the suite must pass, got:\n${report}`);
    assert.match(report, /^OK$/m, `the suite must report OK, got:\n${report}`);
    const ran = /Ran (\d+) tests?/.exec(report);
    assert.ok(ran, `the suite must print its test count, got:\n${report}`);
    const count = Number(ran[1]);
    assert.ok(count >= 10, `the suite must run at least ten tests, ran ${count}`);
  }
);

test(
  "the gate refuses an unapproved licence by name and passes an approved one",
  { skip: PY ? false : "no python interpreter on this machine" },
  () => {
    // A one-liner against the gate itself, so the proof is about the gate
    // and not only about the suite that tests it. One case per refusal code
    // the gate promises, plus one approved licence that passes.
    const probe = `
import sys
sys.path.insert(0, ${JSON.stringify(AI_LOOP)})
from license_gate import LicenseRefusal, check_generator

def refuses(record, code):
    try:
        check_generator(record)
    except LicenseRefusal as exc:
        if exc.code != code:
            sys.exit("expected " + code + ", got " + exc.code)
        return
    sys.exit("the gate let a record through: " + repr(record))

approved = check_generator(
    {"name": "g", "license": "CC0-1.0", "license_source": "row"}
)
print("approved", approved)
refuses({}, "UNKNOWN_GENERATOR")
refuses({"name": "g", "license": "CC0-1.0"}, "UNREADABLE_LICENSE")
refuses({"name": "g", "license": "CC-BY-NC-4.0", "license_source": "row"}, "FORBIDDEN_LICENSE")
refuses({"name": "g", "license": "GPL-3.0", "license_source": "row"}, "INFECTING_LICENSE")
print("refused all four codes by name")
`;
    const { status, out, report } = runPython(["-c", probe]);
    assert.equal(status, 0, `the probe must pass, got:\n${report}`);
    assert.match(out, /approved CC0-1\.0/, `an approved licence passes, got:\n${out}`);
    assert.match(
      out,
      /refused all four codes by name/,
      `every refusal code is named, got:\n${out}`
    );
  }
);
