const fs = require('fs');
const { spawnSync } = require('child_process');
const path = require('path');
const { BLENDER, TMP } = require('./paths');
const LANE = 'C:/engine2040/.lanes/ct7/content/fighters';
const SRC = LANE + '/incoming/modular/modular_rpg_characters';
const FEMALE = SRC + '/human_female.blend';
const MALE = SRC + '/human_male.blend';
const TEXDIR = SRC + '/textures';
const STAGE = LANE + '/incoming/staged';
fs.mkdirSync(STAGE, { recursive: true });
let failed = 0;
for (const v of ['hero', 'opp_male_light', 'opp_female_medium', 'opp_female_light']) {
  const r = spawnSync(BLENDER, ['--background', MALE, '--python',
    path.join(TMP, 'assemble.py'), '--',
    '--variant', v, '--female-src', FEMALE, '--texdir', TEXDIR,
    '--staged', `${STAGE}/${v}.blend`, '--manifest', `${STAGE}/${v}.parts.json`],
    { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  const tail = (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT7-ASSEMBLED|Error|Traceback|missing|has non-hero|RuntimeError/i.test(l));
  console.log('=== ' + v + ' exit=' + r.status);
  console.log(tail.slice(-8).join('\n'));
  if (r.status !== 0) failed = 1;
}
process.exit(failed);
