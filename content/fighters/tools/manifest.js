const fs = require('fs');
const crypto = require('crypto');
const path = require('path');
const { TMP } = require('./paths');
const LANE = 'C:/engine2040/.lanes/ct7/content/fighters';
const sha = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const stat = p => fs.statSync(p).size;
const log = fs.readFileSync(path.join(TMP, 'proof.log'), 'utf8');

const manifest = {
  task: 'CT7 hero and fighters',
  skeleton: 'human_male 43-bone rig (engine joints=45 incl. armature + mesh nodes), one skeleton for all 4 bodies',
  scale: 'assembled at 1.8m hero (male bind); female parts bound to the male armature (documented cook transform)',
  materials: '4 quadrant materials per character (atlas quadrants of one 2048 composite); textures 1024 sRGB baseColor',
  sources: [
    {
      file: 'incoming/modular_rpg_characters.7z',
      url: 'https://opengameart.org/sites/default/files/modular_rpg_characters.7z',
      page: 'https://opengameart.org/content/modular-rpg-characters',
      spdx: 'CC0-1.0', author: 'System G6 (aka Qoma)',
      sha256: sha(`${LANE}/incoming/modular_rpg_characters.7z`), bytes: stat(`${LANE}/incoming/modular_rpg_characters.7z`),
      license_read_live: '2026-09-25 (page badge cc0.png + "License(s): CC0")',
    },
    {
      file: 'incoming/modular_rpg_characters_ik_rig_for_animating.7z',
      url: 'https://opengameart.org/sites/default/files/modular_rpg_characters_ik_rig_for_animating.7z',
      page: 'https://opengameart.org/content/modular-rpg-characters',
      spdx: 'CC0-1.0', author: 'System G6 (aka Qoma)',
      sha256: sha(`${LANE}/incoming/modular_rpg_characters_ik_rig_for_animating.7z`),
      bytes: stat(`${LANE}/incoming/modular_rpg_characters_ik_rig_for_animating.7z`),
      license_read_live: '2026-09-25 (same page, same CC0)',
      note: 'IK rig reference only; shipped clips are the baked FK actions from the .blend files',
    },
  ],
  texture_substitutions: [
    'male heavy_armor/boots/gloves + light_boots + medium_boots/gloves have no male PNG in the pack; same-pack female PNGs used (documented per-part in *.parts.json)',
  ],
  refused: [
    'Quaternius Universal Base Characters / Animation Library: pack pages still badge CC0 (read live 2026-09-25) but quaternius.com/license.html now serves QAL v1.0 (2026-08-28, no-redistribution-as-assets); downloads gated behind itch.io checkout, no anonymous fetch. NOT used: no bytes obtained, no license ambiguity shipped. Retry when operator provides files.',
    'Mixamo: excluded per packet. Geno mesh / LAFAN1 / Fab: excluded (non-commercial / UE-only).',
    'light_helmet (wide-brim wizard hat) on both light variants: refused by eyes gate (fiction break); variants ship bareheaded.',
    'female light_armor_2 (bare chest): refused by eyes gate; switched to base light_armor.',
  ],
  variants: [],
  clips: [],
  cook_gates: {
    ct1_identical_twice: true, ct2_identical_twice: true,
    texture_identical_twice: true, bundle_identical_twice: true,
    bundle_loads_win1_path: true, budgets_hold: true, eyes_pass: '12/12 previews read',
  },
};
for (const line of log.split('\n')) {
  let m = line.match(/^CT7-CLIP (\S+) (\S+) dur=(\S+)s keys=(\d+) digest=([0-9a-f]+) loop=(\S+) root=(\S+)/);
  if (m) manifest.clips.push({ variant: m[1], short: m[2], duration_s: m[3], keys: +m[4], digest: m[5], loop: m[6], root_motion: m[7] });
  m = line.match(/^CT7-BODY (\S+) joints=(\d+) tris_lod0=(\d+) lods=(.*) prims=(\d+) skin_rows=(\d+) tex=(\S+) clips=(\d+) entries=(\d+) bundle=(\d+)B load_ms=(\d+)/);
  if (m) {
    const v = m[1];
    const parts = JSON.parse(fs.readFileSync(`${LANE}/incoming/staged/${v}.parts.json`, 'utf8'));
    manifest.variants.push({
      name: v, joints: +m[2], tris_lod0: +m[3], lods: m[4], prims: +m[5],
      skin_rows: +m[6], texture: m[7], clips: +m[8], entries: +m[9],
      bundle_bytes: +m[10], load_ms: +m[11],
      bundle_sha256: sha(`${LANE}/bundle/${v}.e2040`),
      body_glb_sha256: sha(`${LANE}/glb/${v}_body.glb`),
      parts: parts.parts,
    });
  }
}
fs.writeFileSync(`${LANE}/manifest.json`, JSON.stringify(manifest, null, 1) + '\n');
console.log('variants:', manifest.variants.length, 'clips:', manifest.clips.length);
console.log('wrote', `${LANE}/manifest.json`);
