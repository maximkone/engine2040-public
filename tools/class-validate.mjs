// Class-file content gate: one JSON Schema plus the rules a schema cannot express.
//   node tools/class-validate.mjs                  check every content/classes/*.json (roster.json and schema.json are skipped)
//   node tools/class-validate.mjs --classes DIR    check another directory that holds the same schema.json layout
//   node tools/class-validate.mjs --help           print this
// Validates each class file against content/classes/schema.json with the small
// JSON Schema subset below (no ajv, no dependencies, no network, no cargo), then
// runs the cross-file rules: branch targets, faction counts, global ability-id
// uniqueness, aoe telegraphs, ability counts, growth sums and primary stats.
// Read-only. Prints one line per file plus a final RESULT line; exit 0 clean, 1 on any error.
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_CLASSES = join(REPO, "content", "classes");
/** Files in the classes directory that are not class files. */
const NOT_CLASS_FILES = new Set(["roster.json", "schema.json"]);
/** The five stats, in the order the schema lists them. */
const STATS = ["str", "dex", "con", "int", "wis"];
/**
 * Classes per faction. The Wardens came from a four-branch tree and kept all
 * four; the Ashkin came from two. Owner ruling, 2026-10-21: the set is 20
 * classes, 4 starters and 8 advanced on the Wardens side, 2 starters and 6
 * advanced on the Ashkin side.
 */
const FACTION_SHAPE = {
  wardens: { starters: 4, tier1: 8 },
  ashkin: { starters: 2, tier1: 6 },
};
/** Abilities every class must have. */
const MIN_ABILITIES = 6;

/** JSON Schema keywords this validator understands. Anything else is a tool error, never a silent pass. */
const SUPPORTED_KEYWORDS = new Set([
  "$schema",
  "$id",
  "$comment",
  "$defs",
  "$ref",
  "title",
  "description",
  "default",
  "examples",
  "type",
  "enum",
  "const",
  "required",
  "properties",
  "additionalProperties",
  "items",
  "minItems",
  "maxItems",
  "uniqueItems",
  "minimum",
  "maximum",
  "exclusiveMinimum",
  "exclusiveMaximum",
  "minLength",
  "maxLength",
  "pattern",
]);

/** Join a property path to its parent. */
function child(base, key) {
  return base ? `${base}.${key}` : key;
}

/** Stable text of a JSON value, so two values can be compared for equality. */
function canonical(value) {
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "undefined";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  return `{${Object.keys(value)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`)
    .join(",")}}`;
}

/** The JSON type name of a value, as the schema spells it. */
function typeName(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (typeof value === "number") return Number.isInteger(value) ? "integer" : "number";
  return typeof value;
}

/** Does a value match one JSON Schema type name? An integer is a number. */
function matchesType(value, type) {
  switch (type) {
    case "object":
      return typeof value === "object" && value !== null && !Array.isArray(value);
    case "array":
      return Array.isArray(value);
    case "string":
      return typeof value === "string";
    case "boolean":
      return typeof value === "boolean";
    case "null":
      return value === null;
    case "number":
      return typeof value === "number" && Number.isFinite(value);
    case "integer":
      return typeof value === "number" && Number.isInteger(value);
    default:
      return false;
  }
}

/** Resolve a local `#/a/b` pointer inside the schema. */
function resolvePointer(root, ref) {
  if (!ref.startsWith("#/")) return undefined;
  let node = root;
  for (const raw of ref.slice(2).split("/")) {
    const key = raw.replace(/~1/g, "/").replace(/~0/g, "~");
    if (node === null || typeof node !== "object" || !(key in node)) return undefined;
    node = node[key];
  }
  return node;
}

/** Keywords this validator would ignore, so a schema cannot rely on them by accident. */
function unknownKeywords(schema, path, out) {
  if (schema === null || typeof schema !== "object" || Array.isArray(schema)) return out;
  for (const [key, value] of Object.entries(schema)) {
    if (key.startsWith("x-")) continue;
    if (!SUPPORTED_KEYWORDS.has(key)) {
      out.push(`${path}: unsupported schema keyword "${key}"`);
      continue;
    }
    if (key === "properties") {
      for (const [name, sub] of Object.entries(value ?? {})) unknownKeywords(sub, child(path, name), out);
    } else if (key === "$defs") {
      for (const [name, sub] of Object.entries(value ?? {})) unknownKeywords(sub, child(path, `$defs.${name}`), out);
    } else if (key === "items") {
      unknownKeywords(value, child(path, "items"), out);
    } else if (key === "additionalProperties" && value && typeof value === "object") {
      unknownKeywords(value, child(path, "additionalProperties"), out);
    }
  }
  return out;
}

/**
 * Validate one value against the schema subset. Returns a list of readable
 * errors, each starting with the path of the offending value.
 */
export function validateAgainstSchema(schema, data, root = schema, path = "") {
  const errors = [];
  if (schema === true) return errors;
  if (schema === false) return [`${path || "<root>"}: no value is allowed here`];
  if (schema === null || typeof schema !== "object" || Array.isArray(schema)) {
    return [`${path || "<root>"}: broken schema node`];
  }
  if (typeof schema.$ref === "string") {
    const target = resolvePointer(root, schema.$ref);
    if (target === undefined) errors.push(`${path || "<root>"}: schema $ref ${schema.$ref} does not resolve`);
    else errors.push(...validateAgainstSchema(target, data, root, path));
  }
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.some((type) => matchesType(data, type))) {
      errors.push(`${path || "<root>"}: expected ${types.join(" or ")}, got ${typeName(data)}`);
      return errors;
    }
  }
  if (schema.enum !== undefined && !schema.enum.some((value) => canonical(value) === canonical(data))) {
    errors.push(`${path || "<root>"}: ${JSON.stringify(data)} is not one of ${schema.enum.join("|")}`);
  }
  if (schema.const !== undefined && canonical(schema.const) !== canonical(data)) {
    errors.push(`${path || "<root>"}: must be ${JSON.stringify(schema.const)}`);
  }
  const isObject = typeof data === "object" && data !== null && !Array.isArray(data);
  if (isObject) {
    const known = new Set(Object.keys(schema.properties ?? {}));
    for (const key of schema.required ?? []) {
      if (!(key in data)) errors.push(`${path || "<root>"}: missing required property "${key}"`);
    }
    for (const [key, sub] of Object.entries(schema.properties ?? {})) {
      if (key in data) errors.push(...validateAgainstSchema(sub, data[key], root, child(path, key)));
    }
    if (schema.additionalProperties === false) {
      for (const key of Object.keys(data)) {
        if (!known.has(key)) errors.push(`${path || "<root>"}: unknown property "${key}"`);
      }
    } else if (schema.additionalProperties && typeof schema.additionalProperties === "object") {
      for (const key of Object.keys(data)) {
        if (!known.has(key)) errors.push(...validateAgainstSchema(schema.additionalProperties, data[key], root, child(path, key)));
      }
    }
  }
  if (Array.isArray(data)) {
    if (schema.minItems !== undefined && data.length < schema.minItems) {
      errors.push(`${path || "<root>"}: needs at least ${schema.minItems} items, has ${data.length}`);
    }
    if (schema.maxItems !== undefined && data.length > schema.maxItems) {
      errors.push(`${path || "<root>"}: allows at most ${schema.maxItems} items, has ${data.length}`);
    }
    if (schema.uniqueItems === true) {
      const seen = new Set();
      for (const item of data) {
        const key = canonical(item);
        if (seen.has(key)) errors.push(`${path || "<root>"}: duplicate item ${JSON.stringify(item)}`);
        seen.add(key);
      }
    }
    if (schema.items !== undefined) {
      data.forEach((item, index) => errors.push(...validateAgainstSchema(schema.items, item, root, `${path}[${index}]`)));
    }
  }
  if (typeof data === "string") {
    if (schema.minLength !== undefined && data.length < schema.minLength) {
      errors.push(`${path || "<root>"}: shorter than ${schema.minLength} characters`);
    }
    if (schema.maxLength !== undefined && data.length > schema.maxLength) {
      errors.push(`${path || "<root>"}: longer than ${schema.maxLength} characters`);
    }
    if (schema.pattern !== undefined && !new RegExp(schema.pattern).test(data)) {
      errors.push(`${path || "<root>"}: ${JSON.stringify(data)} does not match /${schema.pattern}/`);
    }
  }
  if (typeof data === "number") {
    if (schema.minimum !== undefined && data < schema.minimum) errors.push(`${path || "<root>"}: ${data} is below ${schema.minimum}`);
    if (schema.maximum !== undefined && data > schema.maximum) errors.push(`${path || "<root>"}: ${data} is above ${schema.maximum}`);
    if (schema.exclusiveMinimum !== undefined && data <= schema.exclusiveMinimum) {
      errors.push(`${path || "<root>"}: ${data} is not above ${schema.exclusiveMinimum}`);
    }
    if (schema.exclusiveMaximum !== undefined && data >= schema.exclusiveMaximum) {
      errors.push(`${path || "<root>"}: ${data} is not below ${schema.exclusiveMaximum}`);
    }
  }
  return errors;
}

/** The class files of a directory: every .json except the roster and the schema. */
export function listClassFiles(classesDir) {
  let names;
  try {
    names = readdirSync(classesDir);
  } catch {
    return [];
  }
  return names
    .filter((name) => name.endsWith(".json") && !NOT_CLASS_FILES.has(name))
    .sort();
}

/** Path as the reader should see it: repo-relative when it is inside the repo. */
function displayPath(path) {
  const rel = relative(REPO, path);
  if (rel && !rel.startsWith("..") && !resolve(rel).startsWith("..")) return rel.split("\\").join("/");
  return path;
}

/** The rules one class file must satisfy on its own. */
function perFileChecks(doc, { skipCount = false } = {}) {
  const problems = [];
  const abilities = Array.isArray(doc.abilities) ? doc.abilities : [];
  if (!skipCount && abilities.length < MIN_ABILITIES) {
    problems.push(`has ${abilities.length} abilities, needs at least ${MIN_ABILITIES}`);
  }
  const ids = new Map();
  const names = new Map();
  abilities.forEach((ability, index) => {
    if (!ability || typeof ability !== "object") return;
    const label = `abilities[${index}]`;
    if (typeof ability.id === "string") {
      if (ids.has(ability.id)) problems.push(`${label}.id "${ability.id}" is used twice in this file`);
      else ids.set(ability.id, index);
    }
    if (typeof ability.name === "string") {
      if (names.has(ability.name)) problems.push(`${label}.name "${ability.name}" is used twice in this file`);
      else names.set(ability.name, index);
    }
    if (ability.kind === "aoe" && ability.telegraph === "none") {
      problems.push(`${label} is aoe and needs a telegraph other than none`);
    }
  });
  const growth = doc.growth && typeof doc.growth === "object" ? doc.growth : null;
  if (growth) {
    const values = STATS.map((stat) => growth[stat]);
    if (values.every((value) => typeof value === "number")) {
      const sum = values.reduce((total, value) => total + value, 0);
      if (sum <= 0) problems.push(`growth sums to ${sum}, needs a positive sum`);
    }
    const primary = Array.isArray(doc.primary) ? doc.primary : [];
    for (const stat of primary) {
      const value = growth[stat];
      if (typeof value === "number" && value <= 0) {
        problems.push(`primary lists "${stat}" but its growth is ${value}`);
      }
    }
  }
  return problems;
}

/**
 * The rules that span files: branch targets, faction counts and ability ids
 * unique across the whole set. `classes` is a list of {id, doc}.
 */
export function crossFileChecks(classes) {
  const problems = [];
  const byId = new Map();
  for (const entry of classes) byId.set(entry.id, entry.doc);
  for (const entry of classes) {
    const branch = entry.doc.branch;
    if (typeof branch !== "string" || branch === "") continue;
    const target = byId.get(branch);
    if (!target) {
      problems.push(`class "${entry.id}" branches from "${branch}", which is not a class in the set`);
      continue;
    }
    if (target.tier !== 0) {
      problems.push(`class "${entry.id}" branches from "${branch}", which is tier ${target.tier}, not tier 0`);
    }
  }
  const factions = new Map();
  for (const entry of classes) {
    const faction = entry.doc.faction;
    if (typeof faction !== "string") continue;
    const row = factions.get(faction) ?? { starters: 0, tier1: 0 };
    if (entry.doc.tier === 0) row.starters += 1;
    else if (entry.doc.tier === 1) row.tier1 += 1;
    factions.set(faction, row);
  }
  for (const [faction, row] of [...factions].sort((a, b) => (a[0] < b[0] ? -1 : 1))) {
    const want = FACTION_SHAPE[faction];
    if (!want) {
      problems.push(`faction "${faction}" is not a known faction`);
      continue;
    }
    if (row.starters !== want.starters) {
      problems.push(`faction "${faction}" has ${row.starters} starters, needs exactly ${want.starters}`);
    }
    if (row.tier1 !== want.tier1) {
      problems.push(`faction "${faction}" has ${row.tier1} tier 1 classes, needs exactly ${want.tier1}`);
    }
  }
  const abilityIds = new Map();
  for (const entry of classes) {
    const abilities = Array.isArray(entry.doc.abilities) ? entry.doc.abilities : [];
    for (const ability of abilities) {
      if (!ability || typeof ability.id !== "string") continue;
      const owner = abilityIds.get(ability.id);
      if (owner && owner !== entry.id) problems.push(`ability id "${ability.id}" is used by "${owner}" and "${entry.id}"`);
      else if (!owner) abilityIds.set(ability.id, entry.id);
    }
  }
  return problems;
}

/** Run the whole gate. Returns { files, errors, lines }. */
export function validateClasses(classesDir = DEFAULT_CLASSES) {
  const lines = [];
  const errors = [];
  const schemaPath = join(classesDir, "schema.json");
  let schema;
  try {
    schema = JSON.parse(readFileSync(schemaPath, "utf8"));
  } catch (err) {
    return {
      files: 0,
      errors: [`${displayPath(schemaPath)}: cannot read the schema (${err.message})`],
      lines: [`${displayPath(schemaPath)}: FAIL cannot read the schema (${err.message})`],
    };
  }
  const keywordProblems = unknownKeywords(schema, "", []);
  if (keywordProblems.length > 0) {
    return { files: 0, errors: keywordProblems, lines: keywordProblems.map((problem) => `schema: FAIL ${problem}`) };
  }
  const names = listClassFiles(classesDir);
  if (names.length === 0) {
    return {
      files: 0,
      errors: [`no class files in ${displayPath(classesDir)}`],
      lines: [`cross: FAIL no class files in ${displayPath(classesDir)}`],
    };
  }
  const parsed = [];
  for (const name of names) {
    const path = join(classesDir, name);
    const shown = displayPath(path);
    let doc;
    try {
      doc = JSON.parse(readFileSync(path, "utf8"));
    } catch (err) {
      const problem = `cannot parse the file (${err.message})`;
      errors.push(`${shown}: ${problem}`);
      lines.push(`${shown}: FAIL ${problem}`);
      continue;
    }
    const problems = validateAgainstSchema(schema, doc, schema, "");
    const countFailed = problems.some((problem) => problem.startsWith("abilities: needs at least"));
    problems.push(...perFileChecks(doc, { skipCount: countFailed }));
    if (problems.length > 0) {
      errors.push(`${shown}: ${problems.join("; ")}`);
      lines.push(`${shown}: FAIL ${problems.join("; ")}`);
    } else {
      lines.push(`${shown}: ok`);
    }
    parsed.push({ id: typeof doc?.id === "string" ? doc.id : name, doc });
  }
  const cross = crossFileChecks(parsed);
  for (const problem of cross) {
    errors.push(`cross: ${problem}`);
    lines.push(`cross: FAIL ${problem}`);
  }
  return { files: names.length, errors, lines };
}

/** Command line: --classes DIR, --help. */
function parseArgs(argv) {
  const options = { classes: DEFAULT_CLASSES, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") options.help = true;
    else if (arg === "--classes") {
      const value = argv[i + 1];
      if (!value) throw new Error("--classes needs a directory");
      options.classes = resolve(value);
      i += 1;
    } else throw new Error(`unknown argument "${arg}"`);
  }
  return options;
}

if (import.meta.main) {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`class-validate: ${err.message}`);
    console.error("usage: node tools/class-validate.mjs [--classes DIR]");
    process.exit(1);
  }
  if (options.help) {
    console.log("usage: node tools/class-validate.mjs [--classes DIR]");
    console.log("  checks every content/classes/*.json (roster.json and schema.json are skipped)");
    console.log("  against content/classes/schema.json plus the cross-file class rules");
    process.exit(0);
  }
  const result = validateClasses(options.classes);
  for (const line of result.lines) console.log(line);
  if (result.errors.length === 0) console.log(`RESULT class-validate ok files=${result.files} errors=0`);
  else {
    for (const error of result.errors) console.error(`class-validate FAIL: ${error}`);
    console.log(`RESULT class-validate errors=${result.errors.length}`);
  }
  process.exit(result.errors.length === 0 ? 0 : 1);
}
