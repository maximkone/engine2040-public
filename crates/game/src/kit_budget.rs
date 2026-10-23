//! GS15: the imported kit's measured mesh cost against the potato caps,
//! with the files and meshes that exceed one. `app` leg only.

use engine_assets::budget::{check, MeshStats, POTATO};

/// One imported mesh over a potato cap: the asset identity, the cap and
/// both numbers. Mirrors [`engine_assets::budget::BudgetFinding`] plus the
/// file and mesh it came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KitBudgetFinding {
    /// Kit file the mesh came from.
    pub file: &'static str,
    /// Mesh index inside that file (document order).
    pub mesh: usize,
    /// Cap name: `tris`, `bones`, `influences`, `materials` or `tex_px`.
    pub kind: &'static str,
    /// Measured value.
    pub actual: u32,
    /// Potato allowance.
    pub allowed: u32,
}

/// How many findings one [`KitBudget::line`] spells out.
const KIT_BUDGET_SHOWN: usize = 4;

/// Measured mesh cost of one kit import against [`engine_assets::budget::POTATO`].
///
/// The kit draws inside the potato arena run, so the weakest tier's caps
/// judge every imported mesh. Triangles, bones, influences and materials
/// are per mesh; a static kit mesh carries no skin, so bones and influences
/// read 0 and a mesh names one material. Texture pixels are the base map the
/// tier draws (the cooked set when the file wears one, else the glTF image).
/// Findings keep file order, then mesh order, then cap order. Zero meshes
/// means no measurement happened and [`Self::status`] reads `UNKNOWN`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KitBudget {
    /// Meshes measured over all distinct kit files.
    pub meshes: usize,
    /// Largest triangle count of one mesh.
    pub max_tris: u32,
    /// Largest bone count of one mesh.
    pub max_bones: u32,
    /// Largest influence count of one vertex.
    pub max_influences: u32,
    /// Largest material count of one mesh.
    pub max_materials: u32,
    /// Largest base-colour texture pixel count of one mesh.
    pub max_tex_px: u32,
    /// Every mesh over a potato cap, in import order.
    pub findings: Vec<KitBudgetFinding>,
}

impl KitBudget {
    /// Measure one mesh and append its potato findings. Callers own the
    /// file and mesh index so a finding names where it came from.
    pub(super) fn add(&mut self, file: &'static str, mesh: usize, stats: MeshStats) {
        self.meshes += 1;
        self.max_tris = self.max_tris.max(stats.tris);
        self.max_bones = self.max_bones.max(stats.bones);
        self.max_influences = self.max_influences.max(stats.influences);
        self.max_materials = self.max_materials.max(stats.materials_per_skin);
        self.max_tex_px = self.max_tex_px.max(stats.tex_px);
        for finding in check(&stats, &POTATO) {
            self.findings.push(KitBudgetFinding {
                file,
                mesh,
                kind: finding.kind,
                actual: finding.actual,
                allowed: finding.allowed,
            });
        }
    }

    /// `PASS` when every mesh is inside the potato caps, `FAIL` when one is
    /// over, `UNKNOWN` when no mesh was measured.
    #[must_use]
    pub fn status(&self) -> &'static str {
        if self.meshes == 0 {
            "UNKNOWN"
        } else if self.findings.is_empty() {
            "PASS"
        } else {
            "FAIL"
        }
    }

    /// One log line for the arena and the potato report: the maxima then the
    /// first [`KIT_BUDGET_SHOWN`] findings. Pure and bounded.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out = format!(
            "meshes={} max_tris={} max_bones={} max_influences={} max_materials={} max_tex_px={} caps=POTATO budget={}",
            self.meshes,
            self.max_tris,
            self.max_bones,
            self.max_influences,
            self.max_materials,
            self.max_tex_px,
            self.status()
        );
        for finding in self.findings.iter().take(KIT_BUDGET_SHOWN) {
            out.push_str(&format!(
                " {} mesh={} {} {}>{}",
                finding.file, finding.mesh, finding.kind, finding.actual, finding.allowed
            ));
        }
        if self.findings.len() > KIT_BUDGET_SHOWN {
            out.push_str(&format!(
                " +{} more",
                self.findings.len() - KIT_BUDGET_SHOWN
            ));
        }
        out
    }
}
