//! hpa parity beside flat astar plus timing shape (ignored, perf gate only).
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing test reads wall clock; never enters tick"
)]
use engine_nav::{astar, hpa};
use std::time::Instant;
/// Ledger fixture: 64x64 walled-door maze.
fn ledger() -> (Vec<bool>, usize, usize) {
    let (w, h) = (64usize, 64usize);
    let mut b = vec![false; w * h];
    for y in 0..h {
        if y != 8 {
            b[y * w + 16] = true;
        }
        if y != 40 {
            b[y * w + 32] = true;
        }
        if y != 24 {
            b[y * w + 48] = true;
        }
    }
    (b, w, h)
}
#[test]
fn ledger_parity_and_determinism() {
    let (b, w, h) = ledger();
    let (s, g) = ((0usize, 0usize), (63usize, 63usize));
    let flat = astar(&b, w, h, s, g).expect("ledger connects");
    let chunked = hpa(&b, w, h, s, g).expect("hpa connects");
    assert_eq!(chunked.len(), flat.len(), "length parity");
    assert_eq!(hpa(&b, w, h, s, g), Some(chunked.clone()), "determinism x2");
    assert_eq!((chunked[0], chunked[chunked.len() - 1]), (s, g));
    eprintln!("ledger flat={} chunked={}", flat.len(), chunked.len());
}
#[test]
fn corpus_parity_over_seeds() {
    for seed in 0..12usize {
        let (w, h) = (32usize, 32usize);
        let mut b = vec![false; w * h];
        for y in 0..h {
            if y != (seed * 7 + 3) % h {
                b[y * w + 10] = true;
            }
            if y != (seed * 11 + 13) % h {
                b[y * w + 21] = true;
            }
        }
        let (s, g) = ((0usize, 0usize), (31usize, 31usize));
        let flat = astar(&b, w, h, s, g);
        let chunked = hpa(&b, w, h, s, g);
        assert_eq!(
            chunked.is_some(),
            flat.is_some(),
            "sealed parity seed {seed}"
        );
        if let (Some(f), Some(c)) = (flat, chunked) {
            assert_eq!(c.len(), f.len(), "length parity seed {seed}");
        }
    }
}
#[test]
fn refusals_match_flat() {
    let (b, w, h) = ledger();
    assert_eq!(hpa(&b, w, h, (0, 0), (64, 64)), None);
    assert_eq!(hpa(&b, w, h, (16, 0), (63, 63)), None);
    let mut shut = vec![false; 8 * 8];
    for y in 0..8 {
        shut[y * 8 + 4] = true;
    }
    assert_eq!(hpa(&shut, 8, 8, (0, 0), (7, 7)), None);
    assert_eq!(astar(&shut, 8, 8, (0, 0), (7, 7)), None);
}
#[test]
#[ignore = "timing: release perf gate measures chunked-faster-than-flat shape"]
fn timing_chunked_faster_than_flat() {
    let (b, w, h) = ledger();
    let (s, g) = ((0usize, 0usize), (63usize, 63usize));
    for _ in 0..3 {
        let _ = astar(&b, w, h, s, g);
        let _ = hpa(&b, w, h, s, g);
    }
    let reps = 20usize;
    let t0 = Instant::now();
    for _ in 0..reps {
        let _ = astar(&b, w, h, s, g);
    }
    let flat = t0.elapsed();
    let t1 = Instant::now();
    for _ in 0..reps {
        let _ = hpa(&b, w, h, s, g);
    }
    let chunked = t1.elapsed();
    eprintln!("shape flat={flat:?} chunked={chunked:?} reps={reps}");
    assert!(
        chunked < flat,
        "chunked faster than flat: {chunked:?} vs {flat:?}"
    );
}
