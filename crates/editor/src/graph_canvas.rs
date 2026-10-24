//! Material graph canvas (E7): node positions, typed ports, validated links.
//!
//! Owner: the material graph tab. Nodes sit on an integer pixel canvas, every
//! port carries a [`PortKind`], and a link is refused unless the kinds match,
//! the input is free and the link keeps the graph acyclic. Evaluation stays in
//! the material pack; this module only holds what the canvas draws and edits.
//! Units: positions and pan are integer pixels, zoom is thousandths of 1.0.
//! Determinism: [`Canvas::layout_digest`] hashes nodes ordered by id, so two
//! canvases with the same nodes and positions agree.
//! Errors: [`CanvasError`]; a refused edit leaves the canvas untouched.

use engine_core::hash_bytes;
use engine_material::graph::{Output, Params};

/// Most nodes one canvas holds.
pub const MAX_NODES: usize = 64;
/// Zoom floor: 0.25x.
pub const ZOOM_MIN_MILLI: i32 = 250;
/// Zoom ceiling: 4x.
pub const ZOOM_MAX_MILLI: i32 = 4_000;
/// Pan bound in pixels on either axis.
pub const PAN_LIMIT_PX: i32 = 200_000;

/// What a port carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortKind {
    /// RGB color.
    Color,
    /// One scalar.
    Scalar,
    /// Surface normal.
    Normal,
    /// Black-and-white mask.
    Mask,
}

impl PortKind {
    /// Short tag for panel rows and the digest.
    pub fn label(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Scalar => "scalar",
            Self::Normal => "normal",
            Self::Mask => "mask",
        }
    }

    /// One byte the layout digest writes for this kind.
    fn code(self) -> u8 {
        match self {
            Self::Color => 0,
            Self::Scalar => 1,
            Self::Normal => 2,
            Self::Mask => 3,
        }
    }
}

/// One canvas node: a stable id, a pixel position, its typed ports and the
/// material node it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasNode {
    /// Caller id; unique on the canvas.
    pub id: u32,
    /// Left edge in canvas pixels.
    pub x: i32,
    /// Top edge in canvas pixels.
    pub y: i32,
    /// Material node kind name (`engine_material::graph::KINDS`), e.g.
    /// `noise`. A name the pack does not know refuses at export.
    pub kind: String,
    /// The parameters the kind reads; defaults when untouched.
    pub params: Params,
    /// Input port kinds, in port order.
    pub inputs: Vec<PortKind>,
    /// Output port kinds, in port order.
    pub outputs: Vec<PortKind>,
}

/// One link from an output port to an input port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    /// Source node id.
    pub from: u32,
    /// Source output port index.
    pub from_port: u8,
    /// Target node id.
    pub to: u32,
    /// Target input port index.
    pub to_port: u8,
}

/// A bounded node canvas with validated links.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    nodes: Vec<CanvasNode>,
    links: Vec<Link>,
    /// Which node drives each material output field; `None` until the host
    /// binds one, and export refuses until then.
    output: Option<Output>,
}

impl Canvas {
    /// Empty canvas.
    pub fn new() -> Self {
        Self::default()
    }

    /// Nodes in insertion order.
    pub fn nodes(&self) -> &[CanvasNode] {
        &self.nodes
    }

    /// Links in insertion order.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Which node drives each material output field.
    pub fn material_output(&self) -> Option<Output> {
        self.output.clone()
    }

    /// Bind the material output block (the host's choice of fields); export
    /// refuses while it is unset.
    pub fn set_material_output(&mut self, output: Output) {
        self.output = Some(output);
    }

    /// Add one node. A duplicate id, the [`MAX_NODES`] bound or a position
    /// outside [`PAN_LIMIT_PX`] refuses without change.
    pub fn add_node(&mut self, node: CanvasNode) -> Result<(), CanvasError> {
        if self.nodes.len() >= MAX_NODES {
            return Err(CanvasError::Full {
                len: self.nodes.len(),
                max: MAX_NODES,
            });
        }
        if self.nodes.iter().any(|held| held.id == node.id) {
            return Err(CanvasError::DuplicateNode { id: node.id });
        }
        if node.x.abs() > PAN_LIMIT_PX || node.y.abs() > PAN_LIMIT_PX {
            return Err(CanvasError::PositionOutOfRange {
                id: node.id,
                x: node.x,
                y: node.y,
            });
        }
        self.nodes.push(node);
        Ok(())
    }

    /// Move one node to a pixel position, bounded like [`Canvas::add_node`].
    pub fn move_node(&mut self, id: u32, x: i32, y: i32) -> Result<(), CanvasError> {
        if x.abs() > PAN_LIMIT_PX || y.abs() > PAN_LIMIT_PX {
            return Err(CanvasError::PositionOutOfRange { id, x, y });
        }
        let node = self
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .ok_or(CanvasError::UnknownNode { id })?;
        node.x = x;
        node.y = y;
        Ok(())
    }

    /// Connect one output port to one input port.
    ///
    /// Refusals: unknown node, missing port, kind mismatch, an input that
    /// already has a link, and a link that would close a cycle. Nothing is
    /// added on error.
    pub fn connect(&mut self, link: Link) -> Result<(), CanvasError> {
        let from = self
            .nodes
            .iter()
            .find(|node| node.id == link.from)
            .ok_or(CanvasError::UnknownNode { id: link.from })?;
        let to = self
            .nodes
            .iter()
            .find(|node| node.id == link.to)
            .ok_or(CanvasError::UnknownNode { id: link.to })?;
        let out = from
            .outputs
            .get(usize::from(link.from_port))
            .copied()
            .ok_or(CanvasError::PortMissing {
                id: link.from,
                port: link.from_port,
                input: false,
            })?;
        let inp =
            to.inputs
                .get(usize::from(link.to_port))
                .copied()
                .ok_or(CanvasError::PortMissing {
                    id: link.to,
                    port: link.to_port,
                    input: true,
                })?;
        if out != inp {
            return Err(CanvasError::KindMismatch {
                from: link.from,
                to: link.to,
                out,
                inp,
            });
        }
        if link.from == link.to {
            return Err(CanvasError::Cycle {
                from: link.from,
                to: link.to,
            });
        }
        if self
            .links
            .iter()
            .any(|held| held.to == link.to && held.to_port == link.to_port)
        {
            return Err(CanvasError::InputTaken {
                to: link.to,
                port: link.to_port,
            });
        }
        if self.reaches(link.to, link.from) {
            return Err(CanvasError::Cycle {
                from: link.from,
                to: link.to,
            });
        }
        self.links.push(link);
        Ok(())
    }

    /// True when `target` is reachable from `start` over existing links.
    fn reaches(&self, start: u32, target: u32) -> bool {
        let mut stack = vec![start];
        let mut seen: Vec<u32> = Vec::new();
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            for link in self.links.iter().filter(|link| link.from == id) {
                stack.push(link.to);
            }
        }
        false
    }

    /// Stable digest over node ids and positions, ordered by id.
    pub fn layout_digest(&self) -> u64 {
        let mut nodes: Vec<&CanvasNode> = self.nodes.iter().collect();
        nodes.sort_by_key(|node| node.id);
        let mut bytes = Vec::new();
        for node in nodes {
            bytes.extend_from_slice(&node.id.to_le_bytes());
            bytes.extend_from_slice(&node.x.to_le_bytes());
            bytes.extend_from_slice(&node.y.to_le_bytes());
            for kind in node.inputs.iter().chain(node.outputs.iter()) {
                bytes.push(kind.code());
            }
        }
        hash_bytes(&bytes)
    }
}

/// What refused a canvas edit. Nothing changed on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasError {
    /// The canvas already holds [`MAX_NODES`] nodes.
    Full {
        /// Nodes held now.
        len: usize,
        /// The bound.
        max: usize,
    },
    /// Two nodes share an id.
    DuplicateNode {
        /// Repeated id.
        id: u32,
    },
    /// No node carries that id.
    UnknownNode {
        /// Id asked for.
        id: u32,
    },
    /// The node has no port at that index.
    PortMissing {
        /// Node id.
        id: u32,
        /// Port index.
        port: u8,
        /// True for an input port.
        input: bool,
    },
    /// Output and input carry different kinds.
    KindMismatch {
        /// Source id.
        from: u32,
        /// Target id.
        to: u32,
        /// Output kind.
        out: PortKind,
        /// Input kind.
        inp: PortKind,
    },
    /// The input port already has a link.
    InputTaken {
        /// Target id.
        to: u32,
        /// Target port.
        port: u8,
    },
    /// The link would close a cycle.
    Cycle {
        /// Source id.
        from: u32,
        /// Target id.
        to: u32,
    },
    /// A position left [`PAN_LIMIT_PX`].
    PositionOutOfRange {
        /// Node id.
        id: u32,
        /// X asked for.
        x: i32,
        /// Y asked for.
        y: i32,
    },
    /// The node's kind has no material node.
    UnmappedNode {
        /// Node id.
        id: u32,
        /// Kind name the node carries.
        kind: String,
        /// The legal kinds, comma separated.
        hint: String,
    },
    /// The canvas has no material output block yet.
    NoOutput,
    /// The node input has no pack port (a kind that reads nothing there, or a
    /// mask in a field slot).
    UnmappedPort {
        /// Node id.
        id: u32,
        /// Input port index.
        port: u8,
        /// What the pack reads there.
        hint: String,
    },
    /// The material pack refused the exported text or the bake.
    Material {
        /// The pack's own message.
        text: String,
    },
}

impl std::fmt::Display for CanvasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full { len, max } => write!(
                f,
                "canvas holds {len} of {max} nodes; delete one before adding"
            ),
            Self::DuplicateNode { id } => {
                write!(f, "node {id} already exists; pick a new id")
            }
            Self::UnknownNode { id } => write!(f, "no node {id}; add it first"),
            Self::PortMissing { id, port, input } => write!(
                f,
                "node {id} has no {} port {port}; pick a listed port",
                if *input { "input" } else { "output" }
            ),
            Self::KindMismatch { from, to, out, inp } => write!(
                f,
                "node {from} emits {} but node {to} reads {}; link matching kinds",
                out.label(),
                inp.label()
            ),
            Self::InputTaken { to, port } => write!(
                f,
                "node {to} input {port} already has a link; disconnect it first"
            ),
            Self::Cycle { from, to } => write!(
                f,
                "link {from} -> {to} closes a cycle; remove the reverse path"
            ),
            Self::PositionOutOfRange { id, x, y } => write!(
                f,
                "node {id} at ({x}, {y}) leaves the {PAN_LIMIT_PX} px canvas; clamp the position"
            ),
            Self::UnmappedNode { id, kind, hint } => write!(
                f,
                "node {id} kind {kind} has no material node; pick one of {hint}"
            ),
            Self::NoOutput => {
                write!(
                    f,
                    "canvas has no material output; bind the color field first"
                )
            }
            Self::UnmappedPort { id, port, hint } => {
                write!(f, "node {id} input {port} has no material port; {hint}")
            }
            Self::Material { text } => write!(f, "material refused: {text}"),
        }
    }
}

impl std::error::Error for CanvasError {}

/// Pan and zoom of one canvas viewport, in integer units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasView {
    /// Horizontal pan in pixels.
    pub pan_x: i32,
    /// Vertical pan in pixels.
    pub pan_y: i32,
    /// Zoom in thousandths of 1.0.
    pub zoom_milli: i32,
}

impl CanvasView {
    /// Identity view: no pan, 1x zoom.
    pub fn new() -> Self {
        Self {
            pan_x: 0,
            pan_y: 0,
            zoom_milli: 1_000,
        }
    }

    /// Pan by a pixel delta, clamped to [`PAN_LIMIT_PX`] on both axes.
    pub fn pan_by(&mut self, dx: i32, dy: i32) {
        self.pan_x = (self.pan_x.saturating_add(dx)).clamp(-PAN_LIMIT_PX, PAN_LIMIT_PX);
        self.pan_y = (self.pan_y.saturating_add(dy)).clamp(-PAN_LIMIT_PX, PAN_LIMIT_PX);
    }

    /// Zoom by a thousandth delta, clamped to [`ZOOM_MIN_MILLI`] and
    /// [`ZOOM_MAX_MILLI`].
    pub fn zoom_by(&mut self, delta_milli: i32) {
        self.zoom_milli =
            (self.zoom_milli.saturating_add(delta_milli)).clamp(ZOOM_MIN_MILLI, ZOOM_MAX_MILLI);
    }
}

impl Default for CanvasView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, inputs: Vec<PortKind>, outputs: Vec<PortKind>) -> CanvasNode {
        CanvasNode {
            id,
            x: 0,
            y: 0,
            kind: "noise".to_owned(),
            params: Params::default(),
            inputs,
            outputs,
        }
    }

    fn canvas() -> Canvas {
        let mut canvas = Canvas::new();
        canvas
            .add_node(node(1, vec![], vec![PortKind::Color]))
            .unwrap();
        canvas
            .add_node(node(2, vec![PortKind::Color], vec![PortKind::Color]))
            .unwrap();
        canvas
            .add_node(node(3, vec![PortKind::Color], vec![]))
            .unwrap();
        canvas
    }

    #[test]
    fn kinds_and_free_inputs_are_enforced() {
        let mut canvas = canvas();
        let link = Link {
            from: 1,
            from_port: 0,
            to: 2,
            to_port: 0,
        };
        canvas.connect(link).unwrap();
        // One input per port: the same input refuses a second link.
        assert_eq!(
            canvas.connect(Link {
                from: 1,
                from_port: 0,
                to: 2,
                to_port: 0
            }),
            Err(CanvasError::InputTaken { to: 2, port: 0 })
        );
        // Kind mismatch: a color output cannot feed a scalar input.
        let mut scalar = Canvas::new();
        scalar
            .add_node(node(1, vec![], vec![PortKind::Color]))
            .unwrap();
        scalar
            .add_node(node(2, vec![PortKind::Scalar], vec![]))
            .unwrap();
        assert_eq!(
            scalar.connect(Link {
                from: 1,
                from_port: 0,
                to: 2,
                to_port: 0
            }),
            Err(CanvasError::KindMismatch {
                from: 1,
                to: 2,
                out: PortKind::Color,
                inp: PortKind::Scalar
            })
        );
        // Missing port and unknown node.
        assert_eq!(
            canvas.connect(Link {
                from: 1,
                from_port: 9,
                to: 2,
                to_port: 0
            }),
            Err(CanvasError::PortMissing {
                id: 1,
                port: 9,
                input: false
            })
        );
        assert_eq!(
            canvas.connect(Link {
                from: 7,
                from_port: 0,
                to: 2,
                to_port: 0
            }),
            Err(CanvasError::UnknownNode { id: 7 })
        );
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut canvas = canvas();
        canvas
            .connect(Link {
                from: 1,
                from_port: 0,
                to: 2,
                to_port: 0,
            })
            .unwrap();
        canvas
            .connect(Link {
                from: 2,
                from_port: 0,
                to: 3,
                to_port: 0,
            })
            .unwrap();
        // 3 -> 2 would close 1 -> 2 -> 3 -> 2. Node 3 has no input, so give it
        // one first to reach the cycle check rather than a missing port.
        let mut cycled = Canvas::new();
        cycled
            .add_node(node(1, vec![PortKind::Color], vec![PortKind::Color]))
            .unwrap();
        cycled
            .add_node(node(2, vec![PortKind::Color], vec![PortKind::Color]))
            .unwrap();
        cycled
            .connect(Link {
                from: 1,
                from_port: 0,
                to: 2,
                to_port: 0,
            })
            .unwrap();
        assert_eq!(
            cycled.connect(Link {
                from: 2,
                from_port: 0,
                to: 1,
                to_port: 0
            }),
            Err(CanvasError::Cycle { from: 2, to: 1 })
        );
        // A self-link is a cycle too.
        assert_eq!(
            cycled.connect(Link {
                from: 2,
                from_port: 0,
                to: 2,
                to_port: 0
            }),
            Err(CanvasError::Cycle { from: 2, to: 2 })
        );
        assert_eq!(cycled.links().len(), 1, "refusals add no link");
    }

    #[test]
    fn two_layouts_with_the_same_nodes_agree() {
        let mut a = canvas();
        let mut b = Canvas::new();
        b.add_node(node(3, vec![PortKind::Color], vec![])).unwrap();
        b.add_node(node(1, vec![], vec![PortKind::Color])).unwrap();
        b.add_node(node(2, vec![PortKind::Color], vec![PortKind::Color]))
            .unwrap();
        assert_eq!(a.layout_digest(), b.layout_digest());
        a.move_node(1, 40, 60).unwrap();
        assert_ne!(a.layout_digest(), b.layout_digest());
        // Bounds refuse: the digest stays.
        let digest = a.layout_digest();
        assert!(a.move_node(1, PAN_LIMIT_PX + 1, 0).is_err());
        assert_eq!(a.layout_digest(), digest);
    }

    #[test]
    fn the_node_bound_and_zoom_clamp_hold() {
        let mut canvas = Canvas::new();
        for id in 0..MAX_NODES as u32 {
            canvas.add_node(node(id, vec![], vec![])).unwrap();
        }
        assert_eq!(
            canvas.add_node(node(999, vec![], vec![])),
            Err(CanvasError::Full {
                len: MAX_NODES,
                max: MAX_NODES
            })
        );
        let mut view = CanvasView::new();
        view.zoom_by(10_000);
        assert_eq!(view.zoom_milli, ZOOM_MAX_MILLI);
        view.zoom_by(-20_000);
        assert_eq!(view.zoom_milli, ZOOM_MIN_MILLI);
        view.pan_by(1_000_000, -1_000_000);
        assert_eq!(view.pan_x, PAN_LIMIT_PX);
        assert_eq!(view.pan_y, -PAN_LIMIT_PX);
    }
}
