// node skills/fleet-lead/scripts/runlog.mjs <run id> [lines]
// The command step's tail of one mirror run: where a passing test's --nocapture numbers are.
import { execFileSync } from "node:child_process";
import { commandLines } from "../../../tools/gh-check.mjs";

const [rid, n = "60"] = process.argv.slice(2);
const log = execFileSync("gh", ["run", "view", rid, "-R", "maximkone/engine2040-public", "--log"], {
  encoding: "utf8",
  maxBuffer: 1 << 28,
});
const lines = commandLines(log).map((l) =>
  l.split("\t").slice(2).join("\t").replace(/^\S+Z\s/, "").replace(/\x1b\[[0-9;]*m/g, ""),
);
console.log(lines.slice(-Number(n)).join("\n"));
