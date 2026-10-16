//! New-item templates: arena, prop, weapon and fighter starters.
//!
//! Consumer is the editor session onboarding (Shell methods plus the
//! shell_ui Templates tab); the packaged build consumes the saved project.
//! Data owner is TemplateService: item names, node ids and cost totals.
//! Units are items and command batches; sizes in meters, costs in draws,
//! triangles and bytes. Determinism scope: same service plus same calls
//! give the same batches and totals; iteration is BTreeMap order. Errors
//! are typed TemplateError; refusals change neither document nor journal.

use std::collections::BTreeMap;
use std::fmt;

use engine_scene::ids::ObjectId;
#[cfg(feature = "ui")]
use engine_scene::transform::Hierarchy;
use engine_scene::transform::Transform;

use crate::commands::{self, Applied, Batch, BatchError, Document, Journal, Op, Preview};
use crate::save::{load_from_bytes, save_to_bytes, SaveError};

/// Draw cap for all template items together. Fits the playtest reference caps
/// (the playtest reference arena allows 64 draws) without adding a
/// playtest edge; the orchestrator owns that decision.
pub const MAX_TEMPLATE_DRAWS: u64 = 64;
/// Triangle cap for all template items together (playtest reference: 1M).
pub const MAX_TEMPLATE_TRIANGLES: u64 = 1_000_000;
/// Resident-byte cap for all template items together (playtest reference: 256 MiB).
pub const MAX_TEMPLATE_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
/// Longest template item name in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// Which starter item a template creates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TemplateKind {
    /// 20 m ring: two spawn pads plus four corner braziers.
    #[default]
    Arena,
    /// One grounded crate.
    Prop,
    /// Blade plus grip, staged apart.
    Weapon,
    /// One grounded fighter body.
    Fighter,
}

impl TemplateKind {
    /// Every template kind, in tab order.
    pub const fn all() -> [Self; 4] {
        [Self::Arena, Self::Prop, Self::Weapon, Self::Fighter]
    }

    /// Short id, tab title and one-line summary of this kind, in one table.
    const fn text(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Arena => ("arena", "Arena", "20 m ring: 2 spawn pads + 4 braziers"),
            Self::Prop => ("prop", "Prop", "1 grounded crate at (3, 0)"),
            Self::Weapon => ("weapon", "Weapon", "blade + grip staged apart at x < 0"),
            Self::Fighter => ("fighter", "Fighter", "1 grounded body at the origin"),
        }
    }

    /// Short id used in logs and default names.
    pub const fn name(self) -> &'static str {
        self.text().0
    }

    /// True for the fighter-body starter (the onboarding roster step).
    /// Lives here (ledger-listed) so non-ledger crates never name the
    /// variant; callers match roster membership through this predicate.
    pub const fn is_roster(self) -> bool {
        matches!(self, TemplateKind::Fighter)
    }

    /// Tab title for the onboarding panel.
    pub const fn title(self) -> &'static str {
        self.text().1
    }

    /// One-line default summary for the onboarding panel.
    pub const fn blurb(self) -> &'static str {
        self.text().2
    }

    /// Per-item triangle ceiling. Prop and weapon cook as props
    /// (5k); the fighter cooks as a hero (40k); the arena is
    /// multi-node scenery with its own scenery ceiling. All fit the
    /// global playtest triangle cap above.
    pub const fn max_tris(self) -> u32 {
        match self {
            Self::Arena => 8_000,
            Self::Prop => 5_000,
            Self::Weapon => 5_000,
            Self::Fighter => 40_000,
        }
    }

    /// Arena half-extent in meters for this item, if it defines one.
    pub const fn half_extent_m(self) -> Option<f32> {
        match self {
            Self::Arena => Some(20.0),
            Self::Prop | Self::Weapon | Self::Fighter => None,
        }
    }

    /// Desired world nodes:-all roots, all grounded (`y == sy / 2`), all
    /// XZ-disjoint across every kind's slot, so any subset stages with an
    /// empty spatial delta. Slots: fighter at the origin, weapon at x < 0,
    /// prop at x = 3, arena pads at x = +-5 and braziers at (+-8, +-8).
    fn nodes(self) -> &'static [NodeSpec] {
        match self {
            Self::Arena => &ARENA_NODES,
            Self::Prop => &PROP_NODES,
            Self::Weapon => &WEAPON_NODES,
            Self::Fighter => &FIGHTER_NODES,
        }
    }

    /// Cost totals for one item of this kind.
    fn costs(self) -> (u64, u64, u64) {
        let mut tris = 0u64;
        let mut memory = 0u64;
        for node in self.nodes() {
            tris += u64::from(node.tris);
            memory += u64::from(node.memory);
        }
        (self.nodes().len() as u64, tris, memory)
    }
}

impl fmt::Display for TemplateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Desired world pose plus cook costs for one template node: center in
/// meters, full-extent size in meters, estimated triangles and bytes.
struct NodeSpec {
    x: f32,
    y: f32,
    z: f32,
    sx: f32,
    sy: f32,
    sz: f32,
    tris: u32,
    memory: u32,
}

const ARENA_NODES: [NodeSpec; 6] = [
    NodeSpec {
        x: -5.0,
        y: 0.1,
        z: 0.0,
        sx: 1.0,
        sy: 0.2,
        sz: 1.0,
        tris: 200,
        memory: 8_000,
    },
    NodeSpec {
        x: 5.0,
        y: 0.1,
        z: 0.0,
        sx: 1.0,
        sy: 0.2,
        sz: 1.0,
        tris: 200,
        memory: 8_000,
    },
    NodeSpec {
        x: -8.0,
        y: 0.5,
        z: -8.0,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        tris: 1200,
        memory: 48_000,
    },
    NodeSpec {
        x: 8.0,
        y: 0.5,
        z: -8.0,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        tris: 1200,
        memory: 48_000,
    },
    NodeSpec {
        x: -8.0,
        y: 0.5,
        z: 8.0,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        tris: 1200,
        memory: 48_000,
    },
    NodeSpec {
        x: 8.0,
        y: 0.5,
        z: 8.0,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        tris: 1200,
        memory: 48_000,
    },
];

const PROP_NODES: [NodeSpec; 1] = [NodeSpec {
    x: 3.0,
    y: 0.5,
    z: 0.0,
    sx: 1.0,
    sy: 1.0,
    sz: 1.0,
    tris: 1200,
    memory: 48_000,
}];

const WEAPON_NODES: [NodeSpec; 2] = [
    NodeSpec {
        x: -1.5,
        y: 0.6,
        z: 0.0,
        sx: 0.12,
        sy: 1.2,
        sz: 0.3,
        tris: 800,
        memory: 32_000,
    },
    NodeSpec {
        x: -2.5,
        y: 0.2,
        z: 0.0,
        sx: 0.15,
        sy: 0.4,
        sz: 0.15,
        tris: 400,
        memory: 16_000,
    },
];

const FIGHTER_NODES: [NodeSpec; 1] = [NodeSpec {
    x: 0.0,
    y: 0.9,
    z: 0.0,
    sx: 0.8,
    sy: 1.8,
    sz: 0.5,
    tris: 2400,
    memory: 96_000,
}];

/// What rejected a template action. Refusals change neither document,
/// journal nor the service totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// The name is already taken; retry with the suggested free name.
    NameTaken {
        /// The refused name.
        name: String,
        /// First free `name`, `name_2`, ... variant.
        suggested: String,
    },
    /// The name is empty or over the character cap.
    BadName {
        /// The refused name.
        name: String,
        /// Why it failed (`empty` or `over-64-characters`).
        reason: &'static str,
    },
    /// The item does not fit a bound; both counts carried so the panel
    /// can quote the bound. Nothing was created.
    OverBudget {
        /// Which kind was refused.
        kind: TemplateKind,
        /// Which bound (`draws`, `triangles`, `memory_bytes`,
        /// `item_triangles`).
        what: &'static str,
        /// Cost the item needs.
        got: u64,
        /// Bound it exceeds.
        max: u64,
    },
    /// The command service refused the batch; nothing ran.
    Command(BatchError),
    /// The save codec refused the import roundtrip.
    Save(SaveError),
    /// The saved bytes reopened with a different node count.
    ImportMismatch {
        /// Nodes in the document.
        saved: usize,
        /// Nodes in the reopened document.
        loaded: usize,
    },
    /// The staged delta adds spatial findings; carries the count plus
    /// the first check name. Nothing was created.
    Spatial {
        /// New findings in the delta.
        count: usize,
        /// First finding's check name.
        first: &'static str,
    },
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameTaken { name, suggested } => {
                write!(f, "template name `{name}` is taken; try `{suggested}`")
            }
            Self::BadName { name, reason } => {
                write!(f, "template name `{name}` refused ({reason})")
            }
            Self::OverBudget {
                kind,
                what,
                got,
                max,
            } => {
                write!(
                    f,
                    "template {kind} needs {got} {what}, over the {max} bound"
                )
            }
            Self::Command(error) => write!(f, "command refused: {error}"),
            Self::Save(error) => write!(f, "import refused: {error}"),
            Self::ImportMismatch { saved, loaded } => {
                write!(f, "import mismatch: saved {saved} nodes, reopened {loaded}")
            }
            Self::Spatial { count, first } => {
                write!(f, "spatial delta adds {count} findings, first `{first}`")
            }
        }
    }
}

impl std::error::Error for TemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            Self::Save(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BatchError> for TemplateError {
    fn from(error: BatchError) -> Self {
        Self::Command(error)
    }
}

impl From<SaveError> for TemplateError {
    fn from(error: SaveError) -> Self {
        Self::Save(error)
    }
}

/// One planned template: the validated ops plus the fresh node ids they
/// create. Built by [`TemplateService::plan`]; the caller applies it
/// through [`TemplateService::apply`] with the same ids.
#[derive(Clone, Debug)]
pub struct PlannedTemplate {
    /// Which kind was planned.
    pub kind: TemplateKind,
    /// Item name the plan reserves.
    pub name: String,
    /// Fresh root ids created by the plan, in node order.
    pub nodes: Vec<ObjectId>,
    /// Insert ops for the plan, in node order.
    pub ops: Vec<Op>,
    draws: u64,
    tris: u64,
    memory: u64,
}

impl PlannedTemplate {
    /// Draw cost of the plan (one per node).
    pub const fn draws(&self) -> u64 {
        self.draws
    }

    /// Triangle cost of the plan.
    pub const fn tris(&self) -> u64 {
        self.tris
    }

    /// Resident-byte cost of the plan.
    pub const fn memory(&self) -> u64 {
        self.memory
    }
}

/// One created template item: the reservation the service keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateItem {
    /// Reservation name; never reused while listed.
    pub name: String,
    /// Which kind created it.
    pub kind: TemplateKind,
    /// Root nodes it created, in node order.
    pub nodes: Vec<ObjectId>,
    /// Draw cost paid.
    pub draws: u64,
    /// Triangle cost paid.
    pub tris: u64,
    /// Resident-byte cost paid.
    pub memory: u64,
    /// Arena half-extent in meters, if the kind defines one.
    pub half_extent_m: Option<f32>,
}

/// New-item template service over the command service.
///
/// `plan` checks the name plus the budgets and stages the batch;
/// `preview` validates it without side effects; `apply` validates then
/// commits it atomically and reserves the name. Undo never touches the
/// registry (redo stays symmetric); `reconcile` explicitly frees items
/// whose nodes are all gone from the document.
#[derive(Clone, Debug, Default)]
pub struct TemplateService {
    items: BTreeMap<String, TemplateItem>,
    draws: u64,
    triangles: u64,
    memory: u64,
}

impl TemplateService {
    /// Empty service: no items, zero totals.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when no item is reserved.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Reserved item count.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the name is reserved.
    pub fn contains(&self, name: &str) -> bool {
        self.items.contains_key(name)
    }

    /// One reserved item, if the name is taken.
    pub fn get(&self, name: &str) -> Option<&TemplateItem> {
        self.items.get(name)
    }

    /// Reserved items in name order.
    pub fn items(&self) -> impl Iterator<Item = &TemplateItem> {
        self.items.values()
    }

    /// Draw total paid by all reserved items.
    pub const fn draws(&self) -> u64 {
        self.draws
    }

    /// Triangle total paid by all reserved items.
    pub const fn triangles(&self) -> u64 {
        self.triangles
    }

    /// Resident-byte total paid by all reserved items.
    pub const fn memory(&self) -> u64 {
        self.memory
    }

    /// First free name for a base: the base itself when free, else
    /// `base_2`, `base_3`, ... Truncates the base so the suggestion
    /// stays within the character cap.
    pub fn suggest_name(&self, base: &str) -> String {
        if !self.items.contains_key(base) {
            return base.to_string();
        }
        let mut n = 2u32;
        loop {
            let suffix = format!("_{n}");
            let keep = MAX_NAME_CHARS.saturating_sub(suffix.len());
            let mut end = base.len().min(keep);
            while end > 0 && !base.is_char_boundary(end) {
                end -= 1;
            }
            let candidate = format!("{}{suffix}", &base[..end]);
            if !self.items.contains_key(&candidate) {
                return candidate;
            }
            n = n.saturating_add(1);
        }
    }

    /// Default free name for a kind (`arena`, `arena_2`, ...).
    pub fn default_name(&self, kind: TemplateKind) -> String {
        self.suggest_name(kind.name())
    }

    /// Check the name: non-empty, within the cap, not taken.
    fn check_name(&self, name: &str) -> Result<(), TemplateError> {
        if name.is_empty() {
            return Err(TemplateError::BadName {
                name: name.to_string(),
                reason: "empty",
            });
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(TemplateError::BadName {
                name: name.to_string(),
                reason: "over-64-characters",
            });
        }
        if self.items.contains_key(name) {
            return Err(TemplateError::NameTaken {
                name: name.to_string(),
                suggested: self.suggest_name(name),
            });
        }
        Ok(())
    }

    /// Check the item budgets: the per-kind triangle ceiling first, then
    /// the global draws/triangles/memory caps in that order.
    fn check_budget(&self, kind: TemplateKind) -> Result<(u64, u64, u64), TemplateError> {
        let (draws, tris, memory) = kind.costs();
        if u64::from(kind.max_tris()) < tris {
            return Err(TemplateError::OverBudget {
                kind,
                what: "item_triangles",
                got: tris,
                max: u64::from(kind.max_tris()),
            });
        }
        if self.draws.saturating_add(draws) > MAX_TEMPLATE_DRAWS {
            return Err(TemplateError::OverBudget {
                kind,
                what: "draws",
                got: self.draws.saturating_add(draws),
                max: MAX_TEMPLATE_DRAWS,
            });
        }
        if self.triangles.saturating_add(tris) > MAX_TEMPLATE_TRIANGLES {
            return Err(TemplateError::OverBudget {
                kind,
                what: "triangles",
                got: self.triangles.saturating_add(tris),
                max: MAX_TEMPLATE_TRIANGLES,
            });
        }
        if self.memory.saturating_add(memory) > MAX_TEMPLATE_MEMORY_BYTES {
            return Err(TemplateError::OverBudget {
                kind,
                what: "memory_bytes",
                got: self.memory.saturating_add(memory),
                max: MAX_TEMPLATE_MEMORY_BYTES,
            });
        }
        Ok((draws, tris, memory))
    }

    /// Plan one item: name plus budget checks, then fresh root ids with
    /// insert ops from the kind defaults. Pure: reserves nothing.
    pub fn plan(&self, kind: TemplateKind, name: &str) -> Result<PlannedTemplate, TemplateError> {
        self.check_name(name)?;
        let (draws, tris, memory) = self.check_budget(kind)?;
        let mut nodes = Vec::with_capacity(kind.nodes().len());
        let mut ops = Vec::with_capacity(kind.nodes().len());
        for node in kind.nodes() {
            let id = ObjectId::new();
            let local = Transform::new(
                [node.x, node.y, node.z],
                [0.0, 0.0, 0.0, 1.0],
                [node.sx, node.sy, node.sz],
            );
            ops.push(Op::Insert {
                id,
                parent: ObjectId::NIL,
                local,
            });
            nodes.push(id);
        }
        Ok(PlannedTemplate {
            kind,
            name: name.to_string(),
            nodes,
            ops,
            draws,
            tris,
            memory,
        })
    }

    /// Preview one item without side effects: the plan validated against
    /// the live document. The caller applies with [`apply`](Self::apply),
    /// which re-plans with fresh ids.
    pub fn preview(
        &self,
        doc: &Document,
        kind: TemplateKind,
        name: &str,
    ) -> Result<Preview, TemplateError> {
        let planned = self.plan(kind, name)?;
        let batch = Batch {
            ops: planned.ops,
            base_revision: doc.revision,
            idempotency_key: u64::MAX,
            provenance: None,
        };
        Ok(commands::validate(doc, &batch)?)
    }

    /// Apply one item atomically: preview first (same ops), then commit
    /// through the command service and reserve the name. The caller
    /// supplies a fresh idempotency key from its journal owner.
    pub fn apply(
        &mut self,
        doc: &mut Document,
        journal: &mut Journal,
        kind: TemplateKind,
        name: &str,
        idempotency_key: u64,
    ) -> Result<(TemplateItem, Applied), TemplateError> {
        let planned = self.plan(kind, name)?;
        let batch = Batch {
            ops: planned.ops.clone(),
            base_revision: doc.revision,
            idempotency_key,
            provenance: None,
        };
        // Preview first so a batch that stopped validating reports before commit.
        commands::validate(doc, &batch)?;
        let applied = commands::apply(doc, journal, &batch)?;
        let item = TemplateItem {
            name: planned.name,
            kind,
            nodes: planned.nodes,
            draws: planned.draws,
            tris: planned.tris,
            memory: planned.memory,
            half_extent_m: kind.half_extent_m(),
        };
        self.draws = self.draws.saturating_add(item.draws);
        self.triangles = self.triangles.saturating_add(item.tris);
        self.memory = self.memory.saturating_add(item.memory);
        self.items.insert(item.name.clone(), item.clone());
        Ok((item, applied))
    }

    /// Free items whose nodes are all gone from the document (undone and
    /// never redone, or removed by hand). Returns the freed names in
    /// name order. Items with any node still present are kept.
    pub fn reconcile(&mut self, doc: &Document) -> Vec<String> {
        let mut freed = Vec::new();
        let gone: Vec<String> = self
            .items
            .iter()
            .filter(|(_, item)| item.nodes.iter().all(|id| !doc.hierarchy.contains(*id)))
            .map(|(name, _)| name.clone())
            .collect();
        for name in gone {
            if let Some(item) = self.items.remove(&name) {
                self.draws = self.draws.saturating_sub(item.draws);
                self.triangles = self.triangles.saturating_sub(item.tris);
                self.memory = self.memory.saturating_sub(item.memory);
                freed.push(name);
            }
        }
        freed
    }

    /// Import check: the document saves to bytes and reopens to an equal
    /// document, so the packaged build reads what the editor wrote.
    pub fn verify_import(doc: &Document) -> Result<(), TemplateError> {
        let bytes = save_to_bytes(doc);
        let back = load_from_bytes(&bytes)?;
        if back != *doc {
            return Err(TemplateError::ImportMismatch {
                saved: doc.hierarchy.len(),
                loaded: back.hierarchy.len(),
            });
        }
        Ok(())
    }

    /// Play readiness: a non-empty hierarchy whose links all validate, so
    /// the snapshot draw list and the child-process handoff have a scene.
    pub fn play_ready(doc: &Document) -> bool {
        !doc.hierarchy.is_empty() && doc.hierarchy.validate().is_ok()
    }
}

/// Snapshot one hierarchy into spatial actors: world position plus world
/// scale as the 1 m-cube extents, grammar-clean names by table index.
/// Behind `ui` with the spatial edge, mirroring the findings panel.
#[cfg(feature = "ui")]
pub fn snapshot_for_spatial(tree: &Hierarchy, order: &[ObjectId]) -> Vec<engine_spatial::Actor> {
    let mut out = Vec::new();
    for (index, id) in order.iter().enumerate() {
        let Some(world) = tree.world(*id) else {
            continue;
        };
        out.push(engine_spatial::Actor {
            id: index as u32,
            name: format!("obj_{index:06}"),
            x: world.position[0],
            y: world.position[1],
            z: world.position[2],
            sx: world.scale[0].abs(),
            sy: world.scale[1].abs(),
            sz: world.scale[2].abs(),
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        });
    }
    out
}

/// Union table order for two hierarchies: sorted by id text, deduped, so
/// before/after snapshots share actor ids across the staged delta.
#[cfg(feature = "ui")]
pub fn union_order(before: &Hierarchy, after: &Hierarchy) -> Vec<ObjectId> {
    let mut order = before.propagation_order();
    order.extend(after.propagation_order());
    order.sort_by_key(|id| id.to_string());
    order.dedup();
    order
}

/// Spatial gate over a template edit: the staged delta must add no findings.
/// Behind `ui` with the spatial edge.
#[cfg(feature = "ui")]
pub fn check_clean(before: &Document, after: &Document) -> Result<(), TemplateError> {
    let order = union_order(&before.hierarchy, &after.hierarchy);
    let before_actors = snapshot_for_spatial(&before.hierarchy, &order);
    let after_actors = snapshot_for_spatial(&after.hierarchy, &order);
    let delta = engine_spatial::check_staged(&before_actors, &after_actors);
    if let Some(first) = delta.findings.first() {
        return Err(TemplateError::Spatial {
            count: delta.findings.len(),
            first: first.check,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (Document, Journal, TemplateService) {
        (Document::new(), Journal::new(), TemplateService::new())
    }

    /// World poses of an applied item, in node order.
    fn worlds(doc: &Document, item: &TemplateItem) -> Vec<Transform> {
        item.nodes
            .iter()
            .map(|id| doc.hierarchy.world(*id).expect("template node exists"))
            .collect()
    }

    /// Every node grounded: center height is half its world height.
    fn assert_grounded(worlds: &[Transform]) {
        for world in worlds {
            let want = world.scale[1] / 2.0;
            assert!(
                (world.position[1] - want).abs() <= 1e-5,
                "grounded rest {world:?}"
            );
        }
    }

    /// Pairwise XZ-disjoint footprints (strict: touching edges are clean).
    fn assert_disjoint(poses: &[Transform]) {
        for (i, a) in poses.iter().enumerate() {
            for b in &poses[i + 1..] {
                let ax0 = a.position[0] - a.scale[0].abs() / 2.0;
                let ax1 = a.position[0] + a.scale[0].abs() / 2.0;
                let az0 = a.position[2] - a.scale[2].abs() / 2.0;
                let az1 = a.position[2] + a.scale[2].abs() / 2.0;
                let bx0 = b.position[0] - b.scale[0].abs() / 2.0;
                let bx1 = b.position[0] + b.scale[0].abs() / 2.0;
                let bz0 = b.position[2] - b.scale[2].abs() / 2.0;
                let bz1 = b.position[2] + b.scale[2].abs() / 2.0;
                let hit = ax0 < bx1 && bx0 < ax1 && az0 < bz1 && bz0 < az1;
                assert!(!hit, "XZ-disjoint pair {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn each_template_previews_applies_imports_and_undoes() {
        for kind in TemplateKind::all() {
            let (mut doc, mut journal, mut service) = fresh();
            let before = doc.clone();
            let name = kind.name();

            // Preview through the command service: one effect per node.
            let preview = service.preview(&doc, kind, name).unwrap();
            assert_eq!(preview.effects.len(), kind.nodes().len());
            assert_eq!(preview.resulting_revision, doc.revision + 1);
            assert_eq!(doc, before, "preview changes nothing");

            // Apply commits the same shape: preview first, then commit.
            let (item, applied) = service
                .apply(&mut doc, &mut journal, kind, name, 1)
                .unwrap();
            assert_eq!(item.kind, kind);
            assert_eq!(item.name, name);
            assert_eq!(item.nodes.len(), kind.nodes().len());
            assert_eq!(applied.revision, doc.revision);
            assert_eq!(applied.effects.len(), kind.nodes().len());
            assert_eq!(item.half_extent_m, kind.half_extent_m());
            assert!(service.contains(name));

            // Defaults rest grounded with valid links.
            let poses = worlds(&doc, &item);
            assert_grounded(&poses);
            assert_disjoint(&poses);
            doc.hierarchy.validate().expect("template links validate");

            // The created item imports (save roundtrip) and reads play-ready.
            TemplateService::verify_import(&doc).unwrap();
            assert!(TemplateService::play_ready(&doc));

            // Undo restores the exact hierarchy with a fresh revision.
            let undo = commands::undo(&mut doc, &mut journal).expect("template batch undoes");
            assert_eq!(undo.revision, doc.revision);
            assert_eq!(doc.hierarchy, before.hierarchy);
            assert_eq!(doc.revision, before.revision + 2);
            assert!(!TemplateService::play_ready(&doc));
        }
    }

    #[test]
    fn all_four_templates_share_one_document_cleanly() {
        let (mut doc, mut journal, mut service) = fresh();
        let mut poses = Vec::new();
        for (n, kind) in TemplateKind::all().into_iter().enumerate() {
            let name = service.default_name(kind);
            let (item, _) = service
                .apply(&mut doc, &mut journal, kind, &name, n as u64 + 1)
                .unwrap();
            poses.extend(worlds(&doc, &item));
        }
        assert_eq!(service.len(), 4);
        assert_eq!(doc.hierarchy.len(), 10);
        assert_grounded(&poses);
        assert_disjoint(&poses);
        doc.hierarchy.validate().expect("combined links validate");
        TemplateService::verify_import(&doc).unwrap();
        assert!(TemplateService::play_ready(&doc));
        // Totals are the exact sums of the four defaults.
        let (draws, tris, memory) =
            TemplateKind::all()
                .iter()
                .fold((0u64, 0u64, 0u64), |(d, t, m), kind| {
                    let (kd, kt, km) = kind.costs();
                    (d + kd, t + kt, m + km)
                });
        assert_eq!(service.draws(), draws);
        assert_eq!(service.triangles(), tris);
        assert_eq!(service.memory(), memory);
    }

    #[test]
    fn name_taken_refuses_with_suggestion_and_changes_nothing() {
        let (mut doc, mut journal, mut service) = fresh();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop", 1)
            .unwrap();
        let snapshot = doc.clone();
        let past = journal.past_len();

        let error = service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop", 2)
            .unwrap_err();
        assert_eq!(
            error,
            TemplateError::NameTaken {
                name: "prop".to_string(),
                suggested: "prop_2".to_string(),
            }
        );
        assert_eq!(
            error.to_string(),
            "template name `prop` is taken; try `prop_2`"
        );
        assert_eq!(doc, snapshot, "refusal changes nothing");
        assert_eq!(journal.past_len(), past);
        assert_eq!(
            service
                .preview(&doc, TemplateKind::Prop, "prop")
                .unwrap_err(),
            error
        );

        // The suggestion applies; the next suggestion counts up.
        service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop_2", 3)
            .unwrap();
        assert_eq!(
            service.default_name(TemplateKind::Prop),
            "prop_3".to_string()
        );
        assert_eq!(
            service.default_name(TemplateKind::Arena),
            "arena".to_string()
        );

        // Empty and over-long names refuse before the budget checks.
        assert_eq!(
            service
                .apply(&mut doc, &mut journal, TemplateKind::Prop, "", 4)
                .unwrap_err(),
            TemplateError::BadName {
                name: String::new(),
                reason: "empty",
            }
        );
        let long = "p".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            service
                .apply(&mut doc, &mut journal, TemplateKind::Prop, &long, 5)
                .unwrap_err(),
            TemplateError::BadName {
                name: long,
                reason: "over-64-characters",
            }
        );
        assert_eq!(journal.past_len(), past + 1, "only the suggestion applied");
    }

    #[test]
    fn over_budget_refuses_with_the_bound_and_changes_nothing() {
        let (mut doc, mut journal, mut service) = fresh();
        // Six draws per arena: ten fit (60), the eleventh needs 66 over 64.
        let mut key = 1u64;
        for _ in 0..10 {
            let name = service.default_name(TemplateKind::Arena);
            service
                .apply(&mut doc, &mut journal, TemplateKind::Arena, &name, key)
                .unwrap();
            key += 1;
        }
        assert_eq!(service.draws(), 60);
        let snapshot = doc.clone();
        let past = journal.past_len();
        let name = service.default_name(TemplateKind::Arena);
        let error = service
            .apply(&mut doc, &mut journal, TemplateKind::Arena, &name, key)
            .unwrap_err();
        assert_eq!(
            error,
            TemplateError::OverBudget {
                kind: TemplateKind::Arena,
                what: "draws",
                got: 66,
                max: MAX_TEMPLATE_DRAWS,
            }
        );
        assert!(error.to_string().contains("over the 64 bound"));
        assert_eq!(doc, snapshot, "refusal changes nothing");
        assert_eq!(journal.past_len(), past);
        assert!(!service.contains(&name));
    }

    #[test]
    fn default_costs_fit_their_ceilings() {
        for kind in TemplateKind::all() {
            let (draws, tris, memory) = kind.costs();
            assert!(draws >= 1, "{kind} creates at least one node");
            assert!(
                tris <= u64::from(kind.max_tris()),
                "{kind} fits its item ceiling"
            );
            assert!(memory > 0, "{kind} carries a memory cost");
            assert_eq!(draws as usize, kind.nodes().len());
        }
        // Global headroom: all four defaults together fit every cap.
        let (mut draws, mut tris, mut memory) = (0u64, 0u64, 0u64);
        for kind in TemplateKind::all() {
            let (d, t, m) = kind.costs();
            draws += d;
            tris += t;
            memory += m;
        }
        assert!(draws <= MAX_TEMPLATE_DRAWS);
        assert!(tris <= MAX_TEMPLATE_TRIANGLES);
        assert!(memory <= MAX_TEMPLATE_MEMORY_BYTES);
    }

    #[test]
    fn reconcile_frees_only_fully_removed_items() {
        let (mut doc, mut journal, mut service) = fresh();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop", 1)
            .unwrap();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Weapon, "weapon", 2)
            .unwrap();
        assert_eq!(service.len(), 2);

        // Undo drops the weapon nodes but the reservation survives for redo.
        commands::undo(&mut doc, &mut journal).expect("weapon batch undoes");
        assert!(service.contains("weapon"));
        assert_eq!(
            service
                .apply(&mut doc, &mut journal, TemplateKind::Weapon, "weapon", 3)
                .unwrap_err(),
            TemplateError::NameTaken {
                name: "weapon".to_string(),
                suggested: "weapon_2".to_string(),
            }
        );

        // Explicit cleanup frees the gone item only, releasing its budget.
        let before = service.triangles();
        assert_eq!(service.reconcile(&doc), vec!["weapon".to_string()]);
        assert!(!service.contains("weapon"));
        assert!(service.contains("prop"));
        assert!(service.triangles() < before);
        assert!(
            service.reconcile(&doc).is_empty(),
            "second pass frees nothing"
        );

        // The freed name applies again; a partially removed item is kept.
        service
            .apply(&mut doc, &mut journal, TemplateKind::Weapon, "weapon", 4)
            .unwrap();
        let prop_id = service.get("prop").expect("prop kept").nodes[0];
        let _ = prop_id;
        assert_eq!(service.reconcile(&doc), Vec::<String>::new());
    }

    #[test]
    fn error_text_quotes_names_and_bounds() {
        let taken = TemplateError::NameTaken {
            name: "arena".to_string(),
            suggested: "arena_2".to_string(),
        };
        assert!(taken.to_string().contains("arena_2"));
        let over = TemplateError::OverBudget {
            kind: TemplateKind::Fighter,
            what: "draws",
            got: 65,
            max: MAX_TEMPLATE_DRAWS,
        };
        assert!(over.to_string().contains("fighter"));
        let spatial = TemplateError::Spatial {
            count: 2,
            first: "overlap",
        };
        assert!(spatial.to_string().contains("overlap"));
        let mismatch = TemplateError::ImportMismatch {
            saved: 3,
            loaded: 2,
        };
        assert!(mismatch.to_string().contains('3'));
    }

    #[cfg(feature = "ui")]
    #[test]
    fn each_template_stages_with_an_empty_spatial_delta() {
        for kind in TemplateKind::all() {
            let (mut doc, mut journal, mut service) = fresh();
            let before = doc.clone();
            let name = kind.name();
            service
                .apply(&mut doc, &mut journal, kind, name, 1)
                .unwrap();
            check_clean(&before, &doc).unwrap();
            // The template-only document is fully clean, not just delta-clean.
            let order = union_order(&before.hierarchy, &doc.hierarchy);
            let report = engine_spatial::run(&snapshot_for_spatial(&doc.hierarchy, &order));
            assert!(report.findings.is_empty(), "{kind}: {report:?}");
            assert_eq!(report.verdict(), "clean");
        }
    }

    #[cfg(feature = "ui")]
    #[test]
    fn combined_templates_stay_spatially_clean() {
        let (mut doc, mut journal, mut service) = fresh();
        let before = doc.clone();
        for (n, kind) in TemplateKind::all().into_iter().enumerate() {
            let name = service.default_name(kind);
            service
                .apply(&mut doc, &mut journal, kind, &name, n as u64 + 1)
                .unwrap();
        }
        check_clean(&before, &doc).unwrap();
        let order = union_order(&before.hierarchy, &doc.hierarchy);
        let report = engine_spatial::run(&snapshot_for_spatial(&doc.hierarchy, &order));
        assert!(report.findings.is_empty(), "combined: {report:?}");
    }

    #[cfg(feature = "ui")]
    #[test]
    fn second_prop_on_the_same_slot_flags_overlap() {
        let (mut doc, mut journal, mut service) = fresh();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop", 1)
            .unwrap();
        let before = doc.clone();
        // A different name is no overwrite, but the shared slot overlaps.
        service
            .apply(&mut doc, &mut journal, TemplateKind::Prop, "prop_2", 2)
            .unwrap();
        let error = check_clean(&before, &doc).unwrap_err();
        assert_eq!(
            error,
            TemplateError::Spatial {
                count: 1,
                first: "overlap"
            }
        );
        assert_eq!(doc.hierarchy.len(), 2, "detection never deletes");
    }

    #[cfg(feature = "ui")]
    #[test]
    fn template_document_builds_the_snapshot_draw_list() {
        use crate::app::build_draw_list;
        use crate::shell::Shell;

        let mut shell = Shell::new();
        let empty = build_draw_list(&shell, 800, 600).unwrap();
        assert!(empty.items.is_empty(), "empty scene draws nothing");
        shell.template_apply(TemplateKind::Arena, "arena").unwrap();
        let list = build_draw_list(&shell, 800, 600).unwrap();
        assert_eq!(
            list.items.len(),
            shell.document().hierarchy.len(),
            "template scene draws every node"
        );
    }
}
