// content/kit-common.mjs — shared deterministic kit-generator helpers (CT8 arena + CT9 maps + CT25 crowd).
// One home for the 12 identical functions the three kit generators carried as
// copies (deadcode dupes 2026-10-02: box, heightPlane, hash2, vnoise, rgbe,
// crcTable, crc, chunk, writePng, mk, push, track, plus emitGltf). All three tools/gen.mjs files
// All three import from here; outputs stay byte-identical.
// Original authored helpers, CC0-1.0. Units: meters, +Y up, UVs in 0..1.
// No randomness: integer-lattice value noise with fixed constants only.
import { writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { deflateSync } from "node:zlib";
import { join } from "node:path";

// ---------------------------------------------------------------------------
// Deterministic value noise (fixed lattice hash, bilinear smoothstep, fbm).
// ---------------------------------------------------------------------------
export function hash2(x, y) {
  let h = (x * 374761393 + y * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}
function smooth(t) { return t * t * (3 - 2 * t); }
function vnoise(x, y) {
  const xi = Math.floor(x), yi = Math.floor(y);
  const xf = x - xi, yf = y - yi;
  const a = hash2(xi, yi), b = hash2(xi + 1, yi);
  const c = hash2(xi, yi + 1), d = hash2(xi + 1, yi + 1);
  const u = smooth(xf), v = smooth(yf);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}
export function fbm(x, y, oct) {
  let s = 0, amp = 0.5, f = 1, norm = 0;
  for (let i = 0; i < oct; i++) { s += amp * vnoise(x * f, y * f); norm += amp; amp *= 0.5; f *= 2.03; }
  return s / norm;
}

// ---------------------------------------------------------------------------
// Mesh accumulator: merged single-primitive meshes (positions, normals,
// uv0 0..1, tangents +X/+Z with w=1). Faces are flat-shaded quads.
// ---------------------------------------------------------------------------
export function newAcc() {
  return { pos: [], nrm: [], uv: [], tan: [], idx: [] };
}
// Subdivided axis box: every face is an n*n grid (default 4). Single-quad
// boxes collapse to empty LODs in one greedy CT2 pass; subdivided boxes
// halve progressively and hold their shape down the whole LOD chain.
export function box(acc, cx, cy, cz, sx, sy, sz, n = 4) {
  const x0 = cx - sx / 2, x1 = cx + sx / 2;
  const y0 = cy - sy / 2, y1 = cy + sy / 2;
  const z0 = cz - sz / 2, z1 = cz + sz / 2;
  // Faces as (origin, u-axis, v-axis, normal): normal = u x v (outward).
  const faces = [
    { o: [x0, y0, z1], u: [sx, 0, 0], v: [0, sy, 0], n: [0, 0, 1] }, // +Z
    { o: [x1, y0, z0], u: [-sx, 0, 0], v: [0, sy, 0], n: [0, 0, -1] }, // -Z
    { o: [x1, y0, z1], u: [0, 0, -sz], v: [0, sy, 0], n: [1, 0, 0] }, // +X
    { o: [x0, y0, z0], u: [0, 0, sz], v: [0, sy, 0], n: [-1, 0, 0] }, // -X
    { o: [x0, y1, z1], u: [sx, 0, 0], v: [0, 0, -sz], n: [0, 1, 0] }, // +Y
    { o: [x0, y0, z0], u: [sx, 0, 0], v: [0, 0, sz], n: [0, -1, 0] }, // -Y
  ];
  const t = (nr) => (Math.abs(nr[0]) > 0.5 ? [0, 0, 1, 1] : [1, 0, 0, 1]);
  for (const f of faces) {
    const base = acc.pos.length;
    for (let iy = 0; iy <= n; iy++) {
      for (let ix = 0; ix <= n; ix++) {
        acc.pos.push([
          f.o[0] + f.u[0] * (ix / n) + f.v[0] * (iy / n),
          f.o[1] + f.u[1] * (ix / n) + f.v[1] * (iy / n),
          f.o[2] + f.u[2] * (ix / n) + f.v[2] * (iy / n),
        ]);
        acc.nrm.push(f.n);
        acc.uv.push([ix / n, iy / n]);
        acc.tan.push(t(f.n));
      }
    }
    for (let iy = 0; iy < n; iy++) {
      for (let ix = 0; ix < n; ix++) {
        const a = base + iy * (n + 1) + ix;
        const b = a + 1, c = a + n + 1, e = c + 1;
        acc.idx.push(a, b, e, a, e, c);
      }
    }
  }
}
// Heightfield plane: grid nx*nz quads over w*d at (cx,cz), base y=cy.
export function heightPlane(acc, cx, cy, cz, w, d, nx, nz, hFn) {
  const base = acc.pos.length;
  const H = (ix, iz) => {
    const x = cx - w / 2 + (w * ix) / nx;
    const z = cz - d / 2 + (d * iz) / nz;
    return hFn(x, z);
  };
  for (let iz = 0; iz <= nz; iz++) {
    for (let ix = 0; ix <= nx; ix++) {
      const x = cx - w / 2 + (w * ix) / nx;
      const z = cz - d / 2 + (d * iz) / nz;
      const e = 0.05;
      const dhdx = (hFn(x + e, z) - hFn(x - e, z)) / (2 * e);
      const dhdz = (hFn(x, z + e) - hFn(x, z - e)) / (2 * e);
      const inv = 1 / Math.hypot(dhdx, 1, dhdz);
      acc.pos.push([x, cy + H(ix, iz), z]);
      acc.nrm.push([-dhdx * inv, inv, -dhdz * inv]);
      acc.uv.push([ix / nx, iz / nz]);
      acc.tan.push([1, 0, 0, 1]);
    }
  }
  for (let iz = 0; iz < nz; iz++) {
    for (let ix = 0; ix < nx; ix++) {
      const a = base + iz * (nx + 1) + ix;
      const b = a + 1, c = a + nx + 1, e = c + 1;
      acc.idx.push(a, c, e, a, e, b);
    }
  }
}

// ---------------------------------------------------------------------------
// PNG writer (stdlib zlib): 8-bit RGBA, filter None, deterministic.
// ---------------------------------------------------------------------------
function crcTable() {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
}
const CRC_T = crcTable();
function crc(buf) {
  let c = -1;
  for (const b of buf) c = CRC_T[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.from(type, 'ascii');
  const cb = Buffer.alloc(4); cb.writeUInt32BE(crc(Buffer.concat([td, data])));
  return Buffer.concat([len, td, data, cb]);
}
function writePng(path, w, h, rgba) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    rgba.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  }
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  const png = Buffer.concat([sig, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
  writeFileSync(path, png);
  return png;
}
const byte = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));

// Texture-slot painter shared by both kits: fills `out[name]` and writes the PNG.
export function makeMk(texDir, out) {
  return (name, size, fn) => {
    const rgba = Buffer.alloc(size * size * 4);
    for (let y = 0; y < size; y++)
      for (let x = 0; x < size; x++) {
        const [r, g, b, a] = fn(x / size, y / size, x, y);
        rgba.set([byte(r), byte(g), byte(b), byte(a)], (y * size + x) * 4);
      }
    out[name] = writePng(join(texDir, name + '.png'), size, size, rgba);
  };
}

// ---------------------------------------------------------------------------
// Radiance RGBE helper (flat scanlines); per-kit sky headers stay local.
// ---------------------------------------------------------------------------
export function rgbe(r, g, b) {
  const m = Math.max(r, g, b);
  if (m < 1e-32) return [0, 0, 0, 0];
  const e = Math.ceil(Math.log2(m));
  const s = Math.pow(2, e - 8);
  return [Math.min(255, Math.round(r / s)), Math.min(255, Math.round(g / s)), Math.min(255, Math.round(b / s)), e + 128];
}

// ---------------------------------------------------------------------------
// glTF buffer push + manifest track shared by both kit mains.
// ---------------------------------------------------------------------------
export function pushInto(parts, views, buf, target) {
  const off = parts.reduce((n, p) => n + p.bytes.length, 0);
  parts.push({ bytes: buf, target });
  views.push({ buffer: 0, byteOffset: off, byteLength: buf.length, ...(target ? { target } : {}) });
  return views.length - 1;
}
export function trackInto(manifest, p, buf) {
  manifest.files[p] = {
    sha256: createHash('sha256').update(buf).digest('hex'),
    bytes: buf.length,
  };
}

// ---------------------------------------------------------------------------
// glTF 2.0 writer shared by the arena, maps and crowd kit generators.
// The three local copies were byte-identical apart from the generator tag,
// so the tag stays a parameter. Sockets ride as translation-only nodes;
// kits without sockets emit the same bytes as before.
// ---------------------------------------------------------------------------
export function emitGltf(name, built, generator) {
  const parts = []; // {bytes: Buffer, target}
  const views = [];
  const accessors = [];
  const push = (buf, target) => pushInto(parts, views, buf, target);
  const f32 = (arr) => {
    const b = Buffer.alloc(arr.length * 4);
    arr.forEach((v, i) => b.writeFloatLE(v, i * 4));
    return b;
  };
  const u32 = (arr) => {
    const b = Buffer.alloc(arr.length * 4);
    arr.forEach((v, i) => b.writeUInt32LE(v >>> 0, i * 4));
    return b;
  };
  const minmax = (vecs, dim) => {
    const mn = Array(dim).fill(Infinity), mx = Array(dim).fill(-Infinity);
    for (const v of vecs) for (let i = 0; i < dim; i++) { mn[i] = Math.min(mn[i], v[i]); mx[i] = Math.max(mx[i], v[i]); }
    return [mn, mx];
  };
  const meshes = [];
  built.meshes.forEach((m) => {
    const { acc } = m;
    const P = acc.pos.flat(), N = acc.nrm.flat(), T = acc.uv.flat(), G = acc.tan.flat();
    const [pmin, pmax] = minmax(acc.pos, 3);
    const [nmin, nmax] = minmax(acc.nrm, 3);
    const [tmin, tmax] = minmax(acc.uv, 2);
    const [gmin, gmax] = minmax(acc.tan, 4);
    const imin = Math.min(...acc.idx), imax = Math.max(...acc.idx);
    const vP = push(f32(P), 34962), vN = push(f32(N), 34962);
    const vT = push(f32(T), 34962), vG = push(f32(G), 34962);
    const vI = push(u32(acc.idx), 34963);
    const aP = accessors.length;
    accessors.push(
      { bufferView: vP, byteOffset: 0, componentType: 5126, count: acc.pos.length, type: 'VEC3', min: pmin, max: pmax },
      { bufferView: vN, byteOffset: 0, componentType: 5126, count: acc.nrm.length, type: 'VEC3', min: nmin, max: nmax },
      { bufferView: vT, byteOffset: 0, componentType: 5126, count: acc.uv.length, type: 'VEC2', min: tmin, max: tmax },
      { bufferView: vG, byteOffset: 0, componentType: 5126, count: acc.tan.length, type: 'VEC4', min: gmin, max: gmax },
      { bufferView: vI, byteOffset: 0, componentType: 5125, count: acc.idx.length, type: 'SCALAR', min: [imin], max: [imax] },
    );
    meshes.push({
      name: m.name,
      primitives: [{
        attributes: { POSITION: aP, NORMAL: aP + 1, TEXCOORD_0: aP + 2, TANGENT: aP + 3 },
        indices: aP + 4, mode: 4, material: 0,
      }],
    });
  });
  const nodes = built.meshes.map((m, i) => ({ name: m.name, mesh: i }));
  for (const s of built.sockets || []) nodes.push({ name: s.name, translation: s.at });
  const total = parts.reduce((n, p) => n + p.bytes.length, 0);
  const blob = Buffer.concat(parts.map((p) => p.bytes));
  const doc = {
    asset: { version: "2.0", generator },
    scene: 0,
    scenes: [{ nodes: nodes.map((_, i) => i) }],
    nodes,
    meshes,
    materials: [{
      name: `MAT_${name}`,
      pbrMetallicRoughness: {
        baseColorFactor: built.material.base,
        metallicFactor: built.material.metallic,
        roughnessFactor: built.material.roughness,
      },
    }],
    buffers: [{ uri: 'data:application/octet-stream;base64,' + blob.toString('base64'), byteLength: total }],
    bufferViews: views,
    accessors,
  };
  return Buffer.from(JSON.stringify(doc));
}