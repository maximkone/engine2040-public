//! Student export to `.e2nn` bytes the `engine_nn` runtime loads (std-only).
//!
//! Consumer: the tournament loads the bytes with `engine_nn::load` and runs them with
//! `engine_nn::infer`. Owner: this module owns the float-to-int fold and the
//! byte layout. Units: weights quantize per-tensor (`w_scale = max|w|/127`,
//! all-zero maps to 1.0); biases fold by `input_scale * w_scale` with
//! `input_scale = 1 / INPUT_QUANT_SCALE`; requantization is an `i32`
//! multiplier plus a `0..=31` shift, mirroring the ONNX converter's
//! single-dense-layer shape; [`export_mlp`] folds a two-layer student
//! (dense + ReLU, dense) with calibrated hidden and output scales.
//! Determinism: pure function of the student plus the dataset list (and the
//! calibration rows for [`export_mlp`]). Errors: every bad param maps to [`ExportError`];
//! [`export`] also reloads the bytes through the runtime and fails when they
//! do not load, and refuses an empty or unlicensed dataset list first, so
//! every emitted `.e2nn` names the datasets it trained on.

use crate::data::{
    check_manifests_for_export, DatasetManifest, ProvenanceError, INPUT_QUANT_SCALE,
};
use crate::data::Sample;
use crate::mlp::Mlp;
use crate::student::Student;
use engine_nn::{NnErr, ACT_NONE, ACT_RELU, MAGIC, MAX_DIM, TAG_DENSE, VERSION};

/// Exported `.e2nn` file bytes, loadable by `engine_nn::load`.
pub type E2nnBytes = Vec<u8>;

/// Export failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// A weight or bias is NaN or infinite.
    NonFiniteParam,
    /// No dataset list was supplied, or a listed dataset carries an
    /// unshippable license; carries the manifest cause.
    Provenance(ProvenanceError),
    /// The folded scale ratio is non-positive, non-finite, or fits no `i32`
    /// multiplier at any shift.
    ScaleOverflow,
    /// Emitted bytes failed the runtime [`load`](engine_nn::load) check.
    Rejected(NnErr),
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExportError::NonFiniteParam => f.write_str("student param is non-finite"),
            ExportError::Provenance(e) => write!(f, "export dataset list refused: {e}"),
            ExportError::ScaleOverflow => f.write_str("scale ratio fits no i32 multiplier"),
            ExportError::Rejected(e) => write!(f, "emitted bytes rejected by NN1: {e}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::Provenance(e) => Some(e),
            ExportError::Rejected(e) => Some(e),
            _ => None,
        }
    }
}

/// Quantize runtime features to the runtime's `i8` inputs through the one
/// quantization edge ([`engine_nn::quantize_lane`]) at `INPUT_QUANT_SCALE`
/// steps per unit: round half to even, saturate to `[-127, 127]`, NaN to 0.
/// Inverts [`Student::forward_quantized`] up to one half step; the
/// tournament calls this before `engine_nn::infer`.
pub fn quantize_input(features: &[f32]) -> Vec<i8> {
    features
        .iter()
        .map(|v| engine_nn::quantize_lane(*v, INPUT_QUANT_SCALE))
        .collect()
}

/// Largest shift `0..=31` whose rounded multiplier fits `i32`: the one
/// maximum-precision rule `engine_nn` shares with its ONNX converter.
fn requant_params(ratio: f64) -> Result<(i32, u8), ExportError> {
    engine_nn::requant_params(ratio).ok_or(ExportError::ScaleOverflow)
}

/// One float dense layer folded to integers: per-tensor `i8` weights, `i32`
/// biases at `in_scale * w_scale`, and the requant to `out_scale`.
struct FoldedDense {
    out_dim: usize,
    mult: i32,
    shift: u8,
    act: u8,
    bias: Vec<i32>,
    weights: Vec<i8>,
}

/// Fold float `weights` (row-major `bias.len() x in_dim`) and `bias` whose
/// input carries `in_scale` real units per integer step; the integer output
/// carries `out_scale` real units per step.
fn fold_dense(
    weights: &[f32],
    bias: &[f32],
    in_scale: f64,
    out_scale: f64,
    act: u8,
) -> Result<FoldedDense, ExportError> {
    let w_scale = engine_nn::weight_scale(weights);
    let acc_scale = in_scale * f64::from(w_scale);
    let bias_q = bias
        .iter()
        .map(|b| {
            (f64::from(*b) / acc_scale)
                .round()
                .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        })
        .collect();
    let weights_q = weights
        .iter()
        .map(|w| (w / w_scale).round().clamp(-128.0, 127.0) as i8)
        .collect();
    let (mult, shift) = requant_params(acc_scale / out_scale)?;
    Ok(FoldedDense {
        out_dim: bias.len(),
        mult,
        shift,
        act,
        bias: bias_q,
        weights: weights_q,
    })
}

/// `.e2nn` bytes for dense layers over `in_dim` inputs, proven loadable.
fn emit_dense_net(in_dim: usize, layers: &[FoldedDense]) -> Result<E2nnBytes, ExportError> {
    debug_assert!(in_dim > 0 && in_dim <= MAX_DIM);
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(u8::try_from(layers.len()).unwrap_or(u8::MAX));
    out.extend_from_slice(&u16::try_from(in_dim).unwrap_or(u16::MAX).to_le_bytes());
    for layer in layers {
        debug_assert!(layer.out_dim > 0 && layer.out_dim <= MAX_DIM);
        out.push(TAG_DENSE);
        out.extend_from_slice(&u16::try_from(layer.out_dim).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(&layer.mult.to_le_bytes());
        out.push(layer.shift);
        out.push(layer.act);
        for b in &layer.bias {
            out.extend_from_slice(&b.to_le_bytes());
        }
        out.extend(layer.weights.iter().map(|w| w.cast_unsigned()));
    }
    engine_nn::load(&out).map_err(ExportError::Rejected)?;
    Ok(out)
}

/// Export one student as single-dense-layer `.e2nn` bytes in the converter's
/// shape (one compute node, no activation), then prove the bytes load in the
/// runtime. `manifests` is the dataset list that ships with these bytes (see
/// [`Shipment`](crate::data::Shipment)): an empty list or an unshippable
/// license is refused before any byte is folded, so every emitted `.e2nn`
/// names the datasets it trained on.
/// Pure function of the student plus the list.
pub fn export(student: &Student, manifests: &[DatasetManifest]) -> Result<E2nnBytes, ExportError> {
    check_manifests_for_export(manifests).map_err(ExportError::Provenance)?;
    if !student
        .weights
        .iter()
        .chain(student.bias.iter())
        .all(|v| v.is_finite())
    {
        return Err(ExportError::NonFiniteParam);
    }
    let in_scale = f64::from(1.0f32 / INPUT_QUANT_SCALE);
    // Identity output scale: the requant ratio is the accumulator scale, as
    // the single-layer converter shape has always folded it.
    let layer = fold_dense(&student.weights, &student.bias, in_scale, 1.0, ACT_NONE)?;
    emit_dense_net(student.spec.dims.input_dim, &[layer])
}

/// Largest absolute hidden activation and logit over the calibration rows
/// (float forward on the runtime-quantized features).
fn calibrate(mlp: &Mlp, calib: &[Sample]) -> (f32, f32) {
    let (_, hid, out) = mlp.dims();
    let mut hidden = vec![0.0f32; hid];
    let mut logits = vec![0.0f32; out];
    let (mut peak_h, mut peak_o) = (0.0f32, 0.0f32);
    for row in calib {
        if mlp
            .forward(&row.quantized_features(), &mut hidden, &mut logits)
            .is_none()
        {
            continue;
        }
        peak_h = hidden.iter().fold(peak_h, |a, v| a.max(*v));
        peak_o = logits.iter().fold(peak_o, |a, v| a.max(v.abs()));
    }
    (peak_h, peak_o)
}

/// Export a two-layer student as `.e2nn` bytes: dense + ReLU, then dense
/// with identity output. Hidden and output requant scales come from the
/// largest float activation over `calib` (post-training calibration), so no
/// calibration row saturates; the input scale is `1 / INPUT_QUANT_SCALE`.
/// Same provenance gate and load proof as [`export`]. Pure function of the
/// student, the calibration rows and the list.
pub fn export_mlp(
    mlp: &Mlp,
    calib: &[Sample],
    manifests: &[DatasetManifest],
) -> Result<E2nnBytes, ExportError> {
    check_manifests_for_export(manifests).map_err(ExportError::Provenance)?;
    let finite = [&mlp.w1, &mlp.b1, &mlp.w2, &mlp.b2]
        .iter()
        .all(|t| t.iter().all(|v| v.is_finite()));
    if !finite {
        return Err(ExportError::NonFiniteParam);
    }
    let (peak_h, peak_o) = calibrate(mlp, calib);
    // An all-dead calibration keeps a unit scale instead of dividing by zero.
    let h_scale = if peak_h > 0.0 { f64::from(peak_h) / 127.0 } else { 1.0 };
    let o_scale = if peak_o > 0.0 { f64::from(peak_o) / 127.0 } else { 1.0 };
    let in_scale = f64::from(1.0f32 / INPUT_QUANT_SCALE);
    let hidden = fold_dense(&mlp.w1, &mlp.b1, in_scale, h_scale, ACT_RELU)?;
    let output = fold_dense(&mlp.w2, &mlp.b2, h_scale, o_scale, ACT_NONE)?;
    emit_dense_net(mlp.spec.student.dims.input_dim, &[hidden, output])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Sample;
    use crate::spec::StudentSpec;

    fn golden() -> Student {
        let spec = StudentSpec::new(2, 2, 0).expect("test spec is legal");
        Student::new(spec, vec![3.0, -4.0, 5.0, 6.0], vec![1.0, -2.0], 0)
            .expect("golden params are legal")
    }

    #[test]
    fn quantize_input_goldens() {
        assert_eq!(quantize_input(&[7.0, 8.0]), vec![112, 127]);
        // 7.0 * 16 = 112 exactly; 8.0 * 16 = 128 clamps to 127.
        assert_eq!(quantize_input(&[0.0]), vec![0]);
        // Symmetric saturation: -8.0 * 16 = -128 clamps to -127; NaN maps to 0.
        assert_eq!(quantize_input(&[-8.0]), vec![-127]);
        assert_eq!(quantize_input(&[f32::NAN, 0.03125, 0.09375]), vec![0, 0, 2]);
        assert_eq!(quantize_input(&[0.1]), vec![2]);
    }

    #[test]
    fn exported_golden_loads_and_matches_layout() {
        let bytes = export(&golden(), &[DatasetManifest::fixture()]).expect("golden exports");
        let model = engine_nn::load(&bytes).expect("exported bytes load in NN1");
        assert_eq!(model.input_dim(), 2);
        assert_eq!(model.output_dim(), 2);
        // Runtime math on q = [7, 8]: acc0 = bias + 3*7 - 4*8 style accumulation
        // in quantized integers; the exact value is asserted below against
        // the integer reference, here only the shape and load matter.
        let out = engine_nn::infer(&model, &[7, 8]);
        assert_eq!(out.len(), 2);
    }

    /// Integer replica of the runtime dense forward (saturating `i32`
    /// accumulation, round-half-away requant, `i8` clamp, identity act).
    fn reference_forward(bytes: &[u8], input: &[i8]) -> Vec<i32> {
        let model = engine_nn::load(bytes).expect("bytes load");
        let layer = match &model.layers()[0] {
            engine_nn::Layer::Dense(d) => d,
            _ => panic!("export is one dense layer"),
        };
        let in_dim = model.input_dim();
        let mut state: Vec<i32> = input.iter().map(|v| i32::from(*v)).collect();
        while state.len() < in_dim {
            state.push(0);
        }
        state.truncate(in_dim);
        let mut out = Vec::with_capacity(layer.out_dim());
        for o in 0..layer.out_dim() {
            let mut acc = layer.bias()[o];
            for (i, v) in state.iter().enumerate() {
                acc = acc
                    .saturating_add(i32::from(layer.weights()[o * in_dim + i]).saturating_mul(*v));
            }
            let prod = acc as i64 * layer.mult() as i64;
            let shift = layer.shift();
            let rounded = if shift == 0 {
                prod
            } else {
                let round: i64 = 1i64 << (shift - 1);
                if prod >= 0 {
                    prod.saturating_add(round)
                } else {
                    prod.saturating_sub(round)
                }
            } >> shift;
            let narrowed = rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            out.push(narrowed.clamp(-128, 127));
        }
        out
    }

    #[test]
    fn integer_reference_matches_nn1_exactly() {
        let bytes = export(&golden(), &[DatasetManifest::fixture()]).expect("golden exports");
        for input in [vec![7, 8], vec![0, 0], vec![-128, 127], vec![1, -1]] {
            let model = engine_nn::load(&bytes).expect("bytes load");
            assert_eq!(
                engine_nn::infer(&model, &input),
                reference_forward(&bytes, &input)
            );
        }
    }

    #[test]
    fn zero_student_exports_to_zero_logits() {
        let spec = StudentSpec::new(3, 2, 9).expect("legal spec");
        let bytes =
            export(&Student::zeros(spec), &[DatasetManifest::fixture()]).expect("zeros export");
        let model = engine_nn::load(&bytes).expect("bytes load");
        assert_eq!(engine_nn::infer(&model, &[1, 2, 3]), vec![0, 0]);
    }

    #[test]
    fn nonfinite_params_rejected() {
        let spec = StudentSpec::new(1, 1, 0).expect("legal spec");
        let mut student = Student::zeros(spec);
        student.weights[0] = f32::NAN;
        assert_eq!(
            export(&student, &[DatasetManifest::fixture()]),
            Err(ExportError::NonFiniteParam)
        );
    }

    #[test]
    fn trained_shape_roundtrips_through_nn1() {
        // The shape train() emits (4 features, 3 classes) must load too.
        let spec = StudentSpec::new(4, 3, 0).expect("legal spec");
        let rows = [Sample::new(vec![0.5; 4], 2, &spec).expect("legal")];
        assert_eq!(rows.len(), 1);
        let student = Student::zeros(spec);
        let bytes =
            export(&student, &[DatasetManifest::fixture()]).expect("train-shaped zeros export");
        let model = engine_nn::load(&bytes).expect("bytes load");
        assert_eq!((model.input_dim(), model.output_dim()), (4, 3));
    }

    #[test]
    fn export_without_listed_datasets_refused() {
        use crate::data::{DatasetManifest, ProvenanceError};

        assert_eq!(
            export(&golden(), &[]),
            Err(ExportError::Provenance(ProvenanceError::MissingManifest))
        );
        let unlicensed = DatasetManifest {
            source: "arena-tapes".to_string(),
            license: "CC-BY-NC-4.0".to_string(),
            digest: 1,
            trains: "melee-policy".to_string(),
        };
        match export(&golden(), &[unlicensed]) {
            Err(ExportError::Provenance(ProvenanceError::BadLicense { .. })) => {}
            other => panic!("unshippable license must be refused, got {other:?}"),
        }
    }

    /// Hand-built two-layer student: hidden h0 = relu(x0 - x1), h1 = relu(x1);
    /// logits [h0 - h1, h1 - h0 + 0.5].
    fn hand_mlp() -> Mlp {
        use crate::mlp::MlpSpec;
        Mlp {
            spec: MlpSpec {
                student: StudentSpec::new(2, 2, 0).expect("legal"),
                hidden: 2,
                epochs: 1,
                batch: 1,
                learning_rate: 0.1,
            },
            w1: vec![1.0, -1.0, 0.0, 1.0],
            b1: vec![0.0, 0.0],
            w2: vec![1.0, -1.0, -1.0, 1.0],
            b2: vec![0.0, 0.5],
        }
    }

    fn grid(spec: &StudentSpec) -> Vec<Sample> {
        let mut rows = Vec::new();
        for a in (-112..=112).step_by(16) {
            for b in (-112..=112).step_by(16) {
                let f = vec![a as f32 / 16.0, b as f32 / 16.0];
                rows.push(Sample::new(f, 0, spec).expect("legal row"));
            }
        }
        rows
    }

    #[test]
    fn two_layer_export_tracks_the_float_argmax() {
        let mlp = hand_mlp();
        let rows = grid(&mlp.spec.student);
        let bytes = export_mlp(&mlp, &rows, &[DatasetManifest::fixture()]).expect("exports");
        let model = engine_nn::load(&bytes).expect("loads");
        assert_eq!(model.layers().len(), 2);
        assert_eq!((model.input_dim(), model.output_dim()), (2, 2));
        let mut hidden = [0.0f32; 2];
        let mut logits = [0.0f32; 2];
        let mut agree = 0usize;
        for row in &rows {
            mlp.forward(&row.quantized_features(), &mut hidden, &mut logits)
                .expect("fits");
            let want = crate::mlp::first_max_f32(&logits);
            let got = crate::student::argmax(&engine_nn::infer(&model, &quantize_input(&row.features)));
            if got == Some(want) {
                agree += 1;
            }
            // Calibration bounds every row: no output lane saturates.
            for v in engine_nn::infer(&model, &quantize_input(&row.features)) {
                assert!((-127..=127).contains(&v), "{v}");
            }
        }
        // Integer rounding may flip only near-tie rows of the 225-row grid.
        assert!(agree * 100 >= rows.len() * 95, "agree {agree}/{}", rows.len());
        // Same student, same calibration: same bytes.
        assert_eq!(
            export_mlp(&mlp, &rows, &[DatasetManifest::fixture()]).expect("exports"),
            bytes
        );
    }

    #[test]
    fn two_layer_export_refuses_bad_params_and_lists() {
        let mut mlp = hand_mlp();
        let rows = grid(&mlp.spec.student);
        assert_eq!(
            export_mlp(&mlp, &rows, &[]),
            Err(ExportError::Provenance(ProvenanceError::MissingManifest))
        );
        mlp.b2[1] = f32::INFINITY;
        assert_eq!(
            export_mlp(&mlp, &rows, &[DatasetManifest::fixture()]),
            Err(ExportError::NonFiniteParam)
        );
        // No calibration rows: unit scales, still a loadable net.
        let bytes = export_mlp(&hand_mlp(), &[], &[DatasetManifest::fixture()]).expect("exports");
        engine_nn::load(&bytes).expect("loads");
    }
}
