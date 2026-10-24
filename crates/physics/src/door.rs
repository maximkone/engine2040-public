//! door: hinged doors with integer angles and states (UW10).
//!
//! Consumer: none yet. The headless composer (UW26) applies squad and fighter actions to the
//! doors of a map, feeds each `Outcome` to nav (link changes) and audio (cue words), and
//! swaps a door collider's layer for `Door::layer` so casts and fighters see the new state.
//! Data owner: this module owns the state machine and its outcome table. Units: decidegrees
//! for angles (900 is a quarter turn), integrity points for `hp`. Determinism: integer table
//! lookup only, no float, no clock, no random draw; the same door and the same action always
//! give the same door and the same outcome, and `Door::hash` folds the whole state. Errors:
//! none; every state accepts every action, because a kick at a broken door is a legal thing to
//! do and does nothing. Budget: one table row per action, no allocation.
//!
//! `hp` is the integrity of whatever holds the door shut: the leaf, its lock or a wedge under
//! it. A locked or wedged door gives way when blows take `hp` to `GIVE_HP` or below; any
//! door at zero `hp` is broken, and a broken door is terminal and blocks nothing.
#![forbid(unsafe_code)]

use crate::layers;
use engine_core::Fnv64;

/// Quarter turn, decidegrees: a door swung wide.
pub const OPEN_ANGLE: i16 = 900;
/// Angle a push cracks a closed door to, decidegrees.
pub const AJAR_ANGLE: i16 = 150;
/// Angle a peek opens a closed door to, decidegrees.
pub const PEEK_ANGLE: i16 = 60;
/// Smallest angle a fighter fits through, decidegrees.
pub const PASS_ANGLE: i16 = 450;
/// Integrity of a fresh door.
pub const DOOR_HP: u16 = 100;
/// A locked or wedged door gives way at this integrity or below.
pub const GIVE_HP: u16 = 40;

/// Where a door stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DoorState {
    /// Shut and free to open.
    Closed,
    /// Cracked open, too narrow to pass.
    Ajar,
    /// Swung wide.
    Open,
    /// Shut and locked.
    Locked,
    /// Shut with a wedge under it.
    Wedged,
    /// Shut with a device on it that fires when it opens.
    Trapped,
    /// Wrecked; stays wrecked.
    Broken,
}

impl DoorState {
    /// Every state, in discriminant order.
    pub const ALL: [DoorState; 7] = [
        DoorState::Closed,
        DoorState::Ajar,
        DoorState::Open,
        DoorState::Locked,
        DoorState::Wedged,
        DoorState::Trapped,
        DoorState::Broken,
    ];

    /// Row of the outcome table, also the byte the hash folds.
    const fn index(self) -> usize {
        match self {
            Self::Closed => 0,
            Self::Ajar => 1,
            Self::Open => 2,
            Self::Locked => 3,
            Self::Wedged => 4,
            Self::Trapped => 5,
            Self::Broken => 6,
        }
    }
}

/// What is done to a door.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DoorAction {
    /// A hand on the leaf.
    Push,
    /// Open a crack to look through.
    Peek,
    /// A boot to the leaf.
    Kick,
    /// A shoulder or a ram.
    Ram,
    /// A shotgun round at the lock and hinges.
    Shotgun,
    /// A breaching charge.
    Charge,
}

impl DoorAction {
    /// Every action, in discriminant order.
    pub const ALL: [DoorAction; 6] = [
        DoorAction::Push,
        DoorAction::Peek,
        DoorAction::Kick,
        DoorAction::Ram,
        DoorAction::Shotgun,
        DoorAction::Charge,
    ];

    /// Column of the outcome table.
    const fn index(self) -> usize {
        match self {
            Self::Push => 0,
            Self::Peek => 1,
            Self::Kick => 2,
            Self::Ram => 3,
            Self::Shotgun => 4,
            Self::Charge => 5,
        }
    }
}

/// What happened to the door, for audio and for the squad's eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorEvent {
    /// A push cracked it open.
    Cracked,
    /// It swung wide.
    Swung,
    /// It was pushed shut.
    Shut,
    /// It crashed open or shut under a kick or a ram.
    Slammed,
    /// A shotgun round hit it.
    Blasted,
    /// Someone opened it a crack to look.
    Peeked,
    /// A locked door shook and held.
    Rattled,
    /// A wedge held the door.
    Held,
    /// The lock gave way.
    LockGave,
    /// The wedge gave way.
    WedgeGave,
    /// The device on the door fired.
    TrapSprung,
    /// A peek saw the device on the door.
    TrapSpotted,
    /// The door was wrecked.
    Broke,
}

impl DoorEvent {
    /// The audio cue word for the event; the audio table maps the word to a sample set.
    pub const fn cue(self) -> &'static str {
        match self {
            Self::Cracked | Self::Swung | Self::Peeked => "creak",
            Self::Shut => "click",
            Self::Slammed => "slam",
            Self::Blasted => "blast",
            Self::Rattled => "rattle",
            Self::Held => "thud",
            Self::LockGave | Self::WedgeGave => "snap",
            Self::TrapSprung => "trap",
            Self::TrapSpotted => "tick",
            Self::Broke => "splinter",
        }
    }
}

/// How the action changed whether a fighter can pass, for the nav link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// Passability did not change.
    Unchanged,
    /// The door now lets a fighter through.
    Opened,
    /// The door now stops a fighter.
    Closed,
}

/// What one action did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The event to play, if the action made one.
    pub event: Option<DoorEvent>,
    /// The nav link change.
    pub link: Link,
}

/// Leave the angle where it is.
const KEEP: i16 = -1;

/// One row of the outcome table.
#[derive(Clone, Copy)]
struct Rule {
    next: DoorState,
    angle: i16,
    damage: u16,
    event: Option<DoorEvent>,
}

const fn rule(next: DoorState, angle: i16, damage: u16, event: Option<DoorEvent>) -> Rule {
    Rule {
        next,
        angle,
        damage,
        event,
    }
}

/// Damage no door survives: a charge.
const WRECK: u16 = u16::MAX;

/// The outcome table, `[state][action]` in discriminant order. A holding state (locked, wedged)
/// names itself as `next` while it holds; [`Door::apply`] turns it into open once the blows
/// have taken `hp` to `GIVE_HP`.
const RULES: [[Rule; 6]; 7] = {
    use DoorEvent as E;
    use DoorState as S;
    let broke = rule(S::Broken, OPEN_ANGLE, WRECK, Some(E::Broke));
    [
        // Closed: push, peek, kick, ram, shotgun, charge.
        [
            rule(S::Ajar, AJAR_ANGLE, 0, Some(E::Cracked)),
            rule(S::Ajar, PEEK_ANGLE, 0, Some(E::Peeked)),
            rule(S::Open, OPEN_ANGLE, 0, Some(E::Slammed)),
            rule(S::Open, OPEN_ANGLE, 10, Some(E::Slammed)),
            rule(S::Open, OPEN_ANGLE, 40, Some(E::Blasted)),
            broke,
        ],
        // Ajar.
        [
            rule(S::Open, OPEN_ANGLE, 0, Some(E::Swung)),
            rule(S::Ajar, KEEP, 0, Some(E::Peeked)),
            rule(S::Open, OPEN_ANGLE, 0, Some(E::Slammed)),
            rule(S::Open, OPEN_ANGLE, 10, Some(E::Slammed)),
            rule(S::Open, OPEN_ANGLE, 40, Some(E::Blasted)),
            broke,
        ],
        // Open.
        [
            rule(S::Closed, 0, 0, Some(E::Shut)),
            rule(S::Open, KEEP, 0, None),
            rule(S::Closed, 0, 0, Some(E::Slammed)),
            rule(S::Open, KEEP, 0, None),
            rule(S::Open, KEEP, 40, Some(E::Blasted)),
            broke,
        ],
        // Locked.
        [
            rule(S::Locked, KEEP, 0, Some(E::Rattled)),
            rule(S::Locked, KEEP, 0, None),
            rule(S::Locked, KEEP, 25, Some(E::Rattled)),
            rule(S::Locked, KEEP, 60, Some(E::Rattled)),
            rule(S::Open, OPEN_ANGLE, 40, Some(E::Blasted)),
            broke,
        ],
        // Wedged.
        [
            rule(S::Wedged, KEEP, 0, Some(E::Held)),
            rule(S::Wedged, KEEP, 0, None),
            rule(S::Wedged, KEEP, 10, Some(E::Held)),
            rule(S::Wedged, KEEP, 40, Some(E::Held)),
            rule(S::Wedged, KEEP, 40, Some(E::Blasted)),
            broke,
        ],
        // Trapped: whatever opens it fires the device.
        [
            rule(S::Ajar, AJAR_ANGLE, 0, Some(E::TrapSprung)),
            rule(S::Trapped, KEEP, 0, Some(E::TrapSpotted)),
            rule(S::Open, OPEN_ANGLE, 0, Some(E::TrapSprung)),
            rule(S::Open, OPEN_ANGLE, 10, Some(E::TrapSprung)),
            rule(S::Open, OPEN_ANGLE, 40, Some(E::TrapSprung)),
            rule(S::Broken, OPEN_ANGLE, WRECK, Some(E::TrapSprung)),
        ],
        // Broken: nothing left to do.
        [rule(S::Broken, KEEP, 0, None); 6],
    ]
};

/// One hinged door.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Door {
    state: DoorState,
    angle: i16,
    hp: u16,
}

impl Door {
    /// A fresh door in `state`, at the angle that state implies.
    pub const fn new(state: DoorState) -> Self {
        let (angle, hp) = match state {
            DoorState::Ajar => (AJAR_ANGLE, DOOR_HP),
            DoorState::Open => (OPEN_ANGLE, DOOR_HP),
            DoorState::Broken => (OPEN_ANGLE, 0),
            _ => (0, DOOR_HP),
        };
        Self { state, angle, hp }
    }

    /// Where the door stands.
    pub const fn state(&self) -> DoorState {
        self.state
    }

    /// Leaf angle, decidegrees, `0..=`[`OPEN_ANGLE`].
    pub const fn angle(&self) -> i16 {
        self.angle
    }

    /// Integrity left, `0..=`[`DOOR_HP`].
    pub const fn hp(&self) -> u16 {
        self.hp
    }

    /// True when a fighter fits through.
    pub const fn passable(&self) -> bool {
        matches!(self.state, DoorState::Broken) || self.angle >= PASS_ANGLE
    }

    /// True while the leaf stops casts. A broken door does not.
    pub const fn blocks_casts(&self) -> bool {
        !matches!(self.state, DoorState::Broken)
    }

    /// The collision layer the door's collider takes: the world while the leaf stands, debris
    /// once it is broken (which blocks casts scoped to the world and fighters alike).
    pub const fn layer(&self) -> u32 {
        if self.blocks_casts() {
            layers::WORLD
        } else {
            layers::DEBRIS
        }
    }

    /// Apply one action: look up the table row, take its damage, then let a holding door give
    /// way or a worn one break. Returns the event and the nav link change.
    pub fn apply(&mut self, action: DoorAction) -> Outcome {
        let was_passable = self.passable();
        let row = RULES[self.state.index()][action.index()];
        let mut event = row.event;
        self.hp = self.hp.saturating_sub(row.damage);
        self.state = row.next;
        if row.angle != KEEP {
            self.angle = row.angle;
        }
        if self.hp == 0 && row.next != DoorState::Broken {
            self.state = DoorState::Broken;
            self.angle = OPEN_ANGLE;
            event = Some(DoorEvent::Broke);
        } else if self.hp <= GIVE_HP && matches!(row.next, DoorState::Locked | DoorState::Wedged) {
            event = Some(if row.next == DoorState::Locked {
                DoorEvent::LockGave
            } else {
                DoorEvent::WedgeGave
            });
            self.state = DoorState::Open;
            self.angle = OPEN_ANGLE;
        }
        let link = match (was_passable, self.passable()) {
            (false, true) => Link::Opened,
            (true, false) => Link::Closed,
            _ => Link::Unchanged,
        };
        Outcome { event, link }
    }

    /// FNV-1a over the whole door state, for tapes and checkpoints.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_len(self.state.index());
        h.write_u16(self.angle.unsigned_abs());
        h.write_u16(self.hp);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collider::tests::cube_hull;
    use crate::collider::Collider;
    use crate::query::ray_cast;
    use engine_core::Rng;

    #[test]
    fn every_state_by_action_fixture_replays_x2() {
        for state in DoorState::ALL {
            for action in DoorAction::ALL {
                let (mut a, mut b) = (Door::new(state), Door::new(state));
                let (oa, ob) = (a.apply(action), b.apply(action));
                assert_eq!((a, oa, a.hash()), (b, ob, b.hash()), "{state:?} {action:?}");
                assert!(
                    (0..=OPEN_ANGLE).contains(&a.angle()),
                    "{state:?} {action:?}"
                );
                assert!(a.hp() <= DOOR_HP);
                if state == DoorState::Broken {
                    assert_eq!(
                        (a, oa.event),
                        (Door::new(state), None),
                        "broken is terminal"
                    );
                }
                if action == DoorAction::Charge {
                    assert_eq!(a.state(), DoorState::Broken, "{state:?} survived a charge");
                }
            }
        }
    }

    #[test]
    fn a_seeded_tape_of_blows_replays_x2_and_never_mends() {
        for state in DoorState::ALL {
            let run = || {
                let mut rng = Rng::new(2040 + u64::try_from(state.index()).expect("small"));
                let mut door = Door::new(state);
                let mut hashes = Vec::new();
                let mut hp = door.hp();
                for _ in 0..200 {
                    let action = DoorAction::ALL[usize::try_from(rng.next_u64() % 6).unwrap_or(0)];
                    door.apply(action);
                    assert!(door.hp() <= hp, "{state:?}: hp rose");
                    hp = door.hp();
                    hashes.push(door.hash());
                }
                (door, hashes)
            };
            assert_eq!(run(), run(), "{state:?}");
        }
    }

    #[test]
    fn a_wedged_door_holds_against_a_push() {
        let mut door = Door::new(DoorState::Wedged);
        for _ in 0..50 {
            let out = door.apply(DoorAction::Push);
            assert_eq!(out.event, Some(DoorEvent::Held));
            assert_eq!(out.link, Link::Unchanged);
        }
        assert_eq!(door, Door::new(DoorState::Wedged));
        assert!(!door.passable());
        // Rams do wear a wedge down: the second one drops it to 20 and the door gives.
        door.apply(DoorAction::Ram);
        assert_eq!(door.state(), DoorState::Wedged);
        let out = door.apply(DoorAction::Ram);
        assert_eq!(out.event, Some(DoorEvent::WedgeGave));
        assert_eq!((door.state(), out.link), (DoorState::Open, Link::Opened));
    }

    #[test]
    fn a_locked_door_takes_three_kicks_and_one_shotgun_round() {
        let mut door = Door::new(DoorState::Locked);
        assert_eq!(door.apply(DoorAction::Push).event, Some(DoorEvent::Rattled));
        door.apply(DoorAction::Kick);
        door.apply(DoorAction::Kick);
        assert_eq!(door.state(), DoorState::Locked);
        let out = door.apply(DoorAction::Kick);
        assert_eq!(out.event, Some(DoorEvent::LockGave));
        assert_eq!((door.state(), door.angle()), (DoorState::Open, OPEN_ANGLE));
        let mut shot = Door::new(DoorState::Locked);
        assert_eq!(shot.apply(DoorAction::Shotgun).link, Link::Opened);
        assert_eq!(shot.state(), DoorState::Open);
    }

    #[test]
    fn a_trap_fires_once_and_a_peek_sees_it_first() {
        let mut door = Door::new(DoorState::Trapped);
        let seen = door.apply(DoorAction::Peek);
        assert_eq!(seen.event, Some(DoorEvent::TrapSpotted));
        assert_eq!(door.state(), DoorState::Trapped);
        assert_eq!(
            door.apply(DoorAction::Push).event,
            Some(DoorEvent::TrapSprung)
        );
        assert_eq!(door.apply(DoorAction::Push).event, Some(DoorEvent::Swung));
    }

    #[test]
    fn blows_wear_a_door_down_to_broken() {
        let mut door = Door::new(DoorState::Closed);
        door.apply(DoorAction::Shotgun);
        door.apply(DoorAction::Shotgun);
        assert_eq!((door.state(), door.hp()), (DoorState::Open, 20));
        let out = door.apply(DoorAction::Shotgun);
        assert_eq!(out.event, Some(DoorEvent::Broke));
        assert_eq!(door.state(), DoorState::Broken);
        assert_eq!(
            out.link,
            Link::Unchanged,
            "an open door was already passable"
        );
    }

    #[test]
    fn events_name_nav_changes_and_audio_cues() {
        let mut shut = Door::new(DoorState::Open);
        assert_eq!(shut.apply(DoorAction::Push).link, Link::Closed);
        assert_eq!(shut.apply(DoorAction::Kick).link, Link::Opened);
        assert_eq!(
            Door::new(DoorState::Closed).apply(DoorAction::Peek).link,
            Link::Unchanged
        );
        for state in DoorState::ALL {
            for action in DoorAction::ALL {
                if let Some(event) = Door::new(state).apply(action).event {
                    assert!(!event.cue().is_empty());
                }
            }
        }
    }

    fn door_collider(door: &Door) -> Collider {
        Collider {
            id: "COL_door".to_owned(),
            layer: door.layer(),
            shape: cube_hull(),
        }
    }

    #[test]
    fn broken_doors_stop_blocking_casts() {
        let cast = |door: &Door| {
            let world = [door_collider(door)];
            ray_cast(
                &world,
                [-3.0, 0.5, 0.5],
                [1.0, 0.0, 0.0],
                10.0,
                layers::WORLD,
            )
            .expect("valid cast")
        };
        let mut door = Door::new(DoorState::Locked);
        assert!(cast(&door).is_some(), "a locked door stops a ray");
        door.apply(DoorAction::Charge);
        assert!(!door.blocks_casts() && door.passable());
        assert!(cast(&door).is_none(), "a broken door lets the ray through");
        assert!(!layers::blocks(layers::FIGHTER, door.layer()));
        assert!(layers::blocks(
            layers::FIGHTER,
            Door::new(DoorState::Closed).layer()
        ));
    }
}
