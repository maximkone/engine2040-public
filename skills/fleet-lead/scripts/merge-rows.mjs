// node skills/fleet-lead/scripts/merge-rows.mjs <config.json> <rows dir> <shard.md>
// Append judged hunter row files as sections of a board shard.
// config: [{ "file": "rows-x.md", "title": "Section", "drop": ["SW9"], "needs": { "SW8": "needs text" } }]
// Refuses a block without an id or an id already on the shard. Run workspace-check, then commit by path.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [cfgPath, dir, shard] = process.argv.slice(2);
if (!cfgPath || !dir || !shard) {
  console.log("usage: merge-rows.mjs <config.json> <rows dir> <shard.md>");
  process.exit(2);
}
const cfg = JSON.parse(readFileSync(cfgPath, "utf8"));
let out = readFileSync(shard, "utf8").replace(/\r\n/g, "\n").trimEnd();
const have = new Set([...out.matchAll(/^- \[[ x]\] ([A-Z]+\d+[a-z]?) /gm)].map((m) => m[1]));
for (const { file, title, drop = [], needs = {} } of cfg) {
  const t = readFileSync(join(dir, file), "utf8").replace(/\r\n/g, "\n");
  const body = t.slice(t.indexOf("- [ ] ")).trimEnd();
  const blocks = body.split(/\n+(?=- \[ \] [A-Z]+\d+[a-z]? )/).map((b) => b.trimEnd());
  const kept = [];
  for (const b of blocks) {
    const id = /^- \[ \] ([A-Z]+\d+[a-z]?) /.exec(b)?.[1];
    if (!id) throw new Error(`${file}: block without an id: ${b.slice(0, 60)}`);
    if (have.has(id)) throw new Error(`${file}: ${id} already on the board`);
    if (drop.includes(id)) continue;
    let block = b;
    if (needs[id]) {
      const at = block.lastIndexOf("needs:");
      block = at < 0 ? `${block}\n  needs: ${needs[id]}` : `${block.slice(0, at)}needs: ${needs[id]}`;
    }
    have.add(id);
    kept.push(block);
  }
  out += `\n\n#### ${title}\n\n${kept.join("\n\n")}`;
  console.log(`${file}: ${kept.length} rows${drop.length ? `, dropped ${drop.join(" ")}` : ""}`);
}
writeFileSync(shard, `${out}\n`);
