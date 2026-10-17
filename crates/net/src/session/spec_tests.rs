//! DSN1: the server and the client build the match from one [`MatchSpec`].

use engine_arena::{ArenaConfig, LiveMatch, MatchSpec};
use engine_input::fighter::{ATTACK, BLOCK};
use engine_input::Buttons;

use super::{BodyId, BodyInput, Client, Server};
use crate::lobby::{BuildId, Lobby, LobbyError};

const TICKS: u32 = 600;
/// Ticks the client predicts ahead of the server's authoring.
const AHEAD: u32 = 3;

fn config() -> ArenaConfig {
    ArenaConfig {
        seed: 0xD511,
        ticks: TICKS,
        fighters: 8,
        half_extent_m: 18.0,
        table_version: 1,
    }
}

/// Scripted human input: walk back and forth, swing and guard on a cycle.
fn input(tick: u32, slot: u8) -> BodyInput {
    let phase = tick + u32::from(slot) * 7;
    let buttons = match phase % 40 {
        0 => ATTACK,
        20 => BLOCK,
        _ => 0,
    };
    BodyInput {
        tick,
        fighter: BodyId::new(slot).expect("slot"),
        mx: if phase % 60 < 30 { 100 } else { -100 },
        mz: if slot == 0 { 40 } else { -40 },
        facing: 0,
        buttons: Buttons(buttons),
    }
}

/// A server with two humans and a client for slot 0 that predicts ahead and
/// never sees slot 1's input, so the server's truth differs from its
/// prediction and the resimulation path runs.
fn bout(spec: &MatchSpec) -> (Server, Client) {
    let mut server = Server::from_spec(spec.clone()).expect("server");
    let mut client =
        Client::from_spec(spec.clone(), BodyId::new(0).expect("slot 0")).expect("client");
    for step in 0..TICKS + AHEAD {
        if step < TICKS {
            client.predict(input(step, 0)).expect("predict");
        }
        if step >= AHEAD {
            let tick = step - AHEAD;
            let (checkpoint, delta) = server
                .tick(&[input(tick, 0), input(tick, 1)])
                .expect("server tick");
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
fn spec_server_and_client_end_a_600_tick_bout_on_equal_digests() {
    let spec = MatchSpec::kit_duel(config()).expect("kit duel");
    let (server, client) = bout(&spec);
    assert_eq!(server.checkpoints().len(), TICKS as usize);
    assert_eq!(client.checkpoints(), server.checkpoints());
    assert!(
        client.total_mispredicts() > 0,
        "slot 1 was hidden from the client, so it mispredicted"
    );
}

#[test]
fn spec_server_plays_the_spec_not_the_default_match() {
    let spec = MatchSpec::kit_duel(config()).expect("kit duel");
    let (duel, _) = bout(&spec);
    let (v1, _) = bout(&MatchSpec::v1(config()));
    assert_ne!(
        duel.checkpoints().last(),
        v1.checkpoints().last(),
        "rules v2 on the kit walls is not rules v1 on open sand"
    );
    assert_eq!(duel.spec(), &spec);
}

#[test]
fn spec_default_server_is_the_live_new_match() {
    let mut server = Server::new(config()).expect("server");
    let mut live = LiveMatch::new(&config()).expect("live");
    for tick in 0..120 {
        let (checkpoint, _) = server.tick(&[input(tick, 0)]).expect("tick");
        let stepped = live.step(&[input(tick, 0)]).expect("live").checkpoint;
        assert_eq!(checkpoint, stepped, "tick {tick}");
    }
    assert_eq!(server.spec(), &MatchSpec::v1(config()));
}

#[test]
fn spec_lobby_refuses_other_rules_or_another_world_naming_both_digests() {
    let host_spec = MatchSpec::kit_duel(config()).expect("kit duel");
    let host = BuildId::of_spec(1, &host_spec);
    let mut lobby = Lobby::new(host, 2, 7).expect("lobby");
    assert_eq!(lobby.join(0xA, host), Ok(0), "the same spec joins");

    let other_rules = MatchSpec::v1(config());
    let other_world = MatchSpec::duel(config(), engine_arena::world::MatchWorld::empty());
    for other in [other_rules, other_world] {
        let got = BuildId::of_spec(1, &other);
        assert_ne!(got.rules, host.rules);
        assert_eq!(
            lobby.join(0xB, got),
            Err(LobbyError::VersionMismatch {
                want_version: 1,
                got_version: 1,
                want_rules: host.rules,
                got_rules: got.rules,
            })
        );
    }
}
