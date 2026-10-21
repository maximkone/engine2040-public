// Run: node --test tools/codex/credits.test.mjs
// The Rome credits gate (CDX8): the credits page is written from the codex
// sidecars and the shipped Rome files. The test never touches the network: it
// feeds the tool a fixture source table, fixture Rome files and fixture
// sidecars, so the gate covers the citation rules, the sidecar precedence, the
// page text and the byte identical second run.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { buildCredits, parseRegistry, renderCredits, romeFileList, run } from "./credits.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

// The source table the fixture uses: two sources that share the Commons api,
// a museum and a book site.
const REGISTRY = {
  version: 1,
  askAboveBytes: 1000,
  sources: [
    {
      id: "pleiades",
      label: "Pleiades ancient world places",
      format: "JSON gz",
      licence: "CC BY 3.0",
      rateLimitMs: 2000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz",
      items: [{ id: "places", name: "pleiades-places-latest.json.gz" }],
    },
    {
      id: "met",
      label: "Met Museum open access object list",
      format: "CSV",
      licence: "CC0",
      rateLimitMs: 1000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://media.githubusercontent.com/media/metmuseum/openaccess/master/MetObjects.csv",
      items: [{ id: "csv", name: "MetObjects.csv" }],
    },
    {
      id: "artic",
      label: "Art Institute of Chicago open access search",
      format: "JSON",
      licence: "CC0",
      rateLimitMs: 1000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://api.artic.edu/api/v1/artworks/search?q={id}",
      items: [{ id: "roman helmet", name: "search-roman-helmet.json" }],
    },
    {
      id: "commons",
      label: "Wikimedia Commons Trajan's Column files",
      format: "JSON",
      licence: "per file, as the file page states",
      rateLimitMs: 1000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://commons.wikimedia.org/w/api.php?action=query&titles=Category%3A{id}&format=json",
      items: [{ id: "Trajan's Column", name: "category-trajans-column.json" }],
    },
    {
      id: "latin-audio",
      label: "Wikimedia Commons Latin pronunciation audio",
      format: "JSON",
      licence: "per file, as the file page states",
      rateLimitMs: 1000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://commons.wikimedia.org/w/api.php?action=query&titles=Category%3A{id}&format=json",
      items: [{ id: "Latin pronunciation", name: "category-latin-pronunciation.json" }],
    },
    {
      id: "gutenberg",
      label: "Project Gutenberg plain text",
      format: "UTF-8 text",
      licence: "US public domain",
      rateLimitMs: 5000,
      sizeCapBytes: 1000,
      unitBytes: 100,
      url: "https://www.gutenberg.org/ebooks/{id}.txt.utf-8",
      items: [{ id: "10657", name: "caesar-bellum-gallicum.txt" }],
    },
  ],
};

// A Rome file that carries the source block of its data.
const PLACES = {
  version: 1,
  generated_by: "tools/codex/places.mjs",
  sources: [
    {
      id: "pleiades",
      file: "pleiades-places-latest.json.gz",
      url: "https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz",
      licence: "CC BY 3.0",
      sha256: "d9c544b2fa8a1aa79f6b3195685f27aa65625a981b9c95b1022c89d7bd61f707",
    },
  ],
  places: [{ name: "Bonna", source: "pleiades" }],
};

// The storyboard of the column: the api of the source and the category it names.
const COLUMN = {
  version: 1,
  kind: "column.storyboard",
  source: {
    name: "Wikimedia Commons",
    category: "Category:Trajan's Column - Cichorius Plates",
    api: "https://commons.wikimedia.org/w/api.php",
    licence: "Public domain, as the file page states",
  },
  scenes: [{ number: "I", source: "https://commons.wikimedia.org/wiki/File:004.jpg", licence: "Public domain" }],
};

// A reference board: the museums it asks.
const BOARD = {
  version: 1,
  kind: "reference.board",
  kit: "helmet",
  queries: [
    { museum: "met", term: "helmet", match: "title" },
    { museum: "artic", term: "helmet", match: "title" },
  ],
  items: [{ name: "Helmet", museum_id: "artic", record_id: "52731", licence: "CC0" }],
};

// The camp table: a number, its ref and the codex row that checks it.
const CAMP = `# Roman camp works.
[ditch.one_night]
width_ft = 5
ref = "rome.md section 5, Vegetius 3.8; Hyginus 49"

[codex.ditch_width]
entry = "ditch.one_night"
key = "width_ft"
ref = "PHG 49"
form = "quinque"
`;

// A Rome table cut from a donor file: the provenance section carries the url,
// the licence and the sha256 of the donor.
const FORTIFICATIONS = `# The frontier of the empire as geometry.
[units]
m_per_deg_lat = 111132
ref = "WGS84: the length of one degree of latitude at 45 degrees north is 111132 m"

[provenance.dare]
id = "dare"
file = "fortifications.geojson"
url = "https://github.com/johaahlf/dare/blob/master/fortifications.geojson"
licence = "Apache-2.0"
sha256 = "f20449fc3a72594d10a88294d0d2c8d3c6b27312a8d6691ab53701674a0bc98d"

[segment.fl_moenvs_main]
name = "FL. MOENVS (Main)"
ref = "DARE:fortifications.geojson, the feature the atlas names FL. MOENVS (Main) (Apache-2.0)"
`;

// A tree with the given Rome files, sidecars and source table.
function fixture({ rome = {}, sidecars = {} } = {}) {
  const root = mkdtempSync(join(tmpdir(), "codex-credits-"));
  const romeDir = join(root, "rome");
  const cacheDir = join(root, "cache");
  mkdirSync(romeDir, { recursive: true });
  mkdirSync(cacheDir, { recursive: true });
  for (const [name, body] of Object.entries(rome)) {
    mkdirSync(join(romeDir, dirname(name)), { recursive: true });
    writeFileSync(join(romeDir, name), typeof body === "string" ? body : `${JSON.stringify(body, null, 2)}\n`);
  }
  for (const [name, body] of Object.entries(sidecars)) {
    const path = join(cacheDir, name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, `${JSON.stringify(body, null, 2)}\n`);
  }
  const sourcesFile = join(root, "sources.json");
  writeFileSync(sourcesFile, `${JSON.stringify(REGISTRY, null, 2)}\n`);
  return { root, romeDir, cacheDir, sourcesFile, outFile: join(root, "out", "CREDITS.md") };
}

// The credits of one source, by its id.
function section(credits, id) {
  return credits.find((entry) => entry.id === id) ?? null;
}

test("the source block of a Rome file credits the source with its url and licence", () => {
  const tree = fixture({ rome: { "places.json": PLACES } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  const pleiades = section(credits, "pleiades");
  assert.ok(pleiades, "the pleiades source is credited");
  assert.equal(pleiades.licence, "CC BY 3.0");
  assert.deepEqual(pleiades.usedBy, ["places.json"]);
  assert.equal(pleiades.rows.length, 1);
  assert.equal(pleiades.rows[0].url, "https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz");
  assert.equal(pleiades.rows[0].licence, "CC BY 3.0");
  assert.equal(pleiades.rows[0].sha256, "d9c544b2fa8a1aa79f6b3195685f27aa65625a981b9c95b1022c89d7bd61f707");
});

test("the sidecar of the cache wins over the source block of the Rome file", () => {
  const tree = fixture({
    rome: { "places.json": PLACES },
    sidecars: {
      "pleiades/pleiades-places-latest.json.gz.src.json": {
        source: "pleiades",
        url: "https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz",
        fetchedAt: "2026-10-21T09:00:00.000Z",
        sha256: "d9c544b2fa8a1aa79f6b3195685f27aa65625a981b9c95b1022c89d7bd61f707",
        licence: "CC BY 3.0",
      },
    },
  });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({
    files: romeFileList(tree.romeDir),
    registry,
    sidecars: [
      {
        source: "pleiades",
        key: "pleiades/pleiades-places-latest.json.gz",
        url: "https://atlantides.org/downloads/pleiades/json/pleiades-places-latest.json.gz",
        licence: "CC BY 3.0",
        sha256: "d9c544b2fa8a1aa79f6b3195685f27aa65625a981b9c95b1022c89d7bd61f707",
        fetchedAt: "2026-10-21T09:00:00.000Z",
      },
    ],
  });
  const pleiades = section(credits, "pleiades");
  assert.equal(pleiades.rows.length, 1);
  assert.equal(pleiades.rows[0].from, "sidecar");
  assert.equal(pleiades.rows[0].fetchedAt, "2026-10-21T09:00:00.000Z");
});

test("a board file credits the museums it asks", () => {
  const tree = fixture({ rome: { "boards/helmet.json": BOARD } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  assert.ok(section(credits, "met"), "the met source is credited");
  assert.ok(section(credits, "artic"), "the artic source is credited");
  assert.deepEqual(section(credits, "met").usedBy, ["boards/helmet.json"]);
});

test("the column file credits the Commons source and not the other Commons source", () => {
  const tree = fixture({ rome: { "column.json": COLUMN } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  assert.ok(section(credits, "commons"), "the commons source is credited");
  assert.equal(section(credits, "latin-audio"), null, "the latin audio source is not credited");
});

test("a ref that names a work of the codex credits the source of the work", () => {
  const tree = fixture({ rome: { "camp.toml": CAMP } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  const gutenberg = section(credits, "gutenberg");
  assert.ok(gutenberg, "the gutenberg source is credited");
  assert.equal(gutenberg.rows[0].key, "gutenberg/vegetius-de-re-militari.txt");
});

test("a ref that names a corpus passage credits the corpus", () => {
  const tree = fixture({ rome: { "camp.toml": CAMP } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  const corpus = section(credits, "lat_text_latin_library");
  assert.ok(corpus, "the corpus is credited");
  assert.equal(corpus.licence, "Public Domain Mark 1.0");
  assert.ok(
    corpus.rows.some((row) => row.key.endsWith("hyginus6.txt")),
    "the corpus holds the Pseudo-Hyginus file",
  );
});

test("a ref that names nothing in the codex is reported", () => {
  const tree = fixture({ rome: { "kit.toml": '[kit.boss]\ncertainty = "attested"\nref = "Adamklisi metope XVII"\n' } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits, unresolved } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  assert.equal(credits.length, 0, "no source is credited");
  assert.equal(unresolved.length, 1);
  assert.equal(unresolved[0].path, "kit.toml");
  assert.equal(unresolved[0].ref, "Adamklisi metope XVII");
});

test("a provenance section credits the donor file with its url, licence and sha256", () => {
  const tree = fixture({ rome: { "fortifications.toml": FORTIFICATIONS } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  const dare = section(credits, "dare");
  assert.ok(dare, "the donor file is credited");
  assert.equal(dare.licence, "Apache-2.0");
  assert.deepEqual(dare.usedBy, ["fortifications.toml"]);
  assert.equal(dare.rows.length, 1);
  assert.equal(dare.rows[0].url, "https://github.com/johaahlf/dare/blob/master/fortifications.geojson");
  assert.equal(dare.rows[0].sha256, "f20449fc3a72594d10a88294d0d2c8d3c6b27312a8d6691ab53701674a0bc98d");
});

test("a ref that names no donor and no source is reported", () => {
  const tree = fixture({ rome: { "fortifications.toml": FORTIFICATIONS } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { unresolved } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  assert.equal(unresolved.length, 1);
  assert.equal(unresolved[0].ref, "WGS84: the length of one degree of latitude at 45 degrees north is 111132 m");
});

test("the page names the source, the url and the licence", () => {
  const tree = fixture({ rome: { "places.json": PLACES } });
  const registry = parseRegistry(readFileSync(tree.sourcesFile, "utf8"));
  const { credits } = buildCredits({ files: romeFileList(tree.romeDir), registry });
  const text = renderCredits(credits, { unresolved: [] });
  assert.match(text, /Pleiades ancient world places/);
  assert.match(text, /CC BY 3\.0/);
  assert.match(text, /https:\/\/atlantides\.org\/downloads\/pleiades\/json\/pleiades-places-latest\.json\.gz/);
  assert.match(text, /places\.json/);
});

test("the run writes the page and the same input twice gives the same bytes", () => {
  const tree = fixture({ rome: { "places.json": PLACES, "column.json": COLUMN } });
  const first = run({
    romeDir: tree.romeDir,
    cacheDir: tree.cacheDir,
    sourcesFile: tree.sourcesFile,
    outFile: tree.outFile,
    log: () => {},
  });
  assert.equal(first.ok, true);
  assert.equal(first.sources, 2, "the pleiades and the commons source are credited");
  const text = readFileSync(tree.outFile, "utf8");
  const second = run({
    romeDir: tree.romeDir,
    cacheDir: tree.cacheDir,
    sourcesFile: tree.sourcesFile,
    outFile: tree.outFile,
    log: () => {},
  });
  assert.equal(second.ok, true);
  assert.equal(readFileSync(tree.outFile, "utf8"), text, "the second run writes the same bytes");
});

test("the run reports every source a Rome file cites", () => {
  const tree = fixture({ rome: { "places.json": PLACES, "boards/helmet.json": BOARD } });
  const lines = [];
  const result = run({
    romeDir: tree.romeDir,
    cacheDir: tree.cacheDir,
    sourcesFile: tree.sourcesFile,
    outFile: tree.outFile,
    log: (line) => lines.push(line),
  });
  assert.equal(result.sources, 3);
  const report = lines.join("\n");
  assert.match(report, /source=pleiades/);
  assert.match(report, /source=met/);
  assert.match(report, /source=artic/);
});

test("the run refuses a source table that is not a table", () => {
  const tree = fixture({ rome: { "places.json": PLACES } });
  writeFileSync(tree.sourcesFile, "not json");
  assert.throws(() => parseRegistry(readFileSync(tree.sourcesFile, "utf8")));
});
