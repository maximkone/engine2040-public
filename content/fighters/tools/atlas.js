// CT7: minimal PNG codec (8-bit RGB/RGBA, non-interlaced) + atlas composite.
const fs = require('fs');
const crypto = require('crypto');
const zlib = require('zlib');

function readChunk(b, pos) {
  const len = b.readUInt32BE(pos);
  const type = b.toString('ascii', pos + 4, pos + 8);
  return { len, type, data: b.subarray(pos + 8, pos + 8 + len), next: pos + 12 + len };
}

function decodePng(buf) {
  if (buf.readUInt32BE(0) !== 0x89504e47 || buf.readUInt32BE(4) !== 0x0d0a1a0a) throw new Error('bad sig');
  let pos = 8, w = 0, h = 0, ctype = 0, depth = 0, interlace = 0;
  const idat = [];
  for (;;) {
    const c = readChunk(buf, pos);
    pos = c.next;
    if (c.type === 'IHDR') {
      w = c.data.readUInt32BE(0); h = c.data.readUInt32BE(4);
      depth = c.data[8]; ctype = c.data[9]; interlace = c.data[12];
    } else if (c.type === 'IDAT') idat.push(c.data);
    else if (c.type === 'IEND') break;
  }
  if (depth !== 8 || interlace !== 0) throw new Error(`unsupported depth=${depth} interlace=${interlace}`);
  const ch = ctype === 6 ? 4 : ctype === 2 ? 3 : 0;
  if (!ch) throw new Error('unsupported ctype ' + ctype);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = w * ch;
  const out = Buffer.alloc(w * h * 4);
  const get = (yy, xx, k) => (yy < 0 || xx < 0 ? 0 : out[(yy * w + xx) * 4 + k]);
  let p = 0;
  for (let y = 0; y < h; y++) {
    const f = raw[p++];
    for (let x = 0; x < w; x++) {
      for (let k = 0; k < ch; k++) {
        const a = get(y, x - 1, k), b = get(y - 1, x, k), c = get(y - 1, x - 1, k);
        let pred = 0;
        if (f === 1) pred = a;
        else if (f === 2) pred = b;
        else if (f === 3) pred = (a + b) >> 1;
        else if (f === 4) {
          const pa = Math.abs(b - c), pb = Math.abs(a - c), pc = Math.abs(a + b - 2 * c);
          pred = (pa <= pb && pa <= pc) ? a : (pb <= pc ? b : c);
        } else if (f !== 0) throw new Error('unsupported filter ' + f);
        out[(y * w + x) * 4 + k] = (raw[p] + pred) & 255;
        p++;
      }
      if (ch === 3) out[(y * w + x) * 4 + 3] = 255;
    }
  }
  return { w, h, px: out };
}

function encodePng(w, h, px) {
  const stride = w * 4;
  const raw = Buffer.alloc(h * (stride + 1));
  for (let y = 0; y < h; y++) {
    raw[y * (stride + 1)] = 0;
    px.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const idat = zlib.deflateSync(raw, { level: 9 });
  const parts = [Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])];
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; ihdr[9] = 6;
  const mk = (type, data) => {
    const h = Buffer.alloc(8);
    h.writeUInt32BE(data.length, 0); h.write(type, 4, 'ascii');
    const c = Buffer.alloc(4);
    c.writeUInt32BE((crc(Buffer.concat([Buffer.from(type, 'ascii'), data])) >>> 0), 0);
    return Buffer.concat([h, data, c]);
  };
  parts.push(mk('IHDR', ihdr), mk('IDAT', idat), mk('IEND', Buffer.alloc(0)));
  return Buffer.concat(parts);
}

// CRC32 (ISO HDLC) for PNG chunks.
const CRC_T = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = (c & 1) ? (0xedb88320 ^ (c >>> 1)) : (c >>> 1);
    t[n] = c;
  }
  return t;
})();
function crc(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_T[(c ^ buf[i]) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) | 0;
}

const LANE = 'C:/engine2040/.lanes/ct19/content/fighters';
const TEXDIR = LANE + '/incoming/modular/modular_rpg_characters/textures';
fs.mkdirSync(LANE + '/atlas', { recursive: true });
// CT19 weapon strip: 512x512 of lane-original steel/leather, composited at
// atlas px (0,1024) — the free half of the Q2 quadrant. Blade, guard, shield
// and grip zones; integer math only, so the cook is identical twice.
function weaponStrip() {
  const S = 512;
  const px = Buffer.alloc(S * S * 4);
  const set = (x, y, r, g, b) => {
    const o = (y * S + x) * 4;
    px[o] = r; px[o + 1] = g; px[o + 2] = b; px[o + 3] = 255;
  };
  for (let y = 0; y < S; y++)
    for (let x = 0; x < S; x++) set(x, y, 0x77, 0x77, 0x77);
  const brush = (y) => Math.round(4 * Math.sin(y * 0.33) + 3 * Math.sin(y * 0.117 + 1.7));
  for (let y = 4; y < 508; y++) {
    const tip = 1 - 0.12 * (y / 511);
    const n = brush(y);
    for (let x = 8; x < 200; x++) {
      const f = x < 96 ? (x - 8) / 88 : (200 - x) / 104; // edge 0 -> center 1
      let r = 0x5f + Math.round((0xe9 - 0x5f) * f);
      let g = 0x64 + Math.round((0xed - 0x64) * f);
      let b = 0x6b + Math.round((0xf1 - 0x6b) * f);
      if (x >= 96 && x < 120) { r = 0x7b; g = 0x80; b = 0x87; } // fuller
      if (x >= 106 && x < 110) { r = 0x67; g = 0x6c; b = 0x73; } // fuller line
      set(x, y,
        Math.max(0, Math.min(255, Math.round(r * tip) + n)),
        Math.max(0, Math.min(255, Math.round(g * tip) + n)),
        Math.max(0, Math.min(255, Math.round(b * tip) + n)));
    }
    for (let x = 208; x < 248; x++) set(x, y, x === 208 ? 0x6a : 0x3f, x === 208 ? 0x70 : 0x44, x === 208 ? 0x76 : 0x4a);
    for (let x = 264; x < 456; x++) {
      const dx = (x - 360) / 200, dy = (y - 256) / 200;
      const d2 = Math.min(1, dx * dx + dy * dy);
      const f = 1 - 0.28 * d2;
      const rim = (x - 264 < 10 || 456 - x <= 10 || y - 4 < 10 || 508 - y <= 10);
      set(x, y, rim ? 0x4c : Math.round(0xa9 * f), rim ? 0x51 : Math.round(0xaf * f), rim ? 0x57 : Math.round(0xb6 * f));
    }
    for (let x = 464; x < 504; x++) {
      const wrap = ((x * 2 + y) % 32) < 10;
      const edge = (x === 464 || x === 503);
      set(x, y, edge ? 0x7a : (wrap ? 0x3e : 0x5b), edge ? 0x5a : (wrap ? 0x2a : 0x3e), edge ? 0x3a : (wrap ? 0x1a : 0x27));
    }
  }
  return { w: S, h: S, px };
}
const N = 2048;
for (const v of ['hero', 'opp_male_light', 'opp_female_medium', 'opp_female_light']) {
  const man = JSON.parse(fs.readFileSync(`${LANE}/incoming/staged/${v}.parts.json`, 'utf8'));
  const atlas = Buffer.alloc(N * N * 4, 0);
  for (const p of man.parts) {
    const [tx, ty, tw] = p.tile;
    const src = decodePng(fs.readFileSync(`${TEXDIR}/${p.texture}`));
    if (src.w !== tw || src.h !== tw) throw new Error(`size mismatch ${p.texture}: ${src.w}x${src.h} vs tile ${tw}`);
    for (let y = 0; y < tw; y++)
      for (let x = 0; x < tw; x++)
        for (let k = 0; k < 4; k++)
          atlas[((ty + y) * N + (tx + x)) * 4 + k] = src.px[(y * tw + x) * 4 + k];
  }
  const strip = weaponStrip();
  for (let y = 0; y < 512; y++)
    for (let x = 0; x < 512; x++)
      for (let k = 0; k < 4; k++)
        atlas[((1024 + y) * N + x) * 4 + k] = strip.px[(y * 512 + x) * 4 + k];
  const png = encodePng(N, N, atlas);
  const dest = `${LANE}/atlas/${v}_atlas.png`;
  fs.writeFileSync(dest, png);
  console.log(v, dest, png.length, 'sha256=' + crypto.createHash('sha256').update(png).digest('hex'));
  // Quadrant crops (1024^2 each): Q0 armor, Q1 body/head/hair/hands,
  // Q2 feet/boots, Q3 gloves/helmet. One PNG + one material per quadrant.
  const Q = 1024;
  const quads = [[0, 0], [Q, 0], [0, Q], [Q, Q]];
  quads.forEach(([qx, qy], qi) => {
    const qp = Buffer.alloc(Q * Q * 4);
    for (let y = 0; y < Q; y++)
      for (let x = 0; x < Q; x++)
        for (let k = 0; k < 4; k++)
          qp[(y * Q + x) * 4 + k] = atlas[((qy + y) * N + (qx + x)) * 4 + k];
    const qpng = encodePng(Q, Q, qp);
    const qdest = `${LANE}/atlas/${v}_atlas_q${qi}.png`;
    fs.writeFileSync(qdest, qpng);
    console.log(v, qdest, qpng.length, 'sha256=' + crypto.createHash('sha256').update(qpng).digest('hex'));
  });
}
