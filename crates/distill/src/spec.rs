//! Student shape and seed contract (std-only).
//!
//! Consumer: the tournament builds one [`StudentSpec`] per candidate, then calls
//! [`train`](crate::train) under the `train` feature. Owner: this module owns
//! the dim caps and the seed plumbing. Units: dims are element counts, the
//! seed is an opaque `u64` fed to the backend RNG. Determinism: the spec is
//! plain data; equal specs plus equal datasets train equal students on the
//! same machine (see crate docs for the scope). Errors: every bad dim maps
//! to [`SpecError`]; [`StudentSpec::new`] never panics.

/// Largest input or output dim: mirrors the runtime dense cap so any valid spec
/// exports to loadable `.e2nn` bytes.
pub const MAX_SPEC_DIM: usize = 1024;

/// Largest epoch count accepted by [`train`](crate::train).
pub const MAX_EPOCHS: u32 = 10_000;

/// Matrix shape of the tiny student: one dense layer, `input_dim` features
/// to `output_dim` action classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dims {
    /// Feature count per sample, `1..=MAX_SPEC_DIM`.
    pub input_dim: usize,
    /// Action-class count, `1..=MAX_SPEC_DIM`.
    pub output_dim: usize,
}

/// Shape plus training seed for one student.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudentSpec {
    /// Student matrix shape.
    pub dims: Dims,
    /// Backend RNG seed: weight init and any stochastic op draw from it.
    pub seed: u64,
}

/// Spec validation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecError {
    /// Input dim is zero or above [`MAX_SPEC_DIM`], culprit carried.
    BadInputDim(usize),
    /// Output dim is zero or above [`MAX_SPEC_DIM`], culprit carried.
    BadOutputDim(usize),
}

impl core::fmt::Display for SpecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpecError::BadInputDim(d) => write!(f, "input dim {d} outside 1..={MAX_SPEC_DIM}"),
            SpecError::BadOutputDim(d) => {
                write!(f, "output dim {d} outside 1..={MAX_SPEC_DIM}")
            }
        }
    }
}

impl std::error::Error for SpecError {}

impl StudentSpec {
    /// Validate dims and pack the spec. Never panics.
    pub fn new(input_dim: usize, output_dim: usize, seed: u64) -> Result<Self, SpecError> {
        if input_dim == 0 || input_dim > MAX_SPEC_DIM {
            return Err(SpecError::BadInputDim(input_dim));
        }
        if output_dim == 0 || output_dim > MAX_SPEC_DIM {
            return Err(SpecError::BadOutputDim(output_dim));
        }
        Ok(Self {
            dims: Dims {
                input_dim,
                output_dim,
            },
            seed,
        })
    }

    /// Flat weight count (`output_dim * input_dim`); fits `usize` because
    /// both dims are capped at [`MAX_SPEC_DIM`].
    pub fn weight_count(&self) -> usize {
        self.dims.output_dim * self.dims.input_dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_specs_pack() {
        let spec = StudentSpec::new(4, 3, 7).expect("small dims are legal");
        assert_eq!(spec.dims.input_dim, 4);
        assert_eq!(spec.dims.output_dim, 3);
        assert_eq!(spec.seed, 7);
        assert_eq!(spec.weight_count(), 12);
        StudentSpec::new(1, 1, 0).expect("unit dims are legal");
        StudentSpec::new(MAX_SPEC_DIM, MAX_SPEC_DIM, u64::MAX).expect("caps are legal");
    }

    #[test]
    fn bad_dims_rejected_with_cause() {
        assert_eq!(StudentSpec::new(0, 2, 0), Err(SpecError::BadInputDim(0)));
        assert_eq!(
            StudentSpec::new(MAX_SPEC_DIM + 1, 2, 0),
            Err(SpecError::BadInputDim(MAX_SPEC_DIM + 1))
        );
        assert_eq!(StudentSpec::new(2, 0, 0), Err(SpecError::BadOutputDim(0)));
        assert_eq!(
            StudentSpec::new(2, MAX_SPEC_DIM + 1, 0),
            Err(SpecError::BadOutputDim(MAX_SPEC_DIM + 1))
        );
    }
}
