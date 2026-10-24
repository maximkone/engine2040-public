//! caps: the one table of per-tier limits.

use crate::error::Error;

/// The hardware class a client runs on. The floor is an integrated GPU with 8 GB of RAM (every
/// game must run there); a 3050 class card with 16 GB is the comfortable tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Weak GPU, little RAM: a couple of ragdolls and a handful of shards.
    Potato,
    /// A phone or tablet: one solver pass, a few ragdolls, no CCD.
    Mobile,
    /// A browser tab: single thread, wasm speed.
    Web,
    /// The comfortable desktop tier: eight ragdolls plus a few hundred debris bodies.
    Comfort,
    /// Headroom: corpses stay, bigger collapses.
    High,
}

/// Hard limits of one world. Plain data, so the quality ladder (RZ8) can hand [`PhysFx`] a table
/// of its own through `PhysFx::with_caps`.
///
/// [`PhysFx`]: crate::PhysFx
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// Dynamic bodies alive at once. A ragdoll bone is one body.
    pub bodies: usize,
    /// Ragdolls alive at once.
    pub ragdolls: usize,
    /// Active contact pairs a step may pay for; above it the oldest limbs and debris are shed.
    pub contacts: usize,
    /// Static boxes (floor, walls) the level may add.
    pub statics: usize,
    /// Solver iterations per step: the substep budget of the soft-step solver.
    pub solver_iterations: usize,
    /// Continuous-collision substeps per step; `0` turns CCD off.
    pub ccd_substeps: usize,
    /// Soft-CCD look-ahead in millimeters: how far ahead of a fast body the narrow phase looks
    /// for another body, so the fast one stops at it instead of passing through. `0` turns soft
    /// CCD off. Only the tier with headroom pays for it.
    pub soft_ccd_mm: u16,
    /// Bodies one blast may shove: the nearest to its center, the rest stay put.
    pub blast_bodies: usize,
    /// Bodies one projectile impact may shove: the nearest to the hit point.
    pub impact_bodies: usize,
    /// Ropes (spear tethers) alive at once; a new one recycles the oldest.
    pub tethers: usize,
    /// Nodes in one rope: the longer the chain, the smoother the sag and the more it costs.
    pub tether_nodes: usize,
}

impl Tier {
    /// Every tier, weakest first.
    pub const ALL: [Tier; 5] = [
        Tier::Potato,
        Tier::Mobile,
        Tier::Web,
        Tier::Comfort,
        Tier::High,
    ];

    /// The cap table: every per-tier number lives in this one place. The values are the first
    /// cut; the physics ladder row (RZ8) tunes them from measured frame cost.
    pub const fn caps(self) -> Caps {
        match self {
            Tier::Potato => Caps {
                bodies: 96,
                ragdolls: 2,
                contacts: 256,
                statics: 64,
                solver_iterations: 2,
                ccd_substeps: 0,
                soft_ccd_mm: 0,
                blast_bodies: 24,
                impact_bodies: 3,
                tethers: 2,
                tether_nodes: 8,
            },
            Tier::Mobile => Caps {
                bodies: 128,
                ragdolls: 3,
                contacts: 384,
                statics: 96,
                solver_iterations: 2,
                ccd_substeps: 0,
                soft_ccd_mm: 0,
                blast_bodies: 36,
                impact_bodies: 4,
                tethers: 2,
                tether_nodes: 10,
            },
            Tier::Web => Caps {
                bodies: 192,
                ragdolls: 4,
                contacts: 512,
                statics: 128,
                solver_iterations: 3,
                ccd_substeps: 1,
                soft_ccd_mm: 0,
                blast_bodies: 48,
                impact_bodies: 4,
                tethers: 3,
                tether_nodes: 12,
            },
            Tier::Comfort => Caps {
                bodies: 384,
                ragdolls: 8,
                contacts: 1024,
                statics: 256,
                solver_iterations: 4,
                ccd_substeps: 1,
                soft_ccd_mm: 0,
                blast_bodies: 96,
                impact_bodies: 6,
                tethers: 6,
                tether_nodes: 20,
            },
            Tier::High => Caps {
                bodies: 768,
                ragdolls: 16,
                contacts: 2048,
                statics: 512,
                solver_iterations: 6,
                ccd_substeps: 2,
                soft_ccd_mm: 300,
                blast_bodies: 192,
                impact_bodies: 8,
                tethers: 12,
                tether_nodes: 32,
            },
        }
    }
}

impl Tier {
    /// What one step may cost on average, in microseconds, in the soak of the `ladder` tests:
    /// the plan scene (8 ragdolls, 200 debris, 6 limbs) cut to what the tier admits, so its
    /// bodies sit at the cap, on a release build. Five times the mean measured on a GitHub
    /// runner (2026-10-17: 72, 161, 266 and 380 us), so a slowdown of that size fails the soak
    /// and runner noise does not. Ordered like the caps.
    pub const fn tick_bar_us(self) -> u64 {
        match self {
            Tier::Potato => 400,
            Tier::Mobile => 600,
            Tier::Web => 800,
            Tier::Comfort => 1_500,
            Tier::High => 2_000,
        }
    }
}

impl Caps {
    /// The soft-CCD look-ahead in meters, the unit Rapier takes: `0.0` when the tier has soft
    /// CCD off.
    pub(crate) fn soft_ccd_m(&self) -> f32 {
        f32::from(self.soft_ccd_mm) / 1_000.0
    }

    /// A cap table is usable when every limit but the CCD substeps is above zero (a rope needs two
    /// nodes at least).
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let positive = [
            self.bodies,
            self.ragdolls,
            self.contacts,
            self.statics,
            self.solver_iterations,
            self.blast_bodies,
            self.impact_bodies,
            self.tethers,
            self.tether_nodes,
        ];
        if positive.contains(&0) || self.tether_nodes < 2 {
            return Err(Error::BadCaps);
        }
        Ok(())
    }
}
