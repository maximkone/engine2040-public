//! Geometry-only batch staging: what the measure-first readers need.
//!
//! The CAP1 spatial gate ([`crate::capability`]) and the findings panel
//! ([`crate::findings`]) both ask "what would this batch commit?" before they
//! touch the document. They need the pose and component tables only, and they
//! need the same answer [`crate::commands::apply`] would produce, so the ops
//! run through the command service's own staged apply. No journal, no preview,
//! no revision gate: a caller that cares about staleness checks it first.
//!
//! Lives beside `commands.rs` instead of inside it because that file is
//! already over the size pin and may not grow (`docs/RUST-QUALITY.md`).

use engine_scene::transform::Hierarchy;

use crate::commands::{apply_ops, BatchError, ComponentTable, Document, Op};

/// Stage `ops` on a private copy and return the resulting hierarchy plus
/// component table. Material ops run too (they must, to keep the staging
/// order identical to a commit) but their tables are dropped: no geometry
/// reader reads them.
pub(crate) fn stage(doc: &Document, ops: &[Op]) -> Result<(Hierarchy, ComponentTable), BatchError> {
    let mut tree = doc.hierarchy.clone();
    let mut materials = doc.materials.clone();
    let mut components = doc.components.clone();
    let mut bonds = doc.bonds.clone();
    let mut material_set = doc.material_set.clone();
    apply_ops(
        &mut tree,
        &mut materials,
        &mut components,
        &mut bonds,
        &mut material_set,
        ops,
    )?;
    Ok((tree, components))
}
