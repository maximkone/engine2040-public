// Shared content-pipeline paths: env-first Blender + temp resolution.
// Invariant: on this machine the resolved Blender path equals the old
// hard-coded default, and the temp dir equals the old hard-coded dir.
const fs = require('fs');
const os = require('os');
const path = require('path');

const BLENDER_DEFAULT = 'C:/Program Files/Blender Foundation/Blender 5.2/blender.exe';

function localAppData() {
  return process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local');
}

// Version-agnostic Blender installs under one vendor root, newest first.
function expandVendorVersions(vendorDir) {
  try {
    return fs.readdirSync(vendorDir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && /^blender/i.test(e.name))
      .map((e) => e.name)
      .sort()
      .reverse()
      .map((name) => path.join(vendorDir, name, 'blender.exe'));
  } catch {
    return [];
  }
}

function blenderCandidates() {
  const perUserVendor = path.join(localAppData(), 'Programs', 'Blender Foundation');
  const seen = new Set();
  const out = [];
  const push = (p) => {
    if (!seen.has(p)) { seen.add(p); out.push(p); }
  };
  push(BLENDER_DEFAULT);
  push(path.join(perUserVendor, 'Blender 5.2', 'blender.exe'));
  for (const vendor of ['C:/Program Files/Blender Foundation', perUserVendor]) {
    for (const p of expandVendorVersions(vendor)) push(p);
  }
  return out;
}

function resolveBlenderPath() {
  const env = (process.env.BLENDER_PATH || '').trim();
  if (env) return env;
  const tried = blenderCandidates();
  for (const candidate of tried) {
    try {
      if (fs.existsSync(candidate)) return candidate;
    } catch {
      // Treat unreadable entries as misses and keep scanning.
    }
  }
  throw new Error(
    'Blender not found. Set BLENDER_PATH to blender.exe, or install Blender. Tried:\n'
    + tried.join('\n'),
  );
}

function opencodeTempDir() {
  const base = process.env.TEMP || process.env.TMP || os.tmpdir();
  return path.join(base, 'opencode');
}

const BLENDER = resolveBlenderPath();
const TMP = opencodeTempDir();

// Only the resolved values cross the module boundary (every consumer takes
// BLENDER and/or TMP); the search helpers stay private (deadcode 2026-10-02).
module.exports = { BLENDER, TMP };
