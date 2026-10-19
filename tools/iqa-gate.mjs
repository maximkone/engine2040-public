// iqa-gate: capture-pair image gate for PQ4 (reference vs capture PNGs).
// PORT(justin13888/iqa-rs@1.2.1:crates/iqa + iqa-cli one-pair one-JSON): JSON report shape
// and metric-set idea only, reimplemented dependency-free against
// crates/playtest/src/look/metric.rs integer semantics (8x8 luma SSIM windows,
// C1/C2 constants, BT.601 luma; Lab/XYZ constants for deltaE; FLIP beside SSIM:
// the E7 sRGB table, YCxCyCz, one CSF pass each way, Sobel feature weighting).
// Zero bytes vendored.
// Consumer: PQ4 reference comparison once Q1/RT7 references land; run headless in CI.
// Donor slots ssimulacra2/butteraugli stay null offline; fill with iqa-cli --metric.
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
export const WINDOW = 8, SSIM_ONE = 10000, DEFAULT_THRESHOLD = 9800;
export const NOTE = "ssimulacra2/butteraugli null offline; fill with justin13888/iqa-rs iqa-cli --metric ssimulacra2|butteraugli";
const SIG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const C1 = 26634n, C2 = 239708n, WN = 64n, PM = 1000000n;
function divRound(n, d) { const a = 2n * n + d, b = 2n * d; let q = a / b; if (a < 0n && a % b !== 0n) q -= 1n; return q; }
function wppm(sx, sy, sxx, syy, sxy) { const ln = 2n * sx * sy + C1, ld = sx * sx + sy * sy + C1; const cn = 2n * (WN * sxy - sx * sy) + C2, cd = WN * (sxx + syy) - sx * sx - sy * sy + C2; const v = divRound(ln * cn * PM, ld * cd); return v > PM ? PM : v < -PM ? -PM : v; }
function luma(r, g, b) { return (77 * r + 150 * g + 29 * b + 128) >> 8; }
export function ssimX10000(a, b, w, h) { // RGBA bytes, same size; non-overlapping 8x8 luma windows
  const la = new Uint8Array(w * h), lb = new Uint8Array(w * h);
  for (let i = 0, p = 0; i < la.length; i++, p += 4) { la[i] = luma(a[p], a[p + 1], a[p + 2]); lb[i] = luma(b[p], b[p + 1], b[p + 2]); }
  let t = 0n, n = 0;
  for (let y = 0; y + 8 <= h; y += 8) for (let x = 0; x + 8 <= w; x += 8) { let s0 = 0n, s1 = 0n, s2 = 0n, s3 = 0n, s4 = 0n;
    for (let j = 0; j < 8; j++) { const r = (y + j) * w + x; for (let k = 0; k < 8; k++) { const pa = BigInt(la[r + k]), pb = BigInt(lb[r + k]); s0 += pa; s1 += pb; s2 += pa * pa; s3 += pb * pb; s4 += pa * pb; } }
    t += wppm(s0, s1, s2, s3, s4); n++; }
  if (n === 0) throw new Error("no 8x8 window");
  return Number(divRound(t, 100n * BigInt(n)));
}
const lin = (c) => { const v = c / 255; return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
function lab(r, g, b) { const R = lin(r), G = lin(g), B = lin(b); const X = (0.4124564 * R + 0.3575761 * G + 0.1804375 * B) / 0.95047, Y = 0.2126729 * R + 0.7151522 * G + 0.072175 * B, Z = (0.0193339 * R + 0.119192 * G + 0.9503041 * B) / 1.08883; const f = (t) => t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116; const fx = f(X), fy = f(Y), fz = f(Z); return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)]; }
export function meanDeX100(a, b) { let s = 0; const n = a.length / 4; for (let p = 0; p < a.length; p += 4) { if (a[p] === b[p] && a[p + 1] === b[p + 1] && a[p + 2] === b[p + 2]) continue; const A = lab(a[p], a[p + 1], a[p + 2]), B = lab(b[p], b[p + 1], b[p + 2]); s += Math.hypot(A[0] - B[0], A[1] - B[1], A[2] - B[2]); } return Math.round(s / n * 100); }
// FLIP beside SSIM (LF15): mirrors crates/playtest/src/look/metric.rs::flip.
export const FLIP_ONE = 10000;
const FLIP_CHROMA = 4500, FLIP_BASE = 4000, FLIP_FEATURE = 14000, FLIP_CAP = 255, FLIP_TAU = [1, 4, 6, 4, 1];
const srgbE7 = (i) => { const s = i / 255, l = s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); return Math.round(l * 1e7); };
export const SRGB_E7 = Array.from({ length: 256 }, (_, i) => srgbE7(i));
function ycxcycz(r, g, b) { const y = Math.floor((2126560 * SRGB_E7[r] + 7151580 * SRGB_E7[g] + 721860 * SRGB_E7[b] + 5000000) / 10000000); return [y, SRGB_E7[r] - y, SRGB_E7[g] - y, SRGB_E7[b] - y]; }
function flipRaw(a, p, b, q) { const A = ycxcycz(a[p], a[p + 1], a[p + 2]), B = ycxcycz(b[q], b[q + 1], b[q + 2]); const l = Math.abs(A[0] - B[0]), c = Math.abs(A[1] - B[1]) + Math.abs(A[2] - B[2]) + Math.abs(A[3] - B[3]); return Math.min(l + Math.floor((c * FLIP_CHROMA + 5000) / 10000), 10000000); }
function csfPass(src, w, h, rows) { const out = new Array(src.length).fill(0); for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const count = rows ? w : h, at = rows ? x : y; let s = 0; for (let k = 0; k < 5; k++) { const pos = Math.min(Math.max(at + k - 2, 0), count - 1); s += FLIP_TAU[k] * src[rows ? y * w + pos : pos * w + x]; } out[y * w + x] = Math.floor((s + 8) / 16); } return out; }
function sobelWeights(l, w, h) { const out = new Array(w * h).fill(0); if (w < 3 || h < 3) return out; for (let y = 1; y < h - 1; y++) for (let x = 1; x < w - 1; x++) { const at = (dx, dy) => l[(y + dy - 1) * w + (x + dx - 1)]; const gx = at(2, 0) + 2 * at(2, 1) + at(2, 2) - at(0, 0) - 2 * at(0, 1) - at(0, 2), gy = at(0, 2) + 2 * at(1, 2) + at(2, 2) - at(0, 0) - 2 * at(1, 0) - at(2, 0); out[y * w + x] = Math.min(Math.abs(gx) + Math.abs(gy), FLIP_CAP); } return out; }
export function flipX10000(a, b, w, h) { // RGBA bytes, same size; mirrors metric.rs::flip
  if (a.length !== b.length || a.length !== w * h * 4) throw new Error("size mismatch");
  const n = w * h, raw = new Array(n);
  for (let i = 0, p = 0; i < n; i++, p += 4) raw[i] = flipRaw(a, p, b, p);
  const sm = csfPass(csfPass(raw, w, h, true), w, h, false);
  const la = new Uint8Array(n), lb = new Uint8Array(n);
  for (let i = 0, p = 0; i < n; i++, p += 4) { la[i] = luma(a[p], a[p + 1], a[p + 2]); lb[i] = luma(b[p], b[p + 1], b[p + 2]); }
  const fa = sobelWeights(la, w, h), fb = sobelWeights(lb, w, h);
  let sum = 0, max = 0;
  for (let i = 0; i < n; i++) { const s = fa[i] > fb[i] ? fa[i] : fb[i], weight = FLIP_BASE + Math.floor((FLIP_FEATURE - FLIP_BASE) * s / FLIP_CAP), vis = Math.floor(sm[i] * weight / 10000); sum += vis; const m = Math.floor(vis / 1000); if (m > max) max = m; }
  const mean = Number(divRound(BigInt(sum) * 10000n, BigInt(n) * 10000000n));
  return { flip_x10000: Math.min(mean, FLIP_ONE), flip_max_x10000: Math.min(max, FLIP_ONE), pixels: n };
}
export function scorePair(a, b, w, h, th = DEFAULT_THRESHOLD) { if (a.length !== b.length || a.length !== w * h * 4) throw new Error("size mismatch"); const s = ssimX10000(a, b, w, h), d = meanDeX100(a, b), f = flipX10000(a, b, w, h); return { ssim_x10000: s, flip_x10000: f.flip_x10000, ssimulacra2: null, butteraugli: null, mean_de_x100: d, verdict: s >= th ? "pass" : "fail", note: NOTE }; }
export function sandAt(x, y) { const t = (x * 7 + y * 13 + (x * y) % 5) % 23 - 11; const c = (v) => Math.min(255, Math.max(0, v + t)); return [c(194), c(160), c(104)]; }
const bri = (v) => Math.min(255, Math.floor((v * 6 + 2) / 5));
export function sandFrames(W = 64, H = 48) { const r = Buffer.alloc(W * H * 4), c = Buffer.alloc(W * H * 4); for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) { const p = (y * W + x) * 4; const s = sandAt(x, y); r.set([...s, 255], p); const q = y >= 8 ? s.map(bri) : s; c.set([...q, 255], p); } return { ref: r, cap: c, w: W, h: H }; }
function chunk(buf, p, l) { if (p + 12 > buf.length) throw new Error(l + ": truncated"); const n = buf.readUInt32BE(p); if (p + 12 + n > buf.length) throw new Error(l + ": truncated"); return { t: buf.toString("ascii", p + 4, p + 8), d: buf.subarray(p + 8, p + 8 + n), n: p + 12 + n }; }
function unfilter(raw, w, h, ch) { const st = w * ch, out = Buffer.alloc(h * st); let pv = Buffer.alloc(st); for (let y = 0; y < h; y++) { const rs = y * (st + 1), f = raw[rs]; if (f > 4) throw new Error("filter " + f); const row = raw.subarray(rs + 1, rs + 1 + st), bs = y * st; for (let x = 0; x < st; x++) { const a = x >= ch ? out[bs + x - ch] : 0, b = pv[x], cc = x >= ch ? pv[x - ch] : 0; let v = row[x]; if (f === 1) v += a; else if (f === 2) v += b; else if (f === 3) v += (a + b) >> 1; else if (f === 4) { const q = a + b - cc, pa = Math.abs(q - a), pb = Math.abs(q - b), pc = Math.abs(q - cc); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : cc; } out[bs + x] = v & 255; } pv = out.subarray(bs, bs + st); } return out; }
export function decodePng(buf, label = "png") { const v = Buffer.isBuffer(buf) ? buf : Buffer.from(buf); if (v.length < 8 || !v.subarray(0, 8).equals(SIG)) throw new Error(label + ": bad signature"); let w = 0, h = 0, ch = 0, seen = false; const ids = []; let p = 8; while (p < v.length) { const c = chunk(v, p, label); if (c.t === "IHDR") { if (c.d.length !== 13) throw new Error(label + ": bad IHDR"); w = c.d.readUInt32BE(0); h = c.d.readUInt32BE(4); if (w === 0 || h === 0 || w * h > 64000000) throw new Error(label + ": bad size"); if (c.d[8] !== 8) throw new Error(label + ": bad depth"); if (c.d[9] === 6) ch = 4; else if (c.d[9] === 2) ch = 3; else throw new Error(label + ": bad color"); if (c.d[12] !== 0) throw new Error(label + ": interlaced"); seen = true; } else if (c.t === "IDAT") ids.push(c.d); else if (c.t === "IEND") break; p = c.n; } if (!seen || !ids.length) throw new Error(label + ": no image"); const raw = inflateSync(Buffer.concat(ids)); if (raw.length !== h * (w * ch + 1)) throw new Error(label + ": bad data"); const f = unfilter(raw, w, h, ch); if (ch === 3) { const o = Buffer.alloc(w * h * 4); for (let i = 0, q = 0; i < f.length; i += 3, q += 4) { o[q] = f[i]; o[q + 1] = f[i + 1]; o[q + 2] = f[i + 2]; o[q + 3] = 255; } return { w, h, data: o }; } return { w, h, data: f }; }
function usage() { return "usage: node tools/iqa-gate.mjs [--threshold N] <reference.png> <capture.png>\n       node tools/iqa-gate.mjs --selfcheck [--threshold N]"; }
function num(v) { const n = Number(v); if (!Number.isFinite(n)) throw new Error("bad threshold"); return n; }
export function selfcheck(th = DEFAULT_THRESHOLD) { const { ref, cap, w, h } = sandFrames(); const eq = scorePair(ref, ref, w, h, th), br = scorePair(ref, cap, w, h, th); const s1 = JSON.stringify(br), s2 = JSON.stringify(scorePair(ref, cap, w, h, th)); const ok = eq.ssim_x10000 === 10000 && eq.verdict === "pass" && br.ssim_x10000 < 9800 && br.verdict === "fail" && s1 === s2 && "ssim_x10000" in br && "ssimulacra2" in br && "butteraugli" in br; return { ok, eq, br, json: s1 }; }
if (import.meta.main) { try { const a = process.argv.slice(2); let th = DEFAULT_THRESHOLD; const t = a.indexOf("--threshold"); if (t !== -1) { if (a[t + 1] === undefined) throw new Error("bad threshold"); th = num(a[t + 1]); a.splice(t, 2); } if (a.includes("--help") || a.includes("-h")) { process.stdout.write(usage() + "\n"); process.exit(0); } if (a.includes("--selfcheck")) { const r = selfcheck(th); process.stdout.write(r.json + "\n"); if (!r.ok) { process.stderr.write("selfcheck failed\n"); process.exit(1); } } else { if (a.length !== 2) throw new Error(usage()); const x = decodePng(readFileSync(a[0]), a[0]), y = decodePng(readFileSync(a[1]), a[1]); if (x.w !== y.w || x.h !== y.h) throw new Error("size mismatch"); const j = scorePair(x.data, y.data, x.w, y.h, th); process.stdout.write(JSON.stringify(j) + "\n"); if (j.verdict !== "pass") process.exit(1); } } catch (e) { process.stderr.write(String((e && e.message) || e) + "\n"); process.exitCode = 2; } }
