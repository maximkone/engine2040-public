//! content: the asset-name guard of the sandbox boundary (CT16).
//!
//! Consumer: [`super::check_before_run`], which runs [`check_payload`] after
//! the manifest cap and the hostile-text scan, so a payload that names a path
//! outside the content root refuses before the tool stages anything. Shapes
//! only: fails closed, never normalizes a name into a legal one. Units: name
//! bytes. Determinism scope: one pass over the text, no clock, no float, no
//! model, so the same name always seals the same refusal. Errors:
//! [`ScanRefusal`] with `code` `asset_name`, the rule that fired as `marker`,
//! and one `re-run` recovery.

use super::scan::ScanRefusal;

/// Longest asset name the guard admits, bytes: a content path, not a URI.
pub const MAX_NAME_BYTES: usize = 255;

/// Windows device stems: a name that resolves to one of these opens a device
/// instead of a file, on every path, so they refuse.
const RESERVED_STEMS: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Characters that end one name in a payload: whitespace, `;`, `,`, quotes.
const SEPARATORS: &[char] = &[' ', '\t', '\n', '\r', ';', ',', '"', '\'', '(', ')'];

/// One bounded, printable echo of a name for the refusal message: control
/// bytes never reach a log, and a long name is cut at 64 chars.
fn echo(name: &str) -> String {
    name.chars()
        .take(64)
        .map(|c| if c.is_ascii_graphic() { c } else { '?' })
        .collect()
}

/// Refusal for an asset name that leaves the content root or names a device.
fn refusal(rule: &'static str, name: &str) -> ScanRefusal {
    ScanRefusal {
        code: "asset_name",
        marker: rule.to_owned(),
        message: format!(
            "asset name `{}` refused ({rule}); names stay inside the content root, \
             relative, with one `/` separator; re-run with a content-relative name",
            echo(name)
        ),
    }
}

/// Check one asset name against the content boundary. Returns the first rule
/// it breaks in table order, so the same name always seals the same refusal.
/// Pure: reads the name only. Empty, absolute, drive-prefixed, backslash,
/// device-stem, `.`/`..` segment and control-byte names refuse.
#[must_use]
pub fn refuse_asset_name(name: &str) -> Option<ScanRefusal> {
    if name.trim().is_empty() {
        return Some(refusal("empty", name));
    }
    if name.len() > MAX_NAME_BYTES {
        return Some(refusal("over_255", name));
    }
    if name.chars().any(|c| c.is_control()) {
        return Some(refusal("control_byte", name));
    }
    let first = name.chars().next();
    if matches!(first, Some('/') | Some('\\')) {
        return Some(refusal("absolute", name));
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(refusal("drive", name));
    }
    if name.contains('\\') {
        return Some(refusal("backslash", name));
    }
    let stem = name.split('.').next().map_or("", str::trim).to_lowercase();
    if RESERVED_STEMS.contains(&stem.as_str()) {
        return Some(refusal("reserved_name", name));
    }
    if name.split('/').any(|segment| segment == "..") {
        return Some(refusal("parent_segment", name));
    }
    None
}

/// Check every name-shaped token in one payload: a token holding `/`, `\` or
/// a dotted extension is read as an asset name. The first refusal wins, so
/// the payload seals one verdict. Pure: reads the text only.
#[must_use]
pub fn check_payload(payload: &str) -> Option<ScanRefusal> {
    payload
        .split(SEPARATORS)
        .filter(|token| !token.is_empty())
        .filter(|token| token.contains('/') || token.contains('\\') || token.contains('.'))
        .find_map(refuse_asset_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{check_before_run, scan};

    #[test]
    fn asset_names_with_traversal_or_absolute_paths_refuse() {
        let hostile: &[(&str, &str)] = &[
            ("/etc/shadow", "absolute"),
            ("\\server\\share\\x.glb", "absolute"),
            ("C:\\Temp\\x.glb", "drive"),
            ("c:rel.glb", "drive"),
            ("..\\..\\win.ini", "backslash"),
            ("fighters\\hero.gltf", "backslash"),
            ("../../etc/passwd", "parent_segment"),
            ("armor/../secret.glb", "parent_segment"),
            ("..", "parent_segment"),
            ("con", "reserved_name"),
            ("NUL.png", "reserved_name"),
            ("com1.glb", "reserved_name"),
            ("con .txt", "reserved_name"),
            ("", "empty"),
            ("   ", "empty"),
            ("sword\u{0}.glb", "control_byte"),
            ("sword\u{7}.glb", "control_byte"),
        ];
        for (name, rule) in hostile {
            let refusal = refuse_asset_name(name)
                .unwrap_or_else(|| panic!("hostile asset name must refuse: {name:?}"));
            assert_eq!(refusal.code, "asset_name", "code for {name:?}");
            assert_eq!(refusal.marker, *rule, "rule for {name:?}");
            assert!(
                refusal.message.contains("re-run"),
                "refusal for {name:?} must carry a recovery"
            );
        }
    }

    #[test]
    fn benign_asset_names_pass() {
        let benign = [
            "blessed-sword.glb",
            "fighters/hero.gltf",
            "armor/steel/helm_lod1.glb",
            "sand_2k.png",
            "a.b.c",
            "sword v2.glb",
            "contents.json",
        ];
        for name in benign {
            assert!(
                refuse_asset_name(name).is_none(),
                "benign asset name must pass: {name:?}"
            );
        }
    }

    #[test]
    fn check_before_run_refuses_a_traversing_asset_name() {
        // Payloads that dodge the hostile-text markers (`../` is the only
        // traversal shape that one carries) still refuse on the name guard.
        let payloads = [
            "Blessed Sword ..\\..\\win.ini",
            "Blessed Sword /etc/passwd",
            "Doom Helm C:\\Windows\\system32\\drivers\\etc\\hosts",
        ];
        for payload in payloads {
            let refusal = check_before_run("asset.refs", payload)
                .expect_err("a traversing asset name must refuse before it runs");
            assert_eq!(refusal.code, "asset_name", "code for {payload:?}");
            assert!(
                refusal.message.contains("re-run"),
                "refusal for {payload:?} must carry a recovery"
            );
        }
    }

    #[test]
    fn benign_payloads_still_pass_the_asset_guard() {
        let payloads = [
            "Tavern note: fresh bread at dawn",
            "Blessed Sword fighters/hero.gltf",
            "sand_2k.png at 2.5 meters; light 0.8",
        ];
        for payload in payloads {
            assert!(
                check_payload(payload).is_none(),
                "benign payload must pass the name guard: {payload:?}"
            );
        }
    }

    #[test]
    fn same_name_replays_the_same_refusal_and_digest() {
        let first = refuse_asset_name("../etc/passwd").expect("traversal refuses");
        let second = refuse_asset_name("../etc/passwd").expect("traversal refuses again");
        assert_eq!(first, second, "the same name seals the same refusal");
        assert_eq!(
            scan::scan_digest(false, first.code),
            scan::scan_digest(false, second.code),
            "same code, same digest"
        );
    }

    #[test]
    fn check_payload_reads_names_and_skips_plain_words() {
        assert!(check_payload("ignore the crowd and strike").is_none());
        assert!(check_payload("host=192.168.1.2 port=7043").is_none());
        let refusal = check_payload("load ../secret.glb now").expect("traversal refuses");
        assert_eq!(refusal.marker, "parent_segment");
    }
}
