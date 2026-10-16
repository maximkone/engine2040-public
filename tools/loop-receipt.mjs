#!/usr/bin/env node
// Keeper per-run receipt writer and gate: spend, policy, verify, rollback plus stamp.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const KEYS = ["spend", "policy", "verify", "rollback"];
export const POLICY_DEFAULT = [
  "owned-paths only",
  "no hash move without receipt",
  "tests named in packet",
];

export function stampNow() {
  return new Date().toISOString();
}

export function normalizeRollback(packet, rollback) {
  if (rollback === true) return { needed: true, patch: `target/abandoned/${packet}.patch` };
  if (typeof rollback === "string") return { needed: true, patch: rollback };
  if (rollback && typeof rollback === "object") return rollback;
  return { needed: false, patch: "" };
}

export function writeReceipt({ packet, spend, policy, verify, rollback, dir = "target/loop-receipts" } = {}) {
  if (!packet) throw new Error("packet name required");
  const receipt = {
    spend: spend ?? { minutes: 0, capMinutes: 40 },
    policy: policy ?? [...POLICY_DEFAULT],
    verify: verify ?? { test: "", result: "pass" },
    rollback: normalizeRollback(packet, rollback),
    stamp: stampNow(),
  };
  mkdirSync(dir, { recursive: true });
  const path = join(dir, `${packet}.json`);
  writeFileSync(path, `${JSON.stringify(receipt, null, 2)}\n`);
  return { path, receipt };
}

export function gateObject(receipt) {
  if (!receipt || typeof receipt !== "object") return { ok: false, missing: "receipt" };
  for (const key of [...KEYS, "stamp"]) {
    if (receipt[key] === undefined) return { ok: false, missing: key };
  }
  return { ok: true, missing: "" };
}

export function gateReceipt(packet, dir = "target/loop-receipts") {
  const path = join(dir, `${packet}.json`);
  if (!existsSync(path)) return { ok: false, missing: "receipt", path };
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return { ok: false, missing: "receipt", path };
  }
  return { ...gateObject(parsed), path };
}

function parseArgs(argv) {
  const out = { dir: "target/loop-receipts" };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--write" || arg === "--check") out.mode = arg.slice(2);
    else if (arg === "--dir" || arg === "--json") out[arg.slice(2)] = argv[++i];
    else if (arg === "--packet") out.packet = argv[++i];
    else if (!arg.startsWith("--") && !out.packet) out.packet = arg;
    else out.unknown = arg;
  }
  return out;
}

function usage() {
  return [
    "usage:",
    "  node tools/loop-receipt.mjs --write <packet> [--dir <dir>] [--json <receipt-json>]",
    "  node tools/loop-receipt.mjs --check <packet> [--dir <dir>]",
    "Writes target/loop-receipts/<packet>.json with spend, policy, verify, rollback plus stamp.",
    "Check prints PASS or FAIL naming the first missing key; FAIL exits 1 to block land.",
  ].join("\n");
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log(usage());
    return;
  }
  const args = parseArgs(argv);
  if (args.unknown || !args.mode || !args.packet) {
    console.log(usage());
    process.exitCode = 1;
    return;
  }
  if (args.mode === "write") {
    let extra = {};
    if (args.json) extra = JSON.parse(args.json);
    const { path } = writeReceipt({ packet: args.packet, dir: args.dir, ...extra });
    console.log(`wrote ${path}`);
    return;
  }
  const res = gateReceipt(args.packet, args.dir);
  console.log(res.ok ? `PASS ${res.path}` : `FAIL ${res.path} missing: ${res.missing}`);
  process.exitCode = res.ok ? 0 : 1;
}

const invoked = (process.argv[1] || "").replace(/\\/g, "/").endsWith("tools/loop-receipt.mjs");
if (invoked) main();
