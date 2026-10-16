// Shared FNV-1a 64 hex over bytes; mirrors package.rs digest_hex (integrity
// only). Single owner: tools/clean-run.mjs and tools/pack-clean.mjs import
// this instead of carrying their own copy.
const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const FNV_MASK = 0xffffffffffffffffn;
const FNV_EMPTY_HEX = "cbf29ce484222325";
export function fnv1a64Hex(bytes) {
  if (bytes.length === 0) return FNV_EMPTY_HEX;
  let hash = FNV_OFFSET;
  const prime = FNV_PRIME;
  const mask = FNV_MASK;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}
