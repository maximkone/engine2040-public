//! ik: two-bone analytic + FABRIK fixed-iter + foot lock. Sample -> blend -> two-bone -> pelvis.

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = len(v);
    if l <= 1e-9 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

/// Two-bone analytic: root->mid->end lengths l1,l2, target t. Returns (mid,end).
/// Clamps unreachable targets to max reach; keeps current pose if degenerate.
pub fn two_bone(
    root: [f32; 3],
    mid: [f32; 3],
    end: [f32; 3],
    target: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let (l1, l2) = (len(sub(mid, root)), len(sub(end, mid)));
    if l1 <= 1e-9 || l2 <= 1e-9 {
        return (mid, end);
    }
    let dir = norm(sub(target, root));
    let mut d = len(sub(target, root));
    d = d.clamp((l1 - l2).abs() + 1e-6, l1 + l2 - 1e-6);
    // Law of cosines: angle at root between root->target and root->mid.
    let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    // Bend in the plane containing root, current mid, target.
    let to_mid = norm(sub(mid, root));
    // pole = component of to_mid perpendicular to dir
    let dot = to_mid[0] * dir[0] + to_mid[1] * dir[1] + to_mid[2] * dir[2];
    let mut pole = [
        to_mid[0] - dir[0] * dot,
        to_mid[1] - dir[1] * dot,
        to_mid[2] - dir[2] * dot,
    ];
    if len(pole) <= 1e-6 {
        pole = if dir[1].abs() < 0.99 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let dot2 = pole[0] * dir[0] + pole[1] * dir[1] + pole[2] * dir[2];
        pole = [
            pole[0] - dir[0] * dot2,
            pole[1] - dir[1] * dot2,
            pole[2] - dir[2] * dot2,
        ];
    }
    pole = norm(pole);
    let mid_p = [
        root[0] + dir[0] * (cos_a * l1) + pole[0] * (sin_a * l1),
        root[1] + dir[1] * (cos_a * l1) + pole[1] * (sin_a * l1),
        root[2] + dir[2] * (cos_a * l1) + pole[2] * (sin_a * l1),
    ];
    let end_p = [
        root[0] + dir[0] * d,
        root[1] + dir[1] * d,
        root[2] + dir[2] * d,
    ];
    (mid_p, end_p)
}

/// FABRIK fixed-iter on a joint chain. joints mutated toward target.
/// Returns iterations used (<= max_iter). Deterministic, no alloc.
pub fn fabrik(joints: &mut [[f32; 3]], target: [f32; 3], max_iter: u32, tol: f32) -> u32 {
    if joints.len() < 2 {
        return 0;
    }
    let n = joints.len();
    let root = joints[0];
    let mut lens = [0.0f32; 16];
    let m = (n - 1).min(16);
    let mut total = 0.0;
    for i in 0..m {
        lens[i] = len(sub(joints[i + 1], joints[i]));
        total += lens[i];
    }
    if total <= 1e-9 {
        return 0;
    }
    let mut used = 0;
    for it in 0..max_iter.min(64) {
        used = it + 1;
        // forward: end -> target
        joints[n - 1] = target;
        for i in (0..n - 1).rev() {
            let l = if i < 16 {
                lens[i]
            } else {
                len(sub(joints[i + 1], joints[i]))
            };
            let d = sub(joints[i + 1], joints[i]);
            let dl = len(d);
            if dl > 1e-9 {
                let f = l / dl;
                joints[i] = [
                    joints[i + 1][0] - d[0] * f,
                    joints[i + 1][1] - d[1] * f,
                    joints[i + 1][2] - d[2] * f,
                ];
            }
        }
        // backward: root pinned
        joints[0] = root;
        for i in 0..n - 1 {
            let l = if i < 16 {
                lens[i]
            } else {
                len(sub(joints[i + 1], joints[i]))
            };
            let d = sub(joints[i + 1], joints[i]);
            let dl = len(d);
            if dl > 1e-9 {
                let f = l / dl;
                joints[i + 1] = [
                    joints[i][0] + d[0] * f,
                    joints[i][1] + d[1] * f,
                    joints[i][2] + d[2] * f,
                ];
            }
        }
        if len(sub(joints[n - 1], target)) <= tol {
            break;
        }
    }
    used
}

/// Foot lock: blend foot toward locked pos when within lock_radius, full lock inside.
/// Returns locked foot pos. Deterministic.
pub fn foot_lock(current: [f32; 3], locked: [f32; 3], lock_radius: f32) -> [f32; 3] {
    let d = len(sub(current, locked));
    if d <= 1e-9 || d >= lock_radius {
        return current;
    }
    let f = 1.0 - d / lock_radius; // 1 at lock point
    [
        current[0] + (locked[0] - current[0]) * f,
        current[1] + (locked[1] - current[1]) * f,
        current[2] + (locked[2] - current[2]) * f,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bone_reaches() {
        let (mid, end) = two_bone(
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [1.0, -1.0, 0.0],
        );
        let d = len(sub(end, [1.0, -1.0, 0.0]));
        assert!(d < 0.05, "end {end:?} d={d}");
        assert!((len(sub(mid, [0.0, 0.0, 0.0])) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn two_bone_clamps_unreachable() {
        let (_, end) = two_bone(
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, -99.0, 0.0],
        );
        assert!((len(sub(end, [0.0, 0.0, 0.0])) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn fabrik_converges_capped() {
        let mut j = [
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, -3.0, 0.0],
        ];
        let used = fabrik(&mut j, [1.0, -2.0, 0.0], 16, 1e-4);
        assert!(used <= 16);
        assert!(len(sub(j[3], [1.0, -2.0, 0.0])) < 0.05, "{j:?}");
        assert_eq!(j[0], [0.0, 0.0, 0.0]); // root pinned
    }

    #[test]
    fn foot_lock_holds() {
        let l = [1.0, 0.0, 0.0];
        assert_eq!(foot_lock([5.0, 0.0, 0.0], l, 1.0), [5.0, 0.0, 0.0]); // outside
        let near = foot_lock([1.3, 0.0, 0.0], l, 1.0);
        assert!(len(sub(near, l)) < 0.3, "{near:?}");
    }
}
