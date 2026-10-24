// Run: node --test tools/frame-audit-laws.test.mjs
// The pure parts of the frame audit laws: the camera projection, the stature and upright reads,
// the equipment split. The laws themselves go red and green in frame-audit.test.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { isEquipment, isUpright, luma, makeProjector, stature } from './frame-audit-laws.mjs';

const CAMERA = { eye: [0, 1.5, 6], target: [0, 1, 0], up: [0, 1, 0], fov_y: 1, aspect: 1, near: 0.1 };

test('the projector puts the look-at target at the picture centre and refuses a point behind the eye', () => {
  const project = makeProjector(CAMERA, 200, 100);
  const centre = project(CAMERA.target);
  assert.ok(Math.abs(centre.x - 100) < 1e-6 && Math.abs(centre.y - 50) < 1e-6, JSON.stringify(centre));
  assert.equal(project([0, 1.5, 9]), null);
  const right = project([1, 1, 0]);
  assert.ok(right.x > 100, 'world +x is picture right');
  const high = project([0, 2, 0]);
  assert.ok(high.y < 50, 'world +y is picture up');
});

test('stature is the longest extent of the body parts, equipment excluded; a lying body is not upright', () => {
  const body = (parts, over = {}) => ({ slot: 0, act: 'idle', ragdolled: false, min: [0, 0, 0], max: [1, 1, 1], parts, ...over });
  const standing = body([
    { name: 'sm002_body', min: [-0.2, 0, -0.1], max: [0.2, 1.8, 0.1] },
    { name: 'gladius_blade', min: [0.3, 1, 0], max: [0.35, 4, 0.05] },
  ]);
  assert.equal(stature(standing), 1.8);
  assert.equal(isUpright(standing), true);
  const lying = body([{ name: 'sm002_body', min: [-0.9, 0, -0.3], max: [0.9, 0.3, 0.3] }]);
  assert.equal(stature(lying), 1.8);
  assert.equal(isUpright(lying), false);
  assert.equal(isUpright({ ...standing, act: 'down' }), false);
  assert.equal(isUpright({ ...standing, ragdolled: true }), false);
});

test('equipment is named by material and luma reads sRGB bytes with Rec.709 weights', () => {
  for (const name of ['gladius_blade', 'scutum_boss', 'spear_head', 'sm002_combat_knife']) assert.equal(isEquipment(name), true, name);
  for (const name of ['sm002_body', 'sm002_head', 'sm002_helmet', '']) assert.equal(isEquipment(name), false, name);
  assert.ok(Math.abs(luma(255, 255, 255) - 1) < 1e-9);
  assert.equal(luma(0, 0, 0), 0);
});
