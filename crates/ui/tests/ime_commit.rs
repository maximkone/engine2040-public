//! IME gate: commits insert at the cursor only when allowed, controls dropped.
use engine_ui::text::{ImeEvent, TextEntry};

#[test]
fn commit_inserts_mid_buffer_when_gated() {
    let mut e = TextEntry::new();
    e.insert_text("ab");
    e.set_cursor(1);
    e.set_ime_allowed(true);
    assert_eq!(e.apply_ime(&ImeEvent::Commit("X".into())), 1);
    assert_eq!(e.text(), "aXb");
    e.set_ime_allowed(false);
    assert_eq!(e.apply_ime(&ImeEvent::Commit("Y".into())), 0);
    assert_eq!(e.text(), "aXb");
}

#[test]
fn commit_drops_control_chars() {
    let mut e = TextEntry::new();
    e.set_ime_allowed(true);
    assert_eq!(e.apply_ime(&ImeEvent::Commit("a\n\tb".into())), 2);
    assert_eq!(e.text(), "ab");
    assert_eq!(e.apply_ime(&ImeEvent::Preedit("x".into())), 0);
    assert_eq!(e.text(), "ab");
}
