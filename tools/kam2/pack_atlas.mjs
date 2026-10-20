// pack_atlas.mjs - ART0: PNG frame sequence -> one atlas PNG + manifest.json.
//
// Node 24 builtins only (node:zlib does PNG inflate/deflate). The manifest carries the
// fields docs/plan/kam2-style-lock.md fixes: id, kind, tile_w, tile_h, pivot, footprint,
// turret_anchor, facings, anims, atlas, mask_atlas, cameo.
//
// Usage from the repo root:
//   node tools/kam2/pack_atlas.mjs --in <frames dir> --out <dir> --id unit.swordsman --kind unit
//   node tools/kam2/pack_atlas.mjs --in <dir> --out <dir> --id house.farm --kind building ^
//       --facings 1 --anim idle:1:8:true --footprint 2x2 --pivot 32,61
//   node tools/kam2/pack_atlas.mjs --help
//
// Input PNGs are facing-major: facings files per animation frame, anims in --anim order.
// Cell layout in the atlas: column = facing, row = animation frame, 1 px transparent gutter.
//
// Exit codes: 0 ok - 1 pack failed - 2 usage.
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { inflateSync, deflateSync } from "node:zlib";

const FACINGS = ["S", "SW", "W", "NW", "N", "NE", "E", "SE"];
const MAX_ATLAS = 2048;

const HELP = `pack_atlas.mjs - pack a KAM2 PNG sequence into one atlas + manifest.json

Usage:
  node tools/kam2/pack_atlas.mjs --in <dir> --out <dir> --id <asset id> --kind <kind> [options]

Required:
  --in <dir>          folder of PNG frames (facing-major, sorted by number)
  --out <dir>         folder for <id>.atlas.png and manifest.json
  --id <id>           stable asset id, e.g. unit.swordsman, house.farm.stage2
  --kind <kind>       unit | building | tile | prop

Options:
  --facings <n>       facings in the set (default 8; buildings use 1)
  --tile <px>         cell width, must match the PNGs (default: PNG width)
  --tile-h <px>       cell height (default: PNG height)
  --pad <px>          transparent gutter around each cell (default 1)
  --pattern <regex>   only PNG names matching it are frames (default: every PNG)
  --anim <spec>       name:frames:fps:loop[:facings], repeatable (default idle:1:2:true)
  --pivot <x,y|auto>  ground anchor in cell px (default: frames.json pivot, else centre-bottom)
  --footprint <WxH>   tiles covered in the world (default 1x1)
  --turret-anchor <x,y>  turret pivot in cell px (default: none)
  --mask-dir <dir>    mask_NN.png set from render_frames.py --mask
  --cameo <file>      cameo PNG; copied beside the atlas, or packed when --cameo-cell
  --cameo-cell <WxH>  cameo cell in px (e.g. 96x72): write <id>.cameo.png with the gutter
                      and record {id, atlas, cell, size, padding, pivot} in the manifest
  --help              this text

Layout: column = facing (S, SW, W, NW, N, NE, E, SE), row = animation frame.
Max atlas side ${MAX_ATLAS} px (style lock). Exit codes: 0 ok - 1 pack failed - 2 usage.
`;

class PackError extends Error {}

function usage(message) {
  if (message) process.stderr.write(`E_PACK ${message}\n\n`);
  process.stderr.write(HELP);
  process.exit(2);
}

// ---------------------------------------------------------------------------
// PNG codec: 8-bit RGB/RGBA, non-interlaced (what Blender and this packer write).

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let c = 0xffffffff;
  for (let i = 0; i < buffer.length; i++) c = CRC_TABLE[(c ^ buffer[i]) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function decodePng(buffer, label) {
  if (buffer.length < 8 || buffer.readUInt32BE(0) !== 0x89504e47 || buffer.readUInt32BE(4) !== 0x0d0a1a0a) {
    throw new PackError(`not a PNG: ${label}`);
  }
  let pos = 8;
  let width = 0;
  let height = 0;
  let depth = 0;
  let colourType = 0;
  let interlace = 0;
  const idat = [];
  while (pos + 8 <= buffer.length) {
    const length = buffer.readUInt32BE(pos);
    const type = buffer.toString("latin1", pos + 4, pos + 8);
    const data = buffer.subarray(pos + 8, pos + 8 + length);
    pos += 12 + length;
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      depth = data[8];
      colourType = data[9];
      interlace = data[12];
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }
  }
  if (!width || !height) throw new PackError(`no IHDR: ${label}`);
  if (depth !== 8) throw new PackError(`only 8-bit PNG: ${label} (depth ${depth})`);
  if (interlace !== 0) throw new PackError(`interlaced PNG not supported: ${label}`);
  const channels = colourType === 6 ? 4 : colourType === 2 ? 3 : 0;
  if (channels === 0) throw new PackError(`only RGB/RGBA PNG: ${label} (colour type ${colourType})`);
  if (idat.length === 0) throw new PackError(`no IDAT: ${label}`);

  let raw;
  try {
    raw = inflateSync(Buffer.concat(idat));
  } catch (err) {
    throw new PackError(`bad IDAT in ${label}: ${err.message}`);
  }
  const stride = width * channels;
  if (raw.length < (stride + 1) * height) throw new PackError(`short pixel data: ${label}`);

  const out = Buffer.alloc(width * height * 4);
  const line = Buffer.alloc(stride);
  const prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? line[x - channels] : 0;
      const b = prev[x];
      const c = x >= channels ? prev[x - channels] : 0;
      let value = src[x];
      if (filter === 1) value += a;
      else if (filter === 2) value += b;
      else if (filter === 3) value += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        value += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) {
        throw new PackError(`unknown PNG filter ${filter} in ${label}`);
      }
      line[x] = value & 255;
    }
    for (let x = 0; x < width; x++) {
      const o = (y * width + x) * 4;
      const i = x * channels;
      out[o] = line[i];
      out[o + 1] = channels === 4 ? line[i + 1] : line[i];
      out[o + 2] = channels === 4 ? line[i + 2] : line[i];
      out[o + 3] = channels === 4 ? line[i + 3] : 255;
    }
    line.copy(prev);
  }
  return { width, height, pixels: out };
}

function encodePng(width, height, pixels) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const chunk = (type, data) => {
    const head = Buffer.alloc(8);
    head.writeUInt32BE(data.length, 0);
    head.write(type, 4, "latin1");
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, "latin1"), data])), 0);
    return Buffer.concat([head, data, crc]);
  };
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---------------------------------------------------------------------------
// Packing.

function naturalSort(names) {
  return names.sort((a, b) => a.localeCompare(b, "en", { numeric: true, sensitivity: "base" }));
}

function frameFiles(dir, stem, pattern) {
  const reserved = new Set([`${stem}.atlas.png`, `${stem}.mask.png`]);
  const names = readdirSync(dir)
    .filter((name) => name.toLowerCase().endsWith(".png"))
    .filter((name) => !name.toLowerCase().startsWith("mask_"))
    .filter((name) => !name.toLowerCase().endsWith(".atlas.png"))
    .filter((name) => !name.toLowerCase().endsWith(".mask.png"))
    .filter((name) => !reserved.has(name))
    .filter((name) => !pattern || pattern.test(name));
  return naturalSort(names);
}

function pair(value, label, fallback) {
  if (value === undefined || value === "auto") return fallback;
  const parts = value.split(",").map((part) => Number(part.trim()));
  if (parts.length !== 2 || parts.some((n) => !Number.isFinite(n) || n < 0)) {
    throw new PackError(`${label} must be "x,y" with non-negative numbers: ${value}`);
  }
  return [Math.round(parts[0]), Math.round(parts[1])];
}

function parseFrameSize(value, label) {
  if (value === undefined) return undefined;
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) throw new PackError(`${label} must be a positive integer: ${value}`);
  return n;
}

function parseFootprint(value) {
  if (value === undefined) return [1, 1];
  const match = /^(\d+)x(\d+)$/i.exec(value.trim());
  if (!match) throw new PackError(`footprint must be WxH in tiles: ${value}`);
  const w = Number(match[1]);
  const h = Number(match[2]);
  if (w < 1 || h < 1 || w > 16 || h > 16) throw new PackError(`footprint out of range: ${value}`);
  return [w, h];
}

function parseAnims(specs, facings) {
  const list = specs && specs.length ? specs : ["idle:1:2:true"];
  const anims = [];
  for (const spec of list) {
    const parts = spec.split(":");
    if (parts.length !== 4 && parts.length !== 5) {
      throw new PackError(`--anim wants name:frames:fps:loop[:facings]: ${spec}`);
    }
    const [name, frames, fps, loop, cols] = parts;
    if (!/^[a-z0-9_]+$/i.test(name)) throw new PackError(`anim name not [a-z0-9_]: ${name}`);
    const frameCount = Number(frames);
    const rate = Number(fps);
    if (!Number.isInteger(frameCount) || frameCount < 1) throw new PackError(`anim ${name}: frames must be >= 1`);
    if (!Number.isFinite(rate) || rate <= 0) throw new PackError(`anim ${name}: fps must be > 0`);
    if (!/^(true|false|1|0)$/i.test(loop)) throw new PackError(`anim ${name}: loop must be true or false`);
    let columns = null;
    if (cols !== undefined) {
      columns = Number(cols);
      if (!Number.isInteger(columns) || columns < 1) throw new PackError(`anim ${name}: facings must be a positive integer`);
      if (columns > facings) throw new PackError(`anim ${name}: ${columns} facings, the set has ${facings}`);
    }
    anims.push({ name, frames: frameCount, fps: rate, loop: /^(true|1)$/i.test(loop), facings: columns });
  }
  const names = new Set();
  for (const anim of anims) {
    if (names.has(anim.name)) throw new PackError(`anim repeated: ${anim.name}`);
    names.add(anim.name);
  }
  return anims;
}

function blit(destination, destinationWidth, source, sourceWidth, sourceHeight, x0, y0) {
  for (let y = 0; y < sourceHeight; y++) {
    const from = y * sourceWidth * 4;
    source.copy(destination, ((y0 + y) * destinationWidth + x0) * 4, from, from + sourceWidth * 4);
  }
}

function loadFrames(dir, names, label) {
  const images = [];
  for (const name of names) {
    const path = join(dir, name);
    if (!existsSync(path) || !statSync(path).isFile()) throw new PackError(`${label} frame not found: ${path}`);
    images.push({ name, ...decodePng(readFileSync(path), path) });
  }
  return images;
}

function main() {
  let values;
  try {
    ({ values } = parseArgs({
      options: {
        in: { type: "string" },
        out: { type: "string" },
        id: { type: "string" },
        kind: { type: "string" },
        facings: { type: "string" },
        tile: { type: "string" },
        "tile-h": { type: "string" },
        pad: { type: "string" },
        pattern: { type: "string" },
        anim: { type: "string", multiple: true },
        pivot: { type: "string" },
        footprint: { type: "string" },
        "turret-anchor": { type: "string" },
        "mask-dir": { type: "string" },
        cameo: { type: "string" },
        "cameo-cell": { type: "string" },
        help: { type: "boolean" },
      },
      allowPositionals: false,
      strict: true,
    }));
  } catch (err) {
    usage(err.message);
  }
  if (values.help) {
    process.stdout.write(HELP);
    return 0;
  }
  for (const key of ["in", "out", "id", "kind"]) {
    if (!values[key]) usage(`--${key} is required`);
  }

  try {
    const inDir = isAbsolute(values.in) ? values.in : resolve(values.in);
    const outDir = isAbsolute(values.out) ? values.out : resolve(values.out);
    if (!existsSync(inDir) || !statSync(inDir).isDirectory()) throw new PackError(`input folder not found: ${inDir}`);
    mkdirSync(outDir, { recursive: true });
    if (!/^[a-z0-9_]+(\.[a-z0-9_]+)*$/i.test(values.id)) throw new PackError(`id must be dotted words: ${values.id}`);
    if (!/^[a-z0-9_]+$/i.test(values.kind)) throw new PackError(`kind must be one word: ${values.kind}`);

    const facings = values.facings === undefined ? 8 : Number(values.facings);
    if (!Number.isInteger(facings) || facings < 1 || facings > 64) throw new PackError(`--facings must be 1..64`);
    const pad = values.pad === undefined ? 1 : Number(values.pad);
    if (!Number.isInteger(pad) || pad < 0 || pad > 8) throw new PackError(`--pad must be 0..8`);
    const anims = parseAnims(values.anim, facings);
    const footprint = parseFootprint(values.footprint);

    const stem = basename(values.id);
    let pattern = null;
    if (values.pattern !== undefined) {
      try {
        pattern = new RegExp(values.pattern, "i");
      } catch (err) {
        throw new PackError(`--pattern is not a valid regex: ${err.message}`);
      }
    }
    const names = frameFiles(inDir, stem, pattern);
    if (names.length === 0) throw new PackError(`no PNG frames in ${inDir}${values.pattern ? ` matching ${values.pattern}` : ""}`);
    const images = loadFrames(inDir, names, "sprite");

    const tileW = parseFrameSize(values.tile, "--tile") ?? images[0].width;
    const tileH = parseFrameSize(values["tile-h"], "--tile-h") ?? images[0].height;
    for (const image of images) {
      if (image.width !== tileW || image.height !== tileH) {
        throw new PackError(`${image.name} is ${image.width}x${image.height}, expected ${tileW}x${tileH}`);
      }
    }

    const totalRows = anims.reduce((sum, anim) => sum + anim.frames, 0);
    const expected = anims.reduce((sum, anim) => sum + anim.frames * (anim.facings ?? facings), 0);
    if (images.length !== expected) {
      const layout = anims.map((a) => `${a.name}=${a.frames}${a.facings ? `x${a.facings}` : ""}`).join(" ");
      throw new PackError(
        `${images.length} PNGs but ${expected} cells expected (${facings} facings, ${layout}); ` +
          `files are facing-major: ${facings} per animation frame unless the anim names its own count`,
      );
    }

    const cellW = tileW + pad * 2;
    const cellH = tileH + pad * 2;
    const atlasW = cellW * facings;
    const atlasH = cellH * totalRows;
    if (atlasW > MAX_ATLAS || atlasH > MAX_ATLAS) {
      throw new PackError(`atlas ${atlasW}x${atlasH} exceeds the ${MAX_ATLAS} px style-lock limit; split the asset`);
    }
    const atlasPixels = Buffer.alloc(atlasW * atlasH * 4);
    let cursor = 0;
    let image = 0;
    for (const anim of anims) {
      const columns = anim.facings ?? facings;
      anim.row = cursor;
      for (let frame = 0; frame < anim.frames; frame++) {
        const row = cursor + frame;
        for (let facing = 0; facing < columns; facing++) {
          blit(atlasPixels, atlasW, images[image].pixels, images[image].width, images[image].height, facing * cellW + pad, row * cellH + pad);
          image += 1;
        }
      }
      cursor += anim.frames;
    }

    // Pivot: frames.json from render_frames.py wins, else centre-bottom of the cell.
    let pivot = [Math.round(tileW / 2), tileH - 2];
    const metaPath = join(inDir, "frames.json");
    if (existsSync(metaPath)) {
      try {
        const meta = JSON.parse(readFileSync(metaPath, "utf8"));
        if (Array.isArray(meta.pivot) && meta.pivot.length === 2) pivot = [Number(meta.pivot[0]), Number(meta.pivot[1])];
        if (Number.isFinite(meta.count) && meta.count !== facings) {
          throw new PackError(`frames.json says ${meta.count} facings, --facings says ${facings}`);
        }
      } catch (err) {
        if (err instanceof PackError) throw err;
        process.stderr.write(`W_PACK frames.json ignored: ${err.message}\n`);
      }
    }
    pivot = pair(values.pivot, "--pivot", pivot);
    const turretAnchor = values["turret-anchor"] ? pair(values["turret-anchor"], "--turret-anchor", null) : null;

    let maskName = null;
    let maskBytes = 0;
    if (values["mask-dir"]) {
      const maskDir = isAbsolute(values["mask-dir"]) ? values["mask-dir"] : resolve(values["mask-dir"]);
      if (!existsSync(maskDir)) throw new PackError(`mask folder not found: ${maskDir}`);
      const maskNames = naturalSort(
        readdirSync(maskDir).filter((name) => /^mask_.*\.png$/i.test(name)),
      );
      if (maskNames.length !== images.length) {
        throw new PackError(`${maskNames.length} mask PNGs but ${images.length} sprite frames in ${maskDir}`);
      }
      const masks = loadFrames(maskDir, maskNames, "mask");
      const maskPixels = Buffer.alloc(atlasW * atlasH * 4);
      let maskImage = 0;
      for (const anim of anims) {
        const columns = anim.facings ?? facings;
        for (let frame = 0; frame < anim.frames; frame++) {
          const row = anim.row + frame;
          for (let facing = 0; facing < columns; facing++) {
            const image = masks[maskImage];
            blit(maskPixels, atlasW, image.pixels, image.width, image.height, facing * cellW + pad, row * cellH + pad);
            maskImage += 1;
          }
        }
      }
      maskName = `${stem}.mask.png`;
      const maskBuffer = encodePng(atlasW, atlasH, maskPixels);
      writeFileSync(join(outDir, maskName), maskBuffer);
      maskBytes = maskBuffer.length;
    }

    const facingNames =
      facings === FACINGS.length
        ? FACINGS.slice()
        : facings === 1
          ? ["S"]
          : Array.from({ length: facings }, (_, i) => `f${i}`);

    let cameoName = null;
    let cameo = null;
    if (values["cameo-cell"] && !values.cameo) throw new PackError("--cameo-cell needs --cameo");
    if (values.cameo) {
      const cameoPath = isAbsolute(values.cameo) ? values.cameo : resolve(values.cameo);
      if (!existsSync(cameoPath)) throw new PackError(`cameo not found: ${cameoPath}`);
      cameoName = basename(cameoPath);
      if (values["cameo-cell"]) {
        const match = /^(\d+)x(\d+)$/i.exec(values["cameo-cell"].trim());
        if (!match) throw new PackError(`--cameo-cell must be WxH: ${values["cameo-cell"]}`);
        const cellW = Number(match[1]);
        const cellH = Number(match[2]);
        if (cellW < 1 || cellH < 1 || cellW > MAX_ATLAS || cellH > MAX_ATLAS) {
          throw new PackError(`--cameo-cell out of range: ${values["cameo-cell"]}`);
        }
        const image = decodePng(readFileSync(cameoPath), cameoPath);
        if (image.width !== cellW || image.height !== cellH) {
          throw new PackError(`cameo is ${image.width}x${image.height}, --cameo-cell says ${cellW}x${cellH}`);
        }
        const cameoW = cellW + pad * 2;
        const cameoH = cellH + pad * 2;
        const cameoPixels = Buffer.alloc(cameoW * cameoH * 4);
        blit(cameoPixels, cameoW, image.pixels, image.width, image.height, pad, pad);
        cameoName = `${stem}.cameo.png`;
        writeFileSync(join(outDir, cameoName), encodePng(cameoW, cameoH, cameoPixels));
        let cameoPivot = [Math.round(cellW / 2), Math.round(cellH / 2)];
        const cameoMeta = join(dirname(cameoPath), "frames.json");
        if (existsSync(cameoMeta)) {
          try {
            const meta = JSON.parse(readFileSync(cameoMeta, "utf8"));
            if (Array.isArray(meta.pivot) && meta.pivot.length === 2) {
              cameoPivot = [Number(meta.pivot[0]), Number(meta.pivot[1])];
            }
          } catch (err) {
            process.stderr.write(`W_PACK cameo frames.json ignored: ${err.message}\n`);
          }
        }
        cameo = {
          id: `${values.id}.cameo`,
          atlas: cameoName,
          cell: [cellW, cellH],
          size: [cameoW, cameoH],
          padding: pad,
          pivot: cameoPivot,
        };
      } else {
        const target = join(outDir, cameoName);
        if (resolve(target) !== resolve(cameoPath)) copyFileSync(cameoPath, target);
      }
    }

    const atlasName = `${stem}.atlas.png`;
    const atlasBuffer = encodePng(atlasW, atlasH, atlasPixels);
    writeFileSync(join(outDir, atlasName), atlasBuffer);

    const manifest = {
      id: values.id,
      kind: values.kind,
      tile_w: tileW,
      tile_h: tileH,
      pivot,
      footprint,
      turret_anchor: turretAnchor,
      facings: facingNames,
      facing_count: facings,
      anims: anims.map((anim) => ({
        name: anim.name,
        frames: anim.frames,
        fps: anim.fps,
        loop: anim.loop,
        row: anim.row,
        ...(anim.facings ? { facings: anim.facings } : {}),
      })),
      atlas: atlasName,
      mask_atlas: maskName,
      cameo: cameo ?? cameoName,
      atlas_size: [atlasW, atlasH],
      padding: pad,
      source_frames: images.length,
      generator: "tools/kam2/pack_atlas.mjs",
    };
    const manifestPath = join(outDir, "manifest.json");
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

    process.stdout.write(
      `KAM2 PACK OK frames=${images.length} atlas=${atlasW}x${atlasH} bytes=${atlasBuffer.length}` +
        `${maskName ? ` mask=${maskBytes}` : ""}` +
        `${cameo ? ` cameo=${statSync(join(outDir, cameo.atlas)).size}` : ""} manifest=${manifestPath}\n`,
    );
    return 0;
  } catch (err) {
    if (err instanceof PackError) {
      process.stderr.write(`E_PACK ${err.message}\n`);
      return 1;
    }
    throw err;
  }
}

const direct = process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url;
if (direct) process.exitCode = main();

export { decodePng, encodePng };
