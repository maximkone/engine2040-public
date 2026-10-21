// Run: node --test tools/codex/boards.test.mjs
// The Rome codex board gate (CDX5): the storyboard of Trajan's Column and
// the reference boards. The test never touches the network: it feeds the
// tool its own fixture answers through a fake fetch, so the gate covers the
// scene list, the board list, the public domain rule, the no caption rule
// and the byte identical second run.
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  BOARDS,
  articItem,
  boardItems,
  columnFile,
  isPublicDomainPlate,
  metItem,
  parsePlateTitle,
  platesFromCommons,
  planRequests,
  run,
  sceneSegments,
  scenesFromPlates,
  sourceRules,
} from "./boards.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
const WORK = "Die Reliefs der Traianss\u00e4ule";

function tmp() {
  return mkdtempSync(join(tmpdir(), "codex-boards-"));
}

// Every file under a folder, as paths relative to it.
function listFiles(root) {
  const out = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    if (entry.isDirectory()) out.push(...listFiles(full).map((name) => join(entry.name, name)));
    else out.push(entry.name);
  }
  return out;
}

// One Commons plate file as the api gives it.
function plateFile(number, plate, description, licence = "Public domain", code = "pd") {
  const title = `File:${number} Conrad Cichorius, ${WORK}, Tafel ${plate}.jpg`;
  return {
    title,
    imageinfo: [
      {
        url: `https://upload.wikimedia.org/wikipedia/commons/a/a${number}/00${number}_Tafel_${plate}.jpg`,
        thumburl: `https://upload.wikimedia.org/wikipedia/commons/thumb/a/a${number}/00${number}_Tafel_${plate}.jpg/800px-00${number}.jpg`,
        descriptionurl: `https://commons.wikimedia.org/wiki/File:00${number}_Tafel_${plate}.jpg`,
        mime: "image/jpeg",
        extmetadata: {
          ImageDescription: { value: description },
          LicenseShortName: { value: licence },
          License: { value: code },
          Credit: { value: `Conrad Cichorius: ${WORK}` },
        },
      },
    ],
  };
}

// The Commons answer of a fixture with four plate files.
function commonsAnswer() {
  return {
    query: {
      pages: {
        1: plateFile("004", "IV", "<i>The watch on the Danube</i> (scene I)"),
        2: plateFile("005", "V", "The watch on the Danube (scene I); river-traffic on the Danube frontier (scene II)"),
        3: plateFile("006", "VI", "<i>The base: northwest side</i> (scene III)"),
        4: plateFile("044", "XLIV", "Advance into the mountains (scene LXIII)"),
      },
    },
  };
}

// One Met object as the api gives it.
function metObject(id, title, objectName, over = {}) {
  return {
    objectID: id,
    title,
    objectName,
    repository: "Metropolitan Museum of Art, New York, NY",
    objectURL: `https://www.metmuseum.org/art/collection/search/${id}`,
    primaryImage: `https://images.metmuseum.org/CRDImages/gr/original/DP-${id}.jpg`,
    isPublicDomain: true,
    culture: "Roman",
    period: "Mid-Imperial",
    objectDate: "1st century CE",
    medium: "Bronze",
    ...over,
  };
}

// One Art Institute artwork as the api gives it.
function articArtwork(id, title, over = {}) {
  return {
    id,
    title,
    image_id: `img-${id}`,
    is_public_domain: true,
    artist_display: "Roman",
    date_display: "1st century CE",
    department_title: "Arts of Greece, Rome, and Byzantium",
    classification_title: "bronze",
    medium_display: "Bronze",
    place_of_origin: "Roman",
    main_reference_number: `19xx.${id}`,
    description: "",
    ...over,
  };
}

const ARTIC_CONFIG = { iiif_url: "https://www.artic.edu/iiif/2", website_url: "https://www.artic.edu" };

// A fake fetch: it answers every url of the plan from the fixture answers.
function fakeFetch({ commons, metObjects, artic }) {
  return async (url) => {
    const ok = (body) => ({ ok: true, status: 200, text: async () => JSON.stringify(body) });
    if (url.includes("commons.wikimedia.org")) return ok(commons);
    if (url.includes("/v1/objects/")) {
      const id = url.slice(url.lastIndexOf("/") + 1);
      const object = metObjects[id];
      if (!object) return { ok: false, status: 404, text: async () => "" };
      return ok(object);
    }
    if (url.includes("metmuseum.org")) return ok({ objectIDs: Object.keys(metObjects) });
    if (url.includes("api.artic.edu")) return ok({ data: artic, config: ARTIC_CONFIG });
    return { ok: false, status: 404, text: async () => "" };
  };
}

// A source table with no rate limit, so the gate runs fast.
function sourceTable(dir) {
  const file = join(dir, "sources.json");
  writeFileSync(
    file,
    JSON.stringify({
      version: 1,
      askAboveBytes: 524288000,
      sources: ["commons", "artic", "met"].map((id) => ({
        id,
        label: id,
        format: "JSON",
        licence: "CC0",
        rateLimitMs: 0,
        sizeCapBytes: 1048576,
        unitBytes: 1024,
        url: `https://example.invalid/${id}/{id}`,
        items: [{ id: "x", name: "x.json" }],
      })),
    }),
  );
  return file;
}

test("a plate file gives its number, its work and its plate", () => {
  const parsed = parsePlateTitle(`File:004 Conrad Cichorius, ${WORK}, Tafel IV.jpg`);
  assert.equal(parsed.number, 4);
  assert.equal(parsed.plate, "IV");
  assert.equal(parsed.plateNumber, 4);
  assert.equal(parsed.work, WORK);
  assert.equal(parsed.detail, false);
  const cut = parsePlateTitle(`File:004 Conrad Cichorius, ${WORK}, Tafel IV (Ausschnitt 01).jpg`);
  assert.equal(cut.detail, true);
  assert.equal(parsePlateTitle("File:Tubicen.jpg"), null);
});

test("a plate page names its scenes and the text that stands before each", () => {
  const segments = sceneSegments("<i>The watch on the Danube</i> (scene I); river-traffic on the Danube frontier (scene II)");
  assert.equal(segments.length, 2);
  assert.equal(segments[0].scene, "I");
  assert.equal(segments[0].caption, "The watch on the Danube");
  assert.equal(segments[1].scene, "II");
  assert.equal(segments[1].caption, "river-traffic on the Danube frontier");
  const two = sceneSegments("Construction of fortifications (scenes CXXVIII, CXXIX)");
  assert.deepEqual(two.map((s) => s.scene), ["CXXVIII", "CXXIX"]);
  assert.equal(two[0].caption, "Construction of fortifications");
});

test("the scene list holds the number, the image url and the caption", () => {
  const plates = platesFromCommons(commonsAnswer());
  const scenes = scenesFromPlates(plates);
  const first = scenes.find((s) => s.number === "I");
  assert.ok(first, "the scene I is missing");
  assert.equal(first.index, 1);
  assert.equal(first.caption, "The watch on the Danube");
  assert.equal(first.caption_from_source, true);
  assert.match(first.image, /^https:\/\/upload\.wikimedia\.org\//);
  assert.match(first.source, /^https:\/\/commons\.wikimedia\.org\/wiki\/File:/);
  assert.equal(first.licence, "Public domain");
  assert.equal(first.plate, "IV");
  const second = scenes.find((s) => s.number === "II");
  assert.equal(second.caption, "river-traffic on the Danube frontier");
  assert.deepEqual(
    scenes.map((s) => s.index),
    [...scenes.map((s) => s.index)].sort((a, b) => a - b),
  );
});

test("a plate that is not public domain is left out", () => {
  const answer = {
    query: {
      pages: {
        1: plateFile("004", "IV", "The watch on the Danube (scene I)"),
        2: plateFile("005", "V", "river-traffic on the Danube frontier (scene II)", "CC BY-SA 4.0", "cc-by-sa-4.0"),
      },
    },
  };
  const plates = platesFromCommons(answer);
  assert.equal(isPublicDomainPlate(plates[0]), true);
  assert.equal(isPublicDomainPlate(plates[1]), false);
  const scenes = scenesFromPlates(plates);
  assert.ok(scenes.find((s) => s.number === "I"), "the public domain plate is missing");
  assert.equal(scenes.find((s) => s.number === "II"), undefined, "a non public domain plate reached the storyboard");
});

test("a plate page without a caption gives a caption that states only the source", () => {
  const answer = { query: { pages: { 1: plateFile("006", "VI", "(scene III)") } } };
  const scenes = scenesFromPlates(platesFromCommons(answer));
  assert.equal(scenes.length, 1);
  const scene = scenes[0];
  assert.equal(scene.number, "III");
  assert.equal(scene.caption_from_source, false);
  assert.equal(scene.caption, `Plate VI of ${WORK}`);
  assert.match(scene.caption, /^Plate VI of /);
});

test("a board item carries the museum id, the image url and the licence", () => {
  const met = metItem(metObject(248742, "Iron sword with bronze hilt", "Sword with bronze handle"), "CC0");
  assert.equal(met.museum_id, "met");
  assert.equal(met.record_id, "248742");
  assert.equal(met.licence, "CC0");
  assert.match(met.image, /^https:\/\/images\.metmuseum\.org\//);
  assert.match(met.record, /^https:\/\/www\.metmuseum\.org\/art\/collection\/search\//);

  const artic = articItem(articArtwork(52731, "Helmet"), "CC0", ARTIC_CONFIG);
  assert.equal(artic.museum_id, "artic");
  assert.equal(artic.record_id, "52731");
  assert.equal(artic.licence, "CC0");
  assert.equal(artic.image_id, "img-52731");
  assert.equal(artic.image, "https://www.artic.edu/iiif/2/img-52731/full/843,/0/default.jpg");
  assert.equal(artic.record, "https://www.artic.edu/artworks/52731");
});

test("a board holds only the records that name its object", () => {
  const board = BOARDS.find((b) => b.kit === "helmet");
  const records = [
    metItem(metObject(255865, "Bronze cheek piece of a helmet", "Helmet cheek-piece of a helmet"), "CC0"),
    metItem(metObject(241717, "Terracotta oil lamp", "Lamp"), "CC0"),
    articItem(articArtwork(52731, "Helmet"), "CC0", ARTIC_CONFIG),
  ];
  const items = boardItems(records, board, 12);
  const ids = items.map((i) => i.record_id);
  assert.ok(ids.includes("255865"), "the helmet is missing");
  assert.ok(ids.includes("52731"), "the art institute helmet is missing");
  assert.ok(!ids.includes("241717"), "an oil lamp reached the helmet board");
  for (const item of items) assert.equal(item.subject, "helmet");
});

test("the five boards of the row are there", () => {
  assert.deepEqual(
    BOARDS.map((b) => b.kit),
    ["helmet", "gladius", "scutum", "lorica", "dolabra"],
  );
  for (const board of BOARDS) {
    assert.ok(board.nameRe.test(board.subject), `${board.kit}: the subject does not match the pattern`);
    assert.ok(board.queries.length > 0, `${board.kit}: no query`);
    for (const query of board.queries) {
      assert.ok(query.museum === "met" || query.museum === "artic", `${board.kit}: unknown museum`);
      assert.ok(query.term.length > 0, `${board.kit}: a query has no term`);
    }
  }
});

test("the source table gives the rate limit and the licence of the sources", () => {
  const rules = sourceRules(join(REPO, "tools", "codex", "sources.json"));
  for (const id of ["commons", "artic", "met"]) {
    const rule = rules.get(id);
    assert.ok(rule, `the source table has no ${id}`);
    assert.ok(rule.rateLimitMs >= 0, `${id} has no rate limit`);
    assert.ok(rule.licence.length > 0, `${id} has no licence`);
  }
});

test("the storyboard file holds the source and the scenes", () => {
  const scenes = scenesFromPlates(platesFromCommons(commonsAnswer()));
  const file = columnFile(scenes, { scenes: scenes.length, plates: 4 });
  assert.equal(file.kind, "column.storyboard");
  assert.match(file.source.category, /Cichorius Plates/);
  assert.equal(file.scenes.length, scenes.length);
  for (const scene of file.scenes) {
    assert.ok(scene.image, "a scene has no image");
    assert.ok(scene.source, "a scene has no source");
  }
});

test("the plan names every request of the run", () => {
  const plan = planRequests([BOARDS.find((b) => b.kit === "helmet")]);
  assert.ok(plan.length > 1, "the plan holds no request");
  assert.equal(plan[0].id, "commons-files");
  assert.equal(plan[0].source, "commons");
  for (const request of plan) {
    assert.match(request.url, /^https:\/\//, `${request.id} has no https url`);
    assert.ok(request.id.length > 0, "a request has no id");
  }
  const ids = plan.map((r) => r.id);
  assert.equal(new Set(ids).size, ids.length, "two requests share an id");
});

test("the same input twice gives byte identical output", async () => {
  const dir = tmp();
  try {
    const board = BOARDS.find((b) => b.kit === "helmet");
    const metObjects = {
      255865: metObject(255865, "Bronze cheek piece of a helmet", "Helmet cheek-piece of a helmet"),
      248742: metObject(248742, "Iron sword with bronze hilt", "Sword with bronze handle"),
    };
    const artic = [articArtwork(52731, "Helmet")];
    const fetchImpl = fakeFetch({ commons: commonsAnswer(), metObjects, artic });
    const first = join(dir, "one");
    const second = join(dir, "two");
    const base = {
      sourcesFile: sourceTable(dir),
      boards: [board],
      maxItems: 5,
      fetchImpl,
      log: () => {},
    };
    const a = await run({ ...base, cacheDir: join(dir, "cache-a"), outDir: first });
    assert.equal(a.ok, true);
    const b = await run({ ...base, cacheDir: join(dir, "cache-b"), outDir: second });
    assert.equal(b.ok, true);
    assert.equal(a.scenes, b.scenes);
    assert.equal(a.items, b.items);
    const names = listFiles(first).sort();
    assert.ok(names.length > 1, "the run wrote no file");
    assert.deepEqual(names, listFiles(second).sort());
    for (const name of names) {
      const left = readFileSync(join(first, name));
      const right = readFileSync(join(second, name));
      assert.deepEqual(left, right, `${name} differs between the two runs`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a run with a broken source writes nothing", async () => {
  const dir = tmp();
  try {
    const out = join(dir, "out");
    const result = await run({
      sourcesFile: sourceTable(dir),
      cacheDir: join(dir, "cache"),
      outDir: out,
      boards: [BOARDS.find((b) => b.kit === "helmet")],
      fetchImpl: async () => ({ ok: false, status: 500, text: async () => "" }),
      log: () => {},
    });
    assert.equal(result.ok, false);
    assert.equal(result.code, 1);
    assert.equal(existsSync(out), false, "a broken run wrote a file");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
