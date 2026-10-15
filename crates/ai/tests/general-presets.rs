//! general-presets: ArenaPolicy phase/role/feint presets over the scripted core.
//!
//! Units: ticks are fixed steps, distances are metres in Percept and
//! millimetres in ArenaPolicy::update_feint. Determinism: same percept
//! sequence through same-policy brains gives byte-identical inputs.
//! Errors: unknown phase/role bytes refuse via DepthError; decides always
//! pass FighterInput::validate.

use engine_ai::{ArenaPolicy, DepthBrain, Percept, Phase, Role, Squad};
use engine_input::fighter::ATTACK;
use engine_input::FighterId;

fn fighter(id: u8) -> FighterId {
    FighterId::new(id).expect("slot is legal")
}

fn percept(tick: u32, dist_m: f32, foe_attacking: bool) -> Percept {
    Percept::new(
        [0.0, 0.0, 0.0],
        0.0,
        [dist_m, 0.0, 0.0],
        foe_attacking,
        dist_m,
        tick,
    )
}

#[test]
fn phase_thirds_drive_bias_order() {
    // Round thirds: open, then pressure, then finish. Zero length stays open.
    assert_eq!(Phase::for_tick(0, 900), Phase::Open);
    assert_eq!(Phase::for_tick(299, 900), Phase::Open);
    assert_eq!(Phase::for_tick(300, 900), Phase::Pressure);
    assert_eq!(Phase::for_tick(599, 900), Phase::Pressure);
    assert_eq!(Phase::for_tick(600, 900), Phase::Finish);
    assert_eq!(Phase::for_tick(899, 900), Phase::Finish);
    assert_eq!(Phase::for_tick(0, 0), Phase::Open);
    // Round-trip bytes for tapes.
    assert_eq!(Phase::from_u8(0), Ok(Phase::Open));
    assert_eq!(Phase::from_u8(1), Ok(Phase::Pressure));
    assert_eq!(Phase::from_u8(2), Ok(Phase::Finish));
    assert!(Phase::from_u8(3).is_err());
    assert_eq!(Phase::Open.as_u8(), 0);
    assert_eq!(Phase::Pressure.as_u8(), 1);
    assert_eq!(Phase::Finish.as_u8(), 2);
    // Finish commits hardest, open probes: ordered attack lane.
    let open = Phase::Open.score_bias();
    let pressure = Phase::Pressure.score_bias();
    let finish = Phase::Finish.score_bias();
    assert!(finish[0] > pressure[0]);
    assert!(pressure[0] > open[0]);
    assert!(open[3] > finish[3]);
}

#[test]
fn holder_pins_center() {
    // Squad seats the lowest id as the single holder; the rest flank.
    let squad = Squad::new(&[fighter(2), fighter(0), fighter(3)]).expect("seats");
    assert_eq!(squad.members(), &[fighter(0), fighter(2), fighter(3)]);
    assert_eq!(squad.holder(), fighter(0));
    assert_eq!(squad.role_of(fighter(0)), Ok(Role::Holder));
    assert_eq!(squad.role_of(fighter(2)), Ok(Role::Flanker));
    // Holder bias pins: block lane tops the table, advance gives ground.
    assert_eq!(Role::Holder.score_bias(), [25, 75, -25, -25, 0]);
    assert_eq!(Role::Holder.score_bias()[1], 75);
    // Same phase, holder blocks harder than a flanker.
    let mut holder = ArenaPolicy::new();
    holder.set_phase(Phase::Pressure);
    holder.set_role(Role::Holder);
    let mut flanker = ArenaPolicy::new();
    flanker.set_phase(Phase::Pressure);
    flanker.set_role(Role::Flanker);
    assert!(holder.bias()[1] > flanker.bias()[1]);
    // Fresh holder + pressure golden: phase plus role, no feint, no gate.
    assert_eq!(holder.bias(), [100, 125, 0, -25, -50]);
    // Brain installs the role and reports it back.
    let mut brain = DepthBrain::new(fighter(0), 1);
    brain.set_role(Role::Holder);
    assert_eq!(brain.role(), Role::Holder);
    assert_eq!(brain.policy().role(), Role::Holder);
}

#[test]
fn flanker_circles_for_side_arcs() {
    // Flanker bias circles: advance and dodge lanes top the table.
    assert_eq!(Role::Flanker.score_bias(), [0, -25, 50, 75, 0]);
    assert_eq!(Role::Flanker.score_bias()[3], 75);
    assert_eq!(Role::Flanker.score_bias()[2], 50);
    // New policy starts as a flanker in the open, matching the brain.
    let policy = ArenaPolicy::new();
    assert_eq!(policy.role(), Role::Flanker);
    assert_eq!(policy.phase(), Phase::Open);
    assert!(!policy.is_feinting());
    let brain = DepthBrain::new(fighter(1), 1);
    assert_eq!(brain.role(), Role::Flanker);
    assert_eq!(brain.phase(), Phase::Open);
    // Same phase, flanker advances harder than a holder.
    let mut holder = ArenaPolicy::new();
    holder.set_role(Role::Holder);
    let mut flanker = ArenaPolicy::new();
    flanker.set_role(Role::Flanker);
    assert!(flanker.bias()[3] > holder.bias()[3]);
    assert!(flanker.bias()[2] > holder.bias()[2]);
    // Fresh flanker + open golden: phase plus role, no feint, no gate.
    assert_eq!(flanker.bias(), [-50, 0, 50, 150, 25]);
    // Role bytes round-trip for tapes.
    assert_eq!(Role::from_u8(0), Ok(Role::Holder));
    assert_eq!(Role::from_u8(1), Ok(Role::Flanker));
    assert!(Role::from_u8(2).is_err());
}

#[test]
fn feint_window_opens_holds_then_cools_down() {
    // Direct policy path: close release opens the window.
    let mut policy = ArenaPolicy::new();
    assert!(!policy.is_feinting());
    policy.update_feint(true, false, 2000);
    assert!(!policy.is_feinting());
    policy.update_feint(true, true, 2000);
    assert!(!policy.is_feinting());
    policy.update_feint(false, true, 2000);
    assert!(policy.is_feinting());
    // Open window adds the strike bonus on the attack lane only.
    let biased = policy.bias();
    let mut calm = ArenaPolicy::new();
    calm.set_phase(policy.phase());
    calm.set_role(policy.role());
    calm.set_stamina(policy.stamina());
    let plain = calm.bias();
    assert_eq!(biased[0] - plain[0], 120);
    assert_eq!(&biased[1..], &plain[1..]);
    // Brain path: two close attacking ticks arm, release opens.
    let mut brain = DepthBrain::new(fighter(0), 1);
    brain.decide(&percept(0, 2.0, true));
    brain.decide(&percept(1, 2.0, true));
    assert!(!brain.is_feinting());
    brain.decide(&percept(2, 2.0, false));
    assert!(brain.is_feinting());
    // Window lasts exactly 12 decides, then closes.
    for tick in 3..(3 + 12) {
        let _ = brain.decide(&percept(tick, 2.0, false));
        if tick < 3 + 12 - 1 {
            assert!(brain.is_feinting(), "tick {tick}");
        }
    }
    assert!(!brain.is_feinting(), "window must close");
    // Cooldown blocks an immediate retrigger on the next release.
    brain.decide(&percept(100, 2.0, true));
    brain.decide(&percept(101, 2.0, false));
    assert!(!brain.is_feinting(), "cooldown must block");
}

#[test]
fn feint_ignores_far_releases() {
    // Far releases never feint, on either path.
    let mut policy = ArenaPolicy::new();
    policy.update_feint(true, false, 20000);
    policy.update_feint(false, true, 20000);
    assert!(!policy.is_feinting());
    let mut brain = DepthBrain::new(fighter(0), 1);
    brain.decide(&percept(0, 20.0, true));
    brain.decide(&percept(1, 20.0, false));
    assert!(!brain.is_feinting(), "far releases never feint");
}

#[test]
fn end_round_seats_phase_and_clears_feint() {
    // Open a window, then end the round: phase seats, feint clears.
    let mut policy = ArenaPolicy::new();
    policy.update_feint(true, false, 2000);
    policy.update_feint(true, true, 2000);
    policy.update_feint(false, true, 2000);
    assert!(policy.is_feinting());
    let stamina_before = policy.stamina();
    policy.end_round(Phase::Finish);
    assert_eq!(policy.phase(), Phase::Finish);
    assert!(!policy.is_feinting());
    assert_eq!(policy.stamina(), stamina_before);
    // Same through the brain: stamina carries, feint clears.
    let mut brain = DepthBrain::new(fighter(0), 1);
    brain.decide(&percept(0, 2.0, true));
    brain.decide(&percept(1, 2.0, true));
    brain.decide(&percept(2, 2.0, false));
    assert!(brain.is_feinting());
    brain.end_round(Phase::Finish);
    assert_eq!(brain.phase(), Phase::Finish);
    assert!(!brain.is_feinting());
}

#[test]
fn general_presets_stay_bounded_and_replay() {
    // Every phase/role combo stays inside the posted +/-445 bound.
    for phase in [Phase::Open, Phase::Pressure, Phase::Finish] {
        for role in [Role::Holder, Role::Flanker] {
            let mut policy = ArenaPolicy::new();
            policy.set_phase(phase);
            policy.set_role(role);
            for bias in policy.bias() {
                assert!((-445..=445).contains(&bias), "{phase:?} {role:?}");
            }
        }
    }
    // Same tape twice strikes the same way; every output validates.
    let bout = || {
        let mut brain = DepthBrain::new(fighter(0), 1);
        brain.set_phase(Phase::Finish);
        brain.set_role(Role::Holder);
        brain.decide(&percept(0, 2.0, true));
        brain.decide(&percept(1, 2.0, true));
        let struck = brain.decide(&percept(2, 2.0, false));
        assert!(brain.is_feinting());
        assert_eq!(struck.buttons.0 & ATTACK, ATTACK);
        struck
    };
    let (first, second) = (bout(), bout());
    assert_eq!(first, second);
    first.validate().expect("feint strike validates");
    second.validate().expect("replay validates");
}
