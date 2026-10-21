#!/usr/bin/env node
// places.mjs - the Rome codex places, roads and lexicon (CDX3).
//
// Job: cut the fetched codex sources into the three game data files of
// the Rome chapter plan:
//   - content/rome/places.json  the forts and the battles of the six
//     chapter areas, from Pleiades and Wikidata,
//   - content/rome/roads.json   the Itin-e roads inside the same areas,
//   - content/rome/lexicon.json the Latin words of the five orders of
//     docs/plan/rome.md section 4.5.
//
// The tool never downloads anything. It reads the raw files the fetch
// tool (CDX1) wrote into content/local/codex/<source>/ and the passage
// index the index tool (CDX2) wrote. A missing raw file is reported with
// the command that fetches it and the run stops.
//
// Usage:
//   node tools/codex/places.mjs [--in DIR] [--out DIR] [--dry-run] [--help]
//
// Rules:
//   - every entry keeps the id of the source it came from: the Pleiades
//     place id, the Wikidata Q-id, the Itin-e line id or the passage ref
//     of the codex index.
//   - a place joins the chapter whose box holds it; when two boxes hold
//     it, the chapter whose box centre is nearest wins.
//   - the five orders of the plan are all or nothing: a term the corpus
//     does not hold stops the run and no lexicon is written.
//   - the files are sorted and pretty printed, so two runs on the same
//     corpus write the same bytes.
//
// Exit: 0 ok, 1 a source is missing or a term is not in the corpus,
// 2 bad CLI.
import { createReadStream, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createGunzip } from "node:zlib";
import { createInterface } from "node:readline";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_IN = join(REPO, "content", "local", "codex");
export const DEFAULT_OUT = join(REPO, "content", "rome");

// The six chapter areas of docs/plan/rome.md section 3. Every box holds
// the ground of one chapter. The numbers of a box are the point of the
// Pleiades place the plan names, with 0.2 degrees around it, and the
// comment says where each number comes from.
export const CHAPTERS = [
  {
    chapter: "I",
    name: "Bonna",
    label: "Bonna fortress",
    year: 83,
    plan: "docs/plan/rome.md section 3: I Bonna fortress, 83 (Bonna, modern Alfen)",
    place: { source: "pleiades", id: "108896", title: "Col. Ulpia Traiana/Tricensimae" },
    boxes: [
      // Pleiades 108896 (Col. Ulpia Traiana/Tricensimae) at 51.666461 N
      // 6.443716 E: the Roman Bonna at Xanten, "modern Alfen" in the
      // plan. The fortress Castra Vetera (108867) at 51.644379 N
      // 6.469807 E and the camps at Wesel-Buederich and Rheinberg lie in
      // the same box.
      { lat_min: 51.45, lat_max: 51.8, lon_min: 6.25, lon_max: 6.7 },
    ],
    note:
      "the plan calls this chapter Bonna; Pleiades gives the place as Col. Ulpia Traiana/Tricensimae, " +
      "the Roman colony at Xanten, and its fortress Castra Vetera (108867) as a fort of the same box. " +
      "The Pleiades place titled Bonna (108818) is modern Bonn and is not in this box.",
  },
  {
    chapter: "II",
    name: "Nida",
    label: "Taunus forest",
    year: 83,
    plan: "docs/plan/rome.md section 3: II Taunus forest, 83 (Nida, modern Heddernheim / Taunus)",
    place: { source: "pleiades", id: "109205", title: "Nida" },
    boxes: [
      // Pleiades 109205 (Nida) at 50.154109 N 8.637743 E: Nida, modern
      // Heddernheim. The north edge takes in the Taunus forts
      // Kapersburg (50.313015 N 8.635684 E) and Langenhain
      // (50.365244 N 8.636999 E).
      { lat_min: 49.95, lat_max: 50.4, lon_min: 8.4, lon_max: 8.85 },
    ],
    note: "the plan calls this chapter the Taunus forest and names Nida as its place; Pleiades gives Nida as a settlement.",
  },
  {
    chapter: "III",
    name: "Mogontiacum",
    label: "Mogontiacum",
    year: 89,
    plan: "docs/plan/rome.md section 3: III Mogontiacum, 89 (Mainz)",
    place: { source: "pleiades", id: "109169", title: "Mogontiacum" },
    boxes: [
      // Pleiades 109169 (Mogontiacum) at 49.998912 N 8.273869 E: Mainz.
      // 0.2 degrees around it.
      { lat_min: 49.8, lat_max: 50.2, lon_min: 8.07, lon_max: 8.48 },
    ],
    note: "the plan calls this chapter Mogontiacum; Pleiades gives Mogontiacum as a settlement.",
  },
  {
    chapter: "IV",
    name: "Drobeta",
    label: "Danube",
    year: "101-105",
    plan: "docs/plan/rome.md section 3: IV Danube, 101-105 (Drobeta, modern Turnu Severin, and the Tapae pass)",
    place: { source: "pleiades", id: "207100", title: "Drobeta" },
    boxes: [
      // Pleiades 207100 (Drobeta) at 44.629271 N 22.6571725 E: modern
      // Turnu Severin. The west edge takes in the Iron Gates and the
      // Tapae pass: Pleiades 207187 (Insula Banului) at 44.658331 N
      // 22.549997 E and the Itin-e Iron Gates road.
      { lat_min: 44.4, lat_max: 44.85, lon_min: 22.15, lon_max: 22.9 },
      // Wikidata gives the Battle of Tapae (Q1476280, 87 CE) and the
      // Third Battle of Tapae (Q2707263, 101 CE) the point 45.5056 N
      // 22.7247 E. The plan puts the Tapae pass in this chapter, so the
      // battle sites join it. The box is small on purpose: the Roman
      // colony Ulpia Traiana Sarmizegetusa (207527) at 45.514745 N
      // 22.785267 E is the chapter V place and stays outside.
      { lat_min: 45.4556, lat_max: 45.5556, lon_min: 22.6747, lon_max: 22.7747 },
    ],
    note:
      "the plan calls this chapter the Danube and names Drobeta and the Tapae pass; Pleiades gives Drobeta as a " +
      "settlement and Insula Banului (207187) as the Iron Gates site. Wikidata gives the two Tapae battles their own point.",
  },
  {
    chapter: "V",
    name: "Sarmizegetusa",
    label: "Dacian mountains",
    year: 106,
    plan: "docs/plan/rome.md section 3: V Dacian mountains, 106 (Sarmizegetusa)",
    place: { source: "pleiades", id: "207425", title: "Sarmizegetusa (Regia)" },
    boxes: [
      // Pleiades 207425 (Sarmizegetusa (Regia)) at 45.669615 N 23.323961 E:
      // the Dacian capital the plan names. The west edge takes in Pleiades
      // 207527 (Ulpia Traiana Sarmizegetusa) at 45.514745 N 22.785267 E,
      // the Roman colony below it, and the mines of Alun and Ruda.
      { lat_min: 45.3, lat_max: 45.75, lon_min: 22.55, lon_max: 23.4 },
    ],
    note:
      "the plan calls this chapter the Dacian mountains and names Sarmizegetusa; Pleiades gives Sarmizegetusa (Regia) " +
      "as the Dacian capital and Ulpia Traiana Sarmizegetusa (207527) as the Roman colony of the same box.",
  },
  {
    chapter: "VI",
    name: "Rome",
    label: "Rome",
    year: 107,
    plan: "docs/plan/rome.md section 3: VI Rome, 107 (Rome)",
    place: { source: "pleiades", id: "423025", title: "Roma" },
    boxes: [
      // Pleiades 423025 (Roma) at 41.889977 N 12.491258 E: Rome.
      // 0.2 degrees around it.
      { lat_min: 41.7, lat_max: 42.05, lon_min: 12.25, lon_max: 12.7 },
    ],
    note: "the plan calls this chapter Rome; Pleiades gives Roma as the ancient city.",
  },
];

// The Pleiades types that are a fort. A place of these types is a fort
// of the chapter ground.
export const FORT_TYPES = new Set([
  "fort",
  "fort-2",
  "fortlet",
  "fort-group",
  "fortified-settlement",
  "hillfort",
  "military-installation-or-camp-temporary",
  "military-base",
  "castellum",
]);
// The Pleiades type of a battle site. Pleiades holds none today; the
// battles come from Wikidata.
export const BATTLE_TYPES = new Set(["battle"]);
// The Wikidata items the rows may hold: the Roman legion and a battle.
export const LEGION_ITEM = "Q163323";
export const BATTLE_ITEM = "Q178561";
// The years the battles of the chapters may fall in. The chapters run
// from 83 to 107 CE, so a battle outside this window is not chapter
// ground.
export const ERA_MIN = -100;
export const ERA_MAX = 150;
// The earth radius of EPSG:3395 (WGS84), the projection of the Itin-e
// road file.
export const EARTH_R = 6378137;

// The five orders of docs/plan/rome.md section 4.5. Each term is a Latin
// word the fetched corpus holds, with the passage it comes from. The
// term is matched with its stem, so the passage may hold an inflected
// form. The imperative a centurion shouts is not in the corpus: no
// source for Vegetius is in tools/codex/sources.json yet, so the terms
// are the attested Latin forms.
export const ORDERS = [
  {
    order: "move",
    label: "move",
    term: "procedere",
    stem: "procedere",
    meaning: "to move forward, to advance",
    use: "the order the player gives to move the line forward",
    ref: "caesar.bg.8.27",
  },
  {
    order: "hold",
    label: "hold",
    term: "consistere",
    stem: "consist",
    meaning: "to stand fast, to hold the ground",
    use: "the order to hold the ground while the ranks close up",
    ref: "caesar.bg.2.11",
  },
  {
    order: "form_line",
    label: "form line",
    term: "aciem instruere",
    stem: "aciem instru",
    meaning: "to draw up the battle line",
    use: "the order to form the line",
    ref: "caesar.bg.1.24",
  },
  {
    order: "loose_pila",
    label: "loose pila",
    term: "pila immittere",
    stem: "pila in hostes immitt",
    meaning: "to throw the javelins at the enemy",
    use: "the order to loose the pila",
    ref: "caesar.bg.6.8",
  },
  {
    order: "relieve",
    label: "relieve",
    term: "succedere",
    stem: "succed",
    meaning: "to take the place of the tired, to relieve",
    use: "the order to relieve the front rank with fresh men",
    ref: "caesar.bg.7.85",
  },
];

/** A number with six decimals. The output is a game table, not a survey. */
export function round6(value) {
  return Math.round(Number(value) * 1e6) / 1e6;
}

/** True when the point is inside the box. */
export function inBox(lat, lon, box) {
  return lat >= box.lat_min && lat <= box.lat_max && lon >= box.lon_min && lon <= box.lon_max;
}

/** The distance of a point from the centre of a box, in degrees. */
function centreDistance(lat, lon, box) {
  const dLat = lat - (box.lat_min + box.lat_max) / 2;
  const dLon = lon - (box.lon_min + box.lon_max) / 2;
  return Math.sqrt(dLat * dLat + dLon * dLon);
}

/** The chapter a point belongs to: the chapter of the box that holds it.
 * When two boxes hold it, the chapter whose box centre is nearest wins;
 * a tie goes to the chapter that comes first in the table. */
export function chapterFor(lat, lon, chapters = CHAPTERS) {
  let best = null;
  let bestDistance = Infinity;
  for (const chapter of chapters) {
    for (const box of chapter.boxes) {
      if (!inBox(lat, lon, box)) continue;
      const distance = centreDistance(lat, lon, box);
      if (distance < bestDistance) {
        best = chapter;
        bestDistance = distance;
      }
    }
  }
  return best;
}

/** The last part of a wikidata uri: "Q163323" of ".../entity/Q163323". */
export function qid(uri) {
  const text = String(uri ?? "");
  const at = text.lastIndexOf("/");
  return at < 0 ? text : text.slice(at + 1);
}

/** The point of a wikidata coordinate: "Point(8.27 50)" gives
 * { lat: 50, lon: 8.27 }. An empty string gives null. */
export function wktPoint(text) {
  const match = /Point\(\s*([-\d.]+)\s+([-\d.]+)\s*\)/.exec(String(text ?? ""));
  if (!match) return null;
  const lon = Number(match[1]);
  const lat = Number(match[2]);
  if (!Number.isFinite(lat) || !Number.isFinite(lon)) return null;
  return { lat, lon };
}

/** The year of a wikidata time value: "0106-01-01T00:00:00Z" gives 106,
 * "-0081-10-30T00:00:00Z" gives -81. An empty string gives null. */
export function yearOf(text) {
  const match = /^([+-])?(\d{4})/.exec(String(text ?? ""));
  if (!match) return null;
  const year = Number(match[2]);
  return match[1] === "-" ? -year : year;
}

/** The places of the Pleiades dump. The dump is one json document and the
 * place objects of its @graph array start and end at the left margin, so
 * the file is read line by line and cut at those objects. The last object
 * ends on the line that closes the array, so the line is cut at its first
 * closing brace. */
export async function* pleiadesPlaces(file) {
  const lines = createInterface({
    input: createReadStream(file).pipe(createGunzip()),
    crlfDelay: Infinity,
  });
  let inGraph = false;
  let element = [];
  for await (const line of lines) {
    if (!inGraph) {
      if (line.startsWith('"@graph": [')) {
        inGraph = true;
        const rest = line.slice('"@graph": ['.length);
        if (rest.startsWith("{")) element = [rest];
      }
      continue;
    }
    if (element.length === 0) {
      if (line === "{") element = [line];
      continue;
    }
    if (line.startsWith("}")) {
      const cut = line.indexOf("}");
      element.push(line.slice(0, cut + 1));
      const text = element.join("\n");
      element = [];
      yield JSON.parse(text);
      continue;
    }
    element.push(line);
  }
}

/** The rows of a wikidata sparql answer. The columns the query names are
 * legion, place, coord, placeCoord, date and kind; the answer of the
 * query of tools/codex/sources.json names legion and place only, so the
 * old column names are read as well. */
export function wikidataRows(file) {
  const raw = JSON.parse(readFileSync(file, "utf8"));
  const bindings = Array.isArray(raw?.results?.bindings) ? raw.results.bindings : [];
  return bindings.map((b) => ({
    item: b.item?.value ?? b.legion?.value ?? "",
    itemLabel: b.itemLabel?.value ?? b.legionLabel?.value ?? "",
    kind: qid(b.kind?.value),
    place: b.place?.value ?? "",
    placeLabel: b.placeLabel?.value ?? "",
    coord: b.coord?.value ?? "",
    placeCoord: b.placeCoord?.value ?? "",
    date: b.date?.value ?? "",
  }));
}

/** What a wikidata row is about: a Roman legion, a battle, or nothing we
 * can use. The kind column names it; without that column the shape of
 * the row tells it. */
export function rowKind(row) {
  if (row.kind === LEGION_ITEM) return "legion";
  if (row.kind === BATTLE_ITEM) return "battle";
  if (row.kind) return "other";
  if (row.place || row.placeCoord) return "legion";
  if (row.date) return "battle";
  return "other";
}

/** The road features of the Itin-e geojson. The file holds one feature
 * per line, the last one without its comma, so it is read line by line. */
export async function* itinereFeatures(file) {
  const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
  for await (const raw of lines) {
    let line = raw.trim();
    if (!line.startsWith("{")) continue;
    if (line.endsWith(",")) line = line.slice(0, -1);
    let feature;
    try {
      feature = JSON.parse(line);
    } catch {
      continue;
    }
    if (feature?.type !== "Feature") continue;
    yield feature;
  }
}

/** The easting of EPSG:3395 for a longitude. */
export function mercatorX(lon) {
  return (lon * Math.PI * EARTH_R) / 180;
}

/** The northing of EPSG:3395 for a latitude. */
export function mercatorY(lat) {
  return EARTH_R * Math.log(Math.tan(Math.PI / 4 + (lat * Math.PI) / 360));
}

/** The longitude of an EPSG:3395 easting. */
export function toLon(x) {
  return (x * 180) / (Math.PI * EARTH_R);
}

/** The latitude of an EPSG:3395 northing. */
export function toLat(y) {
  return ((2 * Math.atan(Math.exp(y / EARTH_R)) - Math.PI / 2) * 180) / Math.PI;
}

/** The sidecar of a fetched file: url, time, sha256 and licence. The
 * time is left out, so the output does not change with every fetch. */
export function readSidecar(file) {
  const path = `${file}.src.json`;
  if (!existsSync(path)) return {};
  try {
    const raw = JSON.parse(readFileSync(path, "utf8"));
    return {
      url: String(raw.url ?? ""),
      licence: String(raw.licence ?? ""),
      sha256: String(raw.sha256 ?? ""),
    };
  } catch {
    return {};
  }
}

/** The files of a source folder, sorted by name. The sidecar of a
 * fetched file (the .src.json next to it) is not a raw file. */
function filesOf(dir, match) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => match.test(name) && !name.endsWith(".src.json"))
    .sort()
    .map((name) => join(dir, name));
}

/** The raw files of the codex folder. Every source names the command
 * that fetches it. */
export function findSources(inDir = DEFAULT_IN) {
  return {
    pleiades: {
      id: "pleiades",
      files: filesOf(join(inDir, "pleiades"), /\.json(\.gz)?$/i),
      fetch: "node tools/codex/fetch.mjs pleiades",
    },
    wikidata: {
      id: "wikidata",
      files: filesOf(join(inDir, "wikidata"), /\.json$/i),
      fetch: "node tools/codex/fetch.mjs wikidata",
    },
    itinere: {
      id: "itinere",
      files: filesOf(join(inDir, "itinere"), /\.geojson$/i),
      fetch: "node tools/codex/fetch.mjs itinere",
    },
    index: {
      id: "index",
      files: existsSync(join(inDir, "index.jsonl")) ? [join(inDir, "index.jsonl")] : [],
      fetch: "node tools/codex/index.mjs",
    },
  };
}

/** The places of the chapter areas: the forts of Pleiades, the places of
 * the Roman legions of Wikidata, the battles of Wikidata and the place
 * every chapter names. */
export function buildPlaces({ pleiades = [], wikidata = [], chapters = CHAPTERS } = {}) {
  const places = [];
  const chapterPlaces = new Map();
  for (const chapter of chapters) {
    const found = pleiades.find((p) => String(p.id) === String(chapter.place.id));
    if (found) chapterPlaces.set(chapter.chapter, found);
  }
  for (const place of pleiades) {
    const point = Array.isArray(place.reprPoint) ? place.reprPoint : null;
    if (!point || point.length < 2) continue;
    const lat = Number(point[1]);
    const lon = Number(point[0]);
    if (!Number.isFinite(lat) || !Number.isFinite(lon)) continue;
    const chapter = chapterFor(lat, lon, chapters);
    if (!chapter) continue;
    const types = Array.isArray(place.placeTypes) ? place.placeTypes : [];
    const isChapterPlace = String(place.id) === String(chapter.place.id);
    const isFort = types.some((type) => FORT_TYPES.has(type));
    const isBattle = types.some((type) => BATTLE_TYPES.has(type));
    if (!isChapterPlace && !isFort && !isBattle) continue;
    const entry = {
      chapter: chapter.chapter,
      area: chapter.name,
      name: isChapterPlace ? chapter.name : String(place.title ?? ""),
      kind: isChapterPlace ? "chapter-place" : isBattle ? "battle" : "fort",
      lat: round6(lat),
      lon: round6(lon),
      source: "pleiades",
      id: String(place.id),
      uri: String(place.uri ?? `https://pleiades.stoa.org/places/${place.id}`),
      title: String(place.title ?? ""),
      types,
    };
    if (isChapterPlace) entry.note = chapter.note;
    places.push(entry);
  }
  const legionPlaces = new Map();
  for (const row of wikidata) {
    const kind = rowKind(row);
    if (kind === "other") continue;
    const point = wktPoint(kind === "legion" ? row.placeCoord || row.coord : row.coord);
    if (!point) continue;
    const chapter = chapterFor(point.lat, point.lon, chapters);
    if (!chapter) continue;
    if (kind === "legion") {
      const place = qid(row.place);
      const id = place || qid(row.item);
      let entry = legionPlaces.get(id);
      if (!entry) {
        entry = {
          chapter: chapter.chapter,
          area: chapter.name,
          name: String(row.placeLabel || row.itemLabel || ""),
          kind: "fort",
          lat: round6(point.lat),
          lon: round6(point.lon),
          source: "wikidata",
          id,
          uri: row.place || row.item,
          legions: [],
        };
        legionPlaces.set(id, entry);
      }
      entry.legions.push({ name: String(row.itemLabel ?? ""), id: qid(row.item) });
      continue;
    }
    const year = yearOf(row.date);
    if (year === null || year < ERA_MIN || year > ERA_MAX) continue;
    places.push({
      chapter: chapter.chapter,
      area: chapter.name,
      name: String(row.itemLabel ?? ""),
      kind: "battle",
      lat: round6(point.lat),
      lon: round6(point.lon),
      source: "wikidata",
      id: qid(row.item),
      uri: row.item,
      year,
    });
  }
  for (const entry of legionPlaces.values()) {
    entry.legions.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    places.push(entry);
  }
  const order = new Map(chapters.map((chapter, at) => [chapter.chapter, at]));
  places.sort((a, b) => {
    const byChapter = (order.get(a.chapter) ?? 99) - (order.get(b.chapter) ?? 99);
    if (byChapter !== 0) return byChapter;
    if (a.kind !== b.kind) return a.kind < b.kind ? -1 : 1;
    if (a.name !== b.name) return a.name < b.name ? -1 : 1;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
  return { places, chapterPlaces };
}

/** The roads of the chapter areas: the Itin-e features with a point
 * inside a box. The points kept are the ones inside the box. */
export function buildRoads({ features = [], chapters = CHAPTERS } = {}) {
  const roads = [];
  for (const feature of features) {
    const properties = feature.properties ?? {};
    const lines = feature.geometry?.coordinates ?? [];
    const counts = new Map();
    const points = [];
    for (const line of lines) {
      for (const point of line) {
        if (!Array.isArray(point) || point.length < 2) continue;
        const lat = toLat(Number(point[1]));
        const lon = toLon(Number(point[0]));
        if (!Number.isFinite(lat) || !Number.isFinite(lon)) continue;
        const chapter = chapterFor(lat, lon, chapters);
        if (!chapter) continue;
        counts.set(chapter.chapter, (counts.get(chapter.chapter) ?? 0) + 1);
        points.push([round6(lon), round6(lat)]);
      }
    }
    if (!points.length) continue;
    let best = null;
    let bestCount = 0;
    for (const chapter of chapters) {
      const count = counts.get(chapter.chapter) ?? 0;
      if (count > bestCount) {
        best = chapter;
        bestCount = count;
      }
    }
    if (!best) continue;
    roads.push({
      chapter: best.chapter,
      area: best.name,
      name: String(properties.Name ?? ""),
      type: String(properties.Type ?? ""),
      route_type: Number(properties.Route_Type) || 0,
      id: String(properties.InLine_FID ?? ""),
      source: "itinere",
      points,
    });
  }
  const order = new Map(chapters.map((chapter, at) => [chapter.chapter, at]));
  roads.sort((a, b) => {
    const byChapter = (order.get(a.chapter) ?? 99) - (order.get(b.chapter) ?? 99);
    if (byChapter !== 0) return byChapter;
    const byId = Number(a.id) - Number(b.id);
    if (byId !== 0) return byId;
    return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
  });
  return roads;
}

/** The sentences of a passage: the text is cut at a full stop or a
 * semicolon that ends a sentence. */
export function sentences(text) {
  return String(text ?? "")
    .replace(/\s+/g, " ")
    .split(/(?<=[.;])\s+/)
    .map((sentence) => sentence.trim())
    .filter(Boolean);
}

/** The sentence of a passage that holds the term. The term is matched
 * with its stem, so an inflected form of the word is found too. */
export function sentenceWith(text, stem) {
  const re = new RegExp(`\\b${stem.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[a-z]*\\b`, "i");
  for (const sentence of sentences(text)) {
    if (re.test(sentence)) return sentence.replace(/[;.,]+$/, "");
  }
  return "";
}

/** The lexicon of the five orders. Every entry keeps the passage ref of
 * the codex index and the source it comes from. */
export function buildLexicon({ records = [], orders = ORDERS } = {}) {
  const byRef = new Map();
  for (const record of records) byRef.set(String(record.ref ?? ""), record);
  const entries = [];
  const missing = [];
  for (const order of orders) {
    const record = byRef.get(order.ref);
    if (!record) {
      missing.push(`${order.order}: the passage ${order.ref} is not in the codex index`);
      continue;
    }
    const quote = sentenceWith(record.text ?? "", order.stem);
    if (!quote) {
      missing.push(`${order.order}: the term ${order.term} is not in the passage ${order.ref}`);
      continue;
    }
    entries.push({
      order: order.order,
      label: order.label,
      term: order.term,
      meaning: order.meaning,
      use: order.use,
      ref: order.ref,
      source: String(record.source ?? ""),
      file: String(record.file ?? ""),
      url: String(record.url ?? ""),
      licence: String(record.licence ?? ""),
      quote,
    });
  }
  return { entries, missing };
}

/** The passages of the codex index: one json line per passage. */
export function readIndex(file) {
  const lines = readFileSync(file, "utf8").split(/\r?\n/);
  const records = [];
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      records.push(JSON.parse(trimmed));
    } catch {
      // a line that is no json is no passage.
    }
  }
  return records;
}

/** The chapter areas of the output: the box of every chapter and the
 * place the plan names, with the point the source gives it. */
function chapterAreaOut(chapter, chapterPlace) {
  const point = Array.isArray(chapterPlace?.reprPoint) ? chapterPlace.reprPoint : null;
  const area = {
    chapter: chapter.chapter,
    name: chapter.name,
    label: chapter.label,
    year: chapter.year,
    plan: chapter.plan,
    source: chapter.place.source,
    id: String(chapter.place.id),
    title: String(chapterPlace?.title ?? chapter.place.title),
    note: chapter.note,
    boxes: chapter.boxes.map((box) => ({
      lat_min: box.lat_min,
      lat_max: box.lat_max,
      lon_min: box.lon_min,
      lon_max: box.lon_max,
    })),
  };
  if (point && point.length >= 2) {
    area.lat = round6(Number(point[1]));
    area.lon = round6(Number(point[0]));
  }
  if (chapterPlace?.uri) area.uri = String(chapterPlace.uri);
  return area;
}

/** The source block of an output file: the url and the licence of every
 * raw file the tool read. */
function sourcesOut(sources) {
  const out = [];
  for (const source of sources) {
    const sidecar = readSidecar(source.file);
    if (!sidecar.url) continue;
    out.push({
      id: source.id,
      file: source.name,
      url: sidecar.url,
      licence: sidecar.licence,
      sha256: sidecar.sha256,
    });
  }
  return out;
}

/** Cut the codex into the three Rome data files. */
export async function run({
  inDir = DEFAULT_IN,
  outDir = DEFAULT_OUT,
  dryRun = false,
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  const found = findSources(inDir);
  const missing = Object.values(found).filter((source) => !source.files.length);
  if (missing.length) {
    for (const source of missing) {
      log(`PLACES missing the ${source.id} source: run ${source.fetch}`);
    }
    return { ok: false, code: 1, places: [], roads: [], lexicon: [] };
  }
  const pleiades = [];
  for (const file of found.pleiades.files) {
    for await (const place of pleiadesPlaces(file)) pleiades.push(place);
  }
  log(`PLACES pleiades places=${pleiades.length}`);
  const wikidata = [];
  for (const file of found.wikidata.files) wikidata.push(...wikidataRows(file));
  const kinds = { legion: 0, battle: 0, other: 0 };
  for (const row of wikidata) kinds[rowKind(row)] += 1;
  log(`PLACES wikidata rows=${wikidata.length} legion=${kinds.legion} battle=${kinds.battle} other=${kinds.other}`);
  if (!kinds.legion && !kinds.battle) {
    log(
      "PLACES the wikidata answer holds no Roman legion and no battle: the query of tools/codex/sources.json " +
        "names Q11303, which is a skyscraper; the Roman legion is Q163323",
    );
  }
  const features = [];
  for (const file of found.itinere.files) {
    for await (const feature of itinereFeatures(file)) features.push(feature);
  }
  log(`PLACES itinere features=${features.length}`);
  const records = readIndex(found.index.files[0]);
  log(`PLACES index passages=${records.length}`);

  const { places, chapterPlaces } = buildPlaces({ pleiades, wikidata });
  const roads = buildRoads({ features });
  const { entries, missing: terms } = buildLexicon({ records });
  if (terms.length) {
    for (const line of terms) log(`PLACES ${line}`);
    log("PLACES the lexicon is not written: a term of the five orders is not in the corpus");
    return { ok: false, code: 1, places, roads, lexicon: [] };
  }
  const areas = CHAPTERS.map((chapter) => chapterAreaOut(chapter, chapterPlaces.get(chapter.chapter)));
  const sources = [
    ...found.pleiades.files.map((file) => ({ id: "pleiades", file, name: file.split(/[\\/]/).pop() })),
    ...found.wikidata.files.map((file) => ({ id: "wikidata", file, name: file.split(/[\\/]/).pop() })),
    ...found.itinere.files.map((file) => ({ id: "itinere", file, name: file.split(/[\\/]/).pop() })),
  ];
  const placesFile = {
    version: 1,
    generated_by: "tools/codex/places.mjs",
    sources: sourcesOut(sources),
    chapter_areas: areas,
    places,
  };
  const roadsFile = {
    version: 1,
    generated_by: "tools/codex/places.mjs",
    sources: sourcesOut(sources),
    roads,
  };
  const lexiconFile = {
    version: 1,
    generated_by: "tools/codex/places.mjs",
    sources: sourcesOut(sources),
    orders: entries,
  };
  const files = [
    { name: "places.json", value: placesFile },
    { name: "roads.json", value: roadsFile },
    { name: "lexicon.json", value: lexiconFile },
  ];
  for (const file of files) {
    const path = join(outDir, file.name);
    if (dryRun) {
      log(`PLACES would write ${path}`);
    } else {
      mkdirSync(outDir, { recursive: true });
      writeFileSync(path, `${JSON.stringify(file.value, null, 2)}\n`, "utf8");
      log(`PLACES wrote ${path}`);
    }
  }
  for (const area of areas) {
    log(
      `PLACE ${area.chapter} ${area.name} ${area.source}:${area.id} ` +
        `${area.lat ?? "-"},${area.lon ?? "-"} ${area.title}`,
    );
  }
  log(`PLACES places=${places.length} roads=${roads.length} orders=${entries.length}`);
  return { ok: true, code: 0, places, roads, lexicon: entries };
}

export const HELP = `Usage: node tools/codex/places.mjs [--in DIR] [--out DIR] [--dry-run] [--help]

Cuts the fetched codex sources into content/rome/places.json (the forts and
the battles of the six chapter areas of docs/plan/rome.md), into
content/rome/roads.json (the Itin-e roads of the same areas) and into
content/rome/lexicon.json (the Latin words of the five orders of section
4.5 of the same plan). Every entry keeps the id of its source.

  --in DIR     the codex folder the fetch tool wrote (content/local/codex)
  --out DIR    where the three files go (content/rome)
  --dry-run    print the plan, write nothing
  --help       print this help

The tool never downloads: a missing raw file is reported with the fetch
command to run. Exit 0 ok, 1 a source is missing or a term is not in the
corpus, 2 bad CLI.
`;

function parseArgs(args) {
  const out = {
    inDir: DEFAULT_IN,
    outDir: DEFAULT_OUT,
    dryRun: false,
    help: false,
  };
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
      continue;
    }
    if (arg === "--dry-run") {
      out.dryRun = true;
      continue;
    }
    if (arg === "--in" || arg === "--out") {
      const value = args[i + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} needs a value`);
      if (arg === "--in") out.inDir = resolve(value);
      else out.outDir = resolve(value);
      i += 1;
      continue;
    }
    if (arg.startsWith("--in=")) {
      out.inDir = resolve(arg.slice("--in=".length));
      continue;
    }
    if (arg.startsWith("--out=")) {
      out.outDir = resolve(arg.slice("--out=".length));
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  return out;
}

if (import.meta.main) {
  try {
    const opts = parseArgs(process.argv.slice(2));
    if (opts.help) {
      process.stdout.write(HELP);
    } else {
      const result = await run(opts);
      process.exitCode = result.code;
    }
  } catch (error) {
    process.stderr.write(`places: error: ${error.message}\n`);
    process.exitCode = 2;
  }
}
