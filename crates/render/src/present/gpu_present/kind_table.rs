//! kind_table: the LF8 per-mesh shading kind of the presenter.
//!
//! Presentation only, never hashed. A key with no entry is stone, so a scene
//! that never sets one packs kind word 0 and draws the same bytes every
//! earlier run drew. Its own impl block keeps `presenter.rs`, already pinned
//! over 3000 lines, from growing.

use super::super::MeshKey;
use super::presenter::Presenter;
use crate::material_kind::MaterialKind;

impl Presenter {
    /// Choose the shading family of one mesh key: stone (the default), skin,
    /// metal or cloth. The next staged frame writes the kind word and its
    /// parameter into that draw's material image; every other draw keeps
    /// stone's zero words. Stone removes the entry, so the table holds only
    /// the kinds that differ. Caller: the scene owner's per-part table.
    pub fn set_material_kind(&mut self, mesh: MeshKey, kind: MaterialKind) {
        if kind == MaterialKind::Stone {
            self.material_kinds.remove(&mesh);
        } else {
            self.material_kinds.insert(mesh, kind);
        }
    }

    /// The kind the next frame packs for `mesh`; a key with no entry is stone.
    pub fn material_kind(&self, mesh: MeshKey) -> MaterialKind {
        self.material_kinds.get(&mesh).copied().unwrap_or_default()
    }
}
