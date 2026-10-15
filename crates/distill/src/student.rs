//! The tiny student: one dense layer of plain floats (std-only).
//!
//! Consumer: the tournament reads [`Student::weights`] for receipts and hands
//! [`export`](crate::export) bytes to the runtime. Owner: this module owns the
//! float parameter layout and the std reference forward. Units: weights and
//! biases are feature-unit floats; [`Student::forward_f32`] takes raw
//! features, [`Student::forward_quantized`] takes runtime `i8` inputs.
//! Determinism: the forward is a fixed-order accumulation with no threads.
//! Errors: shape mismatches give `None`/typed errors, never panics.

use crate::data::INPUT_QUANT_SCALE;
use crate::spec::StudentSpec;

/// One trained dense layer: `logits[o] = bias[o] + sum(weights[o][i] * x[i])`.
/// Weights are row-major `output_dim x input_dim`.
#[derive(Clone, Debug, PartialEq)]
pub struct Student {
    /// Shape and seed this student trained under.
    pub spec: StudentSpec,
    /// Row-major weights, `output_dim * input_dim` values.
    pub weights: Vec<f32>,
    /// Per-class bias, `output_dim` values.
    pub bias: Vec<f32>,
    /// Epochs run to produce these params (0 for hand-built students).
    pub epochs_trained: u32,
}

/// Student construction failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudentError {
    /// Weight count differs from `output_dim * input_dim`; carries
    /// `(got, want)`.
    BadWeightLen {
        /// Weights supplied.
        got: usize,
        /// Weights the spec needs.
        want: usize,
    },
    /// Bias count differs from `output_dim`; carries `(got, want)`.
    BadBiasLen {
        /// Biases supplied.
        got: usize,
        /// Biases the spec needs.
        want: usize,
    },
    /// A param is NaN or infinite.
    NonFiniteParam,
}

impl core::fmt::Display for StudentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StudentError::BadWeightLen { got, want } => {
                write!(f, "weight len {got}, want {want}")
            }
            StudentError::BadBiasLen { got, want } => {
                write!(f, "bias len {got}, want {want}")
            }
            StudentError::NonFiniteParam => f.write_str("param is non-finite"),
        }
    }
}

impl std::error::Error for StudentError {}

impl Student {
    /// Pack validated params. `epochs_trained` is 0 for hand-built students.
    /// Never panics.
    pub fn new(
        spec: StudentSpec,
        weights: Vec<f32>,
        bias: Vec<f32>,
        epochs_trained: u32,
    ) -> Result<Self, StudentError> {
        let want_w = spec.weight_count();
        if weights.len() != want_w {
            return Err(StudentError::BadWeightLen {
                got: weights.len(),
                want: want_w,
            });
        }
        if bias.len() != spec.dims.output_dim {
            return Err(StudentError::BadBiasLen {
                got: bias.len(),
                want: spec.dims.output_dim,
            });
        }
        if !weights.iter().chain(bias.iter()).all(|v| v.is_finite()) {
            return Err(StudentError::NonFiniteParam);
        }
        Ok(Self {
            spec,
            weights,
            bias,
            epochs_trained,
        })
    }

    /// All-zero params under the spec: the no-knowledge baseline the
    /// training test must beat.
    pub fn zeros(spec: StudentSpec) -> Self {
        Self {
            weights: vec![0.0; spec.weight_count()],
            bias: vec![0.0; spec.dims.output_dim],
            spec,
            epochs_trained: 0,
        }
    }

    /// Reference forward on raw features: fixed-order accumulation, no
    /// threads. Returns `None` on a short input instead of panicking.
    pub fn forward_f32(&self, features: &[f32]) -> Option<Vec<f32>> {
        if features.len() != self.spec.dims.input_dim {
            return None;
        }
        let mut out = self.bias.clone();
        for (o, slot) in out.iter_mut().enumerate() {
            let row = o * self.spec.dims.input_dim;
            let mut acc = *slot;
            for (i, v) in features.iter().enumerate() {
                acc += self.weights[row + i] * v;
            }
            *slot = acc;
        }
        Some(out)
    }

    /// Reference forward on runtime `i8` inputs (see
    /// [`quantize_input`](crate::quantize_input)): dequantizes one step to
    /// `1 / INPUT_QUANT_SCALE` units, then runs [`forward_f32`](Self::forward_f32).
    pub fn forward_quantized(&self, input: &[i8]) -> Option<Vec<f32>> {
        if input.len() != self.spec.dims.input_dim {
            return None;
        }
        let features: Vec<f32> = input
            .iter()
            .map(|q| f32::from(*q) / INPUT_QUANT_SCALE)
            .collect();
        self.forward_f32(&features)
    }
}

/// Argmax over logits with first-max-wins ties: the policy head the
/// tournament runs over runtime outputs. Empty input gives `None`, never
/// panics.
pub fn argmax(logits: &[i32]) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, v) in logits.iter().enumerate() {
        match best {
            Some((_, top)) if *v <= top => {}
            _ => best = Some((i, *v)),
        }
    }
    best.map(|(i, _)| i)
}

/// Mean squared error of the student over a dataset, in feature units:
/// one-hot targets, mean over rows and classes. Powers model selection in
/// the tournament and the training smoke test. Returns `None` on shape
/// mismatch.
pub fn mse_loss(student: &Student, dataset: &[crate::data::Sample]) -> Option<f32> {
    if dataset.is_empty() {
        return None;
    }
    let out_dim = student.spec.dims.output_dim;
    let mut sum = 0.0f32;
    for row in dataset {
        let logits = student.forward_f32(&row.quantized_features())?;
        if logits.len() != out_dim {
            return None;
        }
        for (o, logit) in logits.iter().enumerate() {
            let target = if usize::from(row.target) == o {
                1.0
            } else {
                0.0
            };
            let diff = logit - target;
            sum += diff * diff;
        }
    }
    Some(sum / (dataset.len() as f32) / (out_dim as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Sample;

    fn spec() -> StudentSpec {
        StudentSpec::new(2, 2, 0).expect("test spec is legal")
    }

    fn golden() -> Student {
        // o0 = 1 + 3*x0 - 4*x1; o1 = -2 + 5*x0 + 6*x1 (mirrors the runtime
        // dense golden so export tests share one arithmetic story).
        Student::new(spec(), vec![3.0, -4.0, 5.0, 6.0], vec![1.0, -2.0], 0)
            .expect("golden params are legal")
    }

    #[test]
    fn forward_matches_hand_arithmetic() {
        let out = golden().forward_f32(&[7.0, 8.0]).expect("input fits");
        assert_eq!(out, vec![-10.0, 81.0]);
    }

    #[test]
    fn forward_quantized_dequantizes_one_step() {
        // q = [7, 8] means features [7/16, 8/16] = [0.4375, 0.5].
        let out = golden().forward_quantized(&[7, 8]).expect("input fits");
        assert_eq!(
            out,
            vec![
                1.0 + 3.0 * 0.4375 - 4.0 * 0.5,
                -2.0 + 5.0 * 0.4375 + 6.0 * 0.5
            ]
        );
    }

    #[test]
    fn short_input_returns_none() {
        assert_eq!(golden().forward_f32(&[1.0]), None);
        assert_eq!(golden().forward_quantized(&[1]), None);
    }

    #[test]
    fn bad_params_rejected() {
        assert_eq!(
            Student::new(spec(), vec![1.0], vec![0.0, 0.0], 0),
            Err(StudentError::BadWeightLen { got: 1, want: 4 })
        );
        assert_eq!(
            Student::new(spec(), vec![0.0; 4], vec![0.0], 0),
            Err(StudentError::BadBiasLen { got: 1, want: 2 })
        );
        assert_eq!(
            Student::new(spec(), vec![f32::NAN, 0.0, 0.0, 0.0], vec![0.0, 0.0], 0),
            Err(StudentError::NonFiniteParam)
        );
    }

    #[test]
    fn argmax_first_wins_ties() {
        assert_eq!(argmax(&[]), None);
        assert_eq!(argmax(&[-10, 81]), Some(1));
        assert_eq!(argmax(&[5, 5]), Some(0));
        assert_eq!(argmax(&[-3, -3, -4]), Some(0));
    }

    #[test]
    fn loss_beats_nothing_and_rejects_mismatch() {
        let spec = spec();
        let rows = vec![
            Sample::new(vec![1.0, 0.0], 0, &spec).expect("legal"),
            Sample::new(vec![0.0, 1.0], 1, &spec).expect("legal"),
        ];
        // Identity student fits these rows exactly (loss 0); zeros do not.
        let trained = Student::new(spec, vec![1.0, 0.0, 0.0, 1.0], vec![0.0, 0.0], 0)
            .expect("identity params are legal");
        let baseline = Student::zeros(spec);
        let trained_loss = mse_loss(&trained, &rows).expect("shapes fit");
        let base_loss = mse_loss(&baseline, &rows).expect("shapes fit");
        assert!(trained_loss < base_loss, "{trained_loss} < {base_loss}");
        assert_eq!(mse_loss(&trained, &[]), None);
        let wrong = Sample {
            features: vec![1.0],
            target: 0,
        };
        assert_eq!(mse_loss(&trained, &[wrong]), None);
    }
}
