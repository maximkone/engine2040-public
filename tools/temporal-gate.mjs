// temporal-gate: deterministic ghosting score over PNG frame sequences.
//
// Usage:
//   node tools/temporal-gate.mjs [--bar N] [--json] <dir>
//   node tools/temporal-gate.mjs --pair [--bar N] [--json] <offdir> <ondir>
//   node tools/temporal-gate.mjs --selfcheck [--bar N]
//
// Frames are frame_*.png in tick order (the arena capture layout), decoded
// with tools/iqa-gate.mjs::decodePng (node's own zlib, no dependency) and
// reduced to BT.601 luma. Units: luma is 0..255, every score is a fraction
// 0..1. Determinism: integer luma, fixed frame order, same bytes in give the
// same score. The metric reads captures only; it never runs the pass.
//
// Single mode. For each frame t the metric measures
// `step = mean |luma(t) - luma(t-1)|` and `best`, the smallest mean difference
// to a frame 2..LAG_MAX back. A frame that follows its predecessor has
// `best >= step` and scores 0. A frame that repeats an older frame has
// `best ~ 0` and scores ~1. The score is the largest
// `1 - best/step` over the sequence, so a frozen or mis-registered history
// that republishes an old frame reads high and a still sequence reads 0.
//
// Pair mode (the ENG20 gate). <offdir> is the TAA-off capture, the frame the
// scene really holds; <ondir> is the TAA-on capture of the same run, aligned
// by frame name and size. For every pixel the reference moved between t-1 and
// t, the metric measures how much of the test pixel is older reference
// content: `alpha = (ref_t - on_t) / (ref_t - ref_{t-k})` for k in
// 1..LAG_MAX, the largest alpha. A correct resolve reproduces ref_t and
// reads alpha 0; a frozen or mis-registered history keeps the blend
// (alpha near DEFAULT_HISTORY_BLEND). The score is the mean alpha over moved
// pixels, independent of resolution and pan speed. "separation" is that
// score: the off run is the zero reference by construction.
//
// Limit, stated so the gate is not overread. The single score is
// frame-level: a partial ghost trail averaged into the rest of the frame
// moves it little, so it catches "the frame belongs to an older moment", not
// a small edge trail. The pair score is the sensitive arm: it reads the
// trail directly, but only when the TAA-off reference is the same run and
// frame range, so both captures must be kept aligned.
//
// Calibration, measured 2026-10-23. Single mode over the real captures in
// target/captures: TAA-on gameplay scores 0.225 (spear-grapple, 125 frames,
// worst frame_66), 0.481 (rz2w-a, 65 frames) and 0.631/0.669 (the rz2-ab
// ragdoll pair, 70-71 frames; it is not a TAA toggle). The synthetic stale
// frame scores 1.000. Pair mode on the CPU twin capture
// (crates/render/tests/it/taa_pair.rs; 160x90, 12 frames): the registered
// TAA-on arm reproduces the TAA-off reference and scores 0.000; the frozen
// arm scores 0.246 (ghost share 0.273 of moved pixels, worst frame_000001,
// lag 1) and trips the 0.05 pair bar; a 10% old-frame blend scores 0.100.
// No live TAA-on/off pair exists yet, so the pair bar is provisional until
// one is captured through the presenter switch. A subtle blend ghost below
// the bar is the open leg (ENG20). This file lands the number, not the
// CGVQM+D model: that model needs its published weights.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { decodePng } from "./iqa-gate.mjs";

/** Mean luma step (0..255) below which a frame counts as still; ratio noise is ignored. */
export const MOVE_MIN = 1.0;
/** How far back the metric looks for old content, in frames (includes t-1). */
export const LAG_MAX = 4;
/** Ghosting bar: a score above this exits 1. Provisional: above observed real runs. */
export const DEFAULT_BAR = 0.75;
/** Pair bar: mean old-content fraction over pixels that moved. Provisional. */
export const PAIR_BAR = 0.05;
/** A moved pixel counts as ghosted when at least half of it is old content. */
export const PAIR_GHOST_ALPHA = 0.5;
/** Luma step below which the reference did not move this frame (0..255). */
export const PAIR_MIN_STEP = 6;

/** BT.601 luma of RGBA bytes, one u8 per pixel. */
export function lumaFrame(rgba, width, height) {
  const n = width * height;
  if (!Number.isInteger(n) || n <= 0) throw new Error(`bad frame size ${width}x${height}`);
  if (rgba.length !== n * 4) throw new Error(`rgba ${rgba.length} bytes, expected ${n * 4}`);
  const luma = new Uint8Array(n);
  for (let i = 0, p = 0; i < n; i += 1, p += 4) {
    luma[i] = (77 * rgba[p] + 150 * rgba[p + 1] + 29 * rgba[p + 2] + 128) >> 8;
  }
  return luma;
}

/** Mean absolute luma difference between two same-size frames, 0..255. */
export function meanAbsDiff(a, b) {
  if (a.length !== b.length) throw new Error("frame size mismatch");
  let sum = 0;
  for (let i = 0; i < a.length; i += 1) sum += Math.abs(a[i] - b[i]);
  return sum / a.length;
}

/**
 * Ghost score over luma frames, oldest first. Returns the worst frame's
 * score (0..1), its index, the lag that frame matched best (1 = it followed
 * its predecessor, 2..LAG_MAX = it repeated an older frame), its step and
 * best distances, and every measured frame.
 */
export function ghostScore(frames, width, height, opts = {}) {
  const lagMax = opts.lagMax ?? LAG_MAX;
  const moveMin = opts.moveMin ?? MOVE_MIN;
  if (lagMax < 2) throw new Error(`lagMax ${lagMax} must be at least 2`);
  if (frames.length === 0) throw new Error("no frames");
  const n = width * height;
  frames.forEach((frame, i) => {
    if (frame.length !== n) throw new Error(`frame ${i}: ${frame.length} samples, expected ${n}`);
  });
  const measured = [];
  let best = { score: 0, frame: -1, lag: 1, step: 0, best: 0 };
  const recent = []; // recent[0] is t-1, recent[1] is t-2, ...
  for (let t = 0; t < frames.length; t += 1) {
    const cur = frames[t];
    if (t >= 1) {
      const step = meanAbsDiff(cur, recent[0]);
      let closest = step; // t-1 is always a candidate
      let lag = 1;
      for (let i = 1; i < recent.length; i += 1) {
        const diff = meanAbsDiff(cur, recent[i]);
        if (diff < closest) {
          closest = diff;
          lag = i + 1;
        }
      }
      const still = step <= moveMin;
      const score = still ? 0 : Math.max(0, 1 - closest / step);
      measured.push({
        index: t,
        lag,
        step: round3(step),
        best: round3(closest),
        score: round3(score),
      });
      if (score > best.score) {
        best = {
          score,
          frame: t,
          lag,
          step: round3(step),
          best: round3(closest),
        };
      }
    }
    recent.unshift(cur);
    if (recent.length > lagMax) recent.pop();
  }
  return { ...best, pixels: n, measured };
}

function round3(value) {
  return Number(value.toFixed(3));
}

/** Read frame_*.png (sorted by name) into luma frames. */
export function readSequence(dir) {
  let names;
  try {
    names = readdirSync(dir)
      .filter((name) => /^frame_\d+\.png$/.test(name))
      .sort();
  } catch (error) {
    throw new Error(`${dir}: no frame_*.png (${error.code ?? "read failed"})`);
  }
  if (names.length === 0) throw new Error(`${dir}: no frame_*.png`);
  const frames = [];
  let width = 0;
  let height = 0;
  for (const name of names) {
    const png = decodePng(readFileSync(join(dir, name)), name);
    if (frames.length === 0) {
      width = png.w;
      height = png.h;
    } else if (png.w !== width || png.h !== height) {
      throw new Error(`${name}: ${png.w}x${png.h}, the sequence is ${width}x${height}`);
    }
    frames.push(lumaFrame(png.data, width, height));
  }
  return { width, height, names, frames };
}

/** Score one capture directory. */
export function measureDir(dir, opts = {}) {
  const sequence = readSequence(dir);
  const scored = ghostScore(sequence.frames, sequence.width, sequence.height, opts);
  return {
    ...scored,
    dir,
    width: sequence.width,
    height: sequence.height,
    frame_count: sequence.names.length,
    worst: scored.frame >= 0 ? sequence.names[scored.frame] : null,
  };
}

/**
 * Score a TAA-on sequence against its TAA-off reference: same size, same
 * frame count, oldest first. For each frame after the first, take the pixels
 * the reference moved between t-1 and t, then measure the largest fraction of
 * old reference content (t-1..t-lagMax) the test frame still shows there:
 * `alpha = (ref_t - on_t) / (ref_t - ref_{t-k})`. A correct resolve has
 * `on_t == ref_t` (alpha 0); a frozen or mis-registered history keeps the
 * history blend (alpha near the blend). The score is the mean alpha over moved
 * pixels, so it does not depend on resolution or pan speed. Returns that mean
 * (the separation), the share of moved pixels that still show mostly old
 * content, the worst frame, its lag and every measured frame.
 */
export function pairScore(offFrames, onFrames, width, height, opts = {}) {
  const lagMax = opts.lagMax ?? LAG_MAX;
  const minStep = opts.minStep ?? PAIR_MIN_STEP;
  const ghostAlpha = opts.ghostAlpha ?? PAIR_GHOST_ALPHA;
  if (lagMax < 1) throw new Error(`lagMax ${lagMax} must be at least 1`);
  if (offFrames.length !== onFrames.length) {
    throw new Error(`pair frame count ${offFrames.length} vs ${onFrames.length}`);
  }
  if (offFrames.length < 2) throw new Error("pair needs at least 2 frames");
  const n = width * height;
  if (!Number.isInteger(n) || n <= 0) throw new Error(`bad frame size ${width}x${height}`);
  for (const [side, frames] of [["off", offFrames], ["on", onFrames]]) {
    frames.forEach((frame, i) => {
      if (frame.length !== n) {
        throw new Error(`${side} frame ${i}: ${frame.length} samples, expected ${n}`);
      }
    });
  }
  const measured = [];
  let movedMax = 0;
  let best = { score: 0, frame: -1, lag: 1, ghost_share: 0, moved_share: 0 };
  for (let t = 1; t < offFrames.length; t += 1) {
    const ref = offFrames[t];
    const before = offFrames[t - 1];
    const test = onFrames[t];
    let moved = 0;
    let sum = 0;
    let ghosts = 0;
    let frameAlpha = 0;
    let frameLag = 1;
    for (let p = 0; p < n; p += 1) {
      if (Math.abs(ref[p] - before[p]) < minStep) continue;
      moved += 1;
      let alpha = 0;
      let lag = 1;
      for (let k = 1; k <= Math.min(lagMax, t); k += 1) {
        const delta = ref[p] - offFrames[t - k][p];
        if (Math.abs(delta) < minStep) continue;
        const a = (ref[p] - test[p]) / delta;
        if (a > alpha) {
          alpha = a;
          lag = k;
        }
      }
      alpha = Math.min(Math.max(alpha, 0), 1);
      sum += alpha;
      if (alpha >= ghostAlpha) ghosts += 1;
      if (alpha > frameAlpha) {
        frameAlpha = alpha;
        frameLag = lag;
      }
    }
    const score = moved === 0 ? 0 : sum / moved;
    const ghostShare = moved === 0 ? 0 : ghosts / moved;
    const movedShare = moved / n;
    movedMax = Math.max(movedMax, movedShare);
    measured.push({
      index: t,
      lag: frameLag,
      score: round3(score),
      ghost_share: round3(ghostShare),
      moved_share: round3(movedShare),
    });
    if (score > best.score) {
      best = { score, frame: t, lag: frameLag, ghost_share: ghostShare, moved_share: movedShare };
    }
  }
  return {
    ...best,
    score: round3(best.score),
    ghost_share: round3(best.ghost_share),
    moved_share: round3(movedMax),
    pixels: n,
    measured,
  };
}

/** Read both capture directories, check they align by name and size, then score the pair. */
export function measurePair(offDir, onDir, opts = {}) {
  const off = readSequence(offDir);
  const on = readSequence(onDir);
  if (off.width !== on.width || off.height !== on.height) {
    throw new Error(`pair size ${off.width}x${off.height} vs ${on.width}x${on.height}`);
  }
  if (off.names.length !== on.names.length) {
    throw new Error(`pair frames ${off.names.length} vs ${on.names.length}`);
  }
  for (let i = 0; i < off.names.length; i += 1) {
    if (off.names[i] !== on.names[i]) {
      throw new Error(`pair misaligned at ${i}: ${off.names[i]} vs ${on.names[i]}`);
    }
  }
  const scored = pairScore(off.frames, on.frames, off.width, off.height, opts);
  return {
    ...scored,
    mode: "pair",
    separation: scored.score,
    off: offDir,
    on: onDir,
    width: off.width,
    height: off.height,
    frame_count: off.names.length,
    worst: scored.frame >= 0 ? off.names[scored.frame] : null,
  };
}

/**
 * The TAA twin with a frozen history: frame 0 is the current frame, each later
 * frame blends `blend` of the last resolved frame with `1 - blend` of the
 * current one and never reprojects it. This is the calibration ghost the pair
 * gate must separate from the clean reference; it is not production code.
 */
export function frozenTaa(frames, blend = 0.9) {
  if (frames.length === 0) throw new Error("no frames");
  if (!(blend >= 0 && blend <= 1)) throw new Error(`bad blend ${blend}`);
  const out = [frames[0].slice()];
  for (let t = 1; t < frames.length; t += 1) {
    const previous = out[t - 1];
    const current = frames[t];
    if (previous.length !== current.length) throw new Error(`frame ${t}: size mismatch`);
    const frame = new Uint8Array(current.length);
    for (let i = 0; i < current.length; i += 1) {
      frame[i] = Math.round(blend * previous[i] + (1 - blend) * current[i]);
    }
    out.push(frame);
  }
  return out;
}

/**
 * The calibration scene: a 20 px bright block sliding right by `step` px a
 * frame over a dark field. Exported so a test builds the same frames the gate
 * calibrates against.
 */
export function panFrame(width, height, start, opts = {}) {
  const value = opts.value ?? 220;
  const ground = opts.ground ?? 16;
  const block = opts.block ?? 20;
  const frame = new Uint8Array(width * height).fill(ground);
  for (let y = 0; y < height; y += 1) {
    for (let x = Math.max(start, 0); x < Math.min(start + block, width); x += 1) {
      frame[y * width + x] = value;
    }
  }
  return frame;
}

/** A pan of `count` frames, `step` px a frame; `staleAt` repeats the frame two back. */
export function panSequence(width, height, count, step = 4, staleAt = -1) {
  const frames = [];
  for (let k = 0; k < count; k += 1) frames.push(panFrame(width, height, 8 + step * k));
  if (staleAt >= 2 && staleAt < count) frames[staleAt] = frames[staleAt - 2].slice();
  return frames;
}

/** Selfcheck: a still sequence and a pan score 0, a stale frame scores 1, and the frozen pair separates. */
export function selfcheck(bar = DEFAULT_BAR) {
  const width = 64;
  const height = 48;
  const still = Array.from({ length: 6 }, () => panFrame(width, height, 8));
  const pan = panSequence(width, height, 6);
  const stale = panSequence(width, height, 6, 4, 3);
  const stillScore = ghostScore(still, width, height).score;
  const panScore = ghostScore(pan, width, height).score;
  const staleScore = ghostScore(stale, width, height);
  const frozen = frozenTaa(pan, 0.9);
  const pair = pairScore(pan, frozen, width, height);
  const cleanPair = pairScore(pan, pan, width, height);
  const ok =
    stillScore === 0 &&
    panScore === 0 &&
    staleScore.score > bar &&
    staleScore.lag === 2 &&
    cleanPair.score === 0 &&
    pair.score > PAIR_BAR;
  return {
    ok,
    bar,
    still: stillScore,
    pan: panScore,
    stale: staleScore.score,
    lag: staleScore.lag,
    pair: pair.score,
    pair_off: cleanPair.score,
    pair_bar: PAIR_BAR,
  };
}

export function usage() {
  return [
    "usage: node tools/temporal-gate.mjs [--bar N] [--json] <dir>",
    "       node tools/temporal-gate.mjs --pair [--bar N] [--json] <offdir> <ondir>",
    "       node tools/temporal-gate.mjs --selfcheck [--bar N]",
    "  Scores ghosting (0..1) over <dir>/frame_*.png, or the TAA-on capture",
    "  <ondir> against the TAA-off reference <offdir> (same size and names).",
    "  Exit 0 at or below the bar, 1 above it, 2 on a usage or decode error.",
  ].join("\n");
}

if (import.meta.main) {
  try {
    const args = process.argv.slice(2);
    let bar = null;
    let json = false;
    let pair = false;
    const at = args.indexOf("--bar");
    if (at !== -1) {
      if (args[at + 1] === undefined) throw new Error("--bar needs a number");
      bar = Number(args[at + 1]);
      if (!Number.isFinite(bar) || bar < 0) throw new Error(`bad bar ${args[at + 1]}`);
      args.splice(at, 2);
    }
    if (args.includes("--json")) {
      json = true;
      args.splice(args.indexOf("--json"), 1);
    }
    if (args.includes("--pair")) {
      pair = true;
      args.splice(args.indexOf("--pair"), 1);
    }
    if (args.includes("--help") || args.includes("-h")) {
      process.stdout.write(`${usage()}\n`);
      process.exit(0);
    }
    if (args.includes("--selfcheck")) {
      if (pair) throw new Error("--selfcheck takes no --pair");
      const r = selfcheck(bar ?? DEFAULT_BAR);
      process.stdout.write(`${JSON.stringify(r)}\n`);
      if (!r.ok) {
        process.stderr.write("selfcheck failed\n");
        process.exit(1);
      }
    } else if (pair) {
      if (args.length !== 2) throw new Error(usage());
      const limit = bar ?? PAIR_BAR;
      const r = measurePair(args[0], args[1]);
      if (json) {
        process.stdout.write(`${JSON.stringify({ ...r, bar: limit })}\n`);
      } else {
        const worst = r.worst ?? "(none)";
        process.stdout.write(
          `pair separation ${r.score.toFixed(3)} (bar ${limit.toFixed(3)}) frames ${r.frame_count} ` +
            `moved ${r.moved_share.toFixed(3)} ghost ${r.ghost_share.toFixed(3)} ` +
            `worst ${worst} lag ${r.lag}\n`,
        );
      }
      if (r.score > limit) process.exit(1);
    } else {
      if (args.length !== 1) throw new Error(usage());
      const limit = bar ?? DEFAULT_BAR;
      const r = measureDir(args[0]);
      if (json) {
        process.stdout.write(`${JSON.stringify(r)}\n`);
      } else {
        const worst = r.worst ?? "(none)";
        const best = r.best === null ? "-" : r.best.toFixed(3);
        process.stdout.write(
          `ghosting ${r.score.toFixed(3)} (bar ${limit.toFixed(3)}) frames ${r.frame_count} ` +
            `worst ${worst} lag ${r.lag} step ${r.step.toFixed(3)} best ${best}\n`,
        );
      }
      if (r.score > limit) process.exit(1);
    }
  } catch (error) {
    process.stderr.write(`${String((error && error.message) || error)}\n`);
    process.exit(2);
  }
}
