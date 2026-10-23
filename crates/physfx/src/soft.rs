//! soft: Rapier 0.36 soft-body cloth for capes and banners, a float layer that is never hashed.
//!
//! A sheet is a grid of particles pinned along one row. [`SoftWorld`] builds every sheet once
//! (positions, structural and bending edges, the triangle skin and the pins), inserts it into one
//! Rapier `PhysicsWorld` and steps that world at the fixed 60 Hz. The particle positions are
//! copied into a buffer allocated at spawn, so a tick adds no buffer of this module; Rapier keeps
//! its own solver workspace.
//!
//! Units: meters and seconds; `stiffness_hz` is the natural frequency of the edge springs.
//! The authority layer never sees this: the golden hash is unchanged.

use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::dynamics::{SoftBodyBuilder, SoftBodyHandle, SpringCoefficients};
use rapier3d::math::Vector;
use rapier3d::pipeline::PhysicsWorld;
use std::fmt::{Display, Formatter};

/// The most cloths one [`SoftWorld`] holds: a cape per fighter is expected, not a field.
pub const MAX_CLOTHS: usize = 8;
/// Largest rows or columns a cloth may have.
const MAX_SIDE: usize = 32;
/// Most particles one cloth holds.
const MAX_NODES: usize = 1024;
/// Accepted neighbour spacing, meters.
const SPACING_M: (f32, f32) = (0.01, 1.0);
/// Accepted gravity scale.
const GRAVITY_SCALE: (f32, f32) = (0.0, 4.0);
/// Accepted structural stiffness, hertz.
const STIFFNESS_HZ: (f32, f32) = (1.0, 60.0);
/// Mass of one particle, kilograms.
const PARTICLE_KG: f32 = 0.01;
/// Damping ratio of every cloth spring: critical, so a free row settles instead of ringing.
const DAMPING: f32 = 1.0;

/// A sheet of cloth: `rows` particles down the hang, `cols` across the pinned edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothDesc {
    /// Particles down the hang; at least 2.
    pub rows: usize,
    /// Particles across the pinned edge; at least 2, and the pin count.
    pub cols: usize,
    /// Rest distance between neighbours, meters.
    pub spacing_m: f32,
    /// The pinned row, counted from the top (0). At least one free row must lie below it.
    pub pin_row: usize,
    /// Gravity scale of the particles; 1 is the world gravity.
    pub gravity_scale: f32,
    /// Natural frequency of the edge springs, hertz. Higher is stiffer.
    pub stiffness_hz: f32,
}

impl ClothDesc {
    /// A fighter cape: 10 rows down the hang, 6 columns across the shoulders, 9 cm apart.
    pub const CAPE: Self = Self {
        rows: 10,
        cols: 6,
        spacing_m: 0.09,
        pin_row: 0,
        gravity_scale: 1.0,
        stiffness_hz: 5.0,
    };

    /// A small arena banner: 4 by 4 particles, 15 cm apart.
    pub const BANNER: Self = Self {
        rows: 4,
        cols: 4,
        spacing_m: 0.15,
        pin_row: 0,
        gravity_scale: 1.0,
        stiffness_hz: 6.0,
    };

    /// Particles in the sheet.
    #[must_use]
    pub fn nodes(&self) -> usize {
        self.rows.saturating_mul(self.cols)
    }

    /// Particles in the pinned row.
    #[must_use]
    pub fn pins(&self) -> usize {
        self.cols
    }

    fn validate(&self) -> Result<(), ClothError> {
        if self.rows < 2
            || self.cols < 2
            || self.rows > MAX_SIDE
            || self.cols > MAX_SIDE
            || self.nodes() > MAX_NODES
        {
            return Err(ClothError::Grid);
        }
        if !self.spacing_m.is_finite() || !(SPACING_M.0..=SPACING_M.1).contains(&self.spacing_m) {
            return Err(ClothError::Spacing);
        }
        if self.pin_row >= self.rows || self.pin_row + 1 >= self.rows {
            return Err(ClothError::PinRow);
        }
        if !self.gravity_scale.is_finite()
            || !(GRAVITY_SCALE.0..=GRAVITY_SCALE.1).contains(&self.gravity_scale)
        {
            return Err(ClothError::Gravity);
        }
        if !self.stiffness_hz.is_finite()
            || !(STIFFNESS_HZ.0..=STIFFNESS_HZ.1).contains(&self.stiffness_hz)
        {
            return Err(ClothError::Stiffness);
        }
        Ok(())
    }
}

/// Why a cloth desc or a spawn was refused. A refused spawn leaves the world unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothError {
    /// Rows or columns outside 2..=32, or more than 1024 nodes.
    Grid,
    /// Spacing outside 0.01..=1 m, or not finite.
    Spacing,
    /// The pin row is not a row with at least one free row below it.
    PinRow,
    /// The gravity scale is outside 0..=4, or not finite.
    Gravity,
    /// The stiffness is outside 1..=60 Hz, or not finite.
    Stiffness,
    /// The slot holds no cloth; a caller named a slot the world does not hold.
    Missing,
    /// The world already holds [`MAX_CLOTHS`] cloths.
    OverCap {
        /// Cloths the spawn would need.
        need: usize,
        /// Cloths the world holds.
        cap: usize,
    },
}

impl Display for ClothError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Grid => {
                f.write_str("a cloth grid needs 2..=32 rows and columns and at most 1024 nodes")
            }
            Self::Spacing => f.write_str("a cloth spacing needs a finite value in 0.01..=1 m"),
            Self::PinRow => {
                f.write_str("a cloth pin row needs a row with at least one free row below")
            }
            Self::Gravity => f.write_str("a cloth gravity scale needs a finite value in 0..=4"),
            Self::Stiffness => f.write_str("a cloth stiffness needs a finite value in 1..=60 Hz"),
            Self::Missing => f.write_str("the cloth slot holds no cloth"),
            Self::OverCap { need, cap } => write!(f, "{need} needed, the cloth world holds {cap}"),
        }
    }
}

impl std::error::Error for ClothError {}

/// One cloth in a [`SoftWorld`]: its desc, its pinned rest positions and its current particles.
pub struct SoftCloth {
    desc: ClothDesc,
    body: SoftBodyHandle,
    /// Rest position of every pinned particle, in column order.
    pins: Vec<[f32; 3]>,
    /// Triangle indices of the skin, built once.
    triangles: Vec<[u32; 3]>,
    /// Current particle positions, row-major, rewritten in place by [`SoftWorld::step`].
    particles: Vec<[f32; 3]>,
    /// Rest y of the pin row, meters.
    pin_y_m: f32,
}

impl SoftCloth {
    /// The desc this cloth was built from.
    #[must_use]
    pub fn desc(&self) -> ClothDesc {
        self.desc
    }

    /// Particles in the sheet.
    #[must_use]
    pub fn nodes(&self) -> usize {
        self.desc.nodes()
    }

    /// Particles in the pinned row.
    #[must_use]
    pub fn pins(&self) -> usize {
        self.desc.pins()
    }

    /// Current particle positions in row-major order: the pinned row first, then down the hang.
    /// The slice is the buffer [`SoftWorld::step`] rewrites in place.
    #[must_use]
    pub fn particles(&self) -> &[[f32; 3]] {
        &self.particles
    }

    /// The rest positions of the pinned row, in column order.
    #[must_use]
    pub fn pin_positions(&self) -> &[[f32; 3]] {
        &self.pins
    }

    /// Two triangles per grid cell, indices into [`Self::particles`]: the skin, built once.
    #[must_use]
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Mean drop of the free bottom row below the pin row, centimeters. Positive when it hangs.
    #[must_use]
    pub fn sag_cm(&self) -> f32 {
        let start = (self.desc.rows - 1) * self.desc.cols;
        let mut sum = 0.0;
        for p in &self.particles[start..start + self.desc.cols] {
            sum += p[1];
        }
        let mean = sum / f32_of(self.desc.cols);
        (self.pin_y_m - mean) * 100.0
    }

    /// What the cloth measures at this tick.
    #[must_use]
    pub fn receipt(&self) -> SoftReceipt {
        SoftReceipt {
            nodes: self.desc.nodes(),
            pins: self.desc.pins(),
            sag_cm: self.sag_cm(),
        }
    }
}

/// One Rapier soft-body world for every cloth of a [`crate::PhysFx`] world: one
/// `SoftBodySet` and one pipeline, stepped at the fixed 60 Hz. The cloths have no colliders
/// and never meet the ragdolls; they are a float look layer.
pub struct SoftWorld {
    world: PhysicsWorld,
    cloths: Vec<SoftCloth>,
}

impl SoftWorld {
    /// An empty cloth world at the fixed step and the world gravity.
    #[must_use]
    pub fn new() -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, GRAVITY_Y, 0.0);
        world.integration_parameters.dt = FIXED_DT;
        Self {
            world,
            cloths: Vec::new(),
        }
    }

    /// Builds and hangs one sheet. Refused past [`MAX_CLOTHS`] or with a bad desc; a refused
    /// spawn leaves the world unchanged.
    pub fn spawn(&mut self, desc: ClothDesc) -> Result<usize, ClothError> {
        desc.validate()?;
        if self.cloths.len() >= MAX_CLOTHS {
            return Err(ClothError::OverCap {
                need: self.cloths.len() + 1,
                cap: MAX_CLOTHS,
            });
        }
        let sheet = Sheet::new(desc);
        let pin_y_m = sheet.pins[0][1];
        let builder = SoftBodyBuilder::new(sheet.positions)
            .edges(sheet.edges)
            .bend_edges(sheet.bends)
            .surface(sheet.triangles.clone())
            .pinned_particles(sheet.pinned)
            .particle_mass(PARTICLE_KG)
            .particle_radius(desc.spacing_m * 0.5)
            .softness(SpringCoefficients::new(desc.stiffness_hz, DAMPING))
            .gravity_scale(desc.gravity_scale)
            .no_surface_collider()
            .can_sleep(true);
        let body = self.world.soft_bodies.insert(
            builder,
            &mut self.world.bodies,
            &mut self.world.colliders,
        );
        self.cloths.push(SoftCloth {
            desc,
            body,
            pins: sheet.pins,
            triangles: sheet.triangles,
            particles: sheet.particles,
            pin_y_m,
        });
        Ok(self.cloths.len() - 1)
    }

    /// Steps the world one fixed tick and copies every cloth's particles into its buffer. The
    /// pin rows are snapped back to their rest positions before the copy, so a pinned particle
    /// is exactly on its pin.
    pub fn step(&mut self) {
        if self.cloths.is_empty() {
            return;
        }
        self.world.step();
        let Self { world, cloths } = self;
        for cloth in cloths {
            let Some(body) = world.soft_bodies.get_mut(cloth.body) else {
                continue;
            };
            let base = cloth.desc.pin_row * cloth.desc.cols;
            for (col, pin) in cloth.pins.iter().enumerate() {
                let i = base + col;
                let p = body.particle_position(i);
                if p.x != pin[0] || p.y != pin[1] || p.z != pin[2] {
                    body.set_particle_position(i, Vector::new(pin[0], pin[1], pin[2]));
                }
            }
            for (out, p) in cloth.particles.iter_mut().zip(body.particle_positions()) {
                *out = [p.x, p.y, p.z];
            }
        }
    }

    /// Every cloth, oldest first.
    #[must_use]
    pub fn cloths(&self) -> &[SoftCloth] {
        &self.cloths
    }

    /// The cloth in `slot`, or `None` when the world holds no such cloth.
    #[must_use]
    pub fn cloth(&self, slot: usize) -> Option<&SoftCloth> {
        self.cloths.get(slot)
    }
}

impl Default for SoftWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// What a cloth measures at one tick: its size and how far the free bottom row hangs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftReceipt {
    /// Particles in the sheet.
    pub nodes: usize,
    /// Particles in the pinned row.
    pub pins: usize,
    /// Mean drop of the free bottom row below the pin row, centimeters.
    pub sag_cm: f32,
}

impl SoftReceipt {
    /// The one line the page and the window print.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        self.to_string()
    }
}

impl Display for SoftReceipt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "physfx: soft: nodes={} pins={} sag_cm={:.1}",
            self.nodes, self.pins, self.sag_cm
        )
    }
}

/// A grid index as `f32`: the grid is capped at 32, so the fallback is unreachable.
fn f32_of(index: usize) -> f32 {
    f32::from(u16::try_from(index).unwrap_or(u16::MAX))
}

/// The one-time topology of a sheet. The builder consumes the positions, edges, bends and
/// pins; the cloth keeps the triangles, the pins and the particle buffer.
struct Sheet {
    positions: Vec<Vector>,
    edges: Vec<[u32; 2]>,
    bends: Vec<[u32; 2]>,
    triangles: Vec<[u32; 3]>,
    pinned: Vec<u32>,
    pins: Vec<[f32; 3]>,
    particles: Vec<[f32; 3]>,
}

impl Sheet {
    fn new(desc: ClothDesc) -> Self {
        let nodes = desc.nodes();
        let mut particles = Vec::with_capacity(nodes);
        for row in 0..desc.rows {
            for col in 0..desc.cols {
                particles.push([
                    f32_of(col) * desc.spacing_m,
                    -f32_of(row) * desc.spacing_m,
                    0.0,
                ]);
            }
        }
        let index = |row: usize, col: usize| {
            // The validated grid holds at most 1024 nodes; the fallback is unreachable.
            u32::try_from(row * desc.cols + col).unwrap_or(u32::MAX)
        };
        let mut edges = Vec::new();
        let mut bends = Vec::new();
        let mut triangles = Vec::new();
        for row in 0..desc.rows {
            for col in 0..desc.cols {
                if col + 1 < desc.cols {
                    edges.push([index(row, col), index(row, col + 1)]);
                }
                if row + 1 < desc.rows {
                    edges.push([index(row, col), index(row + 1, col)]);
                }
                if row + 1 < desc.rows && col + 1 < desc.cols {
                    edges.push([index(row, col), index(row + 1, col + 1)]);
                    edges.push([index(row + 1, col), index(row, col + 1)]);
                    triangles.push([index(row, col), index(row, col + 1), index(row + 1, col)]);
                    triangles.push([
                        index(row + 1, col),
                        index(row, col + 1),
                        index(row + 1, col + 1),
                    ]);
                }
                if col + 2 < desc.cols {
                    bends.push([index(row, col), index(row, col + 2)]);
                }
                if row + 2 < desc.rows {
                    bends.push([index(row, col), index(row + 2, col)]);
                }
            }
        }
        let pinned: Vec<u32> = (0..desc.cols).map(|col| index(desc.pin_row, col)).collect();
        let pins: Vec<[f32; 3]> = pinned
            .iter()
            .map(|&i| particles[usize::try_from(i).unwrap_or(usize::MAX)])
            .collect();
        let positions: Vec<Vector> = particles
            .iter()
            .map(|p| Vector::new(p[0], p[1], p[2]))
            .collect();
        Self {
            positions,
            edges,
            bends,
            triangles,
            pinned,
            pins,
            particles,
        }
    }
}
