//! Graph evaluation seam (E7): the canvas writes a material graph, the material
//! pack parses, folds and bakes it.
//!
//! The canvas holds layout (positions, typed ports, links); this module turns
//! that layout into the canonical text
//! [`engine_material::graph::Graph`] parses. Nothing is written to disk and no
//! material record is stored here: the caller decides what to do with the
//! digest and the four map digests.
//! Units: map digests come from a [`BAKE_RES`] square bake; ticks play no part.
//! Determinism: nodes are written in id order and positions never enter the
//! material text, so moving a node keeps the material digest and moves only
//! [`Canvas::layout_digest`].
//! Errors: [`CanvasError`]; a refusal keeps the material error's own text.

use engine_material::graph::{Graph, Node, NodeKind, Output, Port, KINDS, MAX_NODES};

use crate::graph_canvas::{Canvas, CanvasError, PortKind};

/// Material name the canvas writes; the pack's name rule accepts it.
pub const MATERIAL: &str = "canvas";
/// Parent record the folded graph extends.
pub const BASE: &str = "stone";
/// Bake size in pixels; the pack bakes [`engine_material::graph::BAKE_SIZES`]
/// only, and this is the smallest.
pub const BAKE_RES: u32 = 512;

/// Pack ports a canvas port kind may feed: color, scalar and normal fields
/// drive `a` or `b`, and only a mask drives `mask`. The pack itself types no
/// field, so a wrong pairing is refused at export rather than guessed.
pub fn ports_of(kind: PortKind) -> &'static [Port] {
    match kind {
        PortKind::Color | PortKind::Scalar | PortKind::Normal => &[Port::A, Port::B],
        PortKind::Mask => &[Port::Mask],
    }
}

/// Pack port a canvas input slot stands for, in slot order.
pub fn port_at(index: u8) -> Option<Port> {
    match index {
        0 => Some(Port::A),
        1 => Some(Port::B),
        2 => Some(Port::Mask),
        _ => None,
    }
}

/// Where a pack port sits in [`Node::inputs`].
fn slot(port: Port) -> usize {
    match port {
        Port::A => 0,
        Port::B => 1,
        Port::Mask => 2,
    }
}

/// What one evaluation produced: the graph digest and the four map digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialVerdict {
    /// Digest of the canonical graph text, as the bake stores it.
    pub digest: u64,
    /// Nodes the folded graph held.
    pub nodes: usize,
    /// Map digests in [`engine_material::graph::BAKE_MAPS`] order.
    pub maps: [u64; 4],
    /// Coverage channels the bake found flat (a warning, not a refusal).
    pub warnings: Vec<String>,
}

/// One `E` press: the row the panel draws plus the verdict when it baked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialReport {
    /// Status row: `material <digest> nodes=<n> map0=<digest>`, or the
    /// refusal's own text.
    pub line: String,
    /// The verdict; `None` when the export, parse or bake refused.
    pub verdict: Option<MaterialVerdict>,
}

impl MaterialReport {
    /// Report one evaluation result.
    pub fn of(result: Result<MaterialVerdict, CanvasError>) -> Self {
        match result {
            Ok(verdict) => Self {
                line: format!(
                    "material {:016x} nodes={} map0={:016x}",
                    verdict.digest, verdict.nodes, verdict.maps[0]
                ),
                verdict: Some(verdict),
            },
            Err(error) => Self {
                line: error.to_string(),
                verdict: None,
            },
        }
    }
}

impl Canvas {
    /// The material graph this canvas describes, in the pack's own types.
    ///
    /// Refusals: an unset output block, a node kind with no material
    /// equivalent, or more input ports than the pack has (`a`, `b`, `mask`).
    pub fn material_graph(&self) -> Result<Graph, CanvasError> {
        let output = self.material_output().ok_or(CanvasError::NoOutput)?;
        let mut nodes = Vec::with_capacity(self.nodes().len());
        for node in self.nodes() {
            let kind =
                NodeKind::from_name(&node.kind).ok_or_else(|| CanvasError::UnmappedNode {
                    id: node.id,
                    kind: node.kind.clone(),
                    hint: KINDS.join(", "),
                })?;
            let mut inputs = [None; 3];
            for link in self.links().iter().filter(|link| link.to == node.id) {
                let index = usize::from(link.to_port);
                let carrier = node
                    .inputs
                    .get(index)
                    .copied()
                    .ok_or(CanvasError::PortMissing {
                        id: node.id,
                        port: link.to_port,
                        input: true,
                    })?;
                let pack_port = port_at(link.to_port).ok_or_else(|| CanvasError::UnmappedPort {
                    id: node.id,
                    port: link.to_port,
                    hint: "a, b or mask".to_owned(),
                })?;
                let Some(read) = kind.ports().get(index).copied() else {
                    return Err(CanvasError::UnmappedPort {
                        id: node.id,
                        port: link.to_port,
                        hint: format!("{} reads fewer inputs", kind.name()),
                    });
                };
                if !ports_of(carrier).contains(&pack_port) {
                    return Err(CanvasError::UnmappedPort {
                        id: node.id,
                        port: link.to_port,
                        hint: format!("a {} port feeds {}", carrier.label(), pack_port.name()),
                    });
                }
                if let Some(place) = inputs.get_mut(slot(read)) {
                    *place = Some(link.from);
                }
            }
            nodes.push(Node {
                id: node.id,
                kind,
                inputs,
                params: node.params,
            });
        }
        Ok(Graph {
            material: MATERIAL.to_owned(),
            base: BASE.to_owned(),
            nodes,
            output,
            layers: Vec::new(),
        })
    }

    /// The canonical text the material pack parses: nodes in id order, one
    /// newline at the end, no layout in it.
    pub fn to_material_text(&self) -> Result<String, CanvasError> {
        Ok(self.material_graph()?.canonical())
    }

    /// Export, parse in the pack, bake [`BAKE_RES`] maps and answer the
    /// digests. A parse or bake refusal keeps the pack's own message.
    pub fn evaluate(&self) -> Result<MaterialVerdict, CanvasError> {
        let text = self.to_material_text()?;
        let graph = Graph::parse(&text).map_err(|error| CanvasError::Material {
            text: error.to_string(),
        })?;
        let baked = graph
            .bake(BAKE_RES)
            .map_err(|error| CanvasError::Material {
                text: error.to_string(),
            })?;
        let coverage = graph
            .bake_coverage(BAKE_RES)
            .map_err(|error| CanvasError::Material {
                text: error.to_string(),
            })?;
        let mut maps = [0u64; 4];
        for (place, map) in maps.iter_mut().zip(baked.maps.iter()) {
            *place = map.digest;
        }
        Ok(MaterialVerdict {
            digest: baked.graph,
            nodes: graph.nodes.len(),
            maps,
            warnings: coverage
                .flat
                .iter()
                .map(|name| format!("coverage {name} is flat"))
                .collect(),
        })
    }
}

/// Node ids the canvas writes are the pack's `1..=MAX_NODES`; a caller id
/// outside that range is refused by the pack's parse.
pub const MATERIAL_MAX_NODES: usize = MAX_NODES;

/// The default output block: color, height and emissive from one field,
/// roughness and metalness from another, black-to-white base color.
pub fn default_output(color: u32, roughness: u32) -> Output {
    Output {
        color,
        color_low: [0.0; 3],
        color_high: [1.0; 3],
        roughness,
        metalness: roughness,
        height: color,
        channels: std::collections::BTreeMap::new(),
        emissive: Some(color),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_canvas::{CanvasNode, Link};

    /// A canvas node, with one scalar input slot when asked.
    fn node(id: u32, kind: &str, scalar_input: bool) -> CanvasNode {
        CanvasNode {
            id,
            x: 0,
            y: 0,
            kind: kind.to_owned(),
            params: engine_material::graph::Params::default(),
            inputs: if scalar_input {
                vec![PortKind::Scalar]
            } else {
                Vec::new()
            },
            outputs: vec![PortKind::Color, PortKind::Scalar],
        }
    }

    /// Base color from a noise field, roughness from a curve over that field.
    fn two_node_canvas() -> Canvas {
        let mut canvas = Canvas::new();
        canvas.add_node(node(1, "noise", false)).unwrap();
        let mut rough = node(2, "curve", true);
        rough.params.exponent = 2;
        canvas.add_node(rough).unwrap();
        canvas
            .connect(Link {
                from: 1,
                from_port: 1,
                to: 2,
                to_port: 0,
            })
            .unwrap();
        canvas.set_material_output(default_output(1, 2));
        canvas
    }

    #[test]
    fn a_canvas_exports_text_the_material_pack_bakes() {
        let canvas = two_node_canvas();
        let text = canvas.to_material_text().unwrap();
        assert!(text.starts_with('{') && text.ends_with("}\n"));
        let graph = Graph::parse(&text).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[1].inputs[0], Some(1), "link lands on port a");
        let baked = graph.bake(BAKE_RES).unwrap();
        assert_eq!(baked.maps.len(), 4);
        for map in &baked.maps {
            assert!(!map.png.is_empty(), "map {} has pixels", map.name);
        }
    }

    #[test]
    fn two_evaluations_agree_on_text_digest_and_maps() {
        let canvas = two_node_canvas();
        let first = canvas.evaluate().unwrap();
        let second = canvas.evaluate().unwrap();
        assert_eq!(
            canvas.to_material_text().unwrap(),
            canvas.to_material_text().unwrap()
        );
        assert_eq!(first, second);
        assert_eq!(first.nodes, 2);
        assert!(first.maps.iter().all(|digest| *digest != 0));
    }

    #[test]
    fn an_unmapped_node_kind_is_refused_by_name() {
        let mut canvas = two_node_canvas();
        canvas.add_node(node(3, "voronoi", false)).unwrap();
        let error = canvas.evaluate().unwrap_err();
        let CanvasError::UnmappedNode { id, kind, hint } = &error else {
            panic!("expected UnmappedNode, got {error:?}");
        };
        assert_eq!((*id, kind.as_str()), (3, "voronoi"));
        assert!(hint.contains("noise"), "hint names a legal kind: {hint}");
        assert!(error.to_string().contains("noise"));
    }

    #[test]
    fn moving_a_node_keeps_the_material_digest_and_changes_the_layout() {
        let mut canvas = two_node_canvas();
        let before = canvas.evaluate().unwrap();
        let layout_before = canvas.layout_digest();
        canvas.move_node(2, 320, 96).unwrap();
        let after = canvas.evaluate().unwrap();
        assert_eq!(
            before.digest, after.digest,
            "position never enters the text"
        );
        assert_eq!(before.maps, after.maps);
        assert_ne!(layout_before, canvas.layout_digest());
    }

    #[test]
    fn a_param_out_of_band_keeps_the_material_error_text() {
        // Noise scale 0 is outside the pack's `1..=64`.
        let mut wide = node(1, "noise", false);
        wide.params.scale = 0;
        let mut canvas = Canvas::new();
        canvas.add_node(wide).unwrap();
        canvas.add_node(node(2, "noise", false)).unwrap();
        canvas.set_material_output(default_output(1, 2));
        let error = canvas.evaluate().unwrap_err();
        let CanvasError::Material { text } = &error else {
            panic!("expected Material, got {error:?}");
        };
        assert!(text.contains("scale"), "pack text kept: {text}");
        assert!(!text.is_empty());
    }

    #[test]
    fn a_canvas_without_an_output_block_is_refused() {
        let mut canvas = Canvas::new();
        canvas.add_node(node(1, "noise", false)).unwrap();
        assert_eq!(canvas.to_material_text(), Err(CanvasError::NoOutput));
    }

    #[test]
    fn the_report_line_names_the_digest_and_the_first_map() {
        let report = MaterialReport::of(two_node_canvas().evaluate());
        let verdict = report.verdict.clone().unwrap();
        assert_eq!(
            report.line,
            format!(
                "material {:016x} nodes=2 map0={:016x}",
                verdict.digest, verdict.maps[0]
            )
        );
        let refused = MaterialReport::of(Err(CanvasError::NoOutput));
        assert!(refused.verdict.is_none());
        assert_eq!(refused.line, CanvasError::NoOutput.to_string());
    }
}
