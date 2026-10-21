//! class_sheet: the class content loader (one class file -> one sheet).
//!
//! Owner: this module owns the class sheet shape and the loaders over
//! `content/classes/`; the JSON reader is `engine_material::json`. The content contract is
//! `content/classes/schema.json`; the rules the schema cannot hold (ability
//! count, ability ids and names unique in a file, an aoe ability needs a
//! telegraph, growth sums to a positive number, a primary stat grows) live
//! here and match `tools/class-validate.mjs`.
//! Units: ranges and radii in meters, cast and cooldown in seconds, mana in
//! mana points, damage in flat points plus the stat that scales it.
//! Determinism: the same file bytes give the same sheet, the directory loader
//! reads in file-name order, no clock and no RNG.
//! Errors: [`ClassSheetError`] names the file and the field; a bad sheet is
//! refused, never patched.
//! Budget: one-shot setup IO (one file read per sheet); never on the tick.

use std::fmt;
use std::path::Path;

use engine_material::json::{parse_up_to, Value as Json};

/// The roster file in the class directory: [`load_class_dir`] skips it.
pub const ROSTER_FILE: &str = "roster.json";

/// The schema file in the class directory: [`load_class_dir`] skips it.
pub const SCHEMA_FILE: &str = "schema.json";

/// Largest class file the loader reads, in bytes.
pub const MAX_CLASS_FILE_BYTES: usize = 256 * 1024;

/// Abilities every class file carries.
pub const MIN_ABILITIES: usize = 6;

/// The fields one class file may hold.
const CLASS_FIELDS: [&str; 17] = [
    "id",
    "faction",
    "tier",
    "branch",
    "role",
    "growth",
    "primary",
    "playstyle",
    "abilities",
    "loadout",
    "counters",
    "weakness",
    "signature",
    "fix",
    "notes",
    "description",
    "version",
];

/// The fields one ability row may hold.
const ABILITY_FIELDS: [&str; 15] = [
    "id",
    "name",
    "tier",
    "kind",
    "shape",
    "range",
    "radius",
    "cast",
    "cooldown",
    "mana",
    "telegraph",
    "damage",
    "effects",
    "notes",
    "tags",
];

/// One of the five stats of the roster.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stat {
    /// Strength.
    Str,
    /// Dexterity.
    Dex,
    /// Constitution.
    Con,
    /// Intelligence.
    Int,
    /// Wisdom.
    Wis,
}

impl Stat {
    /// The stat named by `text`; `None` when the sheet names another stat.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "str" => Some(Self::Str),
            "dex" => Some(Self::Dex),
            "con" => Some(Self::Con),
            "int" => Some(Self::Int),
            "wis" => Some(Self::Wis),
            _ => None,
        }
    }

    /// The sheet word of this stat.
    #[must_use]
    pub const fn as_text(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Dex => "dex",
            Self::Con => "con",
            Self::Int => "int",
            Self::Wis => "wis",
        }
    }
}

/// What an ability does. The sheet names one of these; any other word is
/// refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AbilityKind {
    /// A strike at one target.
    Melee,
    /// A shot that travels to one target.
    Projectile,
    /// A group hit over an area.
    Aoe,
    /// A helpful effect on an ally.
    Buff,
    /// A harmful effect on a target.
    Debuff,
    /// A heal on an ally.
    Heal,
    /// A hold, a root or a snare.
    Control,
    /// A hidden approach.
    Stealth,
    /// A move, a stance or any other support.
    Utility,
}

impl AbilityKind {
    /// The kind named by `text`; `None` when the sheet names another kind.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "melee" => Some(Self::Melee),
            "projectile" => Some(Self::Projectile),
            "aoe" => Some(Self::Aoe),
            "buff" => Some(Self::Buff),
            "debuff" => Some(Self::Debuff),
            "heal" => Some(Self::Heal),
            "control" => Some(Self::Control),
            "stealth" => Some(Self::Stealth),
            "utility" => Some(Self::Utility),
            _ => None,
        }
    }

    /// The sheet word of this kind.
    #[must_use]
    pub const fn as_text(self) -> &'static str {
        match self {
            Self::Melee => "melee",
            Self::Projectile => "projectile",
            Self::Aoe => "aoe",
            Self::Buff => "buff",
            Self::Debuff => "debuff",
            Self::Heal => "heal",
            Self::Control => "control",
            Self::Stealth => "stealth",
            Self::Utility => "utility",
        }
    }
}

/// The footprint an ability covers. The sheet names one of these; any other
/// word is refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AbilityShape {
    /// One target.
    Single,
    /// A wedge in front of the caster.
    Cone,
    /// A circle on the ground.
    Circle,
    /// A line from the caster.
    Line,
    /// The caster.
    SelfTarget,
    /// A picked target.
    Target,
}

impl AbilityShape {
    /// The shape named by `text`; `None` when the sheet names another shape.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "single" => Some(Self::Single),
            "cone" => Some(Self::Cone),
            "circle" => Some(Self::Circle),
            "line" => Some(Self::Line),
            "self" => Some(Self::SelfTarget),
            "target" => Some(Self::Target),
            _ => None,
        }
    }

    /// The sheet word of this shape.
    #[must_use]
    pub const fn as_text(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Cone => "cone",
            Self::Circle => "circle",
            Self::Line => "line",
            Self::SelfTarget => "self",
            Self::Target => "target",
        }
    }
}

/// The marker an ability shows before it lands. An aoe ability carries one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Telegraph {
    /// No marker: the ability lands without a warning.
    None,
    /// A circle on the ground.
    Circle,
    /// A wedge on the ground.
    Cone,
    /// A lane on the ground.
    Line,
    /// A charge towards the target.
    Charge,
}

impl Telegraph {
    /// The telegraph named by `text`; `None` when the sheet names another one.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "circle" => Some(Self::Circle),
            "cone" => Some(Self::Cone),
            "line" => Some(Self::Line),
            "charge" => Some(Self::Charge),
            _ => None,
        }
    }

    /// The sheet word of this telegraph.
    #[must_use]
    pub const fn as_text(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Circle => "circle",
            Self::Cone => "cone",
            Self::Line => "line",
            Self::Charge => "charge",
        }
    }
}

/// One value of a typed effect row.
#[derive(Clone, PartialEq, Debug)]
pub enum EffectValue {
    /// A text value.
    Text(String),
    /// A number value.
    Number(f32),
    /// A true or false value.
    Flag(bool),
}

/// One status effect of an ability: a keyword tag, a sentence, or a typed row
/// with its numbers.
#[derive(Clone, PartialEq, Debug)]
pub enum Effect {
    /// Plain text: a tag (`root`) or a sentence.
    Text(String),
    /// A typed effect: the type name plus its rows, in file order.
    Typed {
        /// The effect type (`heal_per_second`).
        effect_type: String,
        /// The other rows of the effect, in file order.
        rows: Vec<(String, EffectValue)>,
    },
}

/// Per-level growth of the five stats. A stat that does not grow is zero.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Growth {
    /// Strength growth per level.
    pub str: f32,
    /// Dexterity growth per level.
    pub dex: f32,
    /// Constitution growth per level.
    pub con: f32,
    /// Intelligence growth per level.
    pub int: f32,
    /// Wisdom growth per level.
    pub wis: f32,
}

impl Growth {
    /// The growth of one stat.
    #[must_use]
    pub fn get(&self, stat: Stat) -> f32 {
        match stat {
            Stat::Str => self.str,
            Stat::Dex => self.dex,
            Stat::Con => self.con,
            Stat::Int => self.int,
            Stat::Wis => self.wis,
        }
    }

    /// The sum of the five stats.
    #[must_use]
    pub fn sum(&self) -> f32 {
        self.str + self.dex + self.con + self.int + self.wis
    }
}

/// Damage of one ability: flat points plus the stat that scales the rest.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Damage {
    /// Flat damage points.
    pub base: f32,
    /// The stat that scales the damage; `None` when the damage is flat.
    pub scale: Option<Stat>,
}

/// The weapon and offhand a class fights with.
#[derive(Clone, PartialEq, Debug)]
pub struct Loadout {
    /// The weapon in hand.
    pub weapon: String,
    /// The offhand item; `None` when the class fights bare-handed.
    pub offhand: Option<String>,
}

/// One ability row of a class sheet.
#[derive(Clone, PartialEq, Debug)]
pub struct Ability {
    /// Stable ability id (`warden_fighter.iron_snare`).
    pub id: String,
    /// Name shown to the player.
    pub name: String,
    /// Tier of the ability inside the class tree.
    pub tier: u8,
    /// What the ability does.
    pub kind: AbilityKind,
    /// The footprint it covers.
    pub shape: AbilityShape,
    /// Reach of the ability in meters; zero for a self ability.
    pub range: f32,
    /// Radius of the footprint in meters; zero for a point hit.
    pub radius: f32,
    /// Cast time in seconds.
    pub cast: f32,
    /// Cooldown in seconds.
    pub cooldown: f32,
    /// Mana cost in mana points.
    pub mana: f32,
    /// The marker shown before the ability lands.
    pub telegraph: Telegraph,
    /// Damage the ability deals.
    pub damage: Damage,
    /// Status effects the ability applies.
    pub effects: Vec<Effect>,
    /// Design note for the ability; empty when the sheet carries none.
    pub notes: String,
    /// Search tags; empty when the sheet carries none.
    pub tags: Vec<String>,
}

/// One class sheet: the roster row plus its authored class content.
#[derive(Clone, PartialEq, Debug)]
pub struct ClassSheet {
    /// Class id (`warden_fighter`).
    pub id: String,
    /// Faction the class belongs to (`wardens`).
    pub faction: String,
    /// Tier of the class: zero for a starter, one for a branch.
    pub tier: u8,
    /// The base class this one branches from; `None` for a starter.
    pub branch: Option<String>,
    /// Role of the class (`line breaker`).
    pub role: String,
    /// Per-level growth of the five stats.
    pub growth: Growth,
    /// The stats this class builds on.
    pub primary: Vec<Stat>,
    /// How the class is played.
    pub playstyle: String,
    /// The abilities of the class, in sheet order.
    pub abilities: Vec<Ability>,
    /// The weapon and offhand the class fights with.
    pub loadout: Loadout,
    /// What this class answers well.
    pub counters: Vec<String>,
    /// What beats this class, in prose.
    pub weakness: String,
    /// Short identity line, as in `roster.json`.
    pub signature: Option<String>,
    /// Note on the change this class brings over its base.
    pub fix: Option<String>,
    /// Free notes on the class.
    pub notes: Option<String>,
    /// Prose about the class.
    pub description: Option<String>,
    /// File version, when the sheet carries one.
    pub version: Option<u32>,
}

/// Why a class sheet was refused: the file and the field, never a patch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClassSheetError {
    /// The file could not be read; the io kind is carried.
    Io {
        /// Name of the file (never a whole path).
        file: String,
        /// The io failure kind.
        kind: std::io::ErrorKind,
    },
    /// The file is not valid JSON; the reason is carried.
    BadJson {
        /// Name of the file (never a whole path).
        file: String,
        /// What the reader found.
        reason: String,
    },
    /// A required field is missing.
    MissingField {
        /// Name of the file (never a whole path).
        file: String,
        /// Field path, for example `abilities[0].kind`.
        field: String,
    },
    /// A field holds a value the sheet refuses.
    BadField {
        /// Name of the file (never a whole path).
        file: String,
        /// Field path, for example `abilities[0].range`.
        field: String,
        /// Why the value was refused.
        reason: String,
    },
}

impl fmt::Display for ClassSheetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { file, kind } => {
                write!(
                    f,
                    "class sheet {file:?}: the file cannot be read ({kind:?})"
                )
            }
            Self::BadJson { file, reason } => write!(f, "class sheet {file:?}: {reason}"),
            Self::MissingField { file, field } => {
                write!(f, "class sheet {file:?}: the field {field:?} is missing")
            }
            Self::BadField {
                file,
                field,
                reason,
            } => {
                write!(
                    f,
                    "class sheet {file:?}: the field {field:?} is refused: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for ClassSheetError {}

/// Load one class file and return its sheet. A missing file, a broken
/// document or a bad field is refused with the file and the field named.
pub fn load_class_file(path: &Path) -> Result<ClassSheet, ClassSheetError> {
    let label = file_label(path);
    let bytes = std::fs::read(path).map_err(|error| ClassSheetError::Io {
        file: label.clone(),
        kind: error.kind(),
    })?;
    if bytes.len() > MAX_CLASS_FILE_BYTES {
        return Err(ClassSheetError::BadJson {
            file: label,
            reason: format!("the file is larger than {MAX_CLASS_FILE_BYTES} bytes"),
        });
    }
    let text = String::from_utf8(bytes).map_err(|_| ClassSheetError::BadJson {
        file: label.clone(),
        reason: "the file is not UTF-8 text".to_string(),
    })?;
    ClassSheet::parse(&label, &text)
}

/// Load every class file of a directory in file-name order. [`ROSTER_FILE`]
/// and [`SCHEMA_FILE`] are skipped; a bad file fails the whole load, and the
/// cross-file rules hold (a branch points at a tier 0 class of the set, an
/// ability id belongs to one class).
pub fn load_class_dir(dir: &Path) -> Result<Vec<ClassSheet>, ClassSheetError> {
    let entries = std::fs::read_dir(dir).map_err(|error| ClassSheetError::Io {
        file: file_label(dir),
        kind: error.kind(),
    })?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ClassSheetError::Io {
            file: file_label(dir),
            kind: error.kind(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if !name.ends_with(".json") || name == ROSTER_FILE || name == SCHEMA_FILE {
            continue;
        }
        names.push(name);
    }
    names.sort();
    let mut sheets = Vec::with_capacity(names.len());
    for name in &names {
        sheets.push(load_class_file(&dir.join(name))?);
    }
    cross_file_rules(&sheets, &names)?;
    Ok(sheets)
}

/// The rules that span the class files: a branch points at a tier 0 class of
/// the set and an ability id belongs to one class. `names` holds the file
/// name of every sheet, in the same order.
fn cross_file_rules(sheets: &[ClassSheet], names: &[String]) -> Result<(), ClassSheetError> {
    let mut owners: Vec<(&str, &str)> = Vec::new();
    for (sheet, name) in sheets.iter().zip(names) {
        for (index, ability) in sheet.abilities.iter().enumerate() {
            if let Some((owner, _)) = owners.iter().find(|(id, _)| *id == ability.id) {
                return Err(ClassSheetError::BadField {
                    file: name.clone(),
                    field: ability_path(index, "id"),
                    reason: format!("the ability id is used by the class {owner:?} too"),
                });
            }
            owners.push((ability.id.as_str(), sheet.id.as_str()));
        }
    }
    for (sheet, name) in sheets.iter().zip(names) {
        let Some(branch) = sheet.branch.as_deref() else {
            continue;
        };
        let Some(target) = sheets.iter().find(|other| other.id == branch) else {
            return Err(ClassSheetError::BadField {
                file: name.clone(),
                field: "branch".to_string(),
                reason: format!("the class {branch:?} is not in the set"),
            });
        };
        if target.tier != 0 {
            return Err(ClassSheetError::BadField {
                file: name.clone(),
                field: "branch".to_string(),
                reason: format!("the class {branch:?} is tier {}, not tier 0", target.tier),
            });
        }
    }
    Ok(())
}

impl ClassSheet {
    /// Parse one class sheet from JSON text. `file_name` labels the errors;
    /// the same text always gives the same sheet.
    pub fn parse(file_name: &str, text: &str) -> Result<Self, ClassSheetError> {
        let root =
            parse_up_to(text, MAX_CLASS_FILE_BYTES).map_err(|error| ClassSheetError::BadJson {
                file: file_name.to_string(),
                reason: error.to_string(),
            })?;
        let ctx = Ctx { file: file_name };
        let Some(members) = root.as_object() else {
            return Err(ctx.bad("the sheet", "the document is not an object"));
        };
        for (key, _) in members {
            if !CLASS_FIELDS.contains(&key.as_str()) {
                return Err(ctx.bad(key, "the class file has no such field"));
            }
        }
        let id = text_field(&ctx, &root, "id", "id")?;
        let faction = text_field(&ctx, &root, "faction", "faction")?;
        let tier = tier_field(&ctx, &root, "tier", "tier")?;
        let branch = branch_field(&ctx, &root)?;
        let role = text_field(&ctx, &root, "role", "role")?;
        let growth = growth(&ctx, &root)?;
        let primary = primary(&ctx, &root, growth)?;
        let playstyle = text_field(&ctx, &root, "playstyle", "playstyle")?;
        let abilities = abilities(&ctx, &root)?;
        let loadout = loadout(&ctx, &root)?;
        let counters = string_list_field(&ctx, &root, "counters", "counters")?;
        let weakness = text_field(&ctx, &root, "weakness", "weakness")?;
        let signature = optional_text_field(&ctx, &root, "signature", "signature")?;
        let fix = optional_text_field(&ctx, &root, "fix", "fix")?;
        let notes = optional_text_field(&ctx, &root, "notes", "notes")?;
        let description = optional_text_field(&ctx, &root, "description", "description")?;
        let version = optional_tier_field(&ctx, &root, "version", "version")?;
        Ok(Self {
            id,
            faction,
            tier,
            branch,
            role,
            growth,
            primary,
            playstyle,
            abilities,
            loadout,
            counters,
            weakness,
            signature,
            fix,
            notes,
            description,
            version,
        })
    }
}

/// The file label every error carries: the file name, never a whole path.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The file label of one error, kept next to the reader that raises it.
struct Ctx<'a> {
    file: &'a str,
}

impl Ctx<'_> {
    fn missing(&self, field: &str) -> ClassSheetError {
        ClassSheetError::MissingField {
            file: self.file.to_string(),
            field: field.to_string(),
        }
    }

    fn bad(&self, field: &str, reason: &str) -> ClassSheetError {
        ClassSheetError::BadField {
            file: self.file.to_string(),
            field: field.to_string(),
            reason: reason.to_string(),
        }
    }
}

/// Read the branch of a class: a class id, or null for a starter.
fn branch_field(ctx: &Ctx<'_>, root: &Json) -> Result<Option<String>, ClassSheetError> {
    let value = member(ctx, root, "branch", "branch")?;
    match value {
        Json::Null => Ok(None),
        Json::Str(text) => Ok(Some(text.clone())),
        other => Err(ctx.bad(
            "branch",
            &format!("expected a class id or null, found {}", other.kind_name()),
        )),
    }
}

/// Read the growth row: the five stats, all of them named.
fn growth(ctx: &Ctx<'_>, root: &Json) -> Result<Growth, ClassSheetError> {
    let node = member(ctx, root, "growth", "growth")?;
    let Some(members) = node.as_object() else {
        return Err(ctx.bad("growth", "expected an object of stat rows"));
    };
    for (key, _) in members {
        if Stat::from_text(key).is_none() {
            return Err(ctx.bad(&format!("growth.{key}"), "the roster has no such stat"));
        }
    }
    let mut out = Growth::default();
    for stat in [Stat::Str, Stat::Dex, Stat::Con, Stat::Int, Stat::Wis] {
        let path = format!("growth.{}", stat.as_text());
        let value = number_node(
            ctx,
            member(ctx, node, stat.as_text(), &path)?,
            &path,
            f32::MAX,
        )?;
        *growth_slot(&mut out, stat) = value;
    }
    if out.sum() <= 0.0 {
        return Err(ctx.bad("growth", "the growth sums to zero, a class grows something"));
    }
    Ok(out)
}

/// The slot of one stat inside a growth row.
fn growth_slot(growth: &mut Growth, stat: Stat) -> &mut f32 {
    match stat {
        Stat::Str => &mut growth.str,
        Stat::Dex => &mut growth.dex,
        Stat::Con => &mut growth.con,
        Stat::Int => &mut growth.int,
        Stat::Wis => &mut growth.wis,
    }
}

/// Read the primary stats of a class; each one must grow.
fn primary(ctx: &Ctx<'_>, root: &Json, growth: Growth) -> Result<Vec<Stat>, ClassSheetError> {
    let node = member(ctx, root, "primary", "primary")?;
    let Some(items) = node.as_array() else {
        return Err(ctx.bad("primary", "expected an array of stat names"));
    };
    if items.is_empty() {
        return Err(ctx.bad("primary", "the class needs at least one primary stat"));
    }
    let mut out: Vec<Stat> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let path = format!("primary[{index}]");
        let Some(text) = item.as_str() else {
            return Err(ctx.bad(
                &path,
                &format!("expected a stat name, found {}", item.kind_name()),
            ));
        };
        let Some(stat) = Stat::from_text(text) else {
            return Err(ctx.bad(&path, &format!("unknown stat {text:?}")));
        };
        if out.contains(&stat) {
            return Err(ctx.bad(&path, &format!("the stat {text:?} is listed twice")));
        }
        if growth.get(stat) <= 0.0 {
            return Err(ctx.bad(
                &path,
                &format!("the class lists {text:?} but it does not grow it"),
            ));
        }
        out.push(stat);
    }
    Ok(out)
}

/// Read the ability rows of a sheet, in sheet order.
fn abilities(ctx: &Ctx<'_>, root: &Json) -> Result<Vec<Ability>, ClassSheetError> {
    let node = member(ctx, root, "abilities", "abilities")?;
    let Some(items) = node.as_array() else {
        return Err(ctx.bad("abilities", "expected an array of ability rows"));
    };
    if items.len() < MIN_ABILITIES {
        return Err(ctx.bad(
            "abilities",
            &format!(
                "the class has {} abilities, it needs at least {MIN_ABILITIES}",
                items.len()
            ),
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        out.push(ability(ctx, item, index)?);
    }
    let mut ids: Vec<&str> = Vec::with_capacity(out.len());
    for (index, ability) in out.iter().enumerate() {
        if ids.contains(&ability.id.as_str()) {
            return Err(ctx.bad(
                &ability_path(index, "id"),
                &format!("the ability id {:?} is used twice in this file", ability.id),
            ));
        }
        ids.push(ability.id.as_str());
    }
    let mut names: Vec<&str> = Vec::with_capacity(out.len());
    for (index, ability) in out.iter().enumerate() {
        if names.contains(&ability.name.as_str()) {
            return Err(ctx.bad(
                &ability_path(index, "name"),
                &format!(
                    "the ability name {:?} is used twice in this file",
                    ability.name
                ),
            ));
        }
        names.push(ability.name.as_str());
    }
    Ok(out)
}

/// Read one ability row.
fn ability(ctx: &Ctx<'_>, node: &Json, index: usize) -> Result<Ability, ClassSheetError> {
    let Some(members) = node.as_object() else {
        return Err(ctx.bad(&ability_path(index, ""), "expected an object"));
    };
    for (key, _) in members {
        if !ABILITY_FIELDS.contains(&key.as_str()) {
            return Err(ctx.bad(&ability_path(index, key), "the ability has no such field"));
        }
    }
    let id = text_field(ctx, node, "id", &ability_path(index, "id"))?;
    let name = text_field(ctx, node, "name", &ability_path(index, "name"))?;
    let tier = tier_field(ctx, node, "tier", &ability_path(index, "tier"))?;
    let kind = kind_field(ctx, node, &ability_path(index, "kind"))?;
    let shape = shape_field(ctx, node, &ability_path(index, "shape"))?;
    let range = number_field(ctx, node, "range", &ability_path(index, "range"), 60.0)?;
    let radius = number_field(ctx, node, "radius", &ability_path(index, "radius"), 30.0)?;
    let cast = number_field(ctx, node, "cast", &ability_path(index, "cast"), 10.0)?;
    let cooldown = number_field(
        ctx,
        node,
        "cooldown",
        &ability_path(index, "cooldown"),
        600.0,
    )?;
    let mana = number_field(ctx, node, "mana", &ability_path(index, "mana"), 200.0)?;
    let telegraph = telegraph_field(ctx, node, &ability_path(index, "telegraph"))?;
    let damage = damage(ctx, node, &ability_path(index, "damage"))?;
    let effects = effects(ctx, node, &ability_path(index, "effects"))?;
    let notes = text_field(ctx, node, "notes", &ability_path(index, "notes"))?;
    let tags = optional_string_list(ctx, node, "tags", &ability_path(index, "tags"))?;
    if kind == AbilityKind::Aoe && telegraph == Telegraph::None {
        return Err(ctx.bad(
            &ability_path(index, "telegraph"),
            "an aoe ability needs a telegraph other than none",
        ));
    }
    Ok(Ability {
        id,
        name,
        tier,
        kind,
        shape,
        range,
        radius,
        cast,
        cooldown,
        mana,
        telegraph,
        damage,
        effects,
        notes,
        tags,
    })
}

/// Read the damage row of one ability.
fn damage(ctx: &Ctx<'_>, node: &Json, path: &str) -> Result<Damage, ClassSheetError> {
    let row = member(ctx, node, "damage", path)?;
    let Some(members) = row.as_object() else {
        return Err(ctx.bad(path, "expected an object with the rows base and scale"));
    };
    for (key, _) in members {
        if key != "base" && key != "scale" {
            return Err(ctx.bad(&format!("{path}.{key}"), "the damage has no such row"));
        }
    }
    let base = number_field(ctx, row, "base", &format!("{path}.base"), 500.0)?;
    let scale = member(ctx, row, "scale", &format!("{path}.scale"))?;
    let Some(text) = scale.as_str() else {
        return Err(ctx.bad(
            &format!("{path}.scale"),
            &format!("expected a stat name, found {}", scale.kind_name()),
        ));
    };
    let scale = if text == "none" {
        None
    } else {
        let Some(stat) = Stat::from_text(text) else {
            return Err(ctx.bad(&format!("{path}.scale"), &format!("unknown stat {text:?}")));
        };
        Some(stat)
    };
    Ok(Damage { base, scale })
}

/// Read the effects of one ability.
fn effects(ctx: &Ctx<'_>, node: &Json, path: &str) -> Result<Vec<Effect>, ClassSheetError> {
    let row = member(ctx, node, "effects", path)?;
    let Some(items) = row.as_array() else {
        return Err(ctx.bad(path, "expected an array of status effects"));
    };
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        out.push(effect(ctx, item, &format!("{path}[{index}]"))?);
    }
    Ok(out)
}

/// Read one status effect: a tag, a sentence or a typed row.
fn effect(ctx: &Ctx<'_>, item: &Json, path: &str) -> Result<Effect, ClassSheetError> {
    match item {
        Json::Str(text) => Ok(Effect::Text(text.clone())),
        Json::Object(members) => {
            let Some((_, kind)) = members.iter().find(|(key, _)| key == "type") else {
                return Err(ctx.bad(path, "a typed effect needs a type row"));
            };
            let Some(effect_type) = kind.as_str() else {
                return Err(ctx.bad(
                    &format!("{path}.type"),
                    &format!("expected a text, found {}", kind.kind_name()),
                ));
            };
            let mut rows = Vec::with_capacity(members.len());
            for (key, value) in members {
                if key == "type" {
                    continue;
                }
                rows.push((
                    key.clone(),
                    effect_value(ctx, value, &format!("{path}.{key}"))?,
                ));
            }
            Ok(Effect::Typed {
                effect_type: effect_type.to_string(),
                rows,
            })
        }
        other => Err(ctx.bad(
            path,
            &format!(
                "expected a tag, a sentence or an object, found {}",
                other.kind_name()
            ),
        )),
    }
}

/// Read one value of a typed effect row.
fn effect_value(ctx: &Ctx<'_>, value: &Json, path: &str) -> Result<EffectValue, ClassSheetError> {
    match value {
        Json::Str(text) => Ok(EffectValue::Text(text.clone())),
        Json::Bool(flag) => Ok(EffectValue::Flag(*flag)),
        Json::Number(lexeme) => {
            let parsed: f32 = lexeme
                .parse()
                .map_err(|_| ctx.bad(path, "the number does not fit a 32-bit float"))?;
            Ok(EffectValue::Number(parsed))
        }
        other => Err(ctx.bad(
            path,
            &format!(
                "expected a text, a number or a flag, found {}",
                other.kind_name()
            ),
        )),
    }
}

/// Read the loadout of a class.
fn loadout(ctx: &Ctx<'_>, root: &Json) -> Result<Loadout, ClassSheetError> {
    let node = member(ctx, root, "loadout", "loadout")?;
    let Some(members) = node.as_object() else {
        return Err(ctx.bad("loadout", "expected an object with weapon and offhand"));
    };
    for (key, _) in members {
        if key != "weapon" && key != "offhand" {
            return Err(ctx.bad(&format!("loadout.{key}"), "the loadout has no such row"));
        }
    }
    let weapon = text_field(ctx, node, "weapon", "loadout.weapon")?;
    let offhand = member(ctx, node, "offhand", "loadout.offhand")?;
    let offhand = match offhand {
        Json::Null => None,
        Json::Str(text) => Some(text.clone()),
        other => {
            return Err(ctx.bad(
                "loadout.offhand",
                &format!("expected an item or null, found {}", other.kind_name()),
            ))
        }
    };
    Ok(Loadout { weapon, offhand })
}

/// The error path of one field of the ability at `index`.
fn ability_path(index: usize, field: &str) -> String {
    if field.is_empty() {
        format!("abilities[{index}]")
    } else {
        format!("abilities[{index}].{field}")
    }
}

/// Read one required member of an object.
fn member<'a>(
    ctx: &Ctx<'_>,
    node: &'a Json,
    key: &str,
    path: &str,
) -> Result<&'a Json, ClassSheetError> {
    node.get(key).ok_or_else(|| ctx.missing(path))
}

/// Read one required string member; an empty string is refused.
fn text_field(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
) -> Result<String, ClassSheetError> {
    let value = member(ctx, node, key, path)?;
    let Some(text) = value.as_str() else {
        return Err(ctx.bad(
            path,
            &format!("expected a string, found {}", value.kind_name()),
        ));
    };
    if text.is_empty() {
        return Err(ctx.bad(path, "the text is empty"));
    }
    Ok(text.to_string())
}

/// Read one optional string member; a missing member reads as nothing.
fn optional_text_field(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
) -> Result<Option<String>, ClassSheetError> {
    let Some(value) = node.get(key) else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(ctx.bad(
            path,
            &format!("expected a string, found {}", value.kind_name()),
        ));
    };
    Ok(Some(text.to_string()))
}

/// Read one required number member; a negative or too large number is refused.
fn number_field(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
    max: f32,
) -> Result<f32, ClassSheetError> {
    let value = member(ctx, node, key, path)?;
    number_node(ctx, value, path, max)
}

/// Read one number node; a negative or too large number is refused.
fn number_node(ctx: &Ctx<'_>, value: &Json, path: &str, max: f32) -> Result<f32, ClassSheetError> {
    let Some(lexeme) = value.number_text() else {
        return Err(ctx.bad(
            path,
            &format!("expected a number, found {}", value.kind_name()),
        ));
    };
    let parsed: f32 = lexeme
        .parse()
        .map_err(|_| ctx.bad(path, "the number does not fit a 32-bit float"))?;
    if !parsed.is_finite() {
        return Err(ctx.bad(path, "the number is not finite"));
    }
    if parsed < 0.0 {
        return Err(ctx.bad(path, "the number is negative"));
    }
    if parsed > max {
        return Err(ctx.bad(path, &format!("the number is above {max}")));
    }
    Ok(parsed)
}

/// Read one required tier member: a whole number in `0..=4`.
fn tier_field(ctx: &Ctx<'_>, node: &Json, key: &str, path: &str) -> Result<u8, ClassSheetError> {
    let value = member(ctx, node, key, path)?;
    let Some(lexeme) = value.number_text() else {
        return Err(ctx.bad(
            path,
            &format!("expected a number, found {}", value.kind_name()),
        ));
    };
    let tier: u8 = lexeme
        .parse()
        .map_err(|_| ctx.bad(path, "the tier is not a whole number in 0..=4"))?;
    if tier > 4 {
        return Err(ctx.bad(path, "the tier is above 4"));
    }
    Ok(tier)
}

/// Read one optional tier member.
fn optional_tier_field(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
) -> Result<Option<u32>, ClassSheetError> {
    let Some(value) = node.get(key) else {
        return Ok(None);
    };
    let Some(lexeme) = value.number_text() else {
        return Err(ctx.bad(
            path,
            &format!("expected a number, found {}", value.kind_name()),
        ));
    };
    let version: u32 = lexeme
        .parse()
        .map_err(|_| ctx.bad(path, "the version is not a whole number"))?;
    Ok(Some(version))
}

/// Read one required list member: an array of strings.
fn string_list_field(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
) -> Result<Vec<String>, ClassSheetError> {
    let value = member(ctx, node, key, path)?;
    let Some(items) = value.as_array() else {
        return Err(ctx.bad(
            path,
            &format!("expected an array of strings, found {}", value.kind_name()),
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(text) = item.as_str() else {
            return Err(ctx.bad(
                &item_path,
                &format!("expected a string, found {}", item.kind_name()),
            ));
        };
        out.push(text.to_string());
    }
    Ok(out)
}

/// Read one optional list member; a missing member reads as an empty list.
fn optional_string_list(
    ctx: &Ctx<'_>,
    node: &Json,
    key: &str,
    path: &str,
) -> Result<Vec<String>, ClassSheetError> {
    let Some(value) = node.get(key) else {
        return Ok(Vec::new());
    };
    string_list_node(ctx, value, path)
}

/// Read one list node: an array of strings.
fn string_list_node(
    ctx: &Ctx<'_>,
    value: &Json,
    path: &str,
) -> Result<Vec<String>, ClassSheetError> {
    let Some(items) = value.as_array() else {
        return Err(ctx.bad(
            path,
            &format!("expected an array of strings, found {}", value.kind_name()),
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(text) = item.as_str() else {
            return Err(ctx.bad(
                &item_path,
                &format!("expected a string, found {}", item.kind_name()),
            ));
        };
        out.push(text.to_string());
    }
    Ok(out)
}

/// Read one required ability kind.
fn kind_field(ctx: &Ctx<'_>, node: &Json, path: &str) -> Result<AbilityKind, ClassSheetError> {
    let value = member(ctx, node, "kind", path)?;
    let Some(text) = value.as_str() else {
        return Err(ctx.bad(
            path,
            &format!("expected a string, found {}", value.kind_name()),
        ));
    };
    AbilityKind::from_text(text)
        .ok_or_else(|| ctx.bad(path, &format!("unknown ability kind {text:?}")))
}

/// Read one required ability shape.
fn shape_field(ctx: &Ctx<'_>, node: &Json, path: &str) -> Result<AbilityShape, ClassSheetError> {
    let value = member(ctx, node, "shape", path)?;
    let Some(text) = value.as_str() else {
        return Err(ctx.bad(
            path,
            &format!("expected a string, found {}", value.kind_name()),
        ));
    };
    AbilityShape::from_text(text)
        .ok_or_else(|| ctx.bad(path, &format!("unknown ability shape {text:?}")))
}

/// Read one required telegraph.
fn telegraph_field(ctx: &Ctx<'_>, node: &Json, path: &str) -> Result<Telegraph, ClassSheetError> {
    let value = member(ctx, node, "telegraph", path)?;
    let Some(text) = value.as_str() else {
        return Err(ctx.bad(
            path,
            &format!("expected a string, found {}", value.kind_name()),
        ));
    };
    Telegraph::from_text(text).ok_or_else(|| ctx.bad(path, &format!("unknown telegraph {text:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One good class file: six abilities, the roster shape.
    const GOOD_SHEET: &str = r#"{
      "id": "warden_fighter",
      "faction": "wardens",
      "tier": 0,
      "branch": null,
      "role": "line breaker",
      "growth": {"str": 1, "dex": 0, "con": 1, "int": 0, "wis": 0},
      "primary": ["str", "con"],
      "playstyle": "Stand in front of the line and let them come to you.",
      "abilities": [
        {
          "id": "warden_fighter.iron_snare",
          "name": "Iron Snare",
          "tier": 0,
          "kind": "control",
          "shape": "single",
          "range": 9,
          "radius": 0,
          "cast": 0.4,
          "cooldown": 14,
          "mana": 12,
          "telegraph": "none",
          "damage": {"base": 0, "scale": "none"},
          "effects": ["root", "damage"],
          "notes": "Paralyse one enemy for 1.6 s."
        },
        {
          "id": "warden_fighter.heavy_arc",
          "name": "Heavy Arc",
          "tier": 0,
          "kind": "aoe",
          "shape": "cone",
          "range": 2.6,
          "radius": 3,
          "cast": 0.5,
          "cooldown": 3,
          "mana": 0,
          "telegraph": "cone",
          "damage": {"base": 28, "scale": "str"},
          "effects": [{"type": "damage_reduction", "value": 25, "duration": 6}],
          "notes": "A wide swing for everything in front of you."
        },
        {
          "id": "warden_fighter.fencing_step",
          "name": "Fencing Step",
          "tier": 0,
          "kind": "utility",
          "shape": "self",
          "range": 0,
          "radius": 0,
          "cast": 0,
          "cooldown": 12,
          "mana": 0,
          "telegraph": "none",
          "damage": {"base": 0, "scale": "none"},
          "effects": ["move_speed"],
          "notes": "Half a second of faster footwork."
        },
        {
          "id": "warden_fighter.breaker_blow",
          "name": "Breaker Blow",
          "tier": 1,
          "kind": "melee",
          "shape": "single",
          "range": 2.4,
          "radius": 0,
          "cast": 0.6,
          "cooldown": 5,
          "mana": 0,
          "telegraph": "none",
          "damage": {"base": 52, "scale": "str"},
          "effects": ["damage"],
          "notes": "A committed overhead that staggers."
        },
        {
          "id": "warden_fighter.shield_wall",
          "name": "Shield Wall",
          "tier": 2,
          "kind": "buff",
          "shape": "circle",
          "range": 6,
          "radius": 6,
          "cast": 0,
          "cooldown": 22,
          "mana": 18,
          "telegraph": "none",
          "damage": {"base": 0, "scale": "none"},
          "effects": ["damage_reduction"],
          "notes": "A short-lived guard stance."
        },
        {
          "id": "warden_fighter.last_stand",
          "name": "Last Stand",
          "tier": 4,
          "kind": "utility",
          "shape": "self",
          "range": 0,
          "radius": 0,
          "cast": 0,
          "cooldown": 90,
          "mana": 30,
          "telegraph": "none",
          "damage": {"base": 0, "scale": "none"},
          "effects": ["heal"],
          "notes": "For ten seconds you cannot be knocked down."
        }
      ],
      "loadout": {"weapon": "sword", "offhand": "buckler"},
      "counters": ["warden_sorcerer"],
      "weakness": "Low damage."
    }"#;

    /// A roster file: valid JSON, but not a class file.
    const ROSTER: &str = r#"{"version": 1, "classes": [{"id": "warden_fighter"}]}"#;

    fn sheet_with(block: &str) -> String {
        GOOD_SHEET.replace(
            r#""kind": "control",
          "shape": "single",
          "range": 9,"#,
            block,
        )
    }

    fn error_of(text: &str) -> ClassSheetError {
        ClassSheet::parse("warden_fighter.json", text).expect_err("the sheet is refused")
    }

    /// The good sheet with its last ability cut out: five abilities remain.
    fn sheet_without_the_last_ability() -> String {
        let marker = "\"id\": \"warden_fighter.last_stand\"";
        let at = GOOD_SHEET
            .find(marker)
            .expect("the marker is in the fixture");
        let cut = GOOD_SHEET[..at]
            .rfind(",\n        {")
            .expect("the last ability opens with a comma");
        let tail = GOOD_SHEET
            .find("\n      ],\n      \"loadout\"")
            .expect("the ability array closes");
        let mut text = GOOD_SHEET[..cut].to_string();
        text.push_str(&GOOD_SHEET[tail..]);
        text
    }

    #[test]
    fn a_good_sheet_parses() {
        let sheet = ClassSheet::parse("warden_fighter.json", GOOD_SHEET).expect("a good sheet");
        assert_eq!(sheet.id, "warden_fighter");
        assert_eq!(sheet.faction, "wardens");
        assert_eq!(sheet.tier, 0);
        assert_eq!(sheet.branch, None);
        assert_eq!(sheet.role, "line breaker");
        assert_eq!(sheet.growth.str, 1.0);
        assert_eq!(sheet.growth.con, 1.0);
        assert_eq!(sheet.growth.dex, 0.0);
        assert_eq!(sheet.growth.sum(), 2.0);
        assert_eq!(sheet.primary, [Stat::Str, Stat::Con]);
        assert_eq!(
            sheet.playstyle,
            "Stand in front of the line and let them come to you."
        );
        assert_eq!(sheet.loadout.weapon, "sword");
        assert_eq!(sheet.loadout.offhand.as_deref(), Some("buckler"));
        assert_eq!(sheet.counters, ["warden_sorcerer"]);
        assert_eq!(sheet.weakness, "Low damage.");
        assert_eq!(sheet.abilities.len(), MIN_ABILITIES);
        let ability = &sheet.abilities[0];
        assert_eq!(ability.id, "warden_fighter.iron_snare");
        assert_eq!(ability.name, "Iron Snare");
        assert_eq!(ability.kind, AbilityKind::Control);
        assert_eq!(ability.shape, AbilityShape::Single);
        assert_eq!(ability.range, 9.0);
        assert_eq!(ability.radius, 0.0);
        assert_eq!(ability.cast, 0.4);
        assert_eq!(ability.cooldown, 14.0);
        assert_eq!(ability.mana, 12.0);
        assert_eq!(ability.telegraph, Telegraph::None);
        assert_eq!(ability.damage.base, 0.0);
        assert_eq!(ability.damage.scale, None);
        assert_eq!(ability.effects.len(), 2);
        assert_eq!(ability.notes, "Paralyse one enemy for 1.6 s.");
        assert!(ability.tags.is_empty());
        let arc = &sheet.abilities[1];
        assert_eq!(arc.kind, AbilityKind::Aoe);
        assert_eq!(arc.telegraph, Telegraph::Cone);
        assert_eq!(arc.damage.scale, Some(Stat::Str));
        assert_eq!(
            arc.effects[0],
            Effect::Typed {
                effect_type: "damage_reduction".to_string(),
                rows: vec![
                    ("value".to_string(), EffectValue::Number(25.0)),
                    ("duration".to_string(), EffectValue::Number(6.0)),
                ],
            }
        );
    }

    #[test]
    fn the_same_bytes_give_the_same_sheet() {
        let first = ClassSheet::parse("warden_fighter.json", GOOD_SHEET).expect("a good sheet");
        let again = ClassSheet::parse("warden_fighter.json", GOOD_SHEET).expect("a good sheet");
        assert_eq!(first, again);
    }

    #[test]
    fn an_unknown_ability_kind_is_refused() {
        let text = sheet_with(
            r#""kind": "beam",
          "shape": "single",
          "range": 9,"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "abilities[0].kind".to_string(),
                reason: "unknown ability kind \"beam\"".to_string(),
            }
        );
        assert!(error.to_string().contains("abilities[0].kind"));
        assert!(error.to_string().contains("warden_fighter.json"));
    }

    #[test]
    fn an_unknown_ability_shape_is_refused() {
        let text = sheet_with(
            r#""kind": "control",
          "shape": "blob",
          "range": 9,"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "abilities[0].shape".to_string(),
                reason: "unknown ability shape \"blob\"".to_string(),
            }
        );
    }

    #[test]
    fn a_missing_field_is_refused() {
        let text = GOOD_SHEET.replace(
            r#""playstyle": "Stand in front of the line and let them come to you.","#,
            "",
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::MissingField {
                file: "warden_fighter.json".to_string(),
                field: "playstyle".to_string(),
            }
        );
        assert!(error.to_string().contains("playstyle"));
        assert!(error.to_string().contains("warden_fighter.json"));
    }

    #[test]
    fn a_missing_growth_stat_is_refused() {
        let text = GOOD_SHEET.replace(r#""int": 0, "wis": 0"#, r#""int": 0"#);
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::MissingField {
                file: "warden_fighter.json".to_string(),
                field: "growth.wis".to_string(),
            }
        );
    }

    #[test]
    fn a_negative_range_is_refused() {
        let text = sheet_with(
            r#""kind": "control",
          "shape": "single",
          "range": -9,"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "abilities[0].range".to_string(),
                reason: "the number is negative".to_string(),
            }
        );
    }

    #[test]
    fn an_aoe_ability_needs_a_telegraph() {
        let text = sheet_with(
            r#""kind": "aoe",
          "shape": "circle",
          "range": 9,"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "abilities[0].telegraph".to_string(),
                reason: "an aoe ability needs a telegraph other than none".to_string(),
            }
        );
    }

    #[test]
    fn a_class_with_too_few_abilities_is_refused() {
        let text = sheet_without_the_last_ability();
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "abilities".to_string(),
                reason: "the class has 5 abilities, it needs at least 6".to_string(),
            }
        );
    }

    #[test]
    fn a_primary_stat_that_does_not_grow_is_refused() {
        let text = GOOD_SHEET.replace(
            r#""primary": ["str", "con"]"#,
            r#""primary": ["str", "wis"]"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "primary[1]".to_string(),
                reason: "the class lists \"wis\" but it does not grow it".to_string(),
            }
        );
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let text = GOOD_SHEET.replace(
            r#""role": "line breaker","#,
            r#""role": "line breaker", "power": 3,"#,
        );
        let error = error_of(&text);
        assert_eq!(
            error,
            ClassSheetError::BadField {
                file: "warden_fighter.json".to_string(),
                field: "power".to_string(),
                reason: "the class file has no such field".to_string(),
            }
        );
    }

    #[test]
    fn a_broken_document_is_refused() {
        let error = error_of("{ \"id\": ");
        assert!(matches!(error, ClassSheetError::BadJson { .. }));
        assert!(error.to_string().contains("warden_fighter.json"));
    }

    #[test]
    fn the_directory_loader_skips_the_roster() {
        let dir = scratch_dir("dir_skips_roster");
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        std::fs::write(dir.join("warden_fighter.json"), GOOD_SHEET).expect("the class file");
        std::fs::write(dir.join(ROSTER_FILE), ROSTER).expect("the roster file");
        std::fs::write(dir.join(SCHEMA_FILE), "{}").expect("the schema file");
        std::fs::write(dir.join("notes.txt"), "not a class").expect("the text file");
        let sheets = load_class_dir(&dir).expect("the roster and the schema are skipped");
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].id, "warden_fighter");
        assert_eq!(sheets[0].abilities.len(), MIN_ABILITIES);
        std::fs::remove_dir_all(&dir).expect("the scratch dir goes");
    }

    #[test]
    fn the_directory_loader_fails_loud_on_a_bad_class_file() {
        let dir = scratch_dir("dir_bad_file");
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        std::fs::write(dir.join("broken.json"), "{ \"id\": ").expect("the broken file");
        let error = load_class_dir(&dir).expect_err("the broken file is refused");
        assert!(matches!(
            error,
            ClassSheetError::BadJson { ref file, .. } if file == "broken.json"
        ));
        std::fs::remove_dir_all(&dir).expect("the scratch dir goes");
    }

    #[test]
    fn the_directory_loader_refuses_a_branch_outside_the_set() {
        let dir = scratch_dir("dir_bad_branch");
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        let text = GOOD_SHEET.replace(r#""branch": null"#, r#""branch": "warden_rogue""#);
        std::fs::write(dir.join("warden_fighter.json"), text).expect("the class file");
        let error = load_class_dir(&dir).expect_err("the branch target is gone");
        assert!(matches!(
            error,
            ClassSheetError::BadField { ref field, .. } if field == "branch"
        ));
        std::fs::remove_dir_all(&dir).expect("the scratch dir goes");
    }

    #[test]
    fn a_missing_file_is_refused_with_its_name() {
        let dir = scratch_dir("missing_file");
        let error = load_class_file(&dir.join("ghost.json")).expect_err("the file is gone");
        assert_eq!(
            error,
            ClassSheetError::Io {
                file: "ghost.json".to_string(),
                kind: std::io::ErrorKind::NotFound,
            }
        );
    }

    #[test]
    fn the_enum_words_round_trip() {
        for kind in [
            AbilityKind::Melee,
            AbilityKind::Projectile,
            AbilityKind::Aoe,
            AbilityKind::Buff,
            AbilityKind::Debuff,
            AbilityKind::Heal,
            AbilityKind::Control,
            AbilityKind::Stealth,
            AbilityKind::Utility,
        ] {
            assert_eq!(AbilityKind::from_text(kind.as_text()), Some(kind));
        }
        for shape in [
            AbilityShape::Single,
            AbilityShape::Cone,
            AbilityShape::Circle,
            AbilityShape::Line,
            AbilityShape::SelfTarget,
            AbilityShape::Target,
        ] {
            assert_eq!(AbilityShape::from_text(shape.as_text()), Some(shape));
        }
        for telegraph in [
            Telegraph::None,
            Telegraph::Circle,
            Telegraph::Cone,
            Telegraph::Line,
            Telegraph::Charge,
        ] {
            assert_eq!(Telegraph::from_text(telegraph.as_text()), Some(telegraph));
        }
        for stat in [Stat::Str, Stat::Dex, Stat::Con, Stat::Int, Stat::Wis] {
            assert_eq!(Stat::from_text(stat.as_text()), Some(stat));
        }
        assert_eq!(AbilityKind::from_text("beam"), None);
        assert_eq!(AbilityShape::from_text("blob"), None);
        assert_eq!(Telegraph::from_text("maybe"), None);
        assert_eq!(Stat::from_text("luck"), None);
    }

    /// A scratch directory under the system temp dir, one per test.
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("engine2040_class_sheet_{tag}"))
    }
}
