//! KAM2 simulation core: deterministic, std-only, no clock and no IO.
//!
//! Each side keeps its own stock, houses and deliveries; terrain and roads are shared.
//! Invariants live in the two modules: the map refuses what cannot be built, and the
//! economy advances only when `Sim::tick` is called. The same commands from the same
//! start produce the same hash on every machine.

pub mod economy;
pub mod map;
pub mod mayor;
pub mod sim;
pub mod units;

pub use economy::{House, HouseKind, Stock, Ware};
pub use map::{Map, Terrain};
pub use mayor::Mayor;
pub use sim::{Command, Refusal, Sim, SIDE_COUNT};
pub use units::{Shot, Unit, UnitKind};
