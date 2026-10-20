//! BX11: the event-to-sound table covers every material pair, and nothing
//! sounds without an event.
//!
//! Consumer: the arena presentation side maps a cut, a break or a collapse
//! onto a [`SoundEvent`] and queues it on the feed; this test drives the whole
//! table through the shipped cue bank, then drives one event of every kind
//! through the real engine and replays it.

use std::sync::Arc;

use engine_audio::combat_cues::CuePlay;
use engine_audio::strike_sound::{
    cue_for, queue_sound, Contact, SoundEvent, Struck, COLLAPSE_TABLE, CUT_TABLE, GROAN_TABLE,
    LANDING_TABLE, SEVER_TABLE,
};
use engine_audio::{AudioEngine, CueBank, FeedStats, FrameSource};

/// Where every strike in this test happens: two meters east of the listener.
const AT: [i32; 3] = [200, 0, 0];

fn bank() -> Arc<CueBank> {
    Arc::new(CueBank::placeholder(48_000).expect("shipped table is valid"))
}

/// One cue play must name a real bank cue at a level the mixer keeps.
fn assert_pickable(bank: &CueBank, label: &str, play: CuePlay) {
    assert!(!play.name.is_empty(), "{label}: empty cue name");
    assert!(
        bank.id(play.name).is_some(),
        "{label}: `{}` is no cue in the shipped bank",
        play.name
    );
    assert!(
        play.gain > 0.0 && play.gain <= 1.0,
        "{label}: gain {} outside (0, 1]",
        play.gain
    );
    assert_eq!(play.pos_cm, Some(AT), "{label}: the event's source");
}

#[test]
fn every_cut_material_pair_has_a_cue_in_the_shipped_bank() {
    let bank = bank();
    let mut pairs = 0;
    for contact in Contact::ALL {
        for struck in Struck::ALL {
            let event = SoundEvent::Cut { contact, struck };
            assert_pickable(
                &bank,
                &format!("{contact:?} on {struck:?}"),
                cue_for(event, AT),
            );
            pairs += 1;
        }
    }
    assert_eq!(
        pairs,
        Contact::COUNT * Struck::COUNT,
        "the table is one row per pair"
    );
    assert_eq!(
        (CUT_TABLE.len(), CUT_TABLE[0].len()),
        (Contact::COUNT, Struck::COUNT),
        "the cut table has one cell per material pair"
    );
}

/// Event constructors for the per-material kinds, as plain fn pointers so one
/// array can hold all four.
fn sever(struck: Struck) -> SoundEvent {
    SoundEvent::Sever { struck }
}

fn landing(struck: Struck) -> SoundEvent {
    SoundEvent::LimbLanding { struck }
}

fn groan(struck: Struck) -> SoundEvent {
    SoundEvent::Groan { struck }
}

fn collapse(struck: Struck) -> SoundEvent {
    SoundEvent::Collapse { struck }
}

/// Builds one event kind for a material.
type EventMaker = fn(Struck) -> SoundEvent;

#[test]
fn every_event_kind_sounds_for_every_material() {
    let bank = bank();
    let kinds: [(&str, usize, EventMaker); 4] = [
        ("sever", SEVER_TABLE.len(), sever),
        ("landing", LANDING_TABLE.len(), landing),
        ("groan", GROAN_TABLE.len(), groan),
        ("collapse", COLLAPSE_TABLE.len(), collapse),
    ];
    for (label, rows, event) in kinds {
        assert_eq!(rows, Struck::COUNT, "{label}: one row per material");
        for struck in Struck::ALL {
            assert_pickable(
                &bank,
                &format!("{label} on {struck:?}"),
                cue_for(event(struck), AT),
            );
        }
    }
}

/// A body event carries no material: a stump is a stump whether the blade met
/// bone or armor, and it still has to sound.
#[test]
fn the_stump_sounds_without_a_material_axis() {
    let bank = bank();
    let bone = cue_for(
        SoundEvent::Sever {
            struck: Struck::Bone,
        },
        AT,
    );
    let armor = cue_for(
        SoundEvent::Sever {
            struck: Struck::Armor,
        },
        AT,
    );
    assert_pickable(&bank, "sever on bone", bone);
    assert_pickable(&bank, "sever on armor", armor);
}

/// Every cell of every table is filled: a zero gain or an empty name would be
/// a silent event.
#[test]
fn no_table_cell_is_silent() {
    for (contact, row) in Contact::ALL.iter().zip(CUT_TABLE.iter()) {
        for (struck, cell) in Struck::ALL.iter().zip(row.iter()) {
            assert!(
                !cell.name.is_empty() && cell.gain > 0.0,
                "{contact:?} on {struck:?} is a silent cell"
            );
        }
    }
    for (label, table) in [
        ("sever", &SEVER_TABLE[..]),
        ("landing", &LANDING_TABLE[..]),
        ("groan", &GROAN_TABLE[..]),
        ("collapse", &COLLAPSE_TABLE[..]),
    ] {
        for (struck, cell) in Struck::ALL.iter().zip(table.iter()) {
            assert!(
                !cell.name.is_empty() && cell.gain > 0.0,
                "{label} on {struck:?} is a silent cell"
            );
        }
    }
}

/// `cue_for` is pure data: the same event and source give the same play.
#[test]
fn mapping_is_pure_data() {
    for struck in Struck::ALL {
        let event = SoundEvent::Cut {
            contact: Contact::Edge,
            struck,
        };
        assert_eq!(cue_for(event, AT), cue_for(event, AT), "{struck:?}");
    }
}

/// The five kinds the row names: a cut, a sever (stump), a limb landing, a
/// structure groan and a collapse.
fn script_events() -> [SoundEvent; 5] {
    [
        SoundEvent::Cut {
            contact: Contact::Edge,
            struck: Struck::Bone,
        },
        SoundEvent::Sever {
            struck: Struck::Flesh,
        },
        SoundEvent::LimbLanding {
            struck: Struck::Armor,
        },
        SoundEvent::Groan {
            struck: Struck::Timber,
        },
        SoundEvent::Collapse {
            struck: Struck::Stone,
        },
    ]
}

fn run_script(seed: u64) -> (FeedStats, engine_audio::MixerStats, Vec<f32>) {
    let bank = bank();
    let (mut tx, mut engine) = AudioEngine::with_feed(Arc::clone(&bank), 64, 48_000, seed);
    tx.listener(0, [0, 0, 0], 0).expect("room");
    for (tick, event) in (1u32..).zip(script_events()) {
        queue_sound(&mut tx, tick, event, AT).expect("room");
    }
    let mut out = vec![0.0f32; 2 * 4_800];
    engine.render_stereo_into(&mut out);
    (tx.stats(), engine.mixer().mixer_stats(), out)
}

#[test]
fn the_five_event_kinds_reach_the_engine_and_replay_per_seed() {
    let (feed, mix, first) = run_script(2040);
    assert_eq!(
        (feed.sent, feed.applied),
        (6, 6),
        "listener plus five events"
    );
    assert_eq!((feed.dropped_full, feed.unknown_cue), (0, 0));
    assert_eq!(feed.last_applied_tick, 5);
    assert_eq!(mix.started, 5, "every event reached the mixer");
    assert_eq!(mix.rejected, 0, "each event got a voice");
    assert!(first.iter().any(|&s| s != 0.0), "the hits are audible");
    let (_, _, again) = run_script(2040);
    assert_eq!(first, again, "same seed and script replay the same bytes");
}

/// A full feed refuses the event and counts it; nothing else happens.
#[test]
fn a_full_feed_refuses_the_event_and_counts_it() {
    let bank = bank();
    // A feed holds at least two commands (its capacity rounds up).
    let (mut tx, _engine) = AudioEngine::with_feed(Arc::clone(&bank), 2, 48_000, 2040);
    for tick in 0..2 {
        tx.play(tick, engine_audio::cue::names::HIT, 1.0, None)
            .expect("room");
    }
    let event = SoundEvent::Groan {
        struck: Struck::Timber,
    };
    assert_eq!(
        queue_sound(&mut tx, 2, event, AT),
        Err(engine_audio::FeedError::Full)
    );
    assert_eq!(tx.stats().dropped_full, 1);
}
