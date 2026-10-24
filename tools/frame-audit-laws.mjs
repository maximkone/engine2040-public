// frame-audit-laws.mjs - the spatial and visual laws behind tools/frame-audit.mjs.
//
// Why: the window's own view law said `view=ok` on a frame with no shadows, a black sword
// and shield, a spear standing up out of a lying body and a crowd of flat boxes
// (target/clips/showcase/frame_000002.png, 2026-10-24). Those laws judge bodies as boxes.
// These judge the PICTURE against the posed truth the window writes beside every captured
// frame (`view-audit.jsonl`, crates/game/src/view_truth/audit.rs): world bounds, true length,
// material factors and surface points of every body part, the hand and socket joints, every
// spear. Each law reads the PNG under projected points, so a bug that never touches a number
// (a texture that never bound, a shadow pass that draws nothing) still goes red.
//
// Laws (every one answers per frame, with the subjects that broke it):
//   shadow   each body stands in a darker footprint than the floor around it
//   material no drawn part renders near-black and flat (an unbound texture, a missing material)
//   scale    each kit part (blade, shield, spear) and each body sits in its real size range
//   attach   a weapon's grip sits at the hand, a shield on the arm, a spear in a live hand or
//            inside the body it is stuck in at a plausible angle
//
// Pure: no file IO, no cargo. The CLI (frame-audit.mjs) reads the files and prints.
// Units: meters, radians for the camera, pixels for the image, luma 0..1 (Rec.709 on sRGB bytes).

export const GROUND_Y = 0;

// Thresholds. Every one is a stated floor, tuned on real frames (docs/book/ref-tools.md).
export const T = {
  // shadow
  shadowMinBodyShare: 0.1, // a body shorter than this share of the picture height is too small to judge
  shadowReachM: 5, // floor out to this far from the feet is searched for the body's shadow
  shadowCellM: 0.1, // floor cell size of the search
  shadowRefM: 3.5, // floor further than this from the feet is the reference light
  shadowPassM: 1.2, // cells hidden behind the body pass a shadow on only this close to the feet
  shadowDim: 0.9, // a floor cell this fraction of the reference luma or less is in shadow
  shadowCore: 0.72, // and a shadow has a core at most this fraction (soft ambient darkening alone does not)
  shadowMinCore: 4, // core cells needed
  shadowMinCells: 20, // shadowed cells joined to the feet that make a shadow (about 0.2 m2)
  shadowMinRef: 20, // reference cells needed
  // material
  materialDarkLuma: 0.2, // a sample this dark with nothing bright around it is near-black
  materialFlatP90: 0.24, // 90th percentile luma of the patch: no bright texture detail
  materialShare: 0.6, // share of a part's usable samples that must be dark and flat to fail
  materialMinSamples: 3,
  slabLuma: 0.06, // a pixel this dark is black
  slabMinPx: 400, // black pixels with black on every side that make a slab
  slabMinShare: 0.015, // and the share of the body's screen box they fill
  pixelSlabLuma: 0.12, // the pixel-only read (no audit rows): dark enough to be a part with no material
  pixelSlabMinPx: 1500, // the largest dark blob that fails
  pixelSlabMaxShare: 0.4, // a picture this dark is a night frame: not judged
  // attach
  bladeReachM: 0.35, // the blade's box to the right-hand socket: the grip and guard sit between
  swordPartsM: 0.2, // a sword's parts (blade, fittings, grip) touch: box to box
  shieldReachM: 0.3, // the shield's box to the left-hand socket
  spearHandM: 0.15, // a held spear's shaft to the palm that holds it
  spearBodyPadM: 0.3, // a stuck spear's tip may sit this far outside the victim's bounds
  spearMaxElevDeg: 70, // a stuck spear steeper than this is not a throw
  liveBodyMinM: 1.0, // a body shorter than this (standing height) is lying
};

// The kit's real sizes by material name: length is the part's longest span in meters,
// ratio is that span over the body's stature. A part outside either range fails `scale`.
export const KIT = [
  { match: /^gladius_blade$/, role: 'blade', lenM: [0.35, 0.85], ratio: [0.18, 0.5] },
  { match: /^gladius_grip$/, role: 'grip', lenM: [0.05, 0.25], ratio: [0.02, 0.15] },
  { match: /^scutum_(board|face)$/, role: 'shield', lenM: [0.7, 1.5], ratio: [0.4, 0.85] },
];
export const SPEAR_LEN_M = [1.4, 2.4];
export const STATURE_M = [1.4, 2.2];

// Parts of equipment, by material name: not the body, so not its stature or footprint.
const EQUIPMENT = /^(gladius|scutum|gear_|spear|pilum|sword|shield|pugio|clava|axe|sm002_combat_knife)/i;
export const isEquipment = (name) => EQUIPMENT.test(name || '');

// HUD rectangles as image shares [x0, y0, x1, y1]: pixels under them are never floor.
const HUD = [[0, 0.82, 0.22, 1]];

const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const len = (a) => Math.sqrt(dot(a, a));
const norm = (a) => { const l = len(a) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };
const dist = (a, b) => len(sub(a, b));
// Distance from point p to the segment a..b.
const segmentDist = (p, a, b) => {
  const ab = sub(b, a);
  const t = Math.max(0, Math.min(1, dot(sub(p, a), ab) / (dot(ab, ab) || 1)));
  return dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]);
};
const finite3 = (v) => Array.isArray(v) && v.length === 3 && v.every(Number.isFinite);

// The frame camera as a pixel projector: the same look-at maths as view_truth::project_ndc.
// Returns null behind the near plane.
export function makeProjector(camera, width, height) {
  const f = norm(sub(camera.target, camera.eye));
  const r = norm(cross(f, camera.up));
  const u = cross(r, f);
  const half = Math.tan(camera.fov_y / 2);
  const aspect = camera.aspect > 0 ? camera.aspect : width / height;
  return (p) => {
    const d = sub(p, camera.eye);
    const depth = dot(d, f);
    if (!(depth > camera.near)) return null;
    const x = dot(d, r) / (depth * half * aspect);
    const y = dot(d, u) / (depth * half);
    return { x: ((x + 1) / 2) * width, y: ((1 - y) / 2) * height, depth, ndc: [x, y] };
  };
}

export const luma = (r, g, b) => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;

export function lumaAt(img, x, y) {
  const ix = Math.round(x);
  const iy = Math.round(y);
  if (ix < 0 || iy < 0 || ix >= img.width || iy >= img.height) return null;
  const o = (iy * img.width + ix) * 4;
  return luma(img.data[o], img.data[o + 1], img.data[o + 2]);
}

// Luma of the (2r+1)^2 patch around a pixel, sorted ascending; null when it leaves the image.
export function patchLumas(img, x, y, r) {
  const cx = Math.round(x);
  const cy = Math.round(y);
  if (cx - r < 0 || cy - r < 0 || cx + r >= img.width || cy + r >= img.height) return null;
  const out = [];
  for (let j = -r; j <= r; j += 1) {
    for (let i = -r; i <= r; i += 1) out.push(lumaAt(img, cx + i, cy + j));
  }
  return out.sort((a, b) => a - b);
}

const quantile = (sorted, q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
const median = (xs) => quantile([...xs].sort((a, b) => a - b), 0.5);
const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length;
const round = (v, n = 2) => Math.round(v * 10 ** n) / 10 ** n;

const partsOf = (body) => body.parts || [];
const bodyParts = (body) => partsOf(body).filter((p) => !isEquipment(p.name) && finite3(p.min) && finite3(p.max));

// Union box of the body's own (non-equipment) parts; falls back to the body bounds.
export function bodyBox(body) {
  const ps = bodyParts(body);
  if (ps.length === 0) return finite3(body.min) && finite3(body.max) ? { min: body.min, max: body.max } : null;
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  for (const p of ps) {
    for (let k = 0; k < 3; k += 1) {
      min[k] = Math.min(min[k], p.min[k]);
      max[k] = Math.max(max[k], p.max[k]);
    }
  }
  return { min, max };
}

// The body's stature: its longest box extent (a lying body is as long as a standing one is tall).
export function stature(body) {
  const box = bodyBox(body);
  if (!box) return null;
  return Math.max(box.max[0] - box.min[0], box.max[1] - box.min[1], box.max[2] - box.min[2]);
}

// Height of the body's own parts above the lowest one: small for a body lying down.
const standingHeight = (body) => {
  const box = bodyBox(body);
  return box ? box.max[1] - box.min[1] : null;
};

// A body is alive and upright: not down, not a ragdoll, and tall.
export function isUpright(body) {
  return body.act !== 'down' && !body.ragdolled && (standingHeight(body) ?? 0) >= T.liveBodyMinM;
}

function screenBox(project, min, max) {
  const xs = [];
  const ys = [];
  for (let i = 0; i < 8; i += 1) {
    const p = project([i & 1 ? max[0] : min[0], i & 2 ? max[1] : min[1], i & 4 ? max[2] : min[2]]);
    if (!p) return null;
    xs.push(p.x);
    ys.push(p.y);
  }
  return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
}

const inBox = (b, x, y, pad = 0) => x >= b.x0 - pad && x <= b.x1 + pad && y >= b.y0 - pad && y <= b.y1 + pad;

// ---------------------------------------------------------------------------------------
// shadow: a body whose feet touch no darker floor is drawn floating.
// ---------------------------------------------------------------------------------------
// The floor around each body is cut into cells and read from the picture. A cell is shadowed when
// it is clearly darker than the floor further out. The body must have a shadow: a run of shadowed
// cells joined to its feet, in any direction (the sun is low and the shadow may fall metres
// away), passing through the cells the body itself hides. Floor under another body or the HUD is
// never read.
function floorCells(project, img, covers, hud, box, ownPass) {
  const { shadowReachM: R, shadowCellM: C } = T;
  const nx = Math.ceil((box.max[0] - box.min[0] + 2 * R) / C);
  const nz = Math.ceil((box.max[2] - box.min[2] + 2 * R) / C);
  const cells = [];
  for (let i = 0; i < nx; i += 1) {
    for (let j = 0; j < nz; j += 1) {
      const x = box.min[0] - R + (i + 0.5) * C;
      const z = box.min[2] - R + (j + 0.5) * C;
      const gap = Math.hypot(Math.max(box.min[0] - x, 0, x - box.max[0]), Math.max(box.min[2] - z, 0, z - box.max[2]));
      const q = project([x, GROUND_Y, z]);
      let luma = null;
      if (q && !covers.some((c) => inBox(c, q.x, q.y, 6)) && !hud.some((c) => inBox(c, q.x, q.y, 4))) luma = lumaAt(img, q.x, q.y);
      cells.push({ i, j, gap, luma, passable: luma === null && gap <= ownPass });
    }
  }
  return { cells, nx, nz };
}

export function shadowLaw(row, img, project) {
  const bodies = (row.bodies || []).filter((b) => bodyBox(b));
  const covers = [];
  for (const b of bodies) {
    const sb = screenBox(project, b.min || bodyBox(b).min, b.max || bodyBox(b).max);
    if (sb) covers.push(sb);
  }
  for (const s of row.spears || []) {
    const tail = [s.tip[0] - s.dir[0] * s.len, s.tip[1] - s.dir[1] * s.len, s.tip[2] - s.dir[2] * s.len];
    const a = project(s.tip);
    const c = project(tail);
    if (a && c) covers.push({ x0: Math.min(a.x, c.x), y0: Math.min(a.y, c.y), x1: Math.max(a.x, c.x), y1: Math.max(a.y, c.y) });
  }
  const hud = HUD.map(([x0, y0, x1, y1]) => ({ x0: x0 * img.width, y0: y0 * img.height, x1: x1 * img.width, y1: y1 * img.height }));
  const subjects = [];
  const detail = [];
  let judged = 0;
  for (const b of bodies) {
    const box = bodyBox(b);
    const sb = screenBox(project, box.min, box.max);
    if (!sb || sb.y1 - sb.y0 < T.shadowMinBodyShare * img.height) {
      detail.push(`slot${b.slot}: too small or behind the camera to judge`);
      continue;
    }
    const feet = [[box.min[0], box.min[2]], [box.max[0], box.min[2]], [box.min[0], box.max[2]], [box.max[0], box.max[2]]].map(([x, z]) => project([x, GROUND_Y, z]));
    if (feet.some((q) => !q || q.x < 0 || q.y < 0 || q.x >= img.width || q.y >= img.height)) {
      detail.push(`slot${b.slot}: feet are out of the picture, no floor to read`);
      continue;
    }
    const { cells, nx, nz } = floorCells(project, img, covers, hud, box, T.shadowPassM);
    const ref = cells.filter((c) => c.luma !== null && c.gap > T.shadowRefM).map((c) => c.luma);
    if (ref.length < T.shadowMinRef) {
      detail.push(`slot${b.slot}: floor hidden (${ref.length} reference cells, need ${T.shadowMinRef})`);
      continue;
    }
    judged += 1;
    const floor = median(ref);
    const dim = (c) => c.luma !== null && c.luma <= T.shadowDim * floor;
    const at = (i, j) => (i < 0 || j < 0 || i >= nx || j >= nz ? null : cells[i * nz + j]);
    const seen = new Set();
    const queue = cells.filter((c) => c.gap <= 0.5 && (dim(c) || c.passable));
    for (const c of queue) seen.add(c);
    let shadowed = 0;
    let core = 0;
    let darkest = 1;
    for (let k = 0; k < queue.length; k += 1) {
      const c = queue[k];
      if (c.luma !== null) {
        shadowed += 1;
        if (c.luma <= T.shadowCore * floor) core += 1;
      }
      for (let di = -1; di <= 1; di += 1) {
        for (let dj = -1; dj <= 1; dj += 1) {
          const n = at(c.i + di, c.j + dj);
          if (n && !seen.has(n) && (dim(n) || n.passable)) {
            seen.add(n);
            queue.push(n);
          }
        }
      }
    }
    for (const c of cells) if (c.luma !== null && c.gap < 2.5) darkest = Math.min(darkest, c.luma / floor);
    const has = shadowed >= T.shadowMinCells && core >= T.shadowMinCore;
    detail.push(`slot${b.slot}: ${shadowed} shadowed floor cells (${core} dark core) touch the feet, need ${T.shadowMinCells} (${T.shadowMinCore}); shadowed is under ${T.shadowDim} of the floor luma ${round(floor, 3)}, core under ${T.shadowCore}; darkest cell near the feet ${round(darkest)}${has ? '' : ': no shadow'}`);
    if (!has) subjects.push(`slot${b.slot}`);
  }
  return verdict(subjects, detail, judged);
}

// ---------------------------------------------------------------------------------------
// material: a part that renders near-black and flat has no working material.
// ---------------------------------------------------------------------------------------
// Pixel rule on a part's surface points: every sample reads a 5x5 patch; a sample is
// dark-and-flat when its patch median is dark and even its 90th percentile has no bright
// texture detail. A part fails when most of its on-screen samples are dark-and-flat.
export function patchIsDarkFlat(sorted) {
  return quantile(sorted, 0.5) < T.materialDarkLuma && quantile(sorted, 0.9) < T.materialFlatP90;
}

// A body's black slab: black pixels with black on all four sides inside its screen box, where a
// sword or shield without a material draws as a flat black shape. Parts cut from one big mesh (a
// whole body part per atlas quadrant) cannot be named, so the pixels name the slab instead.
export function blackSlab(b, img, project) {
  const sb = screenBox(project, b.min, b.max);
  if (!sb || sb.y1 - sb.y0 < T.shadowMinBodyShare * img.height) return null;
  const x0 = Math.max(1, Math.floor(sb.x0));
  const x1 = Math.min(img.width - 1, Math.ceil(sb.x1));
  const y0 = Math.max(1, Math.floor(sb.y0));
  const y1 = Math.min(img.height - 1, Math.ceil(sb.y1));
  const black = (x, y) => lumaAt(img, x, y) < T.slabLuma;
  let interior = 0;
  for (let y = y0; y < y1; y += 1) {
    for (let x = x0; x < x1; x += 1) {
      if (black(x, y) && black(x - 1, y) && black(x + 1, y) && black(x, y - 1) && black(x, y + 1)) interior += 1;
    }
  }
  const share = interior / Math.max(1, (x1 - x0) * (y1 - y0));
  return { px: interior, share, bad: interior >= T.slabMinPx && share >= T.slabMinShare };
}

export function materialLaw(row, img, project) {
  const subjects = [];
  const detail = [];
  let judged = 0;
  for (const b of row.bodies || []) {
    if (finite3(b.min) && finite3(b.max)) {
      const slab = blackSlab(b, img, project);
      if (slab) {
        judged += 1;
        if (slab.bad) {
          subjects.push(`slot${b.slot}:black_slab`);
          detail.push(`slot${b.slot}: ${slab.px} px (${round(slab.share * 100, 1)}% of its screen box) are black with black on every side: a part drawn with no working material`);
        }
      }
    }
    for (const part of partsOf(b)) {
      const patches = [];
      for (const p of part.pts || []) {
        const q = project(p);
        if (!q) continue;
        const patch = patchLumas(img, q.x, q.y, 2);
        if (patch) patches.push(patch);
      }
      if (patches.length < T.materialMinSamples) continue;
      judged += 1;
      const darkFlat = patches.filter(patchIsDarkFlat).length;
      const share = darkFlat / patches.length;
      const bad = share >= T.materialShare;
      if (bad) {
        const cause = part.tex ? 'texture bound but renders flat' : 'no texture, dark base colour';
        const tag = `${part.name || `part${part.i}`}@${b.slot}`;
        subjects.push(tag);
        detail.push(`${tag}: ${darkFlat}/${patches.length} samples near-black and flat (median luma ${round(median(patches.map((s) => quantile(s, 0.5))), 3)}); ${cause}; base ${part.base?.map((v) => round(v)).join(',')}`);
      }
    }
  }
  return verdict(subjects, detail, judged);
}

// ---------------------------------------------------------------------------------------
// scale: kit parts and bodies inside their real size range.
// ---------------------------------------------------------------------------------------
export function scaleLaw(row) {
  const subjects = [];
  const detail = [];
  let judged = 0;
  for (const b of row.bodies || []) {
    const st = stature(b);
    if (st === null) continue;
    if (bodyParts(b).length > 0) {
      judged += 1;
      if (st < STATURE_M[0] || st > STATURE_M[1]) {
        subjects.push(`stature@${b.slot}`);
        detail.push(`slot${b.slot}: stature ${round(st)} m outside ${STATURE_M[0]}..${STATURE_M[1]} m`);
      }
    }
    for (const part of partsOf(b)) {
      const kit = KIT.find((k) => k.match.test(part.name || ''));
      if (!kit || !Number.isFinite(part.len)) continue;
      judged += 1;
      const ratio = part.len / st;
      const lenOk = part.len >= kit.lenM[0] && part.len <= kit.lenM[1];
      const ratioOk = ratio >= kit.ratio[0] && ratio <= kit.ratio[1];
      if (!lenOk || !ratioOk) {
        subjects.push(`${part.name}@${b.slot}`);
        detail.push(`${part.name}@${b.slot}: ${round(part.len)} m long (kit ${kit.lenM[0]}..${kit.lenM[1]} m), ${round(ratio)} of stature ${round(st)} m (kit ${kit.ratio[0]}..${kit.ratio[1]})`);
      }
    }
  }
  for (const s of row.spears || []) {
    judged += 1;
    if (!(s.len >= SPEAR_LEN_M[0] && s.len <= SPEAR_LEN_M[1])) {
      subjects.push(`spear@${s.slot}`);
      detail.push(`spear@${s.slot}: ${round(s.len)} m long (kit ${SPEAR_LEN_M[0]}..${SPEAR_LEN_M[1]} m)`);
    }
  }
  return verdict(subjects, detail, judged);
}

// ---------------------------------------------------------------------------------------
// attach: grips in hands, shields on arms, spears held by the living or stuck in a body.
// ---------------------------------------------------------------------------------------
const partNamed = (body, re) => partsOf(body).find((p) => re.test(p.name || '') && finite3(p.min) && finite3(p.max));
const partsNamed = (body, re) => partsOf(body).filter((p) => re.test(p.name || '') && finite3(p.min) && finite3(p.max));
// Distance from a point to a part's world box: 0 when the point is inside it.
const boxToPoint = (p, q) => Math.hypot(...[0, 1, 2].map((k) => Math.max(p.min[k] - q[k], 0, q[k] - p.max[k])));
// Gap between two part boxes: 0 when they touch or overlap.
const boxToBox = (a, b) => Math.hypot(...[0, 1, 2].map((k) => Math.max(a.min[k] - b.max[k], 0, b.min[k] - a.max[k])));

export function attachLaw(row) {
  const subjects = [];
  const detail = [];
  let judged = 0;
  const bodies = row.bodies || [];
  for (const b of bodies) {
    const a = b.anchors || {};
    const blade = partNamed(b, /^gladius_blade$/);
    const hand = a.wep_pos_r || a.palm_r;
    if (blade && hand) {
      judged += 1;
      const d = boxToPoint(blade, hand);
      if (d > T.bladeReachM) {
        subjects.push(`gladius_blade@${b.slot}`);
        detail.push(`gladius_blade@${b.slot}: ${round(d)} m from the right-hand socket (limit ${T.bladeReachM} m): the sword is not in the hand`);
      }
    }
    for (const other of blade ? partsNamed(b, /^gladius_(?!blade)/) : []) {
      judged += 1;
      const d = boxToBox(blade, other);
      if (d > T.swordPartsM) {
        subjects.push(`${other.name}@${b.slot}`);
        detail.push(`${other.name}@${b.slot}: ${round(d)} m from its own blade (limit ${T.swordPartsM} m): the sword is in pieces`);
      }
    }
    const shield = partNamed(b, /^scutum_(board|face)$/);
    const off = a.wep_pos_l || a.palm_l;
    if (shield && off) {
      judged += 1;
      const d = boxToPoint(shield, off);
      if (d > T.shieldReachM) {
        subjects.push(`${shield.name}@${b.slot}`);
        detail.push(`${shield.name}@${b.slot}: ${round(d)} m from the left-hand socket (limit ${T.shieldReachM} m): the shield is not on the arm`);
      }
    }
  }
  for (const s of row.spears || []) {
    const tag = `spear@${s.slot}`;
    if (s.kind === 'held') {
      judged += 1;
      const holder = bodies.find((b) => b.slot === s.holder);
      const tail = [s.tip[0] - s.dir[0] * s.len, s.tip[1] - s.dir[1] * s.len, s.tip[2] - s.dir[2] * s.len];
      if (!holder) {
        subjects.push(tag);
        detail.push(`${tag}: held by slot ${s.holder} that is not drawn`);
      } else if (!isUpright(holder)) {
        subjects.push(tag);
        detail.push(`${tag}: held by slot ${s.holder} that is ${holder.ragdolled ? 'a ragdoll' : holder.act === 'down' ? 'down' : 'lying'}; the spear stands ${round(Math.asin(Math.min(1, Math.abs(s.dir[1]))) * 57.3, 0)} deg up out of it`);
      } else if (holder.anchors?.palm_r && segmentDist(holder.anchors.palm_r, tail, s.tip) > T.spearHandM) {
        subjects.push(tag);
        detail.push(`${tag}: shaft ${round(segmentDist(holder.anchors.palm_r, tail, s.tip))} m from the palm that holds it (limit ${T.spearHandM} m)`);
      }
    } else if (s.kind === 'stuck') {
      judged += 1;
      const victim = bodies.find((b) => b.slot === s.victim);
      const elev = (Math.asin(Math.max(-1, Math.min(1, s.dir[1]))) * 180) / Math.PI;
      if (!victim || !finite3(victim.min) || !finite3(victim.max)) {
        subjects.push(tag);
        detail.push(`${tag}: stuck in slot ${s.victim} that is not drawn`);
      } else {
        const pad = T.spearBodyPadM;
        const inside = [0, 1, 2].every((k) => s.tip[k] >= victim.min[k] - pad && s.tip[k] <= victim.max[k] + pad);
        if (!inside) {
          subjects.push(tag);
          detail.push(`${tag}: tip is outside slot ${s.victim}'s bounds by more than ${pad} m`);
        } else if (Math.abs(elev) > T.spearMaxElevDeg) {
          subjects.push(tag);
          detail.push(`${tag}: stuck at ${round(elev, 0)} deg elevation (a throw stays under ${T.spearMaxElevDeg})`);
        }
      }
    }
  }
  return verdict(subjects, detail, judged);
}

function verdict(subjects, detail, judged) {
  if (subjects.length > 0) return { status: 'FAIL', subjects, detail, judged };
  return { status: judged > 0 ? 'ok' : 'n/a', subjects, detail, judged };
}

// Pixel-only read for a capture with no audit rows (an exe from before the audit): the largest
// connected blob of dark pixels with dark on all four sides, HUD aside. A sword and shield drawn
// with no material are one such blob; a night frame (most of the picture dark) is not judged.
export function pixelSlabLaw(img) {
  const { width: w, height: h } = img;
  const dark = new Uint8Array(w * h);
  const L = (x, y) => lumaAt(img, x, y);
  const hud = HUD.map(([x0, y0, x1, y1]) => [x0 * w - 4, y0 * h - 4, x1 * w + 4, y1 * h + 4]);
  const t = T.pixelSlabLuma;
  let total = 0;
  for (let y = 1; y < h - 1; y += 1) {
    for (let x = 1; x < w - 1; x += 1) {
      if (hud.some(([a, b, c, d]) => x >= a && x <= c && y >= b && y <= d)) continue;
      if (L(x, y) < t && L(x - 1, y) < t && L(x + 1, y) < t && L(x, y - 1) < t && L(x, y + 1) < t) {
        dark[y * w + x] = 1;
        total += 1;
      }
    }
  }
  if (total > T.pixelSlabMaxShare * w * h) {
    return { status: 'n/a', subjects: [], detail: ['picture is mostly dark (night frame): not judged'], judged: 0 };
  }
  let best = { px: 0, x: 0, y: 0 };
  const seen = new Uint8Array(w * h);
  const stack = [];
  for (let y = 1; y < h - 1; y += 1) {
    for (let x = 1; x < w - 1; x += 1) {
      const start = y * w + x;
      if (!dark[start] || seen[start]) continue;
      seen[start] = 1;
      stack.push(start);
      let px = 0;
      let sx = 0;
      let sy = 0;
      while (stack.length > 0) {
        const p = stack.pop();
        px += 1;
        sx += p % w;
        sy += Math.floor(p / w);
        for (const q of [p + 1, p - 1, p + w, p - w]) {
          if (dark[q] && !seen[q]) {
            seen[q] = 1;
            stack.push(q);
          }
        }
      }
      if (px > best.px) best = { px, x: Math.round(sx / px), y: Math.round(sy / px) };
    }
  }
  if (best.px >= T.pixelSlabMinPx) {
    const at = best.x + ',' + best.y;
    return {
      status: 'FAIL',
      subjects: ['black_slab@' + at],
      detail: [best.px + ' px in one blob around (' + at + ') are near-black with dark on every side: a part drawn with no working material'],
      judged: 1,
    };
  }
  return { status: 'ok', subjects: [], detail: [], judged: 1 };
}

export const LAWS = ['shadow', 'material', 'scale', 'attach'];

// One frame: the four laws over one audit row and its decoded PNG.
export function auditFrame(row, img) {
  const project = makeProjector(row.camera, img.width, img.height);
  const result = {
    file: row.file,
    tick: row.tick,
    shadow: shadowLaw(row, img, project),
    material: materialLaw(row, img, project),
    scale: scaleLaw(row),
    attach: attachLaw(row),
  };
  // The row's own projection of each body root is the self-check of the tool's camera maths.
  const off = [];
  for (const b of row.bodies || []) {
    const p = project(b.root);
    if (p && Array.isArray(b.root_ndc) && (Math.abs(p.ndc[0] - b.root_ndc[0]) > 0.02 || Math.abs(p.ndc[1] - b.root_ndc[1]) > 0.02)) {
      off.push(`slot${b.slot}`);
    }
  }
  result.camera = off.length > 0 ? { status: 'FAIL', subjects: off, detail: [`camera: the tool and the window project ${off.join(',')} to different pixels`], judged: 1 } : { status: 'ok', subjects: [], detail: [], judged: 1 };
  result.ok = LAWS.every((l) => result[l].status !== 'FAIL') && result.camera.status !== 'FAIL';
  return result;
}

const field = (r) => (r.status === 'FAIL' ? `FAIL(${r.subjects.join(',')})` : r.status);

// `frame 2: shadow=FAIL(slot0,slot1) material=FAIL(gladius_blade@0) scale=ok attach=FAIL(spear@1)`
export function formatFrame(index, r) {
  const cam = r.camera.status === 'FAIL' ? ` camera=${field(r.camera)}` : '';
  return `frame ${index}: ${LAWS.map((l) => `${l}=${field(r[l])}`).join(' ')}${cam}`;
}
