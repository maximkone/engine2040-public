//! Prefabs: authored hierarchy kits with versioned components.
//!
//! Consumer: game templates author prefabs and boot new games on them.
//! Data owner: the prefab document (keys, links, poses, components);
//! instance ids live in the target [`Hierarchy`]. Units: keys
//! (authoring-stable strings), components, versions. Determinism:
//! instantiate inserts parents-first with siblings sorted by key, so the
//! same document yields the same links and poses whatever order nodes were
//! listed in. Copying mints fresh ids (a copy is a new object), so id bytes
//! differ per instantiation by design; compare structure through
//! [`Instantiation`].
//! Errors: [`PrefabError`]; unknown versions refuse loudly, never guess.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use engine_core::Fnv64;

use super::ids::ObjectId;
use super::schema::{SchemaError, SchemaRegistry, VersionedComponent};
use super::transform::{Hierarchy, HierarchyError, Transform};

/// Prefab document version this module writes and reads.
pub const PREFAB_VERSION: u32 = 1;
/// Most nodes accepted in one prefab.
pub const MAX_PREFAB_NODES: usize = 10_000;
/// Longest accepted prefab key in bytes.
pub const MAX_KEY_LEN: usize = 256;
/// Most components accepted on one prefab node.
pub const MAX_COMPONENTS_PER_NODE: usize = 16;
/// Mint retries on the (practically impossible) id collision.
const MAX_MINT_RETRIES: u32 = 8;

/// v1 museum fixture: a three-node hall chain (hall, plinth, bust) with
/// labels, mirroring the scene codec's v1 chain. Nodes are listed
/// roots-first here, but loading accepts any order.
pub const MUSEUM_V1_JSON: &str = r#"{"schema_version":1,"nodes":[{"key":"hall","parent":null,"transform":{"position":[0.0,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},"components":[{"component":"label","version":1,"payload":"Grand Hall"}]},{"key":"hall/plinth","parent":"hall","transform":{"position":[0.0,1.0,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},"components":[{"component":"label","version":1,"payload":"Plinth"}]},{"key":"hall/plinth/bust","parent":"hall/plinth","transform":{"position":[0.0,0.6,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]},"components":[{"component":"label","version":1,"payload":"Bust of the Founder"}]}]}"#;

/// One authored prefab node: a stable key, a parent link, a local pose and
/// versioned components. A `None` parent means a root.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabNode {
    /// Authoring-stable identity, e.g. `"hall/plinth"`.
    pub key: String,
    /// Parent key, or `None` for a root.
    pub parent: Option<String>,
    /// Pose relative to the parent (or to the world for roots).
    pub transform: Transform,
    /// Components at their authored versions; migrated on instantiate.
    pub components: Vec<VersionedComponent>,
}

/// An authored hierarchy kit: a document version plus one entry per node.
///
/// The JSON text form is `{"schema_version":1,"nodes":[...]}`. Decode
/// accepts nodes in any order; instantiate always inserts parents-first
/// with siblings sorted by key.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Prefab {
    /// Prefab document version; see [`PREFAB_VERSION`].
    pub schema_version: u32,
    /// One entry per node, in any order.
    pub nodes: Vec<PrefabNode>,
}

/// What went wrong validating or instantiating a prefab.
///
/// Every variant names the key (and versions where they apply) so template
/// authors can quote the refusal in diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefabError {
    /// A component or the document itself hit the schema layer (unknown
    /// version, failed migration, malformed value or descriptor).
    Schema(SchemaError),
    /// A prefab key (or parent key) was empty, overlong or held NUL.
    BadKey {
        /// Static reason; the offending key is never echoed raw.
        reason: &'static str,
    },
    /// Two entries share one key.
    DuplicateKey {
        /// The repeated key.
        key: String,
    },
    /// A parent key names no node in the same prefab. Removing a prefab
    /// node requires removing or re-keying its subtree; the loader never
    /// guesses where an orphaned child attaches.
    MissingParent {
        /// The node holding the dangling link.
        key: String,
        /// The parent key no entry provides.
        parent: String,
    },
    /// Parent keys form a loop, so no insertion order can satisfy them.
    Cycle {
        /// A key stuck in the loop (smallest of the stuck set).
        key: String,
    },
    /// A prefab pose holds NaN or infinity.
    NonFinite {
        /// The node holding the bad pose.
        key: String,
    },
    /// One node lists the same component twice.
    DuplicateComponent {
        /// The node holding the doubled component.
        key: String,
        /// The repeated component.
        component: String,
    },
    /// One node lists more than [`MAX_COMPONENTS_PER_NODE`] components.
    TooManyComponents {
        /// The node holding the excess components.
        key: String,
        /// How many components it listed.
        count: usize,
    },
    /// The prefab lists more than [`MAX_PREFAB_NODES`] nodes.
    TooManyNodes {
        /// How many nodes it listed.
        count: usize,
    },
    /// A hierarchy operation failed after validation (mint collision past
    /// the retry bound, or a degenerate scale on a reparent during
    /// re-instantiation). Re-instantiation aborts at the first such error
    /// with earlier nodes already applied; validate first to fail fast.
    Hierarchy(HierarchyError),
}

impl fmt::Display for PrefabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "prefab schema: {error}"),
            Self::BadKey { reason } => write!(f, "invalid prefab key: {reason}"),
            Self::DuplicateKey { key } => write!(f, "duplicate prefab key {key}"),
            Self::MissingParent { key, parent } => write!(
                f,
                "prefab node {key} names unknown parent {parent}; \
                 the parent entry is missing from the prefab"
            ),
            Self::Cycle { key } => {
                write!(f, "prefab parent keys form a cycle at node {key}")
            }
            Self::NonFinite { key } => {
                write!(f, "prefab node {key} holds a non-finite transform")
            }
            Self::DuplicateComponent { key, component } => {
                write!(f, "prefab node {key} lists component {component} twice")
            }
            Self::TooManyComponents { key, count } => write!(
                f,
                "prefab node {key} lists {count} components \
                 (at most {MAX_COMPONENTS_PER_NODE})"
            ),
            Self::TooManyNodes { count } => {
                write!(f, "prefab lists {count} nodes (at most {MAX_PREFAB_NODES})")
            }
            Self::Hierarchy(error) => write!(f, "prefab hierarchy: {error}"),
        }
    }
}

impl std::error::Error for PrefabError {}

impl From<SchemaError> for PrefabError {
    fn from(error: SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<HierarchyError> for PrefabError {
    fn from(error: HierarchyError) -> Self {
        Self::Hierarchy(error)
    }
}

/// Migrated components of one instantiation, keyed by instance id.
///
/// Entries sort by instance id, so iteration is deterministic for a given
/// set of ids. Each node's components sort by component name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentSet {
    entries: Vec<(ObjectId, Vec<VersionedComponent>)>,
}

impl ComponentSet {
    /// Components of one instance, if the id was minted by this prefab.
    pub fn get(&self, id: ObjectId) -> Option<&[VersionedComponent]> {
        self.entries
            .binary_search_by_key(&id.as_uuid(), |(id, _)| id.as_uuid())
            .ok()
            .map(|index| self.entries[index].1.as_slice())
    }

    /// Entries in instance-id order.
    pub fn iter(&self) -> impl Iterator<Item = (ObjectId, &[VersionedComponent])> {
        self.entries
            .iter()
            .map(|(id, components)| (*id, components.as_slice()))
    }

    /// Number of instances carrying components.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no instance carries components.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The receipt of one instantiation: prefab keys to minted instance ids
/// plus the migrated components.
///
/// `map` is keyed by prefab key (`BTreeMap`), so iteration is in key order
/// whatever order the document listed its nodes in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instantiation {
    /// Prefab key to minted instance id.
    pub map: BTreeMap<String, ObjectId>,
    /// Migrated components by instance id.
    pub components: ComponentSet,
}

impl Instantiation {
    /// Instance id minted for a prefab key, if that key instantiated.
    pub fn get(&self, key: &str) -> Option<ObjectId> {
        self.map.get(key).copied()
    }

    /// Order-free FNV-1a 64 of the instance as authored, by prefab key: key,
    /// parent key, local pose bits and components, in key order. Minted ids
    /// never enter, so two instantiations of the same state digest alike.
    /// Keys missing from `tree` are skipped.
    pub fn digest(&self, tree: &Hierarchy) -> u64 {
        let key_of: BTreeMap<ObjectId, &str> = self
            .map
            .iter()
            .map(|(key, id)| (*id, key.as_str()))
            .collect();
        let len = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        let mut hash = Fnv64::new();
        hash.write_str("e2prefab1");
        for (key, id) in &self.map {
            let Some(local) = tree.local(*id) else {
                continue;
            };
            let parent = tree.parent(*id).and_then(|p| key_of.get(&p).copied());
            for text in [key.as_str(), parent.unwrap_or("")] {
                hash.write_u64(len(text.len()));
                hash.write_str(text);
            }
            for v in local
                .position
                .into_iter()
                .chain(local.rotation)
                .chain(local.scale)
            {
                hash.write_f32(v);
            }
            let components = self.components.get(*id).unwrap_or(&[]);
            hash.write_u64(len(components.len()));
            for c in components {
                hash.write_u64(len(c.component.len()));
                hash.write_str(&c.component);
                hash.write_u32(c.version);
                hash.write_u64(len(c.payload.len()));
                hash.write_str(&c.payload);
            }
        }
        hash.finish()
    }
}

/// One validated node ready to insert: links resolved to keys, components
/// migrated to current and sorted by component name.
struct PlannedNode<'a> {
    key: &'a str,
    parent_key: Option<&'a str>,
    transform: Transform,
    components: Vec<VersionedComponent>,
}

impl Prefab {
    /// Validate the document against the registry without touching a tree.
    ///
    /// Runs the full planning pass (version gate, keys, links, poses,
    /// component migration) and discards the plan. Any failure leaves the
    /// caller's tree untouched, since validation never takes one.
    pub fn validate(&self, registry: &SchemaRegistry) -> Result<(), PrefabError> {
        plan(self, registry).map(|_| ())
    }

    /// Instantiate the prefab into a tree, minting a fresh stable id per
    /// node and migrating every component to current.
    ///
    /// Nodes insert parents-first with siblings sorted by key, so the same
    /// document always yields the same links and poses. Two instantiations
    /// of the same prefab hold disjoint id sets with identical structure.
    /// Any validation failure returns before inserting anything.
    pub fn instantiate(
        &self,
        tree: &mut Hierarchy,
        registry: &SchemaRegistry,
    ) -> Result<Instantiation, PrefabError> {
        let planned = plan(self, registry)?;
        let mut map = BTreeMap::new();
        let mut components = Vec::with_capacity(planned.len());
        for node in &planned {
            let parent_id = match node.parent_key {
                None => ObjectId::NIL,
                // Planning lists parents first, so the parent instance is
                // always minted by the time a child needs it; a missing
                // entry means the plan regressed, so refuse loudly.
                Some(parent) => *map.get(parent).ok_or_else(|| PrefabError::MissingParent {
                    key: node.key.to_owned(),
                    parent: parent.to_owned(),
                })?,
            };
            let id = mint_into(tree, parent_id, node.transform)?;
            map.insert(node.key.to_owned(), id);
            components.push((id, node.components.clone()));
        }
        components.sort_by_key(|(id, _)| id.as_uuid());
        Ok(Instantiation {
            map,
            components: ComponentSet {
                entries: components,
            },
        })
    }

    /// Re-instantiate after a prefab edit, keeping prior ids by key.
    ///
    /// Keys present in both the new prefab and `prior` keep their instance
    /// ids (moved nodes keep the id too: links and poses restore to the
    /// new prefab); new keys mint fresh ids; vanished keys drop their
    /// instance nodes. Instance nodes deleted outside are re-created with
    /// fresh ids. Components rebuild at current versions throughout.
    ///
    /// The validation pass runs first and mutates nothing; hierarchy
    /// errors mid-apply (a degenerate scale on a reparent, or a reparent
    /// onto a user-moved descendant) abort with earlier nodes applied.
    pub fn reinstantiate(
        &self,
        tree: &mut Hierarchy,
        registry: &SchemaRegistry,
        prior: &mut Instantiation,
    ) -> Result<(), PrefabError> {
        let planned = plan(self, registry)?;
        let planned_keys: BTreeSet<&str> = planned.iter().map(|node| node.key).collect();

        // Drop vanished keys in sorted order. Removal relinks surviving
        // children upward; the apply pass below restores their links.
        let mut vanished: Vec<String> = prior
            .map
            .keys()
            .filter(|key| !planned_keys.contains(key.as_str()))
            .cloned()
            .collect();
        vanished.sort();
        for key in vanished {
            let Some(id) = prior.map.remove(&key) else {
                return Err(PrefabError::Hierarchy(HierarchyError::UnknownNode));
            };
            if tree.contains(id) {
                tree.remove(id)?;
            }
        }

        // Apply parents-first so every parent instance exists on use.
        let mut components = Vec::with_capacity(planned.len());
        for node in &planned {
            let parent_id = match node.parent_key {
                None => ObjectId::NIL,
                // Parents apply first, so their instances are always known;
                // a missing entry means the plan regressed, so refuse.
                Some(parent) => {
                    *prior
                        .map
                        .get(parent)
                        .ok_or_else(|| PrefabError::MissingParent {
                            key: node.key.to_owned(),
                            parent: parent.to_owned(),
                        })?
                }
            };
            let id = match prior.map.get(node.key) {
                Some(&id) if tree.contains(id) => {
                    if tree.parent(id) != Some(parent_id) {
                        tree.reparent(id, parent_id)?;
                    }
                    tree.set_local(id, node.transform)?;
                    id
                }
                _ => {
                    let id = mint_into(tree, parent_id, node.transform)?;
                    prior.map.insert(node.key.to_owned(), id);
                    id
                }
            };
            components.push((id, node.components.clone()));
        }
        components.sort_by_key(|(id, _)| id.as_uuid());
        prior.components = ComponentSet {
            entries: components,
        };
        Ok(())
    }
}

/// Per-instance override deltas over frozen `reinstantiate`, keyed by prefab key.
/// Consumer: editor reapplies after each `reinstantiate`; units: keys plus poses.
/// Determinism: `BTreeMap` order; errors: [`PrefabError`] on bad keys or poses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverrideMap {
    deltas: BTreeMap<String, OverrideDelta>,
}

/// One key's delta: an optional pose plus optional components (`None` follows prefab).
#[derive(Clone, Debug, Default, PartialEq)]
struct OverrideDelta {
    transform: Option<Transform>,
    components: Option<Vec<VersionedComponent>>,
}

impl OverrideMap {
    /// Number of overridden keys.
    pub fn len(&self) -> usize {
        self.deltas.len()
    }

    /// True when no key carries an override.
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    /// Pin a key's pose over the prefab underneath; errors: `BadKey`, `NonFinite`.
    pub fn set_transform(&mut self, key: String, transform: Transform) -> Result<(), PrefabError> {
        check_key(&key)?;
        if !transform.is_finite() {
            return Err(PrefabError::NonFinite { key });
        }
        self.deltas.entry(key).or_default().transform = Some(transform);
        Ok(())
    }

    /// Pin a key's components sorted by name; errors: `BadKey`, `TooMany`, `Duplicate`.
    pub fn set_components(
        &mut self,
        key: String,
        components: Vec<VersionedComponent>,
    ) -> Result<(), PrefabError> {
        check_key(&key)?;
        let mut sorted = components;
        if sorted.len() > MAX_COMPONENTS_PER_NODE {
            return Err(PrefabError::TooManyComponents {
                key,
                count: sorted.len(),
            });
        }
        sorted.sort_by(|a, b| a.component.cmp(&b.component));
        let mut seen = BTreeSet::new();
        for component in &sorted {
            if !seen.insert(component.component.as_str()) {
                return Err(PrefabError::DuplicateComponent {
                    key,
                    component: component.component.clone(),
                });
            }
        }
        self.deltas.entry(key).or_default().components = Some(sorted);
        Ok(())
    }

    /// Reapply deltas over `reinstantiate` output; missing keys kept, never lost.
    /// Non-overridden keys keep the prefab pose; errors: hierarchy `set_local`.
    pub fn apply(
        &self,
        tree: &mut Hierarchy,
        instantiation: &mut Instantiation,
    ) -> Result<(), PrefabError> {
        if self.is_empty() {
            return Ok(());
        }
        for (key, delta) in &self.deltas {
            let Some(id) = instantiation.map.get(key).copied() else {
                continue;
            };
            if !tree.contains(id) {
                continue;
            }
            if let Some(transform) = delta.transform {
                tree.set_local(id, transform)?;
            }
            if let Some(components) = delta.components.clone() {
                let entries = &mut instantiation.components.entries;
                match entries.binary_search_by_key(&id.as_uuid(), |(id, _)| id.as_uuid()) {
                    Ok(index) => entries[index].1 = components,
                    Err(_) => {
                        entries.push((id, components));
                        entries.sort_by_key(|(id, _)| id.as_uuid());
                    }
                }
            }
        }
        Ok(())
    }
}

impl Prefab {
    /// Frozen `reinstantiate` then overrides; underneath edits show on plain keys.
    /// Overridden keys keep deltas; errors: as in `reinstantiate`/`apply`.
    pub fn reinstantiate_with_overrides(
        &self,
        tree: &mut Hierarchy,
        registry: &SchemaRegistry,
        prior: &mut Instantiation,
        overrides: &OverrideMap,
    ) -> Result<(), PrefabError> {
        self.reinstantiate(tree, registry, prior)?;
        overrides.apply(tree, prior)
    }
}

/// Validate a prefab key: non-empty, bounded, and free of NUL (which would
/// poison path-style keys like `"hall/plinth"`).
fn check_key(key: &str) -> Result<(), PrefabError> {
    if key.is_empty() {
        return Err(PrefabError::BadKey {
            reason: "prefab key must not be empty",
        });
    }
    if key.len() > MAX_KEY_LEN {
        return Err(PrefabError::BadKey {
            reason: "prefab key exceeds the key bound",
        });
    }
    if key.contains('\0') {
        return Err(PrefabError::BadKey {
            reason: "prefab key must not hold NUL",
        });
    }
    Ok(())
}

/// Per-node and link checks over the key-sorted nodes: keys, duplicates,
/// poses and component bounds, then that every named parent exists.
fn check_nodes(ordered: &[&PrefabNode]) -> Result<(), PrefabError> {
    let mut known: BTreeMap<&str, &PrefabNode> = BTreeMap::new();
    for node in ordered {
        check_key(&node.key)?;
        if known.contains_key(node.key.as_str()) {
            return Err(PrefabError::DuplicateKey {
                key: node.key.clone(),
            });
        }
        if let Some(parent) = &node.parent {
            check_key(parent)?;
        }
        if !node.transform.is_finite() {
            return Err(PrefabError::NonFinite {
                key: node.key.clone(),
            });
        }
        if node.components.len() > MAX_COMPONENTS_PER_NODE {
            return Err(PrefabError::TooManyComponents {
                key: node.key.clone(),
                count: node.components.len(),
            });
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for component in &node.components {
            if !seen.insert(component.component.as_str()) {
                return Err(PrefabError::DuplicateComponent {
                    key: node.key.clone(),
                    component: component.component.clone(),
                });
            }
        }
        known.insert(node.key.as_str(), node);
    }
    for node in ordered {
        if let Some(parent) = &node.parent {
            if !known.contains_key(parent.as_str()) {
                return Err(PrefabError::MissingParent {
                    key: node.key.clone(),
                    parent: parent.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Migrate one node's components to current in sorted name order, so the
/// output never depends on how the document listed them.
fn migrate_sorted(
    node: &PrefabNode,
    registry: &SchemaRegistry,
) -> Result<Vec<VersionedComponent>, PrefabError> {
    let mut migrated: Vec<VersionedComponent> = Vec::with_capacity(node.components.len());
    let mut by_name: Vec<&VersionedComponent> = node.components.iter().collect();
    by_name.sort_by(|a, b| a.component.cmp(&b.component));
    for component in by_name {
        migrated.push(
            registry
                .migrate_value(component)
                .map_err(PrefabError::Schema)?,
        );
    }
    Ok(migrated)
}

/// Full planning pass: the version gate, key/link/pose checks and
/// component migration, returning nodes in deterministic insertion order
/// (parents before children, siblings sorted by key).
fn plan<'a>(
    prefab: &'a Prefab,
    registry: &SchemaRegistry,
) -> Result<Vec<PlannedNode<'a>>, PrefabError> {
    if prefab.schema_version != PREFAB_VERSION {
        return Err(SchemaError::UnknownVersion {
            component: "prefab".to_string(),
            got: prefab.schema_version,
            current: PREFAB_VERSION,
        }
        .into());
    }
    if prefab.nodes.len() > MAX_PREFAB_NODES {
        return Err(PrefabError::TooManyNodes {
            count: prefab.nodes.len(),
        });
    }

    // Sort first so every reported culprit is deterministic for a given
    // set of entries, whatever order the document listed them in.
    let mut ordered: Vec<&PrefabNode> = prefab.nodes.iter().collect();
    ordered.sort_by(|a, b| a.key.cmp(&b.key));

    check_nodes(&ordered)?;

    // Insert parents before children. A pass with no progress and nodes
    // left means the remaining keys loop, since every named parent is
    // known to exist from the check above.
    let mut planned: Vec<PlannedNode<'a>> = Vec::with_capacity(ordered.len());
    let mut done: BTreeSet<&str> = BTreeSet::new();
    while done.len() < ordered.len() {
        let mut progressed = false;
        for node in &ordered {
            if done.contains(node.key.as_str()) {
                continue;
            }
            let ready = match &node.parent {
                None => true,
                Some(parent) => done.contains(parent.as_str()),
            };
            if !ready {
                continue;
            }
            planned.push(PlannedNode {
                key: node.key.as_str(),
                parent_key: node.parent.as_deref(),
                transform: node.transform,
                components: migrate_sorted(node, registry)?,
            });
            done.insert(node.key.as_str());
            progressed = true;
        }
        if !progressed {
            let mut stuck: Vec<&&PrefabNode> = ordered
                .iter()
                .filter(|node| !done.contains(node.key.as_str()))
                .collect();
            stuck.sort_by(|a, b| a.key.cmp(&b.key));
            let culprit = stuck.first().expect("loop exits only with nodes left");
            return Err(PrefabError::Cycle {
                key: culprit.key.clone(),
            });
        }
    }
    Ok(planned)
}

/// Insert one instance node with a fresh id, retrying the mint on the
/// (practically impossible) collision. Only a collision retries; any other
/// insert failure is unreachable after planning and returns as an error.
fn mint_into(
    tree: &mut Hierarchy,
    parent: ObjectId,
    local: Transform,
) -> Result<ObjectId, PrefabError> {
    for _ in 0..MAX_MINT_RETRIES {
        let id = ObjectId::new();
        match tree.insert(id, parent, local) {
            Ok(()) => return Ok(id),
            Err(HierarchyError::Duplicate) => continue,
            Err(other) => return Err(PrefabError::Hierarchy(other)),
        }
    }
    Err(PrefabError::Hierarchy(HierarchyError::Duplicate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(position: [f32; 3]) -> Transform {
        Transform::new(position, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn label(text: &str) -> VersionedComponent {
        VersionedComponent {
            component: "label".to_string(),
            version: 1,
            payload: text.to_string(),
        }
    }

    fn node(key: &str, parent: Option<&str>, position: [f32; 3]) -> PrefabNode {
        PrefabNode {
            key: key.to_string(),
            parent: parent.map(str::to_string),
            transform: pose(position),
            components: vec![label(key)],
        }
    }

    /// Hall chain mirroring the museum: hall, plinth, bust.
    fn hall_prefab() -> Prefab {
        Prefab {
            schema_version: PREFAB_VERSION,
            nodes: vec![
                node("hall", None, [0.0, 0.0, 0.0]),
                node("hall/plinth", Some("hall"), [0.0, 1.0, 0.0]),
                node("hall/plinth/bust", Some("hall/plinth"), [0.0, 0.6, 0.0]),
            ],
        }
    }

    /// Key to (parent key, local pose, labels) for structure comparison
    /// across instantiations, whose minted ids differ by design.
    fn structure(
        prefab: &Prefab,
        tree: &Hierarchy,
        instantiation: &Instantiation,
    ) -> BTreeMap<String, (Option<String>, Transform, Vec<String>)> {
        let mut out = BTreeMap::new();
        for doc in &prefab.nodes {
            let id = instantiation.get(&doc.key).expect("key instantiated");
            let parent_key = match tree.parent(id).expect("instance in tree") {
                parent if parent.is_nil() => None,
                parent => instantiation
                    .map
                    .iter()
                    .find(|(_, candidate)| **candidate == parent)
                    .map(|(key, _)| key.clone()),
            };
            let labels: Vec<String> = instantiation
                .components
                .get(id)
                .expect("instance carries components")
                .iter()
                .map(|component| component.payload.clone())
                .collect();
            out.insert(
                doc.key.clone(),
                (parent_key, tree.local(id).unwrap(), labels),
            );
        }
        out
    }

    #[test]
    fn instantiate_roundtrips_links_poses_and_labels() {
        let prefab = hall_prefab();
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let instantiation = prefab.instantiate(&mut tree, &registry).unwrap();
        assert_eq!(instantiation.map.len(), 3);
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.validate(), Ok(()));

        let hall = instantiation.get("hall").unwrap();
        let plinth = instantiation.get("hall/plinth").unwrap();
        let bust = instantiation.get("hall/plinth/bust").unwrap();
        assert_ne!(hall, plinth);
        assert_ne!(plinth, bust);
        assert_eq!(tree.parent(hall), Some(ObjectId::NIL));
        assert_eq!(tree.parent(plinth), Some(hall));
        assert_eq!(tree.parent(bust), Some(plinth));
        assert_eq!(tree.local(plinth).unwrap(), pose([0.0, 1.0, 0.0]));

        // World poses compose down the chain.
        let world = tree.world(bust).unwrap();
        assert!((world.position[1] - 1.6).abs() <= 1e-4);

        // Labels arrive migrated to current with their payloads intact.
        let labels = instantiation.components.get(bust).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].component, "label");
        assert_eq!(labels[0].version, 1);
        assert_eq!(labels[0].payload, "hall/plinth/bust");
        assert_eq!(instantiation.components.len(), 3);
    }

    #[test]
    fn instantiate_order_is_deterministic_across_listings() {
        let registry = SchemaRegistry::builtin();
        let first = hall_prefab();
        let mut shuffled = hall_prefab();
        shuffled.nodes.reverse();

        let mut tree_a = Hierarchy::new();
        let inst_a = first.instantiate(&mut tree_a, &registry).unwrap();
        let mut tree_b = Hierarchy::new();
        let inst_b = shuffled.instantiate(&mut tree_b, &registry).unwrap();
        // Same links, poses and labels; only the minted ids differ.
        assert_eq!(
            structure(&first, &tree_a, &inst_a),
            structure(&shuffled, &tree_b, &inst_b)
        );
    }

    #[test]
    fn two_copies_mint_distinct_ids_with_identical_structure() {
        let prefab = hall_prefab();
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let first = prefab.instantiate(&mut tree, &registry).unwrap();
        let second = prefab.instantiate(&mut tree, &registry).unwrap();
        assert_eq!(tree.len(), 6);

        let first_ids: BTreeSet<ObjectId> = first.map.values().copied().collect();
        let second_ids: BTreeSet<ObjectId> = second.map.values().copied().collect();
        assert!(first_ids.is_disjoint(&second_ids));
        assert_eq!(
            structure(&prefab, &tree, &first),
            structure(&prefab, &tree, &second)
        );
    }

    #[test]
    fn reinstantiate_keeps_ids_where_keys_match() {
        let registry = SchemaRegistry::builtin();
        let mut prefab = hall_prefab();
        let mut tree = Hierarchy::new();
        let mut prior = prefab.instantiate(&mut tree, &registry).unwrap();
        let ids_before = prior.map.clone();

        // Same keys, new pose: ids stay, poses update in place.
        prefab.nodes[1].transform = pose([0.0, 2.0, 0.0]);
        prefab
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        assert_eq!(prior.map, ids_before);
        let plinth = prior.get("hall/plinth").unwrap();
        assert_eq!(tree.local(plinth).unwrap(), pose([0.0, 2.0, 0.0]));
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn reinstantiate_after_adding_a_node_keeps_old_ids() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let hall_before = prior.get("hall").unwrap();

        let mut edited = hall_prefab();
        edited
            .nodes
            .push(node("hall/bank", Some("hall"), [3.0, 0.0, 0.0]));
        edited
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        assert_eq!(prior.get("hall"), Some(hall_before));
        assert_eq!(prior.map.len(), 4);
        assert_eq!(tree.len(), 4);
        let bank = prior.get("hall/bank").unwrap();
        assert_eq!(tree.parent(bank), Some(hall_before));
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn reinstantiate_after_removing_a_leaf_drops_its_id() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let bust = prior.get("hall/plinth/bust").unwrap();

        let mut edited = hall_prefab();
        edited.nodes.pop();
        edited
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        assert_eq!(prior.map.len(), 2);
        assert!(!tree.contains(bust));
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn reinstantiate_keeps_moved_node_ids_and_restores_links() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let bust_before = prior.get("hall/plinth/bust").unwrap();

        // Move the bust directly under the hall: the id persists.
        let mut edited = hall_prefab();
        edited.nodes[2].parent = Some("hall".to_string());
        edited
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        assert_eq!(prior.get("hall/plinth/bust"), Some(bust_before));
        assert_eq!(tree.parent(bust_before), Some(prior.get("hall").unwrap()));
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn reinstantiate_recreates_externally_deleted_nodes() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let plinth_before = prior.get("hall/plinth").unwrap();
        tree.remove(plinth_before).unwrap();
        // The bust relinked to the hall on removal; re-instantiation
        // restores the prefab links with a fresh plinth id.
        hall_prefab()
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        let plinth_after = prior.get("hall/plinth").unwrap();
        assert_ne!(plinth_after, plinth_before);
        assert_eq!(
            tree.parent(prior.get("hall/plinth/bust").unwrap()),
            Some(plinth_after)
        );
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn removing_a_mid_node_with_a_surviving_child_refuses() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();

        // Delete the plinth but keep the bust key: the bust's parent key
        // dangles, and the loader refuses instead of guessing a new slot.
        let mut edited = hall_prefab();
        edited.nodes.remove(1);
        let error = edited
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap_err();
        assert_eq!(
            error,
            PrefabError::MissingParent {
                key: "hall/plinth/bust".to_string(),
                parent: "hall/plinth".to_string(),
            }
        );
        // Nothing applied: the old instantiation still stands whole.
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn unknown_document_versions_refuse_loudly() {
        let registry = SchemaRegistry::builtin();
        for bad in [0, PREFAB_VERSION + 1, u32::MAX] {
            let mut prefab = hall_prefab();
            prefab.schema_version = bad;
            let mut tree = Hierarchy::new();
            let error = prefab.instantiate(&mut tree, &registry).unwrap_err();
            assert_eq!(
                error,
                PrefabError::Schema(SchemaError::UnknownVersion {
                    component: "prefab".to_string(),
                    got: bad,
                    current: PREFAB_VERSION,
                })
            );
            assert!(error.to_string().contains(&bad.to_string()));
            // Nothing inserted on refusal.
            assert!(tree.is_empty());
            // Validation refuses the same way without a tree.
            assert_eq!(prefab.validate(&registry).unwrap_err(), error);
        }
    }

    #[test]
    fn malformed_prefabs_name_their_culprit() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();

        let mut doubled = hall_prefab();
        doubled.nodes.push(node("hall", None, [9.0, 9.0, 9.0]));
        assert_eq!(
            doubled.instantiate(&mut tree, &registry).unwrap_err(),
            PrefabError::DuplicateKey {
                key: "hall".to_string()
            }
        );

        let mut orphaned = hall_prefab();
        orphaned
            .nodes
            .push(node("ghost", Some("missing"), [0.0, 0.0, 0.0]));
        assert_eq!(
            orphaned.instantiate(&mut tree, &registry).unwrap_err(),
            PrefabError::MissingParent {
                key: "ghost".to_string(),
                parent: "missing".to_string(),
            }
        );

        let looping = Prefab {
            schema_version: PREFAB_VERSION,
            nodes: vec![
                node("a", Some("b"), [0.0, 0.0, 0.0]),
                node("b", Some("a"), [0.0, 0.0, 0.0]),
            ],
        };
        assert_eq!(
            looping.instantiate(&mut tree, &registry).unwrap_err(),
            PrefabError::Cycle {
                key: "a".to_string()
            }
        );

        let mut unfinite = hall_prefab();
        unfinite.nodes[0].transform =
            Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert_eq!(
            unfinite.instantiate(&mut tree, &registry).unwrap_err(),
            PrefabError::NonFinite {
                key: "hall".to_string()
            }
        );

        let mut keyless = hall_prefab();
        keyless.nodes[0].key.clear();
        assert!(matches!(
            keyless.instantiate(&mut tree, &registry).unwrap_err(),
            PrefabError::BadKey { .. }
        ));

        let mut doubled_component = hall_prefab();
        doubled_component.nodes[0].components.push(label("again"));
        assert_eq!(
            doubled_component
                .instantiate(&mut tree, &registry)
                .unwrap_err(),
            PrefabError::DuplicateComponent {
                key: "hall".to_string(),
                component: "label".to_string(),
            }
        );

        // Nothing partial: every refusal above left the tree empty.
        assert!(tree.is_empty());
    }

    #[test]
    fn empty_prefab_instantiates_empty() {
        let registry = SchemaRegistry::builtin();
        let prefab = Prefab {
            schema_version: PREFAB_VERSION,
            nodes: Vec::new(),
        };
        let mut tree = Hierarchy::new();
        let instantiation = prefab.instantiate(&mut tree, &registry).unwrap();
        assert!(instantiation.map.is_empty());
        assert!(instantiation.components.is_empty());
        assert!(tree.is_empty());
    }

    #[test]
    fn museum_v1_loads_with_hall_plinth_and_bust() {
        // The museum fixture is fixed bytes: parse, validate, instantiate.
        let prefab: Prefab = serde_json::from_str(MUSEUM_V1_JSON).unwrap();
        assert_eq!(prefab.schema_version, PREFAB_VERSION);
        assert_eq!(prefab.nodes.len(), 3);
        let registry = SchemaRegistry::builtin();
        prefab.validate(&registry).unwrap();

        let mut tree = Hierarchy::new();
        let instantiation = prefab.instantiate(&mut tree, &registry).unwrap();
        assert_eq!(tree.len(), 3);
        let hall = instantiation.get("hall").unwrap();
        let plinth = instantiation.get("hall/plinth").unwrap();
        let bust = instantiation.get("hall/plinth/bust").unwrap();
        assert_eq!(tree.parent(hall), Some(ObjectId::NIL));
        assert_eq!(tree.parent(plinth), Some(hall));
        assert_eq!(tree.parent(bust), Some(plinth));

        let world = tree.world(bust).unwrap();
        assert!((world.position[0] - 0.0).abs() <= 1e-4);
        assert!((world.position[1] - 1.6).abs() <= 1e-4);
        assert!((world.position[2] - 0.0).abs() <= 1e-4);

        let bust_labels = instantiation.components.get(bust).unwrap();
        assert_eq!(bust_labels.len(), 1);
        assert_eq!(bust_labels[0].component, "label");
        assert_eq!(bust_labels[0].version, 1);
        assert_eq!(bust_labels[0].payload, "Bust of the Founder");

        // Loading twice gives the same document bytes back: the fixture is
        // byte-stable, only the minted ids differ.
        let again: Prefab = serde_json::from_str(MUSEUM_V1_JSON).unwrap();
        assert_eq!(again, prefab);
    }

    #[test]
    fn museum_text_with_a_future_version_refuses() {
        let text = MUSEUM_V1_JSON.replacen("\"schema_version\":1", "\"schema_version\":99", 1);
        assert!(text.contains("\"schema_version\":99"));
        let prefab: Prefab = serde_json::from_str(&text).unwrap();
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let error = prefab.instantiate(&mut tree, &registry).unwrap_err();
        assert_eq!(
            error,
            PrefabError::Schema(SchemaError::UnknownVersion {
                component: "prefab".to_string(),
                got: 99,
                current: PREFAB_VERSION,
            })
        );
        assert!(tree.is_empty());
    }

    #[test]
    fn instantiate_migrates_components_to_current() {
        fn v1_to_v2(payload: &str) -> Result<String, SchemaError> {
            let _ = payload;
            Ok("migrated".to_string())
        }
        let mut registry = SchemaRegistry::builtin();
        registry
            .register(super::super::schema::Schema {
                component: "sigil",
                version: 1,
                migrate: Some(v1_to_v2),
            })
            .unwrap();
        registry
            .register(super::super::schema::Schema {
                component: "sigil",
                version: 2,
                migrate: None,
            })
            .unwrap();

        let mut prefab = hall_prefab();
        prefab.nodes[0].components.push(VersionedComponent {
            component: "sigil".to_string(),
            version: 1,
            payload: "old".to_string(),
        });
        let mut tree = Hierarchy::new();
        let instantiation = prefab.instantiate(&mut tree, &registry).unwrap();
        // Labels stay sorted by component name: label before sigil.
        let hall = instantiation.get("hall").unwrap();
        let components = instantiation.components.get(hall).unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0].component, "label");
        assert_eq!(components[1].component, "sigil");
        assert_eq!(components[1].version, 2);
        assert_eq!(components[1].payload, "migrated");
    }

    #[test]
    fn dangling_links_fail_typed_on_both_apply_paths() {
        let registry = SchemaRegistry::builtin();
        // Instantiate refuses with both ends named, leaving nothing behind.
        let mut orphaned = hall_prefab();
        orphaned
            .nodes
            .push(node("ghost", Some("missing"), [0.0, 0.0, 0.0]));
        let mut tree = Hierarchy::new();
        let error = orphaned.instantiate(&mut tree, &registry).unwrap_err();
        assert_eq!(
            error,
            PrefabError::MissingParent {
                key: "ghost".to_string(),
                parent: "missing".to_string(),
            }
        );
        let report = error.to_string();
        assert!(report.contains("ghost"));
        assert!(report.contains("missing"));
        assert!(tree.is_empty());

        // Re-instantiation refuses the same way without touching the tree.
        let mut prior_tree = Hierarchy::new();
        let mut prior = hall_prefab()
            .instantiate(&mut prior_tree, &registry)
            .unwrap();
        let len_before = prior_tree.len();
        let error = orphaned
            .reinstantiate(&mut prior_tree, &registry, &mut prior)
            .unwrap_err();
        assert_eq!(
            error,
            PrefabError::MissingParent {
                key: "ghost".to_string(),
                parent: "missing".to_string(),
            }
        );
        assert_eq!(prior_tree.len(), len_before);
        assert_eq!(prior_tree.validate(), Ok(()));
    }

    #[test]
    fn reinstantiate_drops_vanished_without_panicking() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let bust = prior.get("hall/plinth/bust").unwrap();

        // A leaf-only removal drops its id through the vanished-keys path.
        let mut leaf_removed = hall_prefab();
        leaf_removed.nodes.pop();
        leaf_removed
            .reinstantiate(&mut tree, &registry, &mut prior)
            .unwrap();
        assert_eq!(prior.map.len(), 2);
        assert!(!tree.contains(bust));
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn override_deltas_survive_prefab_underneath_edits() {
        let registry = SchemaRegistry::builtin();
        let mut tree = Hierarchy::new();
        let mut prior = hall_prefab().instantiate(&mut tree, &registry).unwrap();
        let mut overrides = OverrideMap::default();
        overrides
            .set_transform("hall/plinth".to_string(), pose([5.0, 0.0, 0.0]))
            .unwrap();
        overrides
            .set_components("hall/plinth/bust".to_string(), vec![label("Kept Bust")])
            .unwrap();
        assert_eq!(overrides.len(), 2);
        assert!(overrides
            .set_transform(String::new(), pose([0.0, 0.0, 0.0]))
            .is_err());
        overrides.apply(&mut tree, &mut prior).unwrap();

        // Underneath: hall moves, plinth moves (overridden), plus new sibling key.
        let mut edited = hall_prefab();
        edited.nodes[0].transform = pose([9.0, 0.0, 0.0]);
        edited.nodes[1].transform = pose([0.0, 2.0, 0.0]);
        edited
            .nodes
            .push(node("hall/bank", Some("hall"), [3.0, 0.0, 0.0]));
        edited
            .reinstantiate_with_overrides(&mut tree, &registry, &mut prior, &overrides)
            .unwrap();

        // 5/5 preserved: two overrides kept, two underneath edits show, new key minted.
        let plinth = prior.get("hall/plinth").unwrap();
        assert_eq!(tree.local(plinth).unwrap(), pose([5.0, 0.0, 0.0]));
        let bust = prior.get("hall/plinth/bust").unwrap();
        assert_eq!(prior.components.get(bust).unwrap()[0].payload, "Kept Bust");
        let hall = prior.get("hall").unwrap();
        assert_eq!(tree.local(hall).unwrap(), pose([9.0, 0.0, 0.0]));
        let bank = prior.get("hall/bank").unwrap();
        assert_eq!(tree.parent(bank), Some(hall));
        assert_eq!(overrides.len(), 2);
    }

    /// Museum run: two overrides, two underneath edits and one new key, twice
    /// from fresh trees (fresh minted ids), digest taken after each.
    fn museum_override_run() -> (u64, u64, u64, u64) {
        let registry = SchemaRegistry::builtin();
        let museum: Prefab = serde_json::from_str(MUSEUM_V1_JSON).unwrap();
        let mut tree = Hierarchy::new();
        let mut prior = museum.instantiate(&mut tree, &registry).unwrap();
        let before = prior.digest(&tree);
        let mut overrides = OverrideMap::default();
        overrides
            .set_transform("hall/plinth".to_string(), pose([5.0, 0.0, 0.0]))
            .unwrap();
        overrides
            .set_components("hall/plinth/bust".to_string(), vec![label("Kept Bust")])
            .unwrap();
        let mut edited = museum;
        edited.nodes[0].transform = pose([9.0, 0.0, 0.0]);
        edited.nodes[1].transform = pose([0.0, 2.0, 0.0]);
        edited
            .nodes
            .push(node("hall/bank", Some("hall"), [3.0, 0.0, 0.0]));
        edited
            .reinstantiate_with_overrides(&mut tree, &registry, &mut prior, &overrides)
            .unwrap();
        let with_overrides = prior.digest(&tree);
        // The same edited prefab with no deltas lands on a different state.
        let mut plain_tree = Hierarchy::new();
        let plain = edited.instantiate(&mut plain_tree, &registry).unwrap();
        let plain_digest = plain.digest(&plain_tree);
        // Fresh edited instance plus the deltas equals the live reinstantiate.
        let mut fresh = plain;
        overrides.apply(&mut plain_tree, &mut fresh).unwrap();
        (
            before,
            with_overrides,
            plain_digest,
            fresh.digest(&plain_tree),
        )
    }

    #[test]
    fn museum_override_digest_is_stable_across_fresh_instantiations() {
        let first = museum_override_run();
        assert_eq!(first, museum_override_run());
        let (before, with_overrides, plain, fresh) = first;
        assert_ne!(before, with_overrides);
        assert_ne!(with_overrides, plain);
        assert_eq!(with_overrides, fresh);
        assert_eq!(with_overrides, 13574963997578328745);
    }
}
