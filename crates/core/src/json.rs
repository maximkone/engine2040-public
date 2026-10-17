//! json: JSON string escaping shared by the hand-rolled writers.
//! Units: text in, escaped text appended. Determinism: pure, no allocation
//! beyond the output buffer.

/// Append `text` to `out` with JSON string escaping: `"` `\` and the common
/// controls as short escapes, every other control char below `0x20` as
/// `\u00xx`. The surrounding quotes are the caller's.
pub fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::escape_into;

    #[test]
    fn escapes_quotes_slashes_and_controls() {
        let mut out = String::new();
        escape_into(&mut out, "a\"b\\c\nd\re\tf\u{1}g");
        assert_eq!(out, "a\\\"b\\\\c\\nd\\re\\tf\\u0001g");
    }

    #[test]
    fn plain_text_is_untouched() {
        let mut out = String::from(">");
        escape_into(&mut out, "caf\u{e9} 42");
        assert_eq!(out, ">caf\u{e9} 42");
    }
}
