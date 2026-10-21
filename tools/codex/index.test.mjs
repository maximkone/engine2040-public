// Run: node --test tools/codex/index.test.mjs
// The Rome codex index gate (CDX2): the passage cutter, the topic tags and
// the numbers with units, on a fixture of Vegetius 3.8 and Caesar BG 4.17.
// The fixture is the shape the fetched texts have: "THE WAR IN GAUL" and
// "BOOK IV" head the Caesar text, "BOOK III" and "CHAPTER VIII" head the
// Vegetius text. The Vegetius camp passage must carry the `camp` tag and
// the measures `5 ft` and `3 ft`; the Caesar passage must carry the ref
// `caesar.bg.4.17`. A bad tag, a lost measure, a wrong ref or a broken
// jsonl line fails the gate.
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  cutTei,
  cutText,
  findNumbers,
  htmlText,
  indexText,
  refFor,
  romanToInt,
  run,
  tagTopics,
  TOPICS,
  WORKS,
} from "./index.mjs";

const VEGETIUS = `THE LIFE OF THE ROMAN SOLDIER

BOOK III

CHAPTER VIII

The camp of a marching army is set out in a square. The ditch which
surrounds it is 5 ft wide and 3 ft deep. The rampart is raised from the
turf the digging has thrown up, and the whole work is done by the men of
the camp before the sun is high.
`;

const CAESAR = `THE COMMENTARIES OF
CAIUS JULIUS CAESAR

THE WAR IN GAUL

BOOK IV

CHAPTER XVII

The march of the army was led to the town of Genava, and the bridge over
the river was built by the soldiers in three days, with the timber of the
neighbouring town.
`;

const TEI = `<TEI xmlns="http://www.tei-c.org/ns/1.0">
<body><div n="1" type="textpart" subtype="book"><head>BOOK I</head>
<div n="1" type="textpart" subtype="chapter"><head>CHAPTER I</head><p>The first chapter.</p></div>
<div n="2" type="textpart" subtype="chapter"><head>CHAPTER II</head><p>The second chapter, which holds a ditch 5 ft wide.</p></div>
</div></body></TEI>`;

const SMITHS_PONS = `<!-- a comment that says 99 ft must not count -->
<html><head><title>Pons</title><script>var n = 7;</script></head>
<body>
<table class="header"><tr><td>Home</td></tr></table>
<p class="start2 justify">PONS, a bridge. The most ancient bridge upon record is the one
erected over the Euphrates at Babylon, of stone piers with a framework of planking.</p>
<p class="pagenum">936</p>
<div class="help_bar"><p class="m1 justify">A page on this site is in the public domain only
if its url carries one asterisk.</p></div>
</body></html>`;

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "codex-index-"));
  mkdirSync(join(root, "gutenberg"), { recursive: true });
  mkdirSync(join(root, "perseus"), { recursive: true });
  writeFileSync(
    join(root, "gutenberg", "vegetius-de-re-militari.txt"),
    VEGETIUS,
    "utf8",
  );
  writeFileSync(join(root, "gutenberg", "caesar-bellum-gallicum.txt"), CAESAR, "utf8");
  writeFileSync(join(root, "perseus", "phi0448.phi001.perseus-eng2.xml"), TEI, "utf8");
  return root;
}

test("the fixture gives the refs vegetius.3.8 and caesar.bg.4.17", () => {
  const vegetius = indexText({
    text: VEGETIUS,
    entry: WORKS["gutenberg/vegetius-de-re-militari.txt"],
    source: "gutenberg",
    file: "vegetius-de-re-militari.txt",
  });
  const caesar = indexText({
    text: CAESAR,
    entry: WORKS["gutenberg/caesar-bellum-gallicum.txt"],
    source: "gutenberg",
    file: "caesar-bellum-gallicum.txt",
  });
  const refs = [...vegetius, ...caesar].map((r) => r.ref);
  assert.ok(refs.includes("vegetius.3.8"), `refs were ${refs.join(", ")}`);
  assert.ok(refs.includes("caesar.bg.4.17"), `refs were ${refs.join(", ")}`);
});

test("the camp passage tags camp and holds 5 ft and 3 ft", () => {
  const made = indexText({
    text: VEGETIUS,
    entry: WORKS["gutenberg/vegetius-de-re-militari.txt"],
    source: "gutenberg",
    file: "vegetius-de-re-militari.txt",
  });
  const camp = made.find((r) => r.ref === "vegetius.3.8");
  assert.ok(camp, "the camp passage is missing");
  assert.ok(camp.topics.includes("camp"), `topics were ${camp.topics.join(", ")}`);
  const texts = camp.numbers.map((n) => n.text);
  assert.ok(texts.includes("5 ft"), `measures were ${texts.join(", ")}`);
  assert.ok(texts.includes("3 ft"), `measures were ${texts.join(", ")}`);
  const five = camp.numbers.find((n) => n.text === "5 ft");
  assert.equal(five.value, 5);
  assert.equal(five.unit, "ft");
});

test("every tag is one of the nine topics", () => {
  const topics = tagTopics(`${VEGETIUS}\n${CAESAR}`);
  for (const topic of topics) assert.ok(TOPICS.includes(topic), `unknown topic ${topic}`);
  assert.ok(topics.includes("bridge"), `topics were ${topics.join(", ")}`);
  assert.ok(topics.includes("place"), `topics were ${topics.join(", ")}`);
});

test("a measure is kept only with a unit", () => {
  const numbers = findNumbers("the ditch is 5 ft wide, three days of work, 12 men, 43 BC");
  const byText = new Map(numbers.map((n) => [n.text, n]));
  assert.ok(byText.has("5 ft"));
  assert.ok(byText.has("12 men"));
  assert.ok(byText.has("43 BC"));
  assert.ok(!numbers.some((n) => n.text === "three days"), "a count without a unit was kept");
  assert.equal(byText.get("43 BC").unit, "year");
});

test("a latin measure comes out too", () => {
  const numbers = findNumbers("fossa quinque pedes lata");
  assert.deepEqual(
    numbers.map((n) => `${n.value} ${n.unit}`),
    ["5 ft"],
  );
});

test("a roman numeral is a number", () => {
  assert.equal(romanToInt("XVII"), 17);
  assert.equal(romanToInt("IV"), 4);
  assert.equal(romanToInt("nonsense"), 0);
  assert.equal(refFor("caesar.bg", 4, 17, 0), "caesar.bg.4.17");
  assert.equal(refFor("vegetius", 3, 8, 2), "vegetius.3.8.2");
});

test("the tei file is cut by its own book and chapter numbers", () => {
  const passages = cutTei(TEI);
  assert.deepEqual(
    passages.map((p) => `${p.book}.${p.chapter}`),
    ["1.1", "1.2"],
  );
  assert.ok(passages[1].text.includes("5 ft"), "the tei text lost its words");
});

test("a plain text without a heading still makes a passage", () => {
  const passages = cutText("just one block of prose", { work: "x" });
  assert.equal(passages.length, 1);
  assert.equal(passages[0].text, "just one block of prose");
});

test("an html entry keeps the article and drops the page furniture", () => {
  const body = htmlText(SMITHS_PONS);
  assert.ok(body.includes("a bridge"), `body was ${body}`);
  assert.ok(!body.includes("asterisk"), `body was ${body}`);
  assert.ok(!body.includes("99 ft"), `body was ${body}`);
  assert.ok(!body.includes("Home"), `body was ${body}`);
});

test("a one page entry is the work itself", () => {
  const made = indexText({
    text: SMITHS_PONS,
    entry: WORKS["smiths/pons.html"],
    source: "smiths",
    file: "pons.html",
  });
  assert.equal(made.length, 1);
  assert.equal(made[0].ref, "smiths.pons");
  assert.ok(made[0].topics.includes("bridge"), `topics were ${made[0].topics.join(", ")}`);
  assert.ok(!made[0].numbers.some((n) => n.text === "99 ft"), "a comment gave a measure");
});

test("the index is written as jsonl and every line parses", () => {
  const root = fixture();
  try {
    const outFile = join(root, "index.jsonl");
    const made = run({ inDir: root, outFile, log: () => {} });
    assert.equal(made.files, 3);
    assert.ok(made.records.length >= 4, `records were ${made.records.length}`);
    const lines = readFileSync(outFile, "utf8").trim().split("\n");
    assert.equal(lines.length, made.records.length);
    const parsed = lines.map((line) => JSON.parse(line));
    const refs = parsed.map((r) => r.ref);
    assert.ok(refs.includes("vegetius.3.8"), `refs were ${refs.join(", ")}`);
    assert.ok(refs.includes("caesar.bg.4.17"), `refs were ${refs.join(", ")}`);
    assert.ok(refs.includes("caesar.bg.1.2"), `refs were ${refs.join(", ")}`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a file with no work entry is indexed under its source id and said", () => {
  const root = fixture();
  try {
    writeFileSync(join(root, "gutenberg", "unknown-work.txt"), "BOOK I\n\nsome text\n", "utf8");
    const lines = [];
    const made = run({ inDir: root, outFile: join(root, "index.jsonl"), log: (l) => lines.push(l) });
    assert.ok(
      lines.some((l) => l.includes("unknown-work.txt")),
      `the log was ${lines.join(" | ")}`,
    );
    const refs = made.records.map((r) => r.ref);
    assert.ok(refs.includes("gutenberg.1"), `refs were ${refs.join(", ")}`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
