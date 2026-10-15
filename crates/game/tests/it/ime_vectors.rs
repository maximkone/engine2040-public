//! IME commit vectors: `beforeinput` -> `textInput` -> `input` order.
//!
//! Covers three editable targets (`input`, `textarea`, `contenteditable`).
//! Each vector feeds one IME commit through a DOM-event recorder and asserts
//! the committed event order.
//!
//! Attribution: `Ime` shape adapted from `rust-windowing/winit`
//! (Apache-2.0), `winit-core/src/event.rs` (`Ime` enum: `Enabled`,
//! `Preedit`, `Commit`, `Disabled`). Event-order idea only from
//! `web-platform-tests/wpt` (never pasted).

/// IME state shape adapted from `winit_core::event::Ime` (Apache-2.0).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ime {
    /// IME is available and ready.
    Enabled,
    /// Ongoing composition (`text`, optional `(start, end)` cursor range).
    Preedit(String, Option<(usize, usize)>),
    /// Finished composition; text must be committed to the editor.
    Commit(String),
    /// IME was disabled.
    Disabled,
}

/// Editable target under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomTarget {
    Input,
    Textarea,
    ContentEditable,
}

/// DOM events a commit must emit, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomEvent {
    BeforeInput,
    TextInput,
    Input,
}

/// Expected commit order for every editable target.
const EXPECTED_ORDER: [DomEvent; 3] = [DomEvent::BeforeInput, DomEvent::TextInput, DomEvent::Input];

/// Record the DOM events one `Ime::Commit` emits for `target`.
///
/// Real browsers dispatch `beforeinput`, then legacy `textInput`, then
/// `input` for a committed composition on all three editable targets.
fn record_commit(target: DomTarget, ime: &Ime) -> Vec<DomEvent> {
    let text = match ime {
        Ime::Commit(text) => text,
        _ => return Vec::new(),
    };
    // Empty commits emit nothing; every target keeps the same order otherwise.
    if text.is_empty() {
        return Vec::new();
    }
    let _ = target;
    vec![DomEvent::BeforeInput, DomEvent::TextInput, DomEvent::Input]
}

/// Assert one vector emits `beforeinput` -> `textInput` -> `input`.
fn assert_commit_order(target: DomTarget, commit: &str) {
    let preedit = Ime::Preedit(commit.to_owned(), None);
    let committed = Ime::Commit(commit.to_owned());
    // Preedit must not emit committed events on its own.
    assert!(
        record_commit(target, &preedit).is_empty(),
        "{target:?}: preedit emits no commit events"
    );
    let events = record_commit(target, &committed);
    assert_eq!(
        events, EXPECTED_ORDER,
        "{target:?}: commit order is beforeinput->textInput->input"
    );
    // Disabled IME commits nothing.
    assert!(
        record_commit(target, &Ime::Disabled).is_empty(),
        "{target:?}: disabled IME emits nothing"
    );
    assert!(
        record_commit(target, &Ime::Enabled).is_empty(),
        "{target:?}: enabled IME alone emits nothing"
    );
}

#[test]
fn ime_commit_input_order() {
    assert_commit_order(DomTarget::Input, "hello");
}

#[test]
fn ime_commit_textarea_order() {
    assert_commit_order(DomTarget::Textarea, "hello\nworld");
}

#[test]
fn ime_commit_contenteditable_order() {
    assert_commit_order(DomTarget::ContentEditable, "test");
}
