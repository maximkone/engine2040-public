//! ONNX int8-QDQ to `.e2nn` converter (feature `convert`).
//!
//! Consumer: the student export. This module runs once per export, outside the
//! fixed tick; the integer runtime in the crate root stays std-only and the
//! tract float reference below never enters gameplay code.
//! Owner: this module owns the whitelist, the QDQ-to-dense mapping and the
//! float-vs-integer reference check.
//! Units: ONNX scales are unitless `f32`; `.e2nn` weights are `i8`, biases
//! `i32`, requantization an integer multiplier plus shift (see crate docs).
//! Determinism: [`convert`] is a pure function of its input bytes; the float
//! reference run is a check only, never part of a replay.
//! Errors: every malformed shape maps to [`ConvertErr`]; [`convert`] and
//! [`reference_check`] never panic.
//!
//! ## Supported scope
//!
//! Exactly one compute node ([`Whitelist`] `Gemm` or `MatMul`), an optional
//! `Relu` on its output, and any number of `QuantizeLinear` /
//! `DequantizeLinear` simulation nodes on the single input-to-output path.
//! Anything else is refused: unknown operators by name
//! ([`ConvertErr::UnsupportedOp`]), other structures with [`ConvertErr`]
//! topology errors. Scales fold into one requant multiplier plus shift;
//! zero-points must be zero (the integer runtime has no zero-point term) and
//! absent zero-point inputs mean zero.

use crate::{infer, load, NnErr, ACT_NONE, ACT_RELU, MAGIC, MAX_DIM, TAG_DENSE, VERSION};

use tract_onnx::prelude::*;

/// Converted `.e2nn` file bytes, loadable by [`load`].
pub type E2nnBytes = Vec<u8>;

/// Largest ONNX payload accepted by [`convert`] and [`reference_check`].
pub const MAX_ONNX_BYTES: usize = 8 * 1024 * 1024;
/// Largest node count accepted (single-layer student scope).
pub const MAX_NODES: usize = 64;
/// Largest initializer count accepted.
pub const MAX_INITIALIZERS: usize = 64;
/// Largest graph-input count accepted.
pub const MAX_GRAPH_INPUTS: usize = 16;
/// Largest single tensor element count accepted.
pub const MAX_TENSOR_ELEMENTS: usize = 1_048_576;
/// Largest tensor rank accepted.
pub const MAX_RANK: usize = 8;
/// Agreement bound for [`reference_check`], in output units at unit output
/// scale: two output least-significant bits, covering one requant rounding
/// plus one quantization rounding per output for single-layer exports.
pub const REFERENCE_TOLERANCE: f32 = 2.0;

/// The only ONNX operators the converter accepts, by name.
pub struct Whitelist;

impl Whitelist {
    /// Supported int8-QDQ operators, verified by the converter tests:
    /// quantization simulation (`QuantizeLinear`, `DequantizeLinear`), one
    /// compute node (`Gemm` or `MatMul`), one optional output `Relu`.
    pub const OPS: [&'static str; 5] = [
        "QuantizeLinear",
        "DequantizeLinear",
        "Gemm",
        "MatMul",
        "Relu",
    ];

    /// True when `op` is in [`Whitelist::OPS`].
    pub fn allows(op: &str) -> bool {
        Self::OPS.contains(&op)
    }

    /// Refuse the first operator outside [`Whitelist::OPS`] by name.
    pub fn check_ops<'a>(ops: impl IntoIterator<Item = &'a str>) -> Result<(), ConvertErr> {
        for op in ops {
            if !Self::allows(op) {
                return Err(ConvertErr::UnsupportedOp { op: op.to_string() });
            }
        }
        Ok(())
    }
}

/// Conversion and reference-check failures.
#[derive(Clone, Debug, PartialEq)]
pub enum ConvertErr {
    /// Input bytes are empty.
    EmptyInput,
    /// Input bytes pass [`MAX_ONNX_BYTES`].
    TooLarge { bytes: usize, cap: usize },
    /// Bytes are not a decodable ONNX model.
    BadProto,
    /// Model carries no graph.
    MissingGraph,
    /// Node count passes [`MAX_NODES`].
    TooManyNodes { count: usize, cap: usize },
    /// Initializer count passes [`MAX_INITIALIZERS`].
    TooManyInitializers { count: usize, cap: usize },
    /// Graph-input count passes [`MAX_GRAPH_INPUTS`].
    TooManyInputs { count: usize, cap: usize },
    /// Operator outside [`Whitelist::OPS`]; carries the refused name.
    UnsupportedOp { op: String },
    /// No `Gemm` or `MatMul` node found.
    NoComputeNode,
    /// More than one `Gemm`/`MatMul` node found.
    TooManyComputeNodes { count: usize },
    /// A referenced initializer is absent.
    MissingInitializer { name: String },
    /// An initializer has the wrong dtype, shape, rank or data.
    BadInitializer { name: String },
    /// A scale is not a finite positive scalar.
    BadScale { name: String },
    /// A zero-point is nonzero; the integer runtime has no zero-point term.
    NonZeroZeroPoint { name: String },
    /// A `Gemm` attribute outside the supported set (unit alpha, matching
    /// beta, no transpose).
    BadAttribute {
        op: &'static str,
        attr: &'static str,
    },
    /// Graph shape outside the single-layer input-to-output path.
    BadTopology { what: &'static str },
    /// A dim is zero or above [`MAX_DIM`].
    DimTooLarge { dim: usize, cap: usize },
    /// A tensor element count passes [`MAX_TENSOR_ELEMENTS`].
    TensorTooLarge { elements: usize, cap: usize },
    /// A folded scale ratio fits no `i32` multiplier at any shift.
    ScaleOverflow,
    /// Emitted bytes failed the [`load`] check (should not happen).
    Nn(NnErr),
    /// Tract float reference failed (parse, optimize or run).
    Tract(String),
    /// Float output length differs from the integer output length.
    UnexpectedOutput { len: usize, want: usize },
    /// Float and integer outputs disagree past [`REFERENCE_TOLERANCE`].
    ToleranceExceeded { max_abs_diff: f32, tolerance: f32 },
}

impl core::fmt::Display for ConvertErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConvertErr::EmptyInput => f.write_str("empty onnx input"),
            ConvertErr::TooLarge { bytes, cap } => {
                write!(f, "onnx input too large: {bytes} bytes, cap {cap}")
            }
            ConvertErr::BadProto => f.write_str("not a decodable onnx model"),
            ConvertErr::MissingGraph => f.write_str("onnx model has no graph"),
            ConvertErr::TooManyNodes { count, cap } => {
                write!(f, "too many nodes: {count}, cap {cap}")
            }
            ConvertErr::TooManyInitializers { count, cap } => {
                write!(f, "too many initializers: {count}, cap {cap}")
            }
            ConvertErr::TooManyInputs { count, cap } => {
                write!(f, "too many graph inputs: {count}, cap {cap}")
            }
            ConvertErr::UnsupportedOp { op } => write!(f, "unsupported operator: {op}"),
            ConvertErr::NoComputeNode => f.write_str("no Gemm or MatMul node"),
            ConvertErr::TooManyComputeNodes { count } => {
                write!(f, "too many compute nodes: {count}")
            }
            ConvertErr::MissingInitializer { name } => {
                write!(f, "missing initializer: {name}")
            }
            ConvertErr::BadInitializer { name } => write!(f, "bad initializer: {name}"),
            ConvertErr::BadScale { name } => write!(f, "bad scale: {name}"),
            ConvertErr::NonZeroZeroPoint { name } => {
                write!(f, "nonzero zero-point: {name}")
            }
            ConvertErr::BadAttribute { op, attr } => {
                write!(f, "unsupported {op} attribute: {attr}")
            }
            ConvertErr::BadTopology { what } => write!(f, "unsupported graph shape: {what}"),
            ConvertErr::DimTooLarge { dim, cap } => {
                write!(f, "dim too large: {dim}, cap {cap}")
            }
            ConvertErr::TensorTooLarge { elements, cap } => {
                write!(f, "tensor too large: {elements} elements, cap {cap}")
            }
            ConvertErr::ScaleOverflow => f.write_str("scale ratio fits no i32 multiplier"),
            ConvertErr::Nn(e) => write!(f, "emitted bytes rejected: {e}"),
            ConvertErr::Tract(e) => write!(f, "tract reference failed: {e}"),
            ConvertErr::UnexpectedOutput { len, want } => {
                write!(f, "float output len {len}, want {want}")
            }
            ConvertErr::ToleranceExceeded {
                max_abs_diff,
                tolerance,
            } => write!(
                f,
                "float/integer disagree: {max_abs_diff} past tolerance {tolerance}"
            ),
        }
    }
}

impl std::error::Error for ConvertErr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConvertErr::Nn(e) => Some(e),
            _ => None,
        }
    }
}

/// Float-vs-integer comparison from [`reference_check`].
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceReport {
    /// Tract float outputs, in output units.
    pub float_out: Vec<f32>,
    /// Integer-runtime outputs (raw `i32`, still scaled by the output scale).
    pub int_out: Vec<i32>,
    /// Largest `|float - int * output_scale|` over outputs.
    pub max_abs_diff: f32,
}

fn tract_err(e: impl core::fmt::Debug) -> ConvertErr {
    ConvertErr::Tract(format!("{e:?}"))
}

/// Validate ONNX bytes, map the single QDQ layer to dense integers and emit
/// runtime-loadable [`.e2nn`](crate) bytes. Pure function of the input bytes.
pub fn convert(onnx_bytes: &[u8]) -> Result<E2nnBytes, ConvertErr> {
    let mapped = map(onnx_bytes)?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(1);
    out.extend_from_slice(&(mapped.input_dim as u16).to_le_bytes());
    out.push(TAG_DENSE);
    out.extend_from_slice(&(mapped.output_dim as u16).to_le_bytes());
    out.extend_from_slice(&mapped.mult.to_le_bytes());
    out.push(mapped.shift);
    out.push(mapped.act);
    for b in &mapped.bias {
        out.extend_from_slice(&b.to_le_bytes());
    }
    out.extend(mapped.weights.iter().map(|w| w.cast_unsigned()));
    load(&out).map_err(ConvertErr::Nn)?;
    Ok(out)
}

/// Run the tract float reference on `onnx_bytes` and the integer runtime
/// inference on `e2nn_bytes` for the same `input`, and report agreement.
/// The float input is the `i8` input times the ONNX input scale (short inputs
/// zero-pad, long inputs truncate, mirroring [`infer`]); the integer output
/// is compared after scaling by the ONNX output scale. Errors with
/// [`ConvertErr::ToleranceExceeded`] past [`REFERENCE_TOLERANCE`].
pub fn reference_check(
    onnx_bytes: &[u8],
    e2nn_bytes: &[u8],
    input: &[i8],
) -> Result<ReferenceReport, ConvertErr> {
    let mapped = map(onnx_bytes)?;
    let model = load(e2nn_bytes).map_err(ConvertErr::Nn)?;
    let int_out = infer(&model, input);
    let mut floats = Vec::with_capacity(mapped.input_dim);
    for i in 0..mapped.input_dim {
        let q = if i < input.len() { input[i] } else { 0 };
        floats.push(f32::from(q) * mapped.input_scale);
    }
    let tensor = Tensor::from_shape(&[1, mapped.input_dim], &floats).map_err(tract_err)?;
    let runnable = Arc::new(
        tract_onnx::onnx()
            .model_for_read(&mut &*onnx_bytes)
            .map_err(tract_err)?
            .into_optimized()
            .map_err(tract_err)?
            .into_runnable()
            .map_err(tract_err)?,
    );
    let outputs = runnable
        .run(tvec![tensor.into_tvalue()])
        .map_err(tract_err)?;
    let first = outputs
        .first()
        .ok_or_else(|| ConvertErr::Tract("model returned no outputs".to_string()))?;
    let float_out = read_float_output(first, mapped.output_scale)?;
    if float_out.len() != int_out.len() {
        return Err(ConvertErr::UnexpectedOutput {
            len: float_out.len(),
            want: int_out.len(),
        });
    }
    let mut max_abs_diff = 0.0f32;
    for (f, q) in float_out.iter().zip(int_out.iter()) {
        let d = (f - (*q as f32) * mapped.output_scale).abs();
        if d > max_abs_diff {
            max_abs_diff = d;
        }
    }
    if max_abs_diff > REFERENCE_TOLERANCE {
        return Err(ConvertErr::ToleranceExceeded {
            max_abs_diff,
            tolerance: REFERENCE_TOLERANCE,
        });
    }
    Ok(ReferenceReport {
        float_out,
        int_out,
        max_abs_diff,
    })
}

/// Read one tract output tensor as floats in output units. Integer outputs
/// (a graph ending at `QuantizeLinear`) dequantize with the output scale;
/// zero-points on that path are already checked zero by [`map`].
fn read_float_output(output: &TValue, output_scale: f32) -> Result<Vec<f32>, ConvertErr> {
    match output.datum_type() {
        DatumType::F32 => output
            .to_plain_array_view::<f32>()
            .map_err(tract_err)
            .map(|v| v.iter().copied().collect()),
        DatumType::I8 => output
            .to_plain_array_view::<i8>()
            .map_err(tract_err)
            .map(|v| v.iter().map(|q| f32::from(*q) * output_scale).collect()),
        DatumType::U8 => output
            .to_plain_array_view::<u8>()
            .map_err(tract_err)
            .map(|v| v.iter().map(|q| f32::from(*q) * output_scale).collect()),
        dt => Err(ConvertErr::Tract(format!(
            "unsupported output datum {dt:?}"
        ))),
    }
}

// --- mapping ---

use tract_onnx::pb::{GraphProto, ModelProto, NodeProto, TensorProto};

/// The single mapped dense layer plus the scales the reference needs.
struct Mapped {
    input_dim: usize,
    output_dim: usize,
    input_scale: f32,
    output_scale: f32,
    mult: i32,
    shift: u8,
    act: u8,
    bias: Vec<i32>,
    /// Row-major `output_dim x input_dim`.
    weights: Vec<i8>,
}

fn decode(onnx_bytes: &[u8]) -> Result<ModelProto, ConvertErr> {
    if onnx_bytes.is_empty() {
        return Err(ConvertErr::EmptyInput);
    }
    if onnx_bytes.len() > MAX_ONNX_BYTES {
        return Err(ConvertErr::TooLarge {
            bytes: onnx_bytes.len(),
            cap: MAX_ONNX_BYTES,
        });
    }
    let proto = tract_onnx::onnx()
        .proto_model_for_read(&mut &*onnx_bytes)
        .map_err(|_| ConvertErr::BadProto)?;
    Ok(proto)
}

/// Decode, bound, whitelist-check and map to one dense layer.
fn map(onnx_bytes: &[u8]) -> Result<Mapped, ConvertErr> {
    let proto = decode(onnx_bytes)?;
    let graph = proto.graph.as_ref().ok_or(ConvertErr::MissingGraph)?;
    if graph.node.len() > MAX_NODES {
        return Err(ConvertErr::TooManyNodes {
            count: graph.node.len(),
            cap: MAX_NODES,
        });
    }
    if graph.initializer.len() > MAX_INITIALIZERS {
        return Err(ConvertErr::TooManyInitializers {
            count: graph.initializer.len(),
            cap: MAX_INITIALIZERS,
        });
    }
    if graph.input.len() > MAX_GRAPH_INPUTS {
        return Err(ConvertErr::TooManyInputs {
            count: graph.input.len(),
            cap: MAX_GRAPH_INPUTS,
        });
    }
    if graph.input.is_empty() {
        return Err(ConvertErr::BadTopology {
            what: "graph has no inputs",
        });
    }
    if graph.output.len() != 1 {
        return Err(ConvertErr::BadTopology {
            what: "graph must have exactly one output",
        });
    }
    Whitelist::check_ops(graph.node.iter().map(|n| n.op_type.as_str()))?;

    let mut inits = std::collections::HashMap::new();
    for t in &graph.initializer {
        if inits.contains_key(t.name.as_str()) {
            return Err(ConvertErr::BadInitializer {
                name: t.name.clone(),
            });
        }
        inits.insert(t.name.as_str(), t);
    }

    let compute: Vec<usize> = graph
        .node
        .iter()
        .enumerate()
        .filter(|(_, n)| n.op_type == "Gemm" || n.op_type == "MatMul")
        .map(|(i, _)| i)
        .collect();
    if compute.is_empty() {
        return Err(ConvertErr::NoComputeNode);
    }
    if compute.len() > 1 {
        return Err(ConvertErr::TooManyComputeNodes {
            count: compute.len(),
        });
    }
    let ci = compute[0];
    let node = &graph.node[ci];
    let is_gemm = node.op_type == "Gemm";
    let a_name = node
        .input
        .first()
        .filter(|s| !s.is_empty())
        .ok_or(ConvertErr::BadTopology {
            what: "compute node is missing its data input",
        })?;
    let b_name = node
        .input
        .get(1)
        .filter(|s| !s.is_empty())
        .ok_or(ConvertErr::BadTopology {
            what: "compute node is missing its weight input",
        })?;
    if is_gemm && node.input.len() > 3 || !is_gemm && node.input.len() != 2 {
        return Err(ConvertErr::BadTopology {
            what: "compute node has the wrong input count",
        });
    }
    let c_name = if is_gemm {
        node.input.get(2).filter(|s| !s.is_empty())
    } else {
        None
    };

    let b = inits
        .get(b_name.as_str())
        .ok_or_else(|| ConvertErr::MissingInitializer {
            name: (*b_name).clone(),
        })?;
    let (weight_f, k, n) = read_weight(b)?;
    if is_gemm {
        check_gemm_attrs(node, c_name.is_some())?;
    }
    let bias_f = match c_name {
        Some(name) => {
            let t = inits
                .get(name.as_str())
                .ok_or_else(|| ConvertErr::MissingInitializer {
                    name: (*name).clone(),
                })?;
            read_bias(t, n)?
        }
        None => vec![0.0f32; n],
    };

    let mut visited = vec![false; graph.node.len()];
    visited[ci] = true;
    let input_scale = walk_input(graph, &inits, a_name, &mut visited)?;
    let (output_scale, act) = walk_output(graph, &inits, node, &mut visited)?;
    if visited.iter().any(|v| !v) {
        return Err(ConvertErr::BadTopology {
            what: "a node sits outside the single-layer path",
        });
    }

    // Fold scales into integers: each output is
    // round(sum(input_q * w_q) * input_scale * w_scale + bias_f) in
    // output units, so the requant ratio is input_scale * w_scale / out.
    let w_scale = crate::weight_scale(&weight_f);
    let denom = f64::from(input_scale) * f64::from(w_scale);
    let mut bias = Vec::with_capacity(n);
    for b in &bias_f {
        if !b.is_finite() {
            return Err(ConvertErr::BadInitializer {
                name: c_name.map_or_else(|| "<bias>".to_string(), |s| s.clone()),
            });
        }
        let q = f64::from(*b) / denom;
        bias.push(q.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32);
    }
    let ratio = denom / f64::from(output_scale);
    let (mult, shift) = requant_params(ratio)?;
    let mut weights = vec![0i8; n * k];
    for (o, row) in weights.chunks_exact_mut(k).enumerate() {
        for (kk, slot) in row.iter_mut().enumerate() {
            // Gemm B is K x N row-major; dense wants output-major rows.
            let w = weight_f[kk * n + o] / w_scale;
            *slot = w.round().clamp(-128.0, 127.0) as i8;
        }
    }
    Ok(Mapped {
        input_dim: k,
        output_dim: n,
        input_scale,
        output_scale,
        mult,
        shift,
        act,
        bias,
        weights,
    })
}

/// Trace the compute data input back through quantization nodes to a graph
/// input; returns the input scale (1.0 when the input is unquantized).
fn walk_input(
    graph: &GraphProto,
    inits: &std::collections::HashMap<&str, &TensorProto>,
    a_name: &str,
    visited: &mut [bool],
) -> Result<f32, ConvertErr> {
    let mut cur = a_name.to_string();
    let mut scale = 1.0f32;
    for _ in 0..graph.node.len() + 1 {
        if graph.input.iter().any(|v| v.name == cur) {
            return Ok(scale);
        }
        let next = producer(&graph.node, &cur).ok_or_else(|| {
            if inits.contains_key(cur.as_str()) {
                ConvertErr::BadTopology {
                    what: "compute input is a constant",
                }
            } else {
                ConvertErr::BadTopology {
                    what: "dangling tensor name",
                }
            }
        })?;
        visited[next] = true;
        let node = &graph.node[next];
        match node.op_type.as_str() {
            "QuantizeLinear" => {
                scale = read_scale(node, inits)?;
                check_zero_point(node, inits)?;
                cur = data_input(node)?;
            }
            "DequantizeLinear" => {
                check_zero_point(node, inits)?;
                cur = data_input(node)?;
            }
            _ => {
                return Err(ConvertErr::BadTopology {
                    what: "node feeding compute is not quantization",
                });
            }
        }
    }
    Err(ConvertErr::BadTopology {
        what: "input quantization chain too deep",
    })
}

/// Walk the compute output forward to the graph output; returns the output
/// scale (1.0 when unquantized) and the activation.
fn walk_output(
    graph: &GraphProto,
    inits: &std::collections::HashMap<&str, &TensorProto>,
    compute: &NodeProto,
    visited: &mut [bool],
) -> Result<(f32, u8), ConvertErr> {
    let want = graph.output[0].name.clone();
    let mut cur = compute
        .output
        .first()
        .filter(|s| !s.is_empty())
        .ok_or(ConvertErr::BadTopology {
            what: "compute node has no output",
        })?
        .clone();
    let mut scale = 1.0f32;
    let mut act = ACT_NONE;
    for _ in 0..graph.node.len() + 1 {
        if cur == want {
            return Ok((scale, act));
        }
        let users = consumers(&graph.node, &cur);
        if users.len() != 1 {
            return Err(ConvertErr::BadTopology {
                what: "compute output must feed exactly one node",
            });
        }
        let u = users[0];
        visited[u] = true;
        let node = &graph.node[u];
        match node.op_type.as_str() {
            "Relu" => {
                act = ACT_RELU;
                cur = node_output(node)?;
            }
            "QuantizeLinear" => {
                scale = read_scale(node, inits)?;
                check_zero_point(node, inits)?;
                cur = node_output(node)?;
            }
            "DequantizeLinear" => {
                check_zero_point(node, inits)?;
                cur = node_output(node)?;
            }
            _ => {
                return Err(ConvertErr::BadTopology {
                    what: "node after compute is not Relu or quantization",
                });
            }
        }
    }
    Err(ConvertErr::BadTopology {
        what: "output quantization chain too deep",
    })
}

fn producer(nodes: &[NodeProto], name: &str) -> Option<usize> {
    nodes
        .iter()
        .position(|n| n.output.iter().any(|o| o == name))
}

fn consumers(nodes: &[NodeProto], name: &str) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.input.iter().any(|i| i == name))
        .map(|(i, _)| i)
        .collect()
}

fn data_input(node: &NodeProto) -> Result<String, ConvertErr> {
    node.input
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or(ConvertErr::BadTopology {
            what: "quantization node has no data input",
        })
}

fn node_output(node: &NodeProto) -> Result<String, ConvertErr> {
    node.output
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or(ConvertErr::BadTopology {
            what: "node has no output",
        })
}

fn check_gemm_attrs(node: &NodeProto, has_bias: bool) -> Result<(), ConvertErr> {
    for attr in &node.attribute {
        match attr.name.as_str() {
            "transA" => {
                if attr.i != 0 {
                    return Err(ConvertErr::BadAttribute {
                        op: "Gemm",
                        attr: "transA",
                    });
                }
            }
            "transB" => {
                if attr.i != 0 {
                    return Err(ConvertErr::BadAttribute {
                        op: "Gemm",
                        attr: "transB",
                    });
                }
            }
            "alpha" => {
                if attr.f != 1.0 {
                    return Err(ConvertErr::BadAttribute {
                        op: "Gemm",
                        attr: "alpha",
                    });
                }
            }
            "beta" if has_bias && attr.f != 1.0 => {
                return Err(ConvertErr::BadAttribute {
                    op: "Gemm",
                    attr: "beta",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read the `f32` weight matrix: rank 2, `K x N`, within the runtime dim caps.
fn read_weight(t: &TensorProto) -> Result<(Vec<f32>, usize, usize), ConvertErr> {
    let name = t.name.clone();
    if t.dims.len() != 2 {
        return Err(ConvertErr::BadInitializer { name });
    }
    let k = dim_len(&t.dims[0], &name)?;
    let n = dim_len(&t.dims[1], &name)?;
    if k == 0 || k > MAX_DIM {
        return Err(ConvertErr::DimTooLarge {
            dim: k,
            cap: MAX_DIM,
        });
    }
    if n == 0 || n > MAX_DIM {
        return Err(ConvertErr::DimTooLarge {
            dim: n,
            cap: MAX_DIM,
        });
    }
    let values = f32_of(t)?;
    debug_assert_eq!(values.len(), k * n);
    Ok((values, k, n))
}

fn read_bias(t: &TensorProto, n: usize) -> Result<Vec<f32>, ConvertErr> {
    let name = t.name.clone();
    if t.dims.len() != 1 {
        return Err(ConvertErr::BadInitializer { name });
    }
    let len = dim_len(&t.dims[0], &name)?;
    if len != n {
        return Err(ConvertErr::BadInitializer { name });
    }
    f32_of(t)
}

fn dim_len(dim: &i64, name: &str) -> Result<usize, ConvertErr> {
    usize::try_from(*dim).map_err(|_| ConvertErr::BadInitializer {
        name: name.to_string(),
    })
}

/// Scale input of a `QuantizeLinear` node: finite positive scalar.
fn read_scale(
    node: &NodeProto,
    inits: &std::collections::HashMap<&str, &TensorProto>,
) -> Result<f32, ConvertErr> {
    let name = node
        .input
        .get(1)
        .filter(|s| !s.is_empty())
        .ok_or(ConvertErr::BadTopology {
            what: "quantization node has no scale input",
        })?;
    let t = inits
        .get(name.as_str())
        .ok_or_else(|| ConvertErr::MissingInitializer {
            name: (*name).clone(),
        })?;
    let values = f32_of(t)?;
    if values.len() != 1 || !values[0].is_finite() || values[0] <= 0.0 {
        return Err(ConvertErr::BadScale {
            name: (*name).clone(),
        });
    }
    Ok(values[0])
}

/// Zero-point input of a quantization node: absent means zero, present must
/// be a single zero integer (per-axis quantization is unsupported).
fn check_zero_point(
    node: &NodeProto,
    inits: &std::collections::HashMap<&str, &TensorProto>,
) -> Result<(), ConvertErr> {
    let Some(name) = node.input.get(2).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let t = inits
        .get(name.as_str())
        .ok_or_else(|| ConvertErr::MissingInitializer {
            name: (*name).clone(),
        })?;
    let values = ints_of(t)?;
    if values.len() != 1 {
        return Err(ConvertErr::BadInitializer {
            name: (*name).clone(),
        });
    }
    if values[0] != 0 {
        return Err(ConvertErr::NonZeroZeroPoint {
            name: (*name).clone(),
        });
    }
    Ok(())
}

fn count_elements(dims: &[i64], name: &str) -> Result<usize, ConvertErr> {
    if dims.len() > MAX_RANK {
        return Err(ConvertErr::BadInitializer {
            name: name.to_string(),
        });
    }
    let mut n = 1usize;
    for d in dims {
        if *d < 0 {
            return Err(ConvertErr::BadInitializer {
                name: name.to_string(),
            });
        }
        n = n
            .checked_mul(*d as usize)
            .ok_or(ConvertErr::TensorTooLarge {
                elements: usize::MAX,
                cap: MAX_TENSOR_ELEMENTS,
            })?;
        if n > MAX_TENSOR_ELEMENTS {
            return Err(ConvertErr::TensorTooLarge {
                elements: n,
                cap: MAX_TENSOR_ELEMENTS,
            });
        }
    }
    Ok(n)
}

/// `f32` tensor contents from `float_data` or little-endian `raw_data`.
fn f32_of(t: &TensorProto) -> Result<Vec<f32>, ConvertErr> {
    let name = t.name.clone();
    if t.data_type != 1 {
        return Err(ConvertErr::BadInitializer { name });
    }
    if !t.external_data.is_empty() {
        return Err(ConvertErr::BadInitializer { name });
    }
    let elements = count_elements(&t.dims, &name)?;
    if !t.raw_data.is_empty() {
        if !t.float_data.is_empty() {
            return Err(ConvertErr::BadInitializer { name });
        }
        if t.raw_data.len() != elements * 4 {
            return Err(ConvertErr::BadInitializer { name });
        }
        let mut out = Vec::with_capacity(elements);
        for chunk in t.raw_data.chunks_exact(4) {
            let bytes: [u8; 4] = chunk
                .try_into()
                .map_err(|_| ConvertErr::BadInitializer { name: name.clone() })?;
            out.push(f32::from_le_bytes(bytes));
        }
        return Ok(out);
    }
    if t.float_data.len() != elements {
        return Err(ConvertErr::BadInitializer { name });
    }
    Ok(t.float_data.clone())
}

/// Integer tensor contents (`int8`/`uint8`/`int32`/`int64`) as `i64`.
fn ints_of(t: &TensorProto) -> Result<Vec<i64>, ConvertErr> {
    let name = t.name.clone();
    let width = match t.data_type {
        2 | 3 => 1,
        6 => 4,
        7 => 8,
        _ => return Err(ConvertErr::BadInitializer { name }),
    };
    if !t.external_data.is_empty() {
        return Err(ConvertErr::BadInitializer { name });
    }
    let elements = count_elements(&t.dims, &name)?;
    if !t.raw_data.is_empty() {
        if !t.int32_data.is_empty() || !t.int64_data.is_empty() {
            return Err(ConvertErr::BadInitializer { name });
        }
        if t.raw_data.len() != elements * width {
            return Err(ConvertErr::BadInitializer { name });
        }
        let mut out = Vec::with_capacity(elements);
        for chunk in t.raw_data.chunks_exact(width) {
            let v = match width {
                1 => {
                    let unsigned = t.data_type == 2;
                    if unsigned {
                        i64::from(chunk[0])
                    } else {
                        i64::from(chunk[0].cast_signed())
                    }
                }
                4 => {
                    let bytes: [u8; 4] = chunk
                        .try_into()
                        .map_err(|_| ConvertErr::BadInitializer { name: name.clone() })?;
                    i64::from(i32::from_le_bytes(bytes))
                }
                _ => {
                    let bytes: [u8; 8] = chunk
                        .try_into()
                        .map_err(|_| ConvertErr::BadInitializer { name: name.clone() })?;
                    i64::from_le_bytes(bytes)
                }
            };
            out.push(v);
        }
        return Ok(out);
    }
    if t.data_type == 7 {
        if t.int64_data.len() != elements {
            return Err(ConvertErr::BadInitializer { name });
        }
        return Ok(t.int64_data.clone());
    }
    if t.int32_data.len() != elements {
        return Err(ConvertErr::BadInitializer { name });
    }
    Ok(t.int32_data.iter().map(|v| i64::from(*v)).collect())
}

/// Largest shift `0..=31` whose rounded multiplier fits `i32`.
fn requant_params(ratio: f64) -> Result<(i32, u8), ConvertErr> {
    crate::requant_params(ratio).ok_or(ConvertErr::ScaleOverflow)
}

#[cfg(test)]
mod convert_tests {
    use super::*;
    use crate::{infer, load};

    // --- tiny protobuf writer (test-only; the fixture is hand-built bytes,
    // no shipped weights) ---

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    fn tag(field: u32, wire: u8, out: &mut Vec<u8>) {
        varint((field << 3 | u32::from(wire)) as u64, out);
    }

    fn field_bytes(field: u32, data: &[u8], out: &mut Vec<u8>) {
        tag(field, 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    fn field_str(field: u32, s: &str, out: &mut Vec<u8>) {
        field_bytes(field, s.as_bytes(), out);
    }

    fn field_msg(field: u32, msg: &[u8], out: &mut Vec<u8>) {
        field_bytes(field, msg, out);
    }

    fn field_varint(field: u32, v: i64, out: &mut Vec<u8>) {
        tag(field, 0, out);
        varint(v as u64, out);
    }

    fn packed_ints(field: u32, xs: &[i64], out: &mut Vec<u8>) {
        let mut payload = Vec::new();
        for x in xs {
            varint(*x as u64, &mut payload);
        }
        field_bytes(field, &payload, out);
    }

    fn packed_floats(field: u32, xs: &[f32], out: &mut Vec<u8>) {
        let mut payload = Vec::new();
        for x in xs {
            payload.extend_from_slice(&x.to_le_bytes());
        }
        field_bytes(field, &payload, out);
    }

    struct TensorSpec {
        name: &'static str,
        dtype: i32,
        dims: Vec<i64>,
        floats: Vec<f32>,
        int32s: Vec<i32>,
    }

    fn tensor_proto(s: &TensorSpec) -> Vec<u8> {
        let mut o = Vec::new();
        if !s.dims.is_empty() {
            packed_ints(1, &s.dims, &mut o);
        }
        field_varint(2, i64::from(s.dtype), &mut o);
        if !s.floats.is_empty() {
            packed_floats(4, &s.floats, &mut o);
        }
        if !s.int32s.is_empty() {
            let ints: Vec<i64> = s.int32s.iter().map(|v| i64::from(*v)).collect();
            packed_ints(5, &ints, &mut o);
        }
        field_str(8, s.name, &mut o);
        o
    }

    struct NodeSpec {
        name: &'static str,
        op: &'static str,
        inputs: Vec<&'static str>,
        outputs: Vec<&'static str>,
    }

    fn node(
        name: &'static str,
        op: &'static str,
        inputs: &[&'static str],
        outputs: &[&'static str],
    ) -> NodeSpec {
        NodeSpec {
            name,
            op,
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
        }
    }

    fn node_proto(s: &NodeSpec) -> Vec<u8> {
        let mut o = Vec::new();
        for i in &s.inputs {
            field_str(1, i, &mut o);
        }
        for x in &s.outputs {
            field_str(2, x, &mut o);
        }
        field_str(3, s.name, &mut o);
        field_str(4, s.op, &mut o);
        o
    }

    fn value_info(name: &str, rows: i64, cols: i64) -> Vec<u8> {
        let mut dim1 = Vec::new();
        field_varint(1, rows, &mut dim1);
        let mut dim2 = Vec::new();
        field_varint(1, cols, &mut dim2);
        let mut shape = Vec::new();
        field_msg(1, &dim1, &mut shape);
        field_msg(1, &dim2, &mut shape);
        let mut tensor = Vec::new();
        field_varint(1, 1, &mut tensor);
        field_msg(2, &shape, &mut tensor);
        let mut ty = Vec::new();
        field_msg(1, &tensor, &mut ty);
        let mut vi = Vec::new();
        field_str(1, name, &mut vi);
        field_msg(2, &ty, &mut vi);
        vi
    }

    fn model_bytes(nodes: &[NodeSpec], inits: &[TensorSpec], out_name: &str) -> Vec<u8> {
        let mut g = Vec::new();
        for n in nodes {
            field_msg(1, &node_proto(n), &mut g);
        }
        field_str(2, "qdq-gemm", &mut g);
        for t in inits {
            field_msg(5, &tensor_proto(t), &mut g);
        }
        field_msg(11, &value_info("x", 1, 2), &mut g);
        field_msg(12, &value_info(out_name, 1, 2), &mut g);
        let mut opset = Vec::new();
        field_varint(2, 13, &mut opset);
        let mut m = Vec::new();
        field_varint(1, 8, &mut m);
        field_str(2, "engine-nn-test", &mut m);
        field_msg(8, &opset, &mut m);
        field_msg(7, &g, &mut m);
        m
    }

    /// Hand-built int8-QDQ net: `y = relu(x @ [[6, -6], [6, 6]] + [1, -2])`
    /// with unit scales and zero zero-points. For `x = [7, 8]` the float
    /// result is `[91, 4]`; weights quantize exactly (`max |w| = 6`).
    fn base_inits() -> Vec<TensorSpec> {
        vec![
            TensorSpec {
                name: "w",
                dtype: 1,
                dims: vec![2, 2],
                floats: vec![6.0, -6.0, 6.0, 6.0],
                int32s: vec![],
            },
            TensorSpec {
                name: "b",
                dtype: 1,
                dims: vec![2],
                floats: vec![1.0, -2.0],
                int32s: vec![],
            },
            TensorSpec {
                name: "xs",
                dtype: 1,
                dims: vec![],
                floats: vec![1.0],
                int32s: vec![],
            },
            TensorSpec {
                name: "xz",
                dtype: 3,
                dims: vec![],
                floats: vec![],
                int32s: vec![0],
            },
            TensorSpec {
                name: "ys",
                dtype: 1,
                dims: vec![],
                floats: vec![1.0],
                int32s: vec![],
            },
            TensorSpec {
                name: "yz",
                dtype: 3,
                dims: vec![],
                floats: vec![],
                int32s: vec![0],
            },
        ]
    }

    fn base_nodes() -> Vec<NodeSpec> {
        vec![
            node("q_in", "QuantizeLinear", &["x", "xs", "xz"], &["xq"]),
            node("dq_in", "DequantizeLinear", &["xq", "xs", "xz"], &["xd"]),
            node("gemm", "Gemm", &["xd", "w", "b"], &["g"]),
            node("relu", "Relu", &["g"], &["r"]),
            node("q_out", "QuantizeLinear", &["r", "ys", "yz"], &["yq"]),
            node("dq_out", "DequantizeLinear", &["yq", "ys", "yz"], &["y"]),
        ]
    }

    fn fixture() -> Vec<u8> {
        model_bytes(&base_nodes(), &base_inits(), "y")
    }

    #[test]
    fn whitelist_lists_and_enforces() {
        for op in Whitelist::OPS {
            assert!(Whitelist::allows(op), "{op} must be allowed");
        }
        for op in ["Conv", "Sigmoid", "GemmTransposed", ""] {
            assert!(!Whitelist::allows(op), "{op} must be refused");
        }
        Whitelist::check_ops(Whitelist::OPS).expect("all listed ops pass");
        assert_eq!(
            Whitelist::check_ops(["Gemm", "Conv"]),
            Err(ConvertErr::UnsupportedOp {
                op: "Conv".to_string()
            })
        );
    }

    #[test]
    fn fixture_converts_and_agrees() {
        let onnx = fixture();
        let e2nn = convert(&onnx).expect("fixture converts");
        let model = load(&e2nn).expect("converted bytes load in NN1");
        assert_eq!(model.input_dim(), 2);
        assert_eq!(model.output_dim(), 2);
        assert_eq!(infer(&model, &[7, 8]), vec![91, 4]);

        let report = reference_check(&onnx, &e2nn, &[7, 8]).expect("agree within tolerance");
        assert_eq!(report.int_out, vec![91, 4]);
        assert_eq!(report.float_out.len(), 2);
        for (f, want) in report.float_out.iter().zip([91.0f32, 4.0]) {
            assert!((f - want).abs() < 1e-6, "float {f} near {want}");
        }
        assert_eq!(report.max_abs_diff, 0.0);
    }

    #[test]
    fn reference_pads_short_inputs() {
        let onnx = fixture();
        let e2nn = convert(&onnx).expect("fixture converts");
        // x = [7, 0]: y = [43, -44] before Relu.
        let report = reference_check(&onnx, &e2nn, &[7]).expect("padded input agrees");
        assert_eq!(report.int_out, vec![43, 0]);
        for (f, want) in report.float_out.iter().zip([43.0f32, 0.0]) {
            assert!((f - want).abs() < 1e-6, "float {f} near {want}");
        }
    }

    #[test]
    fn matmul_variant_converts() {
        // Same weights, no bias: y = relu(x @ W) = [90, 6].
        let nodes = vec![
            node("q_in", "QuantizeLinear", &["x", "xs", "xz"], &["xq"]),
            node("dq_in", "DequantizeLinear", &["xq", "xs", "xz"], &["xd"]),
            node("mm", "MatMul", &["xd", "w"], &["g"]),
            node("relu", "Relu", &["g"], &["r"]),
            node("q_out", "QuantizeLinear", &["r", "ys", "yz"], &["yq"]),
            node("dq_out", "DequantizeLinear", &["yq", "ys", "yz"], &["y"]),
        ];
        let inits: Vec<TensorSpec> = base_inits().into_iter().filter(|t| t.name != "b").collect();
        let onnx = model_bytes(&nodes, &inits, "y");
        let e2nn = convert(&onnx).expect("matmul converts");
        let model = load(&e2nn).expect("matmul bytes load");
        assert_eq!(infer(&model, &[7, 8]), vec![90, 6]);
        let report = reference_check(&onnx, &e2nn, &[7, 8]).expect("matmul agrees");
        assert_eq!(report.int_out, vec![90, 6]);
        assert_eq!(report.max_abs_diff, 0.0);
    }

    #[test]
    fn unsupported_operator_refused_by_name() {
        let mut nodes = base_nodes();
        nodes[3] = node("act", "Sigmoid", &["g"], &["r"]);
        let err = convert(&model_bytes(&nodes, &base_inits(), "y")).expect_err("Sigmoid refused");
        assert_eq!(
            err,
            ConvertErr::UnsupportedOp {
                op: "Sigmoid".to_string()
            }
        );

        let mut nodes = base_nodes();
        nodes[2] = node("conv", "Conv", &["xd", "w"], &["g"]);
        let err = convert(&model_bytes(&nodes, &base_inits(), "y")).expect_err("Conv refused");
        assert_eq!(
            err,
            ConvertErr::UnsupportedOp {
                op: "Conv".to_string()
            }
        );
    }

    #[test]
    fn malformed_rejected_without_panic() {
        assert_eq!(convert(&[]), Err(ConvertErr::EmptyInput));
        assert_eq!(
            convert(&[0xFF; 32]),
            Err(ConvertErr::BadProto),
            "garbage bytes are not a model"
        );
        assert_eq!(
            convert(b"not an onnx model at all .........."),
            Err(ConvertErr::BadProto),
            "text is not a model"
        );
        // Decodable protobuf with no graph.
        let mut bare = Vec::new();
        field_varint(1, 8, &mut bare);
        assert_eq!(convert(&bare), Err(ConvertErr::MissingGraph));

        // Oversize input refused before decode.
        let big = vec![0x08u8; MAX_ONNX_BYTES + 1];
        assert_eq!(
            convert(&big),
            Err(ConvertErr::TooLarge {
                bytes: MAX_ONNX_BYTES + 1,
                cap: MAX_ONNX_BYTES
            })
        );

        // Every truncation of the fixture errors; the full file converts.
        let good = fixture();
        for len in 0..good.len() {
            assert!(
                convert(&good[..len]).is_err(),
                "prefix {len} must not convert"
            );
        }
        convert(&good).expect("full fixture converts");
    }

    #[test]
    fn bounds_refused() {
        // 65 nodes over the cap.
        let mut nodes = Vec::new();
        for i in 0..65 {
            let out = Box::leak(format!("t{i}").into_boxed_str());
            nodes.push(NodeSpec {
                name: Box::leak(format!("n{i}").into_boxed_str()),
                op: "Relu",
                inputs: vec!["x"],
                outputs: vec![out],
            });
        }
        let err = convert(&model_bytes(&nodes, &base_inits(), "y")).expect_err("node cap");
        assert_eq!(
            err,
            ConvertErr::TooManyNodes {
                count: 65,
                cap: MAX_NODES
            }
        );

        // 2M-element scale over the tensor cap (no data needed: the bound
        // fires before the data check).
        let mut inits = base_inits();
        inits[2].dims = vec![2_000_000];
        inits[2].floats.clear();
        let err = convert(&model_bytes(&base_nodes(), &inits, "y")).expect_err("tensor cap");
        assert_eq!(
            err,
            ConvertErr::TensorTooLarge {
                elements: 2_000_000,
                cap: MAX_TENSOR_ELEMENTS
            }
        );

        // 2000-wide input over the runtime dim cap.
        let mut inits = base_inits();
        inits[0].dims = vec![2000, 2];
        inits[0].floats = vec![0.0; 4000];
        let err = convert(&model_bytes(&base_nodes(), &inits, "y")).expect_err("dim cap");
        assert_eq!(
            err,
            ConvertErr::DimTooLarge {
                dim: 2000,
                cap: MAX_DIM
            }
        );

        // Nonzero zero-point has no integer term.
        let mut inits = base_inits();
        inits[3].int32s = vec![1];
        let err = convert(&model_bytes(&base_nodes(), &inits, "y")).expect_err("zero-point");
        assert_eq!(
            err,
            ConvertErr::NonZeroZeroPoint {
                name: "xz".to_string()
            }
        );

        // Graph output nobody produces.
        let err =
            convert(&model_bytes(&base_nodes(), &base_inits(), "nope")).expect_err("dangling");
        assert!(matches!(err, ConvertErr::BadTopology { .. }), "got {err:?}");

        // No compute node at all.
        let nodes: Vec<NodeSpec> = base_nodes()
            .into_iter()
            .filter(|n| n.op != "Gemm")
            .collect();
        let err = convert(&model_bytes(&nodes, &base_inits(), "y")).expect_err("no compute");
        assert_eq!(err, ConvertErr::NoComputeNode);
    }

    #[test]
    fn converted_bytes_replay_bit_exactly() {
        let e2nn = convert(&fixture()).expect("fixture converts");
        let first = load(&e2nn).expect("first load");
        let second = load(&e2nn).expect("second load");
        assert_eq!(infer(&first, &[7, 8]), vec![91, 4]);
        assert_eq!(infer(&first, &[7, 8]), infer(&second, &[7, 8]));
        assert_eq!(infer(&first, &[7, 8]), infer(&first, &[7, 8]));
    }

    #[test]
    fn mismatched_weights_exceed_tolerance() {
        // A different net's bytes against this model's floats.
        let mut inits = base_inits();
        inits[0].floats = vec![1.0, 1.0, 1.0, 1.0];
        inits[1].floats = vec![0.0, 0.0];
        let other = convert(&model_bytes(&base_nodes(), &inits, "y")).expect("variant converts");
        let err = reference_check(&fixture(), &other, &[7, 8]).expect_err("must disagree");
        match err {
            ConvertErr::ToleranceExceeded {
                max_abs_diff,
                tolerance,
            } => {
                assert_eq!(tolerance, REFERENCE_TOLERANCE);
                assert!(
                    max_abs_diff > tolerance,
                    "diff {max_abs_diff} past {tolerance}"
                );
            }
            _ => panic!("wrong error: {err:?}"),
        }
    }

    #[test]
    fn requant_params_goldens() {
        // Largest shift whose rounded multiplier fits i32 (max precision).
        assert_eq!(requant_params(1.0), Ok((1 << 30, 30)));
        assert_eq!(requant_params(0.5), Ok((1 << 30, 31)));
        assert_eq!(requant_params(2.0), Ok((1 << 30, 29)));
        assert_eq!(requant_params(0.1), Ok((214_748_365, 31)));
        assert_eq!(requant_params(1e30), Err(ConvertErr::ScaleOverflow));
        assert_eq!(requant_params(0.0), Err(ConvertErr::ScaleOverflow));
        assert_eq!(requant_params(f64::NAN), Err(ConvertErr::ScaleOverflow));
        assert_eq!(requant_params(-1.0), Err(ConvertErr::ScaleOverflow));
    }
}
