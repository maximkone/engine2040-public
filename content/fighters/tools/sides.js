const fs = require('fs');
const dir = 'C:/engine2040/.lanes/ct7/content/fighters/sidecars';
fs.mkdirSync(dir, { recursive: true });
const F = { left: 'foot_l', right: 'foot_r' };
const defs = [
  ['idle', true, false, ['idle']],
  ['run', true, true, ['run']],
  ['jump', false, true, ['jump']],
  ['death', false, false, ['death']],
  ['idle_combat_1h', true, false, ['idle', 'combat']],
  ['idle_combat_unarmed', true, false, ['idle', 'combat']],
  ['attack_1h_1', false, false, ['attack', 'melee']],
  ['attack_1h_2', false, false, ['attack', 'melee']],
  ['attack_1h_3', false, false, ['attack', 'melee']],
  ['attack_2h_1', false, false, ['attack', 'melee']],
  ['attack_2h_2', false, false, ['attack', 'melee']],
  ['block_shield', false, false, ['block', 'shield']],
  ['attack_dual_1', false, false, ['attack', 'melee']],
  ['taunt', false, false, ['taunt']],
];
for (const [name, loop, root, tags] of defs) {
  const doc = { clip: name, loop, root_motion: root, tags, ground_y: 0.02, feet: F };
  fs.writeFileSync(`${dir}/${name}.json`, JSON.stringify(doc, null, 2) + '\n');
  console.log('wrote', name);
}
