//! Offline student trainers: a continuous-target regression half that always
//! builds, and a burn one-hot half behind the `train` feature.
//!
//! Consumer: the PX5b residual-controller line records a walking body's own
//! control states at 60 Hz together with the correction its balance law
//! chose, fits a two-layer net to those continuous targets here, and ships
//! the int8 fold [`export_regression`](crate::export_regression) emits. The
//! tournament still fits one-hot class students with the `train` fn, the burn half
//! below.
//!
//! Continuous targets: a row is an observation plus one finite value per
//! output lane, so no class bound applies. The student is the same two-layer
//! [`Mlp`] the one-hot line uses (dense, ReLU, dense); the loss is mini-batch
//! mean squared error and the optimizer is Adam, so a controller correction
//! comes out as a value rather than an argmax. Rows enter the fit snapped to
//! the runtime's `1 / INPUT_QUANT_SCALE` steps, exactly as the one-hot rows'
//! `quantized_features` does, so a trained lane equals a runtime lane at the
//! `i8` edge, and the `1 / MAX_FEATURE_ABS` conditioning scale
//! folds back into `w1` on return (both scales are powers of two, so the
//! fold is exact).
//!
//! Errors: every bad spec, row or dataset maps to [`RegressionError`]; the
//! fit never panics. Convergence is measured, not promised: the fit returns
//! params and [`regression_loss`] reports the residual the caller gates.
//!
//! Determinism (both halves): fixed seeds, the batch order drawn from
//! `engine_core::Rng`, no thread spawned here, every sum in a fixed order, so
//! the same rows plus the same spec fit the same params on the same machine
//! (same binary). The burn half additionally serializes whole runs behind
//! `TRAIN_LOCK` because its NdArray RNG is process-global. Cross-machine
//! float-bit reproducibility is NOT claimed for either half: the NdArray
//! backend parallelizes tiny ops with rayon, whose reduction order follows
//! the machine thread count. The exported `i8` student is the portable
//! artifact.

use crate::data::{
    check_provenance, DatasetManifest, ProvenanceError, INPUT_QUANT_SCALE, MAX_FEATURE_ABS,
    MAX_SAMPLES,
};
use crate::mlp::{count_f32, shuffle_order, uniform, Adam, Grads, Mlp, MlpSpec, MAX_HIDDEN};
use crate::spec::{Dims, StudentSpec, MAX_EPOCHS};
use engine_core::Rng;

#[cfg(feature = "train")]
use crate::data::{check_dataset, DataErrorAt, Sample};
#[cfg(feature = "train")]
use crate::spec::SpecError;
#[cfg(feature = "train")]
use crate::student::Student;
#[cfg(feature = "train")]
use crate::student::StudentError;
/// Burn one-hot trainer. Backend policy: `Autodiff<NdArray>` on the default
/// CPU device only — no tch, wgpu, cuda, candle or remote backend is ever
/// selected (see the crate docs for the exclusion list).
///
/// Why the one-hot half is burn and the regression half is plain Rust: burn
/// 0.21 NdArray fits a dense layer with MSE bit-exactly, and its
/// cross-entropy path drifts ~1e-6 per 10 steps (identical inits, identical
/// epoch-0 loss — the drift sits in the gather/log-softmax backward, not in
/// seeding or thread count: same-seed inits match, `B::seed` forwards, and
/// `RAYON_NUM_THREADS=1` still drifts). Revisit if the backend pins its
/// backward order.
#[cfg(feature = "train")]
use burn::backend::ndarray::NdArrayDevice;
#[cfg(feature = "train")]
use burn::backend::{Autodiff, NdArray};
#[cfg(feature = "train")]
use burn::nn::loss::{MseLoss, Reduction};
#[cfg(feature = "train")]
use burn::nn::{Linear, LinearConfig};
#[cfg(feature = "train")]
use burn::optim::{GradientsParams, Optimizer, SgdConfig};
#[cfg(feature = "train")]
use burn::tensor::backend::Backend;
#[cfg(feature = "train")]
use burn::tensor::{Tensor, TensorData};

/// Backend for training: autodiff over the pure-Rust NdArray CPU backend.
#[cfg(feature = "train")]
type TrainBackend = Autodiff<NdArray>;

/// Serializes full training runs in this process. The NdArray backend RNG
/// is process-global, so concurrent runs with different seeds would
/// interleave draws and break seed isolation; the lock gives concurrent
/// callers (candidate sweeps) sequential semantics at the cost of
/// throughput. Poisoned only when a previous run panicked inside burn.
#[cfg(feature = "train")]
static TRAIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Fixed full-batch SGD learning rate for the MSE loss. Contract value,
/// not a tuning knob: 0.1 oscillates past the fixture optimum while 0.01
/// stalls, so 0.03 trains to the least-squares fit within budget.
#[cfg(feature = "train")]
pub const TRAIN_LR: f64 = 0.03;

/// Training failures.
#[cfg(feature = "train")]
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

#[cfg(feature = "train")]
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

#[cfg(feature = "train")]
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

#[cfg(feature = "train")]
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
#[cfg(feature = "train")]
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

#[cfg(all(test, feature = "train"))]
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

/// Largest absolute value a continuous target lane may carry. The export
/// folds the calibration peak into the `127` steps one `i8` lane holds, so a
/// wide lane costs resolution rather than correctness; this cap keeps one
/// runaway record from shipping a net whose steps are coarser than the
/// correction it predicts.
pub const MAX_TARGET_ABS: f32 = 64.0;

/// One recorded control state and the continuous correction its teacher
/// chose: the residual-controller row. Unlike a `Sample` row it carries no
/// class id, so no class bound applies to its target.
#[derive(Clone, Debug, PartialEq)]
pub struct RegressionRow {
    /// Observation, exactly `input_dim` finite values in
    /// `[-MAX_FEATURE_ABS, MAX_FEATURE_ABS]`, in feature units.
    pub inputs: Vec<f32>,
    /// Correction, exactly `output_dim` finite values in
    /// `[-MAX_TARGET_ABS, MAX_TARGET_ABS]`, in the runtime's own units.
    pub target: Vec<f32>,
}

/// Why one regression row refuses its spec.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowCause {
    /// Input count differs from the spec input dim; carries `(got, want)`.
    BadInputLen {
        /// Inputs on the row.
        got: usize,
        /// Spec input dim.
        want: usize,
    },
    /// Target count differs from the spec output dim; carries `(got, want)`.
    BadTargetLen {
        /// Target lanes on the row.
        got: usize,
        /// Spec output dim.
        want: usize,
    },
    /// An input is non-finite or past [`MAX_FEATURE_ABS`].
    BadInput,
    /// A target is non-finite or past [`MAX_TARGET_ABS`].
    BadTarget,
}

impl core::fmt::Display for RowCause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RowCause::BadInputLen { got, want } => write!(f, "input len {got}, want {want}"),
            RowCause::BadTargetLen { got, want } => write!(f, "target len {got}, want {want}"),
            RowCause::BadInput => write!(f, "input is non-finite or past ±{MAX_FEATURE_ABS}"),
            RowCause::BadTarget => write!(f, "target is non-finite or past ±{MAX_TARGET_ABS}"),
        }
    }
}

impl std::error::Error for RowCause {}

/// Regression training failures.
#[derive(Clone, Debug, PartialEq)]
pub enum RegressionError {
    /// Dataset provenance missing or unshippable; carries the manifest cause.
    Provenance(ProvenanceError),
    /// No rows: a mean over zero rows is undefined.
    EmptyDataset,
    /// More rows than [`MAX_SAMPLES`]; carries the count.
    TooManyRows(usize),
    /// A row does not fit the spec; carries its index and cause.
    BadRow {
        /// Offending row.
        index: usize,
        /// Why it refuses.
        cause: RowCause,
    },
    /// Hidden width outside `1..=MAX_HIDDEN`.
    BadHidden(usize),
    /// Epoch count outside `1..=MAX_EPOCHS`.
    BadEpochs(u32),
    /// Batch of zero rows.
    BadBatch,
    /// Learning rate not finite and positive.
    BadLearningRate,
    /// A param went non-finite.
    Diverged,
}

impl core::fmt::Display for RegressionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RegressionError::Provenance(e) => write!(f, "dataset provenance refused: {e}"),
            RegressionError::EmptyDataset => f.write_str("empty dataset"),
            RegressionError::TooManyRows(n) => write!(f, "{n} rows past {MAX_SAMPLES}"),
            RegressionError::BadRow { index, cause } => write!(f, "row {index}: {cause}"),
            RegressionError::BadHidden(h) => write!(f, "hidden width {h} outside 1..={MAX_HIDDEN}"),
            RegressionError::BadEpochs(n) => {
                write!(f, "epoch count {n} outside 1..={MAX_EPOCHS}")
            }
            RegressionError::BadBatch => f.write_str("batch of zero rows"),
            RegressionError::BadLearningRate => {
                f.write_str("learning rate not finite and positive")
            }
            RegressionError::Diverged => f.write_str("fit diverged to non-finite params"),
        }
    }
}

impl std::error::Error for RegressionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RegressionError::Provenance(e) => Some(e),
            RegressionError::BadRow { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

impl RegressionRow {
    /// Pack one row, validated against the spec dims. Never panics.
    pub fn new(inputs: Vec<f32>, target: Vec<f32>, spec: &StudentSpec) -> Result<Self, RowCause> {
        Self::check(&inputs, &target, spec.dims)?;
        Ok(Self { inputs, target })
    }

    /// Validate one observation/correction pair against the net dims. Used by
    /// the fit and by [`RegressionRow::new`], so both read one rule.
    pub(crate) fn check(inputs: &[f32], target: &[f32], dims: Dims) -> Result<(), RowCause> {
        if inputs.len() != dims.input_dim {
            return Err(RowCause::BadInputLen {
                got: inputs.len(),
                want: dims.input_dim,
            });
        }
        if target.len() != dims.output_dim {
            return Err(RowCause::BadTargetLen {
                got: target.len(),
                want: dims.output_dim,
            });
        }
        if !inputs
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_FEATURE_ABS)
        {
            return Err(RowCause::BadInput);
        }
        if !target
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_TARGET_ABS)
        {
            return Err(RowCause::BadTarget);
        }
        Ok(())
    }

    /// Inputs requantized exactly as the runtime sees them: the same
    /// `1 / INPUT_QUANT_SCALE` step the one-hot rows take.
    pub fn quantized_inputs(&self) -> Vec<f32> {
        self.inputs
            .iter()
            .map(|v| (v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE)
            .collect()
    }
}

/// Mean squared error over every lane of every row: `0.0` is an exact fit,
/// and `None` when the net does not match the rows' shape. The row count and
/// lane count stay under 2^24 in this crate's shapes, so the mean divides by
/// an exact integer.
pub fn regression_loss(mlp: &Mlp, rows: &[RegressionRow]) -> Option<f32> {
    let (_, hid, out) = mlp.dims();
    if rows.is_empty() {
        return None;
    }
    let mut hidden = vec![0.0f32; hid];
    let mut pred = vec![0.0f32; out];
    let mut total = 0.0f32;
    for row in rows {
        mlp.forward(&row.quantized_inputs(), &mut hidden, &mut pred)?;
        for (got, want) in pred.iter().zip(&row.target) {
            let d = got - want;
            total += d * d;
        }
    }
    Some(total / (count_f32(rows.len()) * count_f32(out)))
}

/// Checks shared by every regression fit: provenance first, then the spec,
/// then each row against the spec dims.
fn check_regression(
    rows: &[RegressionRow],
    spec: &MlpSpec,
    manifest: Option<&DatasetManifest>,
) -> Result<(), RegressionError> {
    check_provenance(manifest).map_err(RegressionError::Provenance)?;
    if spec.hidden == 0 || spec.hidden > MAX_HIDDEN {
        return Err(RegressionError::BadHidden(spec.hidden));
    }
    if spec.epochs == 0 || spec.epochs > MAX_EPOCHS {
        return Err(RegressionError::BadEpochs(spec.epochs));
    }
    if spec.batch == 0 {
        return Err(RegressionError::BadBatch);
    }
    if !spec.learning_rate.is_finite() || spec.learning_rate <= 0.0 {
        return Err(RegressionError::BadLearningRate);
    }
    if rows.is_empty() {
        return Err(RegressionError::EmptyDataset);
    }
    if rows.len() > MAX_SAMPLES {
        return Err(RegressionError::TooManyRows(rows.len()));
    }
    let dims = spec.student.dims;
    for (index, row) in rows.iter().enumerate() {
        RegressionRow::check(&row.inputs, &row.target, dims)
            .map_err(|cause| RegressionError::BadRow { index, cause })?;
    }
    Ok(())
}

/// Seeded init of one regression student: the same `6 / (fan_in + fan_out)`
/// uniform limits the one-hot fit draws, from the same rng stream, so a spec
/// seed picks the weights and then the epoch orders.
fn init_mlp(spec: &MlpSpec, rng: &mut Rng) -> Mlp {
    let inp = spec.student.dims.input_dim;
    let hid = spec.hidden;
    let out = spec.student.dims.output_dim;
    let lim1 = (6.0 / count_f32(inp + hid)).sqrt();
    let lim2 = (6.0 / count_f32(hid + out)).sqrt();
    Mlp {
        spec: *spec,
        w1: (0..hid * inp).map(|_| uniform(rng, lim1)).collect(),
        b1: vec![0.0; hid],
        w2: (0..out * hid).map(|_| uniform(rng, lim2)).collect(),
        b2: vec![0.0; out],
    }
}

/// Fit a two-layer student on continuous targets: mini-batch mean squared
/// error and Adam, the shape and optimizer the one-hot line already uses, so
/// one artifact path exports either. Inputs are snapped to the runtime's
/// `1 / INPUT_QUANT_SCALE` steps and scaled by `1 / MAX_FEATURE_ABS` inside
/// the loop for conditioning; that scale folds back into `w1` on return (both
/// scales are powers of two, so the fold is exact). Seeded by
/// `spec.student.seed`; see the module docs for the determinism scope.
/// Refuses missing provenance before any math runs.
pub fn fit_regression(
    rows: &[RegressionRow],
    spec: &MlpSpec,
    manifest: Option<&DatasetManifest>,
) -> Result<Mlp, RegressionError> {
    check_regression(rows, spec, manifest)?;
    let inp = spec.student.dims.input_dim;
    let hid = spec.hidden;
    let out = spec.student.dims.output_dim;
    let inv_scale = 1.0 / MAX_FEATURE_ABS;
    // Flat, step-snapped, unit-scaled inputs: one `inp`-wide row per record in
    // order, so the epoch loop slices contiguous batches with no per-row Vec.
    let mut xs: Vec<f32> = Vec::with_capacity(rows.len() * inp);
    for row in rows {
        for v in &row.inputs {
            xs.push((v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE * inv_scale);
        }
    }
    let mut rng = Rng::new(spec.student.seed);
    let mut mlp = init_mlp(spec, &mut rng);
    let mut opt = [
        Adam::new(hid * inp),
        Adam::new(hid),
        Adam::new(out * hid),
        Adam::new(out),
    ];
    let mut grads = Grads::new(inp, hid, out);
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let (mut pow1, mut pow2) = (1.0f32, 1.0f32);
    for _ in 0..spec.epochs {
        shuffle_order(&mut rng, &mut order);
        for batch in order.chunks(spec.batch) {
            grads.clear();
            // The 2.0 is the squared-error derivative, so `regression_loss`
            // and these gradients agree on the same objective.
            let scale = 2.0 / (count_f32(batch.len()) * count_f32(out));
            for &r in batch {
                let x = &xs[r * inp..(r + 1) * inp];
                grads
                    .add_mse_row(&mlp, x, &rows[r].target, scale)
                    .ok_or(RegressionError::Diverged)?;
            }
            pow1 *= 0.9;
            pow2 *= 0.999;
            let (c1, c2, lr) = (1.0 - pow1, 1.0 - pow2, spec.learning_rate);
            let (gw1, gb1, gw2, gb2) = grads.tables();
            opt[0].step(&mut mlp.w1, gw1, lr, c1, c2);
            opt[1].step(&mut mlp.b1, gb1, lr, c1, c2);
            opt[2].step(&mut mlp.w2, gw2, lr, c1, c2);
            opt[3].step(&mut mlp.b2, gb2, lr, c1, c2);
        }
    }
    for w in mlp.w1.iter_mut() {
        *w *= inv_scale;
    }
    let finite = [&mlp.w1, &mlp.b1, &mlp.w2, &mlp.b2]
        .iter()
        .all(|t| t.iter().all(|v| v.is_finite()));
    if !finite {
        return Err(RegressionError::Diverged);
    }
    Ok(mlp)
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::data::Sample;
    use crate::mlp::fit_mlp;
    use engine_core::Fnv64;

    /// Exact quarter-unit steps over `[-1, 1]`: every value already sits on a
    /// `1 / INPUT_QUANT_SCALE` step, so input quantization cannot move a row.
    const STEPS: [f32; 9] = [-1.0, -0.75, -0.5, -0.25, 0.0, 0.25, 0.5, 0.75, 1.0];

    /// 81-row grid with two folded targets, `|x0| - |x1|` and `max(x0, x1)`.
    /// Neither is a straight line, so no single dense layer can draw them.
    fn grid_rows(spec: &StudentSpec) -> Vec<RegressionRow> {
        let mut rows = Vec::new();
        for &a in &STEPS {
            for &b in &STEPS {
                let target = vec![a.abs() - b.abs(), a.max(b)];
                rows.push(RegressionRow::new(vec![a, b], target, spec).expect("legal row"));
            }
        }
        rows
    }

    fn grid_spec() -> MlpSpec {
        MlpSpec {
            student: StudentSpec::new(2, 2, 7).expect("legal spec"),
            hidden: 8,
            epochs: 600,
            batch: 16,
            learning_rate: 0.05,
        }
    }

    /// Same shape, all-zero params: the no-knowledge baseline the fit must
    /// beat (every lane predicts the mean, so its loss is the target variance).
    fn zero_net(spec: &MlpSpec) -> Mlp {
        let (inp, hid, out) = (
            spec.student.dims.input_dim,
            spec.hidden,
            spec.student.dims.output_dim,
        );
        Mlp {
            spec: *spec,
            w1: vec![0.0; hid * inp],
            b1: vec![0.0; hid],
            w2: vec![0.0; out * hid],
            b2: vec![0.0; out],
        }
    }

    /// Bit digest over every param lane in table order.
    fn params_digest(mlp: &Mlp) -> u64 {
        let mut hash = Fnv64::new();
        for table in [&mlp.w1, &mlp.b1, &mlp.w2, &mlp.b2] {
            for v in table {
                hash.write_f32(*v);
            }
        }
        hash.finish()
    }

    #[test]
    fn regression_fit_learns_a_fold_a_dense_layer_cannot_draw() {
        let spec = grid_spec();
        let rows = grid_rows(&spec.student);
        let manifest = DatasetManifest::fixture();
        let mlp = fit_regression(&rows, &spec, Some(&manifest)).expect("the grid fits");
        let base = regression_loss(&zero_net(&spec), &rows).expect("baseline loss");
        let got = regression_loss(&mlp, &rows).expect("trained loss");
        assert!(got < base / 20.0, "trained {got} vs no-knowledge {base}");
        assert!(got < 0.01, "grid mse {got} past the 0.01 bar");
        // Worst lane error over the whole grid: the fold is tracked at every
        // corner, not only on average.
        let mut hidden = [0.0f32; 8];
        let mut pred = [0.0f32; 2];
        let mut worst = 0.0f32;
        for row in &rows {
            mlp.forward(&row.quantized_inputs(), &mut hidden, &mut pred)
                .expect("row fits the net");
            for (g, w) in pred.iter().zip(&row.target) {
                worst = worst.max((g - w).abs());
            }
        }
        assert!(worst < 0.15, "worst lane error {worst}");
    }

    #[test]
    fn regression_fit_is_seed_pinned_and_replays_bit_exact() {
        let spec = grid_spec();
        let rows = grid_rows(&spec.student);
        let manifest = DatasetManifest::fixture();
        let first = fit_regression(&rows, &spec, Some(&manifest)).expect("the grid fits");
        let again = fit_regression(&rows, &spec, Some(&manifest)).expect("fits again");
        assert_eq!(first, again, "same rows and seed replay the same params");
        assert_eq!(params_digest(&first), params_digest(&again));
        let mut other = spec;
        other.student.seed = 12;
        let reshot = fit_regression(&rows, &other, Some(&manifest)).expect("other seed fits");
        assert_ne!(
            params_digest(&reshot),
            params_digest(&first),
            "a different seed must draw a different init and order"
        );
        // Another seed lands in another basin: it must still learn the grid,
        // but the exact residual is the seed-7 test's business, not this one.
        let base = regression_loss(&zero_net(&spec), &rows).expect("baseline loss");
        let reshot_loss = regression_loss(&reshot, &rows).expect("other seed loss");
        assert!(
            reshot_loss * 5.0 < base,
            "seed 12 mse {reshot_loss} vs no-knowledge {base}"
        );
    }

    #[test]
    fn regression_rows_specs_and_lists_are_refused() {
        let spec = grid_spec();
        let s = &spec.student;
        let manifest = DatasetManifest::fixture();
        assert_eq!(
            RegressionRow::new(vec![0.0], vec![0.0, 0.0], s),
            Err(RowCause::BadInputLen { got: 1, want: 2 })
        );
        assert_eq!(
            RegressionRow::new(vec![0.0, 0.0], vec![0.0], s),
            Err(RowCause::BadTargetLen { got: 1, want: 2 })
        );
        assert_eq!(
            RegressionRow::new(vec![MAX_FEATURE_ABS + 0.5, 0.0], vec![0.0, 0.0], s),
            Err(RowCause::BadInput)
        );
        assert_eq!(
            RegressionRow::new(vec![0.0, f32::NAN], vec![0.0, 0.0], s),
            Err(RowCause::BadInput)
        );
        // A wide target lane is refused: the export would ship steps coarser
        // than the correction it predicts.
        assert_eq!(
            RegressionRow::new(vec![0.0, 0.0], vec![MAX_TARGET_ABS + 0.5, 0.0], s),
            Err(RowCause::BadTarget)
        );
        let rows = grid_rows(s);
        assert_eq!(
            fit_regression(&rows, &spec, None),
            Err(RegressionError::Provenance(
                ProvenanceError::MissingManifest
            ))
        );
        let unlicensed = DatasetManifest {
            source: "arena-tapes".to_string(),
            license: "GPL-3.0-only".to_string(),
            digest: 1,
            trains: "residual-gait".to_string(),
        };
        match fit_regression(&rows, &spec, Some(&unlicensed)) {
            Err(RegressionError::Provenance(ProvenanceError::BadLicense { .. })) => {}
            other => panic!("unshippable license must be refused, got {other:?}"),
        }
        // The listed fixture is accepted: the same rows fit under it.
        let one_epoch = MlpSpec { epochs: 1, ..spec };
        fit_regression(&rows, &one_epoch, Some(&manifest)).expect("fixture list trains");
        assert_eq!(
            regression_loss(&zero_net(&spec), &[]),
            None,
            "a mean over zero rows is undefined"
        );
    }

    #[test]
    fn regression_fit_refuses_bad_specs_and_rows() {
        let spec = grid_spec();
        let rows = grid_rows(&spec.student);
        let m = DatasetManifest::fixture();
        assert_eq!(
            fit_regression(&[], &spec, Some(&m)),
            Err(RegressionError::EmptyDataset)
        );
        let mut bad = spec;
        bad.hidden = 0;
        assert_eq!(
            fit_regression(&rows, &bad, Some(&m)),
            Err(RegressionError::BadHidden(0))
        );
        bad = spec;
        bad.hidden = MAX_HIDDEN + 1;
        assert_eq!(
            fit_regression(&rows, &bad, Some(&m)),
            Err(RegressionError::BadHidden(MAX_HIDDEN + 1))
        );
        bad = spec;
        bad.epochs = MAX_EPOCHS + 1;
        assert_eq!(
            fit_regression(&rows, &bad, Some(&m)),
            Err(RegressionError::BadEpochs(MAX_EPOCHS + 1))
        );
        bad = spec;
        bad.batch = 0;
        assert_eq!(
            fit_regression(&rows, &bad, Some(&m)),
            Err(RegressionError::BadBatch)
        );
        bad = spec;
        bad.learning_rate = f32::NAN;
        assert_eq!(
            fit_regression(&rows, &bad, Some(&m)),
            Err(RegressionError::BadLearningRate)
        );
        let mut wrong = rows.clone();
        wrong[13].inputs[1] = 9.0;
        match fit_regression(&wrong, &spec, Some(&m)) {
            Err(RegressionError::BadRow {
                index: 13,
                cause: RowCause::BadInput,
            }) => {}
            other => panic!("want BadRow 13 BadInput, got {other:?}"),
        }
        // Past the row cap the fit refuses before it touches a single value.
        let wide = vec![
            RegressionRow::new(vec![0.0, 0.0], vec![0.0, 0.0], &spec.student)
                .expect("legal");
            MAX_SAMPLES + 1
        ];
        assert_eq!(
            fit_regression(&wide, &spec, Some(&m)),
            Err(RegressionError::TooManyRows(MAX_SAMPLES + 1))
        );
    }

    /// The one-hot dataset the plain-Rust `fit_mlp` tests already learn: an
    /// XOR-like sign rule over 400 step-exact rows.
    fn xor_rows(spec: &StudentSpec) -> Vec<Sample> {
        let mut rng = Rng::new(0x0E57_C0DE);
        (0..400)
            .map(|_| {
                let a = (rng.next_range(0, 255) as i32 - 127) as f32 / 16.0;
                let b = (rng.next_range(0, 255) as i32 - 127) as f32 / 16.0;
                let same = (a >= 0.0) == (b >= 0.0);
                Sample::new(vec![a, b], u8::from(same), spec).expect("legal row")
            })
            .collect()
    }

    /// The epoch shuffle is now one helper shared by both fits, so its draws
    /// are pinned by seed to an integer order (no float math, so no machine
    /// drift), and the one-hot fit it feeds must still learn the sign rule.
    #[test]
    fn regression_leaves_the_one_hot_fit_unchanged() {
        let mut rng = Rng::new(11);
        let mut order: Vec<usize> = (0..8).collect();
        shuffle_order(&mut rng, &mut order);
        const SEED_11_ORDER: [usize; 8] = [2, 6, 7, 1, 0, 5, 4, 3];
        assert_eq!(order, SEED_11_ORDER, "seed 11 epoch order moved");
        let mut rng = Rng::new(404);
        let mut order: Vec<usize> = (0..17).collect();
        shuffle_order(&mut rng, &mut order);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (0..17).collect::<Vec<_>>(),
            "the shuffle must stay a permutation"
        );
        let mut again: Vec<usize> = (0..17).collect();
        shuffle_order(&mut Rng::new(404), &mut again);
        assert_eq!(order, again, "seed 404 replays its order");
        let spec = MlpSpec {
            student: StudentSpec::new(2, 2, 11).expect("legal spec"),
            hidden: 8,
            epochs: 60,
            batch: 32,
            learning_rate: 0.02,
        };
        let rows = xor_rows(&spec.student);
        let manifest = DatasetManifest::fixture();
        let mlp = fit_mlp(&rows, &spec, Some(&manifest)).expect("one-hot fit still runs");
        let (agree, total) = mlp.agreement(&rows);
        assert_eq!(total, 400);
        assert!(agree * 100 >= total * 90, "xor agreement {agree}/{total}");
        assert_eq!(
            params_digest(&mlp),
            params_digest(&fit_mlp(&rows, &spec, Some(&manifest)).expect("one-hot fit replays")),
            "one-hot fit replays bit-exactly"
        );
    }
}
