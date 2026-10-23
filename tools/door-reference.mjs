#!/usr/bin/env node
// door-reference: print every MCP verb of the AI door (core tools and verb packs) as one book page, from the Rust source.
//   node tools/door-reference.mjs            print the page to stdout
//   node tools/door-reference.mjs --write    write docs/book/door-reference.md
//   node tools/door-reference.mjs --check    exit 1 when the checked-in page differs from the generated one
// How: a small text parser over crates/endpoint/src (no running server, no cargo): it reads the `e2_tool!` macros and
// `impl ToolBase` blocks (name, description, read-only hint, argument and output types), the `VerbSpec::of::<Tool>(NAME,
// PACK, CapabilityCard::X)` registrations (pack, scope, feature gate), the argument structs (fields, types, defaults,
// serde aliases) and the refusal sites (`McpToolError::new("code", ..)`, `bad(..)`, `bad_args(..)`) in the verb's handler
// and in the same-file functions it calls. Ranges are quoted from the description and refusal text, never invented; the
// handler's validation stays the authority. Pure text in, text out (the tests feed it fixture sources).
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { lex } from "./rust-quality.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const SRC_DIR = "crates/endpoint/src";
export const PAGE_PATH = "docs/book/door-reference.md";

// ---- lexing helpers ----
// code = the text with comments and string bodies blanked (same length, same offsets), so brackets are safe to count.
class Src {
  constructor(rel, text) {
    this.rel = rel;
    this.text = text.replace(/\r\n/g, "\n");
    this.code = lex(this.text).code;
    this.tests = [];
    for (const m of this.code.matchAll(/#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{/g)) {
      const open = m.index + m[0].length - 1;
      this.tests.push([m.index, closeOf(this.code, open)]);
    }
  }
  inTest(pos) { return this.tests.some(([a, b]) => pos >= a && pos <= b); }
}

// Offset of the bracket closing the one at `open` (same kind), or the end of the text.
function closeOf(code, open) {
  const pairs = { "{": "}", "(": ")", "[": "]" };
  const o = code[open];
  const c = pairs[o];
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    if (code[i] === o) depth++;
    else if (code[i] === c) { depth--; if (depth === 0) return i; }
  }
  return code.length - 1;
}

// Split code[from, to) at commas that sit outside every bracket (and outside <> when angle is set).
function splitTop(code, from, to, angle = false) {
  const parts = [];
  let depth = 0;
  let start = from;
  for (let i = from; i < to; i++) {
    const ch = code[i];
    if (ch === "(" || ch === "[" || ch === "{") depth++;
    else if (ch === ")" || ch === "]" || ch === "}") depth--;
    else if (angle && ch === "<") depth++;
    else if (angle && ch === ">" && code[i - 1] !== "-" && code[i - 1] !== "=") depth--;
    else if (ch === "," && depth === 0) { parts.push([start, i]); start = i + 1; }
  }
  if (code.slice(start, to).trim()) parts.push([start, to]);
  return parts;
}

// The string literals of src.text[from, to): [{ value, at }], found through the blanked quotes of src.code.
function literals(src, from, to) {
  const out = [];
  const q = [];
  for (let i = from; i < to; i++) if (src.code[i] === '"') q.push(i);
  // Raw strings keep their `r#"` prefix in code; an odd quote count means a char literal like '"' slipped through, so skip it.
  for (let k = 0; k + 1 < q.length; k += 2) out.push({ value: unescape(src.text.slice(q[k] + 1, q[k + 1])), at: q[k] });
  return out;
}

function unescape(s) {
  return s
    .replace(/\\\n\s*/g, "")
    .replace(/\\(["'\\])/g, "$1")
    .replace(/\\n/g, "\n")
    .replace(/\\t/g, "\t")
    .replace(/\\u\{([0-9a-fA-F]+)\}/g, (_, h) => String.fromCodePoint(parseInt(h, 16)));
}

const squash = (s) => s.replace(/\s+/g, " ").trim();

// ---- crate index ----
function indexCrate(files) {
  const srcs = [...files.keys()].sort().map((rel) => new Src(rel, files.get(rel)));
  const byRel = new Map(srcs.map((s) => [s.rel, s]));
  // consts: NAME -> [{ src, value }] (integer or string literal, a path to another
  // const, or the sum/difference of two integer consts)
  const consts = new Map();
  for (const src of srcs) {
    for (const m of src.code.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*[^=;]+=\s*([^;]+);/g)) {
      if (src.inTest(m.index)) continue;
      const rhsStart = m.index + m[0].indexOf("=", m[0].indexOf(m[1])) + 1;
      const rhs = squash(src.code.slice(rhsStart, m.index + m[0].length - 1));
      let value = null;
      const lit = literals(src, rhsStart, m.index + m[0].length - 1);
      if (/^"\s*"$/.test(rhs) && lit.length === 1) value = { kind: "str", v: lit[0].value };
      else if (/^-?\d[\d_]*(?:_?[iu](?:8|16|32|64|128|size))?$/.test(rhs)) value = { kind: "int", v: rhs.replace(/_?[iu](?:8|16|32|64|128|size)$/, "").replace(/_/g, "") };
      else if (/^-?\d[\d_]*\.\d[\d_]*(?:[fF](?:32|64))?$/.test(rhs)) value = { kind: "num", v: rhs.replace(/[fF](?:32|64)$/, "").replace(/_/g, "") };
      else if (/^(?:[a-z_]\w*::)*[A-Z][A-Z0-9_]*$/.test(rhs)) value = { kind: "path", v: rhs };
      else if (/^[\w:+\-*/<>() ]+$/.test(rhs) && /[+\-*/<>]/.test(rhs)) value = { kind: "expr", v: rhs };
      if (value) { if (!consts.has(m[1])) consts.set(m[1], []); consts.get(m[1]).push({ src, ...value }); }
    }
  }
  return { srcs, byRel, consts };
}

// One indexed const as { kind, v }, following paths and evaluating integer expressions.
function entryValue(idx, c, depth) {
  if (c.kind === "path") return depth < 6 ? constValue(idx, c.v, c.src, depth + 1) : null;
  if (c.kind === "expr") return evalConstExpr(idx, c, depth + 1);
  return { kind: c.kind, v: c.v };
}

// A const's value as { kind, v }, following `a::b::NAME` aliases; null when unknown or ambiguous.
// The same name in several crates resolves only when every definition agrees, so a shared number
// prints and a conflicting one stays a placeholder.
function constValue(idx, name, fromSrc, depth = 0) {
  if (depth >= 6) return null;
  const last = name.split("::").pop();
  const all = idx.consts.get(last) ?? [];
  let hit = all.filter((c) => c.src === fromSrc);
  if (hit.length !== 1) hit = name.includes("::") ? all.filter((c) => c.src.rel.replace(/\.rs$/, "").endsWith(name.split("::").slice(-2, -1)[0] ?? "\0")) : [];
  if (hit.length === 1) return entryValue(idx, hit[0], depth);
  if (!all.length) return null;
  const vals = all.map((c) => entryValue(idx, c, depth + 1));
  const first = vals[0];
  return first && vals.every((v) => v && v.kind === first.kind && v.v === first.v) ? first : null;
}

// An integer expression (`4 * 1024`, `1 << 20`, `2 * (4 + MAX_ROWS * 26)`) over numbers and
// consts; anything else is null, so an unresolved value stays a placeholder.
function evalConstExpr(idx, c, depth) {
  if (depth >= 6) return null;
  const toks = c.v.match(/<<|>>|[()+\-*/]|\d[\d_]*|[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*/g);
  if (!toks) return null;
  let i = 0;
  const apply = (op, a, b) => {
    const x = BigInt(a), y = BigInt(b);
    switch (op) {
      case "+": return (x + y).toString();
      case "-": return (x - y).toString();
      case "*": return (x * y).toString();
      case "<<": return (x << y).toString();
      case ">>": return (x >> y).toString();
      case "/": return y === 0n ? null : (x / y).toString();
    }
    return null;
  };
  const atom = () => {
    const t = toks[i++];
    if (t === undefined) throw new Error("end");
    if (t === "(") { const v = expr(); if (toks[i++] !== ")") throw new Error(")"); return v; }
    if (/^\d/.test(t)) return t.replace(/_/g, "");
    const v = constValue(idx, t, c.src, depth + 1);
    if (!v || (v.kind !== "int" && v.kind !== "num")) throw new Error("name");
    return v.v;
  };
  const term = () => {
    let v = atom();
    while (["*", "/", "<<", ">>"].includes(toks[i])) { const op = toks[i++]; v = apply(op, v, atom()); }
    return v;
  };
  const expr = () => {
    let v = term();
    while (toks[i] === "+" || toks[i] === "-") { const op = toks[i++]; v = apply(op, v, term()); }
    return v;
  };
  try {
    const v = expr();
    return i === toks.length && v !== null ? { kind: "int", v } : null;
  } catch {
    return null;
  }
}

// A name expression (`"x.y"`, `TOOL_X`, `super::TOOL_X`, `a::MCP_TOOL_NAME`) as its string value, or null.
function stringOf(idx, src, from, to) {
  const lit = literals(src, from, to);
  if (lit.length) return lit.map((l) => l.value).join("");
  const id = squash(src.code.slice(from, to)).replace(/\.into\(\)$/, "").trim();
  if (!/^(?:[A-Za-z_]\w*::)*[A-Z][A-Z0-9_]*$/.test(id)) return null;
  const v = constValue(idx, id, src);
  return v && v.kind === "str" ? v.v : null;
}

// The string value of the first constant named in code[from, to) (a description given as `Some(CONST.into())`), or null.
function constText(idx, src, from, to) {
  const m = /\b([A-Z][A-Z0-9_]{2,})\b/.exec(src.code.slice(from, to));
  const v = m && constValue(idx, m[1], src);
  return v && v.kind === "str" ? v.v : null;
}

// ---- functions ----
function fnIndex(src) {
  const out = [];
  const re = /\bfn\s+([A-Za-z_]\w*)/g;
  for (let m = re.exec(src.code); m; m = re.exec(src.code)) {
    let i = m.index + m[0].length;
    while (/\s/.test(src.code[i] ?? "")) i++;
    if (src.code[i] === "<") {
      let depth = 0;
      for (; i < src.code.length; i++) {
        if (src.code[i] === "<") depth++;
        else if (src.code[i] === ">" && src.code[i - 1] !== "-") { depth--; if (depth === 0) { i++; break; } }
      }
    }
    while (i < src.code.length && src.code[i] !== "(") i++;
    const params = [i, closeOf(src.code, i)];
    let open = -1;
    let square = 0;
    for (let k = params[1] + 1; k < src.code.length; k++) {
      const c = src.code[k];
      if (c === "[") square++;
      else if (c === "]") square--;
      else if (square === 0 && c === ";") break;
      else if (square === 0 && c === "{") { open = k; break; }
    }
    if (open < 0) continue;
    out.push({ name: m[1], at: m.index, params, body: [open + 1, closeOf(src.code, open)] });
  }
  return out;
}

const NOT_CALLS = new Set(["if", "while", "for", "match", "loop", "return", "fn", "let", "in", "as", "move", "dispatch", "invoke", "new", "default", "from", "into", "map_err", "ok_or", "ok_or_else", "unwrap_or", "unwrap_or_else", "map", "and_then", "iter", "collect", "len", "push", "get", "insert", "contains", "clone", "to_owned", "to_string", "format", "write", "lock"]);

// Functions followed from one handler before the walk stops. The visited set
// already makes a cycle terminate; this bounds a call graph that fans out over
// a whole file. 60 cut real verbs mid-graph (scene.place lost its bad_want
// helper, tx.apply lost the spatial delta checks), so the cap sits above every
// handler in the crate; a graph that ever passes it is named in Coverage.
const MAX_REACHED_FNS = 500;

// Names called as `name(` or `.name(` inside code[from, to).
function calledNames(src, from, to) {
  const names = [];
  for (const m of src.code.slice(from, to).matchAll(/(?<![A-Za-z0-9_!])([a-z_][a-z0-9_]*)\s*(?:::<[^>]*>)?\s*\(/g)) if (!NOT_CALLS.has(m[1])) names.push(m[1]);
  return names;
}

// ---- refusal sites ----
const CTOR = /\bMcpToolError::(?:new|wrap)\s*\(|(?<![A-Za-z0-9_:.])bad_args\s*\(|(?<![A-Za-z0-9_:.])bad\s*\(/g;

// Refusal sites in src.code[from, to): { code, message, at, dynamic } where dynamic means the message was not a literal.
function refusalSites(src, from, to) {
  const sites = [];
  const body = src.code.slice(from, to);
  for (const m of body.matchAll(CTOR)) {
    const start = from + m.index;
    const open = start + m[0].length - 1;
    const close = closeOf(src.code, open);
    const parts = splitTop(src.code, open + 1, close);
    const isCtor = m[0].startsWith("McpToolError");
    let code = "bad_args";
    let msg = parts[0];
    if (isCtor) {
      const c = parts[0] ? literals(src, parts[0][0], parts[0][1]) : [];
      code = c.length === 1 && /^\s*"/.test(src.code.slice(parts[0][0], parts[0][1])) ? c[0].value : null;
      msg = parts[1];
    }
    const lit = msg ? literals(src, msg[0], msg[1]) : [];
    sites.push({ code, message: lit.length ? lit[0].value : null, expr: msg ? squash(src.text.slice(msg[0], msg[1])) : "", at: start, dynamic: !lit.length });
  }
  return sites;
}

const fillConsts = (idx, src, message) => message
  .replace(/\{\{/g, "\u0001").replace(/\}\}/g, "\u0002")
  .replace(/\{([A-Z][A-Z0-9_]*)\}/g, (all, name) => { const v = constValue(idx, name, src); return v ? v.v : all; })
  .replace(/\u0001/g, "{").replace(/\u0002/g, "}");

// ---- tool declarations ----
// Every tool type the source declares: e2_tool! macros and `impl ToolBase for T` blocks.
function toolDefs(idx) {
  const defs = new Map();
  for (const src of idx.srcs) {
    for (const m of src.code.matchAll(/\be2_tool!\s*\(/g)) {
      if (src.inTest(m.index)) continue;
      const open = m.index + m[0].length - 1;
      const close = closeOf(src.code, open);
      const parts = splitTop(src.code, open + 1, close, true);
      const ty = parts[0] && squash(src.code.slice(...parts[0]));
      if (!ty || !/^[A-Z]\w*$/.test(ty)) continue; // the macro's own rules use $tool, not a type
      const kv = {};
      for (const p of parts.slice(1)) {
        const mm = /^\s*(\w+)\s*=/.exec(src.code.slice(...p));
        if (mm) kv[mm[1]] = [p[0] + mm[0].length, p[1]];
      }
      const d = { type: ty, src, at: m.index, kind: "macro", name: kv.name ? stringOf(idx, src, ...kv.name) : null };
      d.description = kv.description ? (literals(src, ...kv.description).map((l) => l.value).join("") || stringOf(idx, src, ...kv.description) || "") : "";
      d.readonly = kv.readonly ? /^\s*true\s*$/.test(src.code.slice(...kv.readonly)) ? true : /^\s*false\s*$/.test(src.code.slice(...kv.readonly)) ? false : null : null;
      d.args = kv.args ? squash(src.code.slice(...kv.args)) : null;
      d.output = kv.output ? squash(src.code.slice(...kv.output)) : null;
      d.entries = kv.invoke ? [squash(src.code.slice(...kv.invoke)).split("::").pop()] : [];
      if (d.name) defs.set(ty, d);
    }
    for (const m of src.code.matchAll(/\bimpl\s+(?:[\w:]+::)?ToolBase\s+for\s+(\w+)\s*\{/g)) {
      if (src.inTest(m.index)) continue;
      const open = m.index + m[0].length - 1;
      const close = closeOf(src.code, open);
      const block = [open + 1, close];
      const ty = m[1];
      const d = { type: ty, src, at: m.index, kind: "impl", description: "", readonly: null, entries: [] };
      const get = (re) => { const mm = re.exec(src.code.slice(...block)); return mm ? squash(mm[1]) : null; };
      d.args = get(/type\s+Parameter\s*=\s*([^;]+);/);
      d.output = get(/type\s+Output\s*=\s*([^;]+);/);
      const nameFn = fnIndex(src).find((f) => f.body[0] > block[0] && f.body[1] < block[1] && f.name === "name");
      const descFn = fnIndex(src).find((f) => f.body[0] > block[0] && f.body[1] < block[1] && f.name === "description");
      const annFn = fnIndex(src).find((f) => f.body[0] > block[0] && f.body[1] < block[1] && f.name === "annotations");
      d.name = nameFn ? stringOf(idx, src, nameFn.body[0], nameFn.body[1]) : null;
      d.description = descFn ? literals(src, descFn.body[0], descFn.body[1]).map((l) => l.value).join("") || (constText(idx, src, descFn.body[0], descFn.body[1]) ?? "") : "";
      if (annFn) {
        const a = src.code.slice(annFn.body[0], annFn.body[1]);
        d.readonly = /\bread_?only_annotations\s*\(\)/.test(a) ? true : /read_only_hint\s*=\s*Some\(\s*false\s*\)/.test(a) ? false : /read_only_hint\s*=\s*Some\(\s*true\s*\)/.test(a) ? true : null;
      }
      // The handler: names called inside `fn invoke` of the matching SyncTool impl.
      const sync = new RegExp(String.raw`\bimpl\s+(?:[\w:]+::)?SyncTool\s*<[^>]*>\s+for\s+${ty}\s*\{`).exec(src.code);
      if (sync) {
        const sOpen = sync.index + sync[0].length - 1;
        const inv = fnIndex(src).find((f) => f.body[0] > sOpen && f.body[1] < closeOf(src.code, sOpen) && f.name === "invoke");
        if (inv) { d.entries = calledNames(src, inv.body[0], inv.body[1]); d.invokeFn = inv; }
      }
      if (d.name && !defs.has(ty)) defs.set(ty, d);
    }
  }
  return defs;
}

// ---- argument structs ----
// Rustdoc links to constants (a name in square brackets and backticks) as the name and its value, so the page reads without the source.
const docLinks = (idx, src, text) => text.replace(/\[`([A-Za-z_]\w*)`\]/g, (all, name) => {
  const v = /^[A-Z][A-Z0-9_]*$/.test(name) ? constValue(idx, name, src) : null;
  return v ? `\`${name}\` (${v.v})` : `\`${name}\``;
});
function structOf(idx, tyText) {
  const name = (tyText ?? "").replace(/<.*$/, "").split("::").pop().trim();
  for (const src of idx.srcs) {
    const m = new RegExp(String.raw`\bstruct\s+${name}\b\s*(?:<[^>]*>)?\s*(\{|;|\()`).exec(src.code);
    if (!m || src.inTest(m.index)) continue;
    const attrFrom = Math.max(0, src.text.lastIndexOf("\n\n", m.index));
    const attrs = src.text.slice(attrFrom, m.index);
    const fields = [];
    if (m[1] === "{") {
      const open = m.index + m[0].length - 1;
      const close = closeOf(src.code, open);
      for (const [a, b] of splitTop(src.code, open + 1, close, true)) {
        let i = a;
        const attrList = [];
        const docs = [];
        for (;;) {
          while (/\s/.test(src.code[i] ?? "") && i < b) i++;
          if (src.code.startsWith("#[", i)) { const e = closeOf(src.code, i + 1); attrList.push(src.text.slice(i, e + 1)); i = e + 1; } else break;
        }
        for (const line of src.text.slice(a, b).split("\n")) { const dm = /^\s*\/\/\/\s?(.*)$/.exec(line); if (dm) docs.push(dm[1]); }
        const fm = /^\s*(?:pub(?:\([^)]*\))?\s+)?([A-Za-z_]\w*)\s*:\s*([^]*?)\s*$/.exec(src.code.slice(i, b));
        if (!fm) continue;
        const attr = attrList.join(" ");
        fields.push({ name: fm[1], type: squash(fm[2]), docs: docLinks(idx, src, squash(docs.join(" "))), alias: [...attr.matchAll(/alias\s*=\s*"([^"]+)"/g)].map((x) => x[1]), fieldDefault: /serde\([^)]*\bdefault\b/.test(attr), decimal: /dec_u64/.test(attr), skip: /serde\([^)]*\bskip\b/.test(attr) });
      }
    }
    const structDefault = /serde\([^)]*\bdefault\b/.test(attrs.replace(/\n\s*\n/g, " "));
    // `impl Default` literal values that are plain literals.
    const defaults = new Map();
    const dm = new RegExp(String.raw`\bimpl\s+Default\s+for\s+${name}\s*\{`).exec(src.code);
    if (dm) {
      const dOpen = dm.index + dm[0].length - 1;
      const dClose = closeOf(src.code, dOpen);
      const lit = new RegExp(String.raw`(?<!->\s*)(?:\bSelf|\b${name})\s*\{`).exec(src.code.slice(dOpen, dClose));
      if (lit) {
        const lOpen = dOpen + lit.index + lit[0].length - 1;
        for (const [a, b] of splitTop(src.code, lOpen + 1, closeOf(src.code, lOpen))) {
          const km = /^\s*([A-Za-z_]\w*)\s*:/.exec(src.code.slice(a, b));
          if (!km) continue;
          const expr = squash(src.text.slice(a + km[0].length, b));
          if (/^(?:-?[\d_.]+(?:_?[iuf]\d+)?|true|false|None|"[^"]*"|\[[-\d_., ]*\]|String::new\(\)|Vec::new\(\)|vec!\[\])$/.test(expr)) defaults.set(km[1], expr.replace(/_(?=\d)/g, "").replace(/_?[iuf](?:8|16|32|64|size)$/, ""));
        }
      }
    }
    return { name, src, fields, structDefault, defaults };
  }
  return null;
}

// ---- registrations and packs ----
// `#[cfg(feature = "x")]` gates in force at pos: enclosing blocks plus the attribute on the statement itself.
function gatesAt(src, pos) {
  const stack = [];
  const gateOf = (before) => { const mm = /#\[cfg\(\s*(not\()?\s*feature\s*=\s*"([^"]+)"\s*\)*\s*\]\s*$/.exec(before); return mm && !mm[1] ? mm[2] : null; };
  for (let i = 0; i < pos; i++) {
    const ch = src.code[i];
    if (ch === "{") {
      // text before the brace, up to the previous statement end
      const lo = Math.max(src.code.lastIndexOf(";", i - 1), src.code.lastIndexOf("}", i - 1), src.code.lastIndexOf("{", i - 1)) + 1;
      stack.push(gateOf(src.text.slice(lo, i)));
    } else if (ch === "}") stack.pop();
  }
  const lo = Math.max(src.code.lastIndexOf(";", pos - 1), src.code.lastIndexOf("}", pos - 1), src.code.lastIndexOf("{", pos - 1)) + 1;
  const own = [...src.text.slice(lo, pos).matchAll(/#\[cfg\(\s*feature\s*=\s*"([^"]+)"\s*\)\]/g)].map((x) => x[1]);
  return [...new Set([...stack.filter(Boolean), ...own])];
}

// The module a `super::` path points at: the file that declares this one with #[path = ".."], else the folder's mod.rs.
function parentSrc(idx, src) {
  const base = src.rel.split("/").pop();
  for (const other of idx.srcs) {
    if (other === src) continue;
    if (new RegExp(String.raw`#\[path\s*=\s*"${base.replace(/\./g, "\\.")}"\s*\]`).test(other.text) && dirname(other.rel) === dirname(src.rel)) return other;
  }
  return idx.byRel.get(`${dirname(src.rel)}/mod.rs`) ?? null;
}

function parseRegistrations(idx) {
  const regs = [];
  for (const src of idx.srcs) {
    for (const m of src.code.matchAll(/\bVerbSpec::of::<\s*(\w+)\s*>\s*\(/g)) {
      if (src.inTest(m.index)) continue;
      const open = m.index + m[0].length - 1;
      const parts = splitTop(src.code, open + 1, closeOf(src.code, open));
      if (parts.length < 3) continue;
      const card = /CapabilityCard::(\w+)/.exec(src.code.slice(...parts[2]));
      if (!card) continue;
      const packExpr = squash(src.code.slice(...parts[1]));
      let packOwner = src;
      if (/^super::/.test(packExpr)) packOwner = parentSrc(idx, src) ?? src;
      regs.push({ type: m[1], src, at: m.index, nameExpr: parts[0], packExpr, packOwner, packIsLiteral: /^"/.test(packExpr) ? literals(src, ...parts[1])[0]?.value : null, card: card[1], gates: gatesAt(src, m.index) });
    }
  }
  return regs;
}

// ---- model ----
export function buildModel(files, root = null) {
  const idx = indexCrate(files);
  if (root) mergeWorkspaceConsts(idx, root);
  const defs = toolDefs(idx);
  const regs = parseRegistrations(idx);
  const model = { cards: {}, core: [], packs: [], counts: {}, gateCodes: [], unparsed: [] };

  // Card meanings: the doc line of each CapabilityCard variant.
  const cap = idx.byRel.get("tool_capabilities.rs");
  if (cap) {
    const en = /\bpub\s+enum\s+CapabilityCard\s*\{/.exec(cap.code);
    if (en) {
      const body = cap.text.slice(en.index, closeOf(cap.code, en.index + en[0].length - 1));
      for (const m of body.matchAll(/((?:\s*\/\/\/[^\n]*\n)+)\s*(\w+),/g)) model.cards[m[2].toLowerCase()] = squash(m[1].replace(/\/\/\//g, " "));
    }
    // Gate refusals: the expected codes of the adversarial briefs that were refused.
    const seen = new Map();
    for (const m of cap.text.matchAll(/^\/\/! \|\s*\d+\s*\|\s*([^|]+?)\s*\|\s*[^|]+\|\s*([a-z_]+)\s*\|\s*no\s*\|/gm)) if (!seen.has(m[2])) seen.set(m[2], squash(m[1]));
    model.gateCodes = [...seen].map(([code, brief]) => ({ code, brief }));
  }

  // Core card table: `match tool { "tx.preview" => CapabilityCard::Propose, .. }`.
  const coreCards = {};
  if (cap) {
    const f = /\bpub\s+fn\s+card_for_tool\b/.exec(cap.code);
    if (f) {
      const open = cap.code.indexOf("{", f.index);
      for (const m of cap.text.slice(open, closeOf(cap.code, open)).matchAll(/"([a-z_.]+)"\s*=>\s*CapabilityCard::(\w+)/g)) coreCards[m[1]] = m[2];
    }
  }

  // Door-wide refusals: the sites in packs/mod.rs outside tests (unknown verb, unknown pack, bad args).
  const modSrc = idx.byRel.get("packs/mod.rs");
  const doorWide = [];
  if (modSrc) for (const s of refusalSites(modSrc, 0, modSrc.code.length)) if (!modSrc.inTest(s.at) && s.code && s.message) doorWide.push({ code: s.code, message: fillConsts(idx, modSrc, s.message) });
  model.doorWide = dedupe(doorWide);

  // Every function in the crate by name, so a call into another file is one
  // lookup and not a scan of the whole tree per call.
  const allByName = new Map();
  for (const g of idx.srcs) for (const h of fnIndex(g)) {
    const list = allByName.get(h.name) ?? [];
    list.push([g, h]);
    allByName.set(h.name, list);
  }

  const makeVerb = (def, extra) => {
    const verb = { name: def.name, type: def.type, file: def.src.rel, description: def.description, readonly: def.readonly, ...extra };
    const st = structOf(idx, def.args);
    verb.argsType = def.args;
    verb.args = st ? st.fields.filter((f) => !f.skip).map((f) => ({ ...f, optional: /^Option\s*</.test(f.type) || f.fieldDefault || st.structDefault, default: st.defaults.get(f.name) ?? null })) : null;
    const out = structOf(idx, def.output);
    verb.outputType = def.output;
    verb.outputFields = out ? out.fields.map((f) => f.name) : null;
    // Handler scope: the entry functions and the same-file functions they call.
    const fnsOf = (src) => fnIndex(src);
    const entryFns = def.invokeFn ? [[def.src, def.invokeFn]] : [];
    for (const name of def.entries) {
      const local = fnsOf(def.src).filter((f) => f.name === name);
      let hit = local.length ? local.map((f) => [def.src, f]) : [];
      if (!hit.length) {
        const all = idx.srcs.flatMap((s) => fnsOf(s).filter((f) => f.name === name).map((f) => [s, f]));
        if (all.length === 1) hit = all;
      }
      entryFns.push(...hit);
    }
    const seenFns = new Set();
    const queue = [...entryFns];
    const reached = [];
    while (queue.length && reached.length < MAX_REACHED_FNS) {
      const [s, f] = queue.shift();
      const key = `${s.rel}:${f.at}`;
      if (seenFns.has(key)) continue;
      seenFns.add(key);
      reached.push([s, f]);
      const local = fnsOf(s);
      for (const name of calledNames(s, f.body[0], f.body[1])) {
        const same = local.filter((g) => g.name === name);
        if (same.length === 1) { queue.push([s, same[0]]); continue; }
        // A call into another file: the handler may have moved there, so the
        // refusal sites it owns belong to the verb. A name that is not
        // unique across the crate is ambiguous and stays out.
        if (!same.length) {
          const all = allByName.get(name) ?? [];
          if (all.length === 1) queue.push(all[0]);
        }
      }
    }
    // Helpers that wrap a refusal around a message the caller supplies are sites at every call of the helper.
    const sites = [];
    for (const [s, f] of reached) {
      const helperNames = new Map();
      for (const g of fnsOf(s)) {
        const inner = refusalSites(s, g.body[0], g.body[1]);
        // A helper hands one of its own parameters to the constructor (`fn bad_csv(message: String) -> McpToolError`).
        const params = new Set(s.code.slice(g.params[0] + 1, g.params[1]).split(",").map((p) => p.split(":")[0].trim()));
        if (inner.length && inner.every((x) => x.dynamic && params.has(x.expr))) helperNames.set(g.name, inner[0].code);
      }
      if (!helperNames.has(f.name)) for (const x of refusalSites(s, f.body[0], f.body[1])) sites.push({ ...x, message: x.dynamic ? `(built at run time: ${x.expr})` : x.message, src: s });
      for (const [hn, hcode] of helperNames) {
        if (hn === f.name) continue;
        for (const m of s.code.slice(f.body[0], f.body[1]).matchAll(new RegExp(String.raw`(?<![A-Za-z0-9_:.])${hn}\s*\(`, "g"))) {
          const open = f.body[0] + m.index + m[0].length - 1;
          const lit = literals(s, open + 1, closeOf(s.code, open));
          if (lit.length) sites.push({ code: hcode ?? hn, message: lit[0].value, at: f.body[0] + m.index, src: s, dynamic: false });
        }
      }
    }
    sites.sort((a, b) => (a.src.rel < b.src.rel ? -1 : a.src.rel > b.src.rel ? 1 : a.at - b.at));
    verb.refusals = dedupe(sites.filter((x) => x.code).map((x) => ({ code: x.code, message: fillConsts(idx, x.src, x.message) })));
    verb.journaled = reached.some(([s, f]) => /\bapply_gated\s*\(/.test(s.code.slice(f.body[0], f.body[1])));
    return verb;
  };

  // Core tools in contract order.
  const mcp = idx.byRel.get("mcp.rs");
  const coreList = mcp ? /\bCORE_TOOL_NAMES\s*:\s*\[[^\]]*\]\s*=\s*\[([^\]]*)\]/.exec(mcp.code) : null;
  const byName = new Map([...defs.values()].map((d) => [d.name, d]));
  if (coreList) {
    const open = mcp.code.indexOf("[", coreList.index + coreList[0].indexOf("=")) ;
    for (const [a, b] of splitTop(mcp.code, open + 1, closeOf(mcp.code, open))) {
      const name = stringOf(idx, mcp, a, b);
      const def = name && byName.get(name);
      if (!def) { model.unparsed.push(`core tool ${squash(mcp.code.slice(a, b))}: no declaration found`); continue; }
      model.core.push(makeVerb(def, { card: (coreCards[name] ?? "Observe").toLowerCase(), pack: null, gates: [] }));
    }
  }

  // Packs in PACK_NAMES order.
  const packFiles = [];
  if (modSrc) {
    const pn = /\bPACK_NAMES\s*:\s*\[[^\]]*\]\s*=\s*\[/.exec(modSrc.code);
    if (pn) {
      const open = modSrc.code.indexOf("[", pn.index + pn[0].length - 1);
      for (const [a, b] of splitTop(modSrc.code, open + 1, closeOf(modSrc.code, open))) {
        const mod = /^\s*(\w+)::PACK\s*$/.exec(modSrc.code.slice(a, b));
        if (mod) packFiles.push(mod[1]);
      }
    }
  }
  for (const mod of packFiles) {
    const src = idx.byRel.get(`packs/${mod}.rs`) ?? idx.byRel.get(`packs/${mod}/mod.rs`);
    if (!src) { model.unparsed.push(`pack module ${mod}: file not found`); continue; }
    const pk = constValue(idx, "PACK", src);
    const pack = { module: mod, name: pk ? pk.v : mod, file: src.rel, summary: "", loaded: null, verbs: [] };
    const nm = /\bPackDef::new\s*\(/.exec(src.code);
    if (nm) {
      const open = nm.index + nm[0].length - 1;
      const parts = splitTop(src.code, open + 1, closeOf(src.code, open));
      pack.loaded = parts[2] ? /^\s*true\s*$/.test(src.code.slice(...parts[2])) : null;
      if (parts[1]) {
        const lit = literals(src, ...parts[1]);
        if (lit.length) pack.summary = lit.map((l) => l.value).join("");
        else {
          // `let blurb = "..";` under cfg(feature): prefer the variant a feature turns on
          const vname = squash(src.code.slice(...parts[1]));
          const alts = [...src.code.matchAll(new RegExp(String.raw`\blet\s+${vname}\s*=\s*([^;]+);`, "g"))].map((x) => ({ at: x.index, text: literals(src, x.index, x.index + x[0].length).map((l) => l.value).join(""), gated: /#\[cfg\(feature/.test(src.text.slice(Math.max(0, x.index - 40), x.index)) && !/not\(feature/.test(src.text.slice(Math.max(0, x.index - 40), x.index)) }));
          pack.summary = (alts.find((a) => a.gated) ?? alts[0])?.text ?? "";
        }
      }
    }
    const mine = regs.filter((r) => (r.packOwner === src && (r.packExpr === "PACK" || r.packExpr.startsWith("super::"))) || r.packIsLiteral === pack.name);
    mine.sort((a, b) => (a.src === b.src ? a.at - b.at : a.src === src ? -1 : b.src === src ? 1 : a.src.rel < b.src.rel ? -1 : 1));
    for (const r of mine) {
      const def = defs.get(r.type);
      // VerbSpec::of refuses a name its tool does not carry, so the tool's own name is the verb's name.
      const name = def?.name;
      if (!def || !name) { model.unparsed.push(`verb ${r.type} in ${r.src.rel}: no declaration found`); continue; }
      pack.verbs.push(makeVerb(def, { card: r.card.toLowerCase(), pack: pack.name, gates: r.gates }));
    }
    model.packs.push(pack);
  }
  model.counts = { core: model.core.length, packs: model.packs.length, verbs: model.packs.reduce((n, p) => n + p.verbs.length, 0), gated: model.packs.reduce((n, p) => n + p.verbs.filter((v) => v.gates.length).length, 0) };
  return model;
}

function dedupe(rows) {
  const seen = new Set();
  return rows.filter((r) => { const k = `${r.code}\u0000${r.message}`; if (seen.has(k)) return false; seen.add(k); return true; });
}

// ---- page ----
// Prose is escaped for HTML and emphasis; text inside a backtick span stays as written (only the table pipe is escaped).
const prose = (p) => p.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\|/g, "\\|").replace(/\*/g, "\\*");
const esc = (s) => {
  const one = String(s).replace(/\n/g, " ");
  const parts = one.split("`");
  if (parts.length % 2 === 0) return prose(one.replace(/`/g, "'"));
  return parts.map((p, i) => (i % 2 ? `\`${p.replace(/\|/g, "\\|")}\`` : prose(p))).join("");
};
const code = (s) => (String(s).includes("`") ? `\`\` ${String(s).replace(/\|/g, "\\|")} \`\`` : `\`${String(s).replace(/\|/g, "\\|")}\``);

// Ranges for one argument: the text after the name that holds a `a..=b`, `at most n`, `at least n` or a comparison.
const RANGE = /-?\d[\d_.]*\s*\.\.=?\s*-?[\dA-Za-z_.]+|(?:at most|at least|up to|no more than|no fewer than)\s+-?[\d_.]*\d|[<>]=?\s*-?\d[\d_.]*/g;
// The window after each mention of the name runs to the end of the sentence, 140 characters, or the next argument's name.
export function rangesFor(field, texts, others = []) {
  const out = [];
  const word = (n) => String.raw`(?<![A-Za-z0-9_])${n}(?![A-Za-z0-9_])`;
  const stop = others.length ? new RegExp(others.map(word).join("|")) : null;
  for (const t of texts) {
    for (const m of t.matchAll(new RegExp(word(field.name), "g"))) {
      const raw = t.slice(m.index + field.name.length, m.index + field.name.length + 140);
      let window = raw.split(/\. |\n/)[0];
      if (window === raw && raw.length === 140) window = raw.slice(0, raw.lastIndexOf(" ")); // never end inside a number
      const cut = stop ? stop.exec(window) : null;
      if (cut) window = window.slice(0, cut.index);
      for (const r of window.matchAll(RANGE)) { const v = squash(r[0]); if (!out.includes(v)) out.push(v); }
    }
  }
  return out;
}

function undoLine(v) {
  if (v.card === "revert") return "this verb is the undo";
  if (v.readonly === true) return "none: read-only";
  const sentence = v.description.split(/(?<=[.;])\s+/).find((s) => /\bundo/i.test(s));
  if (sentence) return `description: "${squash(sentence)}"`;
  if (v.journaled) return "journaled: the handler calls `apply_gated`, so core `tx.undo` reverts the newest batch";
  return "not stated in the source";
}

function scopeLine(v, model) {
  const bits = [`\`${v.card}\``];
  if (v.readonly === true) bits.push("read-only");
  else if (v.readonly === false) bits.push("writes");
  if (v.pack === null) bits.push("always advertised");
  if (v.gates.length) bits.push(`cargo feature ${v.gates.map((g) => `\`${g}\``).join(", ")}`);
  return bits.join(" · ");
}

function verbSection(v, model, packLoaded) {
  const L = [];
  L.push(`### \`${v.name}\``, "");
  L.push(`> ${esc(v.description || "(no description in the source)")}`, "");
  L.push(`- **Scope:** ${scopeLine(v, model)}${v.pack && packLoaded === false ? ` · pack \`${v.pack}\` is listed after \`api.load\`` : ""}`);
  L.push(`- **Undo:** ${undoLine(v)}`);
  L.push(`- **Source:** ${code(`${SRC_DIR}/${v.file}`)}`);
  if (v.outputFields?.length) L.push(`- **Returns:** ${code(v.outputType)} with ${v.outputFields.map((f) => code(f)).join(", ")}`);
  else if (v.outputType) L.push(`- **Returns:** ${code(v.outputType)}`);
  L.push("");
  const texts = [v.description, ...v.refusals.map((r) => r.message)];
  if (v.args && v.args.length) {
    L.push(`**Arguments** (${code(v.argsType)}):`, "", "| Name | Type | Required | Default | Range quoted from the source |", "|---|---|---|---|---|");
    for (const f of v.args) {
      const ranges = rangesFor(f, texts, v.args.filter((o) => o !== f).map((o) => o.name));
      const note = [f.alias.length ? `alias ${f.alias.map((a) => code(a)).join(", ")}` : "", f.decimal ? "decimal string on the wire" : "", f.docs].filter(Boolean).join("; ");
      L.push(`| ${code(f.name)} | ${code(f.type)} | ${f.optional ? "no" : "yes"} | ${f.default ? code(f.default) : ""} | ${[ranges.map((r) => code(r)).join(", "), esc(note)].filter(Boolean).join("; ")} |`);
    }
    L.push("");
  } else if (v.args) L.push(`**Arguments:** none (${code(v.argsType)}).`, "");
  else L.push(`**Arguments:** ${code(v.argsType ?? "?")} (the parser found no struct for it).`, "");
  if (v.refusals.length) {
    const table = ["| Code | Message |", "|---|---|", ...v.refusals.map((r) => `| ${code(r.code)} | ${esc(r.message)} |`)];
    if (v.refusals.length > 8) L.push(`<details><summary>${v.refusals.length} typed refusals</summary>`, "", ...table, "", "</details>", "");
    else L.push("**Typed refusals:**", "", ...table, "");
  } else L.push("**Typed refusals:** none found in the handler source (see Coverage at the end of the page).", "");
  return L;
}

export function renderPage(model) {
  const L = [];
  L.push("# Door reference", "");
  L.push("<!-- Generated by `node tools/door-reference.mjs --write` from crates/endpoint/src. Do not edit by hand: CI fails when this page differs from the generated one. -->", "");
  L.push(`This page lists every tool the AI door answers: ${model.counts.core} core tools and ${model.counts.verbs} verbs in ${model.counts.packs} packs. It is printed from the Rust source by \`tools/door-reference.mjs\`, so a renamed argument, a new refusal or a new verb changes the page; \`node tools/door-reference.mjs --check\` fails when the checked-in file is stale. Start with the [AI door page](ai-door.md) for how a session works.`, "");
  L.push("## How the page is made", "");
  L.push("The generator is a small text parser, not a running server. It reads the `e2_tool!` macros and `impl ToolBase` blocks, the `VerbSpec::of` registrations, the argument structs and the refusal sites in the handler of each verb and in the same-crate functions that handler calls (up to 500, with the visited set stopping a cycle). What it quotes is what the source says:", "");
  L.push("- **Scope** is the capability card the verb registers with, plus the read-only hint of its annotations.");
  for (const [k, text] of Object.entries(model.cards)) if (k !== "deny") L.push(`  - \`${k}\`: ${esc(text)}`);
  L.push("- **Ranges** are quoted from the description and from the refusal messages that name the argument. The handler's validation is the authority; a range the source never writes in text does not appear here.");
  L.push("- **Typed refusals** are the `code` plus the message template of each refusal site. A `{NAME}` that is a constant in the endpoint crate or a workspace crate it names is filled in, and an integer expression over constants is evaluated; other placeholders stay as written.");
  L.push("- **Undo** is `none` for a read-only verb and `this verb is the undo` for a `revert` verb. Otherwise it quotes the description sentence that says undo, or says `journaled` when the handler calls `apply_gated` (the path of `tx.apply`, which `tx.undo` reverts), or says `not stated in the source`.", "");
  L.push("## Refusals every call can get", "");
  if (model.doorWide.length) { L.push("From the pack dispatch (`packs/mod.rs`):", "", "| Code | Message |", "|---|---|", ...model.doorWide.map((r) => `| ${code(r.code)} | ${esc(r.message)} |`), ""); }
  if (model.gateCodes.length) {
    L.push("From the capability gate (`tool_capabilities.rs`, the refused cases of its adversarial briefs), seen on the `commit` and `propose` verbs:", "", "| Code | First brief that triggers it |", "|---|---|", ...model.gateCodes.map((g) => `| ${code(g.code)} | ${code(g.brief)} |`), "");
  }
  L.push("## Core tools", "", "These are always advertised.", "", "| Tool | Scope | Description |", "|---|---|---|");
  for (const v of model.core) L.push(`| [${code(v.name)}](#${anchor(v.name)}) | ${scopeLine(v, model)} | ${esc(firstSentence(v.description))} |`);
  L.push("");
  for (const v of model.core) L.push(...verbSection(v, model, true));
  for (const p of model.packs) {
    L.push(`## Pack \`${p.name}\``, "");
    L.push(`${esc(p.summary)}`, "", `Listed in a fresh session: ${p.loaded === true ? "yes" : p.loaded === false ? `no, call \`api.load\` with \`${p.name}\`` : "unknown"}. Source: ${code(`${SRC_DIR}/${p.file}`)}.`, "");
    L.push("| Verb | Scope | Description |", "|---|---|---|");
    for (const v of p.verbs) L.push(`| [${code(v.name)}](#${anchor(v.name)}) | ${scopeLine(v, model)} | ${esc(firstSentence(v.description))} |`);
    L.push("");
    for (const v of p.verbs) L.push(...verbSection(v, model, p.loaded));
  }
  const all = [...model.core, ...model.packs.flatMap((p) => p.verbs)];
  const thin = all.filter((v) => !v.refusals.length).map((v) => v.name);
  L.push("## Coverage", "");
  L.push(`${all.length} tools parsed, ${all.reduce((n, v) => n + v.refusals.length, 0)} refusal rows printed, ${model.unparsed.length} declarations the parser could not read.`);
  L.push("A verb whose handler converts a typed error defined in another file (the log tools, the level builder) shows fewer refusals than it can return, and a handler whose call graph passes the 500-function walk stops there; the tests next to its source are the full list.");
  if (thin.length) L.push("", `Tools with no refusal site found: ${thin.map((n) => code(n)).join(", ")}.`);
  if (model.unparsed.length) L.push("", "Not parsed:", ...model.unparsed.map((u) => `- ${esc(u)}`));
  L.push("");
  return `${L.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n`;
}

const anchor = (name) => name.toLowerCase().replace(/[^a-z0-9_ -]/g, "").replace(/ /g, "-");
const firstSentence = (s) => {
  const t = squash(s);
  const m = /^(.*?\.)(\s|$)/.exec(t);
  const one = m ? m[1] : t;
  return one.length > 170 ? `${one.slice(0, one.lastIndexOf(" ", 167))}...` : one;
};

// ---- files ----
export function readCrate(root = ROOT) {
  const base = join(root, SRC_DIR);
  const files = new Map();
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".rs")) files.set(relative(base, p).split(sep).join("/"), readFileSync(p, "utf8"));
    }
  };
  walk(base);
  // The fighter contract ranges `play.run` imports from engine-input (MCP22): read so the page still quotes their numbers.
  const contract = join(root, "crates/input/src/fighter.rs");
  if (existsSync(contract)) files.set("../../input/src/fighter.rs", readFileSync(contract, "utf8"));
  return files;
}

// A refusal can quote a value imported from another crate (`WISH_RANGE`, `MAX_MATCH_TICKS`).
// Index the other crates' consts so the page prints the number; a name that conflicts across
// crates stays a placeholder. The endpoint crate is already indexed, so it is skipped.
function mergeWorkspaceConsts(idx, root) {
  const crates = join(root, "crates");
  if (!existsSync(crates)) return;
  const endpointSrc = resolve(root, SRC_DIR);
  const files = new Map();
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".rs")) files.set(relative(root, p).split(sep).join("/"), readFileSync(p, "utf8"));
    }
  };
  for (const e of readdirSync(crates, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    const src = join(crates, e.name, "src");
    if (existsSync(src) && resolve(src) !== endpointSrc) walk(src);
  }
  for (const [name, rows] of indexCrate(files).consts) {
    if (!idx.consts.has(name)) idx.consts.set(name, []);
    idx.consts.get(name).push(...rows);
  }
}

export const generate = (root = ROOT) => renderPage(buildModel(readCrate(root), root));

function main(argv) {
  const write = argv.includes("--write");
  const check = argv.includes("--check");
  const unknown = argv.filter((a) => !["--write", "--check"].includes(a));
  if (unknown.length) { console.error(`door-reference: unknown argument ${unknown[0]}\nusage: node tools/door-reference.mjs [--write | --check]`); return 2; }
  const page = generate();
  const path = join(ROOT, PAGE_PATH);
  if (write) { writeFileSync(path, page); console.log(`wrote ${PAGE_PATH}: ${page.split("\n").length - 1} lines`); return 0; }
  if (check) {
    const have = existsSync(path) ? readFileSync(path, "utf8").replace(/\r\n/g, "\n") : "";
    if (have === page) { console.log(`${PAGE_PATH} matches the source`); return 0; }
    console.error(`${PAGE_PATH} is stale: run node tools/door-reference.mjs --write`);
    return 1;
  }
  process.stdout.write(page);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv.slice(2));
