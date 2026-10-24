//! seal_key: one owner for operator-held hex seal keys.
//! Owner: engine-core. Units: bytes as hex text. Determinism: pure parsing,
//! identical on every platform and build. Errors: `SealKeyError`, never panics
//! on input. Budget: off-tick parsing only; key bytes allocate once per parse.

/// Minimum operator key length in bytes; short keys fail loudly.
pub const MIN_SEAL_KEY_BYTES: usize = 32;

/// Why an operator seal key was refused; verification fails closed, never open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealKeyError {
    /// Text is not even-length hexadecimal.
    NotHex,
    /// Decoded key holds fewer than [`MIN_SEAL_KEY_BYTES`] bytes.
    TooShort,
    /// Named environment variable is missing.
    Missing,
}

impl core::fmt::Display for SealKeyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotHex => write!(f, "seal key must be even-length hex"),
            Self::TooShort => write!(f, "seal key is too short"),
            Self::Missing => write!(f, "operator seal key is missing"),
        }
    }
}

impl std::error::Error for SealKeyError {}

/// Even-length hex text decoded to bytes; `None` on odd length or digits
/// outside hex. Shared by key parsing and seal-tag parsing.
#[must_use]
pub fn decode_seal_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() & 1 != 0 {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let part = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(part, 16).ok()
        })
        .collect()
}

/// Parse exactly four ASCII hex digits (a `\uXXXX` payload) as a `u32`.
/// Pure and deterministic on every platform; `None` when the slice is not
/// four bytes or holds a non-hex byte. Shared by the JSON and vault string
/// readers so `\u` escapes decode to identical code points.
#[must_use]
pub fn parse_hex4(digits: &[u8]) -> Option<u32> {
    if digits.len() != 4 {
        return None;
    }
    let mut value: u32 = 0;
    for &byte in digits {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a') + 10,
            b'A'..=b'F' => u32::from(byte - b'A') + 10,
            _ => return None,
        };
        value = value * 16 + digit;
    }
    Some(value)
}

/// Parse an operator seal key from hex text: even-length hex decoding to at
/// least [`MIN_SEAL_KEY_BYTES`] bytes. Pure and deterministic; malformed or
/// short keys fail typed [`SealKeyError`], never truncate or pad.
pub fn parse_seal_key(text: &str) -> Result<Vec<u8>, SealKeyError> {
    match decode_seal_hex(text.trim()) {
        Some(key) if key.len() >= MIN_SEAL_KEY_BYTES => Ok(key),
        Some(_) => Err(SealKeyError::TooShort),
        None => Err(SealKeyError::NotHex),
    }
}

/// Read an operator seal key from the named environment variable. The split
/// from each crate's fixed-variable wrapper keeps the missing-key branch
/// hermetically testable without mutating the process environment.
pub fn seal_key_from_env(var: &str) -> Result<Vec<u8>, SealKeyError> {
    let text = std::env::var(var).map_err(|_| SealKeyError::Missing)?;
    parse_seal_key(&text)
}

/// One-line remediation for a refused operator key: what was wrong plus the
/// variable to set and the minimum length. One owner for the wording, so every
/// crate that parses a key prints the same sentence.
#[must_use]
pub fn seal_key_reason(error: SealKeyError, env: &str) -> String {
    let detail = match error {
        SealKeyError::NotHex => "key must be hex",
        SealKeyError::TooShort => "key is too short",
        SealKeyError::Missing => "operator key is missing",
    };
    format!("{detail}; set {env} to {MIN_SEAL_KEY_BYTES} or more bytes as hex")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_prints_its_own_sentence() {
        assert_eq!(
            SealKeyError::NotHex.to_string(),
            "seal key must be even-length hex"
        );
        assert_eq!(SealKeyError::TooShort.to_string(), "seal key is too short");
        assert_eq!(
            SealKeyError::Missing.to_string(),
            "operator seal key is missing"
        );
    }

    #[test]
    fn minimum_length_key_parses() {
        let key = parse_seal_key(&"0123456789abcdef".repeat(4)).expect("64 hex chars parse");
        assert_eq!(key.len(), MIN_SEAL_KEY_BYTES);
        assert_ne!(key, vec![0u8; MIN_SEAL_KEY_BYTES]);
    }

    #[test]
    fn malformed_or_short_keys_refuse_typed() {
        assert_eq!(parse_seal_key("abc"), Err(SealKeyError::NotHex));
        assert_eq!(
            parse_seal_key("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            Err(SealKeyError::NotHex)
        );
        assert_eq!(parse_seal_key("00ff"), Err(SealKeyError::TooShort));
    }

    #[test]
    fn missing_variable_fails_closed() {
        assert_eq!(
            seal_key_from_env("E2040_CORE_SEAL_KEY_MISSING_IN_TESTS"),
            Err(SealKeyError::Missing)
        );
    }

    #[test]
    fn boundary_lengths_split_at_minimum() {
        let short = "ab".repeat(31);
        assert_eq!(short.len(), 31 * 2);
        assert_eq!(parse_seal_key(&short), Err(SealKeyError::TooShort));
        let exact = "ab".repeat(32);
        let key = parse_seal_key(&exact).expect("32 bytes parse");
        assert_eq!(key.len(), MIN_SEAL_KEY_BYTES);
        let long = "ab".repeat(33);
        let key = parse_seal_key(&long).expect("33 bytes parse");
        assert_eq!(key.len(), MIN_SEAL_KEY_BYTES + 1);
    }

    #[test]
    fn odd_hex_length_refuses_not_hex() {
        assert_eq!(parse_seal_key("0"), Err(SealKeyError::NotHex));
        assert_eq!(decode_seal_hex("abc"), None);
        let odd = "ab".repeat(31) + "a";
        assert_eq!(odd.len() & 1, 1);
        assert_eq!(parse_seal_key(&odd), Err(SealKeyError::NotHex));
        assert_eq!(decode_seal_hex(&odd), None);
    }

    #[test]
    fn distinct_keys_decode_distinctly() {
        let first = parse_seal_key(&"0123456789abcdef".repeat(4)).expect("first key parses");
        let second = parse_seal_key(&"fedcba9876543210".repeat(4)).expect("second key parses");
        assert_eq!(first.len(), MIN_SEAL_KEY_BYTES);
        assert_eq!(second.len(), MIN_SEAL_KEY_BYTES);
        assert_ne!(first, second);
    }

    #[test]
    fn empty_and_blank_refuse_typed() {
        assert_eq!(parse_seal_key(""), Err(SealKeyError::TooShort));
        assert_eq!(parse_seal_key("   "), Err(SealKeyError::TooShort));
        assert_eq!(parse_seal_key("\t\n  "), Err(SealKeyError::TooShort));
    }

    #[test]
    fn reasons_name_the_problem_and_the_variable() {
        let env = "E2040_TEST_SEAL_KEY";
        assert_eq!(
            seal_key_reason(SealKeyError::NotHex, env),
            "key must be hex; set E2040_TEST_SEAL_KEY to 32 or more bytes as hex"
        );
        assert_eq!(
            seal_key_reason(SealKeyError::TooShort, env),
            "key is too short; set E2040_TEST_SEAL_KEY to 32 or more bytes as hex"
        );
        assert_eq!(
            seal_key_reason(SealKeyError::Missing, env),
            "operator key is missing; set E2040_TEST_SEAL_KEY to 32 or more bytes as hex"
        );
    }

    #[test]
    fn shared_four_hex_digits_decode_known_points() {
        assert_eq!(parse_hex4(b"0000"), Some(0x0000));
        assert_eq!(parse_hex4(b"0041"), Some(0x0041));
        assert_eq!(parse_hex4(b"abcd"), Some(0xabcd));
        assert_eq!(parse_hex4(b"ABCD"), Some(0xABCD));
        assert_eq!(parse_hex4(b"FFFF"), Some(0xFFFF));
        assert_eq!(parse_hex4(b"D83D"), Some(0xD83D));
    }

    #[test]
    fn shared_four_hex_digits_refuse_bad_lengths_and_digits() {
        assert_eq!(parse_hex4(b""), None);
        assert_eq!(parse_hex4(b"041"), None);
        assert_eq!(parse_hex4(b"00410"), None);
        assert_eq!(parse_hex4(b"zzzz"), None);
        assert_eq!(parse_hex4(b"12 g"), None);
        assert_eq!(parse_hex4(b"\xc3\xa9\xc3\xa9"), None);
    }
}
