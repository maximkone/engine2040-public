//! FPS budget gate for the tiny in-tick student (offline only, never in the tick).
//!
//! Consumer: `crates/ai` runs the shipped student on the perception clock through
//! [`NetPolicy`](engine_ai::NetPolicy), which calls `engine_nn::infer_into` with
//! caller-owned scratch; the `distill_duel` generator (Caller:
//! `crates/distill/src/bin/distill_duel.rs`) refuses to ship bytes that break this
//! budget. Owner: this module owns the static tick-cost caps and the offline byte
//! check. Units: `.e2nn` bytes, integer params (weights plus biases), integer ops
//! per inference (one multiply-add, one bias add, one requantization and one
//! activation each). Determinism: pure integer math over the file bytes; no clock,
//! no threads, no IO. Errors: malformed or over-budget students map to [`FpsError`];
//! [`check_fps`] never panics.
//!
//! Why static instead of timed: training and
//! search stay offline while the tick runs only the small int8 net, and the default
//! suite never asserts wall-clock time. The gate counts integer ops instead: the
//! shipped 6-24-5 student costs a few hundred ops per perception tick, negligible
//! next to the millisecond-scale AI budgets the arena brains already publish.

use engine_nn::{Layer, NnErr};

/// Lanes the shipped student reads: the ai percept width.
pub const FPS_LANES: usize = engine_ai::FEATURE_DIM;
/// Intents the shipped student emits: the ai intent width.
pub const FPS_CLASSES: usize = engine_ai::INTENT_OUT_DIM;
/// Most `.e2nn` bytes a shipped student may carry. The committed 6-24-5 duel net is
/// a few hundred bytes; one kilobyte of headroom stays, multi-kilobyte nets never
/// ship (every weight is at least one byte, so this cap bounds params too).
pub const MAX_FPS_BYTES: usize = 1024;
/// Most integer ops per inference across every dense layer. The shipped 6-24-5
/// student costs 322, so the cap holds it with room for a 32-wide hidden layer
/// (426) while a 48-wide one (634) must slim down or justify a new cap.
pub const MAX_FPS_TICK_OPS: u64 = 512;

/// FPS-gate refusals. Every variant names the offending value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpsError {
    /// Bytes are not a valid `.e2nn` file; the runtime cause is carried.
    BadStudent(NnErr),
    /// Net shape is not the ai tick shape; carries `(input, output)`.
    BadShape {
        /// Net input dim.
        input: usize,
        /// Net output dim.
        output: usize,
    },
    /// A layer is not integer dense (embedding, GRU); the tick path runs dense only.
    NonDense,
    /// Byte length passes [`MAX_FPS_BYTES`]; length carried.
    TooLarge {
        /// File bytes.
        bytes: usize,
    },
    /// Integer-op cost passes [`MAX_FPS_TICK_OPS`]; cost carried.
    OverBudget {
        /// Ops per inference.
        ops: u64,
    },
}

impl core::fmt::Display for FpsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FpsError::BadStudent(e) => write!(f, "fps: bad student bytes: {e}"),
            FpsError::BadShape { input, output } => write!(
                f,
                "fps: net is {input} -> {output}, want {FPS_LANES} -> {FPS_CLASSES}"
            ),
            FpsError::NonDense => f.write_str("fps: tick path runs dense layers only"),
            FpsError::TooLarge { bytes } => {
                write!(f, "fps: {bytes} bytes passes the {MAX_FPS_BYTES} B cap")
            }
            FpsError::OverBudget { ops } => {
                write!(f, "fps: {ops} ops passes the {MAX_FPS_TICK_OPS} op cap")
            }
        }
    }
}

impl std::error::Error for FpsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FpsError::BadStudent(e) => Some(e),
            _ => None,
        }
    }
}

/// Passed FPS gate: the receipt the ship log prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpsReport {
    /// FNV-1a 64 of the exact `.e2nn` bytes, mirroring `Model::digest`.
    pub digest: u64,
    /// Dense layers counted.
    pub layers: usize,
    /// Byte length of the file.
    pub bytes: usize,
    /// Integer params (weights plus biases) across the dense layers.
    pub params: usize,
    /// Integer ops per inference under the [`mlp_tick_ops`] rule.
    pub tick_ops: u64,
}

impl core::fmt::Display for FpsReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "fps: net {:016x} {}B {} params {} ops (cap {MAX_FPS_BYTES}B/{MAX_FPS_TICK_OPS} ops)",
            self.digest, self.bytes, self.params, self.tick_ops
        )
    }
}

/// Static tick cost of one dense layer: `out * (in + 1)` multiply-adds and bias
/// adds plus `out` requantizations and activations, in `u64` with saturation.
/// Total: never panics.
fn layer_ops(out_dim: usize, in_dim: usize) -> u64 {
    let out = u64::try_from(out_dim).unwrap_or(u64::MAX);
    let inp = u64::try_from(in_dim).unwrap_or(u64::MAX);
    out.saturating_mul(inp.saturating_add(2))
}

/// Static tick cost of a two-layer student shape: [`layer_ops`] for the hidden
/// layer plus [`layer_ops`] for the output layer. Pure sizing math for the trainer
/// to check before fitting; [`check_fps`] applies the same rule to real bytes.
/// Total: never panics.
pub fn mlp_tick_ops(input_dim: usize, hidden: usize, output_dim: usize) -> u64 {
    layer_ops(hidden, input_dim).saturating_add(layer_ops(output_dim, hidden))
}

/// Gate `.e2nn` bytes for the ai tick: the shape must be [`FPS_LANES`] lanes in and
/// [`FPS_CLASSES`] intents out, every layer integer dense, bytes within
/// [`MAX_FPS_BYTES`] and ops within [`MAX_FPS_TICK_OPS`]. Pure function of the
/// bytes: the same file always gives the same report on every machine.
pub fn check_fps(student_bytes: &[u8]) -> Result<FpsReport, FpsError> {
    let model = engine_nn::load(student_bytes).map_err(FpsError::BadStudent)?;
    if model.input_dim() != FPS_LANES || model.output_dim() != FPS_CLASSES {
        return Err(FpsError::BadShape {
            input: model.input_dim(),
            output: model.output_dim(),
        });
    }
    let mut params = 0usize;
    let mut ops = 0u64;
    let mut width = model.input_dim();
    for layer in model.layers() {
        let Layer::Dense(dense) = layer else {
            return Err(FpsError::NonDense);
        };
        params = params
            .saturating_add(dense.weights().len())
            .saturating_add(dense.bias().len());
        ops = ops.saturating_add(layer_ops(dense.out_dim(), width));
        width = dense.out_dim();
    }
    if student_bytes.len() > MAX_FPS_BYTES {
        return Err(FpsError::TooLarge {
            bytes: student_bytes.len(),
        });
    }
    if ops > MAX_FPS_TICK_OPS {
        return Err(FpsError::OverBudget { ops });
    }
    Ok(FpsReport {
        digest: model.digest(),
        layers: model.layers().len(),
        bytes: student_bytes.len(),
        params,
        tick_ops: ops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_nn::{infer, infer_into, Scratch, ACT_NONE, MAGIC, TAG_DENSE, VERSION};

    /// The committed distilled net (`distill_duel` defaults).
    const FIXTURE_NET: &[u8] = include_bytes!("../../../fixtures/nets/depth_student.e2nn");

    #[test]
    fn op_sizer_pins_hand_arithmetic() {
        assert_eq!(mlp_tick_ops(6, 24, 5), 322);
        assert_eq!(mlp_tick_ops(2, 8, 2), 8 * 4 + 2 * 10);
        assert_eq!(mlp_tick_ops(4, 1, 3), 6 + 3 * 3);
        // Saturates instead of wrapping on absurd dims.
        assert_eq!(mlp_tick_ops(usize::MAX, usize::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn committed_net_passes_the_gate() {
        let report = check_fps(FIXTURE_NET).expect("shipped net holds the budget");
        assert_eq!(report.digest, 0x6ace_b82b_2a85_6f5c);
        assert_eq!(report.bytes, FIXTURE_NET.len());
        assert!(report.bytes < MAX_FPS_BYTES, "{} B", report.bytes);
        assert_eq!(report.layers, 2);
        let model = engine_nn::load(FIXTURE_NET).expect("fixture loads");
        assert_eq!(
            (model.input_dim(), model.output_dim()),
            (FPS_LANES, FPS_CLASSES)
        );
        let hidden = match &model.layers()[0] {
            Layer::Dense(dense) => dense.out_dim(),
            _ => panic!("fixture opens dense"),
        };
        assert_eq!(
            report.params,
            hidden * (FPS_LANES + 1) + FPS_CLASSES * (hidden + 1)
        );
        assert_eq!(
            report.tick_ops,
            mlp_tick_ops(FPS_LANES, hidden, FPS_CLASSES)
        );
        assert!(report.tick_ops <= MAX_FPS_TICK_OPS);
        let line = report.to_string();
        assert!(line.contains("fps:") && line.contains("ops"), "{line}");
    }

    #[test]
    fn tick_path_runs_on_caller_owned_buffers() {
        // The ai tick shape: caller-owned scratch plus output, through the same
        // `infer_into` the net policy runs. (The heap-free proof lives in
        // engine-nn; here the gate pins the call shape the budget assumes.)
        let model = engine_nn::load(FIXTURE_NET).expect("fixture loads");
        let mut scratch = Scratch::for_model(&model);
        let mut out = vec![0i32; model.output_dim()];
        let input = [1i8, -2, 3, -4, 5, -6];
        infer_into(&model, &input, &mut scratch, &mut out).expect("sized buffers infer");
        assert_eq!(out, infer(&model, &input));
        // Reused scratch repeats bit-exactly.
        let mut again = vec![0i32; model.output_dim()];
        infer_into(&model, &input, &mut scratch, &mut again).expect("scratch reuses");
        assert_eq!(out, again);
    }

    /// Hand-emit one dense layer body over `in_dim` inputs (bias zero, identity
    /// requant, no activation).
    fn dense_layer(out_dim: u16, in_dim: usize, out: &mut Vec<u8>) {
        out.push(TAG_DENSE);
        out.extend_from_slice(&out_dim.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes());
        out.push(0);
        out.push(ACT_NONE);
        for _ in 0..out_dim {
            out.extend_from_slice(&0i32.to_le_bytes());
        }
        for _ in 0..usize::from(out_dim) * in_dim {
            out.push(0);
        }
    }

    /// Hand-emit a two-layer dense net over 6 lanes out to 5 intents.
    fn two_layer_net(hidden: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(VERSION);
        bytes.push(2);
        bytes.extend_from_slice(&6u16.to_le_bytes());
        dense_layer(hidden, 6, &mut bytes);
        dense_layer(5, usize::from(hidden), &mut bytes);
        bytes
    }

    #[test]
    fn malformed_and_misshapen_nets_refused() {
        assert_eq!(check_fps(&[]), Err(FpsError::BadStudent(NnErr::Truncated)));
        let mut bad_magic = two_layer_net(24);
        bad_magic[0] = b'Z';
        assert_eq!(
            check_fps(&bad_magic),
            Err(FpsError::BadStudent(NnErr::BadMagic))
        );
        // 2 -> 1 dense net: loads, but is not the ai tick shape.
        let mut narrow = Vec::new();
        narrow.extend_from_slice(&MAGIC);
        narrow.push(VERSION);
        narrow.push(1);
        narrow.extend_from_slice(&2u16.to_le_bytes());
        dense_layer(1, 2, &mut narrow);
        assert_eq!(
            check_fps(&narrow),
            Err(FpsError::BadShape {
                input: 2,
                output: 1
            })
        );
    }

    #[test]
    fn over_budget_and_oversize_nets_refused() {
        // 64-wide hidden: bytes still fit, but 842 ops break the 512 cap.
        assert_eq!(mlp_tick_ops(6, 64, 5), 842);
        let wide = two_layer_net(64);
        assert!(wide.len() <= MAX_FPS_BYTES, "{} B", wide.len());
        assert_eq!(
            check_fps(&wide),
            Err(FpsError::OverBudget {
                ops: mlp_tick_ops(6, 64, 5)
            })
        );
        assert!(
            check_fps(&wide).unwrap_err().to_string().contains("842"),
            "refusal names the cost"
        );
        // 256-wide hidden: several kilobytes, refused on size first.
        let huge = two_layer_net(256);
        assert!(huge.len() > MAX_FPS_BYTES);
        assert_eq!(
            check_fps(&huge),
            Err(FpsError::TooLarge { bytes: huge.len() })
        );
    }

    #[test]
    fn non_dense_nets_refused() {
        // 6 lanes in, embedding out 5: the ai shape, but not integer dense.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(VERSION);
        bytes.push(1);
        bytes.extend_from_slice(&6u16.to_le_bytes());
        bytes.push(engine_nn::TAG_EMBEDDING);
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.push(0);
        bytes.push(ACT_NONE);
        bytes.resize(bytes.len() + 4 * 5, 0);
        assert_eq!(check_fps(&bytes), Err(FpsError::NonDense));
    }
}
