#!/usr/bin/env node
// city-osm.mjs - offline OSM slice to glTF 2.0. Caller: tools/gltf-intake.mjs (clean+validate gate).
// PORT(tordanik/OSM2World@f6518b35117cf7864e9c6f68c3b303a006e27b8f master 2026-10-15, gh api):
// extract->typed world->glTF + default-style tables reimplemented, zero bytes vendored. File input only.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
export const STYLE = { storeyM: 3, defaultStoreys: 3, defaultHM: 9, roadW: 6, roadH: 0.15, roof: "flat" };
export const ACCEPTED = new Set(["CC0-1.0", "ODbL-1.0", "CC-BY-4.0"]);
export const MAX_BYTES = 200000; export const MAX_WAYS = 48; export const MAX_NODES = 1500;
const err = (code, msg) => { const e = new Error(`${code} ${msg}`); e.code = code; throw e; };
export const sha256hex = (b) => createHash("sha256").update(b).digest("hex");
const q3 = (v) => Math.round(v * 1000) / 1000;
export function parseOsm(text) {
  const raw = String(text ?? "");
  if (!raw.length) err("E_INPUT_EMPTY", "empty extract");
  if (raw.length > MAX_BYTES) err("E_CAP", `${raw.length}B over cap ${MAX_BYTES}B`);
  const nodes = new Map(); const ways = [];
  if (raw.trimStart().startsWith("{")) {
    let o = null; try { o = JSON.parse(raw); } catch (e) { err("E_PARSE", `bad OSM JSON (${e.message})`); }
    for (const el of o?.elements ?? []) {
      if (el?.type === "node") nodes.set(String(el.id), { lat: +el.lat, lon: +el.lon });
      else if (el?.type === "way") ways.push({ id: String(el.id), refs: (el.nodes ?? []).map(String), tags: el.tags ?? {} });
    }
  } else {
    for (const m of raw.matchAll(/<node\b[^>]*>/g)) { const id = /id="([^"]+)"/.exec(m[0])?.[1]; if (id) nodes.set(id, { lat: +/lat="([^"]+)"/.exec(m[0])?.[1], lon: +/lon="([^"]+)"/.exec(m[0])?.[1] }); }
    for (const m of raw.matchAll(/<way\b[^>]*id="([^"]+)"[^>]*>([\s\S]*?)<\/way>/g)) {
      const tags = {}; for (const t of m[2].matchAll(/k="([^"]+)"[^>]*v="([^"]+)"/g)) tags[t[1]] = t[2];
      ways.push({ id: m[1], refs: [...m[2].matchAll(/ref="([^"]+)"/g)].map((x) => x[1]), tags });
    }
    if (!nodes.size && !ways.length) err("E_PARSE", "no <node>/<way> rows");
  }
  if (nodes.size > MAX_NODES || ways.length > MAX_WAYS) err("E_CAP", "extract over cap");
  let la0 = 0, lo0 = 0; for (const n of nodes.values()) { la0 += n.lat; lo0 += n.lon; }
  la0 /= Math.max(1, nodes.size); lo0 /= Math.max(1, nodes.size);
  const kx = 111320 * Math.cos(la0 * Math.PI / 180), pts = new Map();
  for (const [id, n] of nodes) pts.set(id, [q3((n.lon - lo0) * kx), q3((n.lat - la0) * 110540)]);
  const buildings = [], highways = [], edges = [];
  for (const w of ways.sort((a, b) => (a.id < b.id ? -1 : 1))) {
    if (w.tags?.building && w.tags.building !== "no") {
      const fp = w.refs.filter((r) => pts.has(r)).map((r) => pts.get(r));
      if (fp.length >= 3) buildings.push({ id: w.id, ring: fp, tags: w.tags });
    } else if (w.tags?.highway) {
      const line = w.refs.filter((r) => pts.has(r)).map((r) => pts.get(r));
      if (line.length >= 2) { highways.push({ id: w.id, line }); for (let i = 0; i + 1 < w.refs.length; i++) edges.push([w.refs[i], w.refs[i + 1]]); }
    }
  }
  edges.sort((a, b) => (a[0] === b[0] ? (a[1] < b[1] ? -1 : 1) : (a[0] < b[0] ? -1 : 1)));
  return { pts, buildings, highways, graph: { nodes: [...nodes.keys()].sort(), edges } };
}
const hOf = (t, s = STYLE) => { const h = Number.parseFloat(t?.height ?? "");
  if (Number.isFinite(h) && h > 0) return Math.min(60, q3(h));
  const lv = Number.parseInt(t?.["building:levels"] ?? "", 10);
  return Number.isInteger(lv) && lv > 0 ? Math.min(60, q3(Math.min(lv, 20) * s.storeyM)) : s.defaultHM; };
export function inspectGltf(text) { // headless viewer-path check: severity-0 equivalents + counts
  const issues = []; let o = null;
  try { o = JSON.parse(String(text)); } catch (e) { return { ok: false, issues: [`parse: ${e.message}`], meshes: 0, triangles: 0 }; }
  if (o?.asset?.version !== "2.0") issues.push("asset.version is not 2.0");
  if (!o?.meshes?.length) issues.push("no meshes");
  let bl = -1; try { bl = Buffer.from(String(o?.buffers?.[0]?.uri ?? "").split(",")[1] ?? "", "base64").length; } catch {}
  if (bl !== o?.buffers?.[0]?.byteLength) issues.push("buffer byteLength mismatch");
  for (const a of o?.accessors ?? []) { const v = o?.bufferViews?.[a.bufferView];
    if (!v || v.byteOffset + v.byteLength > bl) { issues.push("accessor outside view"); break; }
    if (a.componentType === 5126 && (!Array.isArray(a.min) || !Array.isArray(a.max))) issues.push("POSITION min/max missing"); }
  let tris = 0; for (const m of o?.meshes ?? []) for (const p of m?.primitives ?? []) tris += (o?.accessors?.[p.indices]?.count ?? 0) / 3;
  return { ok: issues.length === 0, issues, meshes: o?.meshes?.length ?? 0, triangles: Math.round(tris) };
}
export function cookExtractToGltf(text, { licence = "UNKNOWN", style = STYLE } = {}) {
  if (!ACCEPTED.has(licence)) err("E_LICENCE", `licence=${licence} refused`);
  const { buildings, highways } = parseOsm(text);
  if (!buildings.length) err("E_PARSE", "no building ways");
  const chunks = []; let len = 0; const views = []; const accs = []; const meshes = []; const nodes = []; let tris = 0;
  const alloc = (buf, target) => {
    const pad = (4 - (len % 4)) % 4; if (pad) { chunks.push(Buffer.alloc(pad)); len += pad; }
    const off = len; chunks.push(buf); len += buf.length; views.push({ buffer: 0, byteOffset: off, byteLength: buf.length, target }); return views.length - 1;
  };
  const addMesh = (name, pos, idx) => {
    const min = [1e30, 1e30, 1e30], max = [-1e30, -1e30, -1e30];
    for (let i = 0; i < pos.length; i += 3) for (let k = 0; k < 3; k++) { min[k] = Math.min(min[k], pos[i + k]); max[k] = Math.max(max[k], pos[i + k]); }
    const pv = alloc(Buffer.from(new Float32Array(pos).buffer), 34962), iv = alloc(Buffer.from(new Uint16Array(idx).buffer), 34963);
    accs.push({ bufferView: pv, byteOffset: 0, componentType: 5126, count: pos.length / 3, type: "VEC3", min, max });
    accs.push({ bufferView: iv, byteOffset: 0, componentType: 5123, count: idx.length, type: "SCALAR" });
    meshes.push({ name, primitives: [{ attributes: { POSITION: accs.length - 2 }, indices: accs.length - 1, mode: 4 }] });
    nodes.push({ name, mesh: meshes.length - 1 }); tris += idx.length / 3;
  };
  for (const b of buildings) {
    const h = hOf(b.tags, style), ring = b.ring, pos = [], idx = [], n = ring.length;
    for (const [x, y] of ring) pos.push(x, 0, y); for (const [x, y] of ring) pos.push(x, h, y);
    for (let i = 0; i < n; i++) { const j = (i + 1) % n; idx.push(i, j, n + j, i, n + j, n + i); }
    for (let i = 1; i + 1 < n; i++) idx.push(n, n + i, n + i + 1);
    addMesh(`building_${b.id}`, pos.map(q3), idx);
  }
  if (highways.length) {
    const hw = style.roadW / 2, hy = style.roadH, pos = [], idx = []; let v = 0;
    for (const r of highways) for (let i = 0; i + 1 < r.line.length; i++) {
      const [ax, ay] = r.line[i], [bx, by] = r.line[i + 1], dx = bx - ax, dy = by - ay, L = Math.hypot(dx, dy) || 1, px = -dy / L * hw, py = dx / L * hw;
      pos.push(ax + px, hy, ay + py, ax - px, hy, ay - py, bx + px, hy, by + py, bx - px, hy, by - py);
      idx.push(v, v + 2, v + 1, v + 1, v + 2, v + 3); v += 4;
    }
    addMesh("roads", pos.map(q3), idx);
  }
  const order = meshes.map((m, i) => [m.name, i]).sort((a, b) => (a[0] < b[0] ? -1 : 1)).map((x) => x[1]);
  const gltf = { asset: { version: "2.0", generator: "tools/city-osm.mjs" }, scene: 0, scenes: [{ nodes: order.map((_, k) => k) }],
    nodes: order.map((i) => nodes[i]), meshes: order.map((i) => meshes[i]),
    buffers: [{ byteLength: len, uri: `data:application/octet-stream;base64,${Buffer.concat(chunks).toString("base64")}` }], bufferViews: views, accessors: accs };
  const out = JSON.stringify(gltf);
  const prov = { tool: "tools/city-osm.mjs", licence, sourceSha256: sha256hex(Buffer.from(String(text), "utf8")), styleSha256: sha256hex(Buffer.from(JSON.stringify(style), "utf8")),
    cookDigest: sha256hex(Buffer.from(out, "utf8")), bytes: out.length, triangles: Math.round(tris), meshes: meshes.length, buildings: buildings.length, highways: highways.length };
  return { gltf: out, prov, stats: { triangles: Math.round(tris), meshes: meshes.length } };
}
export function cookFile(input, output, { licence = "", stylePath = "", provenance = "" } = {}) {
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(input) || input.startsWith("//")) err("E_NETWORK", "file input only");
  let text = null; try { text = readFileSync(input, "utf8"); } catch { err("E_INPUT_ABSENT", `absent: ${input}`); }
  let style = STYLE;
  if (stylePath) { try { style = { ...STYLE, ...JSON.parse(readFileSync(stylePath, "utf8")) }; } catch (e) { err("E_PARSE", `bad style (${e.message})`); } }
  const { gltf, prov } = cookExtractToGltf(text, { licence: licence || "UNKNOWN", style });
  mkdirSync(dirname(resolve(output)), { recursive: true }); writeFileSync(output, gltf);
  writeFileSync(provenance || `${output}.prov.json`, `${JSON.stringify(prov, null, 2)}\n`); return prov;
}
if (import.meta.main) (async () => {
  try {
    const a = process.argv.slice(2);
    if (a.includes("--help") || a.includes("-h")) { process.stdout.write("Usage: node tools/city-osm.mjs <in.osm|in.json> <out.gltf> [--licence SPDX] [--style p] [--provenance p]\n"); return; }
    const opt = (n) => { const i = a.findIndex((x) => x === n || x.startsWith(`${n}=`)); return i < 0 ? "" : (a[i].includes("=") ? a[i].slice(n.length + 1) : (a[i + 1] ?? "")); };
    const [input, output] = a.filter((x) => !x.startsWith("--"));
    if (!input || !output) { process.stderr.write("E_INPUT_ABSENT need <in> <out>\n"); process.exit(2); }
    const p = cookFile(input, output, { licence: opt("--licence"), stylePath: opt("--style"), provenance: opt("--provenance") });
    process.stdout.write(`RESULT city-osm ok buildings=${p.buildings} meshes=${p.meshes} triangles=${p.triangles} digest=${p.cookDigest.slice(0, 16)}\n`);
  } catch (e) { process.stderr.write(`${e.code ?? "E_COOK_CRASH"} ${e.message}\n`); process.exit(2); }
})();
