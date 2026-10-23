//! mat4: the one row-major 4x4 product shared by animation, the cook and the
//! tools. Determinism: one fixed left-to-right sum order with no fused
//! multiply-add, so every caller that used to inline this keeps the same bits.

/// Row-major product: `out[row][col] = sum_k a[row][k] * b[k][col]`.
///
/// The expression order is fixed here; a caller's local copy must not be
/// reintroduced, because two copies can drift on the sum order.
#[must_use]
pub fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (row_index, row) in out.iter_mut().enumerate() {
        for (cell_index, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[row_index][k] * b[k][cell_index]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mul;

    const ID: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    #[test]
    fn identity_leaves_a_matrix_alone() {
        let m = [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ];
        assert_eq!(mul(&ID, &m), m);
        assert_eq!(mul(&m, &ID), m);
    }

    #[test]
    fn a_row_times_a_column_lands_in_its_own_cell() {
        let a = [
            [0.0, 0.0, 0.0, 0.0],
            [2.0, 3.0, 5.0, 7.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
        ];
        let b = [
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let out = mul(&a, &b);
        assert_eq!(out[1][2], 17.0);
        for (row, line) in out.iter().enumerate() {
            for (cell, value) in line.iter().enumerate() {
                if (row, cell) != (1, 2) {
                    assert_eq!(*value, 0.0, "row {row} cell {cell}");
                }
            }
        }
    }
}
