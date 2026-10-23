#!/usr/bin/env node
// Task 6 referee: the receipt must carry the world-space slip, and the referee
// recomputes it from the pack instead of trusting the number.
//
// The fixture is one planted foot over eight ticks: the local offset walks
// 0.109350 m/s inside the leg while the hips rotation cancels it, so the foot
// stands still on the sand at 0.024263 m/s. The start tree reports the local
// meter (red), the mutation a made-up 0.02 under the bar (red).
//
//   node tools/tasks/06-slip-meter/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/06-slip-meter/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const SLIP_MAX = 0.08;
const WORLD = 0.024263;
const LOCAL = 0.10935;

const PACK = `{
  "dt": 0.016666666666666666,
  "joints": ["hips", "leg", "foot"],
  "parent": [-1, 0, 1],
  "rot": [
    [[0, 0, 0, 1], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.001070417472948083, 0.9999994271030527], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.002140833719418361, 0.9999977084128672], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.003211247512934433, 0.9999948439314128], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.004281657627022712, 0.9999908336619716], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.005352062835213822, 0.9999856776091385], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.006422461911044012, 0.9999793757788213], [0, 0, 0, 1], [0, 0, 0, 1]],
    [[0, 0, 0.007492853628056556, 0.9999719281782407], [0, 0, 0, 1], [0, 0, 0, 1]]
  ],
  "offset": [
    [[0, 0, 0], [0, -1, 0], [0.02, -0.04, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.018177808, -0.040033445, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.01635556, -0.040063698, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.014533262, -0.040090758, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.01271092, -0.040114625, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.010888538, -0.040135299, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.009066123, -0.04015278, 0.01]],
    [[0, 0, 0], [0, -1, 0], [0.00724368, -0.040167068, 0.01]]
  ]
}`;

function receipt(avgSlip) {
  return `${JSON.stringify({ frames: 8, avg_slip: avgSlip, bar: SLIP_MAX }, null, 2)}\n`;
}

const start = { 'pack.json': PACK, 'receipt.json': receipt(LOCAL) };
const fixed = { 'pack.json': PACK, 'receipt.json': receipt(WORLD) };
const mutation = { 'pack.json': PACK, 'receipt.json': receipt(0.02) };

/** Rotate a vector by a unit quaternion [x, y, z, w]. */
function rotate([x, y, z, w], [vx, vy, vz]) {
  const tx = 2 * (y * vz - z * vy);
  const ty = 2 * (z * vx - x * vz);
  const tz = 2 * (x * vy - y * vx);
  return [vx + w * tx + (y * tz - z * ty), vy + w * ty + (z * tx - x * tz), vz + w * tz + (x * ty - y * tx)];
}

function quatMul([ax, ay, az, aw], [bx, by, bz, bw]) {
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bz + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

/** World position of every joint, per tick, from the parent chain. */
export function worldChain(pack) {
  const positions = [];
  for (let tick = 0; tick < pack.rot.length; tick += 1) {
    const worldPos = [];
    const worldRot = [];
    for (let joint = 0; joint < pack.joints.length; joint += 1) {
      const parent = pack.parent[joint];
      const [parentPos, parentRot] = parent < 0 ? [[0, 0, 0], [0, 0, 0, 1]] : [worldPos[parent], worldRot[parent]];
      const offset = rotate(parentRot, pack.offset[tick][joint]);
      worldPos[joint] = [parentPos[0] + offset[0], parentPos[1] + offset[1], parentPos[2] + offset[2]];
      worldRot[joint] = quatMul(parentRot, pack.rot[tick][joint]);
    }
    positions.push(worldPos);
  }
  return positions;
}

/** The foot (last joint) averaged world speed, m/s. */
export function worldSlip(pack, positions = worldChain(pack)) {
  let sum = 0;
  let count = 0;
  for (let tick = 1; tick < positions.length; tick += 1) {
    const [a, b] = [positions[tick].at(-1), positions[tick - 1].at(-1)];
    sum += Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]) / pack.dt;
    count += 1;
  }
  return sum / count;
}

/** The foot's averaged local offset speed, m/s: the wrong meter, kept to name it. */
export function localSlip(pack) {
  let sum = 0;
  let count = 0;
  for (let tick = 1; tick < pack.offset.length; tick += 1) {
    const [a, b] = [pack.offset[tick].at(-1), pack.offset[tick - 1].at(-1)];
    sum += Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]) / pack.dt;
    count += 1;
  }
  return sum / count;
}

export const task = defineTask({
  id: '06-slip-meter',
  title: 'measure a planted foot in the right frame',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const pack = json(files, 'pack.json');
    const out = json(files, 'receipt.json');
    const ticks = pack.rot?.length ?? 0;
    if (ticks < 2) failures.push('pack.json has fewer than two ticks');
    if (pack.rot?.length !== pack.offset?.length) failures.push('pack.json rot and offset disagree on ticks');
    if (!(pack.dt > 0)) failures.push('pack.json has no positive dt');
    if (failures.length > 0) return failures;
    const world = worldSlip(pack);
    const local = localSlip(pack);
    if (!Number.isFinite(world)) return ['the world meter did not produce a number'];
    if (world >= SLIP_MAX) failures.push(`the planted foot moves ${world.toFixed(6)} m/s in world space, the bar is ${SLIP_MAX}`);
    const claimed = out.avg_slip;
    if (!Number.isFinite(claimed)) failures.push(`receipt.avg_slip is not a number: ${JSON.stringify(claimed)}`);
    else if (Math.abs(claimed - world) > 1e-5) {
      const localMeter = Math.abs(claimed - local) <= 1e-5;
      failures.push(
        `receipt.avg_slip is ${claimed}, the world meter is ${world.toFixed(6)}` +
          (localMeter ? `: that is the local meter ${local.toFixed(6)}; the pack is local, so slip is measured in world space` : ''),
      );
    }
    return failures;
  },
});

cli(task);
