//! speed_demo: `arena --speed-demo` (docs/SPRINT-33-speed-demo.md). Fly or drive ever
//! faster over a generated terrain and cut towers in half with a giant sword at your side.
//! Presentation only: nothing here reads into the fight sim, the rules or the hash. One
//! file per board row; the ground is passed in as a `height_at(x, z)` closure so the
//! movers (flight, car) do not depend on the terrain file.
// The rows land one by one before SD7 wires them into the window; until then their items
// have test callers only.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "rows land before SD7 wires every item into the window"
    )
)]

pub(super) mod car;
pub(super) mod city;
pub(super) mod collide;
pub(super) mod crowd;
pub(super) mod cut;
pub(super) mod draw;
pub(super) mod fall;
pub(super) mod flight;
pub(super) mod hero;
pub(crate) mod hud;
mod num;
pub(super) mod nyc;
pub(crate) mod scene;
pub(super) mod stage;
pub(super) mod swing;
pub(super) mod sword;
pub(super) mod terrain;
pub(super) mod traffic;
mod window;
