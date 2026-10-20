// Hunt helper: GitHub repo search -> TSV. Read-only.
// usage: node ghs.mjs "<query>" [per_page] [sort]
import { execFileSync } from "node:child_process";

const q = process.argv[2];
const per = process.argv[3] || "25";
const sort = process.argv[4] || "stars";
if (!q) {
  console.error("usage: node ghs.mjs <query> [per_page] [sort]");
  process.exit(2);
}
const url = `search/repositories?q=${encodeURIComponent(q)}&sort=${sort}&order=desc&per_page=${per}`;
const jq = ".items[] | [.full_name, .stargazers_count, (.license.spdx_id // \"NOASSERTION\"), .pushed_at, (.description // \"\" | gsub(\"[\\n\\t\\r]+\"; \" \") | .[0:120]), .default_branch] | @tsv";
let out = "";
try {
  out = execFileSync("gh", ["api", url, "--jq", jq], { encoding: "utf8", maxBuffer: 1 << 26 });
} catch (e) {
  console.error("SEARCH FAILED: " + (e.stderr || e.message).toString().slice(0, 400));
  process.exit(1);
}
process.stdout.write("repo\tstars\tlicense\tpushed\tdescription\tbranch\n" + out);
