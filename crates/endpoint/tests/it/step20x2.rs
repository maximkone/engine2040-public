//! `play.step` 20x2-tick PNG receipt proof: twenty live steps of two ticks.
//!
//! Consumer: the vision playtester over `e2040-mcp`. Data owner for
//! simulation is `engine-arena`; the receipt decoder for the variance gate
//! is a test-side copy carried in this file, so production keeps one home for
//! the codec. Units are ticks, meters, pixels.
//! Determinism: same seed plus same tape twice gives the same digests and
//! the same PNG bytes. Errors are typed tool codes, asserted by name.

#![cfg(feature = "mcp")]

use engine_core::{adler::adler32, crc32::crc32};
use engine_endpoint::play::play_session::PlaySession;
use engine_endpoint::play::{PlayInputDto, PNG_MAGIC};

/// Receipt size used by this proof.
const W: u32 = 96;
/// Receipt size used by this proof.
const H: u32 = 64;
/// Test seed: fixed so the duel replays.
const SEED: u64 = 2040;

/// Walk a receipt PNG's chunks: IHDR dimensions (RGB8 only) plus the
/// concatenated IDAT. Anything else fails, never passes.
fn receipt_chunks(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if png.len() < PNG_MAGIC.len() || png[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err("receipt bytes are not a PNG (missing the 8-byte magic)".to_owned());
    }
    let mut pos = PNG_MAGIC.len();
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut idat = Vec::new();
    let mut ended = false;
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let tag = [png[pos + 4], png[pos + 5], png[pos + 6], png[pos + 7]];
        pos += 8;
        if pos + len + 4 > png.len() {
            return Err("receipt PNG chunk runs past the buffer end".to_owned());
        }
        let body = &png[pos..pos + len];
        let want = crc32(&png[pos - 4..pos + len]);
        let got = u32::from_be_bytes([
            png[pos + len],
            png[pos + len + 1],
            png[pos + len + 2],
            png[pos + len + 3],
        ]);
        if want != got {
            return Err("receipt PNG chunk fails its CRC".to_owned());
        }
        match &tag {
            b"IHDR" => {
                if len != 13 {
                    return Err("receipt IHDR has the wrong length".to_owned());
                }
                width = Some(u32::from_be_bytes([body[0], body[1], body[2], body[3]]));
                height = Some(u32::from_be_bytes([body[4], body[5], body[6], body[7]]));
                if body[8] != 8 || body[9] != 2 {
                    return Err("receipt PNG is not RGB8".to_owned());
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => {
                ended = true;
                break;
            }
            _ => {}
        }
        pos += len + 4;
    }
    if !ended {
        return Err("receipt PNG never ends (missing IEND)".to_owned());
    }
    let width = width.ok_or_else(|| "receipt PNG has no IHDR".to_owned())?;
    let height = height.ok_or_else(|| "receipt PNG has no IHDR".to_owned())?;
    Ok((width, height, idat))
}

/// Inflate a zlib stream whose deflate blocks are all stored, as our own
/// encoder writes them.
fn inflate_stored(idat: &[u8]) -> Result<Vec<u8>, String> {
    if idat.len() < 6 {
        return Err("receipt PNG IDAT is too short for zlib".to_owned());
    }
    if idat[0] != 0x78 || (u16::from(idat[0]) * 256 + u16::from(idat[1])) % 31 != 0 {
        return Err("receipt PNG IDAT is not a zlib stream".to_owned());
    }
    let mut raw = Vec::new();
    let mut cursor = 2;
    loop {
        if cursor + 5 > idat.len() {
            return Err("receipt PNG deflate block is truncated".to_owned());
        }
        let header = idat[cursor];
        let last = header & 1 == 1;
        if header & 0x06 != 0 {
            return Err("receipt PNG uses a compressed deflate block".to_owned());
        }
        cursor += 1;
        let len = u16::from_le_bytes([idat[cursor], idat[cursor + 1]]);
        let nlen = u16::from_le_bytes([idat[cursor + 2], idat[cursor + 3]]);
        if nlen != !len {
            return Err("receipt PNG stored block fails its length check".to_owned());
        }
        cursor += 4;
        let len = len as usize;
        if cursor + len + 4 > idat.len() {
            return Err("receipt PNG stored block runs past the stream".to_owned());
        }
        raw.extend_from_slice(&idat[cursor..cursor + len]);
        cursor += len;
        if last {
            let want = adler32(&raw);
            let got = u32::from_be_bytes([
                idat[cursor],
                idat[cursor + 1],
                idat[cursor + 2],
                idat[cursor + 3],
            ]);
            if want != got {
                return Err("receipt PNG fails its Adler check".to_owned());
            }
            break;
        }
    }
    Ok(raw)
}

/// Decode our own `play.step` receipts (RGB8, filter 0, stored deflate) for
/// the variance gate. Anything else fails, never passes. Test-side copy of
/// the session codec: `tests/` may carry its own helper, production keeps one
/// home for it.
fn decode_receipt_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let (width, height, idat) = receipt_chunks(png)?;
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err("receipt PNG dimensions are outside 1..=2048".to_owned());
    }
    let raw = inflate_stored(&idat)?;
    let w = width as usize;
    let h = height as usize;
    if raw.len() != h * (1 + w * 3) {
        return Err("receipt PNG scanlines do not match IHDR".to_owned());
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &raw[y * (1 + w * 3)..(y + 1) * (1 + w * 3)];
        if row[0] != 0 {
            return Err("receipt PNG uses a non-zero filter".to_owned());
        }
        rgb.extend_from_slice(&row[1..]);
    }
    Ok((width, height, rgb))
}

/// Pixel-variance gate: mean luma plus variance over per-pixel luma. Black
/// frames and flat fills fail, never pass.
fn variance_gate(rgb: &[u8]) -> Result<(u64, u64), String> {
    if !rgb.len().is_multiple_of(3) || rgb.is_empty() {
        return Err("capture holds no pixels; a step receipt must be non-black".to_owned());
    }
    let n = (rgb.len() / 3) as u64;
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    for pixel in rgb.chunks_exact(3) {
        let luma = (u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2])) / 3;
        sum += luma;
        sum_sq += luma * luma;
    }
    let mean = sum / n;
    let variance = sum_sq / n - mean * mean;
    if mean < 8 || variance < 100 {
        return Err(format!(
            "capture is black or flat (mean {mean}, variance {variance}); a step receipt must vary"
        ));
    }
    Ok((mean, variance))
}

/// One duel step's inputs: fighter 0 pushes +x, fighter 1 pushes -x, and
/// fighter 0 throws an attack every fourth step.
fn step_inputs(base: u32, step: u32) -> Vec<PlayInputDto> {
    let mut out = Vec::new();
    for tick in base..base + 2 {
        out.push(PlayInputDto {
            tick,
            fighter: 0,
            mx: 60,
            mz: 0,
            facing: 0,
            buttons: u8::from(step.is_multiple_of(4)),
        });
        out.push(PlayInputDto {
            tick,
            fighter: 1,
            mx: -60,
            mz: 0,
            facing: 0,
            buttons: 0,
        });
    }
    out
}

#[test]
fn play_step_twenty_by_two_pngs_decode_with_variance() {
    let mut session = PlaySession::new(SEED, 2, 20.0).expect("duel opens");
    let mut digests = Vec::with_capacity(20);
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(20);
    for step in 0..20 {
        let base = session.cursor();
        let out = session
            .advance(&step_inputs(base, step), 2, Some(W), Some(H))
            .expect("duel step advances");
        assert_eq!(out.tick, base + 2, "step {step}: receipt carries the count");
        assert_eq!(out.participants, 2, "step {step}: two fighters");
        assert_eq!(
            (out.manifest.width, out.manifest.height),
            (W, H),
            "step {step}: receipt covers 96x64"
        );
        assert_eq!(
            out.manifest.validation_errors, 0,
            "step {step}: receipt validates clean"
        );
        assert!(
            out.png_bytes.len() >= PNG_MAGIC.len() && out.png_bytes[..PNG_MAGIC.len()] == PNG_MAGIC,
            "step {step}: receipt starts with PNG magic"
        );
        let (w, h, rgb) = decode_receipt_png(&out.png_bytes).expect("every step PNG decodes");
        assert_eq!((w, h), (W, H), "step {step}: decoded size matches ask");
        let (mean, variance) =
            variance_gate(&rgb).expect("every step PNG passes the variance gate");
        eprintln!(
            "step {step}: tick {} digest {:016x} mean {mean} variance {variance} png {} bytes",
            out.tick,
            out.digest,
            out.png_bytes.len()
        );
        assert!(
            out.summary.contains(&format!("tick {}", out.tick)),
            "step {step}: summary names its tick"
        );
        digests.push(out.digest);
        frames.push(out.png_bytes);
    }
    assert_eq!(session.cursor(), 40, "20 steps of 2 ticks cover 0..40");
    assert_ne!(digests[0], digests[19], "the duel moves, so digests move");
    assert!(
        frames.windows(2).any(|pair| pair[0] != pair[1]),
        "consecutive receipts must not all render identical"
    );
}
