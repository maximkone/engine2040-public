use engine_physics::solver::Solver;
use engine_physics::Body;

#[test]
fn probe_ten_box_orig() {
    let mut s = Solver::new();
    let mut b: Vec<Body> = (0..10)
        .map(|i| Body {
            x: 0.0,
            y: 0.5 + i as f32 * 1.02,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        })
        .collect();
    for k in 0..12 {
        for _ in 0..150 {
            s.step(&mut b, 1.0 / 60.0);
        }
        let mut pen = (b[0].hy - b[0].y).max(0.0);
        for w in b.windows(2) {
            pen = pen.max(-((w[1].y - w[0].y) - (w[0].hy + w[1].hy)));
        }
        eprintln!(
            "step={} b0y={:.4} b9y={:.4} maxpen={:.4} sleep={}",
            (k + 1) * 150,
            b[0].y,
            b[9].y,
            pen,
            s.sleeping_count()
        );
    }
}
