// MCP6: install the army's own door for OpenCode.
//
// Copies the release `e2040-mcp` binary to `.opencode/bin/` so the MCP server
// OpenCode spawns runs from the copy: a release rebuild can relink
// `target/release/` on Windows while the server keeps running. Then ensures
// `opencode.jsonc` carries a local MCP entry with `--project-dir` on a
// scratch project, plus a disabled `e2040-remote` entry for the pending
// `e2040-mcp --remote http://127.0.0.1:<port>` stateless loopback shim
// (read-only diagnostics; needs the endpoint Rust support first).
//
// `--check` fails when the copy is older than its source build (mtime) or
// differs in size from it. An install that finds the copy locked by a running
// server renames it aside as `e2040-mcp.exe.stale-<yyyy-mm-dd>` (never deleted)
// and copies the new build in.
//
// Usage from the repo/lane root:
//   node tools/mcp-install.mjs [--project-dir DIR] [--remote-url URL]
//     [--bin PATH] [--check] [--help]
//   node tools/mcp-install.mjs --check   verify copy + config, change nothing
// `/sprint` Start refreshes the copy: rerun this script (idempotent).
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// What a running Windows exe answers when its copy is overwritten.
const LOCKED_CODES = new Set(["EBUSY", "EPERM"]);
// The disabled e2040-remote entry keeps this --project-dir until the probed
// binary advertises --remote (startup.rs rejects every other flag).
const REMOTE_PLACEHOLDER_DIR = ".opencode/e2040-scratch";

const pad = (n) => String(n).padStart(2, "0");
const localDay = (date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
const localStamp = (date) => `${localDay(date)} ${pad(date.getHours())}:${pad(date.getMinutes())}`;

// Copies `source` over `dest`. If `dest` is locked (a running server holds it),
// renames it to `<dest>.stale-<yyyy-mm-dd of its mtime>` (a taken name gets
// `-2`, `-3`, ...), then copies again. Nothing is deleted. `deps` injects the
// filesystem steps so tests can simulate a lock.
export function refreshCopy(source, dest, deps = {}) {
  const { copy = copyFileSync, rename = renameSync, stat = statSync, exists = existsSync } = deps;
  try {
    copy(source, dest);
    return { aside: null };
  } catch (error) {
    if (!LOCKED_CODES.has(error.code)) throw error;
    const day = localDay(stat(dest).mtime);
    let aside = `${dest}.stale-${day}`;
    for (let n = 2; exists(aside); n++) aside = `${dest}.stale-${day}-${n}`;
    rename(dest, aside);
    copy(source, dest);
    return { aside };
  }
}

// --check verdict for the copy against its source build. `label` maps a path
// to the text shown (repo-relative in the CLI).
export function copyVerdict(source, dest, label = (p) => p) {
  if (!existsSync(dest)) {
    return { ok: false, text: `copy missing: ${label(dest)} — rerun without --check to install it` };
  }
  const from = statSync(source);
  const to = statSync(dest);
  if (to.mtimeMs < from.mtimeMs) {
    return {
      ok: false,
      text: `copy is stale: ${label(dest)} (${localStamp(to.mtime)}) is older than ${label(source)} (${localStamp(from.mtime)}) — rerun without --check to refresh it`,
    };
  }
  if (to.size !== from.size) {
    return {
      ok: false,
      text: `copy is stale: ${label(dest)} differs in size from ${label(source)} (${to.size} vs ${from.size} bytes) — rerun without --check to refresh it`,
    };
  }
  return { ok: true, text: `copy current: ${label(dest)} matches ${label(source)}` };
}

function main() {
  const ROOT = process.cwd();
  const isWin = process.platform === "win32";
  const BIN_NAME = isWin ? "e2040-mcp.exe" : "e2040-mcp";
  const DEST_BIN = join(ROOT, ".opencode", "bin", BIN_NAME);
  const DEFAULT_SCRATCH = join(ROOT, ".opencode", "e2040-scratch");
  const DEFAULT_REMOTE = "http://127.0.0.1:18789";
  const CONFIG_PATH = join(ROOT, "opencode.jsonc");

  const argv = process.argv.slice(2);
  const flagValue = (key) => {
    const i = argv.indexOf(key);
    return i !== -1 ? argv[i + 1] ?? null : null;
  };
  const has = (key) => argv.includes(key);

  if (has("--help") || has("-h")) {
    console.log(
      "Usage: node tools/mcp-install.mjs [--project-dir DIR] [--remote-url URL] [--bin PATH] [--check]",
    );
    console.log("  Copies the release e2040-mcp binary to .opencode/bin/ and");
    console.log("  ensures opencode.jsonc has local e2040 + disabled e2040-remote entries.");
    console.log("  --check verifies the copy and config without changing anything.");
    process.exit(0);
  }

  const unknown = argv.filter(
    (a, i) =>
      !["--project-dir", "--remote-url", "--bin", "--check"].includes(a) &&
      argv[i - 1] !== "--project-dir" &&
      argv[i - 1] !== "--remote-url" &&
      argv[i - 1] !== "--bin",
  );
  if (unknown.length) {
    console.error(`unknown flag(s): ${unknown.join(" ")}`);
    console.error("Usage: node tools/mcp-install.mjs [--project-dir DIR] [--remote-url URL] [--bin PATH] [--check]");
    process.exit(2);
  }
  const projectDirArg = flagValue("--project-dir");
  const remoteUrl = flagValue("--remote-url") ?? DEFAULT_REMOTE;
  const binOverride = flagValue("--bin");
  for (const [label, value] of [["--project-dir", projectDirArg], ["--remote-url", remoteUrl], ["--bin", binOverride]]) {
    if (value !== null && value !== undefined && value === "") {
      console.error(`${label} needs a non-empty value`);
      process.exit(2);
    }
  }
  const checkOnly = has("--check");
  const scratchDir = resolve(ROOT, projectDirArg ?? DEFAULT_SCRATCH);

  // Release candidates in order: explicit --bin, env, plain release target,
  // then shared lane pools (lanes build with --target-dir target/pool-<k>).
  const candidates = [
    ...(binOverride ? [resolve(ROOT, binOverride)] : []),
    ...(process.env.E2040_MCP_BIN ? [resolve(process.env.E2040_MCP_BIN)] : []),
    join(ROOT, "target", "release", BIN_NAME),
    ...[1, 2, 3, 4].map((k) => join(ROOT, "target", `pool-${k}`, "release", BIN_NAME)),
  ];
  const source = candidates.find((p) => {
    try {
      return existsSync(p);
    } catch {
      return false;
    }
  });

  let failed = false;
  const ok = (text) => console.log(`[OK] ${text}`);
  const info = (text) => console.log(`[INFO] ${text}`);
  const need = (text) => {
    failed = true;
    console.log(`[NEEDS YOU] ${text}`);
  };

  const toPosix = (p) => p.split("\\").join("/");
  const relPosix = (abs) => toPosix(abs).replace(`${toPosix(ROOT)}/`, "");

  // 1. Source binary must exist (release by contract; --bin only for smoke).
  if (!source) {
    need(
      `no e2040-mcp binary found at ${candidates.map(relPosix).join(", ")} — build it first: cargo build --locked --release -p engine-endpoint --features mcp`,
    );
  } else {
    ok(`source binary: ${relPosix(source)}`);
  }

  // 2. Destination copy: the server runs from here so target/ relinks freely.
  const destRel = relPosix(DEST_BIN);
  if (source && !checkOnly) {
    try {
      mkdirSync(dirname(DEST_BIN), { recursive: true });
      const { aside } = refreshCopy(source, DEST_BIN);
      if (aside) info(`${destRel} was locked by a running server: renamed it to ${relPosix(aside)} (kept, not deleted)`);
      ok(`copied to ${destRel} (server runs from the copy; release rebuilds relink target/)`);
    } catch (error) {
      need(`cannot refresh ${destRel}: ${error.message} — stop the running e2040 server and rerun`);
    }
  } else if (source) {
    const verdict = copyVerdict(source, DEST_BIN, relPosix);
    if (verdict.ok) ok(verdict.text);
    else need(verdict.text);
  }

  // 3. Scratch project folder (the server starts empty on a fresh folder).
  if (!checkOnly) {
    try {
      mkdirSync(scratchDir, { recursive: true });
      ok(`project dir: ${relPosix(scratchDir)}`);
    } catch (error) {
      need(`cannot create project dir ${scratchDir}: ${error.message}`);
    }
  } else if (existsSync(scratchDir)) {
    ok(`project dir: ${relPosix(scratchDir)}`);
  } else {
    need(`project dir missing: ${relPosix(scratchDir)} — rerun without --check to create it`);
  }

  // 4. Probe the binary behind the copy (or source when --check and no copy).
  const probeBin = existsSync(DEST_BIN) ? DEST_BIN : source;
  let remoteSupported = false;
  if (probeBin) {
    const help = spawnSync(probeBin, ["--help"], { encoding: "utf8", timeout: 30_000 });
    const text = `${help.stdout ?? ""}${help.stderr ?? ""}`;
    if (!help.error && help.status === 0 && text.includes("--project-dir")) {
      ok(`${relPosix(probeBin)} --help advertises --project-dir`);
    } else {
      need(`${relPosix(probeBin)} --help failed or hides --project-dir — rebuild the binary: cargo build --locked --release -p engine-endpoint --features mcp`);
    }
    remoteSupported = text.includes("--remote");
    if (remoteSupported) ok(`${relPosix(probeBin)} advertises --remote`);
    else info("--remote shim not in this binary yet: e2040-remote stays disabled until the endpoint --remote support lands (proposed to the orchestrator)");
  } else {
    info("binary probe skipped: no binary to probe");
  }

  // 5. opencode.jsonc entries (surgical text edit; every other key byte-identical).
  const binRel = destRel;
  // Absolute: OpenCode starts a local MCP server outside the project root, so a relative --project-dir fails
  // ("bad project dir ... does not exist") while the relative binary path still resolves.
  const scratchRel = scratchDir.split("\\").join("/");
  const managedNote = "managed by tools/mcp-install.mjs (MCP6)";
  const localCmd = `["${binRel}", "--project-dir", "${scratchRel}"]`;
  const remoteCmd = remoteSupported
    ? `["${binRel}", "--remote", "${remoteUrl}"]`
    : `["${binRel}", "--project-dir", "${REMOTE_PLACEHOLDER_DIR}"]`;
  const localEntry = (indent) =>
    `${indent}"e2040": {\n${indent}  "type": "local",\n${indent}  "command": ${localCmd},\n${indent}  "enabled": true\n${indent}}`;
  const remoteEntry = (indent) =>
    `${indent}"e2040-remote": {\n${indent}  // ${managedNote}: stateless loopback shim, read-only diagnostics; stays disabled until the binary supports --remote.\n${indent}  "type": "local",\n${indent}  "command": ${remoteCmd},\n${indent}  "enabled": false\n${indent}}`;

  const desiredBlock = (indent) =>
    `${indent}// BEGIN e2040-mcp ${managedNote}\n${localEntry(indent)},\n${remoteEntry(indent)}\n${indent}// END e2040-mcp`;

  const insertMcpBlock = (text) => {
    // No "mcp" key yet: add one before the final closing brace, keeping the
    // comma on the previous entry's line so the diff stays one hunk.
    const close = text.lastIndexOf("}");
    if (close === -1) return null;
    const before = text.slice(0, close).trimEnd();
    const comma = before.endsWith("{") ? "" : ",";
    return `${before}${comma}\n  "mcp": {\n${desiredBlock("    ")}\n  }\n${text.slice(close)}`;
  };

  const mergeIntoMcp = (text) => {
    // An "mcp" object exists: refresh our two entries, keep any others.
    const key = text.indexOf('"mcp"');
    const open = text.indexOf("{", key);
    if (key === -1 || open === -1) return null;
    let depth = 0;
    for (let i = open; i < text.length; i++) {
      if (text[i] === "{") depth++;
      if (text[i] === "}") {
        depth--;
        if (depth === 0) {
          const inner = text.slice(open + 1, i);
          const hasLocal = inner.includes('"e2040"');
          const hasRemote = inner.includes('"e2040-remote"');
          if (hasLocal && hasRemote) {
            // Both entries exist: refresh only the command arrays in place so
            // reruns are byte-identical (indent, comments and enabled flags
            // untouched). Anything else is the operator's to keep.
            let updated = text;
            updated = updated.replace(
              /("e2040"\s*:\s*\{[^}]*?"command"\s*:\s*)\[[^\]]*\]/s,
              `$1${localCmd}`,
            );
            updated = updated.replace(
              /("e2040-remote"\s*:\s*\{[^}]*?"command"\s*:\s*)\[[^\]]*\]/s,
              `$1${remoteCmd}`,
            );
            // Normalize a comma left on its own line by an older insert.
            updated = updated.replace(/}\n,\n\s*"mcp"\s*:/, '},\n  "mcp":');
            return updated;
          }
          const needsComma = inner.trim() && !inner.trimEnd().endsWith("{") && !inner.trimEnd().endsWith(",");
          const addition = `${needsComma ? "," : ""}\n${desiredBlock("      ")}\n    `;
          return `${text.slice(0, i)}${addition}${text.slice(i)}`;
        }
      }
    }
    return null;
  };

  try {
    const configText = readFileSync(CONFIG_PATH, "utf8");
    const hasMcp = /"mcp"\s*:/.test(configText);
    const hasOurs = configText.includes('"e2040"');
    let updated = null;
    if (hasMcp) updated = mergeIntoMcp(configText);
    else updated = insertMcpBlock(configText);
    if (updated === null) {
      need(`cannot parse ${relPosix(CONFIG_PATH)} — edit the mcp entries by hand (see --help)`);
    } else if (updated !== configText) {
      if (checkOnly) {
        need(`${relPosix(CONFIG_PATH)} would change — rerun without --check to write the e2040 entries`);
      } else {
        writeFileSync(CONFIG_PATH, updated);
        ok(`${relPosix(CONFIG_PATH)} now lists e2040 (local, enabled) + e2040-remote (disabled until --remote lands)`);
      }
    } else {
      // Byte-identical: verify the commands still point at the copy + scratch.
      const pointsAtCopy = configText.includes(`"${binRel}"`);
      const pointsAtScratch = configText.includes(`"${scratchRel}"`);
      if (pointsAtCopy && (hasOurs ? pointsAtScratch : true)) ok(`${relPosix(CONFIG_PATH)} e2040 entries already current`);
      else if (checkOnly) need(`${relPosix(CONFIG_PATH)} entries are stale — rerun without --check to refresh them`);
      else {
        const refreshed = mergeIntoMcp(configText) ?? insertMcpBlock(configText);
        if (refreshed && refreshed !== configText) {
          writeFileSync(CONFIG_PATH, refreshed);
          ok(`${relPosix(CONFIG_PATH)} e2040 entries refreshed to ${binRel} + ${scratchRel}`);
        } else need(`${relPosix(CONFIG_PATH)} entries are stale and the merge failed — edit by hand`);
      }
    }
  } catch (error) {
    need(`cannot read ${relPosix(CONFIG_PATH)}: ${error.message}`);
  }

  console.log(failed ? "RESULT FAIL" : "RESULT PASS");
  process.exitCode = failed ? 1 : 0;
}

// Run the CLI only when this file is the entry point; tests import the helpers.
const isMain = process.argv[1] !== undefined
  && resolve(process.argv[1]).toLowerCase() === fileURLToPath(import.meta.url).toLowerCase();
if (isMain) main();
