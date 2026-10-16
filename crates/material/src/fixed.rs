//! fixed: exact decimal text to Q16 and back, in integer math.
//!
//! The sim half of a material record (toughness, brittleness, burn rate) is
//! written as plain decimals by an AI (`"toughness": 8`, `"brittleness":
//! 0.85`) and read by the tick as Q16 fixed point. No float touches the
//! conversion: a decimal lexeme becomes `round_half_up(N * 65536 / 10^k)`
//! over `u128`, so the same text gives the same Q16 on every host. The
//! reverse picks the shortest decimal that maps back to the same Q16, so
//! `0.85 -> 55706 -> 0.85` and the shipped table rows print as the numbers
//! the designers wrote. Errors are `None`: the record layer names the path.

use engine_destruction::material::Q16_ONE;

/// Most fractional digits accepted; Q16 resolves 1/65536, so six digits
/// already round-trip every value and nine is generous headroom.
pub const MAX_FRACTION_DIGITS: usize = 9;

/// Parse a plain non-negative decimal lexeme (`digits` or `digits.digits`,
/// no sign, no exponent) into Q16, rounding half up. `None` for any other
/// shape, for more than [`MAX_FRACTION_DIGITS`], or for a value at or above
/// `65536.0` (which does not fit `u32` Q16).
#[must_use]
pub fn decimal_to_q16(lexeme: &str) -> Option<u32> {
    let (int_text, frac_text) = match lexeme.split_once('.') {
        Some((i, f)) => (i, f),
        None => (lexeme, ""),
    };
    if int_text.is_empty() || !is_digits(int_text) || !is_digits(frac_text) {
        return None;
    }
    if lexeme.ends_with('.') {
        return None;
    }
    if frac_text.len() > MAX_FRACTION_DIGITS {
        return None;
    }
    let int_part: u128 = int_text.parse().ok()?;
    if int_part >= u128::from(Q16_ONE) {
        return None;
    }
    let frac_part: u128 = if frac_text.is_empty() {
        0
    } else {
        frac_text.parse().ok()?
    };
    let scale = pow10(frac_text.len());
    let numerator = (int_part * scale + frac_part) * u128::from(Q16_ONE);
    let rounded = (numerator + scale / 2) / scale;
    u32::try_from(rounded).ok()
}

/// Shortest decimal text (at most six fractional digits) that
/// [`decimal_to_q16`] maps back to `q`. Whole values print without a point
/// (`8`, not `8.0`), so the text is also valid JSON.
#[must_use]
pub fn q16_to_decimal(q: u32) -> String {
    let q_wide = u128::from(q);
    let one = u128::from(Q16_ONE);
    for digits in 0..=6usize {
        let scale = pow10(digits);
        let scaled = (q_wide * scale + one / 2) / one;
        let text = format_scaled(scaled, digits);
        if decimal_to_q16(&text) == Some(q) {
            return text;
        }
    }
    // Six digits always round-trip (1e-6 < 1/65536); this line is unreachable
    // by arithmetic, kept total rather than panicking.
    format_scaled((q_wide * pow10(6) + one / 2) / one, 6)
}

fn is_digits(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_digit())
}

fn pow10(digits: usize) -> u128 {
    let mut value: u128 = 1;
    for _ in 0..digits {
        value *= 10;
    }
    value
}

/// `scaled / 10^digits` as decimal text with exactly `digits` fractional
/// digits (none when `digits == 0`).
fn format_scaled(scaled: u128, digits: usize) -> String {
    if digits == 0 {
        return scaled.to_string();
    }
    let scale = pow10(digits);
    let int_part = scaled / scale;
    let frac_part = scaled % scale;
    format!("{int_part}.{frac_part:0width$}", width = digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_bx12_decimals_land_on_the_table_values() {
        let cases: [(&str, u32); 12] = [
            ("8", 524_288),
            ("8.0", 524_288),
            ("0.85", 55_706),
            ("0.8", 52_429),
            ("0.25", 16_384),
            ("0.1", 6_554),
            ("0.7", 45_875),
            ("0.9", 58_982),
            ("0.05", 3_277),
            ("0.08", 5_243),
            ("0.95", 62_259),
            ("32", 2_097_152),
        ];
        for (text, q) in cases {
            assert_eq!(decimal_to_q16(text), Some(q), "{text}");
        }
    }

    #[test]
    fn q16_prints_the_shortest_decimal_that_round_trips() {
        assert_eq!(q16_to_decimal(524_288), "8");
        assert_eq!(q16_to_decimal(55_706), "0.85");
        assert_eq!(q16_to_decimal(52_429), "0.8");
        assert_eq!(q16_to_decimal(3_277), "0.05");
        assert_eq!(q16_to_decimal(0), "0");
        assert_eq!(q16_to_decimal(1), "0.00002");
    }

    #[test]
    fn every_q16_in_a_sweep_round_trips_through_text() {
        for q in (0..200_000u32).step_by(7) {
            let text = q16_to_decimal(q);
            assert_eq!(decimal_to_q16(&text), Some(q), "{q} -> {text}");
        }
        assert_eq!(decimal_to_q16(&q16_to_decimal(u32::MAX)), Some(u32::MAX));
    }

    #[test]
    fn refuses_signs_exponents_blanks_and_overflow() {
        for bad in [
            "-1", "1e3", "", ".5", "1.", "abc", "1.2.3", "65536", "70000.5",
        ] {
            assert_eq!(decimal_to_q16(bad), None, "{bad}");
        }
        assert_eq!(decimal_to_q16("0.1234567890"), None);
        assert_eq!(decimal_to_q16("65535.99999"), Some(4_294_967_295));
    }
}
