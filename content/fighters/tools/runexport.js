const fs = require('fs');
const { spawnSync } = require('child_process');
const { BLENDER, TMP } = require('./paths');
const LANE = 'C:/engine2040/.lanes/ct7/content/fighters';
const SRC = LANE + '/incoming/modular/modular_rpg_characters';
fs.mkdirSync(LANE + '/glb', { recursive: true });
function run(blend, script, extra) {
  const r = spawnSync(BLENDER, ['--background', blend, '--python', script, '--', ...extra],
    { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  return r;
}
let failed = 0;
// Bodies.
for (const v of ['hero', 'opp_male_light', 'opp_female_medium', 'opp_female_light']) {
  const r = run(`${LANE}/incoming/staged/${v}.blend`, `${TMP}/export.py`,
    ['--variant', v, '--atlas', `${LANE}/atlas/${v}_atlas.png`,
     '--bodyglb', `${LANE}/glb/${v}_body.glb`]);
  const tail = (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT7-EXPORTED|Error|Traceback/i.test(l));
  console.log('=== body ' + v + ' exit=' + r.status + '\n' + tail.slice(-6).join('\n'));
  if (r.status !== 0) failed = 1;
}
// Clips (one Blender run per action: the exporter embeds every action
// present, so isolate to get exactly one animation per GLB).
const CLIP_NAMES = ['human_male_idle', 'human_male_run', 'human_male_jump', 'human_male_death',
  'human_male_idle_combat_main_hand_melee', 'human_male_idle_combat_unarmed',
  'human_male_cast_main_hand_melee', 'human_male_cast_main_hand_melee_2', 'human_male_cast_main_hand_melee_3',
  'human_male_cast_two_handed_melee', 'human_male_cast_two_handed_melee_2',
  'human_male_cast_off_hand_shield', 'human_male_cast_dual_wield_melee', 'human_male_power_up'];
for (const c of CLIP_NAMES) {
  const r = run(`${SRC}/human_male.blend`, `${TMP}/clips.py`,
    ['--outdir', `${LANE}/glb/clips`, '--only', c]);
  const tail = (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT7-CLIP|Error|Traceback|missing/i.test(l));
  console.log('=== clip ' + c + ' exit=' + r.status + '\n' + tail.slice(-3).join('\n'));
  if (r.status !== 0) failed = 1;
}
process.exit(failed);
