const fs = require('fs');
const dir = 'C:/engine2040/.lanes/ct7/content/fighters/sidecars';
// Estimated from the 0.417 s (10 f @ 24 fps) attack clips: wind-up,
// active, recover. Authored estimates, not mocap measurements.
const wins = {
  attack_1h_1: [[0.10, 0.25]],
  attack_1h_2: [[0.10, 0.28]],
  attack_1h_3: [[0.12, 0.30]],
  attack_2h_1: [[0.12, 0.32]],
  attack_2h_2: [[0.15, 0.33]],
  attack_dual_1: [[0.08, 0.22], [0.24, 0.34]],
};
for (const [name, ws] of Object.entries(wins)) {
  const p = `${dir}/${name}.json`;
  const doc = JSON.parse(fs.readFileSync(p, 'utf8'));
  doc.hit_windows = ws.map(([start_s, end_s]) => ({ socket: 'SOCKET_wep_R', start_s, end_s }));
  fs.writeFileSync(p, JSON.stringify(doc, null, 2) + '\n');
  console.log('wrote', name, JSON.stringify(doc.hit_windows));
}
