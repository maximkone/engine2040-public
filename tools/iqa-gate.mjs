// iqa-gate: capture-pair image gate for PQ4 (reference vs capture PNGs).
// PORT(justin13888/iqa-rs@1.2.1:crates/iqa + iqa-cli one-pair one-JSON): JSON report shape
// and metric-set idea only, reimplemented dependency-free against
// crates/playtest/src/look/metric.rs integer semantics (8x8 luma SSIM windows,
// C1/C2 constants, BT.601 luma; Lab/XYZ constants for deltaE; FLIP beside SSIM:
// the E7 sRGB table, YCxCyCz, one CSF pass each way, Sobel feature weighting).
// Zero bytes vendored.
// SSIMULACRA2 v2.1 beside SSIM: PORT(rust-av/ssimulacra2@0.5.1), the algorithm
// shape and the 108-weight table only, reimplemented dependency-free against the
// same maths the Rust donor runs (sRGB -> linear -> XYB, six scales, the recursive
// Gaussian blur, the SSIM and edge-diff error maps, the weighted sum). Donor:
// BSD 2-Clause, Copyright (c) 2022-2022, the rav1e contributors,
// <https://github.com/rust-av/ssimulacra2>, LICENSE read live 2026-10-21.
// Zero bytes vendored: the weights below are the donor's published table and the
// constants are computed from the donor's own build-time maths.
// Consumer: PQ4 reference comparison once Q1/RT7 references land; run headless in CI.
// The butteraugli slot stays null offline: that donor is not in the tree.
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
export const WINDOW = 8, SSIM_ONE = 10000, DEFAULT_THRESHOLD = 9800;
export const NOTE = "ssimulacra2 is the SSIMULACRA2 v2.1 twin (10000 = identical, 0 = worst); butteraugli is null offline";
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
// ---- SSIMULACRA2 v2.1 (the second metric) ----
// PORT(rust-av/ssimulacra2@0.5.1): the algorithm shape and the 108-weight
// table only, reimplemented against the same maths the Rust donor runs.
// Units: the donor scores 0..100 with 100 identical; this twin publishes
// hundredths of a donor point, so 10000 is identical and 0 is worst.
// Determinism: the donor is float and not bit-reproducible across machines
// (its own test tolerates a quarter of a point), so the twin's value is a
// coarse fixed point that may differ by a unit from the Rust donor's; nothing
// here enters a hash.

// The donor's 108 weights: 6 scales x 3 components x 2 norms x 3 error maps.
export const SSIMULACRA2_WEIGHTS = [
  0.0, 0.0007376606707406586, 0.0, 0.0, 0.0007793481682867309, 0.0, 0.0, 0.0004371155730107379,
  0.0, 1.1041726426657346, 0.00066284834129271, 0.00015231632783718752, 0.0, 0.0016406437456599754,
  0.0, 1.8422455520539298, 11.441172603757666, 0.0, 0.0007989109436015163, 0.000176816438078653,
  0.0, 1.8787594979546387, 10.94906990605142, 0.0, 0.0007289346991508072, 0.9677937080626833,
  0.0, 0.00014003424285435884, 0.9981766977854967, 0.00031949755934435053, 0.0004550992113792063,
  0.0, 0.0, 0.0013648766163243398, 0.0, 0.0, 0.0, 0.0, 0.0, 7.466890328078848, 0.0,
  17.445833984131262, 0.0006235601634041466, 0.0, 0.0, 6.683678146179332, 0.00037724407979611296,
  1.027889937768264, 225.20515300849274, 0.0, 0.0, 19.213238186143016, 0.0011401524586618361,
  0.001237755635509985, 176.39317598450694, 0.0, 0.0, 24.43300999870476, 0.28520802612117757,
  0.0004485436923833408, 0.0, 0.0, 0.0, 34.77906344483772, 44.835625328877896, 0.0, 0.0, 0.0,
  0.0, 0.0, 0.0, 0.0, 0.0008680556573291698, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0005313191874358747,
  0.0, 0.00016533814161379112, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0004179171803251336,
  0.0017290828234722833, 0.0, 0.0020827005846636437, 0.0, 0.0, 8.826982764996862, 23.19243343998926,
  0.0, 95.1080498811086, 0.9863978034400682, 0.9834382792465353, 0.0012286405048278493,
  171.2667255897307, 0.9807858872435379, 0.0, 0.0, 0.0, 0.0005130064588990679, 0.0,
  0.00010854057858411537,
];
// Identical frames score this: 10000 hundredths of a donor point.
export const SSIMULACRA2_ONE = 10000;
// Smallest edge the donor accepts on each side.
export const SSIMULACRA2_MIN_EDGE = 8;
const NUM_SCALES = 6;
const fr = Math.fround;
// The donor's own build-time constants (ssimulacra2 build.rs, equations 33-57).
const SIGMA = 1.5;
const RADIUS = Math.round(3.2795 * SIGMA + 0.2546);
const SRGB_ALPHA = fr(1.0550107);
const SRGB_BETA = fr(0.0030412825);
// The opsin absorbance matrix (yuvxyb 0.4, rgb_xyb.rs).
const K_M02 = fr(0.078), K_M00 = fr(0.3), K_M01 = fr(1.0 - K_M02 - K_M00);
const K_M12 = fr(0.078), K_M10 = fr(0.23), K_M11 = fr(1.0 - K_M12 - K_M10);
const K_M20 = fr(0.24342269), K_M21 = fr(0.20476745), K_M22 = fr(1.0 - K_M20 - K_M21);
const K_B0 = fr(0.0037930734);
const OPSIN = [K_M00, K_M01, K_M02, K_M10, K_M11, K_M12, K_M20, K_M21, K_M22];
const BIAS = [K_B0, K_B0, K_B0];
/** sRGB electro-optical transfer function (yuvxyb 0.4, transfer.rs). */
function srgbEotf(x) {
  const v = fr(Math.max(x, 0.0));
  return v < fr(12.92 * SRGB_BETA) ? fr(v / 12.92) : fr(Math.pow(fr((v + fr(SRGB_ALPHA - 1.0)) / SRGB_ALPHA), 2.4));
}
/** The donor's cbrt (yuvxyb-math 0.1.1, cbrtf.rs): two Newton steps. */
function cbrtf(x) {
  if (x === 0) return 0;
  const negative = x < 0;
  const a = negative ? -x : x;
  let t = Math.cbrt(a);
  for (let i = 0; i < 2; i++) { const r = t * t * t; t = (t * (a + a + r)) / (a + r + r); }
  const out = fr(t);
  return negative ? -out : out;
}
/** Solve a 3x3 system by Gaussian elimination with partial pivoting. */
function solve3(rows, rhs) {
  const m = rows.map((row, i) => [...row, rhs[i]]);
  for (let c = 0; c < 3; c++) {
    let piv = c;
    for (let r = c + 1; r < 3; r++) if (Math.abs(m[r][c]) > Math.abs(m[piv][c])) piv = r;
    [m[c], m[piv]] = [m[piv], m[c]];
    const d = m[c][c];
    for (let k = c; k < 4; k++) m[c][k] /= d;
    for (let r = 0; r < 3; r++) {
      if (r === c) continue;
      const factor = m[r][c];
      for (let k = c; k < 4; k++) m[r][k] -= factor * m[c][k];
    }
  }
  return [m[0][3], m[1][3], m[2][3]];
}
/** The recursive Gaussian's filter weights (ssimulacra2 build.rs). */
function gaussianConstants() {
  const piDiv2r = Math.PI / (2.0 * RADIUS);
  const omega = [piDiv2r, 3.0 * piDiv2r, 5.0 * piDiv2r];
  const p1 = 1.0 / Math.tan(0.5 * omega[0]), p3 = -1.0 / Math.tan(0.5 * omega[1]), p5 = 1.0 / Math.tan(0.5 * omega[2]);
  const r1 = (p1 * p1) / Math.sin(omega[0]), r3 = (-p3 * p3) / Math.sin(omega[1]), r5 = (p5 * p5) / Math.sin(omega[2]);
  const negHalfSigma2 = -0.5 * SIGMA * SIGMA, recipRadius = 1.0 / RADIUS;
  const rho = omega.map((w) => Math.exp(negHalfSigma2 * w * w) * recipRadius);
  const d13 = p1 * r3 + -r1 * p3, d35 = p3 * r5 + -r3 * p5, d51 = p5 * r1 + -r5 * p1;
  const recipD13 = 1.0 / d13;
  const zeta15 = d35 * recipD13, zeta35 = d51 * recipD13;
  const rows = [[p1, p3, p5], [r1, r3, r5], [zeta15, zeta35, 1.0]];
  const gamma = [1.0, RADIUS * RADIUS - SIGMA * SIGMA, zeta15 * rho[0] + zeta35 * rho[1] + rho[2]];
  const beta = solve3(rows, gamma).map(fr);
  // The donor's own check: the filter weights normalize to 1.
  const norm = fr(beta[2] * p5 + fr(beta[0] * p1 + beta[1] * p3));
  if (Math.abs(norm - 1.0) > 1e-9) throw new Error(`the Gaussian weights do not normalize: ${norm}`);
  const n2 = [], d1 = [], mulIn = [], mulPrev = [], mulPrev2 = [];
  for (let i = 0; i < 3; i++) {
    n2.push(fr(-beta[i] * Math.cos(omega[i] * (RADIUS + 1.0))));
    d1.push(fr(-2.0 * Math.cos(omega[i])));
    const d2 = fr(d1[i] * d1[i]);
    mulIn.push(fr(n2[i]), fr(-d1[i] * n2[i]), fr(d2 * n2[i] - n2[i]), fr((-d2 * d1[i]) * n2[i] + 2.0 * d1[i] * n2[i]));
    mulPrev.push(fr(-d1[i]), fr(d2 - 1.0), fr(-d2 * d1[i] + 2.0 * d1[i]), fr(d2 * d2 + 3.0 * (-d2 + 1.0)));
    mulPrev2.push(fr(-1.0), fr(d1[i]), fr(-d2 + 1.0), fr(d2 * d1[i] - 2.0 * d1[i]));
  }
  return { n2, d1, mulIn, mulPrev, mulPrev2 };
}
const GAUSS = gaussianConstants();
/** One row of the recursive Gaussian (ssimulacra2 blur/gaussian.rs). */
function gaussianRow(input, output, width) {
  const bigN = RADIUS;
  let prev1 = 0, prev3 = 0, prev5 = 0, prev2a = 0, prev2b = 0, prev2c = 0;
  for (let n = -bigN + 1; n < width; n++) {
    const left = n - bigN - 1, right = n + bigN - 1;
    const leftVal = left >= 0 ? input[left] : 0, rightVal = right < width ? input[right] : 0;
    const sum = fr(leftVal + rightVal);
    let out1 = fr(sum * GAUSS.mulIn[0]), out3 = fr(sum * GAUSS.mulIn[4]), out5 = fr(sum * GAUSS.mulIn[8]);
    out1 = fr(GAUSS.mulPrev2[0] * prev2a + out1);
    out3 = fr(GAUSS.mulPrev2[4] * prev2b + out3);
    out5 = fr(GAUSS.mulPrev2[8] * prev2c + out5);
    prev2a = prev1; prev2b = prev3; prev2c = prev5;
    out1 = fr(GAUSS.mulPrev[0] * prev1 + out1);
    out3 = fr(GAUSS.mulPrev[4] * prev3 + out3);
    out5 = fr(GAUSS.mulPrev[8] * prev5 + out5);
    prev1 = out1; prev3 = out3; prev5 = out5;
    if (n >= 0) output[n] = fr(out1 + out3 + out5);
  }
}
/** One column of the recursive Gaussian (ssimulacra2 blur/gaussian.rs). */
function gaussianColumn(input, output, width, height) {
  const bigN = RADIUS;
  for (let x = 0; x < width; x++) {
    let prev = [0, 0, 0], prev2 = [0, 0, 0];
    const out = [0, 0, 0];
    for (let n = -bigN + 1; n < height; n++) {
      const top = n - bigN - 1, bottom = n + bigN - 1;
      const topVal = top >= 0 ? input[top * width + x] : 0, bottomVal = bottom < height ? input[bottom * width + x] : 0;
      const sum = fr(topVal + bottomVal);
      const p1 = fr(GAUSS.d1[0] * prev[0] + prev2[0]);
      const p3 = fr(GAUSS.d1[1] * prev[1] + prev2[1]);
      const p5 = fr(GAUSS.d1[2] * prev[2] + prev2[2]);
      out[0] = fr(sum * GAUSS.n2[0] - p1);
      out[1] = fr(sum * GAUSS.n2[1] - p3);
      out[2] = fr(sum * GAUSS.n2[2] - p5);
      if (n >= 0) output[n * width + x] = fr(out[0] + out[1] + out[2]);
      prev2 = prev; prev = out.slice();
    }
  }
}
/** The donor's plane blur: one horizontal pass then one vertical pass. */
function blurPlane(plane, width, height) {
  const temp = new Float32Array(width * height), out = new Float32Array(width * height);
  for (let y = 0; y < height; y++) gaussianRow(plane.subarray(y * width, (y + 1) * width), temp.subarray(y * width, (y + 1) * width), width);
  gaussianColumn(temp, out, width, height);
  return out;
}
/** Linear RGB to XYB (yuvxyb 0.4, rgb_xyb.rs). */
function linearToXyb(rgb) {
  const absorbBias = BIAS.map((b) => -cbrtf(b));
  const out = new Float32Array(rgb.length);
  for (let i = 0; i < rgb.length; i += 3) {
    const mixed = [0, 0, 0];
    for (let c = 0; c < 3; c++) {
      let m = fr(OPSIN[c * 3] * rgb[i] + OPSIN[c * 3 + 1] * rgb[i + 1] + OPSIN[c * 3 + 2] * rgb[i + 2] + BIAS[c]);
      if (m < 0) m = 0;
      mixed[c] = fr(cbrtf(m) + absorbBias[c]);
    }
    out[i] = fr(0.5 * (mixed[0] - mixed[1]));
    out[i + 1] = fr(0.5 * (mixed[0] + mixed[1]));
    out[i + 2] = mixed[2];
  }
  return out;
}
/** The donor's make_positive_xyb (ssimulacra2 lib.rs). */
function makePositiveXyb(xyb) {
  for (let i = 0; i < xyb.length; i += 3) {
    xyb[i + 2] = fr(fr(xyb[i + 2] - xyb[i + 1]) + 0.55);
    xyb[i] = fr(fr(xyb[i]) * 14.0 + 0.42);
    xyb[i + 1] = fr(xyb[i + 1] + 0.01);
  }
}
/** The donor's downscale_by_2 (ssimulacra2 lib.rs). */
function downscaleBy2(rgb, width, height) {
  const outW = Math.ceil(width / 2), outH = Math.ceil(height / 2);
  const out = new Float32Array(outW * outH * 3), normalize = fr(1.0 / 4.0);
  for (let oy = 0; oy < outH; oy++) for (let ox = 0; ox < outW; ox++) for (let c = 0; c < 3; c++) {
    let sum = 0;
    for (let iy = 0; iy < 2; iy++) for (let ix = 0; ix < 2; ix++) {
      const x = Math.min(ox * 2 + ix, width - 1), y = Math.min(oy * 2 + iy, height - 1);
      sum = fr(sum + rgb[(y * width + x) * 3 + c]);
    }
    out[(oy * outW + ox) * 3 + c] = fr(sum * normalize);
  }
  return { data: out, width: outW, height: outH };
}
/** The donor's ssim_map (ssimulacra2 lib.rs): six averages per plane. */
function ssimMap(width, height, m1, m2, s11, s22, s12) {
  const one = 1.0 / (width * height);
  const averages = new Float64Array(6);
  for (let c = 0; c < 3; c++) {
    let sum0 = 0, sum1 = 0;
    for (let i = 0; i < width * height; i++) {
      const mu1 = m1[c][i], mu2 = m2[c][i];
      const mu11 = fr(mu1 * mu1), mu22 = fr(mu2 * mu2), mu12 = fr(mu1 * mu2);
      const muDiff = fr(mu1 - mu2);
      const numM = fr(muDiff * -muDiff + 1.0);
      const numS = fr(2.0 * (fr(s12[c][i] - mu12)) + 0.0009);
      const denomS = fr(fr(s11[c][i] - mu11) + fr(s22[c][i] - mu22) + 0.0009);
      let d = 1.0 - fr(fr(numM * numS) / denomS);
      if (d < 0) d = 0;
      sum0 += d;
      sum1 += d * d * d * d;
    }
    averages[c * 2] = one * sum0;
    averages[c * 2 + 1] = Math.sqrt(Math.sqrt(one * sum1));
  }
  return averages;
}
/** The donor's edge_diff_map (ssimulacra2 lib.rs): twelve averages. */
function edgeDiffMap(width, height, img1, mu1, img2, mu2) {
  const one = 1.0 / (width * height);
  const averages = new Float64Array(12);
  for (let c = 0; c < 3; c++) {
    const sums = [0, 0, 0, 0];
    for (let i = 0; i < width * height; i++) {
      const d1 = fr(1.0 + Math.abs(fr(img2[c][i] - mu2[c][i]))) / fr(1.0 + Math.abs(fr(img1[c][i] - mu1[c][i]))) - 1.0;
      const artifact = Math.max(d1, 0), detailLost = Math.max(-d1, 0);
      sums[0] += artifact; sums[1] += artifact * artifact * artifact * artifact;
      sums[2] += detailLost; sums[3] += detailLost * detailLost * detailLost * detailLost;
    }
    for (let k = 0; k < 4; k++) averages[c * 4 + k] = k % 2 === 0 ? one * sums[k] : Math.sqrt(Math.sqrt(one * sums[k]));
  }
  return averages;
}
/** RGBA bytes to linear RGB floats (the donor's Rgb -> LinearRgb path). */
function toLinearRgb(rgba, count) {
  const out = new Float32Array(count * 3);
  for (let i = 0; i < count; i++) {
    out[i * 3] = srgbEotf(fr(rgba[i * 4] / 255.0));
    out[i * 3 + 1] = srgbEotf(fr(rgba[i * 4 + 1] / 255.0));
    out[i * 3 + 2] = srgbEotf(fr(rgba[i * 4 + 2] / 255.0));
  }
  return out;
}
/** The donor's score: the weighted sum over the scales (ssimulacra2 lib.rs). */
function donorScore(rgbaA, rgbaB, width, height) {
  let img1 = toLinearRgb(rgbaA, width * height), img2 = toLinearRgb(rgbaB, width * height);
  let w = width, h = height;
  const scales = [];
  for (let scale = 0; scale < NUM_SCALES; scale++) {
    if (w < 8 || h < 8) break;
    if (scale > 0) {
      const a = downscaleBy2(img1, w, h), b = downscaleBy2(img2, w, h);
      img1 = a.data; img2 = b.data; w = a.width; h = a.height;
    }
    const xyb1 = linearToXyb(img1), xyb2 = linearToXyb(img2);
    makePositiveXyb(xyb1); makePositiveXyb(xyb2);
    const planes = (xyb) => {
      const out = [new Float32Array(w * h), new Float32Array(w * h), new Float32Array(w * h)];
      for (let i = 0; i < w * h; i++) { out[0][i] = xyb[i * 3]; out[1][i] = xyb[i * 3 + 1]; out[2][i] = xyb[i * 3 + 2]; }
      return out;
    };
    const p1 = planes(xyb1), p2 = planes(xyb2);
    const mul = (a, b) => a.map((plane, c) => {
      const out = new Float32Array(plane.length);
      for (let i = 0; i < plane.length; i++) out[i] = fr(plane[i] * b[c][i]);
      return out;
    });
    const blur = (planesIn) => planesIn.map((plane) => blurPlane(plane, w, h));
    const s11 = blur(mul(p1, p1)), s22 = blur(mul(p2, p2)), s12 = blur(mul(p1, p2));
    const mu1 = blur(p1), mu2 = blur(p2);
    scales.push({ avgSsim: ssimMap(w, h, mu1, mu2, s11, s22, s12), avgEdge: edgeDiffMap(w, h, p1, mu1, p2, mu2) });
  }
  let ssim = 0.0, i = 0;
  for (let c = 0; c < 3; c++) for (const scale of scales) for (let n = 0; n < 2; n++) {
    ssim = fr(SSIMULACRA2_WEIGHTS[i] * Math.abs(scale.avgSsim[c * 2 + n]) + ssim); i++;
    ssim = fr(SSIMULACRA2_WEIGHTS[i] * Math.abs(scale.avgEdge[c * 4 + n]) + ssim); i++;
    ssim = fr(SSIMULACRA2_WEIGHTS[i] * Math.abs(scale.avgEdge[c * 4 + n + 2]) + ssim); i++;
  }
  ssim = fr(ssim * 0.9562382616834844);
  ssim = fr(fr(6.248496625763138e-5 * ssim * ssim) * ssim + fr(fr(2.326765642916932 * ssim) + -0.020884521182843837 * ssim * ssim));
  if (ssim > 0) ssim = fr(100.0 - Math.pow(ssim, 0.6276336467831387) * 10.0);
  else ssim = 100.0;
  return ssim;
}
/**
 * SSIMULACRA2 v2.1 for two RGBA frames of the same size.
 *
 * The source is the pristine reference and the distorted one is the capture.
 * The score is published in hundredths of a donor point, so 10000 is
 * identical and 0 is worst.
 *
 * @throws when the frames differ in size or a side is under 8x8.
 */
export function ssimulacra2X10000(source, distorted, width, height) {
  if (source.length !== distorted.length || source.length !== width * height * 4) throw new Error("size mismatch");
  if (width < SSIMULACRA2_MIN_EDGE || height < SSIMULACRA2_MIN_EDGE) throw new Error("frame under 8x8");
  const points = donorScore(source, distorted, width, height);
  if (!Number.isFinite(points)) return 0;
  return Math.max(0, Math.min(SSIMULACRA2_ONE, Math.round(points * 100)));
}
export function scorePair(a, b, w, h, th = DEFAULT_THRESHOLD) { if (a.length !== b.length || a.length !== w * h * 4) throw new Error("size mismatch"); const s = ssimX10000(a, b, w, h), d = meanDeX100(a, b), f = flipX10000(a, b, w, h); let second = null; try { second = ssimulacra2X10000(a, b, w, h); } catch { second = null; } return { ssim_x10000: s, flip_x10000: f.flip_x10000, ssimulacra2: second, butteraugli: null, mean_de_x100: d, verdict: s >= th ? "pass" : "fail", note: NOTE }; }
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
