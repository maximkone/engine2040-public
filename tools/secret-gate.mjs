// Donor: gitleaks/gitleaks@b58d3f102cf3a2c84cb7f923d05c25c9b1aed84b, MIT licence (verified live: gh api repos/gitleaks/gitleaks/license -> MIT).
// Link: https://github.com/gitleaks/gitleaks
// Port: Shannon entropy check on each match (detect/utils.go shannonEntropy + detect/detect.go entropy <= r.Entropy skip) as one gate at 3.8.
// Pattern reuse only otherwise: secret-pattern table + staged-diff scan shape. No other donor code.
// Fresh Node implementation.
// Usage: node tools/secret-gate.mjs [--self-test] [--help | -h]
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const SECRET_HELP = [
  "Usage: node tools/secret-gate.mjs [--self-test]",
  "       node tools/secret-gate.mjs --help | -h",
  "",
  "Local secret-scan pre-push gate (secrets escaping to CI go to 0).",
  "  node tools/secret-gate.mjs             scan staged diff added lines, PASS with file count",
  "  node tools/secret-gate.mjs --self-test seed one temp secret, verify it is counted, exit 0",
].join("\n");

export function parseSecretArgs(argv) {
  if (argv.length > 1 || argv.some((arg) => !["--self-test"].includes(arg))) {
    throw new Error("Usage: node tools/secret-gate.mjs [--self-test]\nTry \u0027node tools/secret-gate.mjs --help\u0027 for usage.");
  }
  return argv;
}

// Small high-confidence secret-pattern table (gitleaks shape, fresh regex).
// Kept small so the gate stays fast and false-positive free.
// Per-rule entropy mirrors the donor: entropy 0 disables the check
// (private-key header is fixed text, never random); token rules use 3.8.
export const ENTROPY_THRESHOLD = 3.8;
export const SECRET_PATTERNS = [
  { name: "aws-access-key", re: /\bAKIA[0-9A-Z]{16}\b/g, entropy: ENTROPY_THRESHOLD },
  { name: "github-token", re: /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b/g, entropy: ENTROPY_THRESHOLD },
  { name: "slack-token", re: /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/g, entropy: ENTROPY_THRESHOLD },
  { name: "stripe-live-key", re: /\bsk_live_[A-Za-z0-9]{16,}\b/g, entropy: ENTROPY_THRESHOLD },
  { name: "private-key", re: /-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----/g },
  {
    name: "password-assign",
    re: /\b(?:password|passwd|secret|api[_-]?key)\s*[:=]\s*["\u0027][^"\u0027]{8,}["\u0027]/gi,
    entropy: ENTROPY_THRESHOLD,
  },
];

// Shannon entropy in bits per char (donor detect/utils.go shannonEntropy).
// Empty means 0. Higher means more random.
export function shannonEntropy(data) {
  if (!data) return 0;
  const counts = new Map();
  for (const ch of data) counts.set(ch, (counts.get(ch) ?? 0) + 1);
  const invLength = 1 / data.length;
  let entropy = 0;
  for (const count of counts.values()) {
    const freq = count * invLength;
    entropy -= freq * Math.log2(freq);
  }
  return entropy;
}

const MAX_FILES = 500;
const MAX_DIFF_BYTES = 2 * 1024 * 1024;
const SKIP_PATH = new Set(["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "Cargo.lock"]);

function isSkippedPath(path) {
  const lower = path.toLowerCase().replace(/\\/g, "/");
  if (lower.split("/").some((part) => part.startsWith(".") && part !== ".")) return true;
  const base = lower.split("/").pop() ?? lower;
  if (SKIP_PATH.has(base)) return true;
  return false;
}

function activePatterns(allow) {
  if (!allow || allow.size === 0) return SECRET_PATTERNS;
  return SECRET_PATTERNS.filter((p) => !allow.has(p.name.toLowerCase()));
}

// Allowlist file `.secret-allow` holds one pattern name per line (for example
// `password-assign`). Missing file means every pattern is active.
// Fail closed: an unreadable allowlist throws so the caller fails the scan,
// never silently treats unknown config as clean.
export function loadAllowList(repoRoot) {
  const path = join(repoRoot, ".secret-allow");
  if (!existsSync(path)) return new Set();
  try {
    const text = readFileSync(path, "utf8");
    return new Set(
      text
        .split("\n")
        .map((line) => line.trim().toLowerCase())
        .filter((line) => line && !line.startsWith("#")),
    );
  } catch (error) {
    if (error?.code === "ENOENT") return new Set();
    throw new Error(".secret-allow unreadable: " + (error?.message ?? String(error)));
  }
}

// Scan added-line text for active secret patterns; returns violation lines.
// Each regex match passes the donor Shannon check: entropy <= threshold skips.
// Fail closed: every match on a line is checked, so a low-entropy decoy
// before a real secret never hides it.
export function scanText(relativePath, text, patterns) {
  const found = [];
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    for (const pattern of patterns) {
      pattern.re.lastIndex = 0;
      let match;
      while ((match = pattern.re.exec(lines[i])) !== null) {
        if (match[0].length === 0) {
          pattern.re.lastIndex += 1;
          continue;
        }
        if (pattern.entropy) {
          if (shannonEntropy(match[0]) <= pattern.entropy) continue;
        }
        found.push(relativePath + ":" + (i + 1) + ": [" + pattern.name + "] possible secret");
        break;
      }
    }
  }
  return found;
}

function runGit(repoRoot, args) {
  const git = spawnSync("git", args, { cwd: repoRoot, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  // Fail closed: null means the scan input is unknown, never clean.
  if (git.error || git.status !== 0) return null;
  return String(git.stdout ?? "");
}

// Parse `git diff --cached --unified=0` into per-file added-line buffers.
// Only `+` lines (not `+++` headers) are scanned, gitleaks staged-diff shape.
export function parseStagedDiff(diff) {
  const files = new Map();
  let current = "";
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++ b/")) {
      current = line.slice("+++ b/".length).trim();
      if (!files.has(current)) files.set(current, []);
    } else if (line.startsWith("+") && !line.startsWith("+++") && current) {
      files.get(current).push(line.slice(1));
    } else if (line.startsWith("diff --git ")) {
      current = "";
    }
  }
  const out = [];
  for (const [file, added] of files) out.push({ file, text: added.join("\n") });
  return out;
}

export function capEntries(entries, cap = MAX_FILES) {
  if (entries.length > cap) {
    return { entries: [], error: "staged files exceed " + cap + " (got " + entries.length + ")" };
  }
  return { entries, error: null };
}

export function scanStaged(repoRoot, cap = MAX_FILES) {
  let allow;
  try {
    allow = loadAllowList(repoRoot);
  } catch (error) {
    return { scanned: 0, violations: [], error: error?.message ?? String(error) };
  }
  const patterns = activePatterns(allow);
  const diff = runGit(repoRoot, ["diff", "--cached", "--unified=0", "--no-color"]);
  // Fail closed: unknown or unscannable input is an error, never a clean PASS.
  if (diff === null) return { scanned: 0, violations: [], error: "git diff --cached failed" };
  if (diff.length > MAX_DIFF_BYTES) {
    return { scanned: 0, violations: [], error: "staged diff exceeds " + MAX_DIFF_BYTES + " bytes" };
  }
  if (!diff) return { scanned: 0, violations: [] };
  const filtered = parseStagedDiff(diff).filter((e) => e.file && !isSkippedPath(e.file));
  // Fail closed: over-cap input is an error, never a truncated clean PASS.
  const { entries, error: capError } = capEntries(filtered, cap);
  if (capError) return { scanned: 0, violations: [], error: capError };
  const violations = [];
  for (const entry of entries) {
    violations.push(...scanText(entry.file, entry.text, patterns));
  }
  return { scanned: entries.length, violations };
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), "secret-gate-"));
  try {
    // Fixture assembled at run time: this file must stay clean under its own gate.
    const seeded = "token " + "ghp_" + "abcdefghijklmnopqrstuvwxyz0123456789" + " here\n";
    writeFileSync(join(dir, "seed.txt"), seeded, "utf8");
    const text = readFileSync(join(dir, "seed.txt"), "utf8");
    const found = scanText("seed.txt", text, SECRET_PATTERNS);
    const count = found.length;
    if (count < 1) {
      console.error("FAIL: self-test seeded 1 secret, detected " + count);
      process.exit(1);
    }
    console.log("PASS: self-test detected " + count + " seeded secret in 1 file");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function main() {
  const raw = process.argv.slice(2);
  if (raw.includes("--help") || raw.includes("-h")) {
    console.log(SECRET_HELP);
    process.exit(0);
  }
  let args;
  try {
    args = parseSecretArgs(raw);
  } catch (error) {
    console.error(error.message);
    process.exit(2);
  }
  if (args.includes("--self-test")) {
    selfTest();
    return;
  }
  const repoRoot = resolve(join(fileURLToPath(import.meta.url), "..", ".."));
  const { scanned, violations, error } = scanStaged(repoRoot);
  if (error) {
    console.error("FAIL: secret scan unavailable: " + error);
    process.exit(1);
  }
  if (violations.length > 0) {
    for (const line of violations.slice(0, 50)) console.log(line);
    if (violations.length > 50) console.log("... and " + (violations.length - 50) + " more");
    console.log("FAIL: " + violations.length + " secrets in " + scanned + " staged files");
    process.exit(1);
  }
  console.log("PASS: " + scanned + " staged files scanned, 0 secrets");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
