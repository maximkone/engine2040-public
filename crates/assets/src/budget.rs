//! Mesh budget caps plus a pure over-budget check before import.
//!
//! The caps table carries the true asset cost (triangles, bones,
//! influences, materials, texture pixels) as plain data so an oversized
//! fighter never reaches the cook. [`check`] only reports; the import
//! gate decides. Std-only, integer only, deterministic.

/// Budget caps for one import tier. Plain data, never read from disk here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caps {
    /// Maximum triangle count.
    pub max_tris: u32,
    /// Maximum bones per section.
    pub max_bones: u32,
    /// Maximum skin influences per vertex.
    pub max_influences: u32,
    /// Maximum materials per skin.
    pub max_materials_per_skin: u32,
    /// Maximum base-colour texture pixels.
    pub max_tex_px: u32,
}

/// Potato caps: 1500 tris, 75 bones, 4 influences, 1 material, 1024x1024 px.
pub const POTATO: Caps = Caps {
    max_tris: 1500,
    max_bones: 75,
    max_influences: 4,
    max_materials_per_skin: 1,
    max_tex_px: 1048576,
};

/// Comfort caps: 4000 tris with the other four identical to potato.
pub const COMFORT: Caps = Caps {
    max_tris: 4000,
    max_bones: 75,
    max_influences: 4,
    max_materials_per_skin: 1,
    max_tex_px: 1048576,
};

/// Measured mesh cost to judge against a [`Caps`] table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MeshStats {
    /// Measured triangle count.
    pub tris: u32,
    /// Measured bones per section.
    pub bones: u32,
    /// Measured skin influences per vertex.
    pub influences: u32,
    /// Measured materials per skin.
    pub materials_per_skin: u32,
    /// Measured base-colour texture pixels.
    pub tex_px: u32,
}

/// One exceeded cap: which limit, the measured value, and the allowed value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BudgetFinding {
    /// Exceeded cap name: `tris`, `bones`, `influences`, `materials`, `tex_px`.
    pub kind: &'static str,
    /// Measured value from [`MeshStats`].
    pub actual: u32,
    /// Allowed value from [`Caps`].
    pub allowed: u32,
}

/// Judge mesh stats against caps, one finding per exceeded cap.
///
/// Fixed order: tris, bones, influences, materials_per_skin, tex_px.
/// Empty when inside all caps. Pure: same inputs give the same output,
/// with no float, clock, or IO.
pub fn check(stats: &MeshStats, caps: &Caps) -> Vec<BudgetFinding> {
    let mut out = Vec::new();
    if stats.tris > caps.max_tris {
        out.push(BudgetFinding {
            kind: "tris",
            actual: stats.tris,
            allowed: caps.max_tris,
        });
    }
    if stats.bones > caps.max_bones {
        out.push(BudgetFinding {
            kind: "bones",
            actual: stats.bones,
            allowed: caps.max_bones,
        });
    }
    if stats.influences > caps.max_influences {
        out.push(BudgetFinding {
            kind: "influences",
            actual: stats.influences,
            allowed: caps.max_influences,
        });
    }
    if stats.materials_per_skin > caps.max_materials_per_skin {
        out.push(BudgetFinding {
            kind: "materials",
            actual: stats.materials_per_skin,
            allowed: caps.max_materials_per_skin,
        });
    }
    if stats.tex_px > caps.max_tex_px {
        out.push(BudgetFinding {
            kind: "tex_px",
            actual: stats.tex_px,
            allowed: caps.max_tex_px,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fighter(tris: u32) -> MeshStats {
        MeshStats {
            tris,
            bones: 60,
            influences: 4,
            materials_per_skin: 1,
            tex_px: 1048576,
        }
    }

    #[test]
    fn budget_3900_passes_4100_reports_tris() {
        assert!(check(&fighter(3900), &COMFORT).is_empty());
        let over = check(&fighter(4100), &COMFORT);
        assert_eq!(over.len(), 1);
        assert_eq!(
            over[0],
            BudgetFinding {
                kind: "tris",
                actual: 4100,
                allowed: 4000,
            }
        );
    }

    #[test]
    fn budget_76_bones_reports_bones() {
        let stats = MeshStats {
            tris: 1000,
            bones: 76,
            influences: 4,
            materials_per_skin: 1,
            tex_px: 262144,
        };
        let found = check(&stats, &COMFORT);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0],
            BudgetFinding {
                kind: "bones",
                actual: 76,
                allowed: 75,
            }
        );
    }

    #[test]
    fn budget_inside_all_caps_is_empty() {
        let crowd = MeshStats {
            tris: 1500,
            bones: 75,
            influences: 4,
            materials_per_skin: 1,
            tex_px: 1048576,
        };
        assert!(check(&crowd, &POTATO).is_empty());
    }

    #[test]
    fn budget_same_inputs_twice_equal() {
        let stats = fighter(4100);
        assert_eq!(check(&stats, &COMFORT), check(&stats, &COMFORT));
    }
}
