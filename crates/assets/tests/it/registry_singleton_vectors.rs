//! Registry singleton vectors: FindOrCreate holds one instance per id.
//!
//! Owner: engine-assets registry path. Units: (type, id) in, `Arc<Asset>` out.
//! Deterministic: `BTreeMap` order, no RNG, no wall clock.
//! Errors: unregistered type returns `ManagerError::UnknownType`.
//!
//! Discipline idea from o3de/o3de (Apache-2.0)
//! `Code/Framework/AzCore/AzCore/Asset/AssetManager.h` `FindOrCreateAsset`:
//! singleton per (type, id), handler-per-type creation, catalog-per-type
//! storage. Fresh test-local harness; no donor bytes copied.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Asset kinds in this harness; the third variant stays unregistered
/// so the failure case has a type with no handler and no catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AssetType {
    Mesh,
    Texture,
    Material,
}

/// One asset instance: its type plus its numeric id within that type.
#[derive(Debug, PartialEq, Eq)]
struct Asset {
    kind: AssetType,
    id: u64,
}

/// Creation endpoint for one asset type.
trait AssetHandler {
    /// Type this handler creates; must match its registration slot.
    fn kind(&self) -> AssetType;
    /// Build a fresh asset for `id`; called only on catalog miss.
    fn create(&self, id: u64) -> Asset;
    /// How many times creation ran (dispatch proof).
    fn calls(&self) -> usize;
}

/// Counting handler: records how often its type was created.
struct CountingHandler {
    kind: AssetType,
    created: Cell<usize>,
}

impl CountingHandler {
    fn new(kind: AssetType) -> Self {
        Self {
            kind,
            created: Cell::new(0),
        }
    }
}

impl AssetHandler for CountingHandler {
    fn kind(&self) -> AssetType {
        self.kind
    }

    fn create(&self, id: u64) -> Asset {
        self.created.set(self.created.get().saturating_add(1));
        Asset {
            kind: self.kind,
            id,
        }
    }

    fn calls(&self) -> usize {
        self.created.get()
    }
}

/// Rejected lookup: no handler and no catalog for this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagerError {
    UnknownType(AssetType),
}

impl fmt::Display for ManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownType(kind) => write!(f, "unknown asset type {kind:?}"),
        }
    }
}

impl std::error::Error for ManagerError {}

/// FindOrCreate manager: one catalog per type, one handler per type.
/// A hit returns the stored `Arc`; a miss creates once via the
/// type handler, stores it, then returns it.
struct AssetManager {
    catalogs: BTreeMap<AssetType, BTreeMap<u64, Arc<Asset>>>,
    handlers: BTreeMap<AssetType, Box<dyn AssetHandler>>,
}

impl AssetManager {
    fn new() -> Self {
        Self {
            catalogs: BTreeMap::new(),
            handlers: BTreeMap::new(),
        }
    }

    /// Register the handler for its type and open that type catalog.
    fn register(&mut self, handler: Box<dyn AssetHandler>) {
        let kind = handler.kind();
        self.catalogs.entry(kind).or_default();
        self.handlers.insert(kind, handler);
    }

    /// Find-or-create: same (type, id) always returns the same `Arc`.
    fn get_or_create(&mut self, kind: AssetType, id: u64) -> Result<Arc<Asset>, ManagerError> {
        let catalog = self
            .catalogs
            .get_mut(&kind)
            .ok_or(ManagerError::UnknownType(kind))?;
        if let Some(live) = catalog.get(&id) {
            return Ok(Arc::clone(live));
        }
        let handler = self
            .handlers
            .get(&kind)
            .ok_or(ManagerError::UnknownType(kind))?;
        assert_eq!(
            handler.kind(),
            kind,
            "handler slot must match its asset type"
        );
        let live = Arc::new(handler.create(id));
        catalog.insert(id, Arc::clone(&live));
        Ok(live)
    }

    /// Live instances for one type catalog.
    fn live_count(&self, kind: AssetType) -> usize {
        self.catalogs.get(&kind).map_or(0, BTreeMap::len)
    }

    /// Creation calls observed by one type handler.
    fn handler_calls(&self, kind: AssetType) -> usize {
        self.handlers.get(&kind).map_or(0, |h| h.calls())
    }
}

/// Manager with mesh + texture handlers registered.
fn manager_with_two_types() -> AssetManager {
    let mut manager = AssetManager::new();
    manager.register(Box::new(CountingHandler::new(AssetType::Mesh)));
    manager.register(Box::new(CountingHandler::new(AssetType::Texture)));
    manager
}

#[test]
fn repeated_get_same_id_holds_single_instance() {
    let mut manager = manager_with_two_types();
    let first = manager
        .get_or_create(AssetType::Mesh, 42)
        .expect("mesh catalog is registered");
    let second = manager
        .get_or_create(AssetType::Mesh, 42)
        .expect("mesh repeat must hit");
    assert!(
        Arc::ptr_eq(&first, &second),
        "same (type, id) must share one Arc"
    );
    assert_eq!(manager.live_count(AssetType::Mesh), 1);
    assert_eq!(manager.handler_calls(AssetType::Mesh), 1);
}

#[test]
fn handler_dispatch_stays_per_type() {
    let mut manager = manager_with_two_types();
    let mesh_one = manager
        .get_or_create(AssetType::Mesh, 1)
        .expect("mesh catalog is registered");
    let texture_one = manager
        .get_or_create(AssetType::Texture, 1)
        .expect("texture catalog is registered");
    let mesh_two = manager
        .get_or_create(AssetType::Mesh, 2)
        .expect("second mesh id must create");
    assert_eq!(mesh_one.kind, AssetType::Mesh);
    assert_eq!(texture_one.kind, AssetType::Texture);
    assert!(!Arc::ptr_eq(&mesh_one, &texture_one));
    assert!(!Arc::ptr_eq(&mesh_one, &mesh_two));
    assert_eq!(manager.handler_calls(AssetType::Mesh), 2);
    assert_eq!(manager.handler_calls(AssetType::Texture), 1);
    assert_eq!(manager.live_count(AssetType::Mesh), 2);
    assert_eq!(manager.live_count(AssetType::Texture), 1);
}

#[test]
fn catalog_isolation_per_type_and_unknown_type_fails() {
    let mut manager = manager_with_two_types();
    let mesh_seven = manager
        .get_or_create(AssetType::Mesh, 7)
        .expect("mesh catalog is registered");
    let texture_seven = manager
        .get_or_create(AssetType::Texture, 7)
        .expect("texture catalog is registered");
    assert!(
        !Arc::ptr_eq(&mesh_seven, &texture_seven),
        "same numeric id in two catalogs must not alias"
    );
    assert_eq!(manager.live_count(AssetType::Mesh), 1);
    assert_eq!(manager.live_count(AssetType::Texture), 1);
    let err = manager
        .get_or_create(AssetType::Material, 7)
        .expect_err("unregistered type must fail");
    assert_eq!(err, ManagerError::UnknownType(AssetType::Material));
    assert_eq!(
        err.to_string(),
        "unknown asset type Material",
        "typed error must print"
    );
    assert_eq!(manager.live_count(AssetType::Material), 0);
    assert_eq!(manager.handler_calls(AssetType::Material), 0);
}
