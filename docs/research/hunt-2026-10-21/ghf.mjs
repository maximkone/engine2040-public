// Hunt helper: repo file tree or one file's content. Read-only.
// usage: node ghf.mjs <owner/repo> tree [subpath]
//        node ghf.mjs <owner/repo> file <path> [ref]
import { execFileSync } from "node:child_process";

const repo = process.argv[2];
const mode = process.argv[3];
if (!repo || !mode) {
  console.error("usage: node ghf.mjs <owner/repo> tree [subpath] | file <path> [ref]");
  process.exit(2);
}

function gh(args) {
  return execFileSync("gh", args, { encoding: "utf8", maxBuffer: 1 << 26 });
}

try {
  if (mode === "tree") {
    const sub = process.argv[4] || "";
    const info = JSON.parse(gh(["api", `repos/${repo}`]));
    const branch = info.default_branch;
    const ref = info.pushed_at ? branch : branch;
    const t = JSON.parse(gh(["api", `repos/${repo}/git/trees/${ref}?recursive=1`]));
    const files = (t.tree || []).filter((e) => e.type === "blob");
    const pat = sub ? new RegExp(sub, "i") : /./;
    for (const f of files) {
      if (pat.test(f.path)) console.log(`${(f.size / 1024).toFixed(1)}K\t${f.path}`);
    }
    console.error(`-- ${repo} ${branch} ${files.length} blobs, truncated=${t.truncated}`);
  } else if (mode === "file") {
    const path = process.argv[4];
    const ref = process.argv[5] || "";
    const r = gh(["api", `repos/${repo}/contents/${path}${ref ? `?ref=${ref}` : ""}`, "--jq", ".content"]);
    const b64 = r.replace(/\s+/g, "");
    const buf = Buffer.from(b64, "base64");
    process.stdout.write(buf.toString("utf8"));
  } else if (mode === "meta") {
    const info = JSON.parse(gh(["api", `repos/${repo}`]));
    const keys = ["full_name", "stargazers_count", "forks_count", "language", "size", "pushed_at", "default_branch", "archived"];
    for (const k of keys) console.log(`${k}: ${info[k]}`);
    console.log(`license: ${info.license ? info.license.spdx_id : "none"}`);
    console.log(`desc: ${info.description || ""}`);
    console.log(`topics: ${(info.topics || []).join(",")}`);
  } else {
    console.error("mode must be tree|file|meta");
    process.exit(2);
  }
} catch (e) {
  console.error("FAILED: " + (e.stderr || e.message).toString().slice(0, 500));
  process.exit(1);
}
