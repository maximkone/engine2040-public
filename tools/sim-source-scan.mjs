// sim-source-scan: comment- and string-aware Rust scans for the workspace audit gates (sim determinism, GE2 genre ledger). Library only: imported by tools/workspace-check.mjs.
//
// Why this exists: both gates were raw regexes over source text.
// - Determinism bans `.sin(`-style calls in simulation crates, but `Fixed`
//   (crates/core/src/fixed.rs) has table-based integer `sin`/`cos`/`tan`, so a
//   text match cannot tell `Fixed::sin` from `f32::sin`. A hit is now skipped only
//   when the receiver is positively shown to be `Fixed`; anything unproven
//   still counts, so a real `f32` call keeps failing. The type-aware ban for the
//   core methods is tools/sim-clippy.toml (CI runs it); this leg stays as the wider
//   text net (sinh, cbrt, log10, f64::tan ...) that the clippy list does not carry.
// - GE2 matched genre words inside comments and string literals. It now reads
//   identifier tokens of code only.

const IDENT = /[A-Za-z0-9_]/;
const OPENERS = { "(": ")", "[": "]", "{": "}" };
const CLOSERS = { ")": "(", "]": "[", "}": "{" };

/** Replace comments and string/char literal text with spaces, keeping every
 * newline and every offset, so line numbers and indices stay valid. */
export function maskRust(src) {
  const n = src.length;
  const out = src.split("");
  const blank = (from, to) => {
    for (let k = from; k < to && k < n; k++) if (out[k] !== "\n" && out[k] !== "\r") out[k] = " ";
  };
  let i = 0;
  while (i < n) {
    const c = src[i];
    const d = src[i + 1];
    if (c === "/" && d === "/") {
      let j = i;
      while (j < n && src[j] !== "\n") j++;
      blank(i, j);
      i = j;
    } else if (c === "/" && d === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < n && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") { depth++; j += 2; }
        else if (src[j] === "*" && src[j + 1] === "/") { depth--; j += 2; }
        else j++;
      }
      blank(i, j);
      i = j;
    } else if ((c === "r" || (c === "b" && d === "r")) && !IDENT.test(src[i - 1] ?? " ")) {
      // Raw string r"..", r#".."#, br#".."#; otherwise an ordinary identifier.
      const m = /^b?r(#*)"/.exec(src.slice(i, i + 40));
      if (!m) { i++; continue; }
      const close = `"${m[1]}`;
      const end = src.indexOf(close, i + m[0].length);
      const stop = end < 0 ? n : end + close.length;
      blank(i, stop);
      i = stop;
    } else if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
      blank(i, j + 1);
      i = j + 1;
    } else if (c === "'") {
      // Char literal or lifetime: a literal closes within one code point.
      let stop = -1;
      if (src[i + 1] === "\\") {
        const end = src.indexOf("'", i + 3);
        if (end > 0) stop = end + 1;
      } else {
        const cp = src.codePointAt(i + 1);
        const width = cp !== undefined && cp > 0xffff ? 2 : 1;
        if (src[i + 1 + width] === "'") stop = i + 2 + width;
      }
      if (stop > 0) { blank(i, stop); i = stop; } else i++;
    } else {
      i++;
    }
  }
  return out.join("");
}

/** Code before the first `#[cfg(test)]` with comments and strings masked.
 * Tests sit after it by convention and may use any math or names. */
export function productionCode(src) {
  return maskRust(src).split("#[cfg(test)]")[0];
}

// ---------------------------------------------------------------- GE2 genre

const GENRE_RAW = /Fighter|Gladiator|gladiator/;

/** Identifiers in production code that contain a genre word, sorted. A genre
 * word in a comment, doc line or string literal does not count. */
export function genreHits(src) {
  if (!GENRE_RAW.test(src)) return [];
  const found = new Set();
  for (const m of productionCode(src).matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    if (GENRE_RAW.test(m[0])) found.add(m[0]);
  }
  return [...found].sort();
}

// ------------------------------------------------------------- determinism

export const PLATFORM_MATH_METHODS = [
  "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh",
  "exp", "exp2", "exp_m1", "ln", "ln_1p", "log", "log2", "log10", "powf", "cbrt",
  "hypot", "sin_cos",
];
const MATH_NAMES = PLATFORM_MATH_METHODS.join("|");
const MATH_RAW = new RegExp(`\\.\\s*(?:${MATH_NAMES})\\s*\\(`);

/** What `Fixed` offers, read from its source so the scan follows the type:
 * `consts` (`Fixed::ONE`), `ctors` (`Fixed::from_int(..)`), `methods` taking
 * self and returning Fixed, `selfFns` every method taking self. */
export function deriveFixedApi(fixedSrc) {
  const api = { consts: new Set(), ctors: new Set(), methods: new Set(), selfFns: new Set() };
  const code = maskRust(fixedSrc);
  for (const block of code.matchAll(/\bimpl\s+Fixed\s*\{/g)) {
    const open = block.index + block[0].length - 1;
    const close = matchForward(code, open);
    if (close < 0) continue;
    const body = code.slice(open + 1, close);
    for (const m of body.matchAll(/\bpub\s+const\s+([A-Z][A-Z0-9_]*)\s*:\s*(?:Self|Fixed)\s*=/g)) {
      api.consts.add(m[1]);
    }
    for (const m of body.matchAll(/\bpub\s+(?:const\s+)?fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(->\s*[^{;]*)?[{;]/g)) {
      const takesSelf = /^\s*(?:&\s*(?:mut\s+)?|mut\s+)?self\b/.test(m[2]);
      const ret = (m[3] ?? "").replace(/^->\s*/, "").replace(/\bwhere\b[\s\S]*$/, "").trim();
      const returnsFixed = ret === "Self" || ret === "Fixed";
      if (takesSelf) api.selfFns.add(m[1]);
      if (takesSelf && returnsFixed) api.methods.add(m[1]);
      if (!takesSelf && returnsFixed) api.ctors.add(m[1]);
    }
  }
  return api;
}

const EMPTY_API = { consts: new Set(), ctors: new Set(), methods: new Set(), selfFns: new Set() };

function matchForward(code, openIdx) {
  const stack = [];
  for (let i = openIdx; i < code.length; i++) {
    const c = code[i];
    if (OPENERS[c]) stack.push(OPENERS[c]);
    else if (c === ")" || c === "]" || c === "}") {
      if (stack.pop() !== c) return -1;
      if (stack.length === 0) return i;
    }
  }
  return -1;
}

function matchBack(code, closeIdx) {
  const stack = [];
  for (let i = closeIdx; i >= 0; i--) {
    const c = code[i];
    if (CLOSERS[c]) stack.push(CLOSERS[c]);
    else if (c === "(" || c === "[" || c === "{") {
      if (stack.pop() !== c) return -1;
      if (stack.length === 0) return i;
    }
  }
  return -1;
}

const skipWsBack = (code, i) => {
  while (i > 0 && /\s/.test(code[i - 1])) i--;
  return i;
};
const identStartBack = (code, i) => {
  while (i > 0 && IDENT.test(code[i - 1])) i--;
  return i;
};

/** Index where the receiver expression of the method call at `dot` begins,
 * or -1 when it cannot be delimited (the caller then treats it as unproven). */
function receiverStart(code, dot) {
  let i = dot;
  for (let guard = 0; guard < 64; guard++) {
    const j = skipWsBack(code, i);
    const c = code[j - 1];
    if (c === undefined) return -1;
    if (c === ")" || c === "]") {
      const open = matchBack(code, j - 1);
      if (open < 0) return -1;
      i = open;
      const prev = code[open - 1];
      // Macro calls and calls of calls/closures return unknown types.
      if (prev === "!" || (c === ")" && (prev === ")" || prev === "]"))) return -1;
      if (c === ")" && prev !== undefined && (IDENT.test(prev) || prev === ">")) {
        let k = open;
        if (prev === ">") {
          // Turbofish callee `name::<T>(..)`.
          let depth = 0;
          let m = open - 1;
          for (; m >= 0; m--) {
            if (code[m] === ">") depth++;
            else if (code[m] === "<" && --depth === 0) break;
          }
          if (m < 2 || code[m - 1] !== ":" || code[m - 2] !== ":") return -1;
          k = m - 2;
        }
        const s = identStartBack(code, k);
        if (s === k) return -1;
        i = s;
      } else if (c === "]" && prev !== undefined && (IDENT.test(prev) || prev === ")" || prev === "]")) {
        continue; // indexing: keep walking to the indexed base
      } else {
        return open; // parenthesised group or array literal
      }
    } else if (IDENT.test(c)) {
      i = identStartBack(code, j);
    } else {
      return -1;
    }
    const p = skipWsBack(code, i);
    if (code[p - 1] === "." && code[p - 2] !== ".") { i = p - 1; continue; }
    if (code[p - 1] === ":" && code[p - 2] === ":") { i = p - 2; continue; }
    return i;
  }
  return -1;
}

// --- type evidence: "fixed" only with proof, "other" for everything else

const unanimousFixed = (kinds) => !!kinds && kinds.size === 1 && kinds.has("fixed");
const addKind = (table, name, kind) => {
  let set = table.get(name);
  if (!set) table.set(name, (set = new Set()));
  set.add(kind);
};

// `name: Type` anywhere: params, fields, typed lets, consts, closure params.
const TYPED_DECL = /(?<![\w:'])([A-Za-z_]\w*)\s*:(?!:)\s*([^,;=(){}|\n]*)/g;

function typeKind(text, api) {
  const t = text.trim().replace(/^&\s*(?:'\w+\s+)?(?:mut\s+)?/, "").trim();
  if (t === "Fixed") return "fixed";
  const m = /^Fixed::(\w+)$/.exec(t); // struct-literal initialiser `x: Fixed::ZERO`
  return m && (api.consts.has(m[1]) || api.ctors.has(m[1])) ? "fixed" : "other";
}

function typedDecls(text, api, table = new Map()) {
  for (const m of text.matchAll(TYPED_DECL)) addKind(table, m[1], typeKind(m[2], api));
  return table;
}

/** Every `fn name(..) -> Ret` in the file: name -> kinds, plus body spans. */
function scanFns(code) {
  const returns = new Map();
  const spans = [];
  for (const m of code.matchAll(/\bfn\s+([A-Za-z_]\w*)\s*(?:<(?:[^<>(]|<[^<>]*>)*>)?\s*\(/g)) {
    const open = m.index + m[0].length - 1;
    const close = matchForward(code, open);
    if (close < 0) continue;
    let k = close + 1;
    let depth = 0;
    for (; k < code.length; k++) {
      const c = code[k];
      if (c === "(" || c === "[") depth++;
      else if (c === ")" || c === "]") depth--;
      else if (depth === 0 && (c === "{" || c === ";")) break;
    }
    const header = code.slice(close + 1, k).replace(/\bwhere\b[\s\S]*$/, "").trim();
    const ret = header.startsWith("->") ? header.slice(2).trim() : "";
    addKind(returns, m[1], ret === "Fixed" ? "fixed" : "other");
    if (code[k] === "{") {
      const end = matchForward(code, k);
      if (end > 0) spans.push({ start: m.index, end: end + 1 });
    }
  }
  return { returns, spans };
}

function readUntilSemicolon(text, from) {
  let depth = 0;
  for (let i = from; i < text.length; i++) {
    const c = text[i];
    if (OPENERS[c]) depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    else if (c === ";" && depth <= 0) return text.slice(from, i);
  }
  return text.slice(from);
}

// Names bound in ways whose type we do not track (patterns, loops, closures).
function untrackedBindings(scope, table) {
  const other = (text) => {
    for (const m of text.matchAll(/[A-Za-z_]\w*/g)) addKind(table, m[0], "other");
  };
  for (const m of scope.matchAll(/\blet\s+(?:mut\s+)?([(\[A-Z][^=;]*?)=(?!=)/g)) other(m[1]);
  for (const m of scope.matchAll(/\bfor\s+([^{]*?)\s+in\b/g)) other(m[1]);
  for (const m of scope.matchAll(/\|([^|\n]*)\|/g)) {
    for (const part of m[1].split(",")) if (!part.includes(":")) other(part);
  }
  for (const m of scope.matchAll(/[A-Za-z_]\w*\s*\(([^()]*)\)\s*(?:if\b[^=]*)?=>/g)) other(m[1]);
}

function fnTableFor(scope, ctx) {
  const table = typedDecls(scope, ctx.api);
  untrackedBindings(scope, table);
  const env = { ...ctx, fnTable: table };
  for (const m of scope.matchAll(/\blet\s+(?:mut\s+)?([A-Za-z_]\w*)\s*=(?!=)/g)) {
    const rhs = readUntilSemicolon(scope, m.index + m[0].length);
    addKind(table, m[1], evalExpr(rhs, env));
  }
  return table;
}

function lookupName(name, env) {
  const local = env.fnTable.get(name);
  if (local) return local;
  // File-level consts and statics are visible in every fn.
  return /^[A-Z][A-Z0-9_]*$/.test(name) ? env.fileTable.get(name) : undefined;
}

function baseKind(path, called, env) {
  const { api } = env;
  if (path.length === 1) {
    return unanimousFixed(called ? env.fnReturns.get(path[0]) : lookupName(path[0], env)) ? "fixed" : "other";
  }
  if (path[0] === "Fixed" && path.length === 2) {
    return (called ? api.ctors.has(path[1]) || api.methods.has(path[1]) : api.consts.has(path[1])) ? "fixed" : "other";
  }
  return called && unanimousFixed(env.fnReturns.get(path[path.length - 1])) ? "fixed" : "other";
}

function methodKind(base, name, env) {
  if (base === "fixed" && env.api.methods.has(name)) return "fixed";
  return unanimousFixed(env.fnReturns.get(name)) ? "fixed" : "other";
}

// A receiver chain: `a`, `self.x`, `Fixed::PI`, `f(x).abs()`, `(a * b)`.
function evalChain(text, env) {
  const s = text.trim();
  let pos = 0;
  let kind;
  if (s[0] === "(") {
    const close = matchForward(s, 0);
    if (close < 0) return "other";
    kind = evalExpr(s.slice(1, close), env);
    pos = close + 1;
  } else {
    const m = /^[A-Za-z_]\w*(?:\s*::\s*[A-Za-z_]\w*)*/.exec(s);
    if (!m) return "other";
    pos = m[0].length;
    let called = false;
    if (s[pos] === "(") {
      const close = matchForward(s, pos);
      if (close < 0) return "other";
      called = true;
      pos = close + 1;
    }
    kind = baseKind(m[0].replace(/\s+/g, "").split("::"), called, env);
  }
  while (pos < s.length) {
    const m = /^\s*\.\s*([A-Za-z_]\w*)/.exec(s.slice(pos));
    if (!m) return "other"; // index, `?`, `as`, tuple field, anything unproven
    pos += m[0].length;
    if (s[pos] === "(") {
      const close = matchForward(s, pos);
      if (close < 0) return "other";
      pos = close + 1;
      kind = methodKind(kind, m[1], env);
    } else {
      kind = unanimousFixed(env.fileTable.get(m[1])) ? "fixed" : "other";
    }
  }
  return kind;
}

// Arithmetic: `Fixed` has no mixed-type operators, so one proven operand
// makes the whole expression Fixed; comparisons, casts and blocks are "other".
function evalExpr(text, env) {
  const s = text.trim();
  const operands = [];
  let depth = 0;
  let last = 0;
  let prevSig = "";
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (OPENERS[c]) depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    else if (depth === 0) {
      if ("+-*/%".includes(c)) {
        if (/[\w)\]]/.test(prevSig) && !(c === "-" && s[i + 1] === ">")) {
          operands.push(s.slice(last, i));
          last = i + 1;
        }
      } else if (c === "&" && /[\w)\]]/.test(prevSig)) {
        return "other";
      } else if ("<>=!|,;?".includes(c)) {
        return "other";
      }
    }
    if (!/\s/.test(c)) prevSig = c;
  }
  operands.push(s.slice(last));
  const kinds = operands.map((op) => evalChain(op.replace(/^(?:[-!*&]\s*|mut\s+)+/, ""), env));
  return kinds.includes("fixed") ? "fixed" : "other";
}

function fileContext(code, api) {
  const { returns, spans } = scanFns(code);
  return {
    api,
    fnReturns: returns,
    fileTable: typedDecls(code, api),
    spans,
    fnTables: new Map(),
    importsFixed: /\buse\s+[^;]*\bFixed\b/.test(code) && !/\b(?:struct|enum|type|trait)\s+Fixed\b/.test(code),
  };
}

function isFixedReceiver(code, dot, method, ctx) {
  if (!ctx.importsFixed || !ctx.api.selfFns.has(method)) return false;
  const start = receiverStart(code, dot);
  if (start < 0) return false;
  let span = null;
  for (const s of ctx.spans) if (s.start <= dot && dot < s.end && (!span || s.start > span.start)) span = s;
  let fnTable = new Map();
  if (span) {
    fnTable = ctx.fnTables.get(span);
    if (!fnTable) {
      fnTable = fnTableFor(code.slice(span.start, span.end), ctx);
      ctx.fnTables.set(span, fnTable);
    }
  }
  return evalChain(code.slice(start, dot), { ...ctx, fnTable }) === "fixed";
}

/** Platform float transcendental calls in production code that are not shown
 * to be `Fixed` math. Each hit: `{ method, line, receiver }`. */
export function platformMathHits(src, api = EMPTY_API) {
  if (!MATH_RAW.test(src)) return [];
  const code = productionCode(src);
  const hits = [];
  let ctx = null;
  for (const m of code.matchAll(new RegExp(`\\.\\s*(${MATH_NAMES})\\s*\\(`, "g"))) {
    ctx ??= fileContext(code, api);
    if (isFixedReceiver(code, m.index, m[1], ctx)) continue;
    const start = receiverStart(code, m.index);
    hits.push({
      method: m[1],
      line: code.slice(0, m.index).split("\n").length,
      receiver: start < 0 ? "?" : code.slice(start, m.index).replace(/\s+/g, " ").trim(),
    });
  }
  return hits;
}
