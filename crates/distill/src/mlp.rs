//! Two-layer student and its std-only trainer (no burn).
//!
//! Consumer: the `distill_duel` generator fits one [`Mlp`] on teacher rows
//! and hands it to [`export_mlp`](crate::export_mlp). Owner: this module owns
//! the two-layer float parameter layout, the softmax cross-entropy loss and
//! the optimizer. Units: inputs are feature-unit floats
//! ([`Sample::quantized_features`], `1/16` steps); hidden units are ReLU;
//! logits are unitless. Determinism scope: weight init and the per-epoch
//! batch order come from `engine_core::Rng` seeded by the spec; every sum
//! runs in fixed order on one thread, so the same rows plus the same
//! [`MlpSpec`] fit the same params on the same machine (same binary). The
//! float bits are not claimed across machines (`f32::exp` is the platform
//! libm); the exported `i8` net is the portable artifact.
//! Errors: bad specs, rows and provenance map to [`FitError`]; never panics.
//!
//! Why not the burn trainer: it fits one dense layer with MSE (its own docs
//! explain why cross-entropy drifts there). A distilled policy needs a hidden
//! layer and a classification loss (gogame 6.5.1 softmax, 7 supervised
//! policy), which is ~150 lines of plain Rust: mini-batch softmax
//! cross-entropy with Adam.

use crate::data::{check_dataset, check_provenance, DataErrorAt, DatasetManifest, ProvenanceError};
use crate::data::{Sample, INPUT_QUANT_SCALE, MAX_FEATURE_ABS};
use crate::spec::{StudentSpec, MAX_EPOCHS};
use engine_core::Rng;

/// Largest hidden width accepted (the runtime dense cap is far above this;
/// the tick budget is the real limit).
pub const MAX_HIDDEN: usize = 256;

/// Shape plus optimizer settings for one two-layer student.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MlpSpec {
    /// Input/output dims and the seed.
    pub student: StudentSpec,
    /// Hidden ReLU width, `1..=MAX_HIDDEN`.
    pub hidden: usize,
    /// Passes over the rows, `1..=MAX_EPOCHS`.
    pub epochs: u32,
    /// Rows per Adam step, `>= 1` (the last batch of an epoch may be short).
    pub batch: usize,
    /// Adam step size, finite and positive.
    pub learning_rate: f32,
}

/// Two-layer float student: `logits = W2 . relu(W1 . x + b1) + b2`.
/// Weights are row-major: `w1` is `hidden x input_dim`, `w2` is
/// `output_dim x hidden`. Params are in feature units.
#[derive(Clone, Debug, PartialEq)]
pub struct Mlp {
    /// Spec this student fitted under.
    pub spec: MlpSpec,
    /// Hidden-layer weights, `hidden * input_dim`.
    pub w1: Vec<f32>,
    /// Hidden-layer bias, `hidden`.
    pub b1: Vec<f32>,
    /// Output-layer weights, `output_dim * hidden`.
    pub w2: Vec<f32>,
    /// Output-layer bias, `output_dim`.
    pub b2: Vec<f32>,
}

/// Fit failures.
#[derive(Clone, Debug, PartialEq)]
pub enum FitError {
    /// Dataset provenance missing or unshippable.
    Provenance(ProvenanceError),
    /// No rows.
    EmptyDataset,
    /// A row failed the spec; carries the row index.
    BadData(DataErrorAt),
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

impl core::fmt::Display for FitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FitError::Provenance(e) => write!(f, "dataset provenance refused: {e}"),
            FitError::EmptyDataset => f.write_str("empty dataset"),
            FitError::BadData(e) => write!(f, "bad dataset: {e}"),
            FitError::BadHidden(h) => write!(f, "hidden width {h} outside 1..={MAX_HIDDEN}"),
            FitError::BadEpochs(n) => write!(f, "epoch count {n} outside 1..={MAX_EPOCHS}"),
            FitError::BadBatch => f.write_str("batch of zero rows"),
            FitError::BadLearningRate => f.write_str("learning rate not finite and positive"),
            FitError::Diverged => f.write_str("fit diverged to non-finite params"),
        }
    }
}

impl std::error::Error for FitError {}

impl Mlp {
    /// Forward one row: `hidden` gets the ReLU activations, `logits` the
    /// outputs. `None` (buffers untouched) on any length mismatch.
    pub fn forward(&self, x: &[f32], hidden: &mut [f32], logits: &mut [f32]) -> Option<()> {
        let (inp, hid, out) = self.dims();
        if x.len() != inp || hidden.len() != hid || logits.len() != out {
            return None;
        }
        for (h, slot) in hidden.iter_mut().enumerate() {
            let row = &self.w1[h * inp..(h + 1) * inp];
            let mut acc = self.b1[h];
            for (w, v) in row.iter().zip(x) {
                acc += w * v;
            }
            *slot = acc.max(0.0);
        }
        for (o, slot) in logits.iter_mut().enumerate() {
            let row = &self.w2[o * hid..(o + 1) * hid];
            let mut acc = self.b2[o];
            for (w, v) in row.iter().zip(hidden.iter()) {
                acc += w * v;
            }
            *slot = acc;
        }
        Some(())
    }

    /// `(input_dim, hidden, output_dim)`.
    pub fn dims(&self) -> (usize, usize, usize) {
        (
            self.spec.student.dims.input_dim,
            self.spec.hidden,
            self.spec.student.dims.output_dim,
        )
    }

    /// Argmax agreement with the row targets (first max wins), as
    /// `(agree, total)`.
    pub fn agreement(&self, rows: &[Sample]) -> (usize, usize) {
        let (_, hid, out) = self.dims();
        let mut hidden = vec![0.0f32; hid];
        let mut logits = vec![0.0f32; out];
        let mut agree = 0usize;
        for row in rows {
            if self
                .forward(&row.quantized_features(), &mut hidden, &mut logits)
                .is_some()
                && first_max_f32(&logits) == usize::from(row.target)
            {
                agree += 1;
            }
        }
        (agree, rows.len())
    }
}

/// First-max argmax over floats (strict `>`).
pub(crate) fn first_max_f32(v: &[f32]) -> usize {
    let mut best = 0usize;
    for (i, x) in v.iter().enumerate() {
        if *x > v[best] {
            best = i;
        }
    }
    best
}

/// Count as a float for averages and init limits. Counts here are row and
/// unit totals far below 2^24, so the conversion is exact.
pub(crate) fn count_f32(n: usize) -> f32 {
    n as f32
}

/// Uniform `[-limit, limit)` from the engine rng (24-bit mantissa draw).
pub(crate) fn uniform(rng: &mut Rng, limit: f32) -> f32 {
    let bits = usize::try_from(rng.next_u64() >> 40).unwrap_or(0);
    let unit = count_f32(bits) / count_f32(1 << 24);
    (unit * 2.0 - 1.0) * limit
}

/// Fisher-Yates a batch order with the seeded engine rng: same seed, same
/// order. Shared by the one-hot and the continuous fit, so both draw their
/// epoch shuffle from the spec seed in the same way.
pub(crate) fn shuffle_order(rng: &mut Rng, order: &mut [usize]) {
    for i in (1..order.len()).rev() {
        let bound = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
        let j = usize::try_from(rng.next_range(0, bound)).unwrap_or(0);
        order.swap(i, j);
    }
}

/// Adam moment buffers for one param tensor.
pub(crate) struct Adam {
    m: Vec<f32>,
    v: Vec<f32>,
}

impl Adam {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            m: vec![0.0; n],
            v: vec![0.0; n],
        }
    }

    /// One Adam update with bias correction `c1 = 1 - b1^t`, `c2 = 1 - b2^t`.
    pub(crate) fn step(&mut self, params: &mut [f32], grads: &[f32], lr: f32, c1: f32, c2: f32) {
        const B1: f32 = 0.9;
        const B2: f32 = 0.999;
        const EPS: f32 = 1e-8;
        for ((p, g), (m, v)) in params
            .iter_mut()
            .zip(grads)
            .zip(self.m.iter_mut().zip(self.v.iter_mut()))
        {
            *m = B1 * *m + (1.0 - B1) * g;
            *v = B2 * *v + (1.0 - B2) * g * g;
            let mh = *m / c1;
            let vh = *v / c2;
            *p -= lr * mh / (vh.sqrt() + EPS);
        }
    }
}

/// Gradient accumulators plus per-row work buffers for one batch. Shared by
/// both fits on this net shape: the one-hot softmax row and the continuous
/// squared-error row accumulate into the same tables.
pub(crate) struct Grads {
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    hidden: Vec<f32>,
    logits: Vec<f32>,
    d_hidden: Vec<f32>,
}

impl Grads {
    pub(crate) fn new(inp: usize, hid: usize, out: usize) -> Self {
        Self {
            w1: vec![0.0; hid * inp],
            b1: vec![0.0; hid],
            w2: vec![0.0; out * hid],
            b2: vec![0.0; out],
            hidden: vec![0.0; hid],
            logits: vec![0.0; out],
            d_hidden: vec![0.0; hid],
        }
    }

    pub(crate) fn clear(&mut self) {
        for t in [&mut self.w1, &mut self.b1, &mut self.w2, &mut self.b2] {
            t.fill(0.0);
        }
    }

    /// The four gradient tables in param order, so a fit can hand each one to
    /// its own Adam pass without reaching into the accumulator's fields.
    pub(crate) fn tables(&self) -> (&Vec<f32>, &Vec<f32>, &Vec<f32>, &Vec<f32>) {
        (&self.w1, &self.b1, &self.w2, &self.b2)
    }

    /// Add one row's softmax cross-entropy gradient, weighted by `scale`.
    fn add_row(&mut self, mlp: &Mlp, x: &[f32], target: usize, scale: f32) -> Option<()> {
        let (inp, hid, _) = mlp.dims();
        mlp.forward(x, &mut self.hidden, &mut self.logits)?;
        // Softmax with max shift; the logit gradient is p - onehot.
        let top = self.logits[first_max_f32(&self.logits)];
        let mut sum = 0.0f32;
        for l in self.logits.iter_mut() {
            *l = (*l - top).exp();
            sum += *l;
        }
        self.d_hidden.fill(0.0);
        for (o, l) in self.logits.iter().enumerate() {
            let d = (l / sum - if o == target { 1.0 } else { 0.0 }) * scale;
            self.b2[o] += d;
            let row = o * hid..(o + 1) * hid;
            for ((g, w), (h, dh)) in self.w2[row.clone()]
                .iter_mut()
                .zip(&mlp.w2[row])
                .zip(self.hidden.iter().zip(self.d_hidden.iter_mut()))
            {
                *g += d * h;
                *dh += d * w;
            }
        }
        for (h, (act, d)) in self.hidden.iter().zip(&self.d_hidden).enumerate() {
            if *act <= 0.0 {
                continue;
            }
            self.b1[h] += d;
            for (g, v) in self.w1[h * inp..(h + 1) * inp].iter_mut().zip(x) {
                *g += d * v;
            }
        }
        Some(())
    }

    /// Add one row's squared-error gradient, weighted by `scale`. The output
    /// layer is linear, so its lane derivative is just `pred - target`; the
    /// ReLU gate below it drops every hidden unit that did not fire.
    /// `None` when the row's lengths do not fit the net.
    pub(crate) fn add_mse_row(
        &mut self,
        mlp: &Mlp,
        x: &[f32],
        target: &[f32],
        scale: f32,
    ) -> Option<()> {
        let (inp, hid, out) = mlp.dims();
        mlp.forward(x, &mut self.hidden, &mut self.logits)?;
        if target.len() != out {
            return None;
        }
        self.d_hidden.fill(0.0);
        for (o, (pred, want)) in self.logits.iter().zip(target).enumerate() {
            let d = (pred - want) * scale;
            self.b2[o] += d;
            let row = o * hid..(o + 1) * hid;
            for ((g, w), (h, dh)) in self.w2[row.clone()]
                .iter_mut()
                .zip(&mlp.w2[row])
                .zip(self.hidden.iter().zip(self.d_hidden.iter_mut()))
            {
                *g += d * h;
                *dh += d * w;
            }
        }
        for (h, (act, d)) in self.hidden.iter().zip(&self.d_hidden).enumerate() {
            if *act <= 0.0 {
                continue;
            }
            self.b1[h] += d;
            for (g, v) in self.w1[h * inp..(h + 1) * inp].iter_mut().zip(x) {
                *g += d * v;
            }
        }
        Some(())
    }
}

/// Spec and dataset checks shared by every fit; provenance first.
fn check_fit(
    dataset: &[Sample],
    spec: &MlpSpec,
    manifest: Option<&DatasetManifest>,
) -> Result<(), FitError> {
    check_provenance(manifest).map_err(FitError::Provenance)?;
    if spec.hidden == 0 || spec.hidden > MAX_HIDDEN {
        return Err(FitError::BadHidden(spec.hidden));
    }
    if spec.epochs == 0 || spec.epochs > MAX_EPOCHS {
        return Err(FitError::BadEpochs(spec.epochs));
    }
    if spec.batch == 0 {
        return Err(FitError::BadBatch);
    }
    if !spec.learning_rate.is_finite() || spec.learning_rate <= 0.0 {
        return Err(FitError::BadLearningRate);
    }
    if dataset.is_empty() {
        return Err(FitError::EmptyDataset);
    }
    check_dataset(dataset, &spec.student).map_err(FitError::BadData)
}

/// Fit a two-layer student with mini-batch softmax cross-entropy and Adam.
/// Inputs are scaled by `1 / MAX_FEATURE_ABS` inside the loop for
/// conditioning and the scale is folded back into `w1` on return (a power of
/// two, so exact). Seeded by `spec.student.seed`; see the module docs for
/// the determinism scope. Refuses missing provenance before any math runs.
pub fn fit_mlp(
    dataset: &[Sample],
    spec: &MlpSpec,
    manifest: Option<&DatasetManifest>,
) -> Result<Mlp, FitError> {
    check_fit(dataset, spec, manifest)?;
    let inp = spec.student.dims.input_dim;
    let hid = spec.hidden;
    let out = spec.student.dims.output_dim;
    let inv_scale = 1.0 / MAX_FEATURE_ABS;
    // Flat scaled inputs: one `inp`-wide row per dataset row in order.
    // Lane `r * inp + i` equals the old `quantized_features()[i] * inv_scale`
    // bit-for-bit (same `round(v * 16) / 16 * inv_scale` lane math, folded
    // into one pass with one allocation instead of one `Vec` per row plus the
    // outer `Vec`). The epoch loop below slices rows out of this buffer, so
    // training sees contiguous batches with no per-row indirection.
    let mut xs_flat: Vec<f32> = Vec::with_capacity(dataset.len() * inp);
    for row in dataset {
        for v in &row.features {
            xs_flat.push((v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE * inv_scale);
        }
    }

    let mut rng = Rng::new(spec.student.seed);
    let lim1 = (6.0 / count_f32(inp + hid)).sqrt();
    let lim2 = (6.0 / count_f32(hid + out)).sqrt();
    let mut mlp = Mlp {
        spec: *spec,
        w1: (0..hid * inp).map(|_| uniform(&mut rng, lim1)).collect(),
        b1: vec![0.0; hid],
        w2: (0..out * hid).map(|_| uniform(&mut rng, lim2)).collect(),
        b2: vec![0.0; out],
    };
    let mut opt = [
        Adam::new(hid * inp),
        Adam::new(hid),
        Adam::new(out * hid),
        Adam::new(out),
    ];
    let mut grads = Grads::new(inp, hid, out);
    let mut order: Vec<usize> = (0..dataset.len()).collect();
    let (mut pow1, mut pow2) = (1.0f32, 1.0f32);
    for _ in 0..spec.epochs {
        shuffle_order(&mut rng, &mut order);
        for batch in order.chunks(spec.batch) {
            grads.clear();
            let scale = 1.0 / count_f32(batch.len());
            for &r in batch {
                let target = usize::from(dataset[r].target);
                let x = &xs_flat[r * inp..(r + 1) * inp];
                grads
                    .add_row(&mlp, x, target, scale)
                    .ok_or(FitError::Diverged)?;
            }
            pow1 *= 0.9;
            pow2 *= 0.999;
            let (c1, c2, lr) = (1.0 - pow1, 1.0 - pow2, spec.learning_rate);
            opt[0].step(&mut mlp.w1, &grads.w1, lr, c1, c2);
            opt[1].step(&mut mlp.b1, &grads.b1, lr, c1, c2);
            opt[2].step(&mut mlp.w2, &grads.w2, lr, c1, c2);
            opt[3].step(&mut mlp.b2, &grads.b2, lr, c1, c2);
        }
    }
    for w in mlp.w1.iter_mut() {
        *w *= inv_scale;
    }
    let finite = [&mlp.w1, &mlp.b1, &mlp.w2, &mlp.b2]
        .iter()
        .all(|t| t.iter().all(|v| v.is_finite()));
    if !finite {
        return Err(FitError::Diverged);
    }
    Ok(mlp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// XOR-like 2-lane rule a single dense layer cannot fit: class 1 when
    /// the lanes share a sign. Rows on exact 1/16 steps.
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

    fn spec() -> MlpSpec {
        MlpSpec {
            student: StudentSpec::new(2, 2, 11).expect("legal"),
            hidden: 8,
            epochs: 60,
            batch: 32,
            learning_rate: 0.02,
        }
    }

    #[test]
    fn fits_a_rule_one_layer_cannot_and_repeats_bit_exact() {
        let spec = spec();
        let rows = xor_rows(&spec.student);
        let manifest = DatasetManifest::fixture();
        let a = fit_mlp(&rows, &spec, Some(&manifest)).expect("fits");
        let b = fit_mlp(&rows, &spec, Some(&manifest)).expect("fits again");
        assert_eq!(a, b, "same rows and seed give the same params");
        let (agree, total) = a.agreement(&rows);
        assert!(agree * 100 >= total * 90, "xor agreement {agree}/{total}");
        // A different seed is a different student.
        let mut other = spec;
        other.student.seed = 12;
        assert_ne!(fit_mlp(&rows, &other, Some(&manifest)).expect("fits"), a);
    }

    #[test]
    fn bad_specs_and_rows_refused() {
        let spec = spec();
        let rows = xor_rows(&spec.student);
        let m = DatasetManifest::fixture();
        assert_eq!(
            fit_mlp(&rows, &spec, None),
            Err(FitError::Provenance(ProvenanceError::MissingManifest))
        );
        assert_eq!(fit_mlp(&[], &spec, Some(&m)), Err(FitError::EmptyDataset));
        let mut bad = spec;
        bad.hidden = 0;
        assert_eq!(fit_mlp(&rows, &bad, Some(&m)), Err(FitError::BadHidden(0)));
        bad = spec;
        bad.epochs = 0;
        assert_eq!(fit_mlp(&rows, &bad, Some(&m)), Err(FitError::BadEpochs(0)));
        bad = spec;
        bad.batch = 0;
        assert_eq!(fit_mlp(&rows, &bad, Some(&m)), Err(FitError::BadBatch));
        bad = spec;
        bad.learning_rate = f32::NAN;
        assert_eq!(
            fit_mlp(&rows, &bad, Some(&m)),
            Err(FitError::BadLearningRate)
        );
        let mut wrong = rows.clone();
        wrong[5].target = 9;
        match fit_mlp(&wrong, &spec, Some(&m)) {
            Err(FitError::BadData(at)) => assert_eq!(at.index, 5),
            other => panic!("want BadData, got {other:?}"),
        }
    }

    #[test]
    fn forward_refuses_wrong_lengths() {
        let spec = spec();
        let rows = xor_rows(&spec.student);
        let mlp = fit_mlp(
            &rows[..8],
            &MlpSpec { epochs: 1, ..spec },
            Some(&DatasetManifest::fixture()),
        )
        .expect("fits");
        let mut h = [0.0f32; 8];
        let mut l = [0.0f32; 2];
        assert_eq!(mlp.forward(&[0.0], &mut h, &mut l), None);
        assert_eq!(mlp.forward(&[0.0, 0.0], &mut h[..7], &mut l), None);
        assert_eq!(mlp.forward(&[0.0, 0.0], &mut h, &mut l[..1]), None);
        assert_eq!(mlp.forward(&[0.0, 0.0], &mut h, &mut l), Some(()));
    }

    #[test]
    fn flat_scaled_inputs_match_rowwise_oracle_bit_exact() {
        // Oracle: each flat lane equals the old row-wise
        // `quantized_features()[i] * inv_scale` bit-for-bit. The fold drops
        // the per-row `Vec`, never the lane math.
        let s = spec();
        let rows = xor_rows(&s.student);
        let inv_scale = 1.0 / MAX_FEATURE_ABS;
        let inp = s.student.dims.input_dim;
        let mut flat = Vec::with_capacity(rows.len() * inp);
        for row in &rows {
            for v in &row.features {
                flat.push((v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE * inv_scale);
            }
        }
        for (r, row) in rows.iter().enumerate() {
            let oracle: Vec<f32> = row
                .quantized_features()
                .iter()
                .map(|v| v * inv_scale)
                .collect();
            let slice = &flat[r * inp..(r + 1) * inp];
            assert_eq!(slice.len(), oracle.len());
            for (got, want) in slice.iter().zip(oracle.iter()) {
                assert_eq!(got.to_bits(), want.to_bits(), "flat lane differs");
            }
        }
        // Single-multiply fold is exact too (both scales are powers of two).
        let folded_scale = 1.0 / (INPUT_QUANT_SCALE * MAX_FEATURE_ABS);
        for row in &rows {
            for v in &row.features {
                let folded = (v * INPUT_QUANT_SCALE).round() * folded_scale;
                let direct = (v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE * inv_scale;
                assert_eq!(folded.to_bits(), direct.to_bits(), "fold differs");
            }
        }
    }
}
