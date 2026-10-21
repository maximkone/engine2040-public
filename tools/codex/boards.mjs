#!/usr/bin/env node
// boards.mjs - the Rome codex storyboard and reference boards (CDX5).
//
// Job: build two kinds of game data from the sources the codex names.
//
//   1. The storyboard of Trajan's Column. Every scene the Cichorius plates
//      name (the Cichorius number) gets an entry: the public domain image
//      of the plate that shows it, the plate page as the source and the one
//      line caption the plate page states. A plate page that names a scene
//      but gives no text for it gets a caption that states only the plate
//      and the work, and the entry is marked as no caption in the source.
//
//   2. The reference boards: helmet, gladius, scutum, lorica and dolabra.
//      Each board is a list of public domain museum records with the name
//      of the object, the museum record id, the image url, the licence and
//      the url of the record. The Met gives an image url, the Art
//      Institute gives an IIIF image id.
//
// Nothing is written that a source does not say. A record that is not
// public domain is left out. Every file is written or none is.
//
// The answers are cached under content/local/codex/boards/ with a sidecar
// (url, fetchedAt, sha256, licence), the same style as the fetch tool
// (CDX1). The cache holds the raw answers; the game files hold the data.
// A second run over the same cache writes byte identical files.
//
// Usage:
//   node tools/codex/boards.mjs [--sources FILE] [--cache DIR] [--out DIR]
//                                [--boards LIST] [--max-items N]
//                                [--max-candidates N] [--offline] [--dry-run]
//                                [--help]
//
// Rules:
//   - every request carries a User-Agent and waits the rate limit of its
//     source in tools/codex/sources.json.
//   - a record that is not public domain is never written.
//   - the run prints the plan before the first request.
//   - the run writes every file or none: a board with no item, a scene
//     without an image or a failed request stops the run before any write.
//
// Exit: 0 ok, 1 a source failed or the data is not fit to write, 2 bad CLI.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_SOURCES = join(HERE, "sources.json");
export const DEFAULT_CACHE = join(REPO, "content", "local", "codex", "boards");
export const DEFAULT_OUT = join(REPO, "content", "rome");
export const UA = "engine2040-codex-boards/1.0 (+https://github.com/maximkone/engine2040)";

const COMMONS_API = "https://commons.wikimedia.org/w/api.php";
const MET_SEARCH_API = "https://collectionapi.metmuseum.org/public/collection/v1.1/search";
const MET_OBJECT_API = "https://collectionapi.metmuseum.org/public/collection/v1/objects";
const ARTIC_SEARCH_API = "https://api.artic.edu/api/v1/artworks/search";

// The Commons category of the Cichorius plates. Every file in it is one
// drawing of the column relief; its description names the scenes it shows.
const COMMONS_CATEGORY = "Category:Trajan's Column - Cichorius Plates";
// The api gives the imageinfo of fifty files a page, so the files of the
// category are asked for in batches of this size.
const COMMONS_BATCH = 50;

// The Met department of the Greek and Roman Art. A board takes its objects
// from it, so a board holds objects of the period and not a jousting helm.
const MET_DEPARTMENT = 13;
// The Art Institute department of the Greeks, the Romans and Byzantium.
const ARTIC_DEPARTMENT = "Arts of Greece, Rome, and Byzantium";

const ARTIC_FIELDS = [
  "id",
  "title",
  "image_id",
  "is_public_domain",
  "artist_display",
  "date_display",
  "department_title",
  "classification_title",
  "medium_display",
  "place_of_origin",
  "main_reference_number",
  "description",
].join(",");

// The five boards. The searches fill them; the name pattern keeps a board
// to the object it names, so a helmet board holds helmets only.
export const BOARDS = [
  {
    kit: "helmet",
    label: "Helmet",
    note: "Helmets of the period. The name is the museum's own name for the object and the subject is the object the museum's own text names.",
    nameRe: /\bhelmet|\bgalea|cheek[- ]piece/i,
    subject: "helmet",
    queries: [
      { museum: "met", kind: "title", term: "helmet", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "helmet cheek", department: MET_DEPARTMENT },
      { museum: "artic", kind: "title", term: "Helmet" },
      { museum: "artic", kind: "description", term: "helmet" },
    ],
  },
  {
    kit: "gladius",
    label: "Gladius",
    note: "Swords of the period. The name is the museum's own name for the object and the subject is the object the museum's own text names.",
    nameRe: /\bsword|\bgladius|\bmachaira|\bblade/i,
    subject: "sword",
    queries: [
      { museum: "met", kind: "title", term: "sword", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "machaira", department: MET_DEPARTMENT },
      { museum: "artic", kind: "title", term: "Sword" },
      { museum: "artic", kind: "description", term: "sword" },
    ],
  },
  {
    kit: "scutum",
    label: "Scutum",
    note: "Shields of the period. The name is the museum's own name for the object and the subject is the object the museum's own text names.",
    nameRe: /\bshield|\bscutum|\bclipeus|\bboss/i,
    subject: "shield",
    queries: [
      { museum: "met", kind: "title", term: "shield", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "boss", department: MET_DEPARTMENT },
      { museum: "artic", kind: "title", term: "Shield" },
      { museum: "artic", kind: "description", term: "shield" },
    ],
  },
  {
    kit: "lorica",
    label: "Lorica",
    note: "Body armour of the period. The name is the museum's own name for the object and the subject is the object the museum's own text names.",
    nameRe: /\bcuirass|\blorica|\bsegmentata|\barmou?r|\bbreastplate/i,
    subject: "cuirass",
    queries: [
      { museum: "met", kind: "title", term: "cuirass", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "body armor", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "breastplate", department: MET_DEPARTMENT },
      { museum: "artic", kind: "title", term: "Cuirass" },
      { museum: "artic", kind: "description", term: "cuirass" },
    ],
  },
  {
    kit: "dolabra",
    label: "Dolabra",
    note: "The digging and chopping tools of the period. The name is the museum's own name for the object and the subject is the object the museum's own text names.",
    nameRe: /\btool|\bchisel|\baxe|\baxhead|\bax-head|\bpick|\bmattock|\bshovel|\bspade|\bhoe|\badze/i,
    subject: "tool",
    queries: [
      { museum: "met", kind: "title", term: "tool", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "chisel", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "shovel", department: MET_DEPARTMENT },
      { museum: "met", kind: "title", term: "pickaxe" },
      { museum: "artic", kind: "title", term: "Tool" },
      { museum: "artic", kind: "description", term: "tool" },
    ],
  },
];

// The world of the game sorts first: Rome, Italy, Greece and Byzantium.
// Then the rest of the ancient world. Then everything later.
const NEAR_RE = /\broman\b|\bitalic\b|\bgreek\b|\betrusc|\bbyzantine|\bgaul|\bceltic|\billyri|\bgreek or roman/i;
const ANCIENT_RE = /\bancient\b|\bhellenistic\b|\barchaic\b|\bclassical\b|\bgeometric\b|\bcypriot\b|\bcypro-|\bbronze age\b|\bhelladic\b|\bminoan\b|\bmycenaean\b|\bthrac|\bphrygian|\bscythian\b|\bgeometr|\bassyrian|\bbabylon|\bsumer|\bnear east\b|\bimperial\b/i;
// The Rome of the game: its own records come first on a board.
const ROMAN_RE = /\broman\b|\bimperial\b|\brepublican\b|\bbyzantine\b|\bgaul\b|\bgreek or roman\b/i;
// An object of the stone age is out of the period of the game.
const STONE_RE = /\bneolithic\b|\bstone age\b|\bpaleolithic\b|\bmillennium\b/i;

const ROMAN_VALUE = { I: 1, V: 5, X: 10, L: 50, C: 100 };

/** The number a roman numeral stands for. */
export function romanToInt(text) {
  const letters = String(text ?? "").toUpperCase();
  let total = 0;
  for (let i = 0; i < letters.length; i += 1) {
    const value = ROMAN_VALUE[letters[i]] ?? 0;
    const next = ROMAN_VALUE[letters[i + 1]] ?? 0;
    // A letter before a bigger one is taken away: IV is 4, VI is 6.
    total += value < next ? -value : value;
  }
  return total;
}

/** The html of a wiki page field as plain text. */
export function stripTags(html) {
  return String(html ?? "")
    .replace(/<[^>]*>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

/** A file name safe for the cache: letters, digits and dashes. */
export function slug(text) {
  return String(text ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** The url of the Commons answer with the file names of the category. */
export function commonsListUrl() {
  const params = new URLSearchParams({
    action: "query",
    list: "categorymembers",
    cmtitle: COMMONS_CATEGORY,
    cmlimit: "500",
    cmtype: "file",
    format: "json",
  });
  return `${COMMONS_API}?${params.toString()}`;
}

/** The url of the Commons answer with the imageinfo of a batch of files. */
export function commonsInfoUrl(titles) {
  const params = new URLSearchParams({
    action: "query",
    titles: titles.join("|"),
    prop: "imageinfo",
    iiprop: "url|extmetadata|mime|size",
    iiurlwidth: "800",
    format: "json",
  });
  return `${COMMONS_API}?${params.toString()}`;
}

/** The url of a Met search answer. */
export function metSearchUrl(query) {
  const params = new URLSearchParams({
    q: query.term,
    isCC0: "true",
    hasImages: "true",
    limit: String(query.limit ?? 100),
  });
  if (query.kind === "title") params.set("title", "true");
  if (query.department) params.set("departmentId", String(query.department));
  return `${MET_SEARCH_API}?${params.toString()}`;
}

/** The url of one Met object. */
export function metObjectUrl(id) {
  return `${MET_OBJECT_API}/${id}`;
}

/** The url of an Art Institute search answer. */
export function articSearchUrl(query) {
  const params = new URLSearchParams({ fields: ARTIC_FIELDS, limit: "100" });
  params.set("q", query.term);
  const field = query.kind === "description" ? "query[match][description]" : "query[match][title]";
  params.set(field, query.term);
  return `${ARTIC_SEARCH_API}?${params.toString()}`;
}

/** The cache id of a search request. */
export function requestId(museum, kit, query) {
  const museumId = museum === "artic" ? "artic" : "met";
  return `${museumId}-search-${kit}-${query.kind}-${slug(query.term)}`;
}

/** The cache id of one Met object. */
export function objectId(id) {
  return `met-object-${id}`;
}

/** Every request the run makes, in order. */
export function planRequests(boards = BOARDS) {
  const plan = [{ id: "commons-files", source: "commons", url: commonsListUrl() }];
  for (const board of boards) {
    for (const query of board.queries) {
      plan.push({
        id: requestId(query.museum, board.kit, query),
        source: query.museum === "artic" ? "artic" : "met",
        url: query.museum === "artic" ? articSearchUrl(query) : metSearchUrl(query),
      });
    }
  }
  return plan;
}

/** The rate limit and the licence of a source, from the source table. */
export function sourceRules(sourcesFile = DEFAULT_SOURCES) {
  const raw = JSON.parse(readFileSync(sourcesFile, "utf8"));
  const rules = new Map();
  for (const source of raw.sources ?? []) {
    rules.set(source.id, { rateLimitMs: source.rateLimitMs ?? 1000, licence: source.licence ?? "" });
  }
  return rules;
}

/** The plate facts of one Commons file: its number, its work and its text. */
export function parsePlateTitle(title) {
  const name = String(title ?? "")
    .replace(/^File:/, "")
    .replace(/\.(jpe?g|png|gif|tiff?)$/i, "");
  const match = /^(\d{3}) Conrad Cichorius, (.+?), Tafel ([IVXLC]+)(.*)$/.exec(name);
  if (!match) return null;
  const [, number, work, numeral, suffix] = match;
  return {
    number: Number(number),
    work,
    plate: numeral,
    plateNumber: romanToInt(numeral),
    detail: /Ausschnitt|cropped/i.test(suffix),
  };
}

/**
 * The scenes a plate page names, with the text that stands before each.
 * The text before a scene marker is the caption the page gives for it; a
 * page that names a scene without any text gives an empty caption.
 */
export function sceneSegments(description) {
  const text = stripTags(description);
  const marks = [];
  const pattern = /\((scenes?)\s+([IVXLC]+(?:\s*,\s*[IVXLC]+)*)\)/gi;
  let match;
  while ((match = pattern.exec(text)) !== null) {
    marks.push({
      index: match.index,
      end: match.index + match[0].length,
      scenes: match[2].split(",").map((s) => s.trim().toUpperCase()),
    });
  }
  const out = [];
  let from = 0;
  for (const mark of marks) {
    const caption = text.slice(from, mark.index).replace(/^\s*;\s*/, "").replace(/;\s*$/, "").trim();
    for (const scene of mark.scenes) out.push({ scene, caption });
    from = mark.end;
  }
  return out;
}

/** The plates of the Commons answers, one per file, with their images. */
export function platesFromCommons(answers) {
  const list = Array.isArray(answers) ? answers : [answers];
  const pages = [];
  for (const answer of list) {
    for (const page of Object.values(answer?.query?.pages ?? {})) pages.push(page);
  }
  const plates = [];
  for (const page of pages) {
    const info = (page.imageinfo ?? [])[0] ?? {};
    const meta = info.extmetadata ?? {};
    const parsed = parsePlateTitle(page.title);
    plates.push({
      title: page.title ?? "",
      file: parsed ? `${parsed.number} ${parsed.work}, Tafel ${parsed.plate}` : String(page.title ?? "").replace(/^File:/, ""),
      work: parsed?.work ?? "",
      plate: parsed?.plate ?? "",
      plateNumber: parsed?.plateNumber ?? 0,
      detail: parsed?.detail ?? false,
      licence: stripTags(meta.LicenseShortName?.value ?? ""),
      licenceCode: stripTags(meta.License?.value ?? ""),
      description: meta.ImageDescription?.value ?? "",
      credit: stripTags(meta.Credit?.value ?? ""),
      image: String(info.url ?? "").split("?")[0],
      thumb: String(info.thumburl ?? "").split("?")[0],
      source: String(info.descriptionurl ?? ""),
      mime: info.mime ?? "",
    });
  }
  plates.sort((a, b) => a.plateNumber - b.plateNumber || a.file.localeCompare(b.file));
  return plates;
}

/** A plate image may be used when its file page says it is public domain. */
export function isPublicDomainPlate(plate) {
  return /public domain|^pd$|^pd[- ]/i.test(`${plate.licence} ${plate.licenceCode}`);
}

/**
 * The scenes of the storyboard. A scene takes the plate that names it
 * first, then the plate with the lowest number, and the caption the plate
 * page gives for it. A scene whose plate page gives no text gets a caption
 * that states only the plate and the work.
 */
export function scenesFromPlates(plates) {
  const candidates = new Map();
  for (const plate of plates) {
    if (!isPublicDomainPlate(plate)) continue;
    const segments = sceneSegments(plate.description);
    segments.forEach((segment, index) => {
      const list = candidates.get(segment.scene) ?? [];
      list.push({ plate, caption: segment.caption, primary: index === 0 });
      candidates.set(segment.scene, list);
    });
  }
  const scenes = [];
  for (const [scene, list] of candidates) {
    // One file per plate: the whole plate before a cut of it.
    const byPlate = new Map();
    for (const candidate of list) {
      const key = candidate.plate.plateNumber;
      const held = byPlate.get(key);
      if (!held || (held.plate.detail && !candidate.plate.detail)) byPlate.set(key, candidate);
    }
    const ranked = [...byPlate.values()].sort((a, b) => {
      if (a.primary !== b.primary) return a.primary ? -1 : 1;
      const aEmpty = a.caption ? 0 : 1;
      const bEmpty = b.caption ? 0 : 1;
      if (aEmpty !== bEmpty) return aEmpty - bEmpty;
      return a.plate.plateNumber - b.plate.plateNumber;
    });
    const best = ranked[0];
    const hasCaption = Boolean(best.caption);
    scenes.push({
      number: scene,
      index: romanToInt(scene),
      caption: hasCaption ? best.caption : `Plate ${best.plate.plate} of ${best.plate.work}`,
      caption_from_source: hasCaption,
      plate: best.plate.plate,
      plate_file: best.plate.file,
      work: best.plate.work,
      credit: best.plate.credit,
      image: best.plate.image,
      thumb: best.plate.thumb,
      source: best.plate.source,
      licence: best.plate.licence,
    });
  }
  scenes.sort((a, b) => a.index - b.index);
  return scenes;
}

/** The storyboard file: the scenes with their plate, image and caption. */
export function columnFile(scenes, counts) {
  return {
    version: 1,
    generated_by: "tools/codex/boards.mjs",
    kind: "column.storyboard",
    source: {
      name: "Wikimedia Commons",
      category: COMMONS_CATEGORY,
      api: COMMONS_API,
      licence: "Public domain, as the file page states",
      work: "Conrad Cichorius, Die Reliefs der Traianssäule",
      note: "One entry per Cichorius scene the plate pages name. The image is the plate that shows the scene, the source is the file page and the caption is the text the file page states.",
    },
    counts,
    scenes,
  };
}

/** One board item out of a Met object. */
export function metItem(object, licence) {
  return {
    name: object.title ?? "",
    object_name: object.objectName ?? "",
    museum_id: "met",
    museum: object.repository ?? "Metropolitan Museum of Art",
    record_id: String(object.objectID ?? ""),
    record: object.objectURL ?? "",
    image: object.primaryImage ?? "",
    image_id: "",
    licence,
    detail: "",
    culture: object.culture ?? "",
    period: object.period ?? "",
    date: object.objectDate ?? "",
    medium: object.medium ?? "",
  };
}

/** One board item out of an Art Institute artwork. */
export function articItem(artwork, licence, config) {
  const base = config?.iiif_url ?? "https://www.artic.edu/iiif/2";
  const site = config?.website_url ?? "https://www.artic.edu";
  return {
    name: artwork.title ?? "",
    object_name: artwork.classification_title ?? "",
    museum_id: "artic",
    museum: "Art Institute of Chicago",
    record_id: String(artwork.id ?? ""),
    record: `${site}/artworks/${artwork.id ?? ""}`,
    image: artwork.image_id ? `${base}/${artwork.image_id}/full/843,/0/default.jpg` : "",
    image_id: artwork.image_id ?? "",
    licence,
    detail: stripTags(artwork.description ?? ""),
    culture: stripTags(artwork.artist_display ?? ""),
    period: "",
    date: artwork.date_display ?? "",
    medium: stripTags(artwork.medium_display ?? ""),
  };
}

/**
 * An object of the world of the game sorts before the rest. Rome comes
 * first, then the rest of the ancient world, then the stone age and the
 * later periods, which are out of the period of the game.
 */
export function periodRank(item) {
  const text = `${item.culture} ${item.period} ${item.date} ${item.object_name} ${item.name}`;
  if (STONE_RE.test(text)) return 3;
  if (ROMAN_RE.test(text)) return 0;
  if (NEAR_RE.test(text)) return 1;
  if (ANCIENT_RE.test(text)) return 2;
  return 4;
}

/**
 * A record is the object itself when the museum names it as one: the Met
 * gives the object its own class, the Art Institute gives the name. A record
 * that only shows the object (a coin that depicts a shield) is a picture of
 * it and sorts after.
 */
export function isTheObject(item, board) {
  if (item.museum_id === "met") return board.nameRe.test(item.object_name ?? "");
  if (/\bdepict|\bportray|\bcoin\b|\bmedal\b/i.test(item.name)) return false;
  return board.nameRe.test(item.name);
}

/**
 * A record is a part of the object when the museum names it as one: a
 * helmet cheek piece is a part of a helmet, a shield boss a part of a
 * shield. A part sorts after the whole object.
 */
export function isAPart(item, board) {
  const text = `${item.name} ${item.object_name}`;
  if (board.partRe && board.partRe.test(text)) return true;
  return /\bcheek[- ]piece\b|\battachment\b|\bfragment\b|\bboss\b|\bfragment\b|\bhandle\b|\bterminal\b/i.test(text);
}

/**
 * The items of one board. A record belongs when the museum's own name for
 * the object names it; a record whose own description names it belongs too
 * and sorts after. The detail text of the record is not written, only the
 * fact that the museum names the object.
 */
export function boardItems(records, board, maxItems) {
  const seen = new Set();
  const names = new Set();
  const items = [];
  for (const record of records) {
    const named = board.nameRe.test(`${record.name} ${record.object_name}`);
    const described = !named && board.nameRe.test(record.detail ?? "");
    if (!named && !described) continue;
    const key = `${record.museum_id}:${record.record_id}`;
    if (seen.has(key)) continue;
    // Two records of one museum with one name are the same object twice.
    const nameKey = `${record.museum_id}:${record.name.toLowerCase()}`;
    if (names.has(nameKey)) continue;
    seen.add(key);
    names.add(nameKey);
    items.push({
      ...record,
      subject: board.subject,
      match: named ? "name" : "description",
      kind: isTheObject(record, board) ? "object" : "depiction",
      part: isAPart(record, board),
    });
  }
  items.sort((a, b) => {
    // The object itself comes first: a board is a reference of the object,
    // so a bronze helmet beats a coin that shows a helmeted head.
    if (a.kind !== b.kind) return a.kind === "object" ? -1 : 1;
    if (a.part !== b.part) return a.part ? 1 : -1;
    const rank = periodRank(a) - periodRank(b);
    if (rank) return rank;
    if (a.match !== b.match) return a.match === "name" ? -1 : 1;
    const left = Number(a.record_id);
    const right = Number(b.record_id);
    if (Number.isFinite(left) && Number.isFinite(right) && left !== right) return left - right;
    if (a.record_id !== b.record_id) return a.record_id < b.record_id ? -1 : 1;
    return a.museum_id < b.museum_id ? -1 : 1;
  });
  return items.slice(0, maxItems);
}

/** The board file: the items with their record, image and licence. */
export function boardFile(board, items, queries) {
  const counts = { items: items.length };
  for (const item of items) counts[item.museum_id] = (counts[item.museum_id] ?? 0) + 1;
  return {
    version: 1,
    generated_by: "tools/codex/boards.mjs",
    kind: "reference.board",
    kit: board.kit,
    label: board.label,
    note: board.note,
    queries,
    counts,
    items: items.map((item) => ({
      name: item.name,
      object_name: item.object_name,
      subject: item.subject,
      match: item.match,
      kind: item.kind,
      part: item.part,
      museum_id: item.museum_id,
      museum: item.museum,
      record_id: item.record_id,
      record: item.record,
      image: item.image,
      image_id: item.image_id,
      licence: item.licence,
      culture: item.culture,
      period: item.period,
      date: item.date,
      medium: item.medium,
    })),
  };
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

/** The answer of one request: the cache first, the network after that. */
export async function requestAnswer({ id, url, source, rules, cacheDir, offline, fetchImpl, log }) {
  const body = join(cacheDir, `${id}.json`);
  const sidecar = `${body}.src.json`;
  if (existsSync(body)) {
    log(`CACHE ${id} ${url}`);
    return readFileSync(body, "utf8");
  }
  if (offline) throw new Error(`no cached answer for ${id} (${url})`);
  const rule = rules.get(source);
  if (!rule) throw new Error(`the source table has no source ${source}`);
  const res = await fetchImpl(url, { headers: { "User-Agent": UA } });
  // The search index of a museum can name a record the museum no longer
  // serves. Such a record is left out, not a failure of the source.
  if (res.status === 404 && id.startsWith("met-object-")) {
    log(`GONE ${id} ${url}`);
    return null;
  }
  if (!res.ok) throw new Error(`GET ${url} -> HTTP ${res.status}`);
  const text = await res.text();
  mkdirSync(cacheDir, { recursive: true });
  writeFileSync(body, text, "utf8");
  writeFileSync(
    sidecar,
    `${JSON.stringify(
      {
        source,
        url,
        fetchedAt: new Date().toISOString(),
        sha256: createHash("sha256").update(text).digest("hex"),
        licence: rule.licence,
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  log(`OK ${id} ${text.length}B ${url}`);
  return text;
}

/** Write every file or none: each file lands whole, and the last rename wins. */
export function writeAll(files) {
  const temps = [];
  try {
    for (const file of files) {
      mkdirSync(dirname(file.path), { recursive: true });
      const temp = `${file.path}.tmp`;
      writeFileSync(temp, file.text, "utf8");
      temps.push({ temp, path: file.path });
    }
    for (const entry of temps) renameSync(entry.temp, entry.path);
  } catch (error) {
    for (const entry of temps) {
      try {
        renameSync(entry.temp, entry.path);
      } catch {}
    }
    throw error;
  }
}

/** Build the storyboard and the boards and write them. */
export async function run({
  sourcesFile = DEFAULT_SOURCES,
  cacheDir = DEFAULT_CACHE,
  outDir = DEFAULT_OUT,
  boards = BOARDS,
  maxItems = 12,
  maxCandidates = 60,
  offline = false,
  dryRun = false,
  fetchImpl = fetch,
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  const rules = sourceRules(sourcesFile);
  const plan = planRequests(boards);
  log(`PLAN requests=${plan.length} boards=${boards.length} max_items=${maxItems}`);
  for (const request of plan) log(`PLAN ${request.id} ${request.url}`);

  const lastRequest = new Map();
  const get = async (request) => {
    const rule = rules.get(request.source);
    const wait = rule?.rateLimitMs ?? 0;
    const previous = lastRequest.get(request.source);
    if (previous !== undefined) {
      const gap = previous + wait - Date.now();
      if (gap > 0) await sleep(gap);
    }
    lastRequest.set(request.source, Date.now());
    return requestAnswer({ ...request, rules, cacheDir, offline, fetchImpl, log });
  };

  // The plates of the column: the file names of the category, then the
  // imageinfo of those files in batches.
  let fileTitles = [];
  try {
    const listText = await get(plan[0]);
    fileTitles = (JSON.parse(listText).query?.categorymembers ?? []).map((member) => member.title);
  } catch (error) {
    log(`FAIL ${plan[0].id} ${error.message}`);
    log("RESULT refused: a source failed");
    return { ok: false, code: 1, scenes: 0, boards: 0, items: 0 };
  }
  const answers = [];
  for (let i = 0; i < fileTitles.length; i += COMMONS_BATCH) {
    const batch = fileTitles.slice(i, i + COMMONS_BATCH);
    const id = `commons-plates-${Math.floor(i / COMMONS_BATCH) + 1}`;
    let text;
    try {
      text = await get({ id, source: "commons", url: commonsInfoUrl(batch) });
    } catch (error) {
      log(`FAIL ${id} ${error.message}`);
      log("RESULT refused: a source failed");
      return { ok: false, code: 1, scenes: 0, boards: 0, items: 0 };
    }
    answers.push(JSON.parse(text));
  }
  const plates = platesFromCommons(answers);
  const scenes = scenesFromPlates(plates);
  log(`SCENES ${scenes.length} plates=${plates.length}`);

  const boardFiles = [];
  for (const board of boards) {
    const records = [];
    const queries = [];
    for (const query of board.queries) {
      const request = plan.find((entry) => entry.id === requestId(query.museum, board.kit, query));
      let text;
      try {
        text = await get(request);
      } catch (error) {
        log(`FAIL ${request.id} ${error.message}`);
        log("RESULT refused: a source failed");
        return { ok: false, code: 1, scenes: scenes.length, boards: boardFiles.length, items: 0 };
      }
      queries.push({ museum: query.museum, term: query.term, match: query.kind });
      const answer = JSON.parse(text);
      if (query.museum === "met") {
        const cap = Math.min(query.candidates ?? maxCandidates, maxCandidates);
        const ids = (answer.objectIDs ?? []).slice(0, cap);
        for (const id of ids) {
          let objectText;
          try {
            objectText = await get({ id: objectId(id), source: "met", url: metObjectUrl(id) });
          } catch (error) {
            log(`FAIL ${objectId(id)} ${error.message}`);
            log("RESULT refused: a source failed");
            return { ok: false, code: 1, scenes: scenes.length, boards: boardFiles.length, items: 0 };
          }
          // The search index can name a record the museum no longer serves.
          if (!objectText) continue;
          const object = JSON.parse(objectText);
          if (!object.isPublicDomain || !object.primaryImage) continue;
          records.push(metItem(object, rules.get("met")?.licence ?? "CC0"));
        }
      } else {
        const config = answer.config ?? {};
        for (const artwork of answer.data ?? []) {
          if (!artwork.is_public_domain || !artwork.image_id) continue;
          if (artwork.department_title !== ARTIC_DEPARTMENT) continue;
          records.push(articItem(artwork, rules.get("artic")?.licence ?? "CC0", config));
        }
      }
    }
    const items = boardItems(records, board, maxItems);
    boardFiles.push(boardFile(board, items, queries));
    log(`BOARD ${board.kit} items=${items.length}`);
  }

  // Nothing is written that is not fit: every scene needs an image and a
  // source, every board needs an item, every item needs a record.
  const problems = [];
  for (const scene of scenes) {
    if (!scene.image) problems.push(`scene ${scene.number} has no image`);
    if (!scene.source) problems.push(`scene ${scene.number} has no source url`);
    if (!scene.licence) problems.push(`scene ${scene.number} has no licence`);
  }
  for (const board of boardFiles) {
    if (!board.items.length) problems.push(`the board ${board.kit} has no item`);
    for (const item of board.items) {
      if (!item.image) problems.push(`${board.kit}: ${item.record_id} has no image`);
      if (!item.record) problems.push(`${board.kit}: ${item.record_id} has no record url`);
      if (!item.licence) problems.push(`${board.kit}: ${item.record_id} has no licence`);
    }
  }
  if (problems.length) {
    for (const problem of problems) log(`PROBLEM ${problem}`);
    log("RESULT refused: the data is not fit to write");
    return { ok: false, code: 1, scenes: scenes.length, boards: boardFiles.length, items: 0 };
  }

  const files = [
    {
      path: join(outDir, "column.json"),
      text: `${JSON.stringify(columnFile(scenes, { scenes: scenes.length, plates: plates.length }), null, 2)}\n`,
    },
  ];
  for (const board of boardFiles) {
    files.push({
      path: join(outDir, "boards", `${board.kit}.json`),
      text: `${JSON.stringify(board, null, 2)}\n`,
    });
  }
  if (dryRun) {
    for (const file of files) log(`WOULD WRITE ${file.path}`);
  } else {
    writeAll(files);
    for (const file of files) log(`WROTE ${file.path}`);
  }
  const items = boardFiles.reduce((n, board) => n + board.items.length, 0);
  log(`RESULT boards=${boardFiles.length} scenes=${scenes.length} items=${items} files=${files.length}`);
  return { ok: true, code: 0, scenes: scenes.length, boards: boardFiles.length, items, files: files.map((f) => f.path) };
}

export const HELP = `Usage: node tools/codex/boards.mjs [--sources FILE] [--cache DIR] [--out DIR]
                              [--boards LIST] [--max-items N] [--max-candidates N]
                              [--offline] [--dry-run] [--help]

Builds the Trajan's Column storyboard (content/rome/column.json) and the
reference boards (content/rome/boards/<kit>.json) from the museum and wiki
records the codex names. Every answer is cached under content/local/codex/
with a sidecar (url, fetchedAt, sha256, licence). The run writes every file
or none.

  --sources FILE       the source table with the rate limits (tools/codex/sources.json)
  --cache DIR          where the answers are cached (content/local/codex/boards)
  --out DIR            where the data goes (content/rome)
  --boards LIST        only these kits, comma separated
  --max-items N        items per board (12)
  --max-candidates N   objects to look at per search (60)
  --offline            use only the cached answers, never the network
  --dry-run            print the plan and the files, write nothing
  --help               print this help

Exit 0 ok, 1 a source failed or the data is not fit to write, 2 bad CLI.
`;

function parseArgs(args) {
  const out = {
    sourcesFile: DEFAULT_SOURCES,
    cacheDir: DEFAULT_CACHE,
    outDir: DEFAULT_OUT,
    kits: [],
    maxItems: 12,
    maxCandidates: 60,
    offline: false,
    dryRun: false,
    help: false,
  };
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
      continue;
    }
    if (arg === "--offline") {
      out.offline = true;
      continue;
    }
    if (arg === "--dry-run") {
      out.dryRun = true;
      continue;
    }
    if (["--sources", "--cache", "--out", "--boards", "--max-items", "--max-candidates"].includes(arg)) {
      const value = args[i + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} needs a value`);
      if (arg === "--sources") out.sourcesFile = resolve(value);
      else if (arg === "--cache") out.cacheDir = resolve(value);
      else if (arg === "--out") out.outDir = resolve(value);
      else if (arg === "--boards") out.kits = value.split(",").map((s) => s.trim()).filter(Boolean);
      else if (arg === "--max-items") out.maxItems = Number(value);
      else out.maxCandidates = Number(value);
      i += 1;
      continue;
    }
    const named = arg.match(/^(--[a-z-]+)=(.*)$/);
    if (named) {
      args.splice(i, 1, named[1], named[2]);
      i -= 1;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  if (!Number.isInteger(out.maxItems) || out.maxItems <= 0) throw new Error("--max-items must be a positive whole number");
  if (!Number.isInteger(out.maxCandidates) || out.maxCandidates < 0) throw new Error("--max-candidates must be a whole number");
  return out;
}

if (import.meta.main) {
  try {
    const opts = parseArgs(process.argv.slice(2));
    if (opts.help) {
      process.stdout.write(HELP);
    } else {
      const boards = opts.kits.length ? BOARDS.filter((board) => opts.kits.includes(board.kit)) : BOARDS;
      if (!boards.length) throw new Error(`no board with the name ${opts.kits.join(", ")}`);
      const result = await run({ ...opts, boards });
      process.exitCode = result.code;
    }
  } catch (error) {
    process.stderr.write(`boards: error: ${error.message}\n`);
    process.exitCode = 2;
  }
}
