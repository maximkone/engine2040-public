//! Offline student trainer on burn (feature `train`).
//!
//! Consumer: the tournament trains one student per candidate from a fixed
//! teacher dataset, then exports the winner. Owner: this module owns the training
//! loop, the optimizer settings and the burn-to-[`Student`] readback.
//! Units: inputs are quantized-step features as `f32` (see
//! [`Sample::quantized_features`](crate::data::Sample::quantized_features));
//! targets are one-hot class vectors trained with full-batch MSE. MSE is
//! deliberate: it exercises only the deterministic matmul-plus-reduction
//! op path, so runs are bit-exact. Cross-entropy was tried and dropped:
//! on burn 0.21 NdArray its runs drift ~1e-6 per 10 steps (identical inits,
//! identical epoch-0 loss), which breaks the bit-exactness contract; the
//! drift sits somewhere in the gather/log-softmax backward path, not in
//! seeding or thread counts (verified: same-seed inits match, `B::seed`
//! forwards, `RAYON_NUM_THREADS=1` still drifts). Revisit if the backend
//! pins its backward order. Determinism scope: fixed seeds, full-batch
//! order, no shuffling and no threads spawned by this module; the run is
//! bit-exact on the same machine (same binary, CPU
//! and thread count). Cross-machine float-bit reproducibility is NOT
//! claimed: the NdArray backend parallelizes tiny ops with rayon, whose
//! reduction order follows the machine thread count. The exported `i8`
//! student is the portable artifact. Errors: bad input maps to
//! [`TrainError`]; burn tensor shapes are built from validated dims, so the
//! loop itself has no failure case beyond a diverged (non-finite) result.
//!
//! Backend: `Autodiff<NdArray>` on the default CPU device only. No
//! tch/wgpu/cuda/candle/rocm/remote backend is ever selected; see crate
//! docs for the exclusion list.

use burn::backend::ndarray::NdArrayDevice;
use burn::backend::{Autodiff, NdArray};
use burn::nn::loss::{MseLoss, Reduction};
use burn::nn::{Linear, LinearConfig};
use burn::optim::{GradientsParams, Optimizer, SgdConfig};
use burn::tensor::backend::Backend;
use burn::tensor::{Tensor, TensorData};

use crate::data::{
    check_dataset, check_provenance, DataErrorAt, DatasetManifest, ProvenanceError, Sample,
};
use crate::spec::{SpecError, StudentSpec, MAX_EPOCHS};
use crate::student::{Student, StudentError};

/// Backend for training: autodiff over the pure-Rust NdArray CPU backend.
type TrainBackend = Autodiff<NdArray>;

/// Serializes full training runs in this process. The NdArray backend RNG
/// is process-global, so concurrent runs with different seeds would
/// interleave draws and break seed isolation; the lock gives concurrent
/// callers (candidate sweeps) sequential semantics at the cost of
/// throughput. Poisoned only when a previous run panicked inside burn.
static TRAIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Fixed full-batch SGD learning rate for the MSE loss. Contract value,
/// not a tuning knob: 0.1 oscillates past the fixture optimum while 0.01
/// stalls, so 0.03 trains to the least-squares fit within budget.
pub const TRAIN_LR: f64 = 0.03;

/// Training failures.
#[derive(Clone, Debug, PartialEq)]
pub enum TrainError {
    /// Spec or dataset row invalid; the dataset case carries its row index.
    BadData(DataErrorAt),
    /// Spec itself invalid (re-exported so callers match one error type).
    BadSpec(SpecError),
    /// Dataset provenance missing or unshippable; carries the manifest cause.
    Provenance(ProvenanceError),
    /// Empty dataset: gradients over zero rows are undefined.
    EmptyDataset,
    /// Epoch count is zero or above [`MAX_EPOCHS`]; carries the count.
    BadEpochs(u32),
    /// Burn returned no bias vector (internal invariant: bias is enabled).
    MissingBias,
    /// The training lock was poisoned by a previous panicked run.
    LockPoisoned,
    /// A trained param read back non-finite: the run diverged.
    Diverged,
    /// Param readback from the backend failed; carries the backend message.
    Readback(String),
}

impl core::fmt::Display for TrainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TrainError::BadData(e) => write!(f, "bad dataset: {e}"),
            TrainError::BadSpec(e) => write!(f, "bad spec: {e}"),
            TrainError::Provenance(e) => write!(f, "dataset provenance refused: {e}"),
            TrainError::EmptyDataset => f.write_str("empty dataset"),
            TrainError::BadEpochs(n) => write!(f, "epoch count {n} outside 1..={MAX_EPOCHS}"),
            TrainError::MissingBias => f.write_str("burn returned no bias vector"),
            TrainError::LockPoisoned => f.write_str("training lock poisoned by a panicked run"),
            TrainError::Diverged => f.write_str("training diverged to non-finite params"),
            TrainError::Readback(e) => write!(f, "param readback failed: {e}"),
        }
    }
}

impl std::error::Error for TrainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TrainError::BadData(e) => Some(e),
            TrainError::BadSpec(e) => Some(e),
            TrainError::Provenance(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StudentError> for TrainError {
    fn from(e: StudentError) -> Self {
        match e {
            StudentError::NonFiniteParam => TrainError::Diverged,
            StudentError::BadWeightLen { got, want } => {
                TrainError::Readback(format!("weight len {got}, want {want}"))
            }
            StudentError::BadBiasLen { got, want } => {
                TrainError::Readback(format!("bias len {got}, want {want}"))
            }
        }
    }
}

/// Train one student: full-batch MSE against one-hot teacher classes with
/// plain SGD at [`TRAIN_LR`], `epochs` passes over the slice in order.
/// `manifest` names the training dataset's provenance: `None` or an
/// unshippable license is refused before any tensor runs, so training never
/// learns from unlisted rows.
/// Seeded by `spec.seed`; the same dataset plus spec trains the same
/// student on the same machine.
pub fn train(
    dataset: &[Sample],
    spec: &StudentSpec,
    epochs: u32,
    manifest: Option<&DatasetManifest>,
) -> Result<Student, TrainError> {
    check_provenance(manifest).map_err(TrainError::Provenance)?;
    if epochs == 0 || epochs > MAX_EPOCHS {
        return Err(TrainError::BadEpochs(epochs));
    }
    if dataset.is_empty() {
        return Err(TrainError::EmptyDataset);
    }
    check_dataset(dataset, spec).map_err(TrainError::BadData)?;

    // Held for the whole run: seed, init, epochs and readback stay one
    // atomic unit against other threads (see TRAIN_LOCK).
    let _guard = TRAIN_LOCK.lock().map_err(|_| TrainError::LockPoisoned)?;

    let device = NdArrayDevice::default();
    TrainBackend::seed(&device, spec.seed);

    let in_dim = spec.dims.input_dim;
    let out_dim = spec.dims.output_dim;
    let rows = dataset.len();

    let mut x_flat = Vec::with_capacity(rows * in_dim);
    let mut t_flat = Vec::with_capacity(rows * out_dim);
    for row in dataset {
        // Validated above, so lengths match the spec exactly.
        x_flat.extend_from_slice(&row.quantized_features());
        for o in 0..out_dim {
            t_flat.push(if usize::from(row.target) == o {
                1.0f32
            } else {
                0.0f32
            });
        }
    }
    let inputs =
        Tensor::<TrainBackend, 2>::from_data(TensorData::new(x_flat, [rows, in_dim]), &device);
    let targets =
        Tensor::<TrainBackend, 2>::from_data(TensorData::new(t_flat, [rows, out_dim]), &device);

    let mut model = LinearConfig::new(in_dim, out_dim).init::<TrainBackend>(&device);
    let mut optim = SgdConfig::new().init::<TrainBackend, Linear<TrainBackend>>();
    let loss_fn = MseLoss::new();
    for _ in 0..epochs {
        let logits = model.forward(inputs.clone());
        let loss = loss_fn.forward(logits, targets.clone(), Reduction::Mean);
        let grads = loss.backward();
        let grads = GradientsParams::from_grads(grads, &model);
        model = optim.step(TRAIN_LR, model, grads);
    }

    // Row layout stores `[input_dim, output_dim]`; the student keeps
    // output-major rows, so transpose on readback.
    let raw_w: Vec<f32> = model
        .weight
        .val()
        .into_data()
        .to_vec()
        .map_err(|e| TrainError::Readback(format!("{e:?}")))?;
    if raw_w.len() != in_dim * out_dim {
        return Err(TrainError::Readback(format!(
            "weight len {}, want {}",
            raw_w.len(),
            in_dim * out_dim
        )));
    }
    let mut weights = vec![0.0f32; out_dim * in_dim];
    for o in 0..out_dim {
        for i in 0..in_dim {
            weights[o * in_dim + i] = raw_w[i * out_dim + o];
        }
    }
    let bias: Vec<f32> = model
        .bias
        .as_ref()
        .ok_or(TrainError::MissingBias)?
        .val()
        .into_data()
        .to_vec()
        .map_err(|e| TrainError::Readback(format!("{e:?}")))?;

    Student::new(*spec, weights, bias, epochs).map_err(TrainError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::MAX_FEATURE_ABS;
    use crate::export::{export, quantize_input};
    use crate::student::{argmax, mse_loss};

    /// Tiny SplitMix64 for test fixtures (own 10-line copy; the arena
    /// fixture expander stays behind the arena boundary).
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn next_f32(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u64() >> 33) as f32 / (1u64 << 31) as f32;
            lo + u * (hi - lo)
        }
    }

    /// Fixed teacher: 3 classes over 4 features, columns separated so a
    /// linear student can fit it. Class = argmax of the teacher logits.
    const TEACHER: [[f32; 4]; 3] = [
        [1.5, -1.0, 0.5, 0.25],
        [-1.0, 1.5, -0.5, 0.75],
        [0.25, 0.5, 1.5, -1.5],
    ];

    fn teacher_class(features: &[f32]) -> u8 {
        let mut best = 0;
        let mut top = f32::NEG_INFINITY;
        for (o, row) in TEACHER.iter().enumerate() {
            let mut acc = 0.0;
            for (w, v) in row.iter().zip(features.iter()) {
                acc += w * v;
            }
            if acc > top {
                top = acc;
                best = o;
            }
        }
        best as u8
    }

    /// Fixed 48-row dataset from seed 0xD71L: deterministic across runs.
    fn fixture_dataset(spec: &StudentSpec) -> Vec<Sample> {
        let mut rng = Rng(0xD71E_5EED_2026_0923);
        let mut rows = Vec::new();
        for _ in 0..48 {
            let mut features = Vec::with_capacity(spec.dims.input_dim);
            for _ in 0..spec.dims.input_dim {
                let v = rng.next_f32(-MAX_FEATURE_ABS, MAX_FEATURE_ABS);
                // Snap to quant steps so train values equal infer values.
                features.push((v * 16.0).round() / 16.0);
            }
            let target = teacher_class(&features);
            rows.push(Sample::new(features, target, spec).expect("fixture rows are legal"));
        }
        rows
    }

    fn fixture_spec() -> StudentSpec {
        StudentSpec::new(4, 3, 0x5EED_1234).expect("fixture spec is legal")
    }

    #[test]
    fn bad_inputs_rejected() {
        let spec = fixture_spec();
        let rows = fixture_dataset(&spec);
        let manifest = DatasetManifest::fixture();
        assert_eq!(
            train(&[], &spec, 10, Some(&manifest)),
            Err(TrainError::EmptyDataset)
        );
        assert_eq!(
            train(&rows, &spec, 0, Some(&manifest)),
            Err(TrainError::BadEpochs(0))
        );
        assert_eq!(
            train(&rows, &spec, MAX_EPOCHS + 1, Some(&manifest)),
            Err(TrainError::BadEpochs(MAX_EPOCHS + 1))
        );
        let mut bad = rows.clone();
        bad[3] = Sample {
            features: vec![0.0; 4],
            target: 9,
        };
        match train(&bad, &spec, 10, Some(&manifest)) {
            Err(TrainError::BadData(at)) => assert_eq!(at.index, 3),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn train_refuses_manifestless_or_unlicensed_datasets() {
        use crate::data::{DatasetManifest, ProvenanceError};

        let spec = fixture_spec();
        let rows = fixture_dataset(&spec);
        assert_eq!(
            train(&rows, &spec, 10, None),
            Err(TrainError::Provenance(ProvenanceError::MissingManifest))
        );
        let unlicensed = DatasetManifest {
            source: "arena-tapes".to_string(),
            license: "GPL-3.0-only".to_string(),
            digest: 1,
            trains: "melee-policy".to_string(),
        };
        match train(&rows, &spec, 10, Some(&unlicensed)) {
            Err(TrainError::Provenance(ProvenanceError::BadLicense { .. })) => {}
            other => panic!("unshippable license must be refused, got {other:?}"),
        }
    }

    /// Argmax accuracy of a float student over the fixture.
    fn accuracy(student: &Student, rows: &[Sample]) -> usize {
        let mut correct = 0;
        for row in rows {
            let logits = student
                .forward_f32(&row.quantized_features())
                .expect("fits spec");
            let class = logits
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).expect("finite logits"))
                .map(|(i, _)| i)
                .expect("nonempty outputs");
            if class == usize::from(row.target) {
                correct += 1;
            }
        }
        correct
    }

    #[test]
    fn fixed_dataset_trains_reproducibly_and_learns() {
        let spec = fixture_spec();
        let rows = fixture_dataset(&spec);
        let manifest = DatasetManifest::fixture();
        let first = train(&rows, &spec, 1000, Some(&manifest)).expect("fixture trains");
        let second = train(&rows, &spec, 1000, Some(&manifest)).expect("fixture trains again");
        assert_eq!(
            first, second,
            "same dataset twice trains bit-identical params"
        );
        assert_eq!(first.epochs_trained, 1000);

        // The fit must beat the no-knowledge baseline on its own loss.
        let base_loss = mse_loss(&Student::zeros(spec), &rows).expect("baseline loss");
        let trained_loss = mse_loss(&first, &rows).expect("trained loss");
        assert!(
            trained_loss < base_loss,
            "trained {trained_loss} beats zeros {base_loss}"
        );
        // Separable fixture: the student must nearly match the teacher.
        // (MSE fits logit magnitudes rather than boundaries, so a couple
        // of near-tie rows may stay flipped; the tournament judges.)
        let correct = accuracy(&first, &rows);
        assert!(
            correct >= 42,
            "train accuracy {correct}/48 too low for a separable fixture"
        );
    }

    #[test]
    fn exported_student_loads_and_agrees_with_teacher() {
        let spec = fixture_spec();
        let rows = fixture_dataset(&spec);
        let manifest = DatasetManifest::fixture();
        let student = train(&rows, &spec, 1000, Some(&manifest)).expect("fixture trains");
        let bytes = export(&student, &[manifest]).expect("trained student exports");
        let model = engine_nn::load(&bytes).expect("exported student loads in NN1");
        assert_eq!((model.input_dim(), model.output_dim()), (4, 3));

        // Runtime argmax over the fixture agrees with the teacher on nearly
        // every row: the int8 round trip costs at most a couple of rows.
        let mut correct = 0;
        for row in &rows {
            let q = quantize_input(&row.features);
            let logits = engine_nn::infer(&model, &q);
            if argmax(&logits) == Some(usize::from(row.target)) {
                correct += 1;
            }
        }
        assert!(
            correct >= 42,
            "NN1 accuracy {correct}/48 too low after int8 export"
        );
    }
}
