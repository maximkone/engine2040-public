//! kind_table: the LF8 per-mesh shading kind of the presenter, and the RL51
//! per-mesh surface detail beside it.
//!
//! Presentation only, never hashed. A key with no entry is stone with no
//! detail, so a scene that never sets one packs kind word 0 and parameter
//! word 0 and draws the same bytes every earlier run drew. Its own impl block
//! keeps `presenter.rs`, already pinned over 3000 lines, from growing.

use super::super::MeshKey;
use super::presenter::Presenter;
use crate::hextile::SurfaceDetail;
use crate::material_kind::MaterialKind;

/// What the table holds for one mesh: its shading family and, for stone, its
/// surface detail. The default is the plain stone draw, so an entry equal to
/// it is never stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct KindEntry {
    /// The LF8 shading family.
    kind: MaterialKind,
    /// The RL51 detail; only the stone kind reads it.
    detail: SurfaceDetail,
}

impl Presenter {
    /// Choose the shading family of one mesh key: stone (the default), skin,
    /// metal or cloth. The next staged frame writes the kind word and its
    /// parameter into that draw's material image; every other draw keeps
    /// stone's zero words. Stone removes the entry, so the table holds only
    /// the kinds that differ. Caller: the scene owner's per-part table.
    pub fn set_material_kind(&mut self, mesh: MeshKey, kind: MaterialKind) {
        let entry = KindEntry {
            kind,
            ..self.material_kinds.get(&mesh).copied().unwrap_or_default()
        };
        self.store_kind_entry(mesh, entry);
    }

    /// The kind the next frame packs for `mesh`; a key with no entry is stone.
    pub fn material_kind(&self, mesh: MeshKey) -> MaterialKind {
        self.material_kinds
            .get(&mesh)
            .map(|entry| entry.kind)
            .unwrap_or_default()
    }

    /// RL51: choose the surface detail of one stone mesh (hex-tiled taps and a
    /// macro term over the albedo; see `crate::hextile`). The next staged
    /// frame writes it into that draw's parameter word. `SurfaceDetail::None`
    /// removes it; a mesh of any other kind ignores it (those kinds own the
    /// word). Caller: the scene owner's per-part table, the game's kit upload.
    pub fn set_surface_detail(&mut self, mesh: MeshKey, detail: SurfaceDetail) {
        let entry = KindEntry {
            detail,
            ..self.material_kinds.get(&mesh).copied().unwrap_or_default()
        };
        self.store_kind_entry(mesh, entry);
    }

    /// The surface detail the next frame packs for `mesh`; no entry is none.
    pub fn surface_detail(&self, mesh: MeshKey) -> SurfaceDetail {
        self.material_kinds
            .get(&mesh)
            .map(|entry| entry.detail)
            .unwrap_or_default()
    }

    /// The parameter word the next frame packs for `mesh` under `kind`: the
    /// kind's own parameter for skin and metal, the surface detail for stone,
    /// zero for cloth. `kind` is `self.material_kind(mesh)`, passed in because
    /// the staging loops already hold it.
    pub(crate) fn material_param(&self, mesh: MeshKey, kind: MaterialKind) -> f32 {
        match kind {
            MaterialKind::Stone => self.surface_detail(mesh).param(),
            other => other.default_param(),
        }
    }

    fn store_kind_entry(&mut self, mesh: MeshKey, entry: KindEntry) {
        if entry == KindEntry::default() {
            self.material_kinds.remove(&mesh);
        } else {
            self.material_kinds.insert(mesh, entry);
        }
    }
}
