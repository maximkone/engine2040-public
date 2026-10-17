//! Integer-cm 3D k-d tree: median-split build plus k-nearest and radius queries.
//! No non-test caller yet; every query is exercised by this module's tests.
/// One point in integer centimeters: `[x_cm, y_cm, z_cm]`.
pub type CmPoint = [i64; 3];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    point: usize,
    axis: u8,
    left: Option<usize>,
    right: Option<usize>,
}
/// Median-split k-d tree. Build once, query often.
#[derive(Debug, Clone, Default)]
pub struct KdTree {
    points: Vec<CmPoint>,
    nodes: Vec<Node>,
    root: Option<usize>,
}
impl KdTree {
    /// Build over `points`; empty gives empty tree.
    pub fn build(points: &[CmPoint]) -> Self {
        let mut tree = Self {
            points: points.to_vec(),
            nodes: Vec::with_capacity(points.len()),
            root: None,
        };
        let mut order: Vec<usize> = (0..points.len()).collect();
        tree.root = tree.split(&mut order, 0);
        tree
    }
    fn split(&mut self, order: &mut [usize], depth: usize) -> Option<usize> {
        if order.is_empty() {
            return None;
        }
        let axis: u8 = [0, 1, 2][depth % 3];
        let mid = order.len() / 2;
        order.select_nth_unstable_by(mid, |&a, &b| {
            (self.points[a][usize::from(axis)], a).cmp(&(self.points[b][usize::from(axis)], b))
        });
        let (left, rest) = order.split_at_mut(mid);
        let (median, right) = rest.split_at_mut(1);
        let point = median[0];
        let left = self.split(left, depth + 1);
        let right = self.split(right, depth + 1);
        self.nodes.push(Node {
            point,
            axis,
            left,
            right,
        });
        Some(self.nodes.len() - 1)
    }
    fn dist_sq(a: CmPoint, b: CmPoint) -> u128 {
        let mut sum: u128 = 0;
        for axis in 0..3 {
            let d = (i128::from(a[axis]) - i128::from(b[axis])).unsigned_abs();
            sum += d * d;
        }
        sum
    }
    /// The `k` nearest input indices to `q` by `(distance, index)`.
    pub fn knn(&self, q: CmPoint, k: usize) -> Vec<usize> {
        let mut best: Vec<(u128, usize)> = Vec::new();
        if k > 0 {
            if let Some(root) = self.root {
                self.search(root, q, k, &mut best);
            }
        }
        best.into_iter().map(|(_, i)| i).collect()
    }
    /// Every input index within `radius_cm` of `q`, ordered by `(distance, index)`.
    /// A non-positive radius or an empty tree gives an empty vector.
    pub fn radius(&self, q: CmPoint, radius_cm: i64) -> Vec<usize> {
        let mut found: Vec<(u128, usize)> = Vec::new();
        if radius_cm > 0 {
            if let Some(root) = self.root {
                let r = u128::from(radius_cm.unsigned_abs());
                self.collect(root, q, r * r, &mut found);
            }
        }
        found.sort_unstable();
        found.into_iter().map(|(_, i)| i).collect()
    }
    /// Collect in-radius points; a far child is skipped when the splitting-plane
    /// gap already exceeds the radius, and the near child is always visited
    /// because its points may sit on the query's side of that plane.
    fn collect(&self, slot: usize, q: CmPoint, radius_sq: u128, found: &mut Vec<(u128, usize)>) {
        let node = self.nodes[slot];
        let point = self.points[node.point];
        let here = Self::dist_sq(point, q);
        if here <= radius_sq {
            found.push((here, node.point));
        }
        let axis = usize::from(node.axis);
        let diff = i128::from(q[axis]) - i128::from(point[axis]);
        let (near, far) = if diff <= 0 {
            (node.left, node.right)
        } else {
            (node.right, node.left)
        };
        if let Some(next) = near {
            self.collect(next, q, radius_sq, found);
        }
        if diff.unsigned_abs() * diff.unsigned_abs() <= radius_sq {
            if let Some(next) = far {
                self.collect(next, q, radius_sq, found);
            }
        }
    }
    fn search(&self, slot: usize, q: CmPoint, k: usize, best: &mut Vec<(u128, usize)>) {
        let node = self.nodes[slot];
        let here = Self::dist_sq(self.points[node.point], q);
        if best.len() < k || (here, node.point) < *best.last().unwrap_or(&(u128::MAX, usize::MAX)) {
            let at = best.partition_point(|&(d, i)| (d, i) < (here, node.point));
            best.insert(at, (here, node.point));
            best.truncate(k);
        }
        let axis = usize::from(node.axis);
        let diff = i128::from(q[axis]) - i128::from(self.points[node.point][axis]);
        let (near, far) = if diff <= 0 {
            (node.left, node.right)
        } else {
            (node.right, node.left)
        };
        if let Some(next) = near {
            self.search(next, q, k, best);
        }
        let worst = if best.len() < k {
            u128::MAX
        } else {
            best.last().map_or(u128::MAX, |(d, _)| *d)
        };
        if diff.unsigned_abs() * diff.unsigned_abs() <= worst {
            if let Some(next) = far {
                self.search(next, q, k, best);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;
    fn pt(r: &mut Rng, s: i64) -> CmPoint {
        core::array::from_fn(|_| (r.next_u64() % (s as u64)) as i64)
    }
    #[test]
    fn knn_matches_brute_force_on_seeded_corpus() {
        let mut r = Rng::new(0x5EED_2040);
        let pts: Vec<CmPoint> = (0..500).map(|_| pt(&mut r, 4_000)).collect();
        let t = KdTree::build(&pts);
        let mut q = Rng::new(0xBEEF_2040);
        let mut n = 0;
        for k in [1, 3, 8, 64, 600] {
            for _ in 0..40 {
                let query = pt(&mut q, 4_000);
                let mut all: Vec<(u128, usize)> = pts
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| (KdTree::dist_sq(p, query), i))
                    .collect();
                all.sort_unstable();
                all.truncate(k.min(pts.len()));
                let want: Vec<usize> = all.into_iter().map(|(_, i)| i).collect();
                assert_eq!(t.knn(query, k), want, "k={k} q={query:?}");
                n += 1;
            }
        }
        assert_eq!(n, 200);
    }
    #[test]
    fn kdtree_radius_matches_brute_force_on_seeded_corpus() {
        let mut r = Rng::new(0x5EED_2040);
        let pts: Vec<CmPoint> = (0..500).map(|_| pt(&mut r, 4_000)).collect();
        let t = KdTree::build(&pts);
        let empty = KdTree::build(&[]);
        let mut q = Rng::new(0xFEED_2040);
        let mut n = 0;
        for radius_cm in [0_i64, 400, 8_000] {
            let cap = u128::from(radius_cm.unsigned_abs());
            let cap = cap * cap;
            for _ in 0..40 {
                let query = pt(&mut q, 4_000);
                let mut all: Vec<(u128, usize)> = pts
                    .iter()
                    .enumerate()
                    .filter(|(_, &p)| KdTree::dist_sq(p, query) <= cap)
                    .map(|(i, &p)| (KdTree::dist_sq(p, query), i))
                    .collect();
                all.sort_unstable();
                let want: Vec<usize> = all.iter().map(|&(_, i)| i).collect();
                let got = t.radius(query, radius_cm);
                assert_eq!(got, want, "radius={radius_cm} q={query:?}");
                // The result is ordered by (distance, index), never by input index.
                let pairs: Vec<(u128, usize)> = got
                    .iter()
                    .map(|&i| (KdTree::dist_sq(pts[i], query), i))
                    .collect();
                assert!(
                    pairs.windows(2).all(|w| w[0] <= w[1]),
                    "order for radius={radius_cm}"
                );
                assert_eq!(t.radius(query, radius_cm), got, "repeat run drifted");
                assert!(empty.radius(query, radius_cm).is_empty(), "empty tree");
                assert!(t.radius(query, 0).is_empty(), "radius 0 gives no points");
                assert!(t.radius(query, -7).is_empty(), "negative radius is empty");
                n += 1;
            }
        }
        assert_eq!(n, 120);
    }
    #[test]
    fn kdtree_10k_build_plus_1k_queries_replays_within_budget() {
        let mut r = Rng::new(0xC0FF_2040);
        let pts: Vec<CmPoint> = (0..10_000).map(|_| pt(&mut r, 100_000)).collect();
        let t = KdTree::build(&pts);
        assert_eq!(t.nodes.len(), 10_000, "one node per point");
        let root = t.root.expect("non-empty build has a root");
        let mut depth = 0;
        let mut stack = vec![(root, 1)];
        while let Some((slot, d)) = stack.pop() {
            depth = depth.max(d);
            let n = t.nodes[slot];
            if let Some(l) = n.left {
                stack.push((l, d + 1));
            }
            if let Some(rr) = n.right {
                stack.push((rr, d + 1));
            }
        }
        assert!(depth <= 32, "median split stays balanced, depth {depth}");
        let mut q = Rng::new(0xD1CE_2040);
        let queries: Vec<CmPoint> = (0..1_000).map(|_| pt(&mut q, 100_000)).collect();
        let first: Vec<Vec<usize>> = queries.iter().map(|&qq| t.knn(qq, 8)).collect();
        for (i, qq) in queries.iter().enumerate().step_by(100) {
            let mut all: Vec<(u128, usize)> = pts
                .iter()
                .enumerate()
                .map(|(k, &p)| (KdTree::dist_sq(p, *qq), k))
                .collect();
            all.sort_unstable();
            all.truncate(8);
            let want: Vec<usize> = all.into_iter().map(|(_, k)| k).collect();
            assert_eq!(first[i], want, "scale query {i} matches brute force");
        }
        let t2 = KdTree::build(&pts);
        let second: Vec<Vec<usize>> = queries.iter().map(|&qq| t2.knn(qq, 8)).collect();
        assert_eq!(first, second, "rebuild replays byte-identical");
        let third: Vec<Vec<usize>> = queries.iter().map(|&qq| t.knn(qq, 8)).collect();
        assert_eq!(first, third, "re-query replays byte-identical");
    }
    /// Acceptance probe: 10k build plus 1k k=8 queries stay under 500 ms
    /// total in release; minimum-of-11 so noise can only slow the reading.
    #[test]
    #[ignore = "timing: kdtree 10k budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn kdtree_10k_budget_total_and_replay() {
        let mut r = Rng::new(0xC0FF_2040);
        let pts: Vec<CmPoint> = (0..10_000).map(|_| pt(&mut r, 100_000)).collect();
        let mut q = Rng::new(0xD1CE_2040);
        let queries: Vec<CmPoint> = (0..1_000).map(|_| pt(&mut q, 100_000)).collect();
        let first = KdTree::build(&pts);
        let expect: Vec<Vec<usize>> = queries.iter().map(|&qq| first.knn(qq, 8)).collect();
        let mut best = u128::MAX;
        for _ in 0..11 {
            let start = std::time::Instant::now();
            let t = KdTree::build(&pts);
            let got: Vec<Vec<usize>> = queries.iter().map(|&qq| t.knn(qq, 8)).collect();
            let elapsed = start.elapsed().as_micros();
            assert_eq!(got, expect, "budget run drifted");
            best = best.min(elapsed);
        }
        eprintln!("kdtree budget: build+1k queries best_us={best} runs=11");
        assert!(best <= 500_000, "kdtree best {best} us over 500 ms");
    }
}
