//! overlap: AABB query home. Canon home of the AABB test.

use super::Body;

/// Ids of bodies whose AABB intersects `(x, y, hx, hy)`, sorted.
pub fn overlap_query(bodies: &[Body], x: f32, y: f32, hx: f32, hy: f32) -> Vec<usize> {
    bodies
        .iter()
        .enumerate()
        .filter(|(_, b)| (b.x - x).abs() < b.hx + hx && (b.y - y).abs() < b.hy + hy)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::Space;
    use super::*;

    fn body(x: f32, y: f32, vx: f32, vy: f32) -> Body {
        Body {
            x,
            y,
            vx,
            vy,
            hx: 0.5,
            hy: 0.5,
        }
    }

    #[test]
    fn gate_50_50_60_60_deterministic_count() {
        let mut s = Space::new();
        s.add(body(0.0, 0.0, 0.0, 0.0));
        s.add(body(50.0, 50.0, 1.0, 0.0));
        s.add(body(200.0, 200.0, 0.0, 0.0));
        let (a, b) = (
            s.overlap(50.0, 50.0, 60.0, 60.0),
            s.overlap(50.0, 50.0, 60.0, 60.0),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 2, "{a:?}");
        assert_eq!(a, vec![0, 1]);
    }
}
