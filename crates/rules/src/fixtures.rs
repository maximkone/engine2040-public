//! Fixed worlds a script must survive before `rules.set` installs it. Generated from integers
//! only, so every run builds the same worlds.

use engine_core::mix64;

use crate::host::{ActorView, World, BTN_ATTACK, BTN_BLOCK, BTN_DODGE};
use crate::limits::MAX_FIGHTERS;

/// A fresh fighter at `(x, z)` millimetres on `team`.
pub fn fighter(id: u8, team: u8, x_mm: i32, z_mm: i32) -> ActorView {
    ActorView {
        id,
        team,
        alive: true,
        x_mm,
        z_mm,
        facing_cdeg: 0,
        hp_permille: 1_000,
        stamina: 1_000,
        attack_age: 0,
        attacking: false,
        blocking: false,
        dodging: false,
        react: 0,
        knockdown: 0,
        shield: 0,
        mx: 0,
        mz: 0,
        buttons: 0,
    }
}

/// `count` fighters on a ring of 3 m radius spaced by id, two teams by parity, buttons mixed
/// from `seed` so blocking, attacking and dodging all occur.
pub fn ring_world(count: usize, tick: u32, seed: u64) -> World {
    let count = count.clamp(1, MAX_FIGHTERS);
    let fighters = (0..count)
        .map(|index| {
            let id = u8::try_from(index).unwrap_or(u8::MAX);
            // A square spiral keeps positions integer and distinct without trigonometry.
            let ring = i32::from(id / 8) + 1;
            let step = i32::from(id % 8);
            let x = (step - 4) * 400;
            let z = ring * 1_000 - 2_000;
            let mut view = fighter(id, id % 2, x, z);
            let roll = mix64(seed ^ (u64::from(id) << 8) ^ u64::from(tick));
            view.buttons =
                u8::try_from(roll & 0x7).unwrap_or(0) & (BTN_ATTACK | BTN_BLOCK | BTN_DODGE);
            view.blocking = view.buttons & BTN_BLOCK != 0;
            view.attacking = view.buttons & BTN_ATTACK != 0;
            view.dodging = view.buttons & BTN_DODGE != 0;
            view.hp_permille = u16::try_from(200 + (roll >> 8) % 801).unwrap_or(1_000);
            view.stamina = u16::try_from((roll >> 20) % 1_001).unwrap_or(1_000);
            view
        })
        .collect();
    World {
        tick,
        seed,
        fighters,
    }
}

/// The worlds `Rules::set` runs a candidate through: duel at reach, duel with the first
/// fighter blocking and attacking, a broken fighter (zero health, stamina, knocked down), a
/// ring of 16 and a ring of 64.
pub fn smoke_worlds() -> Vec<World> {
    let mut duel = World {
        tick: 7,
        seed: 0x5eed,
        fighters: vec![fighter(0, 0, 0, 0), fighter(1, 1, 900, 0)],
    };
    let mut guard = duel.clone();
    guard.tick = 8;
    guard.fighters[0].buttons = BTN_BLOCK | BTN_ATTACK;
    guard.fighters[0].blocking = true;
    guard.fighters[0].attacking = true;
    guard.fighters[1].buttons = BTN_ATTACK;
    guard.fighters[1].attacking = true;
    duel.fighters[1].buttons = BTN_DODGE;
    let mut broken = duel.clone();
    broken.tick = 9;
    broken.fighters[0].hp_permille = 0;
    broken.fighters[0].stamina = 0;
    broken.fighters[0].knockdown = 40;
    broken.fighters[1].react = 12;
    vec![
        duel,
        guard,
        broken,
        ring_world(16, 10, 0x5eed),
        ring_world(MAX_FIGHTERS, 11, 0x5eed),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worlds_are_stable_and_in_slot_order() {
        let a = smoke_worlds();
        assert_eq!(a, smoke_worlds());
        for world in &a {
            for (index, view) in world.fighters.iter().enumerate() {
                assert_eq!(usize::from(view.id), index);
            }
        }
        assert_eq!(a.last().map(|w| w.fighters.len()), Some(MAX_FIGHTERS));
    }
}
