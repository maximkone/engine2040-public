#!/usr/bin/env node
// index.mjs - the Rome codex passage index (CDX2).
//
// Job: cut the fetched codex texts into passages by book, chapter and
// section, tag every passage with the topics it holds and the numbers with
// units it states, and write `content/local/codex/index.jsonl`: one JSON
// line per passage. The index is what the door searches (CDX7) and what the
// tech tables are cut from (CDX4).
//
// The text comes from the fetch tool (CDX1): `content/local/codex/<source>/`
// holds the files, each with a `<file>.src.json` sidecar (url, fetchedAt,
// sha256, licence). The sidecar fields go into the index, so a cite can
// name the url and the licence.
//
// Usage:
//   node tools/codex/index.mjs [--in DIR] [--out FILE] [--source ID] [--help]
//
// Rules:
//   - the works table gives every fetched file its work key, title and
//     author; a file with no entry is indexed under its source id and
//     reported, never guessed.
//   - a passage ref is `<work>.<book>.<chapter>.<section>` with the parts
//     the text names: `caesar.bg.4.17`, `vegetius.3.8`.
//   - the topics are the nine of the plan: march, camp, tool, bridge, road,
//     siege, craft, place, year.
//   - a number is kept only with a unit it states: `5 ft`, `3 mi`, `43 BC`.
//   - the index stays local: content/local/ is gitignored.
//
// Exit: 0 ok, 1 nothing to index, 2 bad CLI.
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_IN = join(REPO, "content", "local", "codex");
export const DEFAULT_OUT = join(DEFAULT_IN, "index.jsonl");

// The nine topics of the plan, in the order the index lists them.
export const TOPICS = ["march", "camp", "tool", "bridge", "road", "siege", "craft", "place", "year"];

// The words a topic is spotted by. Latin and English both, because half the
// codex is Latin. A word must stand on its own: "ram" never fires on
// "rampart".
const TOPIC_WORDS = {
  march: [
    "march", "marches", "marching", "marcher", "marchers", "marched", "iter", "agmen",
    "legion", "legions", "legionary", "legionaries", "infantry", "army", "armies",
    "exercitus", "pacing", "paced", "column of men", "days march", "days of march",
  ],
  camp: [
    "camp", "camps", "castra", "castrorum", "castrum", "ditch", "ditches", "fossa",
    "fossae", "trench", "trenches", "rampart", "vallum", "palisade", "tent", "tents",
    "praesidium", "garrison", "encamp", "quarters",
  ],
  tool: [
    "tool", "tools", "axe", "axes", "hammer", "hammers", "pick", "pickaxe", "spade",
    "shovel", "sickle", "plough", "plow", "rake", "saw", "chisel", "ladder", "torch",
    "rope", "ropes", "dolabra", "falx", "sarculum", "ascia", "bipennis", "vanga",
    "instrument", "instruments",
  ],
  bridge: ["bridge", "bridges", "pontem", "pons", "ponte", "pontoon", "trestle", "bridgework"],
  road: [
    "road", "roads", "viae", "strata", "stratae", "path", "paths", "highway",
    "roadway", "route", "journey", "roadside", "milestone",
  ],
  siege: [
    "siege", "sieges", "besiege", "besieged", "besieging", "obsidio", "obsidion",
    "oppugnatio", "assault", "wall", "walls", "murus", "muri", "testudo", "battering ram",
    "belfry", "vineae", "mattio", "turret", "towers", "scale", "scaling", "storm",
    "stormed", "storming", "defend", "defence", "defense",
  ],
  craft: [
    "craft", "crafts", "ship", "ships", "boat", "boats", "navis", "naves", "shipbuilding",
    "carpenter", "carpenters", "artisan", "artisans", "workshop", "wright", "timber",
    "plank", "planks", "forge", "smith", "wheel", "potter", "weaver", "baker", "rope-making",
  ],
  place: [
    "place", "places", "city", "cities", "town", "towns", "fort", "forts", "fortress",
    "vicus", "oppidum", "roma", "romae", "gallia", "gaul", "italia", "italy", "hispania",
    "spain", "aegyptus", "egypt", "syria", "britannia", "britain", "dacia", "raetia",
    "noricum", "pannonia", "macedonia", "africa", "averni", "genava", "bonna",
    "mogontiacum", "drobeta", "sarmizegetusa", "alexandria", "carthage", "capua",
  ],
  year: [
    "year", "years", "annum", "annis", "annos", "consul", "consuls", "consulship",
    "consulate", "consular",
  ],
};

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const TOPIC_RE = Object.fromEntries(
  TOPICS.map((topic) => [
    topic,
    new RegExp(`\\b(?:${TOPIC_WORDS[topic].map(escapeRe).join("|")})\\b`, "i"),
  ]),
);

// The units a number may carry. The unit is the word, the row names them.
const UNIT_WORDS = {
  ft: ["ft", "fts", "feet", "foot", "pes", "pedes", "pedibus", "pedem"],
  in: ["in", "inch", "inches", "pollex", "pollicis", "digitos", "digitus"],
  yd: ["yd", "yds", "yard", "yards"],
  mi: ["mi", "mile", "miles", "milia", "millia", "miliaria", "miliaria"],
  m: ["m", "metre", "metres", "meter", "meters", "metra"],
  cm: ["cm", "centimetre", "centimetres", "centimeter", "centimeters"],
  km: ["km", "kilometre", "kilometres", "kilometer", "kilometers"],
  passus: ["passus"],
  cubit: ["cubit", "cubits", "cubitus", "cubita"],
  men: ["men", "man", "soldier", "soldiers", "milites", "militum"],
  horses: ["horse", "horses", "equus", "equi", "equorum"],
  year: ["year", "years", "annum", "annis", "annos"],
};
const UNIT_OF = new Map();
for (const [unit, words] of Object.entries(UNIT_WORDS)) {
  for (const word of words) UNIT_OF.set(word, unit);
}
// The year words that only count when they are written in caps: "ad" is a
// Latin preposition and would tag every Latin passage.
const YEAR_CAPS = new Map([
  ["BC", "year"],
  ["BCE", "year"],
  ["AD", "year"],
  ["A.D.", "year"],
]);

// The number words, English and Latin, so "quinque pedes" is 5 ft too.
const NUMBER_WORDS = new Map(Object.entries({
  one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9,
  ten: 10, eleven: 11, twelve: 12, thirteen: 13, fourteen: 14, fifteen: 15,
  sixteen: 16, seventeen: 17, eighteen: 18, nineteen: 19, twenty: 20, thirty: 30,
  forty: 40, fifty: 50, sixty: 60, seventy: 70, eighty: 80, ninety: 90, hundred: 100,
  unus: 1, duo: 2, tres: 3, quattuor: 4, quinque: 5, sex: 6, septem: 7, octo: 8,
  novem: 9, decem: 10, undecim: 11, duodecim: 12, viginti: 20, triginta: 30,
  quadraginta: 40, quinquaginta: 50, sexaginta: 60, septuaginta: 70, octoginta: 80,
  nonaginta: 90, centum: 100, ducenti: 200, trecenti: 300, quadringenti: 400,
  quingenti: 500, sescenti: 600, septingenti: 700, octingenti: 800, nongenti: 900,
  mille: 1000,
}));

const ROMAN = { I: 1, V: 5, X: 10, L: 50, C: 100, D: 500, M: 1000 };

/** A roman numeral as a number: "IV" is 4, "XVII" is 17. Zero if it is not
 * one. */
export function romanToInt(text) {
  const s = String(text ?? "").toUpperCase().replace(/[^IVXLCDM]/g, "");
  if (!s) return 0;
  let n = 0;
  for (let i = 0; i < s.length; i += 1) {
    const v = ROMAN[s[i]] ?? 0;
    const next = ROMAN[s[i + 1]] ?? 0;
    n += v < next ? -v : v;
  }
  return n;
}

// The heading shapes the fetched texts use.
const BOOK_RE = /^\s*BOOK\s+([IVXLCDM]+|\d+)\b/i;
const CHAPTER_RE = /^\s*CHAPTER\s+([IVXLCDM]+|\d+)\b/i;
const SECTION_RE = /^\s*SECTION\s+([IVXLCDM]+|\d+)\b/i;
// "I.--All Gaul is divided in three parts": the chapter number and its text
// on one line, the way the Caesar text carries it.
const INLINE_CHAPTER_RE = /^\s*([IVXLCDM]{1,7})\.\s*[-–—]{1,2}\s*(\S.*)$/;
// "17.3 the country": a chapter and a section in one number.
const NUMBERED_RE = /^\s*(\d{1,3})\.(\d{1,3})(?:[\s.]|$)/;

const headNumber = (text) => {
  const s = String(text ?? "").trim();
  if (/^\d+$/.test(s)) return Number(s);
  return romanToInt(s);
};

/** The topics a passage holds, in the order of TOPICS. */
export function tagTopics(text) {
  const body = String(text ?? "");
  const found = [];
  for (const topic of TOPICS) {
    if (TOPIC_RE[topic].test(body)) found.push(topic);
  }
  return found;
}

/** The numbers with units a passage holds: `5 ft`, `3 mi`, `43 BC`. A number
 * with no unit word is not a measure and is left out. */
export function findNumbers(text) {
  const body = String(text ?? "");
  const out = [];
  const seen = new Set();
  const push = (value, unit, raw) => {
    const key = `${value} ${unit}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({ value, unit, text: raw });
  };
  // A digit and the unit word after it.
  for (const m of body.matchAll(/(\d+(?:[.,]\d+)?)\s*([A-Za-z][A-Za-z.]*)/g)) {
    const value = Number(m[1].replace(/,/g, ""));
    const word = m[2];
    const bare = word.replace(/\.+$/, "");
    const unit = UNIT_OF.get(bare.toLowerCase()) ?? YEAR_CAPS.get(word) ?? YEAR_CAPS.get(bare);
    if (!unit) continue;
    push(value, unit, `${m[1]} ${word}`);
  }
  // A number word and the unit word after it: "five feet", "quinque pedes".
  for (const m of body.matchAll(
    /\b(one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred|unus|duo|tres|quattuor|quinque|sex|septem|octo|novem|decem|undecim|duodecim|viginti|triginta|quadraginta|quinquaginta|sexaginta|septuaginta|octoginta|nonaginta|centum|ducenti|trecenti|quadringenti|quingenti|sescenti|septingenti|octingenti|nongenti|mille)\s+([A-Za-z][A-Za-z.]*)/gi,
  )) {
    const value = NUMBER_WORDS.get(m[1].toLowerCase());
    const word = m[2];
    const bare = word.replace(/\.+$/, "");
    const unit = UNIT_OF.get(bare.toLowerCase());
    if (!value || !unit) continue;
    push(value, unit, `${m[1]} ${word}`);
  }
  return out;
}

/** The ref of a passage: `caesar.bg.4.17`. The parts the text does not name
 * are left out. */
export function refFor(work, book, chapter, section) {
  const parts = [work];
  if (book > 0) parts.push(String(book));
  if (chapter > 0) parts.push(String(chapter));
  if (section > 0) parts.push(String(section));
  return parts.join(".");
}

/** Cut a plain text into passages by book, chapter and section. The work
 * entry names the part headings ("THE WAR IN GAUL") that switch the work
 * key. */
export function cutText(text, entry = {}) {
  const lines = String(text ?? "").split(/\r?\n/);
  const parts = entry.parts ?? [];
  const out = [];
  let book = 0;
  let chapter = 0;
  let section = 0;
  let part = "";
  let buf = [];
  const flush = () => {
    const body = buf.join("\n").trim();
    buf = [];
    if (body) out.push({ book, chapter, section, part, text: body });
  };
  for (const raw of lines) {
    const line = raw.trim();
    if (!line) continue;
    const partHeading = parts.find((p) => p.heading.toUpperCase() === line.toUpperCase());
    if (partHeading && line.length < 40) {
      flush();
      part = partHeading.key;
      book = 0;
      chapter = 0;
      section = 0;
      continue;
    }
    const bookMatch = line.length < 40 ? line.match(BOOK_RE) : null;
    if (bookMatch) {
      flush();
      book = headNumber(bookMatch[1]);
      chapter = 0;
      section = 0;
      continue;
    }
    const chapterMatch = line.length < 40 ? line.match(CHAPTER_RE) : null;
    if (chapterMatch) {
      flush();
      chapter = headNumber(chapterMatch[1]);
      section = 0;
      continue;
    }
    const sectionMatch = line.length < 40 ? line.match(SECTION_RE) : null;
    if (sectionMatch) {
      flush();
      section = headNumber(sectionMatch[1]);
      continue;
    }
    const numbered = line.match(NUMBERED_RE);
    if (numbered) {
      flush();
      chapter = Number(numbered[1]);
      section = Number(numbered[2]);
      continue;
    }
    const inline = line.match(INLINE_CHAPTER_RE);
    if (inline) {
      flush();
      chapter = romanToInt(inline[1]);
      section = 0;
      buf.push(inline[2]);
      continue;
    }
    buf.push(line);
  }
  flush();
  return out;
}

/** Cut a TEI xml file into passages. The book and the chapter numbers are in
 * the divs themselves, so no heading guess is needed. */
export function cutTei(text) {
  const src = String(text ?? "");
  const out = [];
  const stack = [];
  let book = 0;
  let open = null;
  const divRe = /<(\/?)div\b([^>]*)/g;
  for (const m of src.matchAll(divRe)) {
    const closing = m[1] === "/";
    const attrs = m[2] ?? "";
    if (!closing && attrs.trimEnd().endsWith("/")) continue;
    const type = attrs.match(/subtype="([^"]+)"/)?.[1] ?? "";
    const n = Number(attrs.match(/\bn="(\d+)"/)?.[1] ?? 0);
    if (!closing) {
      stack.push({ type, n });
      if (type === "book") book = n;
      if (type === "chapter") open = { book, chapter: n, start: m.index + m[0].length };
      continue;
    }
    const top = stack.pop();
    if (top?.type === "chapter" && open) {
      const body = stripTags(src.slice(open.start, m.index));
      if (body) {
        out.push({ book: open.book, chapter: open.chapter, section: 0, part: "", text: body });
      }
      open = null;
    }
  }
  return out;
}

// The blocks that are the page's own furniture: the navigation, the header
// and the footer. Their text is not the article, so it does not go in the
// index.
const NAV_CLASSES = new Set([
  "header",
  "footer",
  "ucfooter",
  "help_bar",
  "help",
  "contents",
  "nav",
  "navigation",
  "sidebar",
  "copyright",
]);
const VOID_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);

/** The html without the navigation and the footer blocks. The blocks are
 * found by their class, with the nesting counted so an inner block does not
 * end them early. */
export function dropNavBlocks(html) {
  const src = String(html ?? "");
  const re = /<(\/?)([a-z][a-z0-9]*)\b([^>]*?)(\/?)>/gi;
  const stack = [];
  const cuts = [];
  for (const m of src.matchAll(re)) {
    const closing = m[1] === "/";
    const tag = m[2].toLowerCase();
    if (VOID_TAGS.has(tag)) continue;
    const attrs = m[3] ?? "";
    if (!closing) {
      const cls = (attrs.match(/class="([^"]*)"/i)?.[1] ?? "").toLowerCase();
      const nav = cls.split(/\s+/).some((c) => NAV_CLASSES.has(c));
      stack.push({ tag, nav, start: m.index });
      continue;
    }
    let at = -1;
    for (let i = stack.length - 1; i >= 0; i -= 1) {
      if (stack[i].tag === tag) {
        at = i;
        break;
      }
    }
    if (at < 0) continue;
    for (const open of stack.splice(at)) {
      if (open.nav) cuts.push([open.start, m.index + m[0].length]);
    }
  }
  if (!cuts.length) return src;
  cuts.sort((a, b) => a[0] - b[0]);
  let out = "";
  let at = 0;
  for (const [start, end] of cuts) {
    if (start < at) continue;
    out += src.slice(at, start);
    at = end;
  }
  out += src.slice(at);
  return out;
}

/** The text of an html entry: the article paragraphs, without the comments,
 * the scripts, the navigation and the footer. A dictionary page is one
 * article, so the paragraphs are the passage. */
export function htmlText(html) {
  let src = dropNavBlocks(String(html ?? ""));
  src = src.replace(/<!--[\s\S]*?-->/g, " ");
  src = src.replace(/<script[\s\S]*?<\/script>/gi, " ");
  src = src.replace(/<style[\s\S]*?<\/style>/gi, " ");
  const bodyAt = src.search(/<body\b/i);
  const body = bodyAt >= 0 ? src.slice(bodyAt) : src;
  const paras = [];
  for (const m of body.matchAll(/<p\b([^>]*)>([\s\S]*?)<\/p>/gi)) {
    const cls = (m[1].match(/class="([^"]*)"/i)?.[1] ?? "").toLowerCase();
    if (cls.includes("pagenum")) continue;
    const text = stripTags(m[2]);
    if (text) paras.push(text);
  }
  return paras.join("\n\n");
}

/** The text of a passage out of xml: no tags, no entities, one space between
 * the words. */
export function stripTags(xml) {
  return String(xml ?? "")
    .replace(/<[^>]*>/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/\s+/g, " ")
    .trim();
}

// The works table: every fetched file, its work key, its title and its
// author. The key is what the refs are built from.
export const WORKS = {
  "gutenberg/caesar-bellum-gallicum.txt": {
    work: "caesar",
    title: "De Bello Gallico and Other Commentaries",
    author: "Julius Caesar",
    parts: [
      { key: "bg", heading: "THE WAR IN GAUL" },
      { key: "civil", heading: "THE CIVIL WAR" },
    ],
  },
  "gutenberg/vitruvius-de-architectura.txt": {
    work: "vitruvius",
    title: "The Ten Books on Architecture",
    author: "Vitruvius Pollio",
  },
  "gutenberg/polybius-histories-vol1.txt": {
    work: "polybius",
    title: "The Histories of Polybius, Vol. 1 (of 2)",
    author: "Polybius",
  },
  "gutenberg/polybius-histories-vol2.txt": {
    work: "polybius",
    title: "The Histories of Polybius, Vol. 2 (of 2)",
    author: "Polybius",
  },
  "gutenberg/tacitus-germania.txt": {
    work: "tacitus.germania",
    title: "Tacitus on Germany",
    author: "Cornelius Tacitus",
  },
  "gutenberg/tacitus-histories.txt": {
    work: "tacitus.histories",
    title: "Tacitus: The Histories, Volumes I and II",
    author: "Cornelius Tacitus",
  },
  "gutenberg/suetonius-twelve-caesars.txt": {
    work: "suetonius.caesars",
    title: "The Lives of the Twelve Caesars, Complete",
    author: "Suetonius",
  },
  // Vegetius is not in the fetch table yet: the plan does not list a source
  // for him. The entry is here so the day a source lands the file, the
  // index cuts it and the refs read vegetius.3.8.
  "gutenberg/vegetius-de-re-militari.txt": {
    work: "vegetius",
    title: "De Re Militari",
    author: "Vegetius",
  },
  "perseus/phi0448.phi001.perseus-lat2.xml": {
    work: "caesar.bg",
    title: "Caesar, Bellum Gallicum (Latin)",
    author: "Julius Caesar",
    format: "tei",
  },
  "perseus/phi0448.phi001.perseus-eng2.xml": {
    work: "caesar.bg",
    title: "Caesar, Bellum Gallicum (English)",
    author: "Julius Caesar",
    format: "tei",
  },
};

// The dictionary entries of Smith's Dictionary of Greek and Roman
// Antiquities (1875). One entry per page, one passage per page.
const SMITHS = ["ara", "cloaca", "gladius", "scutum", "castra", "pons", "testudo"];
for (const head of SMITHS) {
  WORKS[`smiths/${head}.html`] = {
    work: `smiths.${head}`,
    title: head.charAt(0).toUpperCase() + head.slice(1),
    author: "Smith's Dictionary of Greek and Roman Antiquities (1875)",
    format: "html",
  };
}

/** The passages of one text, each with its ref, its topics and its numbers. */
export function indexText({ text, entry, source = "", file = "", url = "", licence = "" }) {
  const work = entry ?? {};
  const format = work.format ?? "text";
  const body = format === "tei" ? text : format === "html" ? htmlText(text) : text;
  const passages = format === "tei" ? cutTei(text) : cutText(body, work);
  const records = [];
  passages.forEach((p, i) => {
    const key = p.part ? `${work.work}.${p.part}` : work.work;
    const hasNumber = p.book > 0 || p.chapter > 0;
    // A file with one passage only is the work itself: `smiths.pons`. A
    // file with many keeps the part: `caesar.p1`.
    const ref = hasNumber
      ? refFor(key, p.book, p.chapter, p.section)
      : passages.length === 1
        ? key
        : `${key}.p${i + 1}`;
    const topics = new Set(tagTopics(p.text));
    const numbers = findNumbers(p.text);
    if (numbers.some((n) => n.unit === "year")) topics.add("year");
    const ordered = TOPICS.filter((t) => topics.has(t));
    records.push({
      ref,
      source,
      file,
      work: key,
      book: p.book,
      chapter: p.chapter,
      section: p.section,
      title: work.title ?? "",
      author: work.author ?? "",
      url,
      licence,
      topics: ordered,
      numbers,
      chars: p.text.length,
      text: p.text,
    });
  });
  return records;
}

const TEXT_EXT = new Set([".txt", ".xml", ".html", ".htm"]);

/** The sidecar of a fetched file: url, time, sha256 and licence. */
export function readSidecar(file) {
  const path = `${file}.src.json`;
  if (!existsSync(path)) return {};
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return {};
  }
}

/** Cut every fetched text under `inDir` and write the index. */
export function run({
  inDir = DEFAULT_IN,
  outFile = DEFAULT_OUT,
  only = "",
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  if (!existsSync(inDir)) {
    log(`INDEX nothing to index: ${inDir} does not exist`);
    return { records: [], files: 0, sources: 0 };
  }
  const sources = readdirSync(inDir, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .filter((name) => (only ? name === only : true))
    .sort();
  const records = [];
  let files = 0;
  for (const source of sources) {
    const dir = join(inDir, source);
    const names = readdirSync(dir)
      .filter((name) => TEXT_EXT.has(name.slice(name.lastIndexOf(".")).toLowerCase()))
      .sort();
    for (const name of names) {
      const file = join(dir, name);
      const key = `${source}/${name}`;
      const entry = WORKS[key];
      if (!entry) {
        log(`INDEX no work entry for ${key}: indexed under the source id`);
      }
      const sidecar = readSidecar(file);
      const text = readFileSync(file, "utf8");
      const made = indexText({
        text,
        entry: entry ?? { work: source, title: name },
        source,
        file: name,
        url: sidecar.url ?? "",
        licence: sidecar.licence ?? "",
      });
      files += 1;
      records.push(...made);
      log(`INDEX ${key} passages=${made.length}`);
    }
  }
  mkdirSync(dirname(outFile), { recursive: true });
  writeFileSync(outFile, records.map((r) => JSON.stringify(r)).join("\n") + (records.length ? "\n" : ""));
  log(`INDEX wrote ${records.length} passage(s) to ${outFile}`);
  return { records, files, sources: sources.length };
}

export const HELP = `Usage: node tools/codex/index.mjs [--in DIR] [--out FILE] [--source ID] [--help]

Cuts the fetched codex texts into passages by book, chapter and section, tags
each passage with the topics it holds and the numbers with units it states,
and writes the index as jsonl (one line per passage). --in is the folder the
fetch tool wrote (content/local/codex), --out the index file, --source one
source folder only. Exit 0 ok, 1 nothing to index, 2 bad CLI.
`;

function opt(args, name) {
  const i = args.findIndex((a) => a === name || a.startsWith(`${name}=`));
  if (i < 0) return "";
  if (args[i].includes("=")) return args[i].slice(name.length + 1);
  return args[i + 1] ?? "";
}

const isMain = import.meta.main;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(HELP);
  } else {
    const inDir = opt(args, "--in") || DEFAULT_IN;
    const outFile = opt(args, "--out") || DEFAULT_OUT;
    const only = opt(args, "--source");
    if (!existsSync(inDir)) {
      process.stderr.write(`codex: error: ${inDir} does not exist\n`);
      process.exit(2);
    }
    const r = run({ inDir, outFile, only });
    if (r.files === 0) process.exitCode = 1;
  }
}
