// pe-exports.mjs - GS8 PE export table reader (no dependency).
// Parses the export directory of a Windows PE file and prints one name
// per line. Used to check adapter exports on hybrid laptops (GS7).
import { readFileSync } from 'node:fs';

function fail(msg) {
  throw new Error('pe-exports: ' + msg);
}

/** Export names of a PE image in `buf`. Throws when it is not a PE file. */
export function parseExports(buf) {
  if (buf.length < 64 || buf[0] !== 0x4d || buf[1] !== 0x5a) {
    fail('not a PE file (missing MZ)');
  }
  const eLfanew = buf.readUInt32LE(0x3c);
  if (eLfanew + 6 > buf.length || buf.readUInt32LE(eLfanew) !== 0x00004550) {
    fail('not a PE file (missing PE sig)');
  }
  const coff = eLfanew + 4;
  const numSections = buf.readUInt16LE(coff + 2);
  const sizeOpt = buf.readUInt16LE(coff + 16);
  const opt = coff + 20;
  if (opt + sizeOpt > buf.length) {
    fail('not a PE file (bad optional header)');
  }
  const magic = buf.readUInt16LE(opt);
  let numDirs = 0;
  let dirBase = 0;
  if (magic === 0x10b) {
    numDirs = buf.readUInt32LE(opt + 92);
    dirBase = opt + 96;
  } else if (magic === 0x20b) {
    numDirs = buf.readUInt32LE(opt + 108);
    dirBase = opt + 112;
  } else {
    fail('not a PE file (bad optional magic)');
  }
  if (numDirs < 1) {
    return [];
  }
  const expRva = buf.readUInt32LE(dirBase);
  const expSize = buf.readUInt32LE(dirBase + 4);
  if (expRva === 0 || expSize === 0) {
    return [];
  }
  const secBase = opt + sizeOpt;
  const sections = [];
  for (let i = 0; i < numSections; i += 1) {
    const s = secBase + i * 40;
    if (s + 40 > buf.length) {
      fail('not a PE file (bad section)');
    }
    sections.push({
      vaddr: buf.readUInt32LE(s + 12),
      vsize: buf.readUInt32LE(s + 8),
      rawSize: buf.readUInt32LE(s + 16),
      rawPtr: buf.readUInt32LE(s + 20),
    });
  }
  const toOff = function (rva) {
    for (const s of sections) {
      const span = Math.max(s.vsize, s.rawSize);
      if (rva >= s.vaddr && rva < s.vaddr + span) {
        const off = s.rawPtr + (rva - s.vaddr);
        if (off >= buf.length) {
          fail('not a PE file (RVA past end)');
        }
        return off;
      }
    }
    fail('not a PE file (RVA with no section)');
    return -1;
  };
  const dirOff = toOff(expRva);
  if (dirOff + 40 > buf.length) {
    fail('not a PE file (bad export dir)');
  }
  const numNames = buf.readUInt32LE(dirOff + 24);
  const addrNames = buf.readUInt32LE(dirOff + 32);
  if (numNames === 0) {
    return [];
  }
  if (numNames > 100000) {
    fail('not a PE file (bad name count)');
  }
  const namesOff = toOff(addrNames);
  const out = [];
  for (let i = 0; i < numNames; i += 1) {
    const ptrOff = namesOff + i * 4;
    if (ptrOff + 4 > buf.length) {
      fail('not a PE file (bad name table)');
    }
    const strOff = toOff(buf.readUInt32LE(ptrOff));
    let end = strOff;
    while (end < buf.length && buf[end] !== 0) {
      end += 1;
    }
    if (end >= buf.length) {
      fail('not a PE file (unterminated name)');
    }
    out.push(buf.toString('ascii', strOff, end));
  }
  return out;
}

/** Export names of the file at `filePath`. Throws when it is not a PE file. */
export function listExports(filePath) {
  const buf = readFileSync(filePath);
  return parseExports(buf);
}

const isMain = import.meta.main;
if (isMain) {
  const arg = process.argv[2];
  if (!arg) {
    process.stderr.write('usage: node tools/pe-exports.mjs <exe|dll>\n');
    process.exitCode = 2;
  } else {
    try {
      for (const name of listExports(arg)) {
        process.stdout.write(name + '\n');
      }
    } catch (err) {
      process.stderr.write((err && err.message ? err.message : String(err)) + '\n');
      process.exitCode = 2;
    }
  }
}
