//! taa_pair: the deterministic TAA-on/TAA-off capture arms for ENG20.
//!
//! Purpose: write three aligned PNG sequences the offline gate
//! (`node tools/temporal-gate.mjs --pair`) can score: the TAA-off reference
//! (the frame the scene really holds each tick), the registered TAA-on run
//! (the real flow: it must reproduce the reference), and the frozen TAA-on
//! run (the regression the pair gate must catch). The frozen field is the
//! failing arm: the resolve reprojects nothing, so old block edge pixels stay
//! on screen where the current frame has moved on. A registered resolve
//! reproduces the reference exactly, which the `post::taa` tests pin; this
//! file captures both arms so the gate is calibrated on real twin output.
//!
//! Owner is the render lane (ENG20); consumer is the offline ghosting gate.
//! Units: 160x90 frames, 8-bit luma (the scene is grey), tick order. Files
//! land in `target/captures/taa-pair/{off,on,registered}`, ignored by git.
//! Determinism: the same scene gives the same PNG bytes twice; the test
//! asserts the second resolve equals the first before writing.
//!
//! Run: `cargo test -p engine-render --test it taa_pair::`.

use engine_render::post::{taa_resolve, MotionField, TaaParams};
use std::fs;
use std::path::{Path, PathBuf};

/// Frame width in pixels.
const W: u32 = 160;
/// Frame height in pixels.
const H: u32 = 90;
/// Frames in each sequence.
const FRAMES: u32 = 12;
/// Block left edge at frame 0.
const START: i32 = 48;
/// Pixels the block moves left per frame.
const STEP: i32 = 4;
/// Block width in pixels.
const BLOCK_W: i32 = 32;
/// Block height in pixels.
const BLOCK_H: i32 = 32;
/// Bright block value (HDR, written as 204).
const BRIGHT: f32 = 0.8;
/// Dark field value (HDR, written as 5).
const DARK: f32 = 0.02;

/// Frame `k` of the pan: a bright block sliding left over a dark field.
fn block_frame(k: u32) -> Vec<[f32; 3]> {
    let left = START - STEP * k as i32;
    let top = (H as i32 - BLOCK_H) / 2;
    let mut frame = vec![[DARK; 3]; (W * H) as usize];
    for y in 0..H {
        for x in 0..W {
            let inside_x = (x as i32) >= left && (x as i32) < left + BLOCK_W;
            let inside_y = (y as i32) >= top && (y as i32) < top + BLOCK_H;
            if inside_x && inside_y {
                frame[(y * W + x) as usize] = [BRIGHT; 3];
            }
        }
    }
    frame
}

/// The TAA-on arm over the pan with an explicit motion field. Frame 0 has no
/// history, so it follows the reference.
fn run(motion: &MotionField) -> Vec<Vec<[f32; 3]>> {
    let mut history = block_frame(0);
    let mut out = Vec::new();
    for k in 0..FRAMES {
        let current = block_frame(k);
        let params = if k == 0 {
            TaaParams::default().with_history(false)
        } else {
            TaaParams::default()
        };
        let resolved =
            taa_resolve(&current, &history, motion, W, H, &params).expect("resolve runs");
        history.clone_from(&resolved);
        out.push(resolved);
    }
    out
}

/// The frozen field: history samples its own texel, so nothing reprojects.
fn run_frozen() -> Vec<Vec<[f32; 3]>> {
    run(&MotionField::zero(W, H).expect("zero motion builds"))
}

/// The real flow of the left pan: sample the previous frame four pixels right.
fn run_registered() -> Vec<Vec<[f32; 3]>> {
    let flow = vec![[-(STEP as f32), 0.0]; (W * H) as usize];
    let motion = MotionField::build(W, H, flow).expect("pan motion builds");
    run(&motion)
}

/// One 0..255 byte from a linear HDR channel, the same mapping both arms use.
fn byte(value: f32) -> u8 {
    u8::try_from((value.clamp(0.0, 1.0) * 255.0).round() as i32).expect("0..255")
}

/// Write `frames` as grey RGB PNGs under `dir`.
fn write_sequence(dir: &Path, frames: &[Vec<[f32; 3]>]) {
    fs::create_dir_all(dir).expect("capture dir");
    for (k, frame) in frames.iter().enumerate() {
        let mut rgb = Vec::with_capacity((W * H * 3) as usize);
        for pixel in frame {
            let v = byte(pixel[0]);
            rgb.extend_from_slice(&[v, v, v]);
        }
        let path = dir.join(format!("frame_{k:06}.png"));
        image::save_buffer(&path, &rgb, W, H, image::ExtendedColorType::Rgb8)
            .expect("pair png writes");
    }
}

/// The capture root, next to the other local captures.
fn capture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/captures/taa-pair")
}

#[test]
fn taa_pair_capture() {
    let off = (0..FRAMES).map(block_frame).collect::<Vec<_>>();
    let on = run_frozen();
    let registered = run_registered();
    assert_eq!(
        registered, off,
        "a registered history reproduces the reference, or the gate cannot be zero on the good arm"
    );
    let last = (FRAMES - 1) as usize;
    assert_ne!(
        on[last], off[last],
        "a frozen history must change the frame, or the pair cannot separate"
    );
    // At frame 6 the left edge sits at x = 48 - 4*6 = 24. The reference
    // holds the bright block there; the frozen history still samples the old
    // dark field, so the on frame keeps the old content instead.
    let t = 6usize;
    let leading = ((H / 2) * W + (START - STEP * t as i32) as u32) as usize;
    assert!(
        off[t][leading][0] > 0.7,
        "the reference holds the bright edge"
    );
    assert!(
        on[t][leading][0] < 0.3,
        "the frozen history keeps the old dark edge"
    );
    assert_eq!(run_frozen(), on, "the capture is deterministic");
    assert_eq!(
        run_registered(),
        registered,
        "the registered capture is deterministic"
    );
    let root = capture_root();
    let _ = fs::remove_dir_all(&root);
    write_sequence(&root.join("off"), &off);
    write_sequence(&root.join("on"), &on);
    write_sequence(&root.join("registered"), &registered);
    assert!(
        root.join("on").join("frame_000011.png").is_file(),
        "the frozen and registered arms are written"
    );
}
