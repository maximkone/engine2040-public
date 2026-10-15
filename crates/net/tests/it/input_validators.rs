// Donor: cBournhonesque/lightyear PR #1529 (MIT) - server input-validator chain
// pattern (ordered push/extend/validate returning the rejecting name).
// Link: https://github.com/cBournhonesque/lightyear/pull/1529. Pattern only,
// no donor bytes: fresh std-only chain over engine-net anticheat guards.
//! Server input-validator chain: ordered sanitize/tick/move guards reject
//! spoofed inputs with the rejecting name; legit inputs pass clean.

use engine_net::anticheat::{
    check_move, sanitize, RawInput, ALLOWED_BUTTONS, F_OK, F_SPEED, F_TELEPORT, MAX_LOOK,
};

struct Probe {
    raw: RawInput,
    from: [f32; 3],
    to: [f32; 3],
    prev_tick: u32,
}

type Rule = fn(&Probe) -> bool;

struct ValidatorChain {
    validators: Vec<(&'static str, Rule)>,
}

impl ValidatorChain {
    fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }
    fn push(&mut self, name: &'static str, rule: Rule) {
        self.validators.push((name, rule));
    }
    fn extend(&mut self, more: &[(&'static str, Rule)]) {
        for (name, rule) in more {
            self.validators.push((*name, *rule));
        }
    }
    fn validate(&self, probe: &Probe) -> Option<&'static str> {
        for (name, rule) in &self.validators {
            if rule(probe) {
                return Some(*name);
            }
        }
        None
    }
}

fn v_sanitize(p: &Probe) -> bool {
    sanitize(&p.raw).1 != F_OK
}

fn v_move(p: &Probe) -> bool {
    check_move(p.from, p.to) & (F_SPEED | F_TELEPORT) != 0
}

fn v_tick(p: &Probe) -> bool {
    p.raw.tick <= p.prev_tick || p.raw.tick.wrapping_sub(p.prev_tick) > 8
}

fn server_chain() -> ValidatorChain {
    let mut chain = ValidatorChain::new();
    chain.push("sanitize", v_sanitize);
    chain.push("tick", v_tick);
    chain.push("move", v_move);
    chain
}

fn server_chain_via_extend() -> ValidatorChain {
    let mut chain = ValidatorChain::new();
    chain.push("sanitize", v_sanitize);
    chain.extend(&[("tick", v_tick as Rule), ("move", v_move as Rule)]);
    chain
}

fn legit_probe(i: u32) -> Probe {
    let step = 0.05;
    let x = f32::from(i as u16) * step;
    Probe {
        raw: RawInput {
            mx: ((i % 5) as f32 - 2.0) * 0.4,
            my: (((i / 5) % 5) as f32 - 2.0) * 0.4,
            lx: (((i / 25) % 9) as f32 - 4.0) * (MAX_LOOK / 4.0),
            ly: 0.0,
            buttons: (i as u16) & ALLOWED_BUTTONS,
            tick: 101 + i,
        },
        from: [x, 0.0, 0.0],
        to: [x + step, 0.0, 0.0],
        prev_tick: 100 + i,
    }
}

#[test]
fn spoof_chain_rejects_all_three() {
    let chain = server_chain();
    let axis_spoof = Probe {
        raw: RawInput {
            mx: 2.5,
            my: 0.0,
            lx: 0.0,
            ly: 0.0,
            buttons: 1,
            tick: 11,
        },
        from: [0.0; 3],
        to: [0.05, 0.0, 0.0],
        prev_tick: 10,
    };
    let teleport_spoof = Probe {
        raw: RawInput {
            mx: 0.0,
            my: 0.0,
            lx: 0.0,
            ly: 0.0,
            buttons: 1,
            tick: 11,
        },
        from: [0.0; 3],
        to: [50.0, 0.0, 0.0],
        prev_tick: 10,
    };
    let tick_spoof = Probe {
        raw: RawInput {
            mx: 0.0,
            my: 0.0,
            lx: 0.0,
            ly: 0.0,
            buttons: 1,
            tick: 10,
        },
        from: [0.0; 3],
        to: [0.05, 0.0, 0.0],
        prev_tick: 10,
    };
    let cases = [
        (axis_spoof, "sanitize"),
        (teleport_spoof, "move"),
        (tick_spoof, "tick"),
    ];
    let mut rejected = 0;
    for (probe, want) in &cases {
        assert_eq!(chain.validate(probe), Some(*want), "spoof must reject");
        rejected += 1;
    }
    assert_eq!(rejected, 3, "3/3 spoofs must reject");
}

#[test]
fn legit_chain_accepts_forty_eight() {
    let chain = server_chain();
    let mut false_rejects = 0;
    for i in 0..48 {
        let probe = legit_probe(i);
        if chain.validate(&probe).is_some() {
            false_rejects += 1;
        }
    }
    assert_eq!(false_rejects, 0, "0/48 legit inputs must falsely reject");
}

#[test]
fn chain_order_returns_first_rejector() {
    let chain = server_chain();
    let both = Probe {
        raw: RawInput {
            mx: f32::NAN,
            my: 0.0,
            lx: 0.0,
            ly: 0.0,
            buttons: 1,
            tick: 11,
        },
        from: [0.0; 3],
        to: [50.0, 0.0, 0.0],
        prev_tick: 10,
    };
    assert_eq!(chain.validate(&both), Some("sanitize"));
    let via_extend = server_chain_via_extend();
    for i in 0..48 {
        let probe = legit_probe(i);
        assert_eq!(via_extend.validate(&probe), None);
    }
    assert_eq!(via_extend.validate(&both), Some("sanitize"));
}
