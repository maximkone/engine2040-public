//! caps: the one table of per-tier limits.

use crate::error::Error;

/// The hardware class a client runs on. The floor is an integrated GPU with 8 GB of RAM (every
/// game must run there); a 3050 class card with 16 GB is the comfortable tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Weak GPU, little RAM: a couple of ragdolls and a handful of shards.
    Potato,
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
}

impl Tier {
    /// Every tier, weakest first.
    pub const ALL: [Tier; 4] = [Tier::Potato, Tier::Web, Tier::Comfort, Tier::High];

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
            },
            Tier::Web => Caps {
                bodies: 192,
                ragdolls: 4,
                contacts: 512,
                statics: 128,
                solver_iterations: 3,
                ccd_substeps: 1,
            },
            Tier::Comfort => Caps {
                bodies: 384,
                ragdolls: 8,
                contacts: 1024,
                statics: 256,
                solver_iterations: 4,
                ccd_substeps: 1,
            },
            Tier::High => Caps {
                bodies: 768,
                ragdolls: 16,
                contacts: 2048,
                statics: 512,
                solver_iterations: 6,
                ccd_substeps: 2,
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
            Tier::Web => 800,
            Tier::Comfort => 1_500,
            Tier::High => 2_000,
        }
    }
}

impl Caps {
    /// A cap table is usable when every limit but the CCD substeps is above zero.
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let positive = [
            self.bodies,
            self.ragdolls,
            self.contacts,
            self.statics,
            self.solver_iterations,
        ];
        if positive.contains(&0) {
            return Err(Error::BadCaps);
        }
        Ok(())
    }
}
