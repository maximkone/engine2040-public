//! health: asset floors. Bytes>=2048, nonZero>0.90 dense / 0.50 container.

/// Minimum byte size for a real asset (quarantine below).
pub(crate) const BYTE_FLOOR: u64 = 2048;
/// Minimum non-zero byte ratio for dense assets (mesh/pcm).
pub(crate) const NONZERO_DENSE: f32 = 0.90;
/// Minimum non-zero byte ratio for containers (glb/ktx).
pub(crate) const NONZERO_CONTAINER: f32 = 0.50;

/// Non-zero byte ratio in 0..=1 (0 on empty).
pub fn nonzero_ratio(bytes: &[u8]) -> f32 {
    if bytes.is_empty() {
        return 0.0;
    }
    let nz = bytes.iter().filter(|&&b| b != 0).count();
    nz as f32 / bytes.len() as f32
}

/// Floor verdicts: `(name, ok)` for byte floor + non-zero floor.
pub fn floors(bytes: &[u8], is_container: bool) -> [(&'static str, bool); 2] {
    let threshold = if is_container {
        NONZERO_CONTAINER
    } else {
        NONZERO_DENSE
    };
    [
        ("byteFloor2048", bytes.len() as u64 >= BYTE_FLOOR),
        ("nonZero", nonzero_ratio(bytes) >= threshold),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fails_both() {
        let f = floors(&[], false);
        assert!(f.iter().all(|(_, ok)| !ok));
    }

    #[test]
    fn small_dense_fails_byte_floor() {
        let bytes = vec![0xABu8; 128];
        let f = floors(&bytes, false);
        assert_eq!(f[0], ("byteFloor2048", false));
        assert_eq!(f[1], ("nonZero", true));
    }

    #[test]
    fn dense_and_container_thresholds() {
        let dense = vec![0xABu8; 4096];
        assert!(floors(&dense, false).iter().all(|(_, ok)| *ok));
        // half-zero: fails dense 0.90, passes container 0.50
        let mut half = vec![0xABu8; 2048];
        half.extend(vec![0u8; 2048]);
        assert_eq!(nonzero_ratio(&half), 0.5);
        assert!(!floors(&half, false)[1].1);
        assert!(floors(&half, true).iter().all(|(_, ok)| *ok));
    }

    #[test]
    fn deterministic() {
        let bytes = vec![7u8; 3000];
        assert_eq!(floors(&bytes, false), floors(&bytes, false));
    }
}
