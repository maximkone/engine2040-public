//! `play.step` 20x2-tick PNG receipt proof: twenty live steps of two ticks.
//!
//! Consumer: the vision playtester over `e2040-mcp`. Data owner for
//! simulation is `engine-arena`; this test owns only the decode and the
//! variance gate over the receipts. Units are ticks, meters, pixels.
//! Determinism: same seed plus same tape twice gives the same digests and
//! the same PNG bytes. Errors are typed tool codes, asserted by name.

#![cfg(feature = "mcp")]

use engine_endpoint::play::play_session::PlaySession;
use engine_endpoint::play::PlayInputDto;

/// PNG file magic: every receipt starts with these bytes.
const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// Receipt size used by this proof.
const W: u32 = 96;
/// Receipt size used by this proof.
const H: u32 = 64;
/// Test seed: fixed so the duel replays.
const SEED: u64 = 2040;
/// Software rasterizer pixel bound, mirrored from the endpoint.
const MAX_SOFTWARE_PIXELS: u64 = 512 * 512;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Decode the endpoint's own receipts (RGB8, filter 0, stored deflate).
/// Anything else fails as a receipt error, never success.
fn decode_png_rgb(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let bad = |detail: &str| detail.to_owned();
    if png.len() < PNG_MAGIC.len() || png[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err(bad(
            "receipt bytes are not a PNG (missing the 8-byte magic)",
        ));
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
            return Err(bad("receipt PNG chunk runs past the buffer end"));
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
            return Err(bad("receipt PNG chunk fails its CRC"));
        }
        match &tag {
            b"IHDR" => {
                if len != 13 {
                    return Err(bad("receipt IHDR has the wrong length"));
                }
                width = Some(u32::from_be_bytes([body[0], body[1], body[2], body[3]]));
                height = Some(u32::from_be_bytes([body[4], body[5], body[6], body[7]]));
                if body[8] != 8 || body[9] != 2 {
                    return Err(bad("receipt PNG is not RGB8"));
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
        return Err(bad("receipt PNG never ends (missing IEND)"));
    }
    let (width, height) = (
        width.ok_or_else(|| bad("receipt PNG has no IHDR"))?,
        height.ok_or_else(|| bad("receipt PNG has no IHDR"))?,
    );
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err(bad("receipt PNG dimensions are outside 1..=2048"));
    }
    if u64::from(width) * u64::from(height) > MAX_SOFTWARE_PIXELS * 16 {
        return Err(bad("receipt PNG dimensions overflow the decode bound"));
    }
    if idat.len() < 6 {
        return Err(bad("receipt PNG IDAT is too short for zlib"));
    }
    if idat[0] != 0x78 || (u16::from(idat[0]) * 256 + u16::from(idat[1])) % 31 != 0 {
        return Err(bad("receipt PNG IDAT is not a zlib stream"));
    }
    let mut raw = Vec::new();
    let mut cursor = 2;
    loop {
        if cursor + 5 > idat.len() {
            return Err(bad("receipt PNG deflate block is truncated"));
        }
        let header = idat[cursor];
        let last = header & 1 == 1;
        if header & 0x06 != 0 {
            return Err(bad("receipt PNG uses a compressed deflate block"));
        }
        cursor += 1;
        let len = u16::from_le_bytes([idat[cursor], idat[cursor + 1]]);
        let nlen = u16::from_le_bytes([idat[cursor + 2], idat[cursor + 3]]);
        if nlen != !len {
            return Err(bad("receipt PNG stored block fails its length check"));
        }
        cursor += 4;
        let len = len as usize;
        if cursor + len + 4 > idat.len() {
            return Err(bad("receipt PNG stored block runs past the stream"));
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
                return Err(bad("receipt PNG fails its Adler check"));
            }
            break;
        }
    }
    let w = width as usize;
    let h = height as usize;
    if raw.len() != h * (1 + w * 3) {
        return Err(bad("receipt PNG scanlines do not match IHDR"));
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &raw[y * (1 + w * 3)..(y + 1) * (1 + w * 3)];
        if row[0] != 0 {
            return Err(bad("receipt PNG uses a non-zero filter"));
        }
        rgb.extend_from_slice(&row[1..]);
    }
    Ok((width, height, rgb))
}

/// Pixel-variance gate: mean luma plus variance over per-pixel luma. Black
/// frames and flat fills fail, never pass.
fn variance_gate(rgb: &[u8]) -> Result<(u64, u64), String> {
    if rgb.len() % 3 != 0 || rgb.is_empty() {
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
            buttons: u8::from(step % 4 == 0),
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
        let (w, h, rgb) = decode_png_rgb(&out.png_bytes).expect("every step PNG decodes");
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
