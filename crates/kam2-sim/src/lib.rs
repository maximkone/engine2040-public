//! KAM2 simulation core: deterministic, std-only, no clock and no IO.
//!
//! Each side keeps its own stock, houses and deliveries; terrain and roads are shared.
//! Invariants live in the two modules: the map refuses what cannot be built, and the
//! economy advances only when `Sim::tick` is called. The same commands from the same
//! start produce the same hash on every machine.
//!
//! Tiles hold one body. A moving unit reserves its next tile for the tick it enters; two
//! units of any side never stand on one tile at the same tick. A blocked unit waits on
//! its tile and retries next tick, a tile vacated this tick is free only next tick (so a
//! swap waits one tick and nobody passes through anybody), a dead unit frees its tile in
//! the tick it falls, and a spawn on a held tile is refused with [`Refusal::Occupied`].

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
