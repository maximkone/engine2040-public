//! locale: static en/de menu labels with fallback plus integer plurals.
//! Consumer: `crate::menus` localized reads (`main_items_locale`,
//! `pause_items_locale`); any game can reuse these tables headlessly.
//! Data owner: `engine-ui`. Units: labels are UTF-8 `&str`, counts are `u32`.
//! Determinism: pure functions of key/locale/count, fixed order, no floats,
//! no clock, no IO. Failure: unknown keys fall back to en then to the key;
//! small buffers truncate without panic. Budget: O(1) lookup, no allocation.
//! PORT(projectfluent/fluent-rs@0f87afbbc3fe7d20c61a3fc75cb5d0e123aefdda:fluent-bundle/src/bundle.rs): key lookup plus fallback subset, static-table only.
//! PORT(projectfluent/fluent-rs@0f87afbbc3fe7d20c61a3fc75cb5d0e123aefdda:fluent-bundle/src/args.rs): integer one/other select subset.
//! PORT(projectfluent/fluent-rs@0f87afbbc3fe7d20c61a3fc75cb5d0e123aefdda:fluent-pseudo/src/lib.rs): accent-wrap plus length-pad test gate.
/// Locale id: English plus German, genre-neutral, static only.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LocaleId {
    /// English fallback.
    En,
    /// German translation.
    De,
}
/// Plural category: CLDR-shaped one/other for en/de.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PluralForm {
    /// Exactly one.
    One,
    /// Zero, two or more.
    Other,
}
/// Menu key for the start-fight label.
pub const KEY_START_FIGHT: &str = "start-fight";
/// Menu key for the settings label.
pub const KEY_SETTINGS: &str = "settings";
/// Menu key for the quit label.
pub const KEY_QUIT: &str = "quit";
/// Menu key for the resume label.
pub const KEY_RESUME: &str = "resume";
/// Menu key for the quit-to-main label.
pub const KEY_QUIT_TO_MAIN: &str = "quit-to-main";
/// Menu key for the back label.
pub const KEY_BACK: &str = "back";
/// Static lookup with en fallback; unknown keys return the key. Pure, no allocation.
#[must_use]
pub fn lookup(key: &str, locale: LocaleId) -> &str {
    let en = match key {
        "start-fight" => "Start Fight",
        "settings" => "Settings",
        "quit" => "Quit",
        "resume" => "Resume",
        "quit-to-main" => "Quit to Main",
        "back" => "Back",
        _ => return key,
    };
    match locale {
        LocaleId::En => en,
        LocaleId::De => match key {
            "start-fight" => "Kampf starten",
            "settings" => "Einstellungen",
            "quit" => "Beenden",
            "resume" => "Fortsetzen",
            "quit-to-main" => "Zum Hauptmenue",
            "back" => "Zurueck",
            _ => en,
        },
    }
}
/// Integer plural select: one iff n==1, other otherwise for en/de. Pure.
#[must_use]
pub const fn plural_category(count: u32, locale: LocaleId) -> PluralForm {
    match locale {
        LocaleId::En | LocaleId::De => {
            if count == 1 {
                PluralForm::One
            } else {
                PluralForm::Other
            }
        }
    }
}
/// Fighter-count label into caller buffer (`1 fighter`, `2 fighters` en). Pure, no allocation.
pub fn format_fighter_count(count: u32, locale: LocaleId, out: &mut [u8]) -> &str {
    let noun = match (locale, plural_category(count, locale)) {
        (LocaleId::En, PluralForm::One) => "fighter",
        (LocaleId::En, PluralForm::Other) => "fighters",
        (LocaleId::De, _) => "Kaempfer",
    };
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    let mut rest = count;
    loop {
        let digit = rest % 10;
        let byte = u8::try_from(digit + 48).unwrap_or(b'?');
        if len < tmp.len() {
            tmp[len] = byte;
            len += 1;
        }
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    let mut pos = 0usize;
    let mut i = len;
    while i > 0 {
        i -= 1;
        if pos < out.len() {
            if let Some(slot) = out.get_mut(pos) {
                *slot = tmp[i];
            }
            pos += 1;
        } else {
            break;
        }
    }
    if pos < out.len() {
        if let Some(slot) = out.get_mut(pos) {
            *slot = b' ';
        }
        pos += 1;
    }
    for b in noun.as_bytes() {
        if pos < out.len() {
            if let Some(slot) = out.get_mut(pos) {
                *slot = *b;
            }
            pos += 1;
        } else {
            break;
        }
    }
    core::str::from_utf8(out.get(..pos).unwrap_or(&[])).unwrap_or_default()
}
/// Pseudo-locale gate, test-only: accent vowels, wrap, pad to 130 percent. Pure, no allocation.
pub fn pseudo_into<'o>(source: &str, out: &'o mut [u8]) -> &'o str {
    let mut pos = 0usize;
    for b in b"[!! ".iter() {
        if pos < out.len() {
            if let Some(slot) = out.get_mut(pos) {
                *slot = *b;
            }
            pos += 1;
        }
    }
    for b in source.as_bytes().iter() {
        let pair = match *b {
            b'a' => Some([0xC3, 0xA0]),
            b'e' => Some([0xC3, 0xA9]),
            b'i' => Some([0xC3, 0xAE]),
            b'o' => Some([0xC3, 0xB2]),
            b'u' => Some([0xC3, 0xBC]),
            b'A' => Some([0xC3, 0x80]),
            b'E' => Some([0xC3, 0x89]),
            b'I' => Some([0xC3, 0x8E]),
            b'O' => Some([0xC3, 0x92]),
            b'U' => Some([0xC3, 0x9C]),
            _ => None,
        };
        match pair {
            Some(bytes) => {
                if pos.saturating_add(2) <= out.len() {
                    if let Some(slot) = out.get_mut(pos) {
                        *slot = bytes[0];
                    }
                    if let Some(slot) = out.get_mut(pos + 1) {
                        *slot = bytes[1];
                    }
                    pos += 2;
                } else {
                    break;
                }
            }
            None => {
                if pos < out.len() {
                    if let Some(slot) = out.get_mut(pos) {
                        *slot = *b;
                    }
                    pos += 1;
                } else {
                    break;
                }
            }
        }
    }
    let target = source.len().saturating_mul(13) / 10;
    while pos < target {
        if pos.saturating_add(4) >= out.len() {
            break;
        }
        if let Some(slot) = out.get_mut(pos) {
            *slot = b'~';
        }
        pos += 1;
    }
    for b in b" !!]".iter() {
        if pos < out.len() {
            if let Some(slot) = out.get_mut(pos) {
                *slot = *b;
            }
            pos += 1;
        }
    }
    core::str::from_utf8(out.get(..pos).unwrap_or(&[])).unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;
    fn digest_labels(locale: LocaleId) -> u64 {
        let mut h = Fnv64::new();
        for key in [KEY_START_FIGHT, KEY_SETTINGS, KEY_QUIT] {
            h.write(lookup(key, locale).as_bytes());
        }
        h.finish()
    }
    fn depseudo(pseudo: &str) -> String {
        let core = pseudo.strip_prefix("[!! ").unwrap_or(pseudo);
        let core = core.strip_suffix(" !!]").unwrap_or(core);
        let core = core.trim_end_matches('~');
        core.replace("à", "a")
            .replace("é", "e")
            .replace("î", "i")
            .replace("ò", "o")
            .replace("ü", "u")
            .replace("À", "A")
            .replace("É", "E")
            .replace("Î", "I")
            .replace("Ò", "O")
            .replace("Ü", "U")
    }
    #[test]
    fn locale_two_languages_plurals_pseudo() {
        for key in [KEY_START_FIGHT, KEY_SETTINGS, KEY_QUIT] {
            let en = lookup(key, LocaleId::En);
            let de = lookup(key, LocaleId::De);
            assert_ne!(en, de, "key {key} must switch");
            assert_eq!(lookup(key, LocaleId::En), en, "switch back");
        }
        assert_eq!(lookup(KEY_START_FIGHT, LocaleId::En), "Start Fight");
        assert_eq!(lookup(KEY_START_FIGHT, LocaleId::De), "Kampf starten");
        assert_eq!(digest_labels(LocaleId::En), digest_labels(LocaleId::En));
        assert_eq!(digest_labels(LocaleId::De), digest_labels(LocaleId::De));
        for locale in [LocaleId::En, LocaleId::De] {
            assert_eq!(plural_category(1, locale), PluralForm::One);
            assert_eq!(plural_category(2, locale), PluralForm::Other);
            assert_eq!(plural_category(5, locale), PluralForm::Other);
        }
        let mut buf = [0u8; 32];
        assert_eq!(format_fighter_count(1, LocaleId::En, &mut buf), "1 fighter");
        assert_eq!(
            format_fighter_count(2, LocaleId::En, &mut buf),
            "2 fighters"
        );
        assert_eq!(
            format_fighter_count(5, LocaleId::De, &mut buf),
            "5 Kaempfer"
        );
        for key in [KEY_START_FIGHT, KEY_SETTINGS, KEY_QUIT] {
            let en = lookup(key, LocaleId::En);
            let mut pbuf = [0u8; 128];
            let pseudo = pseudo_into(en, &mut pbuf);
            assert!(pseudo.len() * 100 >= en.len() * 130, "pseudo expand");
            assert!(pseudo.starts_with("[!! ") && pseudo.ends_with(" !!]"));
            assert_eq!(depseudo(pseudo), en, "round-trip through lookup");
            assert_eq!(lookup(key, LocaleId::En), en);
        }
        assert_eq!(
            lookup("no-such-key", LocaleId::De),
            lookup("no-such-key", LocaleId::En)
        );
        assert_eq!(lookup("no-such-key", LocaleId::En), "no-such-key");
        let start = std::time::Instant::now();
        std::hint::black_box(lookup(KEY_START_FIGHT, LocaleId::En));
        std::hint::black_box(lookup(KEY_SETTINGS, LocaleId::En));
        std::hint::black_box(lookup(KEY_QUIT, LocaleId::En));
        std::hint::black_box(lookup(KEY_RESUME, LocaleId::En));
        assert!(start.elapsed() < std::time::Duration::from_micros(5));
    }
}
