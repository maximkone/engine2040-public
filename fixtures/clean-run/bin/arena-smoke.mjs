// Dry-run stand-in for bin/arena.exe: prints the WIN1 smoke marker and
// exits 0. Full A8 proof always runs the real exe; this file only lets the
// harness prove its copy, strip, assert and log steps without one.
const args = process.argv.slice(2);
let frames = 5;
const at = args.indexOf("--frames");
if (at !== -1 && args[at + 1] !== undefined) {
  const parsed = Number(args[at + 1]);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    console.error(`arena-smoke: bad --frames ${args[at + 1]}`);
    process.exit(2);
  }
  frames = parsed;
}
console.log(
  `arena smoke: ticks=${frames} frames=${frames} alpha=0.500 hud_cells=4 hud_quads=4 audio="device silent" presenter="dry-run twin"`,
);
process.exit(0);
