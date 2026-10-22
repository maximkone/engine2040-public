// One Rocketbox avatar, fetched into a local home. Nothing is committed: the
// FBX and the textures stay in C:\forge-media\rocketbox, the cook writes the
// game files under target/.
//
//   node tools/asset-gen/rocketbox.mjs --avatar Adults/Male_Adult_08 [--home C:\forge-media\rocketbox] [--dry-run]
//   node tools/asset-gen/rocketbox.mjs --help
//
// Layout it writes, per avatar (the repo's own layout, one folder down):
//   <home>/<Name>/Export/<Name>.fbx       the rigged body (3ds Max Biped)
//   <home>/<Name>/Textures/*.tga          every texture the listing names
//   <home>/<Name>/textures.txt            the texture listing: "name size" per line
//   <home>/<Name>/export.txt              the Export listing, the same way
//   <home>/<Name>/PROVENANCE.json         repo, path, licence MIT, fetched_at
//
// Resumable: a file whose size already matches the listing is skipped; a
// partial file is resumed (curl -C -). One download at a time, and the rate
// is capped at 3 MB/s because the link is a phone hotspot. A dropped network
// (curl 6, 7, 28, 56) waits a minute and tries again, three times per URL.
// --dry-run prints the plan and writes nothing; when textures.txt is already
// there it makes no network call at all.
import { spawnSync, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const REPO = "microsoft/Microsoft-Rocketbox";
export const REPO_URL = "https://github.com/microsoft/Microsoft-Rocketbox";
export const GROUPS = ["Adults", "Children", "Professions"];
export const DEFAULT_HOME = "C:\\forge-media\\rocketbox";
export const RAW = `https://raw.githubusercontent.com/${REPO}/master`;
export const MEDIA = `https://media.githubusercontent.com/media/${REPO}/master`;
export const RATE = "3M";
export const RETRY_CODES = [6, 7, 28, 56];
export const RETRY_WAIT_MS = 60 * 1000;
export const ATTEMPTS = 3;

const USAGE = `usage: node tools/asset-gen/rocketbox.mjs --avatar <Group>/<Name> [--home <dir>] [--dry-run]
  Group: ${GROUPS.join(", ")}. Example: --avatar Adults/Male_Adult_08
  --home  where the avatars live (default ${DEFAULT_HOME})
  --dry-run  print the plan; no download, no write, and no network when textures.txt exists`;

// ------------------------------------------------------------------ pure parts

export function argValue(argv, name, fallback = null) {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
}

// "Adults/Male_Adult_08" -> the group, the name and the repo path.
export function parseAvatar(spec) {
  const m = /^(Adults|Children|Professions)\/([A-Za-z0-9_]+)$/.exec(spec || "");
  if (!m) {
    throw new Error(`--avatar takes <Group>/<Name> with Group one of ${GROUPS.join(", ")}, not ${spec}`);
  }
  return { group: m[1], name: m[2], path: `Assets/Avatars/${m[1]}/${m[2]}` };
}

// The text of a listing, "name size" per line (as `gh api ... --jq` prints it).
export function parseListing(text) {
  return String(text).split(/\r?\n/).map((l) => l.trim()).filter(Boolean).map((line) => {
    const cut = line.lastIndexOf(" ");
    const name = cut > 0 ? line.slice(0, cut) : "";
    const size = cut > 0 ? Number(line.slice(cut + 1)) : NaN;
    if (!name || !Number.isInteger(size) || size < 0) throw new Error(`bad listing line: ${line}`);
    return { name, size };
  });
}

export function listingText(entries) {
  return entries.map((e) => `${e.name} ${e.size}`).join("\n") + "\n";
}

// The download plan: every file the avatar needs, and whether the disk already
// has it. Pure: the caller gives the listings and the home.
export function fetchPlan({ home, avatar, fbx, textures }) {
  const dir = join(home, avatar.name);
  const items = [];
  if (fbx) {
    items.push({
      kind: "fbx",
      name: fbx.name,
      size: fbx.size,
      dest: join(dir, "Export", fbx.name),
      urls: [`${MEDIA}/${avatar.path}/Export/${fbx.name}`, `${RAW}/${avatar.path}/Export/${fbx.name}`],
    });
  }
  for (const t of textures || []) {
    items.push({
      kind: "texture",
      name: t.name,
      size: t.size,
      dest: join(dir, "Textures", t.name),
      urls: [`${RAW}/${avatar.path}/Textures/${t.name}`],
    });
  }
  return items;
}

// Skip a file the disk already holds at the listed size.
export function alreadyHave(item, sizeOf = (p) => (existsSync(p) ? statSync(p).size : -1)) {
  return sizeOf(item.dest) === item.size;
}

export function curlArgs(url, dest) {
  return ["--fail", "--location", "--limit-rate", RATE, "--retry", "3", "--retry-delay", "5",
    "--connect-timeout", "30", "-C", "-", "-o", dest, url];
}

// The FBX in the Export listing, by its .fbx name (the main file, not _facial).
export function pickFbx(entries, avatar) {
  const want = `${avatar.name}.fbx`;
  const hit = entries.find((e) => e.name === want);
  if (!hit) throw new Error(`no ${want} in the Export listing of ${avatar.path}`);
  return hit;
}

export function provenance({ avatar, fbxName, textures, bytes, now = new Date() }) {
  return {
    source: REPO_URL,
    path: avatar.path,
    avatar: avatar.name,
    group: avatar.group,
    licence: "MIT",
    fbx: fbxName,
    textures: textures.length,
    bytes,
    fetched_at: now.toISOString(),
  };
}

// ------------------------------------------------------------- side effects

function fail(msg) {
  console.error(`rocketbox: ${msg}`);
  process.exit(1);
}

function sleep(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

// `gh api` for one repo folder, as "name size" lines.
export function ghListing(avatar, sub) {
  const text = execFileSync("gh", [
    "api", `repos/${REPO}/contents/${avatar.path}/${sub}`,
    "--jq", '.[] | select(.type == "file") | "\\(.name) \\(.size)"',
  ], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"], timeout: 120000 });
  return parseListing(text);
}

// The cached listing when there is one. Without a cache, `network` says
// whether to ask GitHub; a fresh listing is written to the cache unless dry.
function listing({ dir, file, avatar, sub, network, dry }) {
  const cache = join(dir, file);
  if (existsSync(cache)) return parseListing(readFileSync(cache, "utf8"));
  if (!network) return null;
  const entries = ghListing(avatar, sub);
  if (!dry) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(cache, listingText(entries), "utf8");
  }
  return entries;
}

function download(item) {
  for (const url of item.urls) {
    for (let attempt = 1; attempt <= ATTEMPTS; attempt++) {
      mkdirSync(join(item.dest, ".."), { recursive: true });
      console.log(`   ${item.name} (${item.size} bytes) from ${url.includes("media.") ? "media" : "raw"}`);
      const r = spawnSync("curl", curlArgs(url, item.dest), { stdio: ["ignore", "inherit", "inherit"] });
      if (r.error) throw new Error(`curl did not start: ${r.error.message}`);
      if (r.status === 0) {
        const got = statSync(item.dest).size;
        if (got !== item.size) throw new Error(`${item.name}: ${got} bytes on disk, the listing says ${item.size}`);
        return;
      }
      if (RETRY_CODES.includes(r.status) && attempt < ATTEMPTS) {
        console.error(`   network dropped (curl ${r.status}), waiting 60 s before retry ${attempt + 1}`);
        sleep(RETRY_WAIT_MS);
        continue;
      }
      console.error(`   curl exit ${r.status} for ${url}`);
      break; // a 404 on the media URL falls through to the raw one
    }
  }
  throw new Error(`could not fetch ${item.name} from any URL, partial file kept for a resume`);
}

export function main(argv = process.argv) {
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log(USAGE);
    return 0;
  }
  let avatar;
  try {
    avatar = parseAvatar(argValue(argv, "avatar"));
  } catch (err) {
    fail(err.message);
  }
  const home = resolve(argValue(argv, "home", DEFAULT_HOME));
  const dry = argv.includes("--dry-run");
  const dir = join(home, avatar.name);
  // Dry runs touch the network only when there is no texture listing yet.
  const network = !dry || !existsSync(join(dir, "textures.txt"));

  let exportList;
  let textures;
  try {
    textures = listing({ dir, file: "textures.txt", avatar, sub: "Textures", network, dry });
    exportList = listing({ dir, file: "export.txt", avatar, sub: "Export", network, dry });
  } catch (err) {
    fail(`could not list ${avatar.path} on GitHub (${err.message.split("\n")[0]})`);
  }
  if (!textures) fail(`no texture listing in ${dir} and no network allowed in a dry run`);

  let fbx = null;
  if (exportList) {
    fbx = pickFbx(exportList, avatar);
  } else {
    const local = join(dir, "Export", `${avatar.name}.fbx`);
    if (dry) {
      // Offline dry run: the FBX is planned once a real run has listed it.
      console.log(`rocketbox: no Export listing yet, the FBX is not sized (a real run lists it)`);
    } else if (existsSync(local)) {
      // Offline real run with the FBX already on disk: keep it, unverified.
      console.error(`rocketbox: no Export listing, keeping ${local} unverified`);
      fbx = { name: `${avatar.name}.fbx`, size: statSync(local).size };
    } else {
      fail(`no FBX on disk at ${local} and no Export listing to size it`);
    }
  }
  const items = fetchPlan({ home, avatar, fbx, textures });
  const todo = items.filter((i) => !alreadyHave(i));
  const bytes = todo.reduce((s, i) => s + i.size, 0);
  console.log(`${avatar.path}: ${items.length} files, ${todo.length} to fetch (${bytes} bytes), home ${home}`);
  for (const item of items) {
    console.log(`  ${alreadyHave(item) ? "have" : "get "} ${item.kind} ${item.name} ${item.size}`);
  }
  if (dry) {
    console.log("dry run: nothing fetched, nothing written");
    return 0;
  }

  mkdirSync(dir, { recursive: true });
  try {
    for (const item of todo) {
      console.log(`-> ${item.name}`);
      download(item);
    }
  } catch (err) {
    fail(err.message);
  }
  const sizeAll = items.reduce((s, i) => s + i.size, 0);
  writeFileSync(join(dir, "PROVENANCE.json"),
    JSON.stringify(provenance({ avatar, fbxName: fbx.name, textures, bytes: sizeAll }), null, 2) + "\n", "utf8");
  console.log(`RB-FETCH ${avatar.path} ${items.length} files in ${dir}`);
  console.log(`WROTE ${join(dir, "PROVENANCE.json")}`);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
