const fs = require('fs');
const { spawnSync } = require('child_process');
const { BLENDER, TMP } = require('./paths');
const LANE = 'C:/engine2040/.lanes/ct7/content/fighters';
fs.mkdirSync(LANE + '/previews', { recursive: true });
function run(blend, script, extra) {
  const args = blend ? ['--background', blend, '--python', script, '--', ...extra]
                     : ['--background', '--python', script, '--', ...extra];
  return spawnSync(BLENDER, args, { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
}
let failed = 0;
for (const v of ['hero', 'opp_male_light', 'opp_female_medium', 'opp_female_light']) {
  let r = run(`${LANE}/incoming/staged/${v}.blend`, `${TMP}/preview_attack.py`,
    ['--variant', v, '--atlasdir', `${LANE}/atlas`, '--outdir', `${LANE}/previews`]);
  let tail = (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT7-PREVIEW|Error|Traceback/i.test(l));
  console.log('=== attack ' + v + ' exit=' + r.status + '\n' + tail.slice(-4).join('\n'));
  if (r.status !== 0) failed = 1;
  r = run(null, `${TMP}/preview_rest.py`,
    ['--variant', v, '--bodyglb', `${LANE}/glb/${v}_body.glb`, '--outdir', `${LANE}/previews`]);
  tail = (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT7-PREVIEW|Error|Traceback/i.test(l));
  console.log('=== rest ' + v + ' exit=' + r.status + '\n' + tail.slice(-4).join('\n'));
  if (r.status !== 0) failed = 1;
}
process.exit(failed);
