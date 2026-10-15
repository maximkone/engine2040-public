// Checked numeric cast with loud failure. Never silently truncates.
// Attribution, idea only, fresh code, no donor copy:
// Port of the cast discipline from rust-num/num-traits (MIT / Apache-2.0) at
// https://github.com/rust-num/num-traits: ToPrimitive / FromPrimitive /
// NumCast where cast(v) returns Option and yields None on overflow,
// fractional truncation, or non-finite float. All lines below are original.
// Consumer: tools/cast-safe.test.mjs. Pure functions; no IO, no clock.
const INT_RANGES = {
  i8: [-128, 127],
  u8: [0, 255],
  i16: [-32768, 32767],
  u16: [0, 65535],
  i32: [-2147483648, 2147483647],
  u32: [0, 4294967295],
};
const BIGINT_RANGES = {
  i64: [-(2n ** 63n), 2n ** 63n - 1n],
  u64: [0n, 2n ** 64n - 1n],
};
const F32_MAX = 3.4028234663852886e38;
const TARGETS = new Set([
  ...Object.keys(INT_RANGES),
  ...Object.keys(BIGINT_RANGES),
  "f32",
  "f64",
]);
function assertTarget(target) {
  if (!TARGETS.has(target)) {
    throw new TypeError("[cast-safe] unknown target " + String(target));
  }
}
function toBigintOrThrow(value) {
  if (typeof value === "bigint") return value;
  if (typeof value === "number") {
    if (!Number.isInteger(value)) {
      throw new RangeError("[cast-safe] fractional " + String(value) + " would truncate");
    }
    return BigInt(value);
  }
  throw new TypeError("[cast-safe] cannot cast " + String(value));
}
function castNumberToInt(value, target) {
  if (typeof value === "boolean" || value === null || typeof value === "string") {
    throw new TypeError("[cast-safe] bad input type for " + target);
  }
  if (typeof value === "bigint") {
    const r = INT_RANGES[target];
    if (value < BigInt(r[0]) || value > BigInt(r[1])) {
      throw new RangeError("[cast-safe] out of range for " + target);
    }
    return Number(value);
  }
  if (typeof value !== "number") {
    throw new TypeError("[cast-safe] bad input type for " + target);
  }
  if (!Number.isFinite(value)) {
    throw new RangeError("[cast-safe] non-finite value for " + target);
  }
  if (!Number.isInteger(value)) {
    throw new RangeError("[cast-safe] fractional value would truncate to " + target);
  }
  const lim = INT_RANGES[target];
  if (value < lim[0] || value > lim[1]) {
    throw new RangeError("[cast-safe] out of range for " + target);
  }
  return value;
}
function castToBigint(value, target) {
  const asBig = toBigintOrThrow(value);
  const r = BIGINT_RANGES[target];
  if (asBig < r[0] || asBig > r[1]) {
    throw new RangeError("[cast-safe] out of range for " + target);
  }
  return asBig;
}
function castToFloat(value, target) {
  if (typeof value === "boolean" || value === null || typeof value === "string") {
    throw new TypeError("[cast-safe] bad input type for " + target);
  }
  let asNumber;
  if (typeof value === "bigint") asNumber = Number(value);
  else if (typeof value === "number") asNumber = value;
  else throw new TypeError("[cast-safe] bad input type for " + target);
  if (!Number.isFinite(asNumber)) {
    throw new RangeError("[cast-safe] non-finite value for " + target);
  }
  if (target === "f64") return asNumber;
  if (Math.abs(asNumber) > F32_MAX) {
    throw new RangeError("[cast-safe] value overflows f32");
  }
  return Math.fround(asNumber);
}
export function cast(value, target) {
  assertTarget(target);
  if (target in INT_RANGES) return castNumberToInt(value, target);
  if (target in BIGINT_RANGES) return castToBigint(value, target);
  return castToFloat(value, target);
}
export function tryCast(value, target) {
  assertTarget(target);
  try {
    return cast(value, target);
  } catch (err) {
    if (err instanceof RangeError || err instanceof TypeError) {
      if (String(err.message).indexOf("unknown target") !== -1) throw err;
      return null;
    }
    throw err;
  }
}
export function isCastable(value, target) {
  assertTarget(target);
  try {
    cast(value, target);
    return true;
  } catch (err) {
    if (err instanceof RangeError || err instanceof TypeError) {
      if (String(err.message).indexOf("unknown target") !== -1) throw err;
      return false;
    }
    throw err;
  }
}
