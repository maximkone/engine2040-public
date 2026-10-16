//! engine-nn: integer-only neural runtime for `.e2nn` files.
//!
//! Consumer: gameplay brains (arena policies) running inside the fixed tick.
//! Owner: this crate owns the `.e2nn` byte layout, the integer math, and the errors.
//! Units: weights are `i8`; activations are `i8`-range values carried as `i32`;
//! accumulators are saturating `i32`; requantization is an integer multiplier
//! plus an arithmetic right shift with round-half-away-from-zero.
//! Determinism: same bytes plus same input give the same output on every
//! machine. Integer-only math, no SIMD paths, no hash-map iteration, no wall clock.
//! Errors: malformed bytes give [`NnErr`]; [`load`] and [`infer`] never panic.
//!
//! ## `.e2nn` layout (all multi-byte integers little-endian)
//!
//! ```text
//! header: MAGIC[4] version[u8=1] layer_count[u8 1..=16] input_dim[u16 1..=1024]
//! per layer: tag[u8] body...
//!   tag 1 dense: out_dim[u16] mult[i32] shift[u8 0..=31] act[u8 0..=4]
//!     bias[out_dim x i32] weights[out_dim x in_dim x i8, row-major]
//!     + lut[256 x i8] only when act == 4
//!   tag 2 embedding: vocab[u16 1..=4096] emb_dim[u16] mult[i32] shift[u8] act[u8]
//!     table[vocab x emb_dim x i8, row-major] + lut[256 x i8] only when act == 4
//!   tag 3 gru: hidden[u16 1..=512] mult[i32] shift[u8]
//!     z: bias[hidden x i32] w_ih[hidden x in_dim x i8] w_hh[hidden x hidden x i8]
//!     r: same shape as z
//!     n: same shape as z
//! ```
//!
//! Layer zero reads `in_dim = input_dim` values; every later layer reads the
//! previous layer's outputs. Every layer output saturates to `[-128, 127]`.
//! Dense sums `bias + W.x` in saturating `i32`, requantizes, then activates.
//! Embedding sums one table row per input token, requantizes, then activates;
//! any input value is a token id wrapped with `rem_euclid(vocab)`.
//! GRU runs one step from a zero hidden state with fixed sigmoid/tanh tables.
//! Short inputs are zero-padded, long inputs are truncated, so [`infer`] is total.
//!
//! ## Tick path
//!
//! [`infer_into`] is the in-tick entry: caller-owned [`Scratch`] plus a
//! caller-owned output slice, zero heap allocation per call (proven by the
//! `engine_core` alloc probe). [`infer`] is the allocating convenience
//! wrapper for tools and tests. Float observations reach the net only through
//! [`quantize_lane`]/[`quantize_lanes`], the one quantization edge. A loaded
//! [`Model`] carries [`Model::digest`], the FNV-1a 64 of its file bytes, so a
//! replay tape can pin exactly which net decided.

// Unit tests prove the zero-allocation tick path with the shared probe.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

/// File magic. Own value, never reused from another crate.
pub const MAGIC: [u8; 4] = *b"E2NN";
// Conversion-time only: ONNX int8-QDQ import plus the tract float reference.
// The integer runtime in this file stays std-only; tract never enters the tick.
#[cfg(feature = "convert")]
pub mod convert;
#[cfg(feature = "convert")]
pub use convert::{
    convert, reference_check, ConvertErr, E2nnBytes, ReferenceReport, Whitelist, MAX_GRAPH_INPUTS,
    MAX_INITIALIZERS, MAX_NODES, MAX_ONNX_BYTES, MAX_RANK, MAX_TENSOR_ELEMENTS,
    REFERENCE_TOLERANCE,
};
mod quant;
pub use quant::{quantize_lane, quantize_lanes, QUANT_MAX, QUANT_MIN, QUANT_NAN};
mod spike;
pub use spike::{
    spike_infer, spike_tick, SpikeErr, SPIKE_BIAS, SPIKE_INPUT_DIM, SPIKE_OUTPUT_DIM, SPIKE_WEIGHTS,
};

/// The only file version this runtime reads.
pub const VERSION: u8 = 1;
/// Largest layer count accepted.
pub const MAX_LAYERS: usize = 16;
/// Largest input/output/embedding dim accepted.
pub const MAX_DIM: usize = 1024;
/// Largest embedding vocabulary accepted.
pub const MAX_VOCAB: usize = 4096;
/// Largest GRU hidden dim accepted.
pub const MAX_HIDDEN: usize = 512;
/// Largest single weight/table element count accepted.
pub const MAX_PARAMS: usize = 4_194_304;

/// Layer tag for [`DenseDesc`].
pub const TAG_DENSE: u8 = 1;
/// Layer tag for [`EmbeddingDesc`].
pub const TAG_EMBEDDING: u8 = 2;
/// Layer tag for [`GruDesc`].
pub const TAG_GRU: u8 = 3;

/// Activation code: identity, then saturate to `[-128, 127]`.
pub const ACT_NONE: u8 = 0;
/// Activation code: `max(0, x)`, then saturate.
pub const ACT_RELU: u8 = 1;
/// Activation code: saturating integer tanh from [`TANH_TABLE`].
pub const ACT_TANH: u8 = 2;
/// Activation code: integer sigmoid from [`SIGMOID_TABLE`], range `0..=127`.
pub const ACT_SIGMOID: u8 = 3;
/// Activation code: custom 256-entry table stored in the file.
pub const ACT_LUT: u8 = 4;

/// Load/inference failures. Malformed input is always an error, never a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NnErr {
    /// First four bytes are not [`MAGIC`].
    BadMagic,
    /// Version byte is not [`VERSION`].
    BadVersion,
    /// Bytes run out mid-header or mid-layer.
    Truncated,
    /// Layer count is zero or above [`MAX_LAYERS`].
    BadLayerCount,
    /// A dim (input, output, vocab, hidden) is zero or above its cap.
    BadDim,
    /// Unknown layer tag byte.
    BadLayerTag,
    /// Requant shift is above 31.
    BadShift,
    /// Activation code is above [`ACT_LUT`].
    BadActivation,
    /// A weight/table element count overflows or passes [`MAX_PARAMS`].
    TooLarge,
    /// Bytes remain after the last declared layer.
    TrailingBytes,
    /// [`infer_into`] output slice length differs from [`Model::output_dim`].
    OutputLen,
    /// [`infer_into`] scratch was sized for a smaller model; build it with
    /// [`Scratch::for_model`].
    ScratchTooSmall,
}

impl core::fmt::Display for NnErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            NnErr::BadMagic => "bad magic",
            NnErr::BadVersion => "bad version",
            NnErr::Truncated => "truncated file",
            NnErr::BadLayerCount => "bad layer count",
            NnErr::BadDim => "bad dim",
            NnErr::BadLayerTag => "bad layer tag",
            NnErr::BadShift => "bad shift",
            NnErr::BadActivation => "bad activation",
            NnErr::TooLarge => "layer too large",
            NnErr::TrailingBytes => "trailing bytes",
            NnErr::OutputLen => "output slice length differs from the model output dim",
            NnErr::ScratchTooSmall => "scratch sized for a smaller model",
        };
        f.write_str(s)
    }
}

impl std::error::Error for NnErr {}

/// Dense layer: `y = act(requant(bias + W.x))`, weights row-major `i8`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DenseDesc {
    out_dim: usize,
    mult: i32,
    shift: u8,
    act: u8,
    bias: Vec<i32>,
    weights: Vec<i8>,
    lut: Option<Box<[i8; 256]>>,
}

/// Embedding layer: `y = act(requant(sum of one table row per token))`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingDesc {
    vocab: usize,
    emb_dim: usize,
    mult: i32,
    shift: u8,
    act: u8,
    table: Vec<i8>,
    lut: Option<Box<[i8; 256]>>,
}

/// Integer GRU layer: one step from a zero hidden state, fixed gate tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GruDesc {
    hidden: usize,
    mult: i32,
    shift: u8,
    bz: Vec<i32>,
    wz_ih: Vec<i8>,
    wz_hh: Vec<i8>,
    br: Vec<i32>,
    wr_ih: Vec<i8>,
    wr_hh: Vec<i8>,
    bn: Vec<i32>,
    wn_ih: Vec<i8>,
    wn_hh: Vec<i8>,
}

/// One network layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Fully connected int8 layer.
    Dense(DenseDesc),
    /// Token-id table lookup plus sum.
    Embedding(EmbeddingDesc),
    /// Single-step integer GRU.
    Gru(GruDesc),
}

/// A loaded `.e2nn` network. Built only by [`load`], so dims always agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    input_dim: usize,
    layers: Vec<Layer>,
    digest: u64,
}

impl DenseDesc {
    /// Output dim (row count of `weights`).
    pub fn out_dim(&self) -> usize {
        self.out_dim
    }
    /// Requant multiplier.
    pub fn mult(&self) -> i32 {
        self.mult
    }
    /// Requant arithmetic right shift.
    pub fn shift(&self) -> u8 {
        self.shift
    }
    /// Activation code (`ACT_*`).
    pub fn act(&self) -> u8 {
        self.act
    }
    /// Per-output bias.
    pub fn bias(&self) -> &[i32] {
        &self.bias
    }
    /// Row-major weights, `out_dim x in_dim`.
    pub fn weights(&self) -> &[i8] {
        &self.weights
    }
    /// Custom table when [`ACT_LUT`] is set.
    pub fn lut(&self) -> Option<&[i8; 256]> {
        self.lut.as_deref()
    }
}

impl EmbeddingDesc {
    /// Vocabulary size (table row count).
    pub fn vocab(&self) -> usize {
        self.vocab
    }
    /// Output dim (table column count).
    pub fn emb_dim(&self) -> usize {
        self.emb_dim
    }
    /// Requant multiplier.
    pub fn mult(&self) -> i32 {
        self.mult
    }
    /// Requant arithmetic right shift.
    pub fn shift(&self) -> u8 {
        self.shift
    }
    /// Activation code (`ACT_*`).
    pub fn act(&self) -> u8 {
        self.act
    }
    /// Row-major table, `vocab x emb_dim`.
    pub fn table(&self) -> &[i8] {
        &self.table
    }
    /// Custom table when [`ACT_LUT`] is set.
    pub fn lut(&self) -> Option<&[i8; 256]> {
        self.lut.as_deref()
    }
}

impl GruDesc {
    /// Hidden dim (also the output dim).
    pub fn hidden(&self) -> usize {
        self.hidden
    }
    /// Shared gate/candidate requant multiplier.
    pub fn mult(&self) -> i32 {
        self.mult
    }
    /// Shared gate/candidate requant shift.
    pub fn shift(&self) -> u8 {
        self.shift
    }
}

impl Layer {
    /// Output dim of this layer.
    pub fn out_dim(&self) -> usize {
        match self {
            Layer::Dense(d) => d.out_dim,
            Layer::Embedding(e) => e.emb_dim,
            Layer::Gru(g) => g.hidden,
        }
    }
}

impl Model {
    /// Expected input length (short pads with zero, long truncates).
    pub fn input_dim(&self) -> usize {
        self.input_dim
    }
    /// Layers in apply order.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    /// Output length of [`infer`]: the last layer's output dim.
    pub fn output_dim(&self) -> usize {
        self.layers.last().map_or(0, Layer::out_dim)
    }
    /// FNV-1a 64 (`engine_core::hash::hash_bytes`, the engine's one content
    /// hash) of the exact `.e2nn` bytes this model loaded from: the value a
    /// tape header pins.
    pub fn digest(&self) -> u64 {
        self.digest
    }
    /// Widest activation vector any layer reads or writes (input included):
    /// the per-buffer length [`Scratch::for_model`] reserves.
    pub fn max_width(&self) -> usize {
        self.layers
            .iter()
            .map(Layer::out_dim)
            .fold(self.input_dim, usize::max)
    }
}

/// Caller-owned activation buffers for [`infer_into`]: two ping-pong rows of
/// [`Model::max_width`] lanes. Build once per model (allocates), then reuse
/// every tick (never allocates again).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scratch {
    front: Vec<i32>,
    back: Vec<i32>,
    /// GRU prior hidden state: all zero (every step starts from rest).
    hidden: Vec<i32>,
    /// GRU reset-gated prior state for the candidate pass.
    gated: Vec<i32>,
}

impl Scratch {
    /// Scratch wide enough for `model`. The only allocation on the tick path.
    pub fn for_model(model: &Model) -> Self {
        let width = model.max_width();
        Self {
            front: vec![0; width],
            back: vec![0; width],
            hidden: vec![0; width],
            gated: vec![0; width],
        }
    }

    /// Lanes per buffer.
    pub fn width(&self) -> usize {
        self.front
            .len()
            .min(self.back.len())
            .min(self.hidden.len())
            .min(self.gated.len())
    }
}

/// Load and validate an `.e2nn` file. Never panics; every malformed shape
/// maps to a typed [`NnErr`].
pub fn load(bytes: &[u8]) -> Result<Model, NnErr> {
    let mut cur = Cursor { b: bytes, pos: 0 };
    if cur.take(4)? != MAGIC {
        return Err(NnErr::BadMagic);
    }
    if cur.take_u8()? != VERSION {
        return Err(NnErr::BadVersion);
    }
    let count = usize::from(cur.take_u8()?);
    if count == 0 || count > MAX_LAYERS {
        return Err(NnErr::BadLayerCount);
    }
    let input_dim = usize::from(cur.take_u16()?);
    if input_dim == 0 || input_dim > MAX_DIM {
        return Err(NnErr::BadDim);
    }
    let mut in_dim = input_dim;
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let layer = match cur.take_u8()? {
            TAG_DENSE => Layer::Dense(read_dense(&mut cur, in_dim)?),
            TAG_EMBEDDING => Layer::Embedding(read_embedding(&mut cur, in_dim)?),
            TAG_GRU => Layer::Gru(read_gru(&mut cur, in_dim)?),
            _ => return Err(NnErr::BadLayerTag),
        };
        in_dim = layer.out_dim();
        layers.push(layer);
    }
    if cur.pos != bytes.len() {
        return Err(NnErr::TrailingBytes);
    }
    Ok(Model {
        input_dim,
        layers,
        digest: engine_core::hash::hash_bytes(bytes),
    })
}

/// Run one forward pass into a fresh vector: the allocating wrapper over
/// [`infer_into`] for tools and tests. Bit-exact: same model plus same input
/// always gives the same output. Total: short inputs zero-pad, long inputs
/// truncate.
pub fn infer(model: &Model, input: &[i8]) -> Vec<i32> {
    let mut scratch = Scratch::for_model(model);
    let mut out = vec![0i32; model.output_dim()];
    // Scratch and output are sized from `model` right here, so neither
    // length check can fail.
    match infer_into(model, input, &mut scratch, &mut out) {
        Ok(()) => out,
        Err(_) => Vec::new(),
    }
}

/// Run one forward pass with caller-owned buffers and no heap allocation:
/// the in-tick entry. `out` must be exactly [`Model::output_dim`] long and
/// `scratch` at least [`Model::max_width`] wide (build it with
/// [`Scratch::for_model`]); otherwise a typed [`NnErr`] and `out` untouched.
/// Same integer math, same bits as [`infer`]. Short inputs zero-pad, long
/// inputs truncate.
pub fn infer_into(
    model: &Model,
    input: &[i8],
    scratch: &mut Scratch,
    out: &mut [i32],
) -> Result<(), NnErr> {
    if out.len() != model.output_dim() {
        return Err(NnErr::OutputLen);
    }
    if scratch.width() < model.max_width() {
        return Err(NnErr::ScratchTooSmall);
    }
    let Scratch {
        front,
        back,
        hidden,
        gated,
    } = scratch;
    // One step from rest: the prior hidden state is zero on every call.
    hidden.fill(0);
    let mut cur: &mut [i32] = front.as_mut_slice();
    let mut next: &mut [i32] = back.as_mut_slice();
    for (i, slot) in cur.iter_mut().take(model.input_dim).enumerate() {
        *slot = input.get(i).map_or(0, |v| i32::from(*v));
    }
    let mut width = model.input_dim;
    for layer in &model.layers {
        let out_dim = layer.out_dim();
        let x = &cur[..width];
        let y = &mut next[..out_dim];
        match layer {
            Layer::Dense(d) => forward_dense(d, x, y),
            Layer::Embedding(e) => forward_embedding(e, x, y),
            Layer::Gru(g) => forward_gru(g, x, &hidden[..out_dim], &mut gated[..out_dim], y),
        }
        core::mem::swap(&mut cur, &mut next);
        width = out_dim;
    }
    out.copy_from_slice(&cur[..width]);
    Ok(())
}

// --- integer math ---

/// Saturating clamp of an `i8`-range value kept as `i32`.
fn clamp_i8(v: i32) -> i32 {
    v.clamp(-128, 127)
}

/// Saturating narrow from `i64` to `i32`.
fn saturate_i32(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// Multiplier-plus-shift requantization: `(acc * mult)` shifted right by
/// `shift` bits with round-half-away-from-zero, saturated to `i32`.
/// Callers only pass `shift <= 31` (checked at load).
fn requant(acc: i32, mult: i32, shift: u8) -> i32 {
    let prod: i64 = i64::from(acc) * i64::from(mult);
    if shift == 0 {
        return saturate_i32(prod);
    }
    let round: i64 = 1i64 << (shift - 1);
    let adj: i64 = if prod >= 0 {
        prod.saturating_add(round)
    } else {
        prod.saturating_sub(round)
    };
    saturate_i32(adj >> shift)
}

const fn build_sigmoid() -> [i8; 256] {
    let mut t = [0i8; 256];
    let mut i = 0usize;
    while i < 256 {
        let x = i as i32 - 128;
        let mut y = 64 + 2 * x;
        if y < 0 {
            y = 0;
        }
        if y > 127 {
            y = 127;
        }
        t[i] = y as i8;
        i += 1;
    }
    t
}

const fn build_tanh() -> [i8; 256] {
    let mut t = [0i8; 256];
    let mut i = 0usize;
    while i < 256 {
        let x = i as i32 - 128;
        let mut y = 3 * x;
        if y < -127 {
            y = -127;
        }
        if y > 127 {
            y = 127;
        }
        t[i] = y as i8;
        i += 1;
    }
    t
}

/// Sigmoid gate table: index `x + 128` for `x` in `-128..=127`,
/// value in `0..=127` where 64 means one half.
pub const SIGMOID_TABLE: [i8; 256] = build_sigmoid();
/// Tanh table: index `x + 128` for `x` in `-128..=127`, value in `-127..=127`.
pub const TANH_TABLE: [i8; 256] = build_tanh();

/// Sigmoid gate lookup: index `x + 128`, value in `0..=127` (64 is one half).
fn sig_lut(q: i32) -> i32 {
    i32::from(SIGMOID_TABLE[(clamp_i8(q) + 128) as usize])
}

/// Tanh lookup: index `x + 128`, value in `-127..=127`.
fn tanh_lut(q: i32) -> i32 {
    i32::from(TANH_TABLE[(clamp_i8(q) + 128) as usize])
}

/// Apply an activation code to a requantized value; output is `i8`-range.
fn apply_act(q: i32, act: u8, lut: Option<&[i8; 256]>) -> i32 {
    let idx = (clamp_i8(q) + 128) as usize;
    match act {
        ACT_NONE => clamp_i8(q),
        ACT_RELU => clamp_i8(q).max(0),
        ACT_TANH => i32::from(TANH_TABLE[idx]),
        ACT_SIGMOID => i32::from(SIGMOID_TABLE[idx]),
        ACT_LUT => match lut {
            Some(t) => i32::from(t[idx]),
            None => clamp_i8(q),
        },
        _ => clamp_i8(q),
    }
}

/// Saturating dot product plus bias: `bias + sum(w[i] * x[i])`.
fn affine(bias: i32, w: &[i8], x: &[i32]) -> i32 {
    debug_assert_eq!(w.len(), x.len());
    let mut acc = bias;
    for (wi, &xv) in w.iter().zip(x.iter()) {
        acc = acc.saturating_add(i32::from(*wi).saturating_mul(xv));
    }
    acc
}

fn forward_dense(d: &DenseDesc, x: &[i32], y: &mut [i32]) {
    debug_assert_eq!(d.weights.len(), d.out_dim * x.len());
    debug_assert_eq!(d.bias.len(), d.out_dim);
    for (o, slot) in y.iter_mut().enumerate() {
        let row = o * x.len();
        let acc = affine(d.bias[o], &d.weights[row..row + x.len()], x);
        *slot = apply_act(requant(acc, d.mult, d.shift), d.act, d.lut());
    }
}

fn forward_embedding(e: &EmbeddingDesc, x: &[i32], y: &mut [i32]) {
    y.fill(0);
    // Vocab is capped at MAX_VOCAB (4096) at load, so it always fits.
    let vocab = i32::try_from(e.vocab).unwrap_or(i32::MAX);
    for &tok in x {
        let row = (tok.rem_euclid(vocab) as usize) * e.emb_dim;
        debug_assert!(row + e.emb_dim <= e.table.len());
        for (j, slot) in y.iter_mut().enumerate() {
            *slot = slot.saturating_add(i32::from(e.table[row + j]));
        }
    }
    for v in y.iter_mut() {
        *v = apply_act(requant(*v, e.mult, e.shift), e.act, e.lut());
    }
}

/// One GRU step from the prior hidden state `h0` (zero today, see
/// [`Scratch`]): gates `z`, `r` and candidate `n` from fixed tables, then
/// `h = z * n` (the `(1 - z) * h0` term vanishes at `h0 = 0`). `y` holds `z`
/// between the two passes; `rh` receives the reset-gated prior state.
fn forward_gru(g: &GruDesc, x: &[i32], h0: &[i32], rh: &mut [i32], y: &mut [i32]) {
    let h = g.hidden;
    let in_len = x.len();
    for (i, (z_slot, (rh_slot, prior))) in y.iter_mut().zip(rh.iter_mut().zip(h0)).enumerate() {
        let lin_z = affine(g.bz[i], &g.wz_ih[i * in_len..(i + 1) * in_len], x)
            .saturating_add(affine(0, &g.wz_hh[i * h..(i + 1) * h], h0));
        *z_slot = sig_lut(requant(lin_z, g.mult, g.shift));
        let lin_r = affine(g.br[i], &g.wr_ih[i * in_len..(i + 1) * in_len], x)
            .saturating_add(affine(0, &g.wr_hh[i * h..(i + 1) * h], h0));
        let r = sig_lut(requant(lin_r, g.mult, g.shift));
        // Reset-gated prior hidden, rescaled by 2^7 back to i8 range.
        *rh_slot = requant(r.saturating_mul(*prior), 1, 7);
    }
    let rh: &[i32] = rh;
    for (i, slot) in y.iter_mut().enumerate() {
        let lin_n = affine(g.bn[i], &g.wn_ih[i * in_len..(i + 1) * in_len], x)
            .saturating_add(affine(0, &g.wn_hh[i * h..(i + 1) * h], rh));
        let n = tanh_lut(requant(lin_n, g.mult, g.shift));
        // h starts at zero, so the new hidden state is the z-scaled candidate.
        *slot = clamp_i8(requant(slot.saturating_mul(n), 1, 7));
    }
}

// --- quantization ---

/// Per-tensor weight quantization scale: the largest absolute weight over
/// 127, or `1.0` for an all-zero matrix. The ONNX converter and the student
/// exporter fold this one rule.
pub fn weight_scale(weights: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    for w in weights {
        let a = w.abs();
        if a > peak {
            peak = a;
        }
    }
    if peak == 0.0 {
        1.0
    } else {
        peak / 127.0
    }
}

/// Requantization `(multiplier, shift)` for the real scale `ratio`: the
/// largest shift `0..=31` whose rounded multiplier still fits `i32` (maximum
/// precision). `None` when `ratio` is not finite, not positive, or too large
/// for any shift.
pub fn requant_params(ratio: f64) -> Option<(i32, u8)> {
    if !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    for shift in (0..=31u8).rev() {
        let m = (ratio * 2f64.powi(i32::from(shift))).round();
        if m <= f64::from(i32::MAX) {
            return Some((m as i32, shift));
        }
    }
    None
}

// --- parsing ---

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], NnErr> {
        let end = self.pos.checked_add(n).ok_or(NnErr::Truncated)?;
        if end > self.b.len() {
            return Err(NnErr::Truncated);
        }
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn take_u8(&mut self) -> Result<u8, NnErr> {
        Ok(self.take(1)?[0])
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], NnErr> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn take_u16(&mut self) -> Result<u16, NnErr> {
        self.take_array().map(u16::from_le_bytes)
    }

    fn take_i32(&mut self) -> Result<i32, NnErr> {
        self.take_array().map(i32::from_le_bytes)
    }

    fn take_i32_vec(&mut self, n: usize) -> Result<Vec<i32>, NnErr> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.take_i32()?);
        }
        Ok(v)
    }

    fn take_i8_vec(&mut self, n: usize) -> Result<Vec<i8>, NnErr> {
        Ok(self.take(n)?.iter().map(|b| b.cast_signed()).collect())
    }
}

/// Checked `a * b` against the param cap.
fn checked_count(a: usize, b: usize) -> Result<usize, NnErr> {
    let n = a.checked_mul(b).ok_or(NnErr::TooLarge)?;
    if n > MAX_PARAMS {
        return Err(NnErr::TooLarge);
    }
    Ok(n)
}

fn read_mult_shift_act(cur: &mut Cursor<'_>) -> Result<(i32, u8, u8), NnErr> {
    let mult = cur.take_i32()?;
    let shift = cur.take_u8()?;
    if shift > 31 {
        return Err(NnErr::BadShift);
    }
    let act = cur.take_u8()?;
    if act > ACT_LUT {
        return Err(NnErr::BadActivation);
    }
    Ok((mult, shift, act))
}

fn read_lut(cur: &mut Cursor<'_>, act: u8) -> Result<Option<Box<[i8; 256]>>, NnErr> {
    if act != ACT_LUT {
        return Ok(None);
    }
    let s = cur.take(256)?;
    let mut t = Box::new([0i8; 256]);
    for (i, b) in s.iter().enumerate() {
        t[i] = b.cast_signed();
    }
    Ok(Some(t))
}

fn read_dense(cur: &mut Cursor<'_>, in_dim: usize) -> Result<DenseDesc, NnErr> {
    let out_dim = usize::from(cur.take_u16()?);
    if out_dim == 0 || out_dim > MAX_DIM {
        return Err(NnErr::BadDim);
    }
    let (mult, shift, act) = read_mult_shift_act(cur)?;
    let bias = cur.take_i32_vec(out_dim)?;
    let weights = cur.take_i8_vec(checked_count(out_dim, in_dim)?)?;
    let lut = read_lut(cur, act)?;
    Ok(DenseDesc {
        out_dim,
        mult,
        shift,
        act,
        bias,
        weights,
        lut,
    })
}

fn read_embedding(cur: &mut Cursor<'_>, _in_dim: usize) -> Result<EmbeddingDesc, NnErr> {
    let vocab = usize::from(cur.take_u16()?);
    if vocab == 0 || vocab > MAX_VOCAB {
        return Err(NnErr::BadDim);
    }
    let emb_dim = usize::from(cur.take_u16()?);
    if emb_dim == 0 || emb_dim > MAX_DIM {
        return Err(NnErr::BadDim);
    }
    let (mult, shift, act) = read_mult_shift_act(cur)?;
    let table = cur.take_i8_vec(checked_count(vocab, emb_dim)?)?;
    let lut = read_lut(cur, act)?;
    Ok(EmbeddingDesc {
        vocab,
        emb_dim,
        mult,
        shift,
        act,
        table,
        lut,
    })
}

/// One GRU gate's parameters: bias, input weights, recurrent weights.
struct GruGate {
    bias: Vec<i32>,
    w_ih: Vec<i8>,
    w_hh: Vec<i8>,
}

fn read_gru_mats(cur: &mut Cursor<'_>, hidden: usize, in_dim: usize) -> Result<GruGate, NnErr> {
    let bias = cur.take_i32_vec(hidden)?;
    let w_ih = cur.take_i8_vec(checked_count(hidden, in_dim)?)?;
    let w_hh = cur.take_i8_vec(checked_count(hidden, hidden)?)?;
    Ok(GruGate { bias, w_ih, w_hh })
}

fn read_gru(cur: &mut Cursor<'_>, in_dim: usize) -> Result<GruDesc, NnErr> {
    let hidden = usize::from(cur.take_u16()?);
    if hidden == 0 || hidden > MAX_HIDDEN {
        return Err(NnErr::BadDim);
    }
    let mult = cur.take_i32()?;
    let shift = cur.take_u8()?;
    if shift > 31 {
        return Err(NnErr::BadShift);
    }
    let z_gate = read_gru_mats(cur, hidden, in_dim)?;
    let r_gate = read_gru_mats(cur, hidden, in_dim)?;
    let n_gate = read_gru_mats(cur, hidden, in_dim)?;
    Ok(GruDesc {
        hidden,
        mult,
        shift,
        bz: z_gate.bias,
        wz_ih: z_gate.w_ih,
        wz_hh: z_gate.w_hh,
        br: r_gate.bias,
        wr_ih: r_gate.w_ih,
        wr_hh: r_gate.w_hh,
        bn: n_gate.bias,
        wn_ih: n_gate.w_ih,
        wn_hh: n_gate.w_hh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_scale_is_peak_over_127_and_one_for_zeros() {
        assert_eq!(weight_scale(&[]), 1.0);
        assert_eq!(weight_scale(&[0.0, -0.0, 0.0]), 1.0);
        assert_eq!(weight_scale(&[127.0, -3.0]), 1.0);
        assert_eq!(weight_scale(&[1.0, -254.0, 100.0]), 2.0);
    }

    #[test]
    fn requant_params_pick_the_largest_fitting_shift() {
        assert_eq!(requant_params(1.0), Some((1 << 30, 30)));
        assert_eq!(requant_params(0.5), Some((1 << 30, 31)));
        assert_eq!(requant_params(2.0), Some((1 << 30, 29)));
        assert_eq!(requant_params(0.1), Some((214_748_365, 31)));
        for refused in [1e30, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(requant_params(refused), None, "{refused}");
        }
    }

    #[test]
    fn saturating_narrow_clamps_both_edges() {
        let (lo, hi) = (i64::from(i32::MIN), i64::from(i32::MAX));
        assert_eq!(saturate_i32(0), 0);
        assert_eq!(saturate_i32(-1), -1);
        assert_eq!(saturate_i32(lo), i32::MIN);
        assert_eq!(saturate_i32(hi), i32::MAX);
        assert_eq!(saturate_i32(lo - 1), i32::MIN);
        assert_eq!(saturate_i32(hi + 1), i32::MAX);
        assert_eq!(saturate_i32(i64::MIN), i32::MIN);
        assert_eq!(saturate_i32(i64::MAX), i32::MAX);
    }

    fn w_u16(v: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn w_i32(v: i32, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn header(input_dim: u16, layers: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.push(VERSION);
        b.push(layers);
        w_u16(input_dim, &mut b);
        b
    }

    fn dense_body(
        out_dim: u16,
        mult: i32,
        shift: u8,
        act: u8,
        bias: &[i32],
        weights: &[i8],
        lut: Option<&[i8; 256]>,
    ) -> Vec<u8> {
        let mut b = vec![TAG_DENSE];
        w_u16(out_dim, &mut b);
        w_i32(mult, &mut b);
        b.push(shift);
        b.push(act);
        for v in bias {
            w_i32(*v, &mut b);
        }
        b.extend_from_slice(&weights.iter().map(|w| *w as u8).collect::<Vec<_>>());
        if let Some(t) = lut {
            b.extend_from_slice(&t.iter().map(|v| *v as u8).collect::<Vec<_>>());
        }
        b
    }

    /// Hand-built net: in 2, one dense out 2, identity requant, no activation.
    /// o0 = 1 + 3*7 - 4*8 = -10; o1 = -2 + 5*7 + 6*8 = 81.
    fn dense_golden_bytes() -> Vec<u8> {
        let mut b = header(2, 1);
        b.extend_from_slice(&dense_body(
            2,
            1,
            0,
            ACT_NONE,
            &[1, -2],
            &[3, -4, 5, 6],
            None,
        ));
        b
    }

    #[test]
    fn dense_single_golden() {
        let m = load(&dense_golden_bytes()).expect("golden loads");
        assert_eq!(m.input_dim(), 2);
        assert_eq!(m.output_dim(), 2);
        assert_eq!(infer(&m, &[7, 8]), vec![-10, 81]);
    }

    /// FPS duel net bytes: 6 lanes in, 5 intents out, one dense layer with
    /// identity requant and no activation. Matches the ai duel contract
    /// (`FEATURE_DIM` 6, `INTENT_OUT_DIM` 5) so the golden pins the exact
    /// bytes the FPS brains run. Hand computation for `[1, 2, 3, 4, 5, 6]`:
    /// o0 = 10 + 21 = 31; o1 = -10 - 21 = -31; o2 = 1; o3 = 2 * 6 = 12;
    /// o4 = 5.
    fn fps_duel_shape_bytes() -> Vec<u8> {
        let mut b = header(6, 1);
        b.extend_from_slice(&dense_body(
            5,
            1,
            0,
            ACT_NONE,
            &[10, -10, 0, 0, 5],
            &[
                1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
                0, 0, 0, 0,
            ],
            None,
        ));
        b
    }

    #[test]
    fn fps_duel_shape_golden_and_tick() {
        let bytes = fps_duel_shape_bytes();
        let m = load(&bytes).expect("fps net loads");
        assert_eq!(m.input_dim(), 6);
        assert_eq!(m.output_dim(), 5);
        assert_eq!(m.max_width(), 6);
        assert_eq!(m.digest(), engine_core::hash::hash_bytes(&bytes));
        let input = [1i8, 2, 3, 4, 5, 6];
        assert_eq!(infer(&m, &input), vec![31, -31, 1, 12, 5]);
        let mut scratch = Scratch::for_model(&m);
        let mut out = [0i32; 5];
        infer_into(&m, &input, &mut scratch, &mut out).expect("sized buffers");
        assert_eq!(out, [31, -31, 1, 12, 5]);
        let mut again = [0i32; 5];
        infer_into(&m, &input, &mut scratch, &mut again).expect("sized buffers");
        assert_eq!(again, out);
        let (result, count) = engine_core::alloc_probe::measure(|| {
            let mut last = Ok(());
            for _ in 0..64 {
                last = infer_into(&m, &input, &mut scratch, &mut out);
            }
            last
        });
        assert_eq!(result, Ok(()));
        assert_eq!(count.calls, 0, "fps tick heap calls");
        assert_eq!(infer(&m, &[1, 2]), vec![13, -13, 1, 0, 5]);
        assert_eq!(infer(&m, &[1, 2, 3, 4, 5, 6, 7]), vec![31, -31, 1, 12, 5]);
        let mut keep = [55i32; 5];
        assert_eq!(
            infer_into(&m, &input, &mut Scratch::default(), &mut keep),
            Err(NnErr::ScratchTooSmall)
        );
        assert_eq!(keep, [55, 55, 55, 55, 55]);
        let mut b2 = header(6, 1);
        b2.extend_from_slice(&dense_body(
            5,
            1,
            0,
            ACT_NONE,
            &[10, -10, 0, 0, 5],
            &[
                2, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
                0, 0, 0, 0,
            ],
            None,
        ));
        let m2 = load(&b2).expect("edited fps net loads");
        assert_eq!(infer(&m2, &input), vec![32, -31, 1, 12, 5]);
        assert_ne!(m.digest(), m2.digest());
    }

    /// Locomotion trajectory net bytes: 4 lanes in, 6 offsets out, one dense
    /// layer with identity requant and no activation. Matches the anim
    /// trajectory contract (`TRAJ_NET_IN_DIM` 4, `TRAJ_NET_OUT_DIM` 6) so the
    /// golden pins the exact bytes locomotion runs. Hand computation for
    /// `[2, -1, 3, 0]`: identity rows give `[2, -1, 3, 0]`, the sum row gives
    /// 1 + 4 = 5, the negated sum gives -4.
    fn traj_shape_bytes() -> Vec<u8> {
        let mut b = header(4, 1);
        b.extend_from_slice(&dense_body(
            6,
            1,
            0,
            ACT_NONE,
            &[0, 0, 0, 0, 1, 0],
            &[
                1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 1, -1, -1, -1, -1,
            ],
            None,
        ));
        b
    }

    #[test]
    fn locomotion_traj_shape_golden_and_tick() {
        let bytes = traj_shape_bytes();
        let m = load(&bytes).expect("traj net loads");
        assert_eq!(m.input_dim(), 4);
        assert_eq!(m.output_dim(), 6);
        assert_eq!(m.max_width(), 6);
        assert_eq!(m.digest(), engine_core::hash::hash_bytes(&bytes));
        let input = [2i8, -1, 3, 0];
        assert_eq!(infer(&m, &input), vec![2, -1, 3, 0, 5, -4]);
        let mut scratch = Scratch::for_model(&m);
        let mut out = [0i32; 6];
        infer_into(&m, &input, &mut scratch, &mut out).expect("sized buffers");
        assert_eq!(out, [2, -1, 3, 0, 5, -4]);
        let mut again = [0i32; 6];
        infer_into(&m, &input, &mut scratch, &mut again).expect("sized buffers");
        assert_eq!(again, out);
        let (result, count) = engine_core::alloc_probe::measure(|| {
            let mut last = Ok(());
            for _ in 0..64 {
                last = infer_into(&m, &input, &mut scratch, &mut out);
            }
            last
        });
        assert_eq!(result, Ok(()));
        assert_eq!(count.calls, 0, "traj tick heap calls");
        assert_eq!(infer(&m, &[2]), vec![2, 0, 0, 0, 3, -2]);
        assert_eq!(infer(&m, &[2, -1, 3, 0, 9]), vec![2, -1, 3, 0, 5, -4]);
        let mut keep = [55i32; 2];
        assert_eq!(
            infer_into(&m, &input, &mut scratch, &mut keep),
            Err(NnErr::OutputLen)
        );
        assert_eq!(keep, [55, 55]);
    }

    #[test]
    fn dense_shift_relu_golden() {
        // Same weights, shift 1 halves with rounding: -10 -> -6, 81 -> 41.
        // -10: (-10 - 1) >> 1 = -6; 81: (81 + 1) >> 1 = 41. ReLU zeroes -6.
        let mut b = header(2, 1);
        b.extend_from_slice(&dense_body(
            2,
            1,
            1,
            ACT_RELU,
            &[1, -2],
            &[3, -4, 5, 6],
            None,
        ));
        let m = load(&b).expect("golden loads");
        assert_eq!(infer(&m, &[7, 8]), vec![0, 41]);
    }

    #[test]
    fn dense_tanh_sigmoid_lut_goldens() {
        // tanh(50) saturates: 3 * 50 = 150 clamps to 127.
        let mut b = header(1, 1);
        b.extend_from_slice(&dense_body(1, 1, 0, ACT_TANH, &[0], &[10], None));
        let m = load(&b).expect("tanh net loads");
        assert_eq!(infer(&m, &[5]), vec![127]);

        // sigmoid(0) is one half: table value 64.
        let mut b = header(1, 1);
        b.extend_from_slice(&dense_body(1, 1, 0, ACT_SIGMOID, &[0], &[7], None));
        let m = load(&b).expect("sigmoid net loads");
        assert_eq!(infer(&m, &[0]), vec![64]);

        // Custom table 127 - i: input acc 10 hits index 138, value -11.
        let mut lut = [0i8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = (127 - i as i32) as i8;
        }
        let mut b = header(1, 1);
        b.extend_from_slice(&dense_body(1, 1, 0, ACT_LUT, &[0], &[2], Some(&lut)));
        let m = load(&b).expect("lut net loads");
        assert_eq!(infer(&m, &[5]), vec![-11]);
    }

    #[test]
    fn embedding_sum_golden() {
        // Tokens [1, 3] sum rows [4,5,6] + [10,11,12] = [14,16,18].
        let mut b = header(2, 1);
        b.push(TAG_EMBEDDING);
        w_u16(4, &mut b);
        w_u16(3, &mut b);
        w_i32(1, &mut b);
        b.push(0);
        b.push(ACT_NONE);
        for v in [1i8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
            b.push(v as u8);
        }
        let m = load(&b).expect("embedding net loads");
        assert_eq!(m.output_dim(), 3);
        assert_eq!(infer(&m, &[1, 3]), vec![14, 16, 18]);
    }

    #[test]
    fn embedding_negative_wraps() {
        // Token -1 wraps to row 3 via rem_euclid(4).
        let mut b = header(1, 1);
        b.push(TAG_EMBEDDING);
        w_u16(4, &mut b);
        w_u16(2, &mut b);
        w_i32(1, &mut b);
        b.push(0);
        b.push(ACT_NONE);
        for v in [10i8, 11, 12, 13, 14, 15, 16, 17] {
            b.push(v as u8);
        }
        let m = load(&b).expect("embedding net loads");
        assert_eq!(infer(&m, &[-1]), vec![16, 17]);
    }

    fn gru_golden_bytes() -> Vec<u8> {
        // in 1, hidden 1, identity requant. Gates see zero -> sigmoid(0) = 64.
        // Candidate: 10 + 3*5 = 25 -> tanh(25) = 75.
        // h = (64 * 75 + 64) >> 7 = 4864 >> 7 = 38.
        let mut b = header(1, 1);
        b.push(TAG_GRU);
        w_u16(1, &mut b);
        w_i32(1, &mut b);
        b.push(0);
        for bias in [0i32, 0, 10] {
            w_i32(bias, &mut b);
            b.push(if bias == 10 { 3u8 } else { 0 });
            b.push(0);
        }
        b
    }

    #[test]
    fn gru_single_step_golden() {
        let m = load(&gru_golden_bytes()).expect("gru net loads");
        assert_eq!(m.output_dim(), 1);
        assert_eq!(infer(&m, &[5]), vec![38]);
    }

    #[test]
    fn two_layer_chain_golden() {
        // Dense golden layer feeds a 2 -> 1 sum layer: -10 + 81 = 71.
        let mut b = header(2, 2);
        b.extend_from_slice(&dense_body(
            2,
            1,
            0,
            ACT_NONE,
            &[1, -2],
            &[3, -4, 5, 6],
            None,
        ));
        b.extend_from_slice(&dense_body(1, 1, 0, ACT_NONE, &[0], &[1, 1], None));
        let m = load(&b).expect("chain loads");
        assert_eq!(m.output_dim(), 1);
        assert_eq!(infer(&m, &[7, 8]), vec![71]);
    }

    #[test]
    fn bit_exact_on_two_runs() {
        let bytes = gru_golden_bytes();
        let a = load(&bytes).expect("first load");
        let c = infer(&a, &[5]);
        let d = infer(&a, &[5]);
        assert_eq!(c, d);
        let e = infer(&load(&bytes).expect("second load"), &[5]);
        assert_eq!(c, e);
        assert_eq!(c, vec![38]);
    }

    #[test]
    fn one_changed_weight_changes_output() {
        let mut bytes = dense_golden_bytes();
        // Header 8 + tag 1 + out_dim 2 + mult 4 + shift 1 + act 1 + bias 8.
        let first_weight = 8 + 1 + 2 + 4 + 1 + 1 + 8;
        assert_eq!(bytes[first_weight], 3u8);
        bytes[first_weight] = 4u8;
        let m = load(&bytes).expect("edited net still loads");
        // o0 = 1 + 4*7 - 4*8 = -3, was -10.
        assert_eq!(infer(&m, &[7, 8]), vec![-3, 81]);
    }

    #[test]
    fn infer_pads_and_truncates() {
        let m = load(&dense_golden_bytes()).expect("golden loads");
        // Short pads with zero: o0 = 1 + 21 = 22; o1 = -2 + 35 = 33.
        assert_eq!(infer(&m, &[7]), vec![22, 33]);
        assert_eq!(infer(&m, &[]), vec![1, -2]);
        // Long truncates to the first two values.
        assert_eq!(infer(&m, &[7, 8, 9]), vec![-10, 81]);
    }

    #[test]
    fn malformed_rejected() {
        let good = dense_golden_bytes();
        // Bad magic.
        let mut bad = good.clone();
        bad[0] = b'Z';
        assert_eq!(load(&bad), Err(NnErr::BadMagic));
        // Bad version.
        let mut bad = good.clone();
        bad[4] = VERSION + 1;
        assert_eq!(load(&bad), Err(NnErr::BadVersion));
        // Empty and short header.
        assert_eq!(load(&[]), Err(NnErr::Truncated));
        assert_eq!(load(&good[..7]), Err(NnErr::Truncated));
        // Zero layer count.
        let mut bad = good.clone();
        bad[5] = 0;
        assert_eq!(load(&bad), Err(NnErr::BadLayerCount));
        // Zero input dim.
        let mut bad = good.clone();
        bad[6] = 0;
        bad[7] = 0;
        assert_eq!(load(&bad), Err(NnErr::BadDim));
        // Unknown layer tag.
        let mut bad = good.clone();
        bad[8] = 9;
        assert_eq!(load(&bad), Err(NnErr::BadLayerTag));
        // Zero output dim.
        let mut bad = good.clone();
        bad[9] = 0;
        bad[10] = 0;
        assert_eq!(load(&bad), Err(NnErr::BadDim));
        // Shift above 31 (shift byte sits after tag 1 + out 2 + mult 4).
        let mut bad = good.clone();
        bad[15] = 32;
        assert_eq!(load(&bad), Err(NnErr::BadShift));
        // Activation above ACT_LUT.
        let mut bad = good.clone();
        bad[16] = 7;
        assert_eq!(load(&bad), Err(NnErr::BadActivation));
        // Truncated mid-weights and mid-bias.
        assert_eq!(load(&good[..good.len() - 1]), Err(NnErr::Truncated));
        assert_eq!(load(&good[..20]), Err(NnErr::Truncated));
        // Trailing byte.
        let mut bad = good.clone();
        bad.push(0);
        assert_eq!(load(&bad), Err(NnErr::TrailingBytes));
        // Zero GRU hidden dim.
        let mut gru = gru_golden_bytes();
        gru[9] = 0;
        gru[10] = 0;
        assert_eq!(load(&gru), Err(NnErr::BadDim));
    }

    #[test]
    fn every_prefix_is_err_or_loads_safely() {
        let good = gru_golden_bytes();
        for len in 0..good.len() {
            let r = load(&good[..len]);
            assert!(r.is_err(), "prefix {len} should not load");
        }
        load(&good).expect("full file loads");
    }

    /// Two-layer net in the distilled-student shape: 3 inputs, 4 hidden
    /// (ReLU, shift 2), 2 outputs (identity, multiplier 3, shift 4).
    fn mlp_golden_bytes() -> Vec<u8> {
        let mut b = header(3, 2);
        b.extend_from_slice(&dense_body(
            4,
            1,
            2,
            ACT_RELU,
            &[10, -20, 0, 300],
            &[5, -3, 7, -8, 2, 1, 0, 0, 127, 127, -128, 64],
            None,
        ));
        b.extend_from_slice(&dense_body(
            2,
            3,
            4,
            ACT_NONE,
            &[-7, 100],
            &[1, 2, -3, 4, -5, 6, 7, -8],
            None,
        ));
        b
    }

    #[test]
    fn infer_into_matches_infer_on_every_layer_kind() {
        let nets = [
            (dense_golden_bytes(), vec![7i8, 8]),
            (gru_golden_bytes(), vec![5]),
            (mlp_golden_bytes(), vec![-3, 100, 17]),
        ];
        for (bytes, input) in nets {
            let m = load(&bytes).expect("net loads");
            let mut scratch = Scratch::for_model(&m);
            let mut out = vec![0i32; m.output_dim()];
            infer_into(&m, &input, &mut scratch, &mut out).expect("sized buffers");
            assert_eq!(out, infer(&m, &input));
            // Reused scratch gives the same bits on the second call.
            let mut again = vec![0i32; m.output_dim()];
            infer_into(&m, &input, &mut scratch, &mut again).expect("sized buffers");
            assert_eq!(out, again);
        }
    }

    #[test]
    fn infer_into_allocates_nothing() {
        for bytes in [mlp_golden_bytes(), gru_golden_bytes()] {
            let m = load(&bytes).expect("net loads");
            let mut scratch = Scratch::for_model(&m);
            let mut out = vec![0i32; m.output_dim()];
            let input = [1i8, -2, 3];
            let (result, count) = engine_core::alloc_probe::measure(|| {
                let mut last = Ok(());
                for _ in 0..64 {
                    last = infer_into(&m, &input, &mut scratch, &mut out);
                }
                last
            });
            assert_eq!(result, Ok(()));
            assert_eq!(count.calls, 0, "infer_into heap calls");
            // The probe is armed: the allocating wrapper does count.
            let (_, wrapped) = engine_core::alloc_probe::measure(|| infer(&m, &input));
            assert!(wrapped.calls > 0, "probe sees the wrapper allocate");
        }
    }

    #[test]
    fn infer_into_refuses_wrong_buffers_and_leaves_out_alone() {
        let m = load(&mlp_golden_bytes()).expect("net loads");
        let mut scratch = Scratch::for_model(&m);
        let mut short = [55i32; 1];
        assert_eq!(
            infer_into(&m, &[1, 2, 3], &mut scratch, &mut short),
            Err(NnErr::OutputLen)
        );
        assert_eq!(short, [55]);
        let mut long = [55i32; 3];
        assert_eq!(
            infer_into(&m, &[1, 2, 3], &mut scratch, &mut long),
            Err(NnErr::OutputLen)
        );
        let mut out = [55i32; 2];
        let mut tiny = Scratch::default();
        assert_eq!(
            infer_into(&m, &[1, 2, 3], &mut tiny, &mut out),
            Err(NnErr::ScratchTooSmall)
        );
        assert_eq!(out, [55, 55]);
        // Scratch from a wider model serves a narrower one.
        let wide = load(&mlp_golden_bytes()).expect("net loads");
        let narrow = load(&dense_golden_bytes()).expect("net loads");
        let mut shared = Scratch::for_model(&wide);
        let mut two = [0i32; 2];
        infer_into(&narrow, &[7, 8], &mut shared, &mut two).expect("wide scratch fits");
        assert_eq!(two, [-10, 81]);
        assert_eq!(m.max_width(), 4);
    }

    #[test]
    fn quantized_percept_golden_bytes() {
        // The tick path end to end: fixed f32 lanes through the one
        // quantization edge, then the fixed two-layer net. Pinned bytes.
        let lanes = [2.5f32, -0.75, f32::NAN];
        let scales = [127.0f32 / 16.0, 16.0, 8.0];
        let mut q = [0i8; 3];
        assert_eq!(quantize_lanes(&lanes, &scales, &mut q), 3);
        // 2.5 * 7.9375 = 19.84 -> 20; -0.75 * 16 = -12; NaN -> 0.
        assert_eq!(q, [20, -12, 0]);
        let m = load(&mlp_golden_bytes()).expect("net loads");
        let mut scratch = Scratch::for_model(&m);
        let mut out = [0i32; 2];
        infer_into(&m, &q, &mut scratch, &mut out).expect("sized buffers");
        // Hidden accs: 10+100+36 = 146 -> (146+2)>>2 = 37; -20-160-24 = -204
        // -> ReLU 0; 0 -> 0; 300+2540+1536 = 4376 -> 1094 -> clamp 127.
        // Out0: 3*(-7+37+508) = 1614 -> (1614+8)>>4 = 101;
        // out1: 3*(100-185-1016) = -3303 -> -207 -> clamp -128.
        assert_eq!(out, [101, -128]);
        let bytes: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(bytes, [101, 0, 0, 0, 128, 255, 255, 255]);
        // Saturated and NaN-laden percepts stay defined bytes.
        let wild = [f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
        assert_eq!(quantize_lanes(&wild, &scales, &mut q), 3);
        assert_eq!(q, [127, -127, 0]);
        infer_into(&m, &q, &mut scratch, &mut out).expect("sized buffers");
        assert_eq!(out.to_vec(), infer(&m, &q));
    }

    #[test]
    fn digest_pins_the_exact_bytes() {
        let bytes = mlp_golden_bytes();
        let m = load(&bytes).expect("net loads");
        assert_eq!(m.digest(), engine_core::hash::hash_bytes(&bytes));
        let mut edited = bytes.clone();
        let last = edited.len() - 1;
        edited[last] ^= 1;
        let m2 = load(&edited).expect("edited net loads");
        assert_ne!(m.digest(), m2.digest());
        assert_ne!(m, m2);
    }

    #[test]
    fn random_bytes_never_panic() {
        // Deterministic LCG sweep: odd shapes must error, never panic, and
        // any file that does load must also infer without panic.
        let mut state: u64 = 0x2440_5EED_9E37_79B9;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        };
        for _ in 0..2000 {
            let len = (next() as usize) % 64;
            let mut bytes = vec![0u8; len];
            for b in bytes.iter_mut() {
                *b = next();
            }
            if let Ok(m) = load(&bytes) {
                let out = infer(&m, &[1, -2, 3]);
                assert_eq!(out.len(), m.output_dim());
                for v in out {
                    assert!((-128..=127).contains(&v), "output stays in i8 range");
                }
            }
        }
    }
}
