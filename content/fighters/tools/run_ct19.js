// CT19 cook driver: assemble -> graft weapons -> atlas -> export -> previews.
// All paths stay inside .lanes/ct19. Cooker code (crates/assets, ct7proof)
// is never touched: this only feeds the frozen CT1/CT2/texture/bundle path.
const { spawnSync } = require('child_process');
const fs = require('fs');
const { BLENDER } = require('./paths');
const LANE = 'C:/engine2040/.lanes/ct19/content/fighters';
const SRC = LANE + '/incoming/modular/modular_rpg_characters';
const IKSRC = LANE + '/incoming/ikrig/modular_rpg_characters_IK_rig_for_animating';
const MALE = SRC + '/human_male.blend';
const FEMALE = SRC + '/human_female.blend';
const TEXDIR = SRC + '/textures';
const STAGE = LANE + '/incoming/staged';
const VARIANTS = [
  { v: 'hero', female: false, sword: 'main', shield: '1' },
  { v: 'opp_male_light', female: false, sword: 'main', shield: '0' },
  { v: 'opp_female_medium', female: true, sword: 'two', shield: '0' },
  { v: 'opp_female_light', female: true, sword: 'main', shield: '1' },
];
const only = process.argv[2]; // optional single variant or 'atlas'/'previews'
function run(blend, script, extra) {
  const args = ['--background', blend, '--python', script, '--', ...extra];
  const r = spawnSync(BLENDER, args, { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  return r;
}
function tail(r) {
  return (r.stdout + '\n' + r.stderr).split('\n').filter(l => /CT19-|CT7-ASSEMBLED|CT7-EXPORTED|CT7-PREVIEW|Error|Traceback|RuntimeError/i.test(l)).slice(-8).join('\n');
}
let failed = 0;
fs.mkdirSync(STAGE, { recursive: true });
const doAtlas = !only || only === 'atlas';
const doPrev = !only || only === 'previews';
for (const c of VARIANTS) {
  if (only && only !== c.v && only !== 'atlas' && only !== 'previews') continue;
  if (!only || only === c.v) {
    let r = run(MALE, `${LANE}/tools/assemble.py`, ['--variant', c.v,
      '--female-src', c.female ? FEMALE : MALE, '--texdir', TEXDIR,
      '--staged', `${STAGE}/${c.v}.blend`, '--manifest', `${STAGE}/${c.v}.parts.json`]);
    console.log(`=== assemble ${c.v} exit=${r.status}\n` + tail(r));
    if (r.status !== 0) { failed = 1; continue; }
    // All variants stage on the male bind (female parts bound to the male
    // armature), so weapons always come from the male IK file: its
    // bone-relative offsets match the staged wep_pos bones exactly.
    const ik = IKSRC + '/animations_bake_human_male.blend';
    r = run(`${STAGE}/${c.v}.blend`, `${LANE}/tools/weapons.py`, ['--variant', c.v,
      '--ikblend', ik, '--sword', c.sword, '--shield', c.shield,
      '--report', `${STAGE}/${c.v}.weapon.json`]);
    console.log(`=== weapons ${c.v} exit=${r.status}\n` + tail(r));
    if (r.status !== 0) { failed = 1; continue; }
  }
}
if (doAtlas) {
  const r = spawnSync('node', [`${LANE}/tools/atlas.js`], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  console.log(`=== atlas exit=${r.status}\n` + (r.stdout + '\n' + r.stderr).split('\n').slice(-12).join('\n'));
  if (r.status !== 0) failed = 1;
}
for (const c of VARIANTS) {
  if (only && only !== c.v && only !== 'atlas' && only !== 'previews') continue;
  if (!only || only === c.v) {
    const r = run(`${STAGE}/${c.v}.blend`, `${LANE}/tools/export.py`, ['--variant', c.v,
      '--atlas', `${LANE}/atlas/${c.v}_atlas.png`, '--bodyglb', `${LANE}/glb/${c.v}_body.glb`]);
    console.log(`=== export ${c.v} exit=${r.status}\n` + tail(r));
    if (r.status !== 0) { failed = 1; continue; }
  }
  if (doPrev) {
    let r = run(`${STAGE}/${c.v}.blend`, `${LANE}/tools/preview_attack.py`,
      ['--variant', c.v, '--atlasdir', `${LANE}/atlas`, '--outdir', `${LANE}/previews`]);
    console.log(`=== attack ${c.v} exit=${r.status}\n` + tail(r));
    if (r.status !== 0) failed = 1;
    r = spawnSync(BLENDER, ['--background', '--python', `${LANE}/tools/preview_rest.py`, '--',
      '--variant', c.v, '--bodyglb', `${LANE}/glb/${c.v}_body.glb`,
      '--atlasdir', `${LANE}/atlas`, '--outdir', `${LANE}/previews`],
      { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
    console.log(`=== rest ${c.v} exit=${r.status}\n` + tail(r));
    if (r.status !== 0) failed = 1;
  }
}
process.exit(failed);
