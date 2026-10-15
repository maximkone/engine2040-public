// Run: node tools/float-gate.test.mjs
// Run: node --test tools/float-gate.test.mjs
// float-gate: no f32/f64 in the sim tick+hash warm path.
// Owner: tools. Units: source lines scanned; violations are `file:line: [scope] text`.
// Errors: missing paths skip (no throw); bad input types throw TypeError, never silent.
// Warm path (docs/HASH-LAW.md, crates/core/src/lib.rs): the per-tick driver and
// hash fold stay integer-only so every machine replays bit-exact. Frozen f32
// receipts (LegacyFixedAcc, DT), canonical bit-writers (write_f32/write_f64),
// the wall poll boundary (push_secs, quantize_wall_secs) and render alpha stay
// allowlisted by scope: they never enter a simulation hash.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), ".."));

// Tick+hash path: the four files whose warm per-tick code must stay integer.
// allowFn holds qualified scopes where floats are lawful (receipt, bit-writer,
// poll boundary, render alpha); allowOuter holds the frozen-const pattern for
// lines outside any fn body (null = no float line allowed outside fns).
export const TICK_HASH_FILES = [
  {
    file: "crates/core/src/tick.rs",
    allowFn: new Set([
      "LegacyFixedAcc::new",
      "LegacyFixedAcc::fixed",
      "LegacyFixedAcc::push",
      "LegacyFixedAcc::alpha",
    ]),
    allowOuter: /LEGACY|LegacyFixedAcc|FixedAcc|\bDT\b|MAX_FRAME_DT|pub\s+(acc|dt)\s*:/,
  },
  {
    file: "crates/core/src/hash.rs",
    allowFn: new Set(["Fnv64::write_f32", "Fnv64::write_f64"]),
    allowOuter: null,
  },
  {
    file: "crates/core/src/fixed.rs",
    allowFn: new Set(),
    allowOuter: null,
  },
  {
    file: "crates/core/src/time.rs",
    allowFn: new Set(["IntAcc::alpha", "IntAcc::push_secs", "quantize_wall_secs"]),
    allowOuter: null,
  },
];

export const FLOAT_RE = /\bf(?:32|64)\b|\b\d[\d_]*(?:\.[\d_]*)?f(?:32|64)\b/;

// Blank strings and comments (same length, newlines kept) so the scan only
// sees code. Block comments nest, escapes and chars are handled, and a missed
// edge blanks too little and fails closed, never green.
export function stripRust(text) {
  if (typeof text !== "string") throw new TypeError("stripRust needs a string");
  const out = [];
  let i = 0;
  const n = text.length;
  const blank = (ch) => {
    out.push(ch === "\n" ? "\n" : " ");
  };
  while (i < n) {
    const c = text[i];
    const d = text[i + 1];
    if (c === "/" && d === "/") {
      blank(c);
      blank(d);
      i += 2;
      while (i < n && text[i] !== "\n") {
        blank(text[i]);
        i++;
      }
      continue;
    }
    if (c === "/" && d === "*") {
      let depth = 0;
      while (i < n) {
        if (text[i] === "/" && text[i + 1] === "*") {
          blank(text[i]);
          blank(text[i + 1]);
          i += 2;
          depth++;
          continue;
        }
        if (text[i] === "*" && text[i + 1] === "/") {
          blank(text[i]);
          blank(text[i + 1]);
          i += 2;
          depth--;
          if (depth === 0) break;
          continue;
        }
        blank(text[i]);
        i++;
      }
      continue;
    }
    // Raw string r"..." or r#"..."# (only when r starts an item, not an ident).
    if (c === "r" && !/[A-Za-z0-9_]/.test(text[i - 1] ?? "")) {
      const m = /^r(#+)"/.exec(text.slice(i, i + 32));
      if (m) {
        const close = `"${m[1]}`;
        for (let k = 0; k < m[0].length; k++) blank(text[i + k]);
        i += m[0].length;
        while (i < n && text.slice(i, i + close.length) !== close) {
          blank(text[i]);
          i++;
        }
        for (let k = 0; k < close.length && i < n; k++) {
          blank(text[i]);
          i++;
        }
        continue;
      }
      if (d === '"') {
        blank(c);
        blank(d);
        i += 2;
        while (i < n && text[i] !== '"') {
          blank(text[i]);
          i++;
        }
        if (i < n) {
          blank(text[i]);
          i++;
        }
        continue;
      }
    }
    if (c === '"') {
      blank(c);
      i++;
      while (i < n && text[i] !== '"') {
        if (text[i] === "\\") {
          blank(text[i]);
          i++;
          if (i < n) {
            blank(text[i]);
            i++;
          }
          continue;
        }
        blank(text[i]);
        i++;
      }
      if (i < n) {
        blank(text[i]);
        i++;
      }
      continue;
    }
    // Char literal ('x', '\n') versus lifetime ('a, 'static).
    if (c === "'") {
      const m = /^'(?:\\.|[^'\\\n])'/.exec(text.slice(i, i + 8));
      if (m) {
        for (let k = 0; k < m[0].length; k++) blank(text[i + k]);
        i += m[0].length;
        continue;
      }
      out.push(c);
      i++;
      continue;
    }
    out.push(c);
    i++;
  }
  return out.join("");
}

// Scan one Rust source for f32/f64 in warm tick+hash scopes. Test modules
// (after the first cfg(test)) never count: tests may use any math.
export function scanSource(relativePath, text, rule) {
  if (typeof relativePath !== "string" || typeof text !== "string" || !rule || typeof rule !== "object") {
    throw new TypeError("scanSource needs (path, text, rule) with strings and a rule");
  }
  const allowFn = rule.allowFn instanceof Set ? rule.allowFn : new Set();
  const allowOuter = rule.allowOuter instanceof RegExp ? rule.allowOuter : null;
  const src = text.replace(/\r\n/g, "\n");
  const code = stripRust(src);
  const origLines = src.split("\n");
  const codeLines = code.split("\n");
  let end = codeLines.length;
  for (let i = 0; i < codeLines.length; i++) {
    if (/#\[cfg\(.*\btest\b/.test(codeLines[i])) {
      end = i;
      break;
    }
  }
  const violations = [];
  const implStack = [];
  let fn = null;
  let depth = 0;
  for (let i = 0; i < end; i++) {
    const line = codeLines[i];
    const implName = /^\s*impl\b/.test(line)
      ? ((/\bfor\s+([A-Za-z_]\w*)/.exec(line) ?? [])[1] ?? (/^\s*impl\s+([A-Za-z_]\w*)/.exec(line) ?? [])[1] ?? null)
      : null;
    if (implName) {
      implStack.push({ name: implName, depth });
      fn = null;
    } else if (fn === null) {
      const fnName = (/\bfn\s+([A-Za-z_]\w*)/.exec(line) ?? [])[1] ?? null;
      if (fnName) fn = { name: fnName, depth };
    }
    const opens = (line.match(/\{/g) ?? []).length;
    const closes = (line.match(/\}/g) ?? []).length;
    depth += opens - closes;
    while (implStack.length > 0 && depth <= implStack[implStack.length - 1].depth) implStack.pop();
    if (fn && depth <= fn.depth) fn = null;
    if (!FLOAT_RE.test(line)) continue;
    const scope = fn ? (implStack.length > 0 ? `${implStack[implStack.length - 1].name}::${fn.name}` : fn.name) : "(outer)";
    const ok = fn ? allowFn.has(scope) : allowOuter !== null && allowOuter.test(line);
    if (!ok) violations.push(`${relativePath}:${i + 1}: [${scope}] ${(origLines[i] ?? "").trim().slice(0, 120)}`);
  }
  return violations;
}

// Read one gate file; a missing path skips cleanly (0 files, 0 violations).
export function scanFile(repoRoot, entry) {
  if (typeof repoRoot !== "string" || !entry || typeof entry.file !== "string") {
    throw new TypeError("scanFile needs (repoRoot, { file }) with strings");
  }
  let text;
  try {
    text = readFileSync(join(repoRoot, entry.file), "utf8");
  } catch {
    return { scanned: 0, violations: [] };
  }
  return { scanned: 1, violations: scanSource(entry.file, text, entry) };
}

export function scanRepo(repoRoot = ROOT) {
  if (typeof repoRoot !== "string") throw new TypeError("scanRepo needs a repoRoot string");
  let scanned = 0;
  const violations = [];
  for (const entry of TICK_HASH_FILES) {
    const r = scanFile(repoRoot, entry);
    scanned += r.scanned;
    violations.push(...r.violations);
  }
  return { scanned, violations };
}

test("stripRust blanks comments and strings but keeps code", () => {
  const stripped = stripRust('let a = "f32"; // f32\n/* f64 */ let b: f32 = 0.0;');
  assert.equal((stripped.match(/\bf32\b/g) ?? []).length, 1);
  assert.match(stripped, /let b: f32/);
  assert.equal(stripped.split("\n").length, 2);
});

test("scanSource flags f32/f64 in a warm tick scope", () => {
  const src = [
    "impl TickState {",
    "    pub fn step(&mut self, h: f32) -> u64 {",
    "        self.acc",
    "    }",
    "}",
  ].join("\n");
  const rule = { allowFn: new Set(), allowOuter: null };
  const found = scanSource("tick.rs", src, rule);
  assert.equal(found.length, 1);
  assert.match(found[0], /tick\.rs:2/);
  assert.match(found[0], /TickState::step/);
  assert.match(found[0], /f32/);
});

test("scanSource passes integer-only warm code", () => {
  const src = [
    "pub fn fold_hash(acc: u64, tick: u64, step_hash: u64) -> u64 {",
    "    let mut h = acc;",
    "    h",
    "}",
  ].join("\n");
  assert.deepEqual(scanSource("tick.rs", src, { allowFn: new Set(), allowOuter: null }), []);
});

test("scanSource allowlists the frozen receipt block", () => {
  const src = [
    "pub const LEGACY_DT: f32 = 1.0 / 60.0;",
    "pub struct LegacyFixedAcc {",
    "    pub acc: f32,",
    "}",
    "impl LegacyFixedAcc {",
    "    pub fn push(&mut self, frame_dt: f32) -> u32 {",
    "        self.acc",
    "    }",
    "}",
  ].join("\n");
  const rule = TICK_HASH_FILES[0];
  assert.deepEqual(scanSource("crates/core/src/tick.rs", src, rule), []);
});

test("scanSource allowlists bit-writers, poll boundary and render alpha", () => {
  const hashSrc = [
    "impl Fnv64 {",
    "    pub fn write_f32(&mut self, value: f32) {",
    "        self.write_u32(value.to_bits());",
    "    }",
    "}",
  ].join("\n");
  assert.deepEqual(scanSource("crates/core/src/hash.rs", hashSrc, TICK_HASH_FILES[1]), []);
  const timeSrc = [
    "impl IntAcc {",
    "    pub fn alpha(&self) -> f32 {",
    "        (0.0) as f32",
    "    }",
    "    pub fn push_ns(&mut self, dt_ns: u64) -> Result<u32, TimeError> {",
    "        Ok(0)",
    "    }",
    "}",
    "pub fn quantize_wall_secs(dt_secs: f32) -> Result<u64, TimeError> {",
    "    Ok(0)",
    "}",
  ].join("\n");
  assert.deepEqual(scanSource("crates/core/src/time.rs", timeSrc, TICK_HASH_FILES[3]), []);
});

test("scanSource flags a new float field and a new float in push_ns", () => {
  const fieldSrc = [
    "pub struct TickState {",
    "    pub tick: u64,",
    "    pub mass: f32,",
    "}",
  ].join("\n");
  const fields = scanSource("crates/core/src/tick.rs", fieldSrc, TICK_HASH_FILES[0]);
  assert.equal(fields.length, 1);
  assert.match(fields[0], /\(outer\)/);
  const pushSrc = [
    "impl IntAcc {",
    "    pub fn push_ns(&mut self, dt_ns: u64) -> Result<u32, TimeError> {",
    "        let rate = 1.0f64;",
    "        Ok(0)",
    "    }",
    "}",
  ].join("\n");
  const pushes = scanSource("crates/core/src/time.rs", pushSrc, TICK_HASH_FILES[3]);
  assert.equal(pushes.length, 1);
  assert.match(pushes[0], /IntAcc::push_ns/);
});

test("scanSource ignores test modules, which may use any math", () => {
  const src = [
    "pub fn fold_hash(acc: u64) -> u64 {",
    "    acc",
    "}",
    "#[cfg(test)]",
    "mod tests {",
    "    fn f32_drift() {",
    "        let x: f32 = 0.01;",
    "    }",
    "}",
  ].join("\n");
  assert.deepEqual(scanSource("tick.rs", src, { allowFn: new Set(), allowOuter: null }), []);
});

test("scanFile skips a missing path cleanly", () => {
  assert.deepEqual(scanFile(ROOT, { file: "crates/core/src/no-such-file.rs", allowFn: new Set(), allowOuter: null }), {
    scanned: 0,
    violations: [],
  });
});

test("gate wires the four tick+hash files", () => {
  assert.deepEqual(
    TICK_HASH_FILES.map((e) => e.file),
    ["crates/core/src/tick.rs", "crates/core/src/hash.rs", "crates/core/src/fixed.rs", "crates/core/src/time.rs"],
  );
});

test("repo: no f32/f64 in sim tick+hash warm path", () => {
  const { scanned, violations } = scanRepo(ROOT);
  assert.equal(scanned, TICK_HASH_FILES.length, `gate scanned ${scanned} of ${TICK_HASH_FILES.length} files`);
  assert.deepEqual(violations, [], `float in tick+hash path:\n${violations.join("\n")}`);
});
