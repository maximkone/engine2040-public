// What a .wasm module declares, read from its binary sections (no wasm tools needed).
//   node tools/wasm-info.mjs dist/web/engine_bg.wasm
// Prints one JSON object: imports by kind, tables and memories with their limits, the names of
// exported tables and memories, and the target features the producer recorded. WEB4 uses it to
// see what wasm-opt changed (a table whose maximum was lowered cannot grow in the browser).
import { readFileSync } from "node:fs";

class Reader {
  constructor(buf, pos = 0, end = buf.length) { this.buf = buf; this.pos = pos; this.end = end; }
  byte() {
    if (this.pos >= this.end) throw new Error("wasm section ends early");
    return this.buf[this.pos++];
  }
  // Unsigned LEB128 as a Number (every value in the sections read here is far below 2^53).
  u32() {
    let result = 0, mul = 1;
    for (let i = 0; i < 10; i++) {
      const b = this.byte();
      result += (b & 0x7f) * mul;
      if ((b & 0x80) === 0) return result;
      mul *= 128;
    }
    throw new Error("bad LEB128");
  }
  name() {
    const n = this.u32();
    const s = this.buf.toString("utf8", this.pos, this.pos + n);
    this.pos += n;
    return s;
  }
  limits() {
    const flag = this.byte();
    const min = this.u32();
    const max = flag & 1 ? this.u32() : null;
    return { min, max, shared: Boolean(flag & 2), memory64: Boolean(flag & 4) };
  }
}

const REF_TYPES = { 0x70: "funcref", 0x6f: "externref" };
const KINDS = ["func", "table", "memory", "global", "tag"];

export function readWasmInfo(buf) {
  if (buf.length < 8 || buf.readUInt32BE(0) !== 0x0061736d) throw new Error("not a wasm module");
  const info = { imports: { func: 0, table: 0, memory: 0, global: 0, tag: 0 }, tables: [], memories: [], exports: { tables: [], memories: [] }, features: [] };
  let pos = 8;
  while (pos < buf.length) {
    const head = new Reader(buf, pos);
    const id = head.byte();
    const size = head.u32();
    if (head.pos + size > buf.length) throw new Error("wasm section ends early");
    const r = new Reader(buf, head.pos, head.pos + size);
    pos = head.pos + size;
    if (id === 2) {
      for (let n = r.u32(); n > 0; n--) {
        r.name(); r.name();
        const kind = r.byte();
        if (kind === 0 || kind === 4) { if (kind === 4) r.byte(); r.u32(); }
        else if (kind === 1) { r.byte(); info.tables.push({ imported: true, ...r.limits() }); }
        else if (kind === 2) info.memories.push({ imported: true, ...r.limits() });
        else if (kind === 3) { r.byte(); r.byte(); }
        else throw new Error(`unknown import kind ${kind}`);
        info.imports[KINDS[kind]] += 1;
      }
    } else if (id === 4) {
      for (let n = r.u32(); n > 0; n--) {
        const type = REF_TYPES[r.byte()] ?? "other";
        info.tables.push({ imported: false, type, ...r.limits() });
      }
    } else if (id === 5) {
      for (let n = r.u32(); n > 0; n--) info.memories.push({ imported: false, ...r.limits() });
    } else if (id === 7) {
      for (let n = r.u32(); n > 0; n--) {
        const name = r.name(), kind = r.byte();
        r.u32();
        if (kind === 1) info.exports.tables.push(name);
        else if (kind === 2) info.exports.memories.push(name);
      }
    } else if (id === 0) {
      if (r.name() === "target_features") {
        for (let n = r.u32(); n > 0; n--) { const prefix = String.fromCharCode(r.byte()); info.features.push(prefix + r.name()); }
      }
    }
  }
  return info;
}

if (import.meta.main) {
  const file = process.argv[2];
  if (!file) { console.error("usage: node tools/wasm-info.mjs <module.wasm>"); process.exit(2); }
  console.log(JSON.stringify(readWasmInfo(readFileSync(file))));
}
