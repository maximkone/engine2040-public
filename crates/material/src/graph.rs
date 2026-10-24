//! graph: a small typed node graph folded into the one material record.
//!
//! An artist writes a material as text: nodes with a kind, typed input
//! ports and one scalar output field, plus one `output` block that names
//! which field drives base color, roughness, metalness, height and the five
//! coverage channels. [`Graph::to_record`] folds it into the existing
//! [`MaterialRecord`] (never a second material model), and [`Graph::bake`]
//! samples the same fields on a square grid into four RGBA8 maps.
//!
//! Determinism: nodes are ordered by id, the sample grid is fixed
//! ([`SAMPLE_GRID`] for the fold, the bake size for a bake), no RNG and no
//! platform `powf` are used (value noise is an integer hash, `curve` is an
//! integer exponent), and every map is linear `0..=1` quantized to 8 bits,
//! so the same graph gives the same bytes twice. Bounds: [`MAX_NODES`]
//! nodes, [`SAMPLE_GRID`] squared samples for the fold, bake sizes
//! [`BAKE_SIZES`]. Errors: [`GraphError`] with a JSON path, a typed kind
//! and one recovery step; nothing here panics on caller text.

use std::collections::BTreeMap;
use std::fmt;

use engine_core::hash_bytes;

use crate::json::{self, Value};
use crate::record::{Channel, Layer, MaterialRecord, RecordError, RECORD_VERSION};

use eval::{compute, count_f32, decimal, layer_value, params_json, to_u8};

/// Most nodes one graph may hold.
pub const MAX_NODES: usize = 64;
/// Sample grid of a folded graph, samples per side.
pub const SAMPLE_GRID: u32 = 64;
/// Largest accepted graph text, bytes.
pub const MAX_GRAPH_BYTES: usize = 16 * 1024;
/// Bake sizes a caller may ask for, pixels per side.
pub const BAKE_SIZES: [u32; 2] = [512, 1024];
/// Map names a bake writes, in write order.
pub const BAKE_MAPS: [&str; 4] = ["base_color", "orm", "normal", "emissive"];

/// Names of the legal node kinds, for an error message.
pub const KINDS: [&str; 6] = ["noise", "tile", "gradient", "curve", "warp", "blend"];

/// What a node computes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    /// Hash-based value noise: fractal octaves of an integer hash.
    Noise,
    /// Brick course pattern: the surface input, cut by mortar lines.
    Tile,
    /// Linear or radial ramp between two values.
    Gradient,
    /// An integer exponent and an invert flag.
    Curve,
    /// Domain warp of one field by another.
    Warp,
    /// Two fields mixed by a third.
    Blend,
}

/// One typed input port of a node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Port {
    /// First input: the surface or the base field.
    A,
    /// Second input: the warp driver or the upper field.
    B,
    /// Blend mask.
    Mask,
}

impl Port {
    /// Document name of the port.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::Mask => "mask",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "a" => Some(Self::A),
            "b" => Some(Self::B),
            "mask" => Some(Self::Mask),
            _ => None,
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::Mask => 2,
        }
    }
}

impl NodeKind {
    /// Document name of the kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Noise => "noise",
            Self::Tile => "tile",
            Self::Gradient => "gradient",
            Self::Curve => "curve",
            Self::Warp => "warp",
            Self::Blend => "blend",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "noise" => Some(Self::Noise),
            "tile" => Some(Self::Tile),
            "gradient" => Some(Self::Gradient),
            "curve" => Some(Self::Curve),
            "warp" => Some(Self::Warp),
            "blend" => Some(Self::Blend),
            _ => None,
        }
    }

    /// The input ports this kind reads, in document order.
    #[must_use]
    pub const fn ports(self) -> &'static [Port] {
        match self {
            Self::Noise | Self::Gradient => &[],
            Self::Tile | Self::Curve => &[Port::A],
            Self::Warp => &[Port::A, Port::B],
            Self::Blend => &[Port::A, Port::B, Port::Mask],
        }
    }

    /// The `params` keys this kind reads.
    const fn param_keys(self) -> &'static [&'static str] {
        match self {
            Self::Noise => &["scale", "seed", "octaves"],
            Self::Tile => &["rows", "cols", "mortar"],
            Self::Gradient => &["axis", "from", "to"],
            Self::Curve => &["exponent", "invert"],
            Self::Warp => &["strength"],
            Self::Blend => &["mode"],
        }
    }
}

/// Ramp direction of a [`NodeKind::Gradient`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Axis {
    /// Left to right.
    #[default]
    X,
    /// Top to bottom.
    Y,
    /// Center outward.
    Radial,
}

impl Axis {
    /// Document name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Radial => "radial",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "radial" => Some(Self::Radial),
            _ => None,
        }
    }
}

/// How a [`NodeKind::Blend`] combines its two fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BlendMode {
    /// `a + b`, clamped to one.
    #[default]
    Add,
    /// `a * b`.
    Multiply,
    /// `a` by the mask, `b` by its complement.
    Mix,
    /// `2ab` below half, `1 - 2(1-a)(1-b)` above.
    Overlay,
}

impl BlendMode {
    /// Document name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Multiply => "multiply",
            Self::Mix => "mix",
            Self::Overlay => "overlay",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "add" => Some(Self::Add),
            "multiply" => Some(Self::Multiply),
            "mix" => Some(Self::Mix),
            "overlay" => Some(Self::Overlay),
            _ => None,
        }
    }
}

/// Every parameter a node may carry; each kind reads only its own keys.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    /// Noise lattice cells across the map, `1..=64`.
    pub scale: u32,
    /// Noise hash seed.
    pub seed: u32,
    /// Noise octaves, `1..=5`.
    pub octaves: u32,
    /// Tile rows, `1..=32`.
    pub rows: u32,
    /// Tile columns, `1..=32`.
    pub cols: u32,
    /// Mortar width as a fraction of one brick, `0..=0.5`.
    pub mortar: f32,
    /// Gradient ramp direction.
    pub axis: Axis,
    /// Gradient value at the low end, `0..=1`.
    pub from: f32,
    /// Gradient value at the high end, `0..=1`.
    pub to: f32,
    /// Curve exponent, `1..=8`.
    pub exponent: u32,
    /// Curve invert flag.
    pub invert: bool,
    /// Warp strength as a fraction of one cell, `0..=1`.
    pub strength: f32,
    /// Blend mode.
    pub mode: BlendMode,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            scale: 8,
            seed: 0,
            octaves: 3,
            rows: 4,
            cols: 4,
            mortar: 0.05,
            axis: Axis::X,
            from: 0.0,
            to: 1.0,
            exponent: 1,
            invert: false,
            strength: 0.5,
            mode: BlendMode::Mix,
        }
    }
}

/// One node: its id, kind, typed inputs and parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Node {
    /// Node id, `1..=MAX_NODES`; evaluation runs in id order.
    pub id: u32,
    /// What the node computes.
    pub kind: NodeKind,
    /// Input fields by port; `None` for a port the kind does not read.
    pub inputs: [Option<u32>; 3],
    /// Parameters.
    pub params: Params,
}

/// Which field drives each record number.
#[derive(Clone, PartialEq, Debug)]
pub struct Output {
    /// Node id of the field that mixes base color between `color_low` and
    /// `color_high`.
    pub color: u32,
    /// Base color of the field at zero, linear `0..=1`.
    pub color_low: [f32; 3],
    /// Base color of the field at one, linear `0..=1`.
    pub color_high: [f32; 3],
    /// Node id of the roughness field.
    pub roughness: u32,
    /// Node id of the metalness field.
    pub metalness: u32,
    /// Node id of the height field (also the normal-map source).
    pub height: u32,
    /// Node id per coverage channel; a channel that is absent stays zero.
    pub channels: BTreeMap<Channel, u32>,
    /// Node id of the emissive field.
    pub emissive: Option<u32>,
}

/// The graph document: name, parent record, nodes and the output block.
#[derive(Clone, PartialEq, Debug)]
pub struct Graph {
    /// Material name the folded record takes.
    pub material: String,
    /// Shipped record the folded record extends (body, sound and lobes).
    pub base: String,
    /// Nodes sorted by id.
    pub nodes: Vec<Node>,
    /// Which field drives which record number.
    pub output: Output,
    /// Gameplay layers the record carries (at most `MAX_LAYERS`).
    pub layers: Vec<Layer>,
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// What was wrong with a graph.
#[derive(Clone, PartialEq, Debug)]
pub enum GraphErrorKind {
    /// The text is not JSON.
    Json(json::JsonError),
    /// A graph value has another JSON kind; expected kind carried.
    Expected(&'static str),
    /// A member that must be present is absent.
    Missing,
    /// A key the document does not allow; legal keys carried.
    UnknownKey {
        /// The keys that are legal here.
        legal: Vec<&'static str>,
    },
    /// A node kind that does not exist; the kind as written is carried.
    UnknownKind {
        /// The kind text the document wrote.
        kind: String,
    },
    /// A port that does not exist on this kind, or a type mismatch.
    UnknownPort {
        /// The node the port was written on.
        node: u32,
        /// The kind of that node.
        kind: &'static str,
    },
    /// An input port the kind needs has no edge.
    MissingPort {
        /// The node missing the port.
        node: u32,
        /// The port name.
        port: &'static str,
    },
    /// An input names a node id that is not in the graph.
    UnknownNode {
        /// The id that was named.
        named: u32,
    },
    /// Two nodes share one id.
    DuplicateId {
        /// The repeated id.
        id: u32,
    },
    /// A node id is outside `1..=MAX_NODES`.
    IdOutOfRange {
        /// The id that was written.
        id: u32,
    },
    /// The graph holds more nodes than the budget.
    TooManyNodes {
        /// The budget.
        max: usize,
    },
    /// A node feeds itself, directly or through other nodes.
    Cycle {
        /// The node where the walk closed.
        node: u32,
    },
    /// A number is outside its bounds; bounds and value as text.
    OutOfRange {
        /// Smallest legal value.
        min: String,
        /// Largest legal value.
        max: String,
        /// The value as written.
        got: String,
    },
    /// A graph with no nodes or no output has nothing to fold.
    EmptyGraph,
    /// The graph asks for more gameplay layers than the record holds.
    LayerBudget {
        /// The budget.
        max: usize,
    },
    /// Two layers read the same coverage channel.
    ChannelBudget {
        /// The channel named twice.
        channel: &'static str,
    },
    /// A bake map came out flat on every channel.
    FlatMap {
        /// The map name.
        map: &'static str,
    },
    /// A bake resolution the bake does not offer.
    BadBakeSize {
        /// The size that was asked for.
        asked: u32,
    },
    /// The folded record refused a value; carries the record refusal.
    Record(RecordError),
}

/// A graph refusal: the JSON path, the typed kind, and one recovery step.
#[derive(Clone, PartialEq, Debug)]
pub struct GraphError {
    /// JSON path of the offending member.
    pub path: String,
    /// What was wrong.
    pub kind: GraphErrorKind,
}

impl GraphError {
    fn at(path: &str, kind: GraphErrorKind) -> Self {
        Self {
            path: path.to_owned(),
            kind,
        }
    }
}

impl fmt::Display for GraphErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "{error}"),
            Self::Expected(kind) => write!(f, "expected {kind}"),
            Self::Missing => write!(f, "missing"),
            Self::UnknownKey { legal } => {
                write!(f, "unknown key; the keys here are {}", legal.join(", "))
            }
            Self::UnknownKind { kind } => write!(
                f,
                "unknown node kind `{kind}`; the kinds are {}",
                KINDS.join(", ")
            ),
            Self::UnknownPort { node, kind } => write!(
                f,
                "node {node} is `{kind}` and does not take this port; next: read the \
                 kind's ports in the graph docs"
            ),
            Self::MissingPort { node, port } => write!(
                f,
                "node {node} has no `{port}` input; next: add an edge into `{port}`"
            ),
            Self::UnknownNode { named } => write!(
                f,
                "no node {named} in this graph; next: name a node id from `nodes`"
            ),
            Self::DuplicateId { id } => {
                write!(
                    f,
                    "node id {id} is used twice; next: give each node its own id"
                )
            }
            Self::IdOutOfRange { id } => write!(
                f,
                "node id {id} is outside 1..={MAX_NODES}; next: number the nodes 1..={MAX_NODES}"
            ),
            Self::TooManyNodes { max } => {
                write!(
                    f,
                    "more than {max} nodes; next: fold the graph into {max} or fewer"
                )
            }
            Self::Cycle { node } => write!(
                f,
                "node {node} feeds itself through its inputs; next: break the loop at {node}"
            ),
            Self::OutOfRange { min, max, got } => {
                write!(
                    f,
                    "{got} is outside {min}..={max}; next: write a value in range"
                )
            }
            Self::EmptyGraph => write!(
                f,
                "the graph has no nodes or no output block; next: add a node and name it \
                 in `output`"
            ),
            Self::LayerBudget { max } => {
                write!(f, "more than {max} layers; next: keep {max} or fewer")
            }
            Self::ChannelBudget { channel } => write!(
                f,
                "channel `{channel}` drives two layers; next: give each layer its own channel"
            ),
            Self::FlatMap { map } => write!(
                f,
                "map `{map}` is flat (min == max on every channel); next: give it a field \
                 that varies across the map"
            ),
            Self::BadBakeSize { asked } => write!(
                f,
                "bake size {asked} is not offered; the sizes are {}",
                BAKE_SIZES
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Record(error) => write!(f, "{error}"),
        }
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.kind)
    }
}

impl std::error::Error for GraphError {}

// ---------------------------------------------------------------------------
// Readers.
// ---------------------------------------------------------------------------

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        return key.to_owned();
    }
    format!("{path}.{key}")
}

fn object_at<'a>(value: &'a Value, path: &str) -> Result<&'a [(String, Value)], GraphError> {
    value
        .as_object()
        .ok_or_else(|| GraphError::at(path, GraphErrorKind::Expected("object")))
}

fn keys(value: &Value, path: &str, legal: &[&'static str]) -> Result<(), GraphError> {
    for (key, _) in object_at(value, path)? {
        if !legal.contains(&key.as_str()) {
            return Err(GraphError::at(
                &join(path, key),
                GraphErrorKind::UnknownKey {
                    legal: legal.to_vec(),
                },
            ));
        }
    }
    Ok(())
}

fn number_at(value: &Value, path: &str) -> Result<f32, GraphError> {
    let text = value
        .number_text()
        .ok_or_else(|| GraphError::at(path, GraphErrorKind::Expected("number")))?;
    text.parse::<f32>()
        .map_err(|_| GraphError::at(path, GraphErrorKind::Expected("number")))
}

fn ranged(value: &Value, path: &str, min: f32, max: f32) -> Result<f32, GraphError> {
    let got = number_at(value, path)?;
    if !got.is_finite() || got < min || got > max {
        return Err(GraphError::at(
            path,
            GraphErrorKind::OutOfRange {
                min: format!("{min}"),
                max: format!("{max}"),
                got: format!("{got}"),
            },
        ));
    }
    Ok(got)
}

fn uint_at(value: &Value, path: &str, min: u32, max: u32) -> Result<u32, GraphError> {
    let text = value
        .number_text()
        .ok_or_else(|| GraphError::at(path, GraphErrorKind::Expected("integer")))?;
    let got: u32 = text
        .parse()
        .map_err(|_| GraphError::at(path, GraphErrorKind::Expected("integer")))?;
    if got < min || got > max {
        return Err(GraphError::at(
            path,
            GraphErrorKind::OutOfRange {
                min: min.to_string(),
                max: max.to_string(),
                got: got.to_string(),
            },
        ));
    }
    Ok(got)
}

/// One node id written in `inputs` or `output`.
fn node_ref(value: &Value, path: &str) -> Result<u32, GraphError> {
    let id = uint_at(value, path, 1, u32::try_from(MAX_NODES).unwrap_or(u32::MAX))?;
    Ok(id)
}

fn rgb(value: &Value, path: &str) -> Result<[f32; 3], GraphError> {
    let items = value
        .as_array()
        .ok_or_else(|| GraphError::at(path, GraphErrorKind::Expected("array of 3 numbers")))?;
    if items.len() != 3 {
        return Err(GraphError::at(
            path,
            GraphErrorKind::Expected("array of 3 numbers"),
        ));
    }
    let mut out = [0.0f32; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = ranged(item, path, 0.0, 1.0)?;
    }
    Ok(out)
}

fn params_of(kind: NodeKind, value: Option<&Value>, path: &str) -> Result<Params, GraphError> {
    let mut params = Params::default();
    let Some(value) = value else {
        return Ok(params);
    };
    let legal = kind.param_keys();
    keys(value, path, legal)?;
    if let Some(v) = value.get("scale") {
        params.scale = uint_at(v, &join(path, "scale"), 1, 64)?;
    }
    if let Some(v) = value.get("seed") {
        params.seed = uint_at(v, &join(path, "seed"), 0, u32::MAX)?;
    }
    if let Some(v) = value.get("octaves") {
        params.octaves = uint_at(v, &join(path, "octaves"), 1, 5)?;
    }
    if let Some(v) = value.get("rows") {
        params.rows = uint_at(v, &join(path, "rows"), 1, 32)?;
    }
    if let Some(v) = value.get("cols") {
        params.cols = uint_at(v, &join(path, "cols"), 1, 32)?;
    }
    if let Some(v) = value.get("mortar") {
        params.mortar = ranged(v, &join(path, "mortar"), 0.0, 0.5)?;
    }
    if let Some(v) = value.get("axis") {
        let text = v.as_str().ok_or_else(|| {
            GraphError::at(&join(path, "axis"), GraphErrorKind::Expected("string"))
        })?;
        params.axis = Axis::from_name(text).ok_or_else(|| {
            GraphError::at(
                &join(path, "axis"),
                GraphErrorKind::UnknownKey {
                    legal: vec!["x", "y", "radial"],
                },
            )
        })?;
    }
    if let Some(v) = value.get("from") {
        params.from = ranged(v, &join(path, "from"), 0.0, 1.0)?;
    }
    if let Some(v) = value.get("to") {
        params.to = ranged(v, &join(path, "to"), 0.0, 1.0)?;
    }
    if let Some(v) = value.get("exponent") {
        params.exponent = uint_at(v, &join(path, "exponent"), 1, 8)?;
    }
    if let Some(v) = value.get("invert") {
        params.invert = v.as_bool().ok_or_else(|| {
            GraphError::at(&join(path, "invert"), GraphErrorKind::Expected("bool"))
        })?;
    }
    if let Some(v) = value.get("strength") {
        params.strength = ranged(v, &join(path, "strength"), 0.0, 1.0)?;
    }
    if let Some(v) = value.get("mode") {
        let text = v.as_str().ok_or_else(|| {
            GraphError::at(&join(path, "mode"), GraphErrorKind::Expected("string"))
        })?;
        params.mode = BlendMode::from_name(text).ok_or_else(|| {
            GraphError::at(
                &join(path, "mode"),
                GraphErrorKind::UnknownKey {
                    legal: vec!["add", "multiply", "mix", "overlay"],
                },
            )
        })?;
    }
    Ok(params)
}

fn node_of(value: &Value, path: &str) -> Result<Node, GraphError> {
    keys(value, path, &["id", "kind", "inputs", "params"])?;
    let id_value = value
        .get("id")
        .ok_or_else(|| GraphError::at(&join(path, "id"), GraphErrorKind::Missing))?;
    let id = uint_at(id_value, &join(path, "id"), 1, 64)?;
    if usize::try_from(id).unwrap_or(usize::MAX) > MAX_NODES {
        return Err(GraphError::at(
            &join(path, "id"),
            GraphErrorKind::IdOutOfRange { id },
        ));
    }
    let kind_text = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| GraphError::at(&join(path, "kind"), GraphErrorKind::Missing))?;
    let kind = NodeKind::from_name(kind_text)
        .ok_or_else(|| GraphError::at(&join(path, "kind"), GraphErrorKind::UnknownKind { kind: kind_text.to_owned() }))?;
    let mut inputs = [None; 3];
    if let Some(block) = value.get("inputs") {
        let block_path = join(path, "inputs");
        let legal: Vec<&'static str> = kind.ports().iter().map(|p| p.name()).collect();
        keys(block, &block_path, &legal)?;
        for (port, _) in object_at(block, &block_path)? {
            let port = Port::from_name(port).ok_or_else(|| {
                GraphError::at(
                    &block_path,
                    GraphErrorKind::UnknownPort {
                        node: id,
                        kind: kind.name(),
                    },
                )
            })?;
            let member = block
                .get(port.name())
                .ok_or_else(|| GraphError::at(&block_path, GraphErrorKind::Missing))?;
            let source = node_ref(member, &join(&block_path, port.name()))?;
            inputs[port.slot()] = Some(source);
        }
    }
    for port in kind.ports() {
        if inputs[port.slot()].is_none() {
            return Err(GraphError::at(
                &join(&join(path, "inputs"), port.name()),
                GraphErrorKind::MissingPort {
                    node: id,
                    port: port.name(),
                },
            ));
        }
    }
    let params = params_of(kind, value.get("params"), &join(path, "params"))?;
    Ok(Node {
        id,
        kind,
        inputs,
        params,
    })
}

fn output_of(value: &Value, path: &str) -> Result<Output, GraphError> {
    keys(
        value,
        path,
        &[
            "color",
            "color_low",
            "color_high",
            "roughness",
            "metalness",
            "height",
            "channels",
            "emissive",
        ],
    )?;
    let field = |key: &str| -> Result<u32, GraphError> {
        value
            .get(key)
            .ok_or_else(|| GraphError::at(&join(path, key), GraphErrorKind::Missing))
            .and_then(|v| node_ref(v, &join(path, key)))
    };
    let color_low = match value.get("color_low") {
        Some(v) => rgb(v, &join(path, "color_low"))?,
        None => [0.05, 0.05, 0.05],
    };
    let color_high = match value.get("color_high") {
        Some(v) => rgb(v, &join(path, "color_high"))?,
        None => [0.8, 0.8, 0.8],
    };
    let mut channels = BTreeMap::new();
    if let Some(block) = value.get("channels") {
        let block_path = join(path, "channels");
        for (key, member) in object_at(block, &block_path)? {
            let channel = Channel::from_name(key).ok_or_else(|| {
                GraphError::at(
                    &join(&block_path, key),
                    GraphErrorKind::UnknownKey {
                        legal: vec!["wetness", "dirt", "blood", "strain", "heat"],
                    },
                )
            })?;
            channels.insert(channel, node_ref(member, &join(&block_path, key))?);
        }
    }
    Ok(Output {
        color: field("color")?,
        color_low,
        color_high,
        roughness: field("roughness")?,
        metalness: field("metalness")?,
        height: field("height")?,
        channels,
        emissive: match value.get("emissive") {
            None | Some(Value::Null) => None,
            Some(v) => Some(node_ref(v, &join(path, "emissive"))?),
        },
    })
}

fn layers_of(value: &Value, path: &str) -> Result<Vec<Layer>, GraphError> {
    let items = value
        .as_array()
        .ok_or_else(|| GraphError::at(path, GraphErrorKind::Expected("array")))?;
    let mut out = Vec::new();
    let mut drivers: Vec<Channel> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        keys(
            item,
            &item_path,
            &[
                "kind",
                "driver",
                "strength",
                "tint",
                "darken",
                "roughness_to",
            ],
        )?;
        let text = |key: &str| -> Result<&str, GraphError> {
            item.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| GraphError::at(&join(&item_path, key), GraphErrorKind::Missing))
        };
        let kind = crate::record::LayerKind::from_name(text("kind")?).ok_or_else(|| {
            GraphError::at(
                &join(&item_path, "kind"),
                GraphErrorKind::UnknownKey {
                    legal: vec!["wet", "dirt", "blood", "damage", "burn"],
                },
            )
        })?;
        let driver = Channel::from_name(text("driver")?).ok_or_else(|| {
            GraphError::at(
                &join(&item_path, "driver"),
                GraphErrorKind::UnknownKey {
                    legal: vec!["wetness", "dirt", "blood", "strain", "heat"],
                },
            )
        })?;
        if drivers.contains(&driver) {
            return Err(GraphError::at(
                &join(&item_path, "driver"),
                GraphErrorKind::ChannelBudget {
                    channel: driver.name(),
                },
            ));
        }
        drivers.push(driver);
        let mut layer = Layer::new(kind, driver);
        if let Some(v) = item.get("strength") {
            layer.strength = ranged(v, &join(&item_path, "strength"), 0.0, 1.0)?;
        }
        if let Some(v) = item.get("tint") {
            layer.tint = Some(rgb(v, &join(&item_path, "tint"))?);
        }
        if let Some(v) = item.get("darken") {
            layer.darken = Some(ranged(v, &join(&item_path, "darken"), 0.0, 1.0)?);
        }
        if let Some(v) = item.get("roughness_to") {
            layer.roughness_to = Some(ranged(v, &join(&item_path, "roughness_to"), 0.0, 1.0)?);
        }
        out.push(layer);
    }
    if out.len() > crate::record::MAX_LAYERS {
        return Err(GraphError::at(
            path,
            GraphErrorKind::LayerBudget {
                max: crate::record::MAX_LAYERS,
            },
        ));
    }
    Ok(out)
}

impl Graph {
    /// Parse a graph document (at most [`MAX_GRAPH_BYTES`]). The graph must
    /// have 1..=[`MAX_NODES`] nodes with unique ids, every port wired, and
    /// no cycle.
    pub fn parse(text: &str) -> Result<Self, GraphError> {
        if text.len() > MAX_GRAPH_BYTES {
            return Err(GraphError::at(
                "",
                GraphErrorKind::OutOfRange {
                    min: "0".to_owned(),
                    max: MAX_GRAPH_BYTES.to_string(),
                    got: format!("{} bytes", text.len()),
                },
            ));
        }
        let doc = json::parse(text).map_err(|e| GraphError::at("", GraphErrorKind::Json(e)))?;
        keys(&doc, "", &["material", "base", "nodes", "output", "layers"])?;
        let material = doc
            .get("material")
            .and_then(Value::as_str)
            .ok_or_else(|| GraphError::at("material", GraphErrorKind::Missing))?
            .to_owned();
        let base = doc
            .get("base")
            .and_then(Value::as_str)
            .unwrap_or("stone")
            .to_owned();
        let node_items = doc
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| GraphError::at("nodes", GraphErrorKind::Missing))?;
        if node_items.is_empty() {
            return Err(GraphError::at("nodes", GraphErrorKind::EmptyGraph));
        }
        if node_items.len() > MAX_NODES {
            return Err(GraphError::at(
                "nodes",
                GraphErrorKind::TooManyNodes { max: MAX_NODES },
            ));
        }
        let mut nodes = Vec::with_capacity(node_items.len());
        for (index, item) in node_items.iter().enumerate() {
            let node = node_of(item, &format!("nodes[{index}]"))?;
            if nodes.iter().any(|seen: &Node| seen.id == node.id) {
                return Err(GraphError::at(
                    &format!("nodes[{index}].id"),
                    GraphErrorKind::DuplicateId { id: node.id },
                ));
            }
            nodes.push(node);
        }
        nodes.sort_by_key(|node| node.id);
        let output = doc
            .get("output")
            .ok_or_else(|| GraphError::at("output", GraphErrorKind::Missing))
            .and_then(|v| output_of(v, "output"))?;
        let layers = match doc.get("layers") {
            None => Vec::new(),
            Some(v) => layers_of(v, "layers")?,
        };
        let graph = Self {
            material,
            base,
            nodes,
            output,
            layers,
        };
        graph.check_refs()?;
        Ok(graph)
    }

    /// Every field a node or the output names exists, and no node feeds
    /// itself.
    fn check_refs(&self) -> Result<(), GraphError> {
        let ids: Vec<u32> = self.nodes.iter().map(|node| node.id).collect();
        let mut refs: Vec<(String, u32)> = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            for source in node.inputs.iter().flatten() {
                refs.push((format!("nodes[{index}].inputs"), *source));
            }
        }
        for (key, id) in [
            ("output.color", self.output.color),
            ("output.roughness", self.output.roughness),
            ("output.metalness", self.output.metalness),
            ("output.height", self.output.height),
        ] {
            refs.push((key.to_owned(), id));
        }
        for (channel, id) in &self.output.channels {
            refs.push((format!("output.channels.{}", channel.name()), *id));
        }
        if let Some(id) = self.output.emissive {
            refs.push(("output.emissive".to_owned(), id));
        }
        for (path, id) in refs {
            if !ids.contains(&id) {
                return Err(GraphError::at(
                    &path,
                    GraphErrorKind::UnknownNode { named: id },
                ));
            }
        }
        // A depth-first walk over ids; a node seen twice on one path is a
        // cycle, and the walk closes on the first node that repeats.
        let mut state = vec![0u8; self.nodes.len()];
        for start in 0..self.nodes.len() {
            self.walk(start, &mut state)?;
        }
        Ok(())
    }

    fn walk(&self, index: usize, state: &mut [u8]) -> Result<(), GraphError> {
        match state.get(index) {
            Some(2) => return Ok(()),
            Some(1) => {
                return Err(GraphError::at(
                    &format!("nodes[{index}]"),
                    GraphErrorKind::Cycle {
                        node: self.nodes.get(index).map_or(0, |node| node.id),
                    },
                ))
            }
            _ => {}
        }
        if let Some(slot) = state.get_mut(index) {
            *slot = 1;
        }
        let node = self.nodes.get(index).copied().ok_or_else(|| {
            GraphError::at("nodes", GraphErrorKind::UnknownNode { named: u32::MAX })
        })?;
        for source in node.inputs.iter().flatten() {
            if let Some(next) = self.nodes.iter().position(|n| n.id == *source) {
                self.walk(next, state)?;
            }
        }
        if let Some(slot) = state.get_mut(index) {
            *slot = 2;
        }
        Ok(())
    }

    /// Digest of the canonical graph text: the same graph written twice,
    /// in any node order, has one id.
    #[must_use]
    pub fn digest(&self) -> u64 {
        hash_bytes(self.canonical().as_bytes())
    }

    /// Canonical graph text: two-space JSON, nodes in id order, ending in
    /// one newline.
    #[must_use]
    pub fn canonical(&self) -> String {
        let mut out = json::write(&self.to_json());
        out.push('\n');
        out
    }

    /// The graph as a JSON value tree in canonical key order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let mut members = vec![
                    ("id".to_owned(), Value::Number(node.id.to_string())),
                    ("kind".to_owned(), Value::Str(node.kind.name().to_owned())),
                ];
                let mut inputs = Vec::new();
                for port in node.kind.ports() {
                    if let Some(source) = node.inputs[port.slot()] {
                        inputs.push((port.name().to_owned(), Value::Number(source.to_string())));
                    }
                }
                if !inputs.is_empty() {
                    members.push(("inputs".to_owned(), Value::Object(inputs)));
                }
                let params = params_json(node);
                if let Value::Object(members_of) = &params {
                    if !members_of.is_empty() {
                        members.push(("params".to_owned(), params));
                    }
                }
                Value::Object(members)
            })
            .collect();
        let mut output = vec![
            (
                "color".to_owned(),
                Value::Number(self.output.color.to_string()),
            ),
            (
                "color_low".to_owned(),
                Value::Array(self.output.color_low.iter().map(|v| decimal(*v)).collect()),
            ),
            (
                "color_high".to_owned(),
                Value::Array(self.output.color_high.iter().map(|v| decimal(*v)).collect()),
            ),
            (
                "roughness".to_owned(),
                Value::Number(self.output.roughness.to_string()),
            ),
            (
                "metalness".to_owned(),
                Value::Number(self.output.metalness.to_string()),
            ),
            (
                "height".to_owned(),
                Value::Number(self.output.height.to_string()),
            ),
        ];
        let channels: Vec<(String, Value)> = self
            .output
            .channels
            .iter()
            .map(|(channel, id)| (channel.name().to_owned(), Value::Number(id.to_string())))
            .collect();
        if !channels.is_empty() {
            output.push(("channels".to_owned(), Value::Object(channels)));
        }
        if let Some(id) = self.output.emissive {
            output.push(("emissive".to_owned(), Value::Number(id.to_string())));
        }
        let mut top = vec![
            ("material".to_owned(), Value::Str(self.material.clone())),
            ("base".to_owned(), Value::Str(self.base.clone())),
            ("nodes".to_owned(), Value::Array(nodes)),
            ("output".to_owned(), Value::Object(output)),
        ];
        if !self.layers.is_empty() {
            top.push((
                "layers".to_owned(),
                Value::Array(self.layers.iter().map(layer_value).collect()),
            ));
        }
        Value::Object(top)
    }

    /// Fold the graph into the record, extending `parent` (its body, sound,
    /// lobes and material name rule). The fold samples [`SAMPLE_GRID`] and
    /// reads the mean of every output field.
    pub fn to_record(&self, parent: &MaterialRecord) -> Result<MaterialRecord, GraphError> {
        let surface = self.sample(SAMPLE_GRID)?;
        let mut look = crate::record::Look {
            base_color: surface.base_color,
            base_metalness: surface.metalness,
            specular_roughness: surface.roughness,
            ..crate::record::Look::default()
        };
        let map = |slot: &str| format!("tex:{}_{slot}.png", self.material);
        look.textures = crate::record::Textures {
            base_color: Some(map("base_color")),
            orm: Some(map("orm")),
            normal: Some(map("normal")),
            emissive: Some(map("emissive")),
        };
        let mut layers = Vec::new();
        for layer in &self.layers {
            let mut copy = *layer;
            copy.strength = layer.strength * surface.coverage_of(layer.driver);
            layers.push(copy);
        }
        let mut top = vec![
            ("material".to_owned(), Value::Str(self.material.clone())),
            (
                "version".to_owned(),
                Value::Number(RECORD_VERSION.to_string()),
            ),
            ("extends".to_owned(), Value::Str(parent.name.clone())),
            ("look".to_owned(), self.look_json(&look)),
        ];
        if !layers.is_empty() {
            top.push((
                "layers".to_owned(),
                Value::Array(layers.iter().map(layer_value).collect()),
            ));
        }
        let text = json::write(&Value::Object(top));
        MaterialRecord::parse_with(&text, Some(parent)).map_err(|error| {
            let path = error.path.clone();
            GraphError::at(&path, GraphErrorKind::Record(error))
        })
    }

    fn look_json(&self, look: &crate::record::Look) -> Value {
        let color = Value::Array(look.base_color.iter().map(|v| decimal(*v)).collect());
        let textures = Value::Object(vec![
            (
                "base_color".to_owned(),
                Value::Str(look.textures.base_color.clone().unwrap_or_default()),
            ),
            (
                "orm".to_owned(),
                Value::Str(look.textures.orm.clone().unwrap_or_default()),
            ),
            (
                "normal".to_owned(),
                Value::Str(look.textures.normal.clone().unwrap_or_default()),
            ),
            (
                "emissive".to_owned(),
                Value::Str(look.textures.emissive.clone().unwrap_or_default()),
            ),
        ]);
        Value::Object(vec![
            ("base_color".to_owned(), color),
            ("base_metalness".to_owned(), decimal(look.base_metalness)),
            (
                "specular_roughness".to_owned(),
                decimal(look.specular_roughness),
            ),
            ("normal_scale".to_owned(), decimal(look.normal_scale)),
            ("textures".to_owned(), textures),
        ])
    }

    /// Sample every output field on a `size` square grid.
    pub fn sample(&self, size: u32) -> Result<Surface, GraphError> {
        if size == 0 {
            return Err(GraphError::at(
                "",
                GraphErrorKind::OutOfRange {
                    min: "1".to_owned(),
                    max: u32::MAX.to_string(),
                    got: "0".to_owned(),
                },
            ));
        }
        let mut memo: Vec<Option<Field>> = self.nodes.iter().map(|_| None).collect();
        let mut grid = |id: u32| -> Result<Field, GraphError> {
            let index = self
                .nodes
                .iter()
                .position(|node| node.id == id)
                .ok_or_else(|| {
                    GraphError::at("output", GraphErrorKind::UnknownNode { named: id })
                })?;
            self.eval(index, size, &mut memo)
        };
        let color = grid(self.output.color)?;
        let roughness = grid(self.output.roughness)?;
        let metalness = grid(self.output.metalness)?;
        let height = grid(self.output.height)?;
        let mut channels: [Field; 5] = std::array::from_fn(|_| Field::flat(size, 0.0));
        for (channel, id) in &self.output.channels {
            let field = grid(*id)?;
            if let Some(slot) = Channel::all().iter().position(|c| c == channel) {
                channels[slot] = field;
            }
        }
        let emissive = match self.output.emissive {
            Some(id) => grid(id)?,
            None => Field::flat(size, 0.0),
        };
        let mut base_color = [0.0f32; 3];
        let low = self.output.color_low;
        let high = self.output.color_high;
        for value in &color.values {
            let t = value.clamp(0.0, 1.0);
            for channel in 0..3 {
                let a = low.get(channel).copied().unwrap_or(0.0);
                let b = high.get(channel).copied().unwrap_or(1.0);
                if let Some(slot) = base_color.get_mut(channel) {
                    *slot += a + (b - a) * t;
                }
            }
        }
        let count = count_f32(color.values.len());
        for slot in &mut base_color {
            *slot = (*slot / count).clamp(0.0, 1.0);
        }
        let mut channel_mean = [0.0f32; 5];
        for (slot, field) in channel_mean.iter_mut().zip(channels.iter()) {
            *slot = field.mean();
        }
        Ok(Surface {
            base_color,
            roughness: roughness.mean(),
            metalness: metalness.mean(),
            coverage: channel_mean,
            coverage_fields: channels,
            height,
            emissive,
            size,
        })
    }

    fn eval(
        &self,
        index: usize,
        size: u32,
        memo: &mut Vec<Option<Field>>,
    ) -> Result<Field, GraphError> {
        if let Some(field) = memo.get(index).and_then(Option::as_ref) {
            return Ok(field.clone());
        }
        let node = self.nodes.get(index).copied().ok_or_else(|| {
            GraphError::at("nodes", GraphErrorKind::UnknownNode { named: u32::MAX })
        })?;
        let mut port = |slot: usize| -> Result<Field, GraphError> {
            let id = node.inputs.get(slot).copied().flatten().ok_or_else(|| {
                GraphError::at(
                    "nodes",
                    GraphErrorKind::MissingPort {
                        node: node.id,
                        port: match slot {
                            0 => "a",
                            2 => "mask",
                            _ => "b",
                        },
                    },
                )
            })?;
            let next = self.nodes.iter().position(|n| n.id == id).ok_or_else(|| {
                GraphError::at("nodes", GraphErrorKind::UnknownNode { named: id })
            })?;
            self.eval(next, size, memo)
        };
        let a = if node.kind.ports().contains(&Port::A) {
            Some(port(0)?)
        } else {
            None
        };
        let b = if node.kind.ports().contains(&Port::B) {
            Some(port(1)?)
        } else {
            None
        };
        let mask = if node.kind.ports().contains(&Port::Mask) {
            Some(port(2)?)
        } else {
            None
        };
        let field = compute(node, size, a.as_ref(), b.as_ref(), mask.as_ref());
        if let Some(slot) = memo.get_mut(index) {
            *slot = Some(field.clone());
        }
        Ok(field)
    }

    /// Bake four maps at `resolution` (one of [`BAKE_SIZES`]). A map whose
    /// every channel is constant is refused ([`GraphErrorKind::FlatMap`]).
    pub fn bake(&self, resolution: u32) -> Result<Baked, GraphError> {
        if !BAKE_SIZES.contains(&resolution) {
            return Err(GraphError::at(
                "resolution",
                GraphErrorKind::BadBakeSize { asked: resolution },
            ));
        }
        let surface = self.sample(resolution)?;
        let raw = self.rasterize(&surface, resolution);
        let mut maps = Vec::with_capacity(raw.len());
        for (name, rgba) in raw {
            maps.push(baked_map(name, resolution, rgba)?);
        }
        Ok(Baked {
            maps,
            graph: self.digest(),
        })
    }

    /// Bake the five coverage channel maps and the two packed sheets at
    /// `resolution` (one of [`BAKE_SIZES`]). A flat channel is named in
    /// [`ChannelBake::flat`] instead of refused: a graph may drive a channel
    /// flat on purpose.
    pub fn bake_coverage(&self, resolution: u32) -> Result<ChannelBake, GraphError> {
        if !BAKE_SIZES.contains(&resolution) {
            return Err(GraphError::at(
                "resolution",
                GraphErrorKind::BadBakeSize { asked: resolution },
            ));
        }
        let surface = self.sample(resolution)?;
        Ok(channel_maps::channel_bake(&surface, resolution))
    }

    /// The four RGBA8 sample runs of a baked surface, in [`BAKE_MAPS`] order.
    fn rasterize(&self, surface: &Surface, resolution: u32) -> [(&'static str, Vec<u8>); 4] {
        let count = usize::try_from(resolution)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(resolution).unwrap_or(0));
        let mut base_color = vec![0u8; count.saturating_mul(4)];
        let mut orm = vec![0u8; count.saturating_mul(4)];
        let mut normal = vec![0u8; count.saturating_mul(4)];
        let mut emissive = vec![0u8; count.saturating_mul(4)];
        let low = self.output.color_low;
        let high = self.output.color_high;
        for index in 0..count {
            let height = surface.height.at(index);
            let base = base_color.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4);
            if let Some(pixel) = base {
                for (channel, slot) in pixel.iter_mut().take(3).enumerate() {
                    let a = low.get(channel).copied().unwrap_or(0.0);
                    let b = high.get(channel).copied().unwrap_or(1.0);
                    *slot = to_u8(a + (b - a) * height);
                }
                if let Some(alpha) = pixel.get_mut(3) {
                    *alpha = 255;
                }
            }
            let entry = orm.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4);
            if let Some(pixel) = entry {
                pixel[0] = to_u8(1.0 - height);
                pixel[1] = to_u8(surface.roughness);
                pixel[2] = to_u8(surface.metalness);
                pixel[3] = 255;
            }
            let (dx, dy) = surface.height.slope(index, resolution);
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            let entry = normal.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4);
            if let Some(pixel) = entry {
                pixel[0] = to_u8(0.5 - 0.5 * dx / len);
                pixel[1] = to_u8(0.5 - 0.5 * dy / len);
                pixel[2] = to_u8(0.5 + 0.5 / len);
                pixel[3] = 255;
            }
            let level = surface.emissive.at(index);
            let entry = emissive.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4);
            if let Some(pixel) = entry {
                for slot in pixel.iter_mut().take(3) {
                    *slot = to_u8(level);
                }
                if let Some(alpha) = pixel.get_mut(3) {
                    *alpha = 255;
                }
            }
        }
        [
            ("base_color", base_color),
            ("orm", orm),
            ("normal", normal),
            ("emissive", emissive),
        ]
    }
}

/// One baked map: its PNG bytes, digest, per-channel range. A map whose
/// every channel is constant is refused.
fn baked_map(name: &'static str, resolution: u32, rgba: Vec<u8>) -> Result<BakeMap, GraphError> {
    let (min, max) = eval::range(&rgba);
    let flat = (0..4)
        .all(|slot| min.get(slot).copied().unwrap_or(1.0) == max.get(slot).copied().unwrap_or(0.0));
    if flat {
        return Err(GraphError::at(
            "output",
            GraphErrorKind::FlatMap { map: name },
        ));
    }
    Ok(BakeMap {
        name,
        width: resolution,
        height: resolution,
        png: crate::png::encode_rgba(resolution, resolution, &rgba),
        digest: hash_bytes(&rgba),
        min,
        max,
        rgba,
    })
}

mod channel_maps;
mod eval;

pub use channel_maps::{ChannelBake, COVERAGE_MAPS, COVERAGE_SHEET_MAPS};
pub use eval::{BakeMap, Baked, Field, Surface};

#[cfg(test)]
mod tests {
    use super::*;

    const BRICK: &str = r#"{
      "material": "brick_wall",
      "base": "stone",
      "nodes": [
        {"id": 3, "kind": "noise", "params": {"scale": 24, "seed": 1}},
        {"id": 1, "kind": "gradient", "params": {"axis": "y", "from": 0.2, "to": 1.0}},
        {"id": 2, "kind": "tile", "inputs": {"a": 1}, "params": {"rows": 8, "cols": 3, "mortar": 0.08}},
        {"id": 4, "kind": "blend", "inputs": {"a": 2, "b": 3, "mask": 3}, "params": {"mode": "multiply"}},
        {"id": 5, "kind": "curve", "inputs": {"a": 2}, "params": {"exponent": 2}}
      ],
      "output": {
        "color": 2, "color_low": [0.45, 0.33, 0.28], "color_high": [0.72, 0.6, 0.48],
        "roughness": 4, "metalness": 3, "height": 2,
        "channels": {"wetness": 3, "dirt": 2, "heat": 5},
        "emissive": 5
      },
      "layers": [{"kind": "wet", "driver": "wetness", "strength": 0.6}]
    }"#;

    fn brick() -> Graph {
        Graph::parse(BRICK).expect("the brick graph parses")
    }

    #[test]
    fn the_brick_graph_parses_into_id_order() {
        let graph = brick();
        let ids: Vec<u32> = graph.nodes.iter().map(|node| node.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        assert_eq!(graph.base, "stone");
        assert_eq!(graph.output.channels.len(), 3);
    }

    #[test]
    fn the_same_graph_twice_has_one_digest_and_one_canonical_text() {
        let first = brick();
        let second = Graph::parse(&first.canonical()).expect("canonical text reparses");
        assert_eq!(first.canonical(), second.canonical());
        assert_eq!(first.digest(), second.digest());
        // Node order in the text never moves the digest.
        let reordered = BRICK.replace(
            r#"{"id": 3, "kind": "noise", "params": {"scale": 24, "seed": 1}},"#,
            "",
        );
        let reordered = reordered.replace(
            r#""nodes": ["#,
            r#""nodes": [{"id": 3, "kind": "noise", "params": {"scale": 24, "seed": 1}},"#,
        );
        let reordered = Graph::parse(&reordered).expect("reordered parses");
        assert_eq!(reordered.digest(), first.digest());
    }

    #[test]
    fn folds_into_the_record_with_four_texture_refs_and_one_layer() {
        let graph = brick();
        let kit = crate::MaterialSet::siege_kit();
        let stone = kit.get("stone").expect("stone");
        let record = graph.to_record(stone).expect("folds");
        assert_eq!(record.name, "brick_wall");
        assert_eq!(record.extends.as_deref(), Some("stone"));
        assert_eq!(
            record.body, stone.body,
            "the sim half comes from the parent"
        );
        assert_eq!(record.layers.len(), 1);
        assert!(record.look.textures.base_color.is_some());
        assert!(record.look.textures.orm.is_some());
        assert!(record.look.textures.normal.is_some());
        assert!(record.look.textures.emissive.is_some());
        assert!(record.look.specular_roughness > 0.0);
        // Folding twice gives one record digest.
        let again = graph.to_record(stone).expect("folds twice");
        assert_eq!(record.look_digest(), again.look_digest());
        assert_eq!(record.to_text(), again.to_text());
    }

    #[test]
    fn a_cycle_missing_port_unknown_kind_and_too_many_nodes_are_typed() {
        let cyc = BRICK.replace(
            r#"{"id": 2, "kind": "tile", "inputs": {"a": 1}"#,
            r#"{"id": 2, "kind": "tile", "inputs": {"a": 4}"#,
        );
        let err = Graph::parse(&cyc).expect_err("a cycle is refused");
        assert!(
            matches!(err.kind, GraphErrorKind::Cycle { .. }),
            "{:?}",
            err.kind
        );
        let missing = BRICK.replace(
            r#""inputs": {"a": 2, "b": 3, "mask": 3}"#,
            r#""inputs": {"a": 2, "b": 3}"#,
        );
        let err = Graph::parse(&missing).expect_err("a missing port is refused");
        assert!(
            matches!(err.kind, GraphErrorKind::MissingPort { port: "mask", .. }),
            "{:?}",
            err.kind
        );
        let unknown = BRICK.replace(r#""kind": "tile""#, r#""kind": "voronoi""#);
        let err = Graph::parse(&unknown).expect_err("an unknown kind is refused");
        assert!(matches!(err.kind, GraphErrorKind::UnknownKind { .. }));
        assert!(err
            .to_string()
            .contains("noise, tile, gradient, curve, warp, blend"));
        let mut nodes = String::new();
        for id in 1..=65 {
            if id > 1 {
                nodes.push(',');
            }
            nodes.push_str(&format!(r#"{{"id": {id}, "kind": "gradient"}}"#));
        }
        let many = format!(
            r#"{{"material": "x", "nodes": [{nodes}], "output": {{"color": 1, "roughness": 1, "metalness": 1, "height": 1}}}}"#
        );
        let err = Graph::parse(&many).expect_err("the node budget is refused");
        assert!(matches!(
            err.kind,
            GraphErrorKind::TooManyNodes { max: MAX_NODES }
        ));
        assert!(err.to_string().contains("next:"));
    }

    #[test]
    fn bake_writes_four_maps_and_the_same_graph_twice_is_byte_equal() {
        let graph = brick();
        let first = graph.bake(512).expect("bakes");
        assert_eq!(first.maps.len(), 4);
        let names: Vec<&str> = first.maps.iter().map(|map| map.name).collect();
        assert_eq!(names, BAKE_MAPS);
        for map in &first.maps {
            assert_eq!(map.png.len().min(8), 8);
            assert_eq!(&map.png[1..4], b"PNG");
            assert!(map.max.iter().zip(&map.min).any(|(high, low)| high > low));
        }
        let second = graph.bake(512).expect("bakes twice");
        for (a, b) in first.maps.iter().zip(&second.maps) {
            assert_eq!(a.digest, b.digest, "{} digest", a.name);
            assert_eq!(a.png, b.png, "{} bytes", a.name);
        }
        assert_eq!(first.graph, second.graph, "one graph digest");
    }

    #[test]
    fn a_flat_map_is_a_typed_error() {
        let text = r#"{"material": "flat", "nodes": [{"id": 1, "kind": "gradient", "params": {"from": 0.5, "to": 0.5}}, {"id": 2, "kind": "gradient", "params": {"from": 0.5, "to": 0.5}}],
          "output": {"color": 1, "roughness": 2, "metalness": 2, "height": 2}}"#;
        let graph = Graph::parse(text).expect("parses");
        let err = graph.bake(512).expect_err("a flat bake is refused");
        assert!(
            matches!(err.kind, GraphErrorKind::FlatMap { .. }),
            "{:?}",
            err.kind
        );
        assert!(err.to_string().contains("next:"));
    }

    #[test]
    fn a_bake_size_off_the_list_is_refused() {
        let err = brick().bake(256).expect_err("256 is not offered");
        assert!(matches!(
            err.kind,
            GraphErrorKind::BadBakeSize { asked: 256 }
        ));
    }
}
