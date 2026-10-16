//! Offline quantization-aware training for the tiny int8 student.
//!
//! Consumer: offline trainer that shapes float weights with simulated int8
//! noise, then ships only int8 bytes the tick runtime loads. Owner: this
//! module owns the simulation rule, the bias-free trainer and the toolchain
//! receipt. Units: features in real units, scales in real units per int step,
//! losses unitless. Determinism: fixed order, seeded init, integer hashes, no
//! clock or threads. Errors: bad shapes map to typed errors, never panics.

use crate::{
    load, quantize_lane, requant_params, weight_scale, NnErr, ACT_NONE, MAGIC, MAX_DIM, TAG_DENSE,
    VERSION,
};
use engine_core::{hash::hash_bytes, Rng};

/// Input steps per unit on this path (matches the runtime edge).
pub const QAT_INPUT_STEPS: f32 = 16.0;
/// Fixed full-batch step size for the offline trainer.
pub const QAT_LR: f32 = 0.03;
/// Largest epoch count the trainer accepts.
pub const QAT_MAX_EPOCHS: u32 = 2000;
/// Largest training row count accepted.
pub const QAT_MAX_SAMPLES: usize = 4096;

/// Offline training and export failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QatErr {
    /// A dim is zero or above the runtime cap.
    BadDim,
    /// A slice length does not match its dims.
    BadLen,
    /// No training rows were supplied.
    EmptyDataset,
    /// Epoch count is zero or above the cap.
    BadEpochs,
    /// A target names no output class.
    BadTarget,
    /// A param or feature is not finite.
    NonFinite,
    /// A scale ratio fits no integer multiplier.
    ScaleOverflow,
    /// Emitted bytes failed the runtime load check.
    Rejected(NnErr),
}

impl core::fmt::Display for QatErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            QatErr::BadDim => f.write_str("qat dim outside 1..=1024"),
            QatErr::BadLen => f.write_str("qat slice length differs from its dims"),
            QatErr::EmptyDataset => f.write_str("qat dataset is empty"),
            QatErr::BadEpochs => f.write_str("qat epoch count outside 1..=2000"),
            QatErr::BadTarget => f.write_str("qat target names no output class"),
            QatErr::NonFinite => f.write_str("qat param or feature is non-finite"),
            QatErr::ScaleOverflow => f.write_str("qat scale fits no i32 multiplier"),
            QatErr::Rejected(e) => write!(f, "qat bytes rejected: {e}"),
        }
    }
}

impl std::error::Error for QatErr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            QatErr::Rejected(e) => Some(e),
            _ => None,
        }
    }
}

/// Training knobs for one bias-free dense student.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QatConfig {
    /// Feature count per row, 1..=1024.
    pub input_dim: usize,
    /// Class count, 1..=1024.
    pub output_dim: usize,
    /// Seed for the deterministic initial weights.
    pub seed: u64,
    /// Full-batch passes, 1..=2000.
    pub epochs: u32,
    /// True runs the int8 simulation in the forward pass.
    pub qat: bool,
}

impl QatConfig {
    /// Validate dims and epochs and pack the config.
    ///
    /// Units: dims are element counts, epochs are passes. Errors: zero or
    /// over-cap dims give [`QatErr::BadDim`], zero or over-cap epochs give
    /// [`QatErr::BadEpochs`]. Determinism: pure data, no IO.
    pub fn new(
        input_dim: usize,
        output_dim: usize,
        seed: u64,
        epochs: u32,
        qat: bool,
    ) -> Result<Self, QatErr> {
        if input_dim == 0 || input_dim > MAX_DIM {
            return Err(QatErr::BadDim);
        }
        if output_dim == 0 || output_dim > MAX_DIM {
            return Err(QatErr::BadDim);
        }
        if epochs == 0 || epochs > QAT_MAX_EPOCHS {
            return Err(QatErr::BadEpochs);
        }
        Ok(Self {
            input_dim,
            output_dim,
            seed,
            epochs,
            qat,
        })
    }
}

/// Simulate int8 quantization in a float forward pass.
///
/// Units: `value` in real units, `scale` in real units per int step; the
/// result is the dequantized value the int8 grid would carry. Determinism:
/// one `f32` divide then the shared lane rule (`round half to even`,
/// saturate, NaN to zero), no fused ops. Errors: none; a bad scale maps to
/// zero so training never diverges on an uncalibrated step.
pub fn simulate_quantize(value: f32, scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0f32 {
        return 0.0f32;
    }
    let steps = 1.0f32 / scale;
    let q = quantize_lane(value, steps);
    f32::from(q) * scale
}

/// Deterministic initial weights in [-0.5, 0.5] from the seed.
fn init_qat_weights(input_dim: usize, output_dim: usize, seed: u64) -> Result<Vec<f32>, QatErr> {
    let count = input_dim.checked_mul(output_dim).ok_or(QatErr::BadDim)?;
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = rng.next_u64() % 65536u64;
        let narrowed = u16::try_from(raw).map_err(|_| QatErr::NonFinite)?;
        out.push(f32::from(narrowed) / 65535.0f32 - 0.5f32);
    }
    Ok(out)
}

/// Validate flat features plus targets against the config; returns row count.
fn check_train_inputs(
    all_features: &[f32],
    targets: &[u8],
    cfg: &QatConfig,
) -> Result<usize, QatErr> {
    let rows = targets.len();
    if rows == 0 {
        return Err(QatErr::EmptyDataset);
    }
    if rows > QAT_MAX_SAMPLES {
        return Err(QatErr::BadLen);
    }
    let want = rows.checked_mul(cfg.input_dim).ok_or(QatErr::BadDim)?;
    if all_features.len() != want {
        return Err(QatErr::BadLen);
    }
    for v in all_features {
        if !v.is_finite() {
            return Err(QatErr::NonFinite);
        }
    }
    for t in targets {
        if usize::from(*t) >= cfg.output_dim {
            return Err(QatErr::BadTarget);
        }
    }
    Ok(rows)
}

/// Logits for one row: runtime-quantized inputs through the (optionally
/// simulated) dense weights. Caller owns all slices; no heap per call beyond
/// the returned vector, which the trainer reuses per row.
fn logits_for_row(
    weights: &[f32],
    cfg: &QatConfig,
    row_features: &[f32],
    w_scale: f32,
) -> Vec<f32> {
    let mut x = Vec::with_capacity(cfg.input_dim);
    for v in row_features {
        let q = quantize_lane(*v, QAT_INPUT_STEPS);
        x.push(f32::from(q) / QAT_INPUT_STEPS);
    }
    let mut out = vec![0.0f32; cfg.output_dim];
    for o in 0..cfg.output_dim {
        let mut acc = 0.0f32;
        for i in 0..cfg.input_dim {
            let w = weights[o * cfg.input_dim + i];
            let w_use = if cfg.qat {
                simulate_quantize(w, w_scale)
            } else {
                w
            };
            acc += w_use * x[i];
        }
        out[o] = acc;
    }
    out
}

/// Train a bias-free dense student with full-batch SGD on one-hot MSE.
///
/// Units: features in real units within the runtime range, targets are class
/// ids. Determinism: fixed row order, seeded init, no threads; same inputs
/// train same weights on the same build. Errors: bad shapes, empty sets,
/// out-of-range targets, non-finite features or diverged params.
pub fn train_student(
    all_features: &[f32],
    targets: &[u8],
    cfg: &QatConfig,
) -> Result<Vec<f32>, QatErr> {
    let rows = check_train_inputs(all_features, targets, cfg)?;
    let mut weights = init_qat_weights(cfg.input_dim, cfg.output_dim, cfg.seed)?;
    let rows_u16 = u16::try_from(rows).map_err(|_| QatErr::BadLen)?;
    let out_u16 = u16::try_from(cfg.output_dim).map_err(|_| QatErr::BadDim)?;
    let denom = f32::from(rows_u16) * f32::from(out_u16);
    let mse_scale = 2.0f32 / denom;
    for _ in 0..cfg.epochs {
        let w_scale = weight_scale(&weights);
        let mut grads = vec![0.0f32; weights.len()];
        for r in 0..rows {
            let base = r * cfg.input_dim;
            let logits = logits_for_row(
                &weights,
                cfg,
                &all_features[base..base + cfg.input_dim],
                w_scale,
            );
            let mut x = Vec::with_capacity(cfg.input_dim);
            for v in &all_features[base..base + cfg.input_dim] {
                let q = quantize_lane(*v, QAT_INPUT_STEPS);
                x.push(f32::from(q) / QAT_INPUT_STEPS);
            }
            for o in 0..cfg.output_dim {
                let want = if usize::from(targets[r]) == o {
                    1.0f32
                } else {
                    0.0f32
                };
                let diff = logits[o] - want;
                let dlogit = diff * mse_scale;
                for i in 0..cfg.input_dim {
                    grads[o * cfg.input_dim + i] += dlogit * x[i];
                }
            }
        }
        for (w, g) in weights.iter_mut().zip(grads.iter()) {
            *w -= QAT_LR * *g;
            if !w.is_finite() {
                return Err(QatErr::NonFinite);
            }
        }
    }
    Ok(weights)
}

/// Export bias-free float weights as single-dense int8 bytes.
///
/// Units: weights in feature units, input scale is `1/16` real units per
/// step, output scale is identity. Determinism: pure function of the weights
/// plus dims. Errors: bad dims, length mismatch, non-finite params, scale
/// overflow, or bytes the runtime refuses.
pub fn export_qat(weights: &[f32], input_dim: usize, output_dim: usize) -> Result<Vec<u8>, QatErr> {
    if input_dim == 0 || input_dim > MAX_DIM {
        return Err(QatErr::BadDim);
    }
    if output_dim == 0 || output_dim > MAX_DIM {
        return Err(QatErr::BadDim);
    }
    let want = input_dim.checked_mul(output_dim).ok_or(QatErr::BadDim)?;
    if weights.len() != want {
        return Err(QatErr::BadLen);
    }
    for w in weights {
        if !w.is_finite() {
            return Err(QatErr::NonFinite);
        }
    }
    let w_scale = weight_scale(weights);
    if !w_scale.is_finite() || w_scale <= 0.0f32 {
        return Err(QatErr::NonFinite);
    }
    let in_scale = 1.0f32 / QAT_INPUT_STEPS;
    let ratio = f64::from(in_scale) * f64::from(w_scale);
    let (mult, shift) = requant_params(ratio).ok_or(QatErr::ScaleOverflow)?;
    let steps = 1.0f32 / w_scale;
    let mut weights_q = Vec::with_capacity(want);
    for w in weights {
        weights_q.push(quantize_lane(*w, steps));
    }
    let in_u16 = u16::try_from(input_dim).map_err(|_| QatErr::BadDim)?;
    let out_u16 = u16::try_from(output_dim).map_err(|_| QatErr::BadDim)?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(1u8);
    out.extend_from_slice(&in_u16.to_le_bytes());
    out.push(TAG_DENSE);
    out.extend_from_slice(&out_u16.to_le_bytes());
    out.extend_from_slice(&mult.to_le_bytes());
    out.push(shift);
    out.push(ACT_NONE);
    for _ in 0..output_dim {
        out.extend_from_slice(&0i32.to_le_bytes());
    }
    for q in weights_q {
        out.push(q.cast_unsigned());
    }
    load(&out).map_err(QatErr::Rejected)?;
    Ok(out)
}

/// Toolchain receipt for one shipped int8 student.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolchainReport {
    /// FNV-1a 64 of the exact shipped bytes.
    pub digest: u64,
    /// Byte length of the shipped file.
    pub bytes: usize,
    /// Student input lanes.
    pub input_dim: usize,
    /// Student output classes.
    pub output_dim: usize,
    /// Compile target architecture.
    pub arch: &'static str,
    /// Compile target OS.
    pub os: &'static str,
}

impl core::fmt::Display for ToolchainReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "qat student digest={} bytes={} dims={}x{} arch={} os={}",
            self.digest, self.bytes, self.input_dim, self.output_dim, self.arch, self.os
        )
    }
}

/// Build the toolchain receipt for shipped bytes.
///
/// Units: digest over raw bytes, dims are element counts. Determinism: pure
/// function of the bytes plus dims; arch and os are compile-time constants.
/// Errors: none.
pub fn determinism_report(
    student_bytes: &[u8],
    input_dim: usize,
    output_dim: usize,
) -> ToolchainReport {
    ToolchainReport {
        digest: hash_bytes(student_bytes),
        bytes: student_bytes.len(),
        input_dim,
        output_dim,
        arch: std::env::consts::ARCH,
        os: std::env::consts::OS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn teacher_class(features: &[f32]) -> u8 {
        const TEACHER: [[f32; 2]; 2] = [[4.5f32, -3.75f32], [-2.5f32, 5.0f32]];
        let mut best = 0u8;
        let mut top = f32::NEG_INFINITY;
        for (o, row) in TEACHER.iter().enumerate() {
            let mut acc = 0.0f32;
            for (w, v) in row.iter().zip(features.iter()) {
                acc += *w * *v;
            }
            if acc > top {
                top = acc;
                best = u8::try_from(o).unwrap_or(0);
            }
        }
        best
    }

    fn fixture_rows() -> (Vec<f32>, Vec<u8>) {
        let mut rng = Rng::new(2040);
        let mut feats = Vec::new();
        let mut targets = Vec::new();
        for _ in 0..64 {
            let mut row = Vec::new();
            for _ in 0..2 {
                let raw = rng.next_u64() % 65536u64;
                let narrowed = u16::try_from(raw).unwrap_or(0);
                let v = f32::from(narrowed) / 65535.0f32 * 16.0f32 - 8.0f32;
                let snapped = (v * QAT_INPUT_STEPS).round_ties_even() / QAT_INPUT_STEPS;
                row.push(snapped);
            }
            let target = teacher_class(&row);
            feats.extend_from_slice(&row);
            targets.push(target);
        }
        (feats, targets)
    }

    fn argmax_i32(logits: &[i32]) -> Option<usize> {
        let mut best: Option<(usize, i32)> = None;
        for (i, v) in logits.iter().enumerate() {
            match best {
                Some((_, top)) if *v <= top => {}
                _ => best = Some((i, *v)),
            }
        }
        best.map(|(i, _)| i)
    }

    fn agreement(student_bytes: &[u8], feats: &[f32], targets: &[u8]) -> usize {
        let model = crate::load(student_bytes).expect("bytes load");
        let mut hit = 0usize;
        for (r, t) in targets.iter().enumerate() {
            let base = r * 2;
            let mut q = [0i8; 2];
            for i in 0..2 {
                q[i] = quantize_lane(feats[base + i], QAT_INPUT_STEPS);
            }
            let logits = crate::infer(&model, &q);
            if argmax_i32(&logits) == Some(usize::from(*t)) {
                hit += 1;
            }
        }
        hit
    }

    #[test]
    fn simulate_quantize_goldens_and_edges() {
        assert_eq!(simulate_quantize(1.0f32, 0.5f32), 1.0f32);
        assert_eq!(simulate_quantize(1.3f32, 0.5f32), 1.5f32);
        assert_eq!(simulate_quantize(-1.3f32, 0.5f32), -1.5f32);
        assert_eq!(simulate_quantize(f32::NAN, 0.1f32), 0.0f32);
        assert_eq!(simulate_quantize(1.0f32, 0.0f32), 0.0f32);
        assert_eq!(simulate_quantize(1.0f32, -0.5f32), 0.0f32);
        assert_eq!(simulate_quantize(1.0f32, f32::INFINITY), 0.0f32);
        assert_eq!(simulate_quantize(1000.0f32, 0.1f32), 12.7f32);
        assert_eq!(simulate_quantize(-1000.0f32, 0.1f32), -12.7f32);
    }

    #[test]
    fn config_and_inputs_refused() {
        assert_eq!(QatConfig::new(0, 2, 1, 10, true), Err(QatErr::BadDim));
        assert_eq!(QatConfig::new(2, 0, 1, 10, false), Err(QatErr::BadDim));
        assert_eq!(QatConfig::new(2, 2, 1, 0, true), Err(QatErr::BadEpochs));
        let cfg = QatConfig::new(2, 2, 7, 4, true).expect("legal");
        assert_eq!(train_student(&[], &[], &cfg), Err(QatErr::EmptyDataset));
        assert_eq!(
            train_student(&[1.0f32; 3], &[0u8], &cfg),
            Err(QatErr::BadLen)
        );
        assert_eq!(
            train_student(&[1.0f32, 2.0f32], &[9u8], &cfg),
            Err(QatErr::BadTarget)
        );
        assert_eq!(
            train_student(&[f32::NAN, 0.0f32], &[0u8], &cfg),
            Err(QatErr::NonFinite)
        );
        assert_eq!(export_qat(&[1.0f32; 3], 2, 2), Err(QatErr::BadLen));
        assert_eq!(export_qat(&[], 0, 2), Err(QatErr::BadDim));
    }

    #[test]
    fn qat_beats_post_training_on_fixture() {
        let (feats, targets) = fixture_rows();
        let ptq_cfg = QatConfig::new(2, 2, 99, 600, false).expect("legal");
        let qat_cfg = QatConfig::new(2, 2, 99, 600, true).expect("legal");
        let ptq_w = train_student(&feats, &targets, &ptq_cfg).expect("trains");
        let qat_w = train_student(&feats, &targets, &qat_cfg).expect("trains");
        let ptq_bytes = export_qat(&ptq_w, 2, 2).expect("exports");
        let qat_bytes = export_qat(&qat_w, 2, 2).expect("exports");
        let ptq_hit = agreement(&ptq_bytes, &feats, &targets);
        let qat_hit = agreement(&qat_bytes, &feats, &targets);
        assert!(
            qat_hit > ptq_hit,
            "qat {qat_hit} must beat ptq {ptq_hit} on 64 rows"
        );
    }

    #[test]
    fn int8_student_is_bit_identical_with_report() {
        let (feats, targets) = fixture_rows();
        let cfg = QatConfig::new(2, 2, 99, 600, true).expect("legal");
        let first_w = train_student(&feats, &targets, &cfg).expect("trains");
        let second_w = train_student(&feats, &targets, &cfg).expect("trains again");
        assert_eq!(first_w, second_w);
        let first = export_qat(&first_w, 2, 2).expect("exports");
        let second = export_qat(&second_w, 2, 2).expect("exports again");
        assert_eq!(first, second);
        let report = determinism_report(&first, 2, 2);
        let again = determinism_report(&second, 2, 2);
        assert_eq!(report, again);
        assert_eq!(report.digest, hash_bytes(&first));
        assert!(!report.arch.is_empty());
        assert!(!report.os.is_empty());
        let text = report.to_string();
        assert!(text.contains("digest="));
        assert!(text.contains("arch="));
    }

    #[test]
    fn exported_bytes_run_in_tick_runtime() {
        let (feats, targets) = fixture_rows();
        let cfg = QatConfig::new(2, 2, 11, 50, true).expect("legal");
        let weights = train_student(&feats, &targets, &cfg).expect("trains");
        let bytes = export_qat(&weights, 2, 2).expect("exports");
        let model = crate::load(&bytes).expect("loads");
        assert_eq!((model.input_dim(), model.output_dim()), (2, 2));
        let mut scratch = crate::Scratch::for_model(&model);
        let mut out = [0i32; 2];
        let q = [10i8, -20i8];
        crate::infer_into(&model, &q, &mut scratch, &mut out).expect("tick infer");
        for v in out {
            assert!((-128..=127).contains(&v));
        }
        let (result, count) = engine_core::alloc_probe::measure(|| {
            crate::infer_into(&model, &q, &mut scratch, &mut out)
        });
        assert_eq!(result, Ok(()));
        assert_eq!(count.calls, 0);
    }
}
