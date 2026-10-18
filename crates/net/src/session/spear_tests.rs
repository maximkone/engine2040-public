//! Spear grapple over the session: the button and the spec flag ride the
//! existing wire, a resimulation equals a single run, a peer without the flag
//! is refused (`docs/plan/spear-grapple.md`).

use engine_arena::world::MatchWorld;
use engine_arena::{ArenaConfig, MatchSpec};
use engine_core::fixed::Fixed;
use engine_input::fighter::{BLOCK, THROW_SPEAR};
use engine_input::Buttons;

use super::{BodyId, BodyInput, Client, Server};
use crate::lobby::{BuildId, Lobby, LobbyError};

const TICKS: u32 = 400;
/// Ticks the client predicts ahead of the server's authoring.
const AHEAD: u32 = 3;

fn spec(spear: bool) -> MatchSpec {
    let config = ArenaConfig {
        seed: 0x5EA2,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 12.0,
        table_version: 2,
    };
    let spec = MatchSpec::duel(config, MatchWorld::empty());
    if spear {
        spec.with_spear()
    } else {
        spec
    }
}

/// Facing in centidegrees (0 is +z) from `me` toward `foe`, through the
/// engine's integer `Fixed::atan2` (no platform trig).
fn aim(me: [f32; 3], foe: [f32; 3]) -> i16 {
    let mm = |v: f32| Fixed::from_bits((v * 65_536.0) as i64);
    let angle = Fixed::atan2(mm(foe[0] - me[0]), mm(foe[2] - me[2]));
    i16::try_from(angle.to_bits() * 18_000 / Fixed::PI.to_bits()).unwrap_or(0)
}

/// Both humans' scripted inputs per tick: stand still, face the foe (aimed
/// from a probe match on the same spec), throw every 50 ticks, slot 1 guards.
fn script(spec: &MatchSpec) -> Vec<[BodyInput; 2]> {
    let mut probe = spec.live().expect("probe");
    let mut facing = [0i16; 2];
    let mut tape = Vec::new();
    for tick in 0..TICKS {
        let row: [BodyInput; 2] = [0u8, 1].map(|slot| {
            let mut buttons = 0;
            if tick > 0 && (tick + u32::from(slot) * 25) % 50 == 0 {
                buttons |= THROW_SPEAR;
            }
            if slot == 1 && (tick / 40) % 3 == 2 {
                buttons |= BLOCK;
            }
            BodyInput {
                tick,
                fighter: BodyId::new(slot).expect("slot"),
                mx: 0,
                mz: 0,
                facing: facing[usize::from(slot)],
                buttons: Buttons(buttons),
            }
        });
        let stepped = probe.step(&row).expect("probe step");
        let poses = &stepped.rows[0].observation.fighters;
        facing = [
            aim(poses[0].pos, poses[1].pos),
            aim(poses[1].pos, poses[0].pos),
        ];
        tape.push(row);
    }
    tape
}

/// A server with both humans and a client for slot 0 that never sees slot 1,
/// so the resimulation path runs on a match with spears in the air.
fn bout(spec: &MatchSpec, tape: &[[BodyInput; 2]]) -> (Server, Client) {
    let mut server = Server::from_spec(spec.clone()).expect("server");
    let mut client =
        Client::from_spec(spec.clone(), BodyId::new(0).expect("slot")).expect("client");
    for step in 0..TICKS + AHEAD {
        if step < TICKS {
            client.predict(tape[step as usize][0]).expect("predict");
        }
        if step >= AHEAD {
            let tick = step - AHEAD;
            let (checkpoint, delta) = server.tick(&tape[tick as usize]).expect("server tick");
            client.on_server_state(checkpoint, &delta).expect("state");
            if tick.is_multiple_of(25) {
                client.resimulate();
            }
        }
    }
    client.resimulate();
    (server, client)
}

#[test]
fn spear_resim_equals_a_single_run_and_x2_is_identical() {
    let spec = spec(true);
    let tape = script(&spec);
    let (server, client) = bout(&spec, &tape);
    assert_eq!(server.checkpoints().len(), TICKS as usize);
    assert_eq!(
        client.checkpoints(),
        server.checkpoints(),
        "resim == single run"
    );
    assert!(
        client.total_mispredicts() > 0,
        "the hidden slot mispredicted"
    );
    let (again, _) = bout(&spec, &tape);
    assert_eq!(server.checkpoints(), again.checkpoints(), "x2 identical");
    let mut single = spec.live().expect("live");
    for (tick, row) in tape.iter().enumerate() {
        let stepped = single.step(row).expect("step").checkpoint;
        assert_eq!(stepped, server.checkpoints()[tick], "tick {tick}");
    }
}

#[test]
fn spear_on_plays_another_match_than_spear_off_on_the_same_tape() {
    let on = spec(true);
    let tape = script(&on);
    let (with, _) = bout(&on, &tape);
    let (without, _) = bout(&spec(false), &tape);
    assert_ne!(with.checkpoints().last(), without.checkpoints().last());
}

#[test]
fn spear_lobby_refuses_a_peer_without_the_flag() {
    let host = BuildId::of_spec(1, &spec(true));
    let mut lobby = Lobby::new(host, 2, 7).expect("lobby");
    assert_eq!(lobby.join(0xA, host), Ok(0));
    let plain = BuildId::of_spec(1, &spec(false));
    assert_eq!(
        lobby.join(0xB, plain),
        Err(LobbyError::VersionMismatch {
            want_version: 1,
            got_version: 1,
            want_rules: host.rules,
            got_rules: plain.rules,
        })
    );
}

#[test]
fn spear_browser_duel_host_builds_from_the_kit_spec_with_its_own_build_id() {
    use crate::webduel::{duel_spec, scripted_input, DuelHost};

    let plain = duel_spec().expect("kit duel");
    let armed = plain.clone().with_spear();
    let host = |spec: &MatchSpec| DuelHost::new(spec.clone()).expect("host").build();
    assert_ne!(
        host(&plain),
        host(&armed),
        "a spear page cannot join a plain host"
    );
    let run = || {
        let mut server = Server::from_spec(armed.clone()).expect("server");
        for tick in 0..240 {
            let mut input = scripted_input(tick, 0);
            if tick % 40 == 5 {
                input.buttons = Buttons(input.buttons.0 | THROW_SPEAR);
            }
            server.tick(&[input]).expect("tick");
        }
        server.checkpoints().to_vec()
    };
    assert_eq!(run(), run(), "the kit walls plus spears replay x2");
}
