// CT7 cook fix: strip "scale" animation channels from Blender-exported
// clip GLBs (engine A3 refuses scale targets; the exports are uniform
// scale 1 baked per frame, so removal is semantics-preserving).
// Unreferenced samplers/accessors stay in place (legal glTF); only the
// animation channel arrays shrink. Deterministic: same input, same bytes.
const fs = require('fs');
const dir = 'C:/engine2040/.lanes/ct7/content/fighters/glb/clips';
for (const f of fs.readdirSync(dir).filter(x => x.endsWith('.glb'))) {
  const path = dir + '/' + f;
  const buf = fs.readFileSync(path);
  if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error('bad magic ' + f);
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString('utf8'));
  let dropped = 0;
  for (const anim of json.animations || []) {
    const keep = anim.channels.filter(c => c.target.path !== 'scale');
    dropped += anim.channels.length - keep.length;
    anim.channels = keep;
  }
  const out = Buffer.from(JSON.stringify(json));
  const pad = (4 - (out.length % 4)) % 4;
  const binLen = buf.readUInt32LE(20 + jsonLen);
  const binOff = 20 + jsonLen + 8;
  const head = Buffer.alloc(12);
  head.writeUInt32LE(0x46546c67, 0); head.writeUInt32LE(2, 4);
  head.writeUInt32LE(12 + 8 + out.length + pad + 8 + binLen, 8);
  const jh = Buffer.alloc(8);
  jh.writeUInt32LE(out.length + pad, 0); jh.writeUInt32LE(0x4e4f534a, 4);
  const bh = Buffer.alloc(8);
  bh.writeUInt32LE(binLen, 0); bh.writeUInt32LE(0x004e4942, 4);
  fs.writeFileSync(path, Buffer.concat([head, jh, out, Buffer.alloc(pad, 0x20),
    bh, buf.subarray(binOff, binOff + binLen)]));
  console.log(f, 'dropped scale channels:', dropped);
}
