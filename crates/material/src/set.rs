//! set: an ordered, name-unique collection of records with one sim digest.
//!
//! Owner: `engine-material`. A [`MaterialSet`] is the table v2: records in
//! wire order (the position is the `u8` material id bond arrays carry),
//! names unique and append-only, `extends` resolved against earlier
//! records. [`MaterialSet::sim_digest`] folds the rows exactly as
//! `MaterialTable::digest` does, so the shipped fourteen
//! ([`MaterialSet::siege_kit`]) give the shipped table digest and the golden never
//! moves. [`MaterialSet::set_param`] is the one edit primitive behind the
//! door verb: it changes one JSON path, re-validates the whole record and
//! returns the inverse edit, so undo is `set_param` with `before`. Errors:
//! [`RecordError`]; nothing here panics on user text.

use engine_destruction::material::{
    digest_rows, DecalKind, DustKind, MaterialId, MaterialRow, MaterialTable, ShardRule, SoundKind,
    SparkKind, MATERIAL_VERSION_V1,
};

use crate::fixed::decimal_to_q16;
use crate::json::{self, Value};
use crate::record::{
    Channel, Coat, Layer, LayerKind, Look, MaterialRecord, RecordError, RecordErrorKind, Sheen,
    Subsurface, Transmission, RECORD_VERSION,
};

/// Most records one set holds: wire ids are one byte and `255` stays free
/// for "no material".
pub const MAX_RECORDS: usize = 255;

/// One applied parameter edit and its inverse. `before` and `after` are
/// JSON texts of the value at `path`; undo is
/// `set.set_param(&material, &path, &before)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParamEdit {
    /// Record that changed.
    pub material: String,
    /// Dotted path that changed (`look.specular_roughness`, `layers[0].darken`).
    pub path: String,
    /// Canonical JSON of the value before; `null` when the key was absent.
    pub before: String,
    /// Canonical JSON of the value after.
    pub after: String,
}

/// Ordered, name-unique records. See the module docs.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MaterialSet {
    records: Vec<MaterialRecord>,
}

impl MaterialSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records in wire order.
    #[must_use]
    pub fn records(&self) -> &[MaterialRecord] {
        &self.records
    }

    /// Record count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when the set holds no record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The record named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&MaterialRecord> {
        self.records.iter().find(|r| r.name == name)
    }

    /// Wire id of `name`: its position, the byte bond arrays carry.
    #[must_use]
    pub fn wire_id(&self, name: &str) -> Option<u8> {
        self.records
            .iter()
            .position(|r| r.name == name)
            .and_then(|i| u8::try_from(i).ok())
    }

    /// Append a record. A name already present is
    /// [`RecordErrorKind::Duplicate`]; past [`MAX_RECORDS`] is
    /// [`RecordErrorKind::TooMany`].
    pub fn push(&mut self, record: MaterialRecord) -> Result<(), RecordError> {
        if self.records.len() >= MAX_RECORDS {
            return Err(RecordError {
                path: "$".to_string(),
                kind: RecordErrorKind::TooMany { max: MAX_RECORDS },
            });
        }
        if self.get(&record.name).is_some() {
            return Err(RecordError {
                path: "material".to_string(),
                kind: RecordErrorKind::Duplicate(record.name),
            });
        }
        self.records.push(record);
        Ok(())
    }

    /// Parse `text` against this set (its `extends` parent must already be
    /// here) and append it. Returns the record's wire id.
    pub fn parse_push(&mut self, text: &str) -> Result<u8, RecordError> {
        let parent_name = parent_name(text)?;
        let parent = match &parent_name {
            Some(name) => Some(self.get(name).ok_or_else(|| RecordError {
                path: "extends".to_string(),
                kind: RecordErrorKind::UnknownParent(name.clone()),
            })?),
            None => None,
        };
        let record = MaterialRecord::parse_with(text, parent)?;
        self.push(record)?;
        u8::try_from(self.records.len() - 1).map_err(|_| RecordError {
            path: "$".to_string(),
            kind: RecordErrorKind::TooMany { max: MAX_RECORDS },
        })
    }

    /// Replace the record of the same name, or append. Returns the record
    /// replaced, so a journal can undo with `put(previous)`.
    pub fn put(&mut self, record: MaterialRecord) -> Result<Option<MaterialRecord>, RecordError> {
        match self.records.iter().position(|r| r.name == record.name) {
            Some(i) => Ok(Some(std::mem::replace(&mut self.records[i], record))),
            None => self.push(record).map(|()| None),
        }
    }

    /// Change one value of one record: `value_text` is JSON (`0.08`,
    /// `[0.3, 0.1, 0.1]`, `{"weight": 1, "roughness": 0.05, "ior": 1.33}`,
    /// `null` to clear a lobe). The whole record re-validates before the set
    /// changes; a refusal leaves the set untouched. `material` and
    /// `extends` are not parameters: use [`MaterialSet::put`].
    pub fn set_param(
        &mut self,
        material: &str,
        path: &str,
        value_text: &str,
    ) -> Result<ParamEdit, RecordError> {
        if matches!(path, "material" | "extends" | "version") {
            return Err(RecordError {
                path: path.to_string(),
                kind: RecordErrorKind::Expected("a parameter (use put for identity)"),
            });
        }
        let Some(index) = self.records.iter().position(|r| r.name == material) else {
            return Err(RecordError {
                path: "material".to_string(),
                kind: RecordErrorKind::UnknownName {
                    what: "material",
                    got: material.to_string(),
                },
            });
        };
        let new_value = json::parse(value_text).map_err(|e| RecordError {
            path: path.to_string(),
            kind: RecordErrorKind::Json(e),
        })?;
        let mut tree = self.records[index].to_json();
        let slot = navigate(&mut tree, path)?;
        let before = json::write(slot);
        let after = json::write(&new_value);
        *slot = new_value;
        let text = json::write(&tree);
        let parent = self.records[index]
            .extends
            .as_ref()
            .and_then(|name| self.get(name))
            .cloned();
        let updated = MaterialRecord::parse_with(&text, parent.as_ref())?;
        self.records[index] = updated;
        Ok(ParamEdit {
            material: material.to_string(),
            path: path.to_string(),
            before,
            after,
        })
    }

    /// The rows in wire order.
    #[must_use]
    pub fn rows(&self) -> Vec<MaterialRow> {
        self.records.iter().map(|r| r.body).collect()
    }

    /// Digest of the sim half of every record, in wire order, under the
    /// table version: the stream `MaterialTable::digest` folds.
    #[must_use]
    pub fn sim_digest(&self) -> u64 {
        digest_rows(MATERIAL_VERSION_V1, &self.rows())
    }

    /// Digest of every record's look, in wire order, with names.
    #[must_use]
    pub fn look_digest(&self) -> u64 {
        let mut hasher = engine_core::Fnv64::new();
        for record in &self.records {
            hasher.write_u64(record.look_digest());
            hasher.write_u8(0);
            hasher.write_str(&record.name);
        }
        hasher.finish()
    }

    /// The fourteen shipped table materials as records, in `MaterialId` order, with
    /// an authored look each. `sim_digest` equals
    /// `MaterialTable::siege_kit().digest()`.
    #[must_use]
    pub fn siege_kit() -> Self {
        let table = MaterialTable::siege_kit();
        let mut set = Self::new();
        for id in MaterialId::all() {
            let row = table.row_of(id);
            let (look, layers) = siege_look(id);
            // Fourteen unique names and no `extends`: push cannot refuse.
            let _ = set.push(record(id.name(), None, look, layers, row));
        }
        set
    }

    /// Everything the engine ships: the fourteen siege records in wire
    /// order, then each reference record whose name is not taken (sand,
    /// steel, skin, wood, `marble_wet`). The wire ids of the first fourteen
    /// never move, so the sim digest of that prefix is the table digest.
    #[must_use]
    pub fn shipped() -> Self {
        let mut set = Self::siege_kit();
        for record in Self::reference().records() {
            if set.get(&record.name).is_none() {
                // A name just proven free and fewer than 255 records: push
                // cannot refuse.
                let _ = set.push(record.clone());
            }
        }
        set
    }

    /// The six reference materials (sand, steel, skin, marble, wood, cloth)
    /// plus `marble_wet`, which extends `marble`: the fixtures the compare
    /// and import rows judge against.
    #[must_use]
    pub fn reference() -> Self {
        let table = MaterialTable::siege_kit();
        let mut set = Self::new();
        for seed in [
            sand(),
            steel(),
            skin(),
            from_table(&table, "marble", MaterialId::Marble),
            from_table(&table, "wood", MaterialId::Timber),
            from_table(&table, "cloth", MaterialId::Cloth),
        ] {
            // Six unique names: push cannot refuse.
            let _ = set.push(record(seed.name, None, seed.look, seed.layers, seed.row));
        }
        // Marble was pushed above, so the lookup holds.
        if let Some(mut wet) = set.get("marble").cloned() {
            wet.name = "marble_wet".to_string();
            wet.extends = Some("marble".to_string());
            wet.look.base_color = [0.62, 0.6, 0.57];
            wet.look.specular_roughness = 0.08;
            wet.look.coat = Some(Coat {
                weight: 1.0,
                roughness: 0.05,
                ior: 1.33,
            });
            wet.body.toughness_q16 = q16("6.5");
            wet.body.sound = SoundKind::Soft;
            let _ = set.push(wet);
        }
        set
    }
}

/// One authored record before it becomes a [`MaterialRecord`].
struct Seed {
    name: &'static str,
    look: Look,
    layers: Vec<Layer>,
    row: MaterialRow,
}

/// Exact decimal to Q16 for authored constants; a typo gives `0`, which the
/// round-trip test refuses as out of range, never a silent default.
fn q16(text: &str) -> u32 {
    decimal_to_q16(text).unwrap_or(0)
}

/// `extends` of a record text without a full parse, so a set can find the
/// parent first.
fn parent_name(text: &str) -> Result<Option<String>, RecordError> {
    let doc = json::parse(text).map_err(|e| RecordError {
        path: "$".to_string(),
        kind: RecordErrorKind::Json(e),
    })?;
    match doc.get("extends") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(name)) => Ok(Some(name.clone())),
        Some(_) => Err(RecordError {
            path: "extends".to_string(),
            kind: RecordErrorKind::Expected("string"),
        }),
    }
}

/// Walk a dotted path (`look.coat.ior`, `layers[0].darken`) to the value it
/// names, inserting a missing final key into an object so a lobe can be
/// added. Refuses with the failing prefix as the path.
fn navigate<'a>(root: &'a mut Value, path: &str) -> Result<&'a mut Value, RecordError> {
    let mut current = root;
    let mut walked = String::new();
    let segments: Vec<&str> = path.split('.').collect();
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.iter().enumerate() {
        let (key, indices) = split_indices(segment, path)?;
        if !walked.is_empty() {
            walked.push('.');
        }
        walked.push_str(key);
        current = member_mut(current, key, &walked, i == last && indices.is_empty())?;
        for index in indices {
            walked.push_str(&format!("[{index}]"));
            let Value::Array(items) = current else {
                return Err(RecordError {
                    path: walked,
                    kind: RecordErrorKind::Expected("array"),
                });
            };
            current = items.get_mut(index).ok_or_else(|| RecordError {
                path: walked.clone(),
                kind: RecordErrorKind::Missing,
            })?;
        }
    }
    Ok(current)
}

/// `layers[0][1]` -> `("layers", [0, 1])`.
fn split_indices<'a>(segment: &'a str, path: &str) -> Result<(&'a str, Vec<usize>), RecordError> {
    let bad = || RecordError {
        path: path.to_string(),
        kind: RecordErrorKind::Expected("path like look.coat.ior or layers[0].darken"),
    };
    let Some(open) = segment.find('[') else {
        if segment.is_empty() {
            return Err(bad());
        }
        return Ok((segment, Vec::new()));
    };
    let key = &segment[..open];
    if key.is_empty() {
        return Err(bad());
    }
    let mut indices = Vec::new();
    for part in segment[open..].split('[').skip(1) {
        let digits = part.strip_suffix(']').ok_or_else(bad)?;
        indices.push(digits.parse().map_err(|_| bad())?);
    }
    Ok((key, indices))
}

/// Member `key` of an object; a missing final key is inserted as `null`.
fn member_mut<'a>(
    value: &'a mut Value,
    key: &str,
    walked: &str,
    insert: bool,
) -> Result<&'a mut Value, RecordError> {
    let Value::Object(members) = value else {
        return Err(RecordError {
            path: walked.to_string(),
            kind: RecordErrorKind::Expected("object"),
        });
    };
    let position = members.iter().position(|(k, _)| k == key);
    let index = match position {
        Some(i) => i,
        None if insert => {
            members.push((key.to_string(), Value::Null));
            members.len() - 1
        }
        None => {
            return Err(RecordError {
                path: walked.to_string(),
                kind: RecordErrorKind::Missing,
            })
        }
    };
    Ok(&mut members[index].1)
}

fn record(
    name: &str,
    extends: Option<&str>,
    look: Look,
    layers: Vec<Layer>,
    body: MaterialRow,
) -> MaterialRecord {
    MaterialRecord {
        name: name.to_string(),
        version: RECORD_VERSION,
        extends: extends.map(str::to_string),
        domain: crate::record::Domain::Surface,
        look,
        layers,
        body,
    }
}

fn base(color: [f32; 3], roughness: f32, metalness: f32) -> Look {
    Look {
        base_color: color,
        base_metalness: metalness,
        specular_roughness: roughness,
        ..Look::default()
    }
}

fn sheen(weight: f32, color: [f32; 3], roughness: f32) -> Option<Sheen> {
    Some(Sheen {
        weight,
        color,
        roughness,
    })
}

/// The default break layers every solid gets: cracks from strain, scorch
/// from heat.
fn break_layers(decal: DecalKind) -> Vec<Layer> {
    let mut damage = Layer::new(LayerKind::Damage, Channel::Strain);
    damage.decal = Some(decal);
    let mut burn = Layer::new(LayerKind::Burn, Channel::Heat);
    burn.decal = Some(DecalKind::Scorch);
    vec![damage, burn]
}

/// Wet and blood layers for stone-like surfaces.
fn wet_blood_layers(decal: DecalKind) -> Vec<Layer> {
    let mut wet = Layer::new(LayerKind::Wet, Channel::Wetness);
    wet.darken = Some(0.35);
    wet.roughness_to = Some(0.08);
    let mut blood = Layer::new(LayerKind::Blood, Channel::Blood);
    blood.tint = Some([0.35, 0.02, 0.02]);
    let mut layers = vec![wet, blood];
    layers.extend(break_layers(decal));
    layers
}

/// Authored look per shipped material; the row keeps the sim numbers.
fn siege_look(id: MaterialId) -> (Look, Vec<Layer>) {
    match id {
        MaterialId::Stone => (
            base([0.55, 0.53, 0.5], 0.85, 0.0),
            wet_blood_layers(DecalKind::Crack),
        ),
        MaterialId::Brick => (
            base([0.62, 0.33, 0.26], 0.8, 0.0),
            wet_blood_layers(DecalKind::Crack),
        ),
        MaterialId::Timber => (
            base([0.45, 0.3, 0.17], 0.7, 0.0),
            break_layers(DecalKind::Chip),
        ),
        MaterialId::Thatch => {
            let mut look = base([0.6, 0.5, 0.28], 0.95, 0.0);
            look.sheen = sheen(0.3, [0.6, 0.5, 0.3], 0.8);
            (look, break_layers(DecalKind::Tear))
        }
        MaterialId::Plaster => (
            base([0.86, 0.84, 0.78], 0.9, 0.0),
            wet_blood_layers(DecalKind::Crack),
        ),
        MaterialId::Concrete => (
            base([0.5, 0.5, 0.48], 0.9, 0.0),
            wet_blood_layers(DecalKind::Crack),
        ),
        MaterialId::Marble => (
            base([0.82, 0.8, 0.76], 0.35, 0.0),
            wet_blood_layers(DecalKind::Shatter),
        ),
        MaterialId::Iron => (
            base([0.56, 0.57, 0.58], 0.45, 1.0),
            break_layers(DecalKind::Dent),
        ),
        MaterialId::Bronze => (
            base([0.8, 0.55, 0.3], 0.4, 1.0),
            break_layers(DecalKind::Dent),
        ),
        MaterialId::Leather => {
            let mut look = base([0.35, 0.2, 0.12], 0.6, 0.0);
            look.sheen = sheen(0.15, [0.4, 0.3, 0.2], 0.6);
            (look, break_layers(DecalKind::Tear))
        }
        MaterialId::Cloth => {
            let mut look = base([0.6, 0.55, 0.5], 0.9, 0.0);
            look.sheen = sheen(0.6, [0.7, 0.65, 0.6], 0.7);
            (look, break_layers(DecalKind::Tear))
        }
        MaterialId::Rope => (
            base([0.55, 0.45, 0.3], 0.85, 0.0),
            break_layers(DecalKind::Tear),
        ),
        MaterialId::Glass => {
            let mut look = base([0.95, 0.97, 0.98], 0.05, 0.0);
            look.transmission = Some(Transmission {
                weight: 1.0,
                thickness: 0.01,
                attenuation_color: [1.0, 1.0, 1.0],
            });
            (look, break_layers(DecalKind::Shatter))
        }
        MaterialId::Ceramic => {
            let mut look = base([0.9, 0.88, 0.84], 0.25, 0.0);
            look.coat = Some(Coat {
                weight: 0.8,
                roughness: 0.1,
                ior: 1.5,
            });
            (look, break_layers(DecalKind::Shatter))
        }
    }
}

/// Loose sand: crumbles to powder, never burns, takes water and blood.
fn sand() -> Seed {
    Seed {
        name: "sand",
        look: base([0.76, 0.7, 0.5], 0.95, 0.0),
        layers: wet_blood_layers(DecalKind::Tear),
        row: MaterialRow {
            toughness_q16: q16("0.25"),
            brittleness_q16: q16("1"),
            shard: ShardRule::Dust,
            sound: SoundKind::Soft,
            dust: DustKind::Pale,
            spark: SparkKind::None,
            decal: DecalKind::Tear,
            burn_q16: 0,
        },
    }
}

/// Brushed steel: tougher than the iron row, dents, spark shower.
fn steel() -> Seed {
    let mut look = base([0.6, 0.6, 0.6], 0.3, 1.0);
    look.anisotropy = Some(crate::record::Anisotropy {
        strength: 0.4,
        rotation: 0.0,
    });
    Seed {
        name: "steel",
        look,
        layers: break_layers(DecalKind::Dent),
        row: MaterialRow {
            toughness_q16: q16("40"),
            brittleness_q16: q16("0.03"),
            shard: ShardRule::FewLarge,
            sound: SoundKind::Metal,
            dust: DustKind::None,
            spark: SparkKind::Shower,
            decal: DecalKind::Dent,
            burn_q16: 0,
        },
    }
}

/// Skin: wrapped subsurface, tears in sheets, takes blood.
fn skin() -> Seed {
    let mut look = base([0.8, 0.56, 0.47], 0.5, 0.0);
    look.subsurface = Some(Subsurface {
        weight: 0.8,
        radius: [0.012, 0.004, 0.002],
        color: [0.9, 0.4, 0.3],
    });
    let mut blood = Layer::new(LayerKind::Blood, Channel::Blood);
    blood.tint = Some([0.35, 0.02, 0.02]);
    Seed {
        name: "skin",
        look,
        layers: vec![blood],
        row: MaterialRow {
            toughness_q16: q16("1"),
            brittleness_q16: q16("0.02"),
            shard: ShardRule::Sheets,
            sound: SoundKind::Soft,
            dust: DustKind::None,
            spark: SparkKind::None,
            decal: DecalKind::Tear,
            burn_q16: q16("0.5"),
        },
    }
}

/// A reference material that reuses a shipped table row under its own name.
fn from_table(table: &MaterialTable, name: &'static str, id: MaterialId) -> Seed {
    let (look, layers) = siege_look(id);
    Seed {
        name,
        look,
        layers,
        row: table.row_of(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_destruction::material::MATERIAL_COUNT;

    #[test]
    fn siege_kit_sim_digest_is_the_bx12_table_digest() {
        let set = MaterialSet::siege_kit();
        assert_eq!(set.len(), MATERIAL_COUNT);
        assert_eq!(set.sim_digest(), MaterialTable::siege_kit().digest());
        assert_eq!(set.sim_digest(), MaterialSet::siege_kit().sim_digest());
        for id in MaterialId::all() {
            assert_eq!(set.wire_id(id.name()), Some(id.as_u8()));
        }
    }

    #[test]
    fn shipped_records_round_trip_byte_identical_twice() {
        for set in [MaterialSet::siege_kit(), MaterialSet::reference()] {
            let mut rebuilt = MaterialSet::new();
            for record in set.records() {
                let text = record.to_text();
                let id = rebuilt.parse_push(&text).expect("shipped record parses");
                let again = &rebuilt.records()[usize::from(id)];
                assert_eq!(again, record, "{}", record.name);
                assert_eq!(again.to_text(), text, "{}", record.name);
            }
            assert_eq!(rebuilt, set);
            assert_eq!(rebuilt.sim_digest(), set.sim_digest());
            assert_eq!(rebuilt.look_digest(), set.look_digest());
        }
    }

    #[test]
    fn reference_set_holds_the_six_plus_wet_marble() {
        let set = MaterialSet::reference();
        let names: Vec<&str> = set.records().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "sand",
                "steel",
                "skin",
                "marble",
                "wood",
                "cloth",
                "marble_wet"
            ]
        );
        let wet = set.get("marble_wet").expect("wet marble");
        let dry = set.get("marble").expect("marble");
        assert_eq!(wet.extends.as_deref(), Some("marble"));
        assert!(wet.look.specular_roughness < dry.look.specular_roughness);
        assert!(wet.look.base_color[0] < dry.look.base_color[0]);
        assert!(wet.body.toughness_q16 < dry.body.toughness_q16);
        assert_ne!(wet.body.sound, dry.body.sound);
        assert_eq!(wet.body.shard, dry.body.shard);
        assert_eq!(wet.look.coat.map(|c| c.ior), Some(1.33));
    }

    #[test]
    fn wet_marble_text_is_a_short_child_of_marble() {
        let set = MaterialSet::reference();
        let child_text = r#"{"material":"marble_wet","version":1,"extends":"marble","look":{"base_color":[0.62,0.6,0.57],"specular_roughness":0.08,"coat":{"weight":1,"roughness":0.05,"ior":1.33}},"body":{"toughness":6.5},"sound":{"impact":"soft"}}"#;
        let mut from_text = MaterialSet::new();
        for record in set.records().iter().take(6) {
            from_text.push(record.clone()).expect("six parents");
        }
        from_text.parse_push(child_text).expect("child parses");
        assert_eq!(from_text, set);
    }

    #[test]
    fn duplicate_names_and_unknown_parents_refuse() {
        let mut set = MaterialSet::siege_kit();
        let stone = set.get("stone").cloned().expect("stone");
        let err = set.push(stone).expect_err("duplicate");
        assert_eq!(err.path, "material");
        assert_eq!(err.kind, RecordErrorKind::Duplicate("stone".to_string()));
        let orphan = r#"{"material":"x","version":1,"extends":"granite"}"#;
        let err = set.parse_push(orphan).expect_err("orphan");
        assert_eq!(
            err.kind,
            RecordErrorKind::UnknownParent("granite".to_string())
        );
        assert_eq!(set.len(), MATERIAL_COUNT);
    }

    #[test]
    fn put_replaces_by_name_and_returns_the_previous_record() {
        let mut set = MaterialSet::siege_kit();
        let before = set.sim_digest();
        let mut stone = set.get("stone").cloned().expect("stone");
        stone.look.specular_roughness = 0.2;
        let previous = set.put(stone.clone()).expect("put").expect("replaced");
        assert_eq!(previous.look.specular_roughness, 0.85);
        assert_eq!(set.get("stone"), Some(&stone));
        assert_eq!(set.sim_digest(), before);
        assert_eq!(set.wire_id("stone"), Some(0));
        set.put(previous.clone()).expect("undo");
        assert_eq!(set, MaterialSet::siege_kit());
        let mut fresh = set.get("iron").cloned().expect("iron");
        fresh.name = "steel".to_string();
        assert_eq!(set.put(fresh).expect("append"), None);
        assert_eq!(set.wire_id("steel"), Some(14));
    }

    #[test]
    fn set_param_changes_one_value_and_undoes_through_before() {
        let mut set = MaterialSet::siege_kit();
        let look0 = set.look_digest();
        let sim0 = set.sim_digest();
        let edit = set
            .set_param("marble", "look.specular_roughness", "0.08")
            .expect("look edit");
        assert_eq!(edit.before, "0.35");
        assert_eq!(edit.after, "0.08");
        assert_eq!(
            set.get("marble").map(|m| m.look.specular_roughness),
            Some(0.08)
        );
        assert_ne!(set.look_digest(), look0);
        assert_eq!(set.sim_digest(), sim0);
        let body = set
            .set_param("marble", "body.toughness", "6.5")
            .expect("sim edit");
        assert_eq!(body.before, "7");
        assert_eq!(
            set.get("marble").map(|m| m.body.toughness_q16),
            Some(425_984)
        );
        assert_ne!(set.sim_digest(), sim0);
        set.set_param("marble", "body.toughness", &body.before)
            .expect("undo sim");
        set.set_param("marble", "look.specular_roughness", &edit.before)
            .expect("undo look");
        assert_eq!(set, MaterialSet::siege_kit());
    }

    #[test]
    fn set_param_reaches_layers_lobes_and_inserts_a_missing_lobe() {
        let mut set = MaterialSet::siege_kit();
        let edit = set
            .set_param("marble", "layers[0].darken", "0.5")
            .expect("layer edit");
        assert_eq!(edit.before, "0.35");
        assert_eq!(
            set.get("marble").and_then(|m| m.layers[0].darken),
            Some(0.5)
        );
        let coat = set
            .set_param(
                "marble",
                "look.coat",
                r#"{"weight": 1, "roughness": 0.05, "ior": 1.33}"#,
            )
            .expect("insert coat");
        assert_eq!(coat.before, "null");
        assert_eq!(
            set.get("marble").and_then(|m| m.look.coat).map(|c| c.ior),
            Some(1.33)
        );
        set.set_param("marble", "look.coat", "null")
            .expect("clear coat");
        assert_eq!(set.get("marble").and_then(|m| m.look.coat), None);
        let sound = set
            .set_param("marble", "sound.impact", "\"soft\"")
            .expect("sound");
        assert_eq!(sound.before, "\"stone\"");
        assert_eq!(
            set.get("marble").map(|m| m.body.sound),
            Some(SoundKind::Soft)
        );
    }

    #[test]
    fn set_param_refusals_leave_the_set_untouched() {
        let mut set = MaterialSet::siege_kit();
        let snapshot = set.clone();
        let err = set
            .set_param("marble", "look.specular_roughness", "1.5")
            .expect_err("out of range");
        assert_eq!(err.path, "look.specular_roughness");
        let err = set
            .set_param("marble", "look.velvet", "1")
            .expect_err("unknown lobe");
        assert_eq!(err.path, "look.velvet");
        assert_eq!(err.kind, RecordErrorKind::UnknownKey);
        let err = set
            .set_param("marble", "layers[9].darken", "1")
            .expect_err("no such layer");
        assert_eq!(err.path, "layers[9]");
        assert_eq!(err.kind, RecordErrorKind::Missing);
        let err = set
            .set_param("granite", "look.specular_roughness", "0.1")
            .expect_err("no such material");
        assert_eq!(err.path, "material");
        let err = set
            .set_param("marble", "material", "\"x\"")
            .expect_err("identity");
        assert_eq!(err.path, "material");
        let err = set
            .set_param("marble", "look.specular_roughness", "{")
            .expect_err("bad json");
        assert!(matches!(err.kind, RecordErrorKind::Json(_)));
        let err = set
            .set_param("marble", "look..x", "1")
            .expect_err("bad path");
        assert_eq!(err.path, "look..x");
        assert_eq!(set, snapshot);
    }

    #[test]
    fn set_param_on_a_child_keeps_inheriting_from_its_parent() {
        let mut set = MaterialSet::reference();
        let edit = set
            .set_param("marble_wet", "look.coat.ior", "1.4")
            .expect("child edit");
        assert_eq!(edit.before, "1.33");
        let wet = set.get("marble_wet").expect("wet");
        assert_eq!(wet.extends.as_deref(), Some("marble"));
        assert_eq!(wet.look.coat.map(|c| c.ior), Some(1.4));
        assert_eq!(wet.layers, set.get("marble").expect("marble").layers);
    }

    #[test]
    fn caps_hold_at_255_records() {
        let mut set = MaterialSet::new();
        let template = MaterialSet::siege_kit()
            .get("stone")
            .cloned()
            .expect("stone");
        for i in 0..MAX_RECORDS {
            let mut record = template.clone();
            record.name = format!("m{i}");
            set.push(record).expect("under cap");
        }
        assert_eq!(set.wire_id("m254"), Some(254));
        let mut over = template;
        over.name = "m255".to_string();
        let err = set.push(over).expect_err("over cap");
        assert_eq!(err.kind, RecordErrorKind::TooMany { max: MAX_RECORDS });
    }

    #[test]
    fn set_budget() {
        // Budget: wire ids are one byte with 255 kept free, so the 256th push refuses.
        assert_eq!(MAX_RECORDS, 255);
        let mut full = MaterialSet::new();
        let template = MaterialSet::siege_kit()
            .get("stone")
            .cloned()
            .expect("shipped stone");
        for i in 0..MAX_RECORDS {
            let mut record = template.clone();
            record.name = format!("m{i}");
            full.push(record).expect("slot under the cap");
        }
        assert_eq!(full.len(), MAX_RECORDS);
        let mut over = template.clone();
        over.name = "m255".to_string();
        let err = full.push(over).expect_err("256th push refuses");
        assert_eq!(err.path, "$");
        assert_eq!(err.kind, RecordErrorKind::TooMany { max: 255 });
        assert_eq!(err.kind, RecordErrorKind::TooMany { max: MAX_RECORDS });
        // Param edit lock: one look edit plus its inverse round-trips on 7 shipped records, bytes x2.
        let shipped = MaterialSet::shipped();
        let names: Vec<String> = shipped
            .records()
            .iter()
            .take(7)
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(names.len(), 7);
        for name in &names {
            let mut set = shipped.clone();
            let before_text = set.get(name).expect("shipped record").to_text();
            assert_eq!(
                before_text,
                set.get(name).expect("shipped record").to_text(),
                "{name} bytes x2"
            );
            let sim0 = set.sim_digest();
            let look0 = set.look_digest();
            let edit = set
                .set_param(name, "look.specular_roughness", "0.5")
                .expect("look edit on shipped record");
            assert_eq!(
                set.get(name).map(|m| m.look.specular_roughness),
                Some(0.5),
                "{name}"
            );
            assert_ne!(set.look_digest(), look0, "{name} look moves");
            assert_eq!(set.sim_digest(), sim0, "{name} sim still");
            set.set_param(name, "look.specular_roughness", &edit.before)
                .expect("inverse restores shipped record");
            assert_eq!(set, shipped, "{name} inverse round-trip");
            assert_eq!(set.sim_digest(), sim0, "{name} sim back");
            assert_eq!(set.look_digest(), look0, "{name} look back");
            assert_eq!(
                set.get(name).expect("shipped record").to_text(),
                before_text,
                "{name} text back"
            );
        }
    }

    #[test]
    fn shipped_is_the_siege_kit_then_the_new_reference_records() {
        let shipped = MaterialSet::shipped();
        let siege = MaterialSet::siege_kit();
        assert_eq!(shipped.len(), 19);
        for (i, record) in siege.records().iter().enumerate() {
            assert_eq!(shipped.records()[i], *record);
        }
        assert_eq!(shipped.wire_id("marble_wet"), Some(18));
        assert!(shipped.get("sand").is_some());
    }

    #[test]
    fn look_digest_names_the_record_order() {
        let mut a = MaterialSet::new();
        let mut b = MaterialSet::new();
        let kit = MaterialSet::siege_kit();
        let stone = kit.get("stone").cloned().expect("stone");
        let brick = kit.get("brick").cloned().expect("brick");
        a.push(stone.clone()).expect("a1");
        a.push(brick.clone()).expect("a2");
        b.push(brick).expect("b1");
        b.push(stone).expect("b2");
        assert_ne!(a.look_digest(), b.look_digest());
        assert_ne!(a.sim_digest(), b.sim_digest());
    }

    #[test]
    fn shipped_files_match_the_emitter() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/materials");
        let siege = MaterialSet::siege_kit();
        for (folder, set) in [
            ("siege", MaterialSet::siege_kit()),
            ("reference", MaterialSet::reference()),
        ] {
            for record in set.records() {
                // A reference record that the siege kit already owns ships once,
                // from `siege/`; a second copy in `reference/` would go unchecked.
                let owned_by_siege = folder == "reference" && siege.get(&record.name).is_some();
                let folder = if owned_by_siege { "siege" } else { folder };
                let path = root.join(folder).join(format!("{}.json", record.name));
                if owned_by_siege {
                    let dup = root.join("reference").join(format!("{}.json", record.name));
                    assert!(!dup.exists(), "{}: duplicate of siege/", dup.display());
                    assert_eq!(
                        Some(record),
                        siege.get(&record.name),
                        "{}: reference and siege disagree",
                        record.name
                    );
                }
                let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("{}: {e} (run the emit_seeds example)", path.display())
                });
                assert_eq!(
                    on_disk.replace("\r\n", "\n"),
                    record.to_text(),
                    "{}",
                    path.display()
                );
            }
        }
    }
}
