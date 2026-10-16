//! Join-gate + skill-label match vectors (S98 Nakama leg).
//!
//! Shape idea from heroiclabs/nakama (Apache-2.0) `sample_go_module`
//! `MatchJoinAttempt` (accept bool + reason) and skill-label `MatchCreate`.
//! PORT(heroiclabs/nakama:sample_go_module) idea-only: fresh implementation,
//! no vendor bytes copied; NOT the sample accept-all policy (this gate
//! rejects over-capacity and out-of-band skill with a typed reason).
//! Owner: net lane. Consumer: S98 Nakama match leg (next). Units: player
//! ids (`u64`), skill (`u32`), tape positions (index). Determinism:
//! integer-only, join order decides slots; no float, clock or map order.
//! Errors: [`GateError`] for bad create input; joins refuse via decision.

use std::fmt::Write as _;

/// Reject reason: typed code that also renders into the tape, never a bare bool.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// Every slot is taken.
    OverCapacity,
    /// Skill sits outside the match band.
    SkillMismatch,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::OverCapacity => write!(f, "over capacity"),
            Self::SkillMismatch => write!(f, "skill out of band"),
        }
    }
}

/// Skill-label create shape: label names the band, bounds enforce it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchCreate {
    /// Human label, e.g. `"ranked:1000-1999"`.
    pub label: &'static str,
    /// Max players, at least 1.
    pub max_players: u8,
    /// Inclusive skill floor.
    pub skill_floor: u32,
    /// Inclusive skill ceiling, must cover the floor.
    pub skill_ceiling: u32,
}

/// Bad create input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GateError {
    /// Empty label, zero cap, or floor above ceiling.
    BadInput,
}

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadInput => write!(f, "bad match create input"),
        }
    }
}

impl std::error::Error for GateError {}

impl MatchCreate {
    /// Range-check the create shape.
    pub fn validate(self) -> Result<Self, GateError> {
        if self.label.is_empty() || self.max_players == 0 {
            return Err(GateError::BadInput);
        }
        if self.skill_floor > self.skill_ceiling {
            return Err(GateError::BadInput);
        }
        Ok(self)
    }

    /// True when `skill` sits inside the labelled band.
    pub const fn admits_skill(self, skill: u32) -> bool {
        skill >= self.skill_floor && skill <= self.skill_ceiling
    }
}

/// One recorded attempt: accept bool plus reason, in arrival order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JoinRecord {
    /// Player id attempted.
    pub player: u64,
    /// Skill presented.
    pub skill: u32,
    /// Accept flag (Nakama `accept`).
    pub accepted: bool,
    /// Refusal reason when rejected.
    pub reason: Option<RejectReason>,
    /// Tape position (index in arrival order).
    pub tape_pos: usize,
}

/// Decision returned per attempt: accept bool + reason + tape position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JoinDecision {
    /// Accept flag.
    pub accepted: bool,
    /// Refusal reason when rejected.
    pub reason: Option<RejectReason>,
    /// Tape position of this attempt.
    pub tape_pos: usize,
}

/// Hook counters: attempt hook fires per try, join hook per accept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HookLog {
    /// Attempt-hook fires.
    pub attempts: u32,
    /// Join-hook fires.
    pub joins: u32,
}

impl HookLog {
    /// Total hook fires.
    pub const fn total(self) -> u32 {
        self.attempts + self.joins
    }
}

/// Capacity + skill gate with an append-only attempt tape.
pub struct JoinGate {
    create: MatchCreate,
    members: Vec<u64>,
    tape: Vec<JoinRecord>,
    hooks: HookLog,
}

impl JoinGate {
    /// Open a gate from a validated create shape.
    pub fn open(create: MatchCreate) -> Result<Self, GateError> {
        let create = create.validate()?;
        let cap = usize::from(create.max_players);
        Ok(Self {
            create,
            members: Vec::with_capacity(cap),
            tape: Vec::new(),
            hooks: HookLog::default(),
        })
    }

    /// Hook fires so far.
    pub const fn hooks(&self) -> HookLog {
        self.hooks
    }

    /// Members admitted so far.
    pub fn members(&self) -> &[u64] {
        &self.members
    }

    /// Attempt tape in arrival order.
    pub fn tape(&self) -> &[JoinRecord] {
        &self.tape
    }

    /// Attempt one join: capacity first, then skill band. Always tapes the
    /// attempt with its position; hooks fire attempt-then-join-on-accept.
    /// Rejects carry a reason; accepts carry none.
    pub fn join_attempt(&mut self, player: u64, skill: u32) -> JoinDecision {
        self.hooks.attempts += 1;
        let verdict = if self.members.len() >= usize::from(self.create.max_players) {
            (false, Some(RejectReason::OverCapacity))
        } else if !self.create.admits_skill(skill) {
            (false, Some(RejectReason::SkillMismatch))
        } else {
            self.members.push(player);
            self.hooks.joins += 1;
            (true, None)
        };
        let tape_pos = self.tape.len();
        self.tape.push(JoinRecord {
            player,
            skill,
            accepted: verdict.0,
            reason: verdict.1,
            tape_pos,
        });
        JoinDecision {
            accepted: verdict.0,
            reason: verdict.1,
            tape_pos,
        }
    }
}

/// One matchmaking ticket: player plus skill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ticket {
    /// Player id.
    pub player: u64,
    /// Skill rating.
    pub skill: u32,
}

/// Place tickets by skill-label filter: first gate whose band admits the
/// skill gets the attempt. Returns placed count.
pub fn place_tickets(gates: &mut [JoinGate], tickets: &[Ticket]) -> usize {
    let mut placed = 0_usize;
    for ticket in tickets {
        for gate in gates.iter_mut() {
            if gate.create.admits_skill(ticket.skill) {
                let call = gate.join_attempt(ticket.player, ticket.skill);
                if call.accepted {
                    placed += 1;
                }
                break;
            }
        }
    }
    placed
}

fn ranked_low() -> MatchCreate {
    MatchCreate {
        label: "ranked:0-999",
        max_players: 4,
        skill_floor: 0,
        skill_ceiling: 999,
    }
}

fn ranked_high() -> MatchCreate {
    MatchCreate {
        label: "ranked:1000-1999",
        max_players: 4,
        skill_floor: 1000,
        skill_ceiling: 1999,
    }
}

fn eight_tickets() -> [Ticket; 8] {
    [
        Ticket {
            player: 1,
            skill: 100,
        },
        Ticket {
            player: 2,
            skill: 250,
        },
        Ticket {
            player: 3,
            skill: 500,
        },
        Ticket {
            player: 4,
            skill: 900,
        },
        Ticket {
            player: 5,
            skill: 1100,
        },
        Ticket {
            player: 6,
            skill: 1300,
        },
        Ticket {
            player: 7,
            skill: 1500,
        },
        Ticket {
            player: 8,
            skill: 1900,
        },
    ]
}

#[test]
fn over_capacity_join_rejected_with_reason_and_tape_pos() {
    let create = MatchCreate {
        label: "duel:1000-1999",
        max_players: 1,
        skill_floor: 1000,
        skill_ceiling: 1999,
    };
    let mut gate = JoinGate::open(create).expect("create validates");
    let first = gate.join_attempt(7, 1200);
    assert!(first.accepted, "in-band first join accepts");
    assert_eq!(first.reason, None, "accept carries no reason");
    assert_eq!(first.tape_pos, 0, "first attempt tapes at 0");
    let second = gate.join_attempt(9, 1300);
    assert!(!second.accepted, "over-capacity join must reject");
    assert_eq!(
        second.reason,
        Some(RejectReason::OverCapacity),
        "reason names capacity"
    );
    assert_eq!(second.tape_pos, 1, "second attempt tapes at 1");
    let tape = gate.tape();
    assert_eq!(tape.len(), 2, "tape holds both attempts");
    assert_eq!(tape[1].tape_pos, 1, "tape position matches arrival order");
    assert_eq!(
        tape[1].reason,
        Some(RejectReason::OverCapacity),
        "tape keeps the reason"
    );
    let mut text = String::new();
    write!(
        &mut text,
        "{}",
        tape[1].reason.expect("rejected tape has reason")
    )
    .expect("reason renders");
    assert!(!text.is_empty(), "reason renders non-empty");
    assert_eq!(text, "over capacity", "reason text is the capacity line");
    let rejected = tape.iter().filter(|r| !r.accepted).count();
    assert_eq!(rejected, 1, "rejected/attempted is 1/1 past the fill");
    assert_eq!(gate.members(), &[7], "only the first player is a member");
}

#[test]
fn skill_label_tickets_place_eight_of_eight() {
    let mut gates = [
        JoinGate::open(ranked_low()).expect("low band validates"),
        JoinGate::open(ranked_high()).expect("high band validates"),
    ];
    let tickets = eight_tickets();
    let placed = place_tickets(&mut gates, &tickets);
    assert_eq!(placed, 8, "skill tickets placed 8/8");
    assert_eq!(gates[0].members().len(), 4, "low band takes its 4");
    assert_eq!(gates[1].members().len(), 4, "high band takes its 4");
    let stray = [Ticket {
        player: 99,
        skill: 9999,
    }];
    let extra = place_tickets(&mut gates, &stray);
    assert_eq!(extra, 0, "out-of-band ticket places 0");
    let mut solo = JoinGate::open(ranked_low()).expect("low band validates");
    let call = solo.join_attempt(50, 1500);
    assert!(!call.accepted, "high skill must not enter the low band");
    assert_eq!(
        call.reason,
        Some(RejectReason::SkillMismatch),
        "reason names skill"
    );
}

#[test]
fn hooks_fire_sixteen_of_sixteen() {
    let mut gates = [
        JoinGate::open(ranked_low()).expect("low band validates"),
        JoinGate::open(ranked_high()).expect("high band validates"),
    ];
    let tickets = eight_tickets();
    let placed = place_tickets(&mut gates, &tickets);
    assert_eq!(placed, 8, "8 tickets place first");
    let mut attempts = 0_u32;
    let mut joins = 0_u32;
    for gate in &gates {
        attempts += gate.hooks().attempts;
        joins += gate.hooks().joins;
    }
    assert_eq!(attempts, 8, "attempt hooks fire 8/8");
    assert_eq!(joins, 8, "join hooks fire 8/8");
    assert_eq!(attempts + joins, 16, "hooks fire 16/16");
    for gate in &gates {
        assert_eq!(
            gate.hooks().total(),
            gate.tape().len() as u32 + gate.members().len() as u32
        );
    }
}
