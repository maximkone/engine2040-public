// Run: node --test tools/doc-pin.test.mjs
// SW10: a doc that copies a source file's contract carries that file's hash,
// so a source edit without the doc edit fails and names both files.
import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { DOC_PINS, DOC_PIN_HELP, PIN_HEX_CHARS, checkPin, checkPins, findPin, hasHelpFlag, marker, pinHash, writePin, writePins } from "./doc-pin.mjs";

const TOOLS = dirname(fileURLToPath(import.meta.url));
const TOOL = join(TOOLS, "doc-pin.mjs");
const REPO = join(TOOLS, "..");
const REG = [{ docPath: "docs/book/for-ai.md", sourcePath: "crates/endpoint/src/resources.rs" }];

// A tree with one page that copies one source file, pinned at a wrong hash.
// The page and source are the live registry's paths: the CLI reads DOC_PINS.
function fixture() {
  const entry = DOC_PINS[0];
  const root = mkdtempSync(join(tmpdir(), "doc-pin-"));
  mkdirSync(join(root, dirname(entry.docPath)), { recursive: true });
  mkdirSync(join(root, dirname(entry.sourcePath)), { recursive: true });
  writeFileSync(join(root, entry.docPath), [
    "# page",
    "",
    "| Verb | Use it to |",
    "|---|---|",
    "| `scene.read` | get the scene revision |",
    "",
    `<!-- doc-pin: ${entry.sourcePath} sha256-8:0000000000000000 -->`,
    "",
  ].join("\n"));
  writeFileSync(join(root, entry.sourcePath), "pub const TOOLS: u8 = 1;\n");
  return root;
}

const run = (args) => {
  const r = spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8" });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

test("pinHash is the first 8 bytes of sha256 over CRLF-normalised text", () => {
  assert.equal(pinHash("a\r\nb\r\n"), pinHash("a\nb\n"), "a CRLF checkout pins like an LF one");
  assert.equal(pinHash("a\nb\n").length, PIN_HEX_CHARS);
  assert.match(pinHash("a\nb\n"), /^[0-9a-f]{16}$/);
  assert.notEqual(pinHash("a\nb\n"), pinHash("a\nc\n"), "one changed byte moves the pin");
});

test("a source edit without the doc edit exits 1 and names both files", () => {
  const root = fixture();
  try {
    // The placeholder pin does not match the source: the gate refuses.
    const stale = run(["--root", root]);
    assert.equal(stale.status, 1, stale.out);
    assert.match(stale.out, /docs\/book\/for-ai\.md/, "the refusal names the doc");
    assert.match(stale.out, /crates\/endpoint\/src\/resources\.rs/, "the refusal names the source");
    assert.match(stale.out, /RESULT FAIL: 1 doc pin\(s\) stale/);
    assert.match(stale.out, /--write/, "the refusal names the way to bless it");

    // --write moves the pin to the source's hash, and the gate is green.
    const wrote = run(["--root", root, "--write"]);
    assert.equal(wrote.status, 0, wrote.out);
    assert.match(wrote.out, /WROTE 1 doc pin\(s\): docs\/book\/for-ai\.md/);
    assert.match(readFileSync(join(root, "docs", "book", "for-ai.md"), "utf8"), new RegExp(`sha256-8:${pinHash("pub const TOOLS: u8 = 1;\n")}`));
    const green = run(["--root", root]);
    assert.equal(green.status, 0, green.out);
    assert.match(green.out, /RESULT PASS: 1 doc pin\(s\) match their source/);

    // Now the page is current, edit only the source: stale again, both hashes named.
    writeFileSync(join(root, "crates", "endpoint", "src", "resources.rs"), "pub const TOOLS: u8 = 2;\n");
    const again = run(["--root", root]);
    assert.equal(again.status, 1, again.out);
    assert.match(again.out, new RegExp(pinHash("pub const TOOLS: u8 = 1;\n")));
    assert.match(again.out, new RegExp(pinHash("pub const TOOLS: u8 = 2;\n")));
    assert.match(again.out, /the source changed and the page that copies it did not/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a CRLF page and a CRLF source pin the same as LF", () => {
  const lfSource = "pub const A: u8 = 1;\n";
  const lfDoc = writePin("# page\n", REG[0].sourcePath, pinHash(lfSource));
  const crlfDoc = writePin("# page\r\n", REG[0].sourcePath, pinHash(lfSource)).replace(/\n/g, "\r\n");
  const crlfSource = lfSource.replace(/\n/g, "\r\n");
  assert.equal(checkPin(REG[0], lfDoc, lfSource), null, "the LF pair matches");
  assert.equal(checkPin(REG[0], crlfDoc, crlfSource), null, "the CRLF pair matches");
  assert.equal(checkPin(REG[0], crlfDoc, lfSource), null, "a CRLF doc with an LF source also matches");
  assert.equal(findPin(crlfDoc, REG[0].sourcePath).line, 3, "the marker line is read on a CRLF page");
});

test("a page with no marker fails, and --write appends one", () => {
  const entry = REG[0];
  const source = "pub const A: u8 = 1;\n";
  const bare = "# page\r\n\r\nno pin here\r\n";
  assert.equal(findPin(bare, entry.sourcePath), null);
  const finding = checkPin(entry, bare, source);
  assert.match(finding, /no doc-pin marker/);
  assert.match(finding, /docs\/book\/for-ai\.md/);
  const written = writePin(bare, entry.sourcePath, pinHash(source));
  assert.match(written, new RegExp(marker(entry.sourcePath, pinHash(source)).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.ok(written.includes("\r\n"), "the page keeps its own line endings");
  assert.equal(checkPin(entry, written, source), null);
  assert.equal(writePin(written, entry.sourcePath, pinHash(source)), written, "a second write is a no-op");
});

test("a registry row whose doc or source is gone fails, and writePins skips it", () => {
  const root = mkdtempSync(join(tmpdir(), "doc-pin-gone-"));
  try {
    assert.equal(checkPins(root, REG).length, 1, "a missing doc is a finding");
    assert.match(checkPins(root, REG)[0], /docs\/book\/for-ai\.md: missing/);
    mkdirSync(join(root, "docs", "book"), { recursive: true });
    writeFileSync(join(root, "docs", "book", "for-ai.md"), "# page\n");
    const findings = checkPins(root, REG);
    assert.equal(findings.length, 1);
    assert.match(findings[0], /crates\/endpoint\/src\/resources\.rs: missing/);
    assert.deepEqual(writePins(root, REG), [], "writePins writes nothing when a side is gone");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("--help exits 0 and an unknown argument is a usage error", () => {
  assert.equal(hasHelpFlag(["--help"]), true);
  assert.equal(hasHelpFlag([]), false);
  assert.match(DOC_PIN_HELP, /Usage:/);
  const help = run(["--help"]);
  assert.equal(help.status, 0, help.out);
  assert.match(help.out, /doc-pin\.mjs/);
  assert.equal(run(["--nope"]).status, 2);
  assert.equal(run(["--root"]).status, 2, "--root without a directory is a usage error");
});

// The live registry: the door page is pinned to the resource catalog. The hash
// is checked against the COMMITTED source (`git show HEAD:<path>`), not the
// worktree: a neighbouring AI's uncommitted edit is theirs to re-pin, and
// tools/workspace-check.mjs reports the worktree's pin in the full audit.
test("the live registry pins the door page to the committed resource catalog", () => {
  assert.ok(DOC_PINS.length >= 1, "the registry holds the door page");
  const door = DOC_PINS.find((e) => e.docPath === "docs/book/for-ai.md" && e.sourcePath === "crates/endpoint/src/resources.rs");
  assert.ok(door, "the door page is pinned to the resource catalog");
  for (const entry of DOC_PINS) {
    assert.ok(existsSync(join(REPO, entry.docPath)), `${entry.docPath} exists`);
    assert.ok(existsSync(join(REPO, entry.sourcePath)), `${entry.sourcePath} exists`);
    assert.ok(entry.what.trim().length > 10, `${entry.docPath} says what it copies`);
    const docText = readFileSync(join(REPO, entry.docPath), "utf8");
    assert.ok(findPin(docText, entry.sourcePath), `${entry.docPath} carries a marker for ${entry.sourcePath}`);
    const head = spawnSync("git", ["-C", REPO, "show", `HEAD:${entry.sourcePath}`], { encoding: "utf8" });
    assert.equal(head.status, 0, `git show HEAD:${entry.sourcePath} must work`);
    assert.equal(checkPin(entry, docText, head.stdout), null, `${entry.docPath} matches the committed ${entry.sourcePath}`);
  }
});
