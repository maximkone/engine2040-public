//! text: byte-bounded string clipping that never splits a char boundary.
//! Units: bytes in, a prefix slice out. Determinism: pure, no allocation.

/// Longest char-boundary prefix of `text` within `max` bytes.
///
/// The whole string comes back when it already fits, and an empty prefix when
/// `max` is 0 or the first character needs more bytes. Callers use this to
/// bound notices, streamed text and crash backtraces without producing a
/// `String` that panics on a multi-byte cut.
#[must_use]
pub fn clip_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::clip_bytes;

    #[test]
    fn short_and_exact_strings_come_back_whole() {
        assert_eq!(clip_bytes("", 4), "");
        assert_eq!(clip_bytes("sand", 4), "sand");
        assert_eq!(clip_bytes("sand", 999), "sand");
    }

    #[test]
    fn ascii_clips_to_the_byte_budget() {
        assert_eq!(clip_bytes("sandstorm", 4), "sand");
        assert_eq!(clip_bytes("sandstorm", 0), "");
    }

    #[test]
    fn multibyte_never_splits_a_char() {
        // "é" is two bytes, so a one-byte budget cannot take it.
        assert_eq!(clip_bytes("é", 1), "");
        assert_eq!(clip_bytes("aé", 2), "a");
        assert_eq!(clip_bytes("aé", 3), "aé");
    }

    #[test]
    fn every_prefix_is_a_str_and_fits_the_budget() {
        let text = "héllo";
        for max in 0..=text.len() {
            let got = clip_bytes(text, max);
            assert!(got.len() <= max, "max {max} gave {} bytes", got.len());
            assert!(text.starts_with(got));
        }
    }
}
