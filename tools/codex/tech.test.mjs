// Run: node --test tools/codex/tech.test.mjs
// The Rome codex tech draft gate (CDX4): the passages of the codex index
// become draft tech tables. The fixture is the Vegetius 3.8 text the index
// gate uses, cut with the index tool itself, so the camp draft must hold the
// 5 ft and the 3 ft ditch with the ref `vegetius.3.8`. Nothing may write a
// file that is not a draft, a number without a unit is dropped, and a
// passage with no ref or no topic a table wants is skipped.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { indexText, WORKS } from "./index.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "tech.mjs");
const REPO = resolve(HERE, "..", "..");

// The Vegetius 3.8 fixture, the same text the index gate uses: the camp
// passage with the ditch 5 ft wide and 3 ft deep.
const VEGETIUS = `THE LIFE OF THE ROMAN SOLDIER

BOOK III

CHAPTER VIII

The camp of a marching army is set out in a square. The ditch which
surrounds it is 5 ft wide and 3 ft deep. The rampart is raised from the
turf the digging has thrown up, and the whole work is done by the men of
the camp before the sun is high.
`;

function tmp() {
  return mkdtempSync(join(tmpdir(), "codex-tech-"));
}

// The index records of the fixture, cut with the index tool.
function vegetiusRecords() {
  return indexText({
    text: VEGETIUS,
    entry: WORKS["gutenberg/vegetius-de-re-militari.txt"],
    source: "gutenberg",
    file: "vegetius-de-re-militari.txt",
    url: "https://www.gutenberg.org/ebooks/vegetius.txt",
    licence: "US public domain",
  });
}

// The index file: one json line per passage.
function writeIndex(dir, records) {
  const file = join(dir, "index.jsonl");
  writeFileSync(file, records.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
  return file;
}

// The tool runs as a child process: the gate covers the CLI a person uses.
function cli(args) {
  return new Promise((done) => {
    const child = spawn("node", [TOOL, ...args], { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("close", (status) => done({ status, stdout, stderr }));
  });
}

// The paths of the repo with their size and mtime, so the gate can prove the
// tool wrote nothing outside its own out dir.
function snapshot(root) {
  const out = new Map();
  const skip = new Set([".git", "target", "node_modules"]);
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (skip.has(entry.name)) continue;
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      const st = statSync(full);
      out.set(relative(root, full), `${st.size}:${st.mtimeMs}`);
    }
  };
  walk(root);
  return out;
}

// The tables of a draft file: the header keys and the measure tables.
function parseDraft(text) {
  const header = {};
  const tables = new Map();
  let current = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const head = /^\[([^\]]+)\]$/.exec(line);
    if (head) {
      current = head[1];
      tables.set(current, {});
      continue;
    }
    const pair = /^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+)$/.exec(line);
    if (!pair) continue;
    const value = pair[2].startsWith('"') ? pair[2].slice(1, -1) : pair[2];
    if (current) tables.get(current)[pair[1]] = value;
    else header[pair[1]] = value;
  }
  return { header, tables };
}

test("the vegetius 3.8 fixture yields a camp draft with the 5 ft and 3 ft ditch", async () => {
  const dir = tmp();
  try {
    const index = writeIndex(dir, vegetiusRecords());
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const draft = readFileSync(join(out, "camp.draft.toml"), "utf8");
    const { tables } = parseDraft(draft);
    const measures = [...tables.values()].filter((t) => t.ref === "vegetius.3.8");
    assert.equal(measures.length, 2, `the draft holds ${measures.length} measure(s)`);
    const values = measures.map((m) => `${m.value} ${m.unit}`).sort();
    assert.deepEqual(values, ["3 ft", "5 ft"]);
    const labels = measures.map((m) => m.label).sort();
    assert.deepEqual(labels, ["ditch depth", "ditch width"]);
    for (const measure of measures) assert.equal(measure.ref, "vegetius.3.8");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the tool names the passage, the measure and the thing it gives", async () => {
  const dir = tmp();
  try {
    const index = writeIndex(dir, vegetiusRecords());
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const lines = r.stdout.trim().split("\n");
    assert.ok(lines.includes("camp: vegetius.3.8 5 ft ditch width"), `the lines were ${lines.join(" | ")}`);
    assert.ok(lines.includes("camp: vegetius.3.8 3 ft ditch depth"), `the lines were ${lines.join(" | ")}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("nothing writes a non-draft file and no other path changes", async () => {
  const dir = tmp();
  try {
    const index = writeIndex(dir, vegetiusRecords());
    const out = join(dir, "tech");
    const before = snapshot(REPO);
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const files = readdirSync(out);
    assert.ok(files.length > 0, "the tool wrote no file");
    for (const name of files) {
      assert.ok(name.endsWith(".draft.toml"), `${name} is no draft file`);
    }
    const after = snapshot(REPO);
    assert.deepEqual([...after.keys()].sort(), [...before.keys()].sort());
    for (const [path, stamp] of before) assert.equal(after.get(path), stamp, `${path} changed`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a measure without a unit is dropped", async () => {
  const dir = tmp();
  try {
    const records = vegetiusRecords();
    records.push({
      ref: "nowhere.1.1",
      source: "gutenberg",
      file: "nowhere.txt",
      work: "nowhere",
      book: 1,
      chapter: 1,
      section: 0,
      title: "Nowhere",
      author: "Nobody",
      url: "",
      licence: "",
      topics: ["camp"],
      numbers: [{ text: "7", value: 7 }],
      chars: 20,
      text: "the camp holds 7 without a unit",
    });
    const index = writeIndex(dir, records);
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const draft = readFileSync(join(out, "camp.draft.toml"), "utf8");
    assert.ok(!draft.includes("value = 7"), "a number without a unit reached the draft");
    assert.ok(!draft.includes("nowhere"), "a measure without a unit reached the draft");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a passage with no topic a table wants is skipped", async () => {
  const dir = tmp();
  try {
    const records = vegetiusRecords();
    records.push({
      ref: "only.year.1",
      source: "gutenberg",
      file: "year.txt",
      work: "only.year",
      book: 1,
      chapter: 0,
      section: 0,
      title: "A year",
      author: "Nobody",
      url: "",
      licence: "",
      topics: ["year", "place"],
      numbers: [{ text: "43 BC", value: 43, unit: "year" }],
      chars: 20,
      text: "the year 43 BC is no tech table",
    });
    const index = writeIndex(dir, records);
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const files = readdirSync(out);
    for (const name of files) {
      const draft = readFileSync(join(out, name), "utf8");
      assert.ok(!draft.includes("value = 43"), "a passage with no tech topic reached the draft");
      assert.ok(!draft.includes("only.year"), "a passage with no tech topic reached the draft");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a passage with no ref is skipped", async () => {
  const dir = tmp();
  try {
    const records = vegetiusRecords();
    records.push({
      ref: "",
      source: "gutenberg",
      file: "noref.txt",
      work: "noref",
      book: 1,
      chapter: 1,
      section: 0,
      title: "No ref",
      author: "Nobody",
      url: "",
      licence: "",
      topics: ["camp"],
      numbers: [{ text: "42 ft", value: 42, unit: "ft" }],
      chars: 20,
      text: "the ditch is 42 ft wide",
    });
    const index = writeIndex(dir, records);
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const draft = readFileSync(join(out, "camp.draft.toml"), "utf8");
    assert.ok(!draft.includes("value = 42"), "a passage with no ref reached the draft");
    assert.ok(!draft.includes("[]"), "a passage with no ref made a broken table");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a record without the numbers field is measured from its text", async () => {
  const dir = tmp();
  try {
    const records = vegetiusRecords();
    const record = records.find((r) => r.ref === "vegetius.3.8");
    delete record.numbers;
    const index = writeIndex(dir, records);
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const draft = readFileSync(join(out, "camp.draft.toml"), "utf8");
    assert.ok(draft.includes("value = 5"), "the text was not measured again");
    assert.ok(draft.includes("value = 3"), "the text was not measured again");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the draft marks itself a draft and carries the ref and the source", async () => {
  const dir = tmp();
  try {
    const index = writeIndex(dir, vegetiusRecords());
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out]);
    assert.equal(r.status, 0, r.stderr);
    const draft = readFileSync(join(out, "camp.draft.toml"), "utf8");
    assert.ok(draft.startsWith("#"), "the draft has no header comment");
    const { header, tables } = parseDraft(draft);
    assert.equal(header.draft, "true");
    assert.equal(header.table, "camp");
    const measure = [...tables.entries()].find(([, t]) => t.ref === "vegetius.3.8");
    assert.ok(measure, "the draft holds no measure of the passage");
    const [, fields] = measure;
    assert.equal(fields.url, "https://www.gutenberg.org/ebooks/vegetius.txt");
    assert.equal(fields.licence, "US public domain");
    assert.equal(fields.work, "vegetius");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a dry run prints the plan and writes nothing", async () => {
  const dir = tmp();
  try {
    const index = writeIndex(dir, vegetiusRecords());
    const out = join(dir, "tech");
    const r = await cli(["--index", index, "--out", out, "--dry-run"]);
    assert.equal(r.status, 0, r.stderr);
    assert.ok(r.stdout.includes("would write"), `the lines were ${r.stdout}`);
    assert.ok(r.stdout.includes("camp: vegetius.3.8 5 ft ditch width"), `the lines were ${r.stdout}`);
    assert.ok(!existsSync(out), "the dry run wrote a file");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing index fails, a bad flag is bad use and the help names the flags", async () => {
  const dir = tmp();
  try {
    const missing = await cli(["--index", join(dir, "nothing.jsonl"), "--out", join(dir, "tech")]);
    assert.equal(missing.status, 1);
    const bad = await cli(["--nonsense"]);
    assert.equal(bad.status, 2);
    assert.match(bad.stderr, /unknown argument/);
    const noValue = await cli(["--index"]);
    assert.equal(noValue.status, 2);
    const help = await cli(["--help"]);
    assert.equal(help.status, 0);
    assert.match(help.stdout, /--index/);
    assert.match(help.stdout, /--out/);
    assert.match(help.stdout, /--dry-run/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
