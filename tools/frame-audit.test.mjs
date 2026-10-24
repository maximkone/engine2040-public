// Run: node --test tools/frame-audit.test.mjs
// Each law of tools/frame-audit-laws.mjs goes red on a small synthetic scene and green on its
// fixed twin: a 200x200 picture rendered here (sand floor, shadow disc, painted body and parts)
// and an audit row written the way crates/game/src/view_truth/audit.rs writes it.
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { deflateSync } from 'node:zlib';
import { auditDir, main, render } from './frame-audit.mjs';
import { auditFrame, formatFrame, makeProjector, patchIsDarkFlat, shadowLaw } from './frame-audit-laws.mjs';

const W = 200;
const H = 200;
const CAMERA = { eye: [0, 2.2, 4.6], target: [0, 0.9, 0], up: [0, 1, 0], fov_y: 0.9, aspect: 1, near: 0.1 };

// A small deterministic hash noise in 0..1, so the sand has texture like the real one.
const noise = (x, y) => {
  let h = (x * 374761393 + y * 668265263) >>> 0;
  h = ((h ^ (h >>> 13)) * 1274126177) >>> 0;
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
};

// Render the floor (sand, optional shadow disc under the origin) and the sky.
// `shadow`: a disc of shade under the origin, or `{ dx, dz, factor }` a shadow cast that far away.
function floorImage({ shadow, ao = 1 }) {
  const data = Buffer.alloc(W * H * 4, 255);
  const aim = [0, CAMERA.target[1] - CAMERA.eye[1], CAMERA.target[2] - CAMERA.eye[2]];
  const f = aim.map((v) => v / Math.hypot(...aim));
  const r = [1, 0, 0];
  const u = [0, -f[2], f[1]];
  const half = Math.tan(CAMERA.fov_y / 2);
  for (let y = 0; y < H; y += 1) {
    for (let x = 0; x < W; x += 1) {
      const nx = ((x + 0.5) / W) * 2 - 1;
      const ny = 1 - ((y + 0.5) / H) * 2;
      const d = [f[0] + r[0] * nx * half + u[0] * ny * half, f[1] + r[1] * nx * half + u[1] * ny * half, f[2] + r[2] * nx * half + u[2] * ny * half];
      let l = 0.85;
      if (d[1] < -1e-4) {
        const t = -CAMERA.eye[1] / d[1];
        const gx = CAMERA.eye[0] + d[0] * t;
        const gz = CAMERA.eye[2] + d[2] * t;
        l = 0.62 + (noise(x, y) - 0.5) * 0.06;
        if (shadow === true && Math.hypot(gx, gz) < 0.9) l *= 0.4;
        if (shadow && shadow.dx !== undefined && Math.hypot(gx - shadow.dx, gz - shadow.dz) < 0.6) l *= shadow.factor ?? 0.5;
        if (ao < 1 && Math.hypot(gx, gz) < 0.9) l *= ao;
      }
      const o = (y * W + x) * 4;
      data[o] = data[o + 1] = data[o + 2] = Math.round(l * 255);
    }
  }
  return { width: W, height: H, data };
}

function paintRect(img, x0, y0, x1, y1, fn) {
  for (let y = Math.max(0, Math.floor(y0)); y < Math.min(H, Math.ceil(y1)); y += 1) {
    for (let x = Math.max(0, Math.floor(x0)); x < Math.min(W, Math.ceil(x1)); x += 1) {
      const l = fn(x, y);
      const o = (y * W + x) * 4;
      img.data[o] = img.data[o + 1] = img.data[o + 2] = Math.round(l * 255);
    }
  }
}

// Paint a world box as a bright body silhouette and return its screen box.
function paintBox(img, min, max, l = 0.92) {
  const p = makeProjector(CAMERA, W, H);
  const pts = [];
  for (let i = 0; i < 8; i += 1) pts.push(p([i & 1 ? max[0] : min[0], i & 2 ? max[1] : min[1], i & 4 ? max[2] : min[2]]));
  const xs = pts.map((q) => q.x);
  const ys = pts.map((q) => q.y);
  paintRect(img, Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys), () => l);
}

const samples = (min, max) => [0, 1, 2, 3, 4, 5, 6, 7].map((i) => [i & 1 ? max[0] : min[0], i & 2 ? max[1] : min[1], i & 4 ? max[2] : min[2]]);

function part(i, name, min, max, over = {}) {
  const dx = max[0] - min[0];
  const dy = max[1] - min[1];
  const dz = max[2] - min[2];
  return { i, name, min, max, len: Math.max(dx, dy, dz), tex: true, base: [1, 1, 1], metal: 0, rough: 0.5, pts: samples(min, max), ...over };
}

// A standing legionary: body, blade, grip, shield; hands where the kit puts them.
function legionary(slot = 0, over = {}) {
  const body = part(0, 'sm002_body', [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  const grip = part(5, 'gladius_grip', [0.25, 1.0, 0], [0.3, 1.1, 0.05]);
  const blade = part(6, 'gladius_blade', [0.25, 1.0, 0.05], [0.3, 1.0 + 0.55, 0.07], { len: 0.55 });
  const shield = part(7, 'scutum_board', [-0.5, 0.7, 0.1], [-0.3, 1.5, 0.2], { len: 0.9 });
  return {
    slot,
    act: 'idle',
    ragdolled: false,
    root: [0, 0, 0],
    root_ndc: makeProjector(CAMERA, W, H)([0, 0, 0]).ndc,
    min: [-0.5, 0, -0.15],
    max: [0.3, 1.8, 0.2],
    anchors: { palm_r: [0.27, 1.05, 0.03], wep_pos_r: [0.27, 1.05, 0.03], palm_l: [-0.4, 1.0, 0.15], wep_pos_l: [-0.4, 1.0, 0.15] },
    parts: [body, grip, blade, shield],
    ...over,
  };
}

const rowOf = (bodies, spears = []) => ({ v: 1, file: 'frame_000002.png', tick: 3115, camera: CAMERA, bodies, spears });

// A sane scene: shadow under the body, the body painted over it.
function goodScene(over = {}) {
  const img = floorImage({ shadow: true });
  paintBox(img, [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  return { img, row: rowOf([legionary(0, over)]) };
}

const run = (scene) => auditFrame(scene.row, scene.img);

test('a sane frame passes every law', () => {
  const result = run(goodScene());
  for (const law of ['shadow', 'material', 'scale', 'attach']) {
    assert.notEqual(result[law].status, 'FAIL', `${law}: ${result[law].detail.join('; ')}`);
  }
  assert.equal(result.shadow.status, 'ok', result.shadow.detail.join('; '));
  assert.equal(result.camera.status, 'ok');
  assert.match(formatFrame(2, result), /^frame 2: shadow=ok material=ok scale=ok attach=ok$/);
});

test('shadow: a body on bright floor with no darker footprint fails, the shadowed twin passes', () => {
  const img = floorImage({ shadow: false });
  paintBox(img, [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  const bad = shadowLaw(rowOf([legionary(0)]), img, makeProjector(CAMERA, W, H));
  assert.equal(bad.status, 'FAIL', bad.detail.join('; '));
  assert.deepEqual(bad.subjects, ['slot0']);
  assert.match(bad.detail[0], /no shadow/);
  assert.equal(run(goodScene()).shadow.status, 'ok');
});

// A legionary shifted along x, every world number with it.
function shifted(slot, dx) {
  const base = legionary(slot);
  const move = (v) => [v[0] + dx, v[1], v[2]];
  return {
    ...base,
    root: move(base.root),
    root_ndc: makeProjector(CAMERA, W, H)(move(base.root)).ndc,
    min: move(base.min),
    max: move(base.max),
    anchors: Object.fromEntries(Object.entries(base.anchors).map(([k, v]) => [k, move(v)])),
    parts: base.parts.map((p) => ({ ...p, min: move(p.min), max: move(p.max), pts: p.pts.map(move) })),
  };
}

test('shadow: two bodies both lacking a footprint are both named', () => {
  const img = floorImage({ shadow: false });
  for (const dx of [-1, 1]) paintBox(img, [-0.25 + dx, 0, -0.15], [0.25 + dx, 1.8, 0.15]);
  const result = auditFrame(rowOf([shifted(0, -1), shifted(1, 1)]), img);
  assert.deepEqual(result.shadow.subjects, ['slot0', 'slot1'], result.shadow.detail.join('; '));
  assert.match(formatFrame(2, result), /shadow=FAIL\(slot0,slot1\)/);
});

test('shadow: a long low-sun shadow cast three metres away still counts, a faint ambient blob does not', () => {
  const img = floorImage({ shadow: { dx: 1.6, dz: 0.2, factor: 0.45 } });
  paintBox(img, [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  // The cast shadow is a streak from the feet to its far end: a few discs joined.
  for (const dx of [0.3, 0.7, 1.1, 1.6]) {
    const p = makeProjector(CAMERA, W, H)([dx, 0, 0.1]);
    paintRect(img, p.x - 8, p.y - 4, p.x + 9, p.y + 6, () => 0.62 * 0.5);
  }
  const long = shadowLaw(rowOf([legionary(0)]), img, makeProjector(CAMERA, W, H));
  assert.equal(long.status, 'ok', long.detail.join('; '));
  const ao = floorImage({ shadow: false, ao: 0.82 });
  paintBox(ao, [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  const faint = shadowLaw(rowOf([legionary(0)]), ao, makeProjector(CAMERA, W, H));
  assert.equal(faint.status, 'FAIL', faint.detail.join('; '));
  assert.match(faint.detail[0], /0 dark core/);
});

test('shadow: a body with its feet out of the picture is not judged', () => {
  const img = floorImage({ shadow: false });
  const low = shifted(0, 0);
  low.parts = low.parts.map((p) => ({ ...p, min: [p.min[0], p.min[1], p.min[2] + 2.6], max: [p.max[0], p.max[1], p.max[2] + 2.6] }));
  const result = shadowLaw(rowOf([low]), img, makeProjector(CAMERA, W, H));
  assert.equal(result.status, 'n/a', result.detail.join('; '));
  assert.match(result.detail[0], /feet are out of the picture/);
});

test('shadow: a body too far to judge is not a failure', () => {
  const img = floorImage({ shadow: false });
  const tiny = legionary(0, { parts: [part(0, 'sm002_body', [-0.01, 0, -0.01], [0.01, 0.02, 0.01])], min: [-0.01, 0, -0.01], max: [0.01, 0.02, 0.01] });
  const result = shadowLaw(rowOf([tiny]), img, makeProjector(CAMERA, W, H));
  assert.equal(result.status, 'n/a');
});

test('material: a part that renders near-black and flat fails; the same part with texture passes', () => {
  const scene = goodScene();
  const blade = scene.row.bodies[0].parts[2];
  const p = makeProjector(CAMERA, W, H);
  const screen = blade.pts.map((q) => p(q));
  const paintBlade = (fn) => {
    for (const q of screen) paintRect(scene.img, q.x - 4, q.y - 4, q.x + 5, q.y + 5, fn);
  };
  paintBlade(() => 0.05);
  const bad = run(scene).material;
  assert.equal(bad.status, 'FAIL', bad.detail.join('; '));
  assert.deepEqual(bad.subjects, ['gladius_blade@0']);
  assert.match(bad.detail[0], /texture bound but renders flat/);
  paintBlade((x, y) => 0.18 + noise(x, y) * 0.7);
  assert.equal(run(scene).material.status, 'ok');
});

test('material: a dark patch with a bright speck is texture, not a missing material', () => {
  const dark = Array.from({ length: 25 }, () => 0.06);
  assert.equal(patchIsDarkFlat(dark), true);
  assert.equal(patchIsDarkFlat([...dark.slice(0, 20), 0.5, 0.5, 0.5, 0.5, 0.5].sort((a, b) => a - b)), false);
});

test('scale: a blade three times too long and a giant body fail; the sane kit passes', () => {
  const long = goodScene();
  long.row.bodies[0].parts[2].len = 1.7;
  const result = run(long);
  assert.equal(result.scale.status, 'FAIL');
  assert.deepEqual(result.scale.subjects, ['gladius_blade@0']);
  assert.match(result.scale.detail[0], /1\.7 m long/);
  const giant = goodScene();
  giant.row.bodies[0].parts[0] = part(0, 'sm002_body', [-1, 0, -1], [1, 5.4, 1]);
  assert.ok(run(giant).scale.subjects.includes('stature@0'));
  assert.equal(run(goodScene()).scale.status, 'ok');
});

test('attach: a sword away from the hand and a sword in pieces fail, a shield off the arm fails', () => {
  const far = goodScene();
  far.row.bodies[0].anchors.wep_pos_r = [0.27, 1.05, 0.9];
  const result = run(far).attach;
  assert.equal(result.status, 'FAIL');
  assert.deepEqual(result.subjects, ['gladius_blade@0']);
  assert.match(result.detail[0], /the sword is not in the hand/);
  const apart = goodScene();
  apart.row.bodies[0].parts[1] = part(5, 'gladius_grip', [3, 1, 0], [3.05, 1.1, 0.05]);
  assert.deepEqual(run(apart).attach.subjects, ['gladius_grip@0']);
  assert.match(run(apart).attach.detail[0], /the sword is in pieces/);
  const shield = goodScene();
  shield.row.bodies[0].anchors.palm_l = [2, 1, 2];
  shield.row.bodies[0].anchors.wep_pos_l = [2, 1, 2];
  assert.deepEqual(run(shield).attach.subjects, ['scutum_board@0']);
});

const spear = (over) => ({ slot: 1, kind: 'held', tip: [0.27, 2.95, 0.03], dir: [0, 1, 0], len: 1.9, holder: 1, victim: null, ...over });

test('attach: a spear standing up out of a lying body fails, in a live hand passes', () => {
  const lying = goodScene();
  lying.row.bodies = [
    legionary(1, {
      act: 'down',
      min: [-0.9, 0, -0.3],
      max: [0.9, 0.3, 0.3],
      anchors: { palm_r: [0.27, 0.2, 0.03] },
      parts: [part(0, 'sm002_body', [-0.9, 0, -0.3], [0.9, 0.3, 0.3])],
    }),
  ];
  lying.row.spears = [spear({ tip: [0.27, 2.1, 0.03] })];
  const bad = run(lying).attach;
  assert.equal(bad.status, 'FAIL', bad.detail.join('; '));
  assert.deepEqual(bad.subjects, ['spear@1']);
  assert.match(bad.detail[0], /held by slot 1 that is down/);
  assert.match(formatFrame(2, run(lying)), /attach=FAIL\(spear@1\)/);
  const live = goodScene();
  live.row.bodies[0].slot = 1;
  live.row.spears = [spear({ tip: [0.27, 2.95, 0.03] })];
  assert.equal(run(live).attach.status, 'ok', run(live).attach.detail.join('; '));
});

test('attach: a stuck spear must have its tip in the victim at a throwing angle', () => {
  const tipOut = goodScene();
  tipOut.row.spears = [spear({ kind: 'stuck', holder: null, victim: 0, tip: [4, 1, 0], dir: [-1, 0, 0] })];
  assert.match(run(tipOut).attach.detail[0], /outside slot 0's bounds/);
  const upright = goodScene();
  upright.row.spears = [spear({ kind: 'stuck', holder: null, victim: 0, tip: [0, 1.2, 0], dir: [0, 1, 0] })];
  assert.match(run(upright).attach.detail[0], /elevation/);
  const fine = goodScene();
  fine.row.spears = [spear({ kind: 'stuck', holder: null, victim: 0, tip: [0.2, 1.2, 0], dir: [-0.97, 0.2, 0] })];
  assert.equal(run(fine).attach.status, 'ok', run(fine).attach.detail.join('; '));
});

test('material: a black slab on a body fails even when no named part owns it, a lit body passes', () => {
  const scene = goodScene();
  const p = makeProjector(CAMERA, W, H);
  const a = p([-0.5, 0.7, 0.1]);
  const c = p([-0.3, 1.5, 0.2]);
  const mx = (a.x + c.x) / 2;
  paintRect(scene.img, mx - 14, Math.min(a.y, c.y) - 10, mx + 14, Math.max(a.y, c.y) + 10, () => 0.01);
  const result = run(scene).material;
  assert.equal(result.status, 'FAIL', result.detail.join('; '));
  assert.ok(result.subjects.includes('slot0:black_slab'), result.subjects.join(','));
  assert.match(result.detail.find((d) => d.startsWith('slot0')), /black with black on every side/);
  assert.equal(run(goodScene()).material.status, 'ok');
});

test('attach: a held spear must lie along the holder palm, not stand beside it', () => {
  const scene = goodScene();
  scene.row.bodies[0].slot = 1;
  scene.row.bodies[0].anchors.palm_r = [0.27, 1.05, 0.03];
  // Gripped at 0.6 m up the shaft: the tail is 0.6 m from the palm, the shaft passes through it.
  scene.row.spears = [spear({ tip: [0.27, 1.05 + 1.3, 0.03] })];
  assert.equal(run(scene).attach.status, 'ok', run(scene).attach.detail.join('; '));
  scene.row.spears = [spear({ tip: [0.9, 2.35, 0.03] })];
  assert.match(run(scene).attach.detail[0], /shaft .* m from the palm that holds it/);
});

test('camera: a window projection that disagrees with the tool is named', () => {
  const scene = goodScene();
  scene.row.bodies[0].root_ndc = [0.5, 0.5];
  const result = run(scene);
  assert.equal(result.camera.status, 'FAIL');
  assert.equal(result.ok, false);
});

// ---- the CLI over a capture dir ---------------------------------------------------------
const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}
function encodePng(img) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(img.width, 0);
  ihdr.writeUInt32BE(img.height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const raw = Buffer.alloc(img.height * (img.width * 4 + 1));
  for (let y = 0; y < img.height; y += 1) {
    img.data.copy(raw, y * (img.width * 4 + 1) + 1, y * img.width * 4, (y + 1) * img.width * 4);
  }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

function capture(scene, { audit = true } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'frame-audit-'));
  writeFileSync(join(dir, 'frame_000002.png'), encodePng(scene.img));
  if (audit) writeFileSync(join(dir, 'view-audit.jsonl'), `${JSON.stringify(scene.row)}\n`);
  return dir;
}

test('cli: a bad capture dir prints per-frame per-law lines and exits 1; a good one exits 0', () => {
  const bad = goodScene();
  bad.img = floorImage({ shadow: false });
  paintBox(bad.img, [-0.25, 0, -0.15], [0.25, 1.8, 0.15]);
  bad.row.bodies[0].anchors.wep_pos_r = [3, 1, 0];
  const dir = capture(bad);
  try {
    const result = auditDir(dir);
    const text = render(result);
    assert.match(text, /^frame 2: shadow=FAIL\(slot0\) material=\S+ scale=ok attach=FAIL\(gladius_blade@0\)/);
    assert.match(text, /audit=FAIL\(shadow,attach\) frames=1 failed_frames=1/);
    assert.match(text, /\n {2}attach gladius_blade@0: /);
    assert.equal(main(['node', 'frame-audit.mjs', dir, '--quiet']), 1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  const goodDir = capture(goodScene());
  try {
    assert.equal(main(['node', 'frame-audit.mjs', goodDir]), 0);
    assert.match(render(auditDir(goodDir)), /audit=ok frames=1 failed_frames=0/);
  } finally {
    rmSync(goodDir, { recursive: true, force: true });
  }
});

test('cli: a stray flag, no dir and a dir with no frames are usage errors', () => {
  const empty = mkdtempSync(join(tmpdir(), 'frame-audit-empty-'));
  const dir = capture(goodScene(), { audit: false });
  try {
    assert.equal(main(['node', 'frame-audit.mjs', dir, '--nope']), 2);
    assert.equal(main(['node', 'frame-audit.mjs']), 2);
    assert.equal(main(['node', 'frame-audit.mjs', empty]), 2);
    assert.throws(() => auditDir(empty), /no frame_\*\.png and no view-audit\.jsonl/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
    rmSync(empty, { recursive: true, force: true });
  }
});

test('pixel-only: a capture from an exe with no audit rows still fails a black slab and says what it did not judge', () => {
  const slab = goodScene();
  paintRect(slab.img, 60, 70, 110, 130, () => 0.03);
  const dir = capture(slab, { audit: false });
  const clean = capture(goodScene(), { audit: false });
  const night = floorImage({ shadow: false });
  paintRect(night, 0, 0, W, H, () => 0.02);
  const nightDir = capture({ img: night, row: slab.row }, { audit: false });
  try {
    const bad = auditDir(dir);
    assert.equal(bad.pixelOnly, true);
    const text = render(bad);
    assert.match(text, /^frame 2: shadow=n\/a material=FAIL\(black_slab@\d+,\d+\) scale=n\/a attach=n\/a/);
    assert.match(text, /audit=FAIL\(material\) frames=1 failed_frames=1 \(pixel-only/);
    assert.equal(main(['node', 'frame-audit.mjs', dir]), 1);
    assert.match(render(auditDir(clean)), /audit=ok\(material only\)/);
    assert.equal(main(['node', 'frame-audit.mjs', clean]), 0);
    assert.equal(auditDir(nightDir).frames[0].material.status, 'n/a', 'a night frame is not judged');
  } finally {
    for (const d of [dir, clean, nightDir]) rmSync(d, { recursive: true, force: true });
  }
});
