import { createHash } from "node:crypto";

// Cache identical inputs: repeated hashes of the same bytes skip node:crypto.
// Bounded Map (FIFO evict) for varied repeats plus a last-value fast path
// (memcmp, no hex) for back-to-back repeats. Digests are unchanged.
const cache = new Map();
const MAX_CACHE = 128;
// Large inputs (frame PNGs, glb bytes) skip the Map cache: the hex key
// build scans the whole buffer and never hits on varied bytes, so it
// costs more than the hash it tries to save. The last-value path still
// covers back-to-back repeats. Digests are unchanged.
const CACHE_BYPASS_BYTES = 4096;
let lastInput = null;
let lastDigest = "";

function asView(buf) {
  if (typeof buf === "string") return Buffer.from(buf);
  if (Buffer.isBuffer(buf)) return buf;
  return Buffer.from(buf.buffer, buf.byteOffset, buf.byteLength);
}

function hexKey(view) {
  return view.length + ":" + view.toString("hex");
}

export function sha256(buf) {
  const view = asView(buf);
  if (lastInput !== null && lastInput.length === view.length && lastInput.equals(view)) {
    return lastDigest;
  }
  if (view.length > CACHE_BYPASS_BYTES) {
    const digest = createHash("sha256").update(buf).digest("hex");
    lastInput = Buffer.from(view);
    lastDigest = digest;
    return digest;
  }
  const key = hexKey(view);
  const hit = cache.get(key);
  if (hit !== undefined) {
    lastInput = Buffer.from(view);
    lastDigest = hit;
    return hit;
  }
  const digest = createHash("sha256").update(buf).digest("hex");
  if (cache.size >= MAX_CACHE) {
    const oldest = cache.keys().next();
    if (!oldest.done) cache.delete(oldest.value);
  }
  cache.set(key, digest);
  lastInput = Buffer.from(view);
  lastDigest = digest;
  return digest;
}
