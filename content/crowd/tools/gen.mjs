// CT25 crowd kit generator — splitter piece 1 (single spectator impostor).
// Deterministic, dependency-free (node stdlib + content/kit-common.mjs).
// Original authored geometry, CC0-1.0, engine2040 CT25 lane.
// Units: meters, +Y up, rests on y=0, ~1.75 m tall. UVs in 0..1.
// One mesh per solid part (CT8 lesson: merged disjoint boxes degenerate to
// empty LODs in the greedy CT2 pass). Prop budget: LOD0 <= 5k tris.
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { emitGltf, newAcc, box, pushInto, trackInto } from '../../kit-common.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SRC = join(ROOT, 'src');
mkdirSync(SRC, { recursive: true });

// Spectator: torso box + head box, feet at y=0.
// Variant 0 is the piece-1 specimen (frozen bytes); variant 1 is the piece-3
// sibling (wider torso, different tunic color, same subdivision + budget).
function buildSpectator(variant = 0) {
  if (variant === 1) {
    const torso = newAcc();
    box(torso, 0, 0.92, 0, 0.6, 0.84, 0.34, 4);
    const head = newAcc();
    box(head, 0, 1.52, 0, 0.24, 0.28, 0.24, 4);
    return {
      meshes: [
        { name: 'Body_torso', acc: torso },
        { name: 'Body_head', acc: head },
      ],
      material: { base: [0.36, 0.3, 0.52, 1], metallic: 0.0, roughness: 0.9 },
    };
  }
  const torso = newAcc();
  box(torso, 0, 0.95, 0, 0.52, 0.9, 0.3, 4);
  const head = newAcc();
  box(head, 0, 1.58, 0, 0.26, 0.3, 0.26, 4);
  return {
    meshes: [
      { name: 'Body_torso', acc: torso },
      { name: 'Body_head', acc: head },
    ],
    material: { base: [0.5, 0.42, 0.34, 1], metallic: 0.0, roughness: 0.9 },
  };
}

// glTF 2.0 writer: shared emitGltf in kit-common (generator tag passed per kit).
// Emitted bytes are unchanged from the removed local copy.

const manifest = { kit: 'ct25-crowd-kit-split1', generator: 'engine2040-ct25-gen', license: 'CC0-1.0', files: {} };
const built = buildSpectator(0);
const buf = emitGltf("crowd_impostor_01", built, "engine2040-ct25-gen (CC0-1.0 original)");
writeFileSync(join(SRC, 'crowd_impostor_01.gltf'), buf);
trackInto(manifest, 'src/crowd_impostor_01.gltf', buf);
const built2 = buildSpectator(1);
const buf2 = emitGltf("crowd_impostor_02", built2, "engine2040-ct25-gen (CC0-1.0 original)");
writeFileSync(join(SRC, 'crowd_impostor_02.gltf'), buf2);
trackInto(manifest, 'src/crowd_impostor_02.gltf', buf2);
const specOf = (name, b) => ({
  name,
  meshes: b.meshes.map((m) => ({ name: m.name, verts: m.acc.pos.length, tris: m.acc.idx.length / 3 })),
});
manifest.specimens = [specOf('crowd_impostor_01', built), specOf('crowd_impostor_02', built2)];
writeFileSync(join(ROOT, 'manifest.json'), Buffer.from(JSON.stringify(manifest, null, 2) + '\n'));
const sha = createHash('sha256').update(buf).digest('hex');
console.log(`crowd_impostor_01.gltf bytes=${buf.length} sha256=${sha}`);
for (const m of manifest.specimens[0].meshes) console.log(`mesh ${m.name} verts=${m.verts} tris=${m.tris}`);
const sha2 = createHash('sha256').update(buf2).digest('hex');
console.log(`crowd_impostor_02.gltf bytes=${buf2.length} sha256=${sha2}`);
for (const m of manifest.specimens[1].meshes) console.log(`mesh ${m.name} verts=${m.verts} tris=${m.tris}`);
