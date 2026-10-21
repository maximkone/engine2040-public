//! process: timed processes. A step eats inputs, uses a tool, takes work and
//! makes outputs; a recipe is a list of steps that run in order.
//!
//! The engine here is genre-neutral: a material is a name and an amount, a
//! tool is a name with the measurements a recipe gives it, and a step is
//! inputs, a tool, work, outputs and the rule that can fail it. The Roman
//! bloomery loop is data, not code: `content/rome/tech/iron.toml` holds the
//! recipe and every number in it names its source in `ref`.
//!
//! Units: amounts are whole pieces of a material, work is counted in the
//! step's own unit (`hour`, `stroke`, `beat`), and time is engine ticks. A
//! step that is time-skipped says so in its tick scale: one tick can stand for
//! one hour of game time.
//!
//! Determinism: integers only, no clock, no hash map. The same recipe, the
//! same stock, the same kit and the same work give the same outcome and the
//! same digest.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use engine_core::Fnv64;

/// The Roman bloomery recipe shipped in `content/rome/tech/iron.toml`.
pub const IRON_SHIPPED: &str = include_str!("../../../content/rome/tech/iron.toml");

/// A named amount of one material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part<'a> {
    /// The material's name, as the recipe writes it.
    pub name: &'a str,
    /// How much of it the step eats or makes.
    pub amount: u32,
}

impl<'a> Part<'a> {
    /// One part of a material.
    pub fn new(name: &'a str, amount: u32) -> Self {
        Self { name, amount }
    }
}

/// One measurement of a tool: what it measures and the range the recipe gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measure<'a> {
    /// What the measurement is, for example `height_mm`.
    pub name: &'a str,
    /// The least value the recipe gives.
    pub least: u32,
    /// The most value the recipe gives.
    pub most: u32,
}

/// The tool a step needs: its name and the measurements the recipe gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool<'a> {
    /// The tool's name, as the recipe writes it.
    pub name: &'a str,
    /// The measurements the recipe gives the tool, in file order.
    pub measures: Vec<Measure<'a>>,
}

impl<'a> Tool<'a> {
    /// A tool with no measurements.
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            measures: Vec::new(),
        }
    }

    /// The measurement the recipe gives the tool under `name`.
    pub fn measure(&self, name: &str) -> Option<Measure<'a>> {
        self.measures
            .iter()
            .find(|measure| measure.name == name)
            .copied()
    }
}

/// The rule that fails a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The step cannot fail by itself.
    None,
    /// The step fails when the work it gets falls short of what it needs.
    ShortWork,
}

/// The work one step takes: the unit it is counted in, how much of it the
/// recipe wants, and what one unit costs in engine ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work<'a> {
    /// The unit the work is counted in: `hour`, `stroke`, `beat`.
    pub unit: &'a str,
    /// The least work the step needs. Below it the step's rule fails.
    pub least: u32,
    /// The most work the recipe wants. More work is capped here.
    pub most: u32,
    /// The engine ticks one unit of work costs.
    pub ticks_per_unit: u32,
}

impl<'a> Work<'a> {
    /// The work a step takes.
    pub fn new(unit: &'a str, least: u32, most: u32, ticks_per_unit: u32) -> Self {
        Self {
            unit,
            least,
            most,
            ticks_per_unit,
        }
    }
}

/// One step of a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step<'a> {
    /// The step's name, as the recipe writes it.
    pub id: &'a str,
    /// The materials the step eats.
    pub inputs: Vec<Part<'a>>,
    /// The tool the step needs.
    pub tool: Tool<'a>,
    /// The work the step takes.
    pub work: Work<'a>,
    /// The materials the step makes.
    pub outputs: Vec<Part<'a>>,
    /// The rule that fails the step.
    pub rule: Rule,
}

/// A recipe: the steps of one process, in the order they run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe<'a> {
    /// The recipe's id, as the file writes it.
    pub id: &'a str,
    /// The steps, in order.
    pub steps: Vec<Step<'a>>,
}

/// The materials on hand, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stock<'a> {
    items: BTreeMap<&'a str, u32>,
}

impl<'a> Stock<'a> {
    /// An empty stock.
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
        }
    }

    /// A stock holding one material.
    pub fn with(name: &'a str, amount: u32) -> Self {
        let mut stock = Self::new();
        stock.add(name, amount);
        stock
    }

    /// How much of a material the stock holds.
    pub fn amount(&self, name: &str) -> u32 {
        self.items.get(name).copied().unwrap_or_default()
    }

    /// Put materials into the stock.
    pub fn add(&mut self, name: &'a str, amount: u32) {
        let held = self.items.entry(name).or_insert(0);
        *held = held.saturating_add(amount);
    }

    /// Take materials out of the stock. The stock never goes below zero and
    /// the amount taken is returned.
    pub fn take(&mut self, name: &str, amount: u32) -> u32 {
        let Some(held) = self.items.get_mut(name) else {
            return 0;
        };
        let taken = (*held).min(amount);
        *held -= taken;
        if *held == 0 {
            self.items.remove(name);
        }
        taken
    }

    /// Every material in the stock, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, u32)> + '_ {
        self.items.iter().map(|(name, amount)| (*name, *amount))
    }

    /// How many materials the stock holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the stock holds nothing.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The tools a worker carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kit<'a> {
    tools: BTreeSet<&'a str>,
}

impl<'a> Kit<'a> {
    /// An empty kit.
    pub fn new() -> Self {
        Self {
            tools: BTreeSet::new(),
        }
    }

    /// A kit holding one tool.
    pub fn with(name: &'a str) -> Self {
        let mut kit = Self::new();
        kit.tools.insert(name);
        kit
    }

    /// True when the kit holds the tool.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains(name)
    }

    /// Every tool in the kit, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.tools.iter().copied()
    }

    /// How many tools the kit holds.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// True when the kit holds nothing.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// One attempt at a recipe: the materials on hand, the tools carried, and the
/// work the worker puts into each step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attempt<'a> {
    /// The materials on hand.
    pub stock: Stock<'a>,
    /// The tools carried.
    pub kit: Kit<'a>,
    /// The work each step gets, by step name.
    pub work: BTreeMap<&'a str, u32>,
}

impl<'a> Attempt<'a> {
    /// An attempt with nothing on hand and no tools.
    pub fn new() -> Self {
        Self::default()
    }

    /// Put materials on hand.
    pub fn with_stock(mut self, name: &'a str, amount: u32) -> Self {
        self.stock.add(name, amount);
        self
    }

    /// Carry a tool.
    pub fn with_tool(mut self, name: &'a str) -> Self {
        self.kit.tools.insert(name);
        self
    }

    /// Give a step its work.
    pub fn with_work(mut self, step: &'a str, amount: u32) -> Self {
        self.work.insert(step, amount);
        self
    }
}

/// What one run of a recipe made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome<'a> {
    /// The recipe that ran.
    pub recipe: &'a str,
    /// What the steps made, in step order.
    pub made: Vec<Part<'a>>,
    /// The stock after the run.
    pub stock: Stock<'a>,
    /// The engine ticks the run took.
    pub ticks: u32,
    /// A digest of the run: the same run always digests the same.
    pub hash: u64,
}

impl Outcome<'_> {
    /// A digest of the whole run: the recipe, what it made, what is left and
    /// how long it took.
    fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write(b"engine-world/process/outcome/1");
        hash.write_str(self.recipe);
        for part in &self.made {
            hash.write_str(part.name);
            hash.write_u32(part.amount);
        }
        for (name, amount) in self.stock.iter() {
            hash.write_str(name);
            hash.write_u32(amount);
        }
        hash.write_u32(self.ticks);
        hash.finish()
    }
}

/// Why a run of a recipe stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessError<'a> {
    /// A step needs a material the stock does not hold. The name of the
    /// material is in the error.
    MissingInput {
        /// The step that asked for it.
        step: &'a str,
        /// The material the step needs.
        input: &'a str,
        /// How much of it the step needs.
        needed: u32,
        /// How much of it the stock holds.
        have: u32,
    },
    /// A step needs a tool the kit does not hold.
    MissingTool {
        /// The step that asked for it.
        step: &'a str,
        /// The tool the step needs.
        tool: &'a str,
    },
    /// A step failed by its rule: the work was short.
    ShortWork {
        /// The step that failed.
        step: &'a str,
        /// The unit the work is counted in.
        unit: &'a str,
        /// How much work the step needs.
        needed: u32,
        /// How much work the worker gave.
        given: u32,
    },
}

impl fmt::Display for ProcessError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput {
                step,
                input,
                needed,
                have,
            } => write!(
                f,
                "step `{step}` needs {needed} of `{input}`, the stock holds {have}"
            ),
            Self::MissingTool { step, tool } => {
                write!(f, "step `{step}` needs the tool `{tool}`")
            }
            Self::ShortWork {
                step,
                unit,
                needed,
                given,
            } => write!(
                f,
                "step `{step}` needs {needed} {unit}, the worker gave {given}"
            ),
        }
    }
}

impl std::error::Error for ProcessError<'_> {}

impl Recipe<'static> {
    /// Read the Roman bloomery recipe shipped in `content/rome/tech/iron.toml`.
    ///
    /// # Errors
    ///
    /// Same as [`Recipe::parse`].
    pub fn iron() -> Result<Recipe<'static>, RecipeError<'static>> {
        Self::parse(IRON_SHIPPED)
    }
}

impl<'a> Recipe<'a> {
    /// Read a recipe file.
    ///
    /// The file holds one `[recipe]` section, one `[tool.name]` section per
    /// tool with measurements, and one `[step.name]` section per step. A
    /// section that carries a number must also carry a `ref` naming its
    /// source: no number enters a recipe unsourced.
    ///
    /// # Errors
    ///
    /// Returns the first [`RecipeError`] the text earns: a malformed line, a
    /// repeated key or section, a section with a number and no `ref`, or a
    /// section that does not hold what a recipe needs.
    pub fn parse(text: &'a str) -> Result<Self, RecipeError<'a>> {
        let sections = read_sections(text)?;
        let header = find_section(&sections, "recipe")?;
        only(header, &["id"])?;
        let id = header.text("id").ok_or(RecipeError::MissingKey {
            entry: header.key,
            key: "id",
        })?;
        let tools = read_tools(&sections)?;
        let mut steps = Vec::new();
        for section in sections
            .iter()
            .filter(|section| section.key.starts_with("step."))
        {
            steps.push(read_step(section, &tools)?);
        }
        if steps.is_empty() {
            return Err(RecipeError::NoSteps);
        }
        Ok(Self { id, steps })
    }

    /// Run the recipe against an attempt.
    ///
    /// The steps run in the order the recipe writes them. Each step eats its
    /// inputs, needs its tool, takes its work and makes its outputs. The work
    /// the worker gives is capped at what the recipe wants, and a step whose
    /// rule fails stops the run.
    ///
    /// # Errors
    ///
    /// [`ProcessError::MissingInput`] when the stock does not hold a material
    /// a step needs, [`ProcessError::MissingTool`] when the kit lacks a tool,
    /// and [`ProcessError::ShortWork`] when a step's rule fails on short work.
    pub fn run(&self, attempt: &Attempt<'a>) -> Result<Outcome<'a>, ProcessError<'a>> {
        let mut stock = attempt.stock.clone();
        let mut made: Vec<Part<'a>> = Vec::new();
        let mut ticks: u32 = 0;
        for step in &self.steps {
            for input in &step.inputs {
                let have = stock.amount(input.name);
                if have < input.amount {
                    return Err(ProcessError::MissingInput {
                        step: step.id,
                        input: input.name,
                        needed: input.amount,
                        have,
                    });
                }
            }
            if !attempt.kit.has(step.tool.name) {
                return Err(ProcessError::MissingTool {
                    step: step.id,
                    tool: step.tool.name,
                });
            }
            let given = attempt.work.get(step.id).copied().unwrap_or_default();
            if step.rule == Rule::ShortWork && given < step.work.least {
                return Err(ProcessError::ShortWork {
                    step: step.id,
                    unit: step.work.unit,
                    needed: step.work.least,
                    given,
                });
            }
            let applied = given.min(step.work.most);
            for input in &step.inputs {
                stock.take(input.name, input.amount);
            }
            for output in &step.outputs {
                stock.add(output.name, output.amount);
                made.push(*output);
            }
            ticks = ticks.saturating_add(applied.saturating_mul(step.work.ticks_per_unit));
        }
        let outcome = Outcome {
            recipe: self.id,
            made,
            stock,
            ticks,
            hash: 0,
        };
        let hash = outcome.digest();
        Ok(Outcome { hash, ..outcome })
    }
}

/// Why a recipe file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeError<'a> {
    /// A line is not a section header, a `key = value` pair, a comment or a
    /// blank line.
    Syntax {
        /// The 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A section header is not `[recipe]`, `[tool.name]` or `[step.name]`.
    BadSection {
        /// The 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A section appears twice.
    DuplicateSection {
        /// The 1-based line number.
        line: usize,
        /// The section key.
        key: &'a str,
    },
    /// A key appears twice inside one section.
    DuplicateKey {
        /// The 1-based line number.
        line: usize,
        /// The section key.
        entry: &'a str,
        /// The repeated key.
        key: &'a str,
    },
    /// A value is neither a whole number nor a quoted string.
    NotAnInteger {
        /// The 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A section carries a number and no `ref` naming its source.
    MissingRef {
        /// The 1-based line number of the section header.
        line: usize,
        /// The section key.
        entry: &'a str,
    },
    /// A required section is missing.
    NoSection {
        /// The section key.
        entry: &'a str,
    },
    /// A required key is missing from a section that exists.
    MissingKey {
        /// The section key.
        entry: &'a str,
        /// The missing key.
        key: &'a str,
    },
    /// A section holds a key this reader does not read.
    UnknownKey {
        /// The section key.
        entry: &'a str,
        /// The unknown key.
        key: &'a str,
    },
    /// A `name amount` list is not written as this reader reads it.
    BadList {
        /// The section key.
        entry: &'a str,
        /// The list as written.
        text: &'a str,
    },
    /// A rule name is not one this reader reads.
    BadRule {
        /// The section key.
        entry: &'a str,
        /// The rule as written.
        text: &'a str,
    },
    /// A range is written backwards: the least value is above the most.
    BadRange {
        /// The section key.
        entry: &'a str,
        /// The key the range sits under.
        key: &'a str,
    },
    /// A number that must be above zero is zero.
    Zero {
        /// The section key.
        entry: &'a str,
        /// The key the zero sits under.
        key: &'a str,
    },
    /// The file holds no steps.
    NoSteps,
}

impl fmt::Display for RecipeError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => {
                write!(f, "line {line}: not a `key = value` line: {text}")
            }
            Self::BadSection { line, text } => {
                write!(
                    f,
                    "line {line}: not a `[recipe]`, `[tool.name]` or `[step.name]` header: {text}"
                )
            }
            Self::DuplicateSection { line, key } => {
                write!(f, "line {line}: section `[{key}]` appears twice")
            }
            Self::DuplicateKey { line, entry, key } => {
                write!(f, "line {line}: `[{entry}]` holds `{key}` twice")
            }
            Self::NotAnInteger { line, text } => {
                write!(
                    f,
                    "line {line}: not a whole number or a quoted string: {text}"
                )
            }
            Self::MissingRef { line, entry } => {
                write!(
                    f,
                    "line {line}: `[{entry}]` has numbers but no `ref` naming its source"
                )
            }
            Self::NoSection { entry } => write!(f, "the file has no `[{entry}]` section"),
            Self::MissingKey { entry, key } => write!(f, "`[{entry}]` has no `{key}`"),
            Self::UnknownKey { entry, key } => {
                write!(f, "`[{entry}]` has an unknown key `{key}`")
            }
            Self::BadList { entry, text } => {
                write!(f, "`[{entry}]` does not read the list `{text}`")
            }
            Self::BadRule { entry, text } => {
                write!(f, "`[{entry}]` has the unknown rule `{text}`")
            }
            Self::BadRange { entry, key } => {
                write!(f, "`[{entry}]` has `{key}` written backwards")
            }
            Self::Zero { entry, key } => write!(f, "`[{entry}]` {key} must be above zero"),
            Self::NoSteps => write!(f, "the file holds no steps"),
        }
    }
}

impl std::error::Error for RecipeError<'_> {}

/// One value written in a recipe file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value<'a> {
    /// A whole number exactly as written.
    Int(u64),
    /// A quoted string without its quotes.
    Text(&'a str),
}

/// One section of a recipe file: its key, the line it starts on, its values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Section<'a> {
    key: &'a str,
    line: usize,
    values: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Section<'a> {
    /// The whole number written under `key`.
    fn int(&self, key: &str) -> Option<u64> {
        self.values
            .iter()
            .filter(|(name, _)| *name == key)
            .find_map(|(_, value)| match value {
                Value::Int(number) => Some(*number),
                Value::Text(_) => None,
            })
    }

    /// The string written under `key`, without its quotes.
    fn text(&self, key: &str) -> Option<&'a str> {
        self.values
            .iter()
            .filter(|(name, _)| *name == key)
            .find_map(|(_, value)| match value {
                Value::Text(text) => Some(*text),
                Value::Int(_) => None,
            })
    }

    /// True when the section holds a number.
    fn holds_number(&self) -> bool {
        self.values
            .iter()
            .any(|(_, value)| matches!(value, Value::Int(_)))
    }

    /// True when the section names its source.
    fn holds_ref(&self) -> bool {
        self.text("ref").is_some_and(|text| !text.trim().is_empty())
    }
}

/// Read every section of a recipe file.
fn read_sections(text: &str) -> Result<Vec<Section<'_>>, RecipeError<'_>> {
    let mut sections: Vec<Section<'_>> = Vec::new();
    let mut open: Option<usize> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if body.starts_with('[') {
            let key = header_key(body, line)?;
            if sections.iter().any(|section| section.key == key) {
                return Err(RecipeError::DuplicateSection { line, key });
            }
            sections.push(Section {
                key,
                line,
                values: Vec::new(),
            });
            open = Some(sections.len() - 1);
            continue;
        }
        let Some(open) = open else {
            return Err(RecipeError::Syntax { line, text: body });
        };
        let (key, value) = key_value(body, line)?;
        let section = &mut sections[open];
        if section.values.iter().any(|(name, _)| *name == key) {
            return Err(RecipeError::DuplicateKey {
                line,
                entry: section.key,
                key,
            });
        }
        section.values.push((key, value));
    }
    for section in &sections {
        if section.holds_number() && !section.holds_ref() {
            return Err(RecipeError::MissingRef {
                line: section.line,
                entry: section.key,
            });
        }
    }
    Ok(sections)
}

/// The section written under `key`.
fn find_section<'a, 's>(
    sections: &'s [Section<'a>],
    key: &'a str,
) -> Result<&'s Section<'a>, RecipeError<'a>> {
    sections
        .iter()
        .find(|section| section.key == key)
        .ok_or(RecipeError::NoSection { entry: key })
}

/// Read the tools of a recipe file, with the measurements they carry.
fn read_tools<'a>(
    sections: &[Section<'a>],
) -> Result<BTreeMap<&'a str, Tool<'a>>, RecipeError<'a>> {
    let mut tools: BTreeMap<&'a str, Tool<'a>> = BTreeMap::new();
    for section in sections
        .iter()
        .filter(|section| section.key.starts_with("tool."))
    {
        let name = &section.key["tool.".len()..];
        let mut measures = Vec::new();
        for (key, _) in &section.values {
            if *key == "ref" || key.ends_with("_least") || key.ends_with("_most") {
                continue;
            }
            return Err(RecipeError::UnknownKey {
                entry: section.key,
                key,
            });
        }
        for (key, _) in &section.values {
            let Some(measure) = key.strip_suffix("_least") else {
                continue;
            };
            let least =
                measure_value(section, measure, "_least").ok_or(RecipeError::MissingKey {
                    entry: section.key,
                    key,
                })?;
            let most = measure_value(section, measure, "_most").ok_or(RecipeError::MissingKey {
                entry: section.key,
                key,
            })?;
            if most < least {
                return Err(RecipeError::BadRange {
                    entry: section.key,
                    key: measure,
                });
            }
            measures.push(Measure {
                name: measure,
                least,
                most,
            });
        }
        tools.insert(name, Tool { name, measures });
    }
    Ok(tools)
}

/// The number a tool section gives under the key that measures `name` and
/// ends with `suffix`.
fn measure_value(section: &Section<'_>, name: &str, suffix: &str) -> Option<u32> {
    section.values.iter().find_map(|(key, value)| {
        let measured = key.strip_suffix(suffix)?;
        if measured != name {
            return None;
        }
        match value {
            Value::Int(number) => u32::try_from(*number).ok(),
            Value::Text(_) => None,
        }
    })
}

/// Read one step of a recipe file.
fn read_step<'a>(
    section: &Section<'a>,
    tools: &BTreeMap<&'a str, Tool<'a>>,
) -> Result<Step<'a>, RecipeError<'a>> {
    only(
        section,
        &[
            "tool",
            "inputs",
            "outputs",
            "unit",
            "least",
            "most",
            "ticks_per_unit",
            "rule",
        ],
    )?;
    let id = &section.key["step.".len()..];
    let tool_name = text(section, "tool")?;
    let tool = tools
        .get(tool_name)
        .cloned()
        .unwrap_or_else(|| Tool::new(tool_name));
    let inputs = parts(section, "inputs")?;
    let outputs = parts(section, "outputs")?;
    let unit = text(section, "unit")?;
    let least = number(section, "least")?;
    let most = number(section, "most")?;
    if most < least {
        return Err(RecipeError::BadRange {
            entry: section.key,
            key: "most",
        });
    }
    let ticks_per_unit = number(section, "ticks_per_unit")?;
    if ticks_per_unit == 0 {
        return Err(RecipeError::Zero {
            entry: section.key,
            key: "ticks_per_unit",
        });
    }
    let rule = rule(section)?;
    Ok(Step {
        id,
        inputs,
        tool,
        work: Work {
            unit,
            least,
            most,
            ticks_per_unit,
        },
        outputs,
        rule,
    })
}

/// The rule a step fails by.
fn rule<'a>(section: &Section<'a>) -> Result<Rule, RecipeError<'a>> {
    let name = text(section, "rule")?;
    match name {
        "none" => Ok(Rule::None),
        "short_work" => Ok(Rule::ShortWork),
        _ => Err(RecipeError::BadRule {
            entry: section.key,
            text: name,
        }),
    }
}

/// The `name amount` list written under `key`.
fn parts<'a>(section: &Section<'a>, key: &'a str) -> Result<Vec<Part<'a>>, RecipeError<'a>> {
    let Some(list) = section.text(key) else {
        return Err(RecipeError::MissingKey {
            entry: section.key,
            key,
        });
    };
    let words: Vec<&str> = list.split(' ').collect();
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return Err(RecipeError::BadList {
            entry: section.key,
            text: list,
        });
    }
    let mut parts = Vec::new();
    for pair in words.chunks(2) {
        let name = pair[0];
        if !is_identifier(name) {
            return Err(RecipeError::BadList {
                entry: section.key,
                text: list,
            });
        }
        let Some(amount) = parse_count(pair[1]) else {
            return Err(RecipeError::BadList {
                entry: section.key,
                text: list,
            });
        };
        let Some(amount) = u32::try_from(amount).ok() else {
            return Err(RecipeError::BadList {
                entry: section.key,
                text: list,
            });
        };
        if amount == 0 {
            return Err(RecipeError::BadList {
                entry: section.key,
                text: list,
            });
        }
        parts.push(Part { name, amount });
    }
    Ok(parts)
}

/// One whole number written under `key`.
fn number<'a>(section: &Section<'a>, key: &'a str) -> Result<u32, RecipeError<'a>> {
    let value = section.int(key).ok_or(RecipeError::MissingKey {
        entry: section.key,
        key,
    })?;
    u32::try_from(value).map_err(|_| RecipeError::BadRange {
        entry: section.key,
        key,
    })
}

/// One string written under `key`.
fn text<'a>(section: &Section<'a>, key: &'a str) -> Result<&'a str, RecipeError<'a>> {
    section.text(key).ok_or(RecipeError::MissingKey {
        entry: section.key,
        key,
    })
}

/// Refuse a key the reader does not read; `ref` is always allowed.
fn only<'a>(section: &Section<'a>, allowed: &[&str]) -> Result<(), RecipeError<'a>> {
    for (key, _) in &section.values {
        if *key == "ref" || allowed.contains(key) {
            continue;
        }
        return Err(RecipeError::UnknownKey {
            entry: section.key,
            key,
        });
    }
    Ok(())
}

/// The key of a section header.
fn header_key(body: &str, line: usize) -> Result<&str, RecipeError<'_>> {
    let Some(inner) = body
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Err(RecipeError::BadSection { line, text: body });
    };
    let valid = match inner.split_once('.') {
        Some((head, tail)) => (head == "tool" || head == "step") && is_identifier(tail),
        None => inner == "recipe",
    };
    if !valid {
        return Err(RecipeError::BadSection { line, text: body });
    }
    Ok(inner)
}

/// One `key = value` line, split into its key and its typed value.
fn key_value<'a>(body: &'a str, line: usize) -> Result<(&'a str, Value<'a>), RecipeError<'a>> {
    let Some((key, value)) = body.split_once('=') else {
        return Err(RecipeError::Syntax { line, text: body });
    };
    let key = key.trim();
    if !is_identifier(key) {
        return Err(RecipeError::Syntax { line, text: body });
    }
    Ok((key, read_value(value.trim(), body, line)?))
}

/// A value: a bare whole number, or a double-quoted string with no escapes.
fn read_value<'a>(text: &'a str, body: &'a str, line: usize) -> Result<Value<'a>, RecipeError<'a>> {
    if let Some(inner) = text.strip_prefix('"') {
        let Some(inner) = inner.strip_suffix('"') else {
            return Err(RecipeError::Syntax { line, text: body });
        };
        if inner.contains('"') || inner.contains('\\') {
            return Err(RecipeError::Syntax { line, text: body });
        }
        return Ok(Value::Text(inner));
    }
    let Some(number) = parse_count(text) else {
        return Err(RecipeError::NotAnInteger { line, text: body });
    };
    Ok(Value::Int(number))
}

/// A whole decimal number with no sign, separator or decimal point.
fn parse_count(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 19 {
        return None;
    }
    for character in text.chars() {
        if !character.is_ascii_digit() {
            return None;
        }
    }
    text.parse().ok()
}

/// A lowercase identifier: a letter or `_`, then letters, digits and `_`.
fn is_identifier(text: &str) -> bool {
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let starts_well = first.is_ascii_lowercase() || first == '_';
    let rest_well = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    });
    starts_well && rest_well
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped Roman bloomery recipe.
    fn iron() -> Recipe<'static> {
        Recipe::iron().expect("the shipped iron recipe reads")
    }

    /// A smith at the bloomery: ore and charcoal, the tools, and the work.
    fn smith() -> Attempt<'static> {
        Attempt::new()
            .with_stock("ore", 1)
            .with_stock("charcoal", 1)
            .with_tool("shaft_furnace")
            .with_tool("tongs")
            .with_tool("hearth")
            .with_tool("anvil")
            .with_work("smelt", 14)
            .with_work("break_out", 1)
            .with_work("reheat", 1)
            .with_work("forge", 8)
    }

    /// A one-step recipe that dries clay: the generic engine on its own.
    fn brick() -> Recipe<'static> {
        Recipe {
            id: "brick",
            steps: vec![Step {
                id: "dry",
                inputs: vec![Part::new("clay", 1)],
                tool: Tool::new("kiln"),
                work: Work::new("hour", 2, 4, 1),
                outputs: vec![Part::new("brick", 1)],
                rule: Rule::ShortWork,
            }],
        }
    }

    #[test]
    fn the_iron_recipe_makes_a_bloom_and_a_bar() {
        let outcome = iron().run(&smith()).expect("the bloomery loop runs");
        assert_eq!(outcome.stock.amount("bar"), 1, "the bar is forged");
        assert_eq!(
            outcome.stock.amount("bloom"),
            0,
            "the bloom goes into the bar"
        );
        assert_eq!(outcome.stock.amount("ore"), 0, "the ore is spent");
        assert_eq!(outcome.stock.amount("charcoal"), 0, "the charcoal is spent");
        assert!(
            outcome.made.iter().any(|part| part.name == "bloom"),
            "the bloom is made: {:?}",
            outcome.made
        );
        assert!(
            outcome.made.iter().any(|part| part.name == "bar"),
            "the bar is made: {:?}",
            outcome.made
        );
    }

    #[test]
    fn a_missing_input_is_refused_with_its_name() {
        let attempt = Attempt::new()
            .with_stock("ore", 1)
            .with_tool("shaft_furnace")
            .with_work("smelt", 14);
        let error = iron().run(&attempt).expect_err("no charcoal to burn");
        assert_eq!(
            error,
            ProcessError::MissingInput {
                step: "smelt",
                input: "charcoal",
                needed: 1,
                have: 0,
            }
        );
        assert!(error.to_string().contains("charcoal"), "{error}");
    }

    #[test]
    fn the_same_beats_give_the_same_bar_twice() {
        let recipe = iron();
        let first = recipe.run(&smith()).expect("the first run");
        let second = recipe.run(&smith()).expect("the second run");
        assert_eq!(first, second, "the same beats give the same outcome");
        assert_eq!(first.hash, second.hash, "and the same digest");
        assert_ne!(first.hash, 0, "the digest is not empty");
    }

    #[test]
    fn the_iron_recipe_reads_the_sourced_numbers() {
        let recipe = iron();
        let smelt = recipe
            .steps
            .iter()
            .find(|step| step.id == "smelt")
            .expect("the smelt step");
        assert_eq!(smelt.work.unit, "hour");
        assert_eq!(
            (smelt.work.least, smelt.work.most),
            (10, 20),
            "10 to 20 h of game time"
        );
        assert_eq!(smelt.work.ticks_per_unit, 1, "the smelt is time-skipped");
        let forge = recipe
            .steps
            .iter()
            .find(|step| step.id == "forge")
            .expect("the forge step");
        assert_eq!(forge.work.unit, "beat");
        assert_eq!(
            (forge.work.least, forge.work.most),
            (6, 10),
            "6 to 10 anvil beats"
        );
        let furnace = smelt
            .tool
            .measure("height_mm")
            .expect("the shaft furnace has a height");
        assert_eq!((furnace.least, furnace.most), (1200, 1500), "1.2 to 1.5 m");
    }

    #[test]
    fn the_skipped_smelting_and_the_forged_bar_take_their_own_time() {
        let outcome = iron().run(&smith()).expect("the bloomery loop runs");
        assert_eq!(
            outcome.ticks,
            14 + 1 + 1 + 8 * 60,
            "the smelt is skipped, the forge is not"
        );
    }

    #[test]
    fn a_step_fails_by_its_rule() {
        let short = Attempt::new()
            .with_stock("clay", 1)
            .with_tool("kiln")
            .with_work("dry", 1);
        let error = brick().run(&short).expect_err("one hour is not enough");
        assert_eq!(
            error,
            ProcessError::ShortWork {
                step: "dry",
                unit: "hour",
                needed: 2,
                given: 1,
            }
        );
        let enough = Attempt::new()
            .with_stock("clay", 1)
            .with_tool("kiln")
            .with_work("dry", 3);
        let outcome = brick().run(&enough).expect("three hours dry the clay");
        assert_eq!(outcome.stock.amount("brick"), 1, "the clay is a brick");
        assert_eq!(outcome.ticks, 3, "one tick an hour");
    }

    #[test]
    fn a_step_without_its_tool_is_refused() {
        let attempt = Attempt::new().with_stock("clay", 1).with_work("dry", 3);
        let error = brick()
            .run(&attempt)
            .expect_err("no kiln to dry the clay in");
        assert_eq!(
            error,
            ProcessError::MissingTool {
                step: "dry",
                tool: "kiln",
            }
        );
    }

    #[test]
    fn a_section_with_numbers_and_no_ref_is_refused() {
        let text = "[recipe]\nid = \"brick\"\n\n[step.dry]\nunit = \"hour\"\nleast = 2\n";
        let error = Recipe::parse(text).expect_err("the step has no source");
        assert_eq!(
            error,
            RecipeError::MissingRef {
                line: 4,
                entry: "step.dry",
            }
        );
        assert!(error.to_string().contains("step.dry"), "{error}");
    }

    #[test]
    fn a_recipe_without_steps_is_refused() {
        let text = "[recipe]\nid = \"brick\"\n";
        let error = Recipe::parse(text).expect_err("no steps to run");
        assert_eq!(error, RecipeError::NoSteps);
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = "[recipe]\nid = \"brick\"\n\n[step.dry]\nunit = \"hour\"\nleast = 2\nmost = 4\nticks_per_unit = 1\nrule = \"none\"\ninputs = \"clay 1\"\noutputs = \"brick 1\"\ntool = \"kiln\"\ndept = 1\nref = \"rome.md section 4.4\"\n";
        let error = Recipe::parse(text).expect_err("a typo in a key");
        assert!(error.to_string().contains("dept"), "{error}");
    }
}
