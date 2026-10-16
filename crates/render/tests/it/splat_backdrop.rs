#[path = "../../src/splat.rs"]
mod splat;
use splat::{LicenseRow, SplatBackdrop};

fn license() -> LicenseRow {
    LicenseRow {
        source: "fixture-local",
        terms: "test-only",
        capture: "synthetic-10k",
    }
}

fn make_ply(n: usize) -> Vec<u8> {
    let head = format!("ply\nformat binary_little_endian 1.0\nelement vertex {n}\nend_header\n");
    let mut out = head.into_bytes();
    for i in 0..n {
        let px = (i * 37 % 60) as f32 / 10.0 - 3.0;
        let py = (i * 53 % 60) as f32 / 10.0 - 3.0;
        let pz = 5.0 + (i * 29 % 40) as f32 / 10.0;
        let sx = 0.2 + (i * 11 % 6) as f32 / 10.0;
        let qz = (i * 17 % 20) as f32 / 10.0 - 1.0;
        let rr = (i * 23 % 20) as f32 / 10.0 - 1.0;
        for v in [px, py, pz, sx, sx, sx, 0.0, 0.0, qz, 1.0, rr, 0.0, 0.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[test]
fn spike_cook_sort_checksum() {
    let ply = make_ply(10_000);
    let bd = SplatBackdrop::try_new(Some(license()), &ply).unwrap();
    assert_eq!(bd.cooked_len(), 10_000 * 5);
    assert_eq!(bd.capture(), "synthetic-10k");
    let ratio = bd.cooked_len() as f64 / ply.len() as f64;
    assert!(ratio <= 0.10, "cook ratio {ratio}");
    let (w, h) = (64u32, 64u32);
    let mut arena = vec![100.0f32; (w * h) as usize];
    for y in 24..40 {
        for x in 24..40 {
            arena[(y * w + x) as usize] = 2.0;
        }
    }
    let mut t1 = vec![0u8; (w * h * 3) as usize];
    let mut t2 = vec![0u8; (w * h * 3) as usize];
    bd.composite(&mut t1, &arena, w, h).unwrap();
    bd.composite(&mut t2, &arena, w, h).unwrap();
    assert_eq!(t1, t2);
    assert_eq!(engine_core::hash_bytes(&t1), engine_core::hash_bytes(&t2));
    for y in 24..40 {
        for x in 24..40 {
            let o = ((y * w + x) * 3) as usize;
            assert_eq!(&t1[o..o + 3], &[16, 32, 48]);
        }
    }
    let mut painted = 0;
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 3) as usize;
            let outside = x < 24 || x >= 40 || y < 24 || y >= 40;
            let kept = t1[o] == 16 && t1[o + 1] == 32 && t1[o + 2] == 48;
            if outside && !kept {
                painted += 1;
            }
        }
    }
    assert!(painted > 0, "backdrop wrote nothing outside");
}

#[test]
fn missing_license_refuses() {
    let ply = make_ply(8);
    assert!(SplatBackdrop::try_new(None, &ply).is_err());
    let mut row = license();
    row.source = "";
    assert!(SplatBackdrop::try_new(Some(row), &ply).is_err());
}
