//! A recipe: a seed and a list of typed ops over named meshes, the shape the door's `geo.preview`
//! and `geo.apply` verbs take (center note `engine-steal-blender-2026-10-04.md` section 4).
//!
//! The AI never sends code, only ops from this closed list; every op is checked before it runs
//! and refused with a typed error, never a panic. A recipe is a pure function of its text: the
//! same recipe gives the same meshes and the same digest. Ops whose donor feature is off refuse
//! with `RecipeError::FeatureOff`. Serialization stays with the door (it maps JSON to these
//! types); this module has no serde dependency.

use std::collections::BTreeMap;

use engine_core::{Fnv64, Rng};

use crate::mesh::{ix, narrow, to_u64, GeoError, GeoMesh};
use crate::primitives;

/// Most ops in one recipe.
pub const MAX_OPS: usize = 64;
/// Most triangles any mesh in a recipe may have.
pub const MAX_TRIANGLES: usize = 200_000;
/// Longest mesh name in characters.
pub const MAX_NAME: usize = 32;

/// Which solid a `Primitive` op builds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Solid {
    /// Box with the given edge lengths.
    Cuboid([f32; 3]),
    /// UV sphere: radius, rings, segments.
    Sphere(f32, u32, u32),
    /// Capped cylinder along Y: radius, height, segments.
    Cylinder(f32, f32, u32),
    /// Torus: major radius, minor radius, major segments, minor segments.
    Torus(f32, f32, u32, u32),
    /// L-shaped extrusion: size, arm, height.
    LPrism(f32, f32, f32),
}

/// One step of a recipe. Names refer to meshes made by earlier steps.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Make `name` from a solid.
    Primitive {
        /// Name of the new mesh.
        name: String,
        /// The solid.
        solid: Solid,
    },
    /// Scale (positive), quarter turns about Y, then translate `name` in place.
    Place {
        /// Mesh to move.
        name: String,
        /// Per-axis scale, each above zero.
        scale: [f32; 3],
        /// Quarter turns about Y.
        quarter_turns_y: u32,
        /// Translation in meters.
        offset: [f32; 3],
    },
    /// Move every vertex of `name` by up to `amount` meters in each axis, from the recipe seed
    /// (a stand-in for hand-made roughness; the same seed gives the same bytes).
    Jitter {
        /// Mesh to roughen.
        name: String,
        /// Largest shift per axis in meters.
        amount: f32,
    },
    /// `out = a <op> b` (feature `boolean`).
    Boolean {
        /// Name of the result.
        out: String,
        /// First operand.
        a: String,
        /// Second operand.
        b: String,
        /// Union, difference or intersection.
        op: BooleanKind,
    },
    /// Replace the UVs of `name` with an unwrapped atlas (feature `uv`).
    Unwrap {
        /// Mesh to unwrap.
        name: String,
    },
    /// `out` = `name` simplified to `ratio` of its triangles, UV seams locked (feature `lod`).
    Lod {
        /// Source mesh.
        name: String,
        /// Name of the new level.
        out: String,
        /// Fraction of triangles to keep, above 0 and at most 1.
        ratio: f32,
    },
    /// Convex collision parts of `name` under the same name in `hulls` (feature `collision`).
    Collision {
        /// Source mesh.
        name: String,
        /// Most parts, 1 to 32.
        max_hulls: u32,
    },
}

/// Boolean flavour, independent of the `boolean` feature so a recipe always parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanKind {
    /// A or B.
    Union,
    /// A minus B.
    Difference,
    /// A and B.
    Intersection,
}

/// A seed and its ops.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    /// Seed for the ops that use randomness.
    pub seed: u64,
    /// The steps, run in order.
    pub ops: Vec<Op>,
}

/// Why a recipe was refused or failed at step `step` (0-based).
#[derive(Clone, Debug, PartialEq)]
pub enum RecipeError {
    /// More than `MAX_OPS` steps.
    TooManyOps,
    /// Step names a mesh no earlier step made.
    UnknownMesh {
        /// Failing step.
        step: usize,
        /// The missing name.
        name: String,
    },
    /// Step reuses a name already taken.
    NameTaken {
        /// Failing step.
        step: usize,
        /// The duplicated name.
        name: String,
    },
    /// A name is empty, too long or has characters outside `a-z 0-9 _ -`.
    BadName {
        /// Failing step.
        step: usize,
    },
    /// A number is not finite or outside its allowed range.
    BadNumber {
        /// Failing step.
        step: usize,
        /// Which field.
        field: &'static str,
    },
    /// A mesh would pass `MAX_TRIANGLES`.
    TooLarge {
        /// Failing step.
        step: usize,
    },
    /// The op needs a donor feature this build does not have.
    FeatureOff {
        /// Failing step.
        step: usize,
        /// The feature name.
        feature: &'static str,
    },
    /// The tool itself refused the mesh.
    Tool {
        /// Failing step.
        step: usize,
        /// Its error.
        error: GeoError,
    },
}

/// What a recipe produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeResult {
    /// Every named mesh, in name order.
    pub meshes: BTreeMap<String, GeoMesh>,
    /// Collision parts by mesh name.
    pub hulls: BTreeMap<String, Vec<GeoMesh>>,
}

/// One line of `geo.preview`: what a mesh is, without writing anything.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshReport {
    /// Mesh name.
    pub name: String,
    /// Triangles.
    pub triangles: usize,
    /// Vertices.
    pub vertices: usize,
    /// Axis-aligned bounds.
    pub bounds: ([f32; 3], [f32; 3]),
    /// Every edge shared by two consistent triangles.
    pub closed: bool,
    /// Has UVs.
    pub has_uvs: bool,
    /// Digest of the mesh.
    pub digest: u64,
}

impl RecipeResult {
    /// One report line per mesh, name order.
    #[must_use]
    pub fn report(&self) -> Vec<MeshReport> {
        self.meshes
            .iter()
            .map(|(name, m)| MeshReport {
                name: name.clone(),
                triangles: m.triangle_count(),
                vertices: m.positions.len(),
                bounds: m.bounds(),
                closed: m.edge_report().is_closed(),
                has_uvs: !m.uvs.is_empty(),
                digest: m.digest(),
            })
            .collect()
    }

    /// Digest of every mesh and every part: the recipe's receipt.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(to_u64(self.meshes.len()));
        for (name, m) in &self.meshes {
            h.write_str(name);
            h.write_u64(m.digest());
        }
        h.write_u64(to_u64(self.hulls.len()));
        for (name, parts) in &self.hulls {
            h.write_str(name);
            h.write_u64(to_u64(parts.len()));
            for p in parts {
                h.write_u64(p.digest());
            }
        }
        h.finish()
    }
}

fn check_name(step: usize, name: &str) -> Result<(), RecipeError> {
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(RecipeError::BadName { step })
    }
}

fn check_range(
    step: usize,
    field: &'static str,
    v: f32,
    lo: f32,
    hi: f32,
) -> Result<(), RecipeError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(RecipeError::BadNumber { step, field })
    }
}

fn check_count(
    step: usize,
    field: &'static str,
    v: u32,
    lo: u32,
    hi: u32,
) -> Result<(), RecipeError> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(RecipeError::BadNumber { step, field })
    }
}

fn build_solid(step: usize, solid: Solid) -> Result<GeoMesh, RecipeError> {
    const LEN: (f32, f32) = (0.001, 1000.0);
    let seg = |s: usize, f: &'static str, v: u32, lo: u32| check_count(s, f, v, lo, 256);
    let mesh = match solid {
        Solid::Cuboid(size) => {
            for v in size {
                check_range(step, "size", v, LEN.0, LEN.1)?;
            }
            primitives::cuboid(size)
        }
        Solid::Sphere(r, rings, segments) => {
            check_range(step, "radius", r, LEN.0, LEN.1)?;
            seg(step, "rings", rings, 2)?;
            seg(step, "segments", segments, 3)?;
            primitives::uv_sphere(r, ix(rings), ix(segments))
        }
        Solid::Cylinder(r, h, segments) => {
            check_range(step, "radius", r, LEN.0, LEN.1)?;
            check_range(step, "height", h, LEN.0, LEN.1)?;
            seg(step, "segments", segments, 3)?;
            primitives::cylinder(r, h, ix(segments))
        }
        Solid::Torus(major, minor, ms, ns) => {
            check_range(step, "major", major, LEN.0, LEN.1)?;
            check_range(step, "minor", minor, LEN.0, major)?;
            seg(step, "major_segments", ms, 3)?;
            seg(step, "minor_segments", ns, 3)?;
            primitives::torus(major, minor, ix(ms), ix(ns))
        }
        Solid::LPrism(size, arm, height) => {
            check_range(step, "size", size, LEN.0, LEN.1)?;
            check_range(step, "arm", arm, LEN.0, size * 0.99)?;
            check_range(step, "height", height, LEN.0, LEN.1)?;
            primitives::l_prism(size, arm, height)
        }
    };
    Ok(mesh)
}

struct Runner {
    seed: u64,
    out: RecipeResult,
}

impl Runner {
    fn get(&self, step: usize, name: &str) -> Result<&GeoMesh, RecipeError> {
        self.out
            .meshes
            .get(name)
            .ok_or_else(|| RecipeError::UnknownMesh {
                step,
                name: name.to_string(),
            })
    }

    fn insert(&mut self, step: usize, name: &str, mesh: GeoMesh) -> Result<(), RecipeError> {
        check_name(step, name)?;
        if self.out.meshes.contains_key(name) {
            return Err(RecipeError::NameTaken {
                step,
                name: name.to_string(),
            });
        }
        if mesh.triangle_count() > MAX_TRIANGLES {
            return Err(RecipeError::TooLarge { step });
        }
        self.out.meshes.insert(name.to_string(), mesh);
        Ok(())
    }

    fn step(&mut self, step: usize, op: &Op) -> Result<(), RecipeError> {
        match op {
            Op::Primitive { name, solid } => {
                let mesh = build_solid(step, *solid)?;
                self.insert(step, name, mesh)
            }
            Op::Place {
                name,
                scale,
                quarter_turns_y,
                offset,
            } => {
                for s in scale {
                    check_range(step, "scale", *s, 0.001, 1000.0)?;
                }
                for o in offset {
                    check_range(step, "offset", *o, -10_000.0, 10_000.0)?;
                }
                let placed = self
                    .get(step, name)?
                    .placed(*scale, *quarter_turns_y, *offset);
                self.out.meshes.insert(name.clone(), placed);
                Ok(())
            }
            Op::Jitter { name, amount } => {
                check_range(step, "amount", *amount, 0.0, 10.0)?;
                let mut mesh = self.get(step, name)?.clone();
                let mut rng = Rng::new(self.seed).fork(to_u64(step));
                for p in &mut mesh.positions {
                    for v in p.iter_mut() {
                        // Uniform in -amount..=amount from 16 random bits.
                        let unit =
                            narrow(f64::from(rng.next_u32() & 0xFFFF) / 65_535.0) * 2.0 - 1.0;
                        *v += unit * amount;
                    }
                }
                // Moved vertices invalidate stored normals.
                mesh.normals.clear();
                self.out.meshes.insert(name.clone(), mesh);
                Ok(())
            }
            Op::Boolean { out, a, b, op } => self.boolean(step, out, a, b, *op),
            Op::Unwrap { name } => self.unwrap(step, name),
            Op::Lod { name, out, ratio } => self.lod(step, name, out, *ratio),
            Op::Collision { name, max_hulls } => self.collision(step, name, *max_hulls),
        }
    }

    #[cfg(feature = "boolean")]
    fn boolean(
        &mut self,
        step: usize,
        out: &str,
        a: &str,
        b: &str,
        op: BooleanKind,
    ) -> Result<(), RecipeError> {
        use crate::boolean::{boolean, BoolOp};
        let kind = match op {
            BooleanKind::Union => BoolOp::Union,
            BooleanKind::Difference => BoolOp::Difference,
            BooleanKind::Intersection => BoolOp::Intersection,
        };
        let result = boolean(self.get(step, a)?, self.get(step, b)?, kind)
            .map_err(|error| RecipeError::Tool { step, error })?;
        self.insert(step, out, result)
    }

    #[cfg(not(feature = "boolean"))]
    fn boolean(
        &mut self,
        step: usize,
        _: &str,
        _: &str,
        _: &str,
        _: BooleanKind,
    ) -> Result<(), RecipeError> {
        Err(RecipeError::FeatureOff {
            step,
            feature: "boolean",
        })
    }

    #[cfg(feature = "uv")]
    fn unwrap(&mut self, step: usize, name: &str) -> Result<(), RecipeError> {
        let source = self.get(step, name)?;
        let unwrapped = crate::uv::unwrap(source, &crate::uv::UnwrapOptions::default())
            .map_err(|error| RecipeError::Tool { step, error })?;
        if unwrapped.mesh.triangle_count() > MAX_TRIANGLES {
            return Err(RecipeError::TooLarge { step });
        }
        self.out.meshes.insert(name.to_string(), unwrapped.mesh);
        Ok(())
    }

    #[cfg(not(feature = "uv"))]
    fn unwrap(&mut self, step: usize, _: &str) -> Result<(), RecipeError> {
        Err(RecipeError::FeatureOff {
            step,
            feature: "uv",
        })
    }

    #[cfg(feature = "lod")]
    fn lod(&mut self, step: usize, name: &str, out: &str, ratio: f32) -> Result<(), RecipeError> {
        check_range(step, "ratio", ratio, 0.01, 1.0)?;
        let options = crate::lod::LodOptions {
            target_ratio: ratio,
            ..crate::lod::LodOptions::default()
        };
        let result = crate::lod::simplify(self.get(step, name)?, &[], 0, 0.0, &options)
            .map_err(|error| RecipeError::Tool { step, error })?;
        self.insert(step, out, result.mesh)
    }

    #[cfg(not(feature = "lod"))]
    fn lod(&mut self, step: usize, _: &str, _: &str, _: f32) -> Result<(), RecipeError> {
        Err(RecipeError::FeatureOff {
            step,
            feature: "lod",
        })
    }

    #[cfg(feature = "collision")]
    fn collision(&mut self, step: usize, name: &str, max_hulls: u32) -> Result<(), RecipeError> {
        check_count(step, "max_hulls", max_hulls, 1, 32)?;
        let options = crate::collision::HullOptions {
            max_hulls,
            ..crate::collision::HullOptions::default()
        };
        let parts = crate::collision::decompose(self.get(step, name)?, &options)
            .map_err(|error| RecipeError::Tool { step, error })?;
        self.out.hulls.insert(name.to_string(), parts);
        Ok(())
    }

    #[cfg(not(feature = "collision"))]
    fn collision(&mut self, step: usize, _: &str, _: u32) -> Result<(), RecipeError> {
        Err(RecipeError::FeatureOff {
            step,
            feature: "collision",
        })
    }
}

/// Run a recipe. Nothing is written anywhere: the caller decides what to do with the result.
pub fn run(recipe: &Recipe) -> Result<RecipeResult, RecipeError> {
    if recipe.ops.len() > MAX_OPS {
        return Err(RecipeError::TooManyOps);
    }
    let mut runner = Runner {
        seed: recipe.seed,
        out: RecipeResult::default(),
    };
    for (step, op) in recipe.ops.iter().enumerate() {
        runner.step(step, op)?;
    }
    Ok(runner.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prim(name: &str, solid: Solid) -> Op {
        Op::Primitive {
            name: name.to_string(),
            solid,
        }
    }

    fn base() -> Recipe {
        Recipe {
            seed: 7,
            ops: vec![
                prim("body", Solid::Cuboid([2.0, 1.0, 1.0])),
                prim("knob", Solid::Sphere(0.4, 8, 12)),
                Op::Place {
                    name: "knob".into(),
                    scale: [1.0; 3],
                    quarter_turns_y: 0,
                    offset: [1.0, 0.5, 0.0],
                },
                Op::Jitter {
                    name: "body".into(),
                    amount: 0.01,
                },
            ],
        }
    }

    #[test]
    fn same_recipe_same_digest_and_seed_matters() {
        let a = run(&base()).unwrap();
        assert_eq!(a, run(&base()).unwrap());
        assert_eq!(a.digest(), run(&base()).unwrap().digest());
        let other = Recipe { seed: 8, ..base() };
        assert_ne!(a.digest(), run(&other).unwrap().digest());
        let report = a.report();
        assert_eq!(report.len(), 2);
        assert!(report.iter().all(|r| r.closed && r.triangles > 0));
        // Jitter moved the body but kept it closed and within 0.01 m of the cube corners.
        let (lo, hi) = a.meshes["body"].bounds();
        assert!((lo[0] + 1.0).abs() <= 0.0101 && (hi[0] - 1.0).abs() <= 0.0101);
    }

    #[test]
    fn bad_recipes_are_refused_with_typed_errors() {
        let ops = |op| Recipe {
            seed: 1,
            ops: vec![prim("a", Solid::Cuboid([1.0; 3])), op],
        };
        assert_eq!(
            run(&Recipe {
                seed: 1,
                ops: vec![Op::Unwrap { name: "x".into() }]
            })
            .unwrap_err(),
            if cfg!(feature = "uv") {
                RecipeError::UnknownMesh {
                    step: 0,
                    name: "x".into(),
                }
            } else {
                RecipeError::FeatureOff {
                    step: 0,
                    feature: "uv",
                }
            }
        );
        assert_eq!(
            run(&ops(prim("a", Solid::Cuboid([1.0; 3])))).unwrap_err(),
            RecipeError::NameTaken {
                step: 1,
                name: "a".into()
            }
        );
        assert_eq!(
            run(&ops(prim("Bad Name", Solid::Cuboid([1.0; 3])))).unwrap_err(),
            RecipeError::BadName { step: 1 }
        );
        assert_eq!(
            run(&ops(prim("b", Solid::Cuboid([f32::NAN, 1.0, 1.0])))).unwrap_err(),
            RecipeError::BadNumber {
                step: 1,
                field: "size"
            }
        );
        assert_eq!(
            run(&ops(prim("b", Solid::Sphere(1.0, 4000, 12)))).unwrap_err(),
            RecipeError::BadNumber {
                step: 1,
                field: "rings"
            }
        );
        assert_eq!(
            run(&ops(Op::Place {
                name: "zz".into(),
                scale: [1.0; 3],
                quarter_turns_y: 0,
                offset: [0.0; 3]
            }))
            .unwrap_err(),
            RecipeError::UnknownMesh {
                step: 1,
                name: "zz".into()
            }
        );
        assert_eq!(
            run(&ops(Op::Place {
                name: "a".into(),
                scale: [-1.0, 1.0, 1.0],
                quarter_turns_y: 0,
                offset: [0.0; 3]
            }))
            .unwrap_err(),
            RecipeError::BadNumber {
                step: 1,
                field: "scale"
            }
        );
        let many = Recipe {
            seed: 0,
            ops: vec![prim("a", Solid::Cuboid([1.0; 3])); MAX_OPS + 1],
        };
        assert_eq!(run(&many).unwrap_err(), RecipeError::TooManyOps);
    }

    #[cfg(not(feature = "boolean"))]
    #[test]
    fn boolean_without_the_feature_is_refused() {
        let r = Recipe {
            seed: 0,
            ops: vec![
                prim("a", Solid::Cuboid([1.0; 3])),
                prim("b", Solid::Cuboid([1.0; 3])),
                Op::Boolean {
                    out: "c".into(),
                    a: "a".into(),
                    b: "b".into(),
                    op: BooleanKind::Union,
                },
            ],
        };
        assert_eq!(
            run(&r).unwrap_err(),
            RecipeError::FeatureOff {
                step: 2,
                feature: "boolean"
            }
        );
    }

    #[cfg(all(
        feature = "boolean",
        feature = "uv",
        feature = "lod",
        feature = "collision"
    ))]
    #[test]
    fn full_pipeline_recipe_cuts_unwraps_levels_and_collides() {
        let recipe = Recipe {
            seed: 2040,
            ops: vec![
                prim("block", Solid::Cuboid([2.0, 2.0, 2.0])),
                prim("hole", Solid::Cylinder(0.5, 3.0, 24)),
                Op::Boolean {
                    out: "holed".into(),
                    a: "block".into(),
                    b: "hole".into(),
                    op: BooleanKind::Difference,
                },
                Op::Unwrap {
                    name: "holed".into(),
                },
                Op::Lod {
                    name: "holed".into(),
                    out: "holed_lod1".into(),
                    ratio: 0.5,
                },
                Op::Collision {
                    name: "holed".into(),
                    max_hulls: 8,
                },
            ],
        };
        let a = run(&recipe).unwrap();
        assert_eq!(a.digest(), run(&recipe).unwrap().digest());
        let holed = &a.meshes["holed"];
        assert_eq!(holed.open_edge_count(), 0);
        assert!(!holed.uvs.is_empty());
        assert!(holed.volume() < 8.0 && holed.volume() > 6.0);
        assert!(a.meshes["holed_lod1"].triangle_count() <= holed.triangle_count());
        assert_eq!(a.meshes["holed_lod1"].open_edge_count(), 0);
        let parts = &a.hulls["holed"];
        assert!(!parts.is_empty() && parts.len() <= 8);
        assert!(parts.iter().all(|p| p.open_edge_count() == 0));
    }
}
