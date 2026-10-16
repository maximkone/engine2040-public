use serde_json::Value;

use super::anim_door::AnimDoor;
use super::door::{graph_set, parse_script, preview, PreviewError, PreviewScript};
use super::json;
use super::tests::fighter;
use super::*;

#[test]
fn anim_door_sets_undoes_and_previews_without_applying() {
    let mut door = AnimDoor::new();
    assert_eq!(
        door.undo().expect_err("nothing yet").code,
        "nothing_to_undo"
    );
    assert_eq!(
        door.preview(None, r#"{"ticks":5}"#)
            .expect_err("no graph")
            .code,
        "no_graph"
    );
    let first = json::to_json(&fighter());
    let set = door.set(&first).expect("first set");
    assert_eq!(set["before"], Value::Null);
    assert_eq!(set["states"], 8);
    let mut second = fighter();
    second.transitions[0].fade_ticks = 9;
    let set2 = door.set(&json::to_json(&second)).expect("second set");
    assert_eq!(set2["before"], set["after"]);
    let ghost_target = first.replace("\"to\":\"walk\"", "\"to\":\"ghost\"");
    for text in [ghost_target.as_str(), "nonsense", "{\"version\":1}"] {
        let refused = door.set(text).expect_err("refused");
        assert!(
            refused.code == "bad_document" || refused.code == "invalid_machine",
            "{refused:?}"
        );
        assert!(!refused.message.is_empty() && !refused.recovery.is_empty());
        assert_eq!(door.get(), Some(json::to_value(&second)), "last good kept");
    }
    let back = door.undo().expect("undo");
    assert_eq!(
        back["after"], set["after"],
        "undo restores the first digest"
    );
    let script = r#"{"ticks":120,"params":[{"at":3,"name":"speed","value":4}]}"#;
    let a = door.preview(None, script).expect("preview");
    assert_eq!(
        a,
        door.preview(None, script).expect("again"),
        "preview x2 equal"
    );
    assert_eq!(
        door.get(),
        Some(json::to_value(&fighter())),
        "preview applied nothing"
    );
    assert_eq!(a["events"][0]["to"], "walk");
    let candidate = door
        .preview(Some(&json::to_json(&second)), script)
        .expect("candidate");
    assert_ne!(
        candidate["digest"], a["digest"],
        "a different fade changes the run"
    );
    assert_eq!(
        door.preview(None, "{\"ticks\":-1}").expect_err("bad").code,
        "bad_script"
    );
    let ghost = r#"{"ticks":5,"travel":[{"at":1,"to":"ghost"}]}"#;
    assert_eq!(
        door.preview(None, ghost).expect_err("play").code,
        "preview_failed"
    );
}

#[test]
fn json_round_trip_keeps_the_digest() {
    let m = fighter();
    let text = json::to_json(&m);
    let back = json::parse(&text).expect("parse");
    assert_eq!(back, m);
    assert_eq!(back.digest(), m.digest());
    assert_eq!(json::to_json(&back), text, "canonical text is stable");
}

#[test]
fn json_refuses_unknown_fields_floats_and_bad_choices() {
    let good = json::to_json(&fighter());
    let with_extra = good.replacen("\"name\":\"fighter\"", "\"name\":\"fighter\",\"mood\":1", 1);
    assert!(matches!(
        json::parse(&with_extra),
        Err(StateMachineError::Json {
            reason: "unknown field",
            ..
        })
    ));
    let float = good.replacen("\"length_ticks\":20", "\"length_ticks\":20.5", 1);
    assert!(matches!(
        json::parse(&float),
        Err(StateMachineError::Json { .. })
    ));
    let choice = good.replacen("\"immediate\"", "\"teleport\"", 1);
    let err = json::parse(&choice).expect_err("bad switch");
    assert!(err.to_string().contains("transitions[0].switch"), "{err}");
    assert!(json::parse("{not json").is_err());
}

#[test]
fn graph_set_is_undoable_and_a_bad_reload_keeps_the_last_good() {
    let mut doc = Document::new(fighter()).expect("doc");
    let d0 = doc.def().digest();
    let mut next = fighter();
    next.transitions[0].fade_ticks = 9;
    let receipt = graph_set(&mut doc, &json::to_json(&next)).expect("set");
    assert_eq!((receipt.before, receipt.after), (d0, next.digest()));
    assert_eq!((receipt.states, receipt.transitions), (8, 14));
    let ghost = json::to_json(&fighter()).replace("\"to\":\"walk\"", "\"to\":\"ghost\"");
    for bad in ["{\"version\":1}", "nonsense", ghost.as_str()] {
        assert!(graph_set(&mut doc, bad).is_err());
        assert_eq!(doc.def().digest(), next.digest(), "last good kept");
    }
    doc.undo().expect("undo");
    assert_eq!(doc.def().digest(), d0);
}

#[test]
fn preview_reports_transitions_without_touching_the_document() {
    let doc = Document::new(fighter()).expect("doc");
    let d0 = doc.def().digest();
    let script = parse_script(
        r#"{"ticks":200,"params":[{"at":5,"name":"speed","value":4},{"at":90,"name":"speed","value":0}],"travel":[{"at":100,"to":"dead"}]}"#,
    )
    .expect("script");
    let a = preview(doc.def(), &script).expect("preview");
    let b = preview(doc.def(), &script).expect("preview");
    assert_eq!(a, b, "replays equal");
    assert_eq!(doc.def().digest(), d0);
    let route: Vec<&str> = a.events.iter().map(|e| e.to.as_str()).collect();
    assert_eq!(route.first(), Some(&"walk"));
    assert_eq!(a.final_state, "dead");
    assert!(matches!(
        preview(
            doc.def(),
            &PreviewScript {
                ticks: 20_000,
                ..PreviewScript::default()
            }
        ),
        Err(PreviewError::TooLong(20_000))
    ));
    let bad = PreviewScript {
        ticks: 5,
        travel: vec![(1, "ghost".into())],
        ..PreviewScript::default()
    };
    assert!(matches!(
        preview(doc.def(), &bad),
        Err(PreviewError::Play { tick: 1, .. })
    ));
    assert!(parse_script("{\"ticks\":-1}").is_err());
}
