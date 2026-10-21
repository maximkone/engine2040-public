// Run: node --test tools/codex/places.test.mjs
// The Rome codex places, roads and lexicon gate (CDX3). The fixtures are
// small and hand made: a Pleiades dump with two places in a chapter box
// and one far away, a Wikidata answer with a legion and a battle, an
// Itin-e file with one road in a box and one far away, and a codex index
// with the five passages of the orders of docs/plan/rome.md 4.5.
//
// The gate covers: a place in a box keeps its id, a place out of every
// box is dropped, a road in a box is kept, a lexicon term keeps its
// passage ref, and the same input twice gives the same bytes.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { mercatorX, mercatorY } from "./places.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "places.mjs");

function tmp() {
  return mkdtempSync(join(tmpdir(), "codex-places-"));
}

// The Pleiades fixture: the dump shape the real file has, the place
// objects at the left margin.
function writePleiades(dir) {
  const doc = [
    "{",
    '"@context": {',
    '    "title": "dcterms:title"',
    "},",
    '"@graph": [{',
    '    "id": "108896",',
    '    "title": "Col. Ulpia Traiana/Tricensimae",',
    '    "reprPoint": [',
    "        6.443715983988469,",
    "        51.66646063969803",
    "    ],",
    '    "placeTypes": [',
    '        "settlement"',
    "    ],",
    '    "uri": "https://pleiades.stoa.org/places/108896"',
    "},",
    "{",
    '    "id": "108867",',
    '    "title": "Castra Vetera",',
    '    "reprPoint": [',
    "        6.46980705,",
    "        51.644379",
    "    ],",
    '    "placeTypes": [',
    '        "fort-2"',
    "    ],",
    '    "uri": "https://pleiades.stoa.org/places/108867"',
    "},",
    "{",
    '    "id": "999999",',
    '    "title": "Far away",',
    '    "reprPoint": [',
    "        10.0,",
    "        10.0",
    "    ],",
    '    "placeTypes": [',
    '        "fort"',
    "    ],",
    '    "uri": "https://pleiades.stoa.org/places/999999"',
    "}]}",
    "",
  ].join("\n");
  writeFileSync(join(dir, "pleiades", "places.json.gz"), gzipSync(Buffer.from(doc, "utf8")));
}

// The Wikidata fixture: a sparql answer with a Roman legion at Xanten, a
// battle at Tapae and a battle of another year at the same place.
function writeWikidata(dir) {
  const answer = {
    head: { vars: ["item", "itemLabel", "kind", "place", "placeLabel", "placeCoord", "coord", "date"] },
    results: {
      bindings: [
        {
          item: { type: "uri", value: "http://www.wikidata.org/entity/Q317576" },
          itemLabel: { type: "literal", value: "Legio XVII" },
          kind: { type: "uri", value: "http://www.wikidata.org/entity/Q163323" },
          place: { type: "uri", value: "http://www.wikidata.org/entity/Q162890" },
          placeLabel: { type: "literal", value: "Xanten" },
          placeCoord: { type: "literal", value: "Point(6.453888888 51.662222222)" },
        },
        {
          item: { type: "uri", value: "http://www.wikidata.org/entity/Q2707263" },
          itemLabel: { type: "literal", value: "Third Battle of Tapae" },
          kind: { type: "uri", value: "http://www.wikidata.org/entity/Q178561" },
          coord: { type: "literal", value: "Point(22.7247 45.5056)" },
          date: { type: "literal", value: "0101-09-01T00:00:00Z" },
        },
        {
          item: { type: "uri", value: "http://www.wikidata.org/entity/Q9999999" },
          itemLabel: { type: "literal", value: "Battle of another year" },
          kind: { type: "uri", value: "http://www.wikidata.org/entity/Q178561" },
          coord: { type: "literal", value: "Point(22.7247 45.5056)" },
          date: { type: "literal", value: "1700-01-01T00:00:00Z" },
        },
      ],
    },
  };
  writeFileSync(join(dir, "wikidata", "legions-and-places.json"), `${JSON.stringify(answer, null, 2)}\n`);
}

// The Itin-e fixture: one road in the box of chapter I and one far away.
function writeItinere(dir) {
  const road = (id, name, lat, lon) =>
    `{ "type": "Feature", "properties": { "Name": "${name}", "Route_Type": 1, "Type": "Main Road", "InLine_FID": ${id} }, "geometry": { "type": "MultiLineString", "coordinates": [[[${mercatorX(lon)}, ${mercatorY(lat)}, 0], [${mercatorX(lon + 0.01)}, ${mercatorY(lat + 0.01)}, 0]]] } }`;
  const doc = [
    "{",
    '"type": "FeatureCollection",',
    '"name": "itinere_roads",',
    '"crs": { "type": "name", "properties": { "name": "urn:ogc:def:crs:EPSG::3395" } },',
    '"features": [',
    `${road(42, "Nijmegen-Xanten", 51.644, 6.469)},`,
    `${road(43, "Far away", 40.0, 10.0)}`,
    "]",
    "}",
    "",
  ].join("\n");
  writeFileSync(join(dir, "itinere", "itinere_roads.geojson"), doc);
}

// The codex index fixture: the five passages of the orders, with short
// Latin sentences that hold the terms.
function writeIndex(dir) {
  const records = [
    {
      ref: "caesar.bg.8.27",
      source: "perseus",
      file: "phi0448.phi001.perseus-lat2.xml",
      url: "https://example.invalid/phi0448.phi001.perseus-lat2.xml",
      licence: "CC BY-SA 4.0",
      text: "Itaque cum copiis contendit equitatumque tantum procedere ante agmen imperat legionum.",
    },
    {
      ref: "caesar.bg.2.11",
      source: "perseus",
      file: "phi0448.phi001.perseus-lat2.xml",
      url: "https://example.invalid/phi0448.phi001.perseus-lat2.xml",
      licence: "CC BY-SA 4.0",
      text: "Hi novissimos adorti magnam multitudinem eorum fugientium conciderunt, cum ab extremo agmine consisterent fortiterque impetum sustinerent.",
    },
    {
      ref: "caesar.bg.1.24",
      source: "perseus",
      file: "phi0448.phi001.perseus-lat2.xml",
      url: "https://example.invalid/phi0448.phi001.perseus-lat2.xml",
      licence: "CC BY-SA 4.0",
      text: "Ipse interim in colle medio triplicem aciem instruxit legionum quattuor veteranarum; in summo iugo duas legiones conlocavit.",
    },
    {
      ref: "caesar.bg.6.8",
      source: "perseus",
      file: "phi0448.phi001.perseus-lat2.xml",
      url: "https://example.invalid/phi0448.phi001.perseus-lat2.xml",
      licence: "CC BY-SA 4.0",
      text: "Celeriter nostri clamore sublato pila in hostes immittunt. Illi proximas silvas petierunt.",
    },
    {
      ref: "caesar.bg.7.85",
      source: "perseus",
      file: "phi0448.phi001.perseus-lat2.xml",
      url: "https://example.invalid/phi0448.phi001.perseus-lat2.xml",
      licence: "CC BY-SA 4.0",
      text: "Alii tela coniciunt, alii testudine facta subeunt; defatigatis in vicem integri succedunt.",
    },
  ];
  const file = join(dir, "index.jsonl");
  writeFileSync(file, `${records.map((r) => JSON.stringify(r)).join("\n")}\n`);
}

// A codex folder with the four fixtures.
function corpus() {
  const dir = tmp();
  for (const sub of ["pleiades", "wikidata", "itinere"]) {
    mkdirSync(join(dir, sub), { recursive: true });
  }
  writePleiades(dir);
  writeWikidata(dir);
  writeItinere(dir);
  writeIndex(dir);
  return dir;
}

// The tool runs as a child process: the gate covers the cli a person uses.
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

async function runOn(dir, out) {
  const r = await cli(["--in", dir, "--out", out]);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  return {
    places: JSON.parse(readFileSync(join(out, "places.json"), "utf8")),
    roads: JSON.parse(readFileSync(join(out, "roads.json"), "utf8")),
    lexicon: JSON.parse(readFileSync(join(out, "lexicon.json"), "utf8")),
  };
}

test("a place inside a box is kept with its id", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { places } = await runOn(dir, out);
    const fort = places.places.find((p) => p.id === "108867");
    assert.ok(fort, "the fort of the box was dropped");
    assert.equal(fort.chapter, "I");
    assert.equal(fort.kind, "fort");
    assert.equal(fort.source, "pleiades");
    assert.equal(fort.lat, 51.644379);
    assert.equal(fort.lon, 6.469807);
    const chapterPlace = places.places.find((p) => p.id === "108896");
    assert.ok(chapterPlace, "the place of the chapter was dropped");
    assert.equal(chapterPlace.name, "Bonna");
    assert.equal(chapterPlace.kind, "chapter-place");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a place outside every box is dropped", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { places } = await runOn(dir, out);
    assert.equal(places.places.find((p) => p.id === "999999"), undefined);
    assert.ok(!JSON.stringify(places).includes("Far away"), "a place out of every box reached the file");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a road inside a box is kept and a road outside every box is dropped", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { roads } = await runOn(dir, out);
    const road = roads.roads.find((r) => r.id === "42");
    assert.ok(road, "the road of the box was dropped");
    assert.equal(road.chapter, "I");
    assert.equal(road.name, "Nijmegen-Xanten");
    assert.equal(road.source, "itinere");
    assert.ok(road.points.length > 0, "the road has no point");
    const [lon, lat] = road.points[0];
    assert.ok(lat > 51.45 && lat < 51.8, `the point is at ${lat}`);
    assert.ok(lon > 6.25 && lon < 6.7, `the point is at ${lon}`);
    assert.equal(roads.roads.find((r) => r.id === "43"), undefined);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a lexicon term is kept with its passage ref", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { lexicon } = await runOn(dir, out);
    const move = lexicon.orders.find((o) => o.order === "move");
    assert.ok(move, "the move order is missing");
    assert.equal(move.term, "procedere");
    assert.equal(move.ref, "caesar.bg.8.27");
    assert.equal(move.source, "perseus");
    assert.ok(move.quote.includes("procedere"), `the quote was ${move.quote}`);
    assert.ok(move.meaning.length > 0, "the term has no meaning");
    assert.ok(move.use.length > 0, "the term has no command use");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the lexicon holds the five orders of the plan", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { lexicon } = await runOn(dir, out);
    const orders = lexicon.orders.map((o) => o.order);
    assert.deepEqual(orders, ["move", "hold", "form_line", "loose_pila", "relieve"]);
    for (const entry of lexicon.orders) {
      assert.ok(entry.ref.length > 0, `${entry.order} has no ref`);
      assert.ok(entry.quote.length > 0, `${entry.order} has no quote`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a wikidata battle outside the era of the chapters is dropped", async () => {
  const dir = corpus();
  try {
    const out = join(dir, "rome");
    const { places } = await runOn(dir, out);
    const battle = places.places.find((p) => p.kind === "battle");
    assert.ok(battle, "the battle of the box was dropped");
    assert.equal(battle.id, "Q2707263");
    assert.equal(battle.year, 101);
    assert.equal(places.places.find((p) => p.id === "Q9999999"), undefined);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the same input twice gives the same bytes", async () => {
  const dir = corpus();
  try {
    const first = join(dir, "rome-a");
    const second = join(dir, "rome-b");
    await runOn(dir, first);
    await runOn(dir, second);
    for (const name of ["places.json", "roads.json", "lexicon.json"]) {
      const a = readFileSync(join(first, name));
      const b = readFileSync(join(second, name));
      assert.equal(a.toString("utf8"), b.toString("utf8"), `${name} differs between two runs`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing source names the fetch command and the run fails", async () => {
  const dir = tmp();
  try {
    const out = join(dir, "rome");
    const r = await cli(["--in", dir, "--out", out]);
    assert.equal(r.status, 1);
    assert.match(r.stdout, /pleiades/);
    assert.match(r.stdout, /node tools\/codex\/fetch\.mjs pleiades/);
    assert.match(r.stdout, /node tools\/codex\/fetch\.mjs wikidata/);
    assert.match(r.stdout, /node tools\/codex\/fetch\.mjs itinere/);
    assert.equal(existsSync(join(out, "places.json")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a term the corpus does not hold stops the run and writes no lexicon", async () => {
  const dir = corpus();
  try {
    const file = join(dir, "index.jsonl");
    const records = readFileSync(file, "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => JSON.parse(line));
    const move = records.find((r) => r.ref === "caesar.bg.8.27");
    move.text = "nihil in hoc loco dictum est";
    writeFileSync(file, `${records.map((r) => JSON.stringify(r)).join("\n")}\n`);
    const out = join(dir, "rome");
    const r = await cli(["--in", dir, "--out", out]);
    assert.equal(r.status, 1);
    assert.match(r.stdout, /procedere/);
    assert.equal(existsSync(join(out, "lexicon.json")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
