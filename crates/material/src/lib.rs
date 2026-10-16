//! engine-material: one material record an AI writes as text, shared by the
//! look, physics, sound and damage layers.
//!
//! Callers and edges (from `crates/material/Cargo.toml:17-22`): above
//! `engine-core` digests and `engine-destruction` (the sim half is its
//! `MaterialRow`, composed never mirrored, `crates/material/src/record.rs:22-24`).
//! Readers are the door `material` pack
//! (`crates/endpoint/src/packs/material.rs:38-39`, seven verbs). In
//! `default-members` (root `Cargo.toml`).
//!
//! Invariants: no GPU, file or clock in the default build; sim numbers go
//! decimal text to Q16 integer math (`fixed`), look numbers are `f32`
//! round-trip text (`crates/material/src/record.rs:13-15`); `validate` is off
//! by default and pulls `naga` only on request
//! (`crates/material/Cargo.toml:11-15`). Budgets: 8 layers, 64 name bytes,
//! 128 texture-ref bytes (`record.rs:33-37`), 255 records per set
//! (`set.rs:28`), 8 KB schema (`schema.rs:98`), 64 KB JSON at depth 32
//! (`json.rs:15-17`). Entry points: [`MaterialRecord`],
//! [`MaterialSet::set_param`] and its inverse, [`MaterialSet::shipped`],
//! sim/look digests, [`SCHEMA`], [`record`], [`set`], [`json`], [`fixed`].
//!
//! ```rust
//! use engine_material::{MaterialRecord, MaterialSet};
//!
//! let kit = MaterialSet::siege_kit();
//! let marble = kit.get("marble").expect("shipped");
//! let text = marble.to_text();
//! let again = MaterialRecord::parse(&text).expect("canonical text parses");
//! assert_eq!(again.to_text(), text);
//! assert_eq!(again.sim_digest(), marble.sim_digest());
//! ```

pub mod channels;
pub mod compare;
pub mod compile;
pub mod dfg;
pub mod fixed;
#[cfg(test)]
mod furnace;
pub mod gltf_import;
pub mod json;
pub mod record;
pub mod reload;
pub mod schema;
pub mod set;

pub use record::{
    Anisotropy, Channel, Coat, Domain, Emission, Layer, LayerKind, Look, MaterialRecord,
    RecordError, RecordErrorKind, Sheen, Subsurface, Textures, Transmission, RECORD_VERSION,
};
pub use schema::SCHEMA;
pub use set::{MaterialSet, ParamEdit};

/// Root wiring vectors: the material crate root as its callers see it.
///
/// Consumer: the door `material` pack plus the look, physics, sound and
/// damage readers reach every number through the crate root
/// (`crate::MaterialRecord`, `::MaterialSet`, `::RECORD_VERSION`,
/// `::SCHEMA`). Data owner stays each module; this module owns only the
/// root re-export contract. Units: Q16 sim text, f32 look text, wire ids,
/// digests. Determinism: canonical text plus integer digests only, no
/// clock, no IO, no GPU. Errors are typed `RecordError`, asserted through
/// the root path.
///
/// Why this module exists: `crates/material/src/lib.rs` changed 5 times in
/// 30 days yet held zero `#[test]` and no `tests/it` module named it, so a
/// dropped `pub mod` or `pub use` would compile the crate but break every
/// caller at their `use` line. Each vector below names the root path, so
/// removing or gating a re-export fails here first.
#[cfg(test)]
mod root_wiring_tests {
    use crate::{MaterialRecord, MaterialSet, RecordErrorKind, RECORD_VERSION, SCHEMA};

    /// Every re-exported headline number stays reachable with its frozen
    /// value intact. Removing one `pub mod` or `pub use` line fails this
    /// file at its `use` or at the path below.
    #[test]
    fn root_pins_headline_consts_and_modules() {
        assert_eq!(RECORD_VERSION, 1, "record version via root");
        assert_eq!(crate::set::MAX_RECORDS, 255, "wire budget via root");
        assert_eq!(crate::record::MAX_LAYERS, 8, "layer budget via root");
        assert_eq!(crate::record::MAX_NAME_BYTES, 64, "name budget via root");
        assert_eq!(
            crate::record::MAX_TEXTURE_REF_BYTES,
            128,
            "texture-ref budget via root"
        );
        assert!(
            SCHEMA.contains("material"),
            "schema text mentions material via root"
        );
        // Module paths the door pack reads through the root.
        let _ = crate::record::RECORD_VERSION;
        let _ = crate::set::MAX_RECORDS;
        let _ = crate::schema::SCHEMA;
        let _ =
            crate::json::parse as fn(&str) -> Result<crate::json::Value, crate::json::JsonError>;
    }

    /// The shipped siege kit round-trips through the root: fourteen
    /// records, marble parses plus emits byte-identical twice, digests are
    /// stable, and a sim edit moves only the sim digest.
    #[test]
    fn siege_kit_roundtrip_and_digests_via_root() {
        let kit = MaterialSet::siege_kit();
        assert_eq!(kit.len(), 14, "fourteen siege records via root");
        let marble = kit.get("marble").expect("shipped marble via root");
        assert_eq!(marble.version, RECORD_VERSION);
        let text = marble.to_text();
        assert!(text.ends_with('\n'), "canonical text ends in newline");
        let again = MaterialRecord::parse(&text).expect("canonical text parses via root");
        assert_eq!(&again, marble, "parse inverts emit via root");
        assert_eq!(again.to_text(), text, "emit is byte-stable twice");
        assert_eq!(
            again.sim_digest(),
            marble.sim_digest(),
            "sim digest stable via root"
        );
        assert_eq!(
            again.look_digest(),
            marble.look_digest(),
            "look digest stable via root"
        );
        let mut tampered = marble.clone();
        tampered.body.toughness_q16 = tampered.body.toughness_q16.wrapping_add(1);
        assert_ne!(
            tampered.sim_digest(),
            marble.sim_digest(),
            "sim edit moves sim digest"
        );
        assert_eq!(
            tampered.look_digest(),
            marble.look_digest(),
            "sim edit keeps look digest"
        );
    }

    /// The one door edit primitive guards identity fields through the root
    /// and undoes a look edit with its inverse.
    #[test]
    fn set_param_guards_identity_and_undoes_via_root() {
        let mut kit = MaterialSet::siege_kit();
        for path in ["material", "extends", "version"] {
            let err = kit
                .set_param("marble", path, "0.1")
                .expect_err("identity field refuses via root");
            assert_eq!(err.path, path, "refusal names the path");
            assert!(
                matches!(err.kind, RecordErrorKind::Expected(_)),
                "identity refusal is Expected"
            );
        }
        let err = kit
            .set_param("no-such-stone", "look.specular_roughness", "0.1")
            .expect_err("unknown material refuses via root");
        assert_eq!(err.path, "material");
        assert!(
            matches!(err.kind, RecordErrorKind::UnknownName { .. }),
            "unknown name names the material"
        );
        let before_text = kit.get("marble").expect("marble present").to_text();
        let edit = kit
            .set_param("marble", "look.specular_roughness", "0.1")
            .expect("look edit admits via root");
        assert_eq!(edit.material, "marble");
        assert_eq!(edit.path, "look.specular_roughness");
        assert_ne!(edit.before, edit.after, "edit records a change");
        let undone = kit
            .set_param("marble", "look.specular_roughness", &edit.before)
            .expect("inverse restores via root");
        assert_eq!(undone.after, edit.before, "undo writes the old value");
        assert_eq!(
            kit.get("marble").expect("marble kept").to_text(),
            before_text,
            "undo restores canonical bytes"
        );
    }

    /// Wire ids plus push guards read through the root: known names map
    /// inside the siege prefix, unknown names miss, duplicates refuse.
    #[test]
    fn wire_ids_and_push_guards_via_root() {
        let kit = MaterialSet::siege_kit();
        let marble_id = kit.wire_id("marble").expect("marble has a wire id");
        assert!(
            (marble_id as usize) < kit.len(),
            "marble id sits in the siege prefix"
        );
        assert_eq!(kit.wire_id("no-such-stone"), None, "unknown misses");
        let shipped = MaterialSet::shipped();
        assert!(shipped.len() >= kit.len(), "shipped keeps the siege prefix");
        assert_eq!(
            shipped.wire_id("marble"),
            Some(marble_id),
            "siege wire ids never move"
        );
        let mut owned = MaterialSet::siege_kit();
        let dupe = owned.get("marble").expect("marble present").clone();
        let err = owned.push(dupe).expect_err("duplicate refuses via root");
        assert!(
            matches!(err.kind, RecordErrorKind::Duplicate(_)),
            "duplicate names the record"
        );
    }
}
