//! mtlx_import: MaterialX `standard_surface` and `open_pbr_surface` constants and images to
//! records (NS31f2).
//!
//! Reads a `.mtlx` text with a small XML reader of its own (elements, attributes, self-closing
//! tags, comments, the `<?xml?>` line; no new dependency). Each `<standard_surface>` or
//! `<open_pbr_surface>` node under `<materialx>` becomes one record. An input is a constant
//! (`value=`) or a link (`nodename=`) to an `<image>` node, and only `base_color` and
//! `emission_color` take an image (the record's base-color and emissive slots).
//!
//! What the record cannot hold is named, never dropped silently, in the same receipt as the glTF
//! importer ([`Import`], [`Warning`]): an input the mapping does not know is
//! [`WarningKind::Unsupported`]; a value out of range is [`WarningKind::Clamped`]. A procedural
//! graph is refused whole with [`MtlxError::UnsupportedNode`]: a `<nodegraph>`, or an input linked
//! to any node that is not an `<image>`.
//!
//! Mapped inputs (standard_surface name, then OpenPBR name where it differs): `base`
//! (`base_weight`, scales the color), `base_color`, `metalness` (`base_metalness`),
//! `specular_roughness`, `specular_IOR` (`specular_ior`), `coat` (`coat_weight`),
//! `coat_roughness`, `coat_IOR` (`coat_ior`), `sheen` (`fuzz_weight`), `sheen_color`
//! (`fuzz_color`), `sheen_roughness` (`fuzz_roughness`), `emission` (`emission_luminance`, nits),
//! `emission_color`, `transmission` (`transmission_weight`). A missing input has the spec default
//! of its node family. Units: linear colors, nits, meters; no clock, no IO.

use crate::gltf_import::{plain_record, record_name, texture_ref_of, Ctx, Import, WarningKind};
use crate::record::{Coat, Emission, Look, Sheen, Transmission, MAX_IOR, MAX_LUMINANCE, MIN_IOR};

/// Largest `.mtlx` text read: 1 MiB.
pub const MAX_MTLX_BYTES: usize = 1 << 20;
/// Deepest element nesting read.
const MAX_DEPTH: usize = 16;
/// Most elements read from one file.
const MAX_ELEMENTS: usize = 4096;

/// Why a file was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MtlxError {
    /// The text is longer than [`MAX_MTLX_BYTES`].
    TooLarge,
    /// The XML is malformed: byte offset and what is wrong.
    Xml {
        /// Byte offset of the problem.
        at: usize,
        /// What is wrong.
        why: &'static str,
    },
    /// The file has no `<standard_surface>` or `<open_pbr_surface>` node.
    NoSurface,
    /// A procedural graph: the name of the `<nodegraph>` or of the linked node.
    UnsupportedNode(String),
    /// A constant that is not a number (or three numbers for a color).
    BadValue {
        /// The input name.
        input: String,
        /// The text found.
        text: String,
    },
}

impl std::fmt::Display for MtlxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "mtlx text over {MAX_MTLX_BYTES} bytes"),
            Self::Xml { at, why } => write!(f, "mtlx xml at byte {at}: {why}"),
            Self::NoSurface => f.write_str("no standard_surface or open_pbr_surface node"),
            Self::UnsupportedNode(name) => {
                write!(
                    f,
                    "procedural node `{name}` is not supported (constants and images only)"
                )
            }
            Self::BadValue { input, text } => {
                write!(f, "input `{input}` has no number in `{text}`")
            }
        }
    }
}

impl std::error::Error for MtlxError {}

/// One XML element: tag, attributes in file order, child elements. Text is not kept.
#[derive(Debug)]
struct Element {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
}

impl Element {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn name(&self) -> &str {
        self.attr("name").unwrap_or(&self.tag)
    }
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
    elements: usize,
}

impl<'a> Reader<'a> {
    fn err(&self, why: &'static str) -> MtlxError {
        MtlxError::Xml { at: self.at, why }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn skip_space(&mut self) {
        self.at += self.rest().len() - self.rest().trim_start().len();
    }

    /// Skip past `end`, which must occur.
    fn skip_past(&mut self, end: &str, why: &'static str) -> Result<(), MtlxError> {
        let found = self.rest().find(end).ok_or_else(|| self.err(why))?;
        self.at += found + end.len();
        Ok(())
    }

    /// Skip one comment, `<?...?>`, CDATA or `<!...>` (doctype) at the cursor; false when none.
    fn skip_one_misc(&mut self) -> Result<bool, MtlxError> {
        let rest = self.rest();
        if rest.starts_with("<!--") {
            self.skip_past("-->", "comment is not closed")?;
        } else if rest.starts_with("<?") {
            self.skip_past("?>", "processing instruction is not closed")?;
        } else if rest.starts_with("<![CDATA[") {
            self.skip_past("]]>", "CDATA is not closed")?;
        } else if rest.starts_with("<!") {
            self.skip_past(">", "declaration is not closed")?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// Skip whitespace and every comment, instruction and declaration.
    fn skip_misc(&mut self) -> Result<(), MtlxError> {
        loop {
            self.skip_space();
            if !self.skip_one_misc()? {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, MtlxError> {
        let rest = self.rest();
        let len = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '=' | '/' | '>' | '<' | '"' | '\''))
            .unwrap_or(rest.len());
        if len == 0 {
            return Err(self.err("a name is expected"));
        }
        self.at += len;
        Ok(rest[..len].to_string())
    }

    fn quoted(&mut self) -> Result<String, MtlxError> {
        let quote = match self.rest().chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => return Err(self.err("an attribute value must be quoted")),
        };
        self.at += 1;
        let len = self
            .rest()
            .find(quote)
            .ok_or_else(|| self.err("attribute value is not closed"))?;
        let raw = &self.rest()[..len];
        self.at += len + 1;
        Ok(decode_entities(raw))
    }

    fn element(&mut self, depth: usize) -> Result<Element, MtlxError> {
        if depth > MAX_DEPTH {
            return Err(self.err("elements nest too deep"));
        }
        self.elements += 1;
        if self.elements > MAX_ELEMENTS {
            return Err(self.err("too many elements"));
        }
        if !self.rest().starts_with('<') {
            return Err(self.err("an element is expected"));
        }
        self.at += 1;
        let tag = self.name()?;
        let mut attrs = Vec::new();
        loop {
            self.skip_space();
            if self.rest().starts_with("/>") {
                self.at += 2;
                return Ok(Element {
                    tag,
                    attrs,
                    children: Vec::new(),
                });
            }
            if self.rest().starts_with('>') {
                self.at += 1;
                break;
            }
            let key = self.name()?;
            self.skip_space();
            if !self.rest().starts_with('=') {
                return Err(self.err("an attribute needs `=`"));
            }
            self.at += 1;
            self.skip_space();
            let value = self.quoted()?;
            attrs.push((key, value));
        }
        let mut children = Vec::new();
        loop {
            // Text between tags carries no meaning here.
            let text_len = self
                .rest()
                .find('<')
                .ok_or_else(|| self.err("element is not closed"))?;
            self.at += text_len;
            if self.skip_one_misc()? {
                continue;
            }
            if self.rest().starts_with("</") {
                self.at += 2;
                let close = self.name()?;
                self.skip_space();
                if close != tag || !self.rest().starts_with('>') {
                    return Err(self.err("closing tag does not match"));
                }
                self.at += 1;
                return Ok(Element {
                    tag,
                    attrs,
                    children,
                });
            }
            children.push(self.element(depth + 1)?);
        }
    }
}

/// The five predefined XML entities; anything else stays as written.
fn decode_entities(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_document(text: &str) -> Result<Element, MtlxError> {
    if text.len() > MAX_MTLX_BYTES {
        return Err(MtlxError::TooLarge);
    }
    let mut reader = Reader {
        text,
        at: 0,
        elements: 0,
    };
    reader.skip_misc()?;
    let root = reader.element(0)?;
    reader.skip_misc()?;
    if !reader.rest().is_empty() {
        return Err(reader.err("text after the root element"));
    }
    if root.tag != "materialx" {
        return Err(MtlxError::Xml {
            at: 0,
            why: "the root element is not <materialx>",
        });
    }
    Ok(root)
}

/// The inputs this importer maps, shared by both node families. The order is the order of
/// [`INPUTS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    Base,
    BaseColor,
    Metalness,
    Roughness,
    Ior,
    Coat,
    CoatRoughness,
    CoatIor,
    Sheen,
    SheenColor,
    SheenRoughness,
    Emission,
    EmissionColor,
    Transmission,
}

impl Key {
    /// Position in [`INPUTS`].
    const fn index(self) -> usize {
        match self {
            Self::Base => 0,
            Self::BaseColor => 1,
            Self::Metalness => 2,
            Self::Roughness => 3,
            Self::Ior => 4,
            Self::Coat => 5,
            Self::CoatRoughness => 6,
            Self::CoatIor => 7,
            Self::Sheen => 8,
            Self::SheenColor => 9,
            Self::SheenRoughness => 10,
            Self::Emission => 11,
            Self::EmissionColor => 12,
            Self::Transmission => 13,
        }
    }
}

/// One mapped input: its name in `standard_surface` and in `open_pbr_surface`, the key, and the
/// spec default in each family (one number; a color default is grey).
struct Input {
    standard: &'static str,
    open_pbr: &'static str,
    key: Key,
    standard_default: f32,
    open_pbr_default: f32,
}

const fn input(
    standard: &'static str,
    open_pbr: &'static str,
    key: Key,
    standard_default: f32,
    open_pbr_default: f32,
) -> Input {
    Input {
        standard,
        open_pbr,
        key,
        standard_default,
        open_pbr_default,
    }
}

const INPUTS: [Input; 14] = [
    input("base", "base_weight", Key::Base, 0.8, 1.0),
    input("base_color", "base_color", Key::BaseColor, 0.8, 0.8),
    input("metalness", "base_metalness", Key::Metalness, 0.0, 0.0),
    input(
        "specular_roughness",
        "specular_roughness",
        Key::Roughness,
        0.2,
        0.3,
    ),
    input("specular_IOR", "specular_ior", Key::Ior, 1.5, 1.5),
    input("coat", "coat_weight", Key::Coat, 0.0, 0.0),
    input(
        "coat_roughness",
        "coat_roughness",
        Key::CoatRoughness,
        0.1,
        0.0,
    ),
    input("coat_IOR", "coat_ior", Key::CoatIor, 1.5, 1.6),
    input("sheen", "fuzz_weight", Key::Sheen, 0.0, 0.0),
    input("sheen_color", "fuzz_color", Key::SheenColor, 1.0, 1.0),
    input(
        "sheen_roughness",
        "fuzz_roughness",
        Key::SheenRoughness,
        0.3,
        0.5,
    ),
    input("emission", "emission_luminance", Key::Emission, 0.0, 0.0),
    input(
        "emission_color",
        "emission_color",
        Key::EmissionColor,
        1.0,
        1.0,
    ),
    input(
        "transmission",
        "transmission_weight",
        Key::Transmission,
        0.0,
        0.0,
    ),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Standard,
    OpenPbr,
}

impl Family {
    fn of(tag: &str) -> Option<Self> {
        match tag {
            "standard_surface" => Some(Self::Standard),
            "open_pbr_surface" => Some(Self::OpenPbr),
            _ => None,
        }
    }

    /// The mapped key of an input name in this family.
    fn key(self, name: &str) -> Option<Key> {
        INPUTS
            .iter()
            .find(|i| name == self.name_of(i))
            .map(|i| i.key)
    }

    fn name_of(self, input: &Input) -> &'static str {
        match self {
            Self::Standard => input.standard,
            Self::OpenPbr => input.open_pbr,
        }
    }

    /// The spec default of `key` in this family: one number for a float, grey for a color.
    fn fallback(self, key: Key) -> [f32; 3] {
        let entry = &INPUTS[key.index()];
        let v = match self {
            Self::Standard => entry.standard_default,
            Self::OpenPbr => entry.open_pbr_default,
        };
        [v; 3]
    }
}

/// A float or color3 constant: one number, or three separated by commas.
fn parse_value(input: &str, text: &str) -> Result<[f32; 3], MtlxError> {
    let bad = || MtlxError::BadValue {
        input: input.to_string(),
        text: text.to_string(),
    };
    let numbers: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()
        .ok_or_else(bad)?;
    match numbers[..] {
        [v] => Ok([v; 3]),
        [r, g, b] => Ok([r, g, b]),
        _ => Err(bad()),
    }
}

/// The `tex:` reference of an `<image>` node's `file` input.
fn image_ref(image: &Element) -> Option<String> {
    let file = image
        .children
        .iter()
        .find(|c| c.tag == "input" && c.attr("name") == Some("file"))?
        .attr("value")?;
    let last = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
    Some(texture_ref_of(stem))
}

/// The constants one surface node gave, by key; the family default stands for the rest.
struct Values {
    family: Family,
    given: [Option<[f32; 3]>; INPUTS.len()],
}

impl Values {
    fn color(&self, key: Key) -> [f32; 3] {
        self.given[key.index()].unwrap_or_else(|| self.family.fallback(key))
    }

    fn float(&self, key: Key) -> f32 {
        self.color(key)[0]
    }
}

/// Read the inputs of one surface node: constants into the returned values, linked images into
/// the look's texture slots, unknown inputs into the warnings.
fn read_inputs(
    family: Family,
    node: &Element,
    root: &Element,
    cx: &mut Ctx<'_>,
    look: &mut Look,
) -> Result<Values, MtlxError> {
    let mut values = Values {
        family,
        given: [None; INPUTS.len()],
    };
    for input in node.children.iter().filter(|c| c.tag == "input") {
        let input_name = input.attr("name").unwrap_or("");
        let key = family.key(input_name);
        if let Some(target) = input.attr("nodename").or_else(|| input.attr("nodegraph")) {
            let linked = root
                .children
                .iter()
                .find(|c| c.attr("name") == Some(target));
            let Some(image) = linked.filter(|n| n.tag == "image") else {
                return Err(MtlxError::UnsupportedNode(target.to_string()));
            };
            match (key, image_ref(image)) {
                (Some(Key::BaseColor), Some(tex)) => look.textures.base_color = Some(tex),
                (Some(Key::EmissionColor), Some(tex)) => look.textures.emissive = Some(tex),
                _ => cx.warn(input_name, WarningKind::Unsupported),
            }
            continue;
        }
        let Some(key) = key else {
            cx.warn(input_name, WarningKind::Unsupported);
            continue;
        };
        if let Some(text) = input.attr("value") {
            values.given[key.index()] = Some(parse_value(input_name, text)?);
        }
    }
    Ok(values)
}

/// The base layer of the look: color (or the image factor), metalness, roughness, ior.
fn base_look(values: &Values, cx: &mut Ctx<'_>, look: &mut Look) {
    let weight = cx.unit("base", values.float(Key::Base));
    // A linked image supplies the color itself: the record's color is then only the factor.
    let tint = if look.textures.base_color.is_some() {
        [1.0; 3]
    } else {
        cx.color("base_color", values.color(Key::BaseColor))
    };
    look.base_color = tint.map(|c| c * weight);
    look.base_metalness = cx.unit("metalness", values.float(Key::Metalness));
    look.specular_roughness = cx.unit("specular_roughness", values.float(Key::Roughness));
    look.specular_ior = clamp_ior(cx, "specular_IOR", values.float(Key::Ior));
}

fn clamp_ior(cx: &mut Ctx<'_>, field: &str, ior: f32) -> f32 {
    let clamped = ior.clamp(MIN_IOR, MAX_IOR);
    if (clamped - ior).abs() > f32::EPSILON {
        cx.warn(field, WarningKind::Clamped);
    }
    clamped
}

/// The optional lobes: each exists only when its weight is above zero.
fn lobes(values: &Values, cx: &mut Ctx<'_>, look: &mut Look) {
    let coat = cx.unit("coat", values.float(Key::Coat));
    if coat > 0.0 {
        look.coat = Some(Coat {
            weight: coat,
            roughness: cx.unit("coat_roughness", values.float(Key::CoatRoughness)),
            ior: clamp_ior(cx, "coat_IOR", values.float(Key::CoatIor)),
        });
    }
    let sheen = cx.unit("sheen", values.float(Key::Sheen));
    if sheen > 0.0 {
        look.sheen = Some(Sheen {
            weight: sheen,
            color: cx.color("sheen_color", values.color(Key::SheenColor)),
            roughness: cx.unit("sheen_roughness", values.float(Key::SheenRoughness)),
        });
    }
    let nits = values.float(Key::Emission);
    if nits > 0.0 {
        let luminance = nits.min(MAX_LUMINANCE);
        if luminance < nits {
            cx.warn("emission", WarningKind::Clamped);
        }
        let color = if look.textures.emissive.is_some() {
            [1.0; 3]
        } else {
            cx.color("emission_color", values.color(Key::EmissionColor))
        };
        look.emission = Some(Emission { color, luminance });
    }
    let transmission = cx.unit("transmission", values.float(Key::Transmission));
    if transmission > 0.0 {
        look.transmission = Some(Transmission {
            weight: transmission,
            thickness: 0.0,
            attenuation_color: [1.0; 3],
        });
    }
}

fn import_surface(
    family: Family,
    node: &Element,
    root: &Element,
    index: usize,
    out: &mut Import,
) -> Result<(), MtlxError> {
    let name = record_name(Some(node.name()), index, &out.records);
    let mut cx = Ctx {
        material: &name,
        warnings: &mut out.warnings,
    };
    let mut look = Look::default();
    let values = read_inputs(family, node, root, &mut cx, &mut look)?;
    base_look(&values, &mut cx, &mut look);
    lobes(&values, &mut cx, &mut look);
    out.records.push(plain_record(name, look));
    Ok(())
}

/// Import every surface node of a `.mtlx` text, in file order.
///
/// # Errors
///
/// [`MtlxError`]: malformed XML, no surface node, a bad constant, or a procedural graph.
pub fn import_mtlx(text: &str) -> Result<Import, MtlxError> {
    let root = parse_document(text)?;
    if let Some(graph) = root.children.iter().find(|c| c.tag == "nodegraph") {
        return Err(MtlxError::UnsupportedNode(graph.name().to_string()));
    }
    let mut out = Import::default();
    for node in &root.children {
        if let Some(family) = Family::of(&node.tag) {
            let index = out.records.len();
            import_surface(family, node, &root, index, &mut out)?;
        }
    }
    if out.records.is_empty() {
        return Err(MtlxError::NoSurface);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gltf_import::Warning;
    use crate::record::MaterialRecord;

    fn doc(body: &str) -> String {
        format!("<?xml version=\"1.0\"?>\n<materialx version=\"1.38\">\n{body}\n</materialx>\n")
    }

    fn round_trips(import: &Import) {
        for record in &import.records {
            let again = MaterialRecord::parse(&record.to_text()).expect("canonical text");
            assert_eq!(&again, record, "{}", record.name);
        }
    }

    #[test]
    fn mtlx_import_brass_constants_are_exact() {
        let imp = import_mtlx(&doc(r#"<!-- a constant metal -->
            <standard_surface name="brass" type="surfaceshader">
              <input name="base" type="float" value="1" />
              <input name="base_color" type="color3" value="0.8, 0.6, 0.2" />
              <input name="metalness" type="float" value="1" />
              <input name="specular_roughness" type="float" value="0.3" />
              <input name="specular_IOR" type="float" value="1.5" />
            </standard_surface>
            <surfacematerial name="brass_mat" type="material">
              <input name="surfaceshader" type="surfaceshader" nodename="brass" />
            </surfacematerial>"#))
        .expect("imports");
        let r = &imp.records[0];
        assert_eq!(r.name, "brass");
        assert_eq!(r.look.base_color, [0.8, 0.6, 0.2]);
        assert_eq!(r.look.base_metalness, 1.0);
        assert_eq!(r.look.specular_roughness, 0.3);
        assert!(imp.is_exact("brass"), "{:?}", imp.warnings);
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_spec_defaults_fill_missing_inputs() {
        let imp = import_mtlx(&doc(r#"<standard_surface name="plain"/>"#)).expect("imports");
        let look = &imp.records[0].look;
        // standard_surface: base 0.8 times base_color 0.8, roughness 0.2.
        assert!((look.base_color[0] - 0.64).abs() < 1e-6);
        assert_eq!(look.specular_roughness, 0.2);
        assert!(look.coat.is_none() && look.sheen.is_none() && look.emission.is_none());
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_lobes_map_to_the_record() {
        let imp = import_mtlx(&doc(r#"<standard_surface name="lacquer">
              <input name="base" value="1"/>
              <input name="base_color" value="0.5, 0.1, 0.1"/>
              <input name="specular_IOR" value="1.33"/>
              <input name="coat" value="0.8"/>
              <input name="coat_roughness" value="0.05"/>
              <input name="sheen" value="0.4"/>
              <input name="sheen_color" value="0.9, 0.9, 1"/>
              <input name="sheen_roughness" value="0.6"/>
              <input name="emission" value="250"/>
              <input name="emission_color" value="1, 0.5, 0"/>
              <input name="transmission" value="0.25"/>
            </standard_surface>"#))
        .expect("imports");
        let look = &imp.records[0].look;
        assert_eq!(look.specular_ior, 1.33);
        let coat = look.coat.expect("coat");
        assert_eq!((coat.weight, coat.roughness, coat.ior), (0.8, 0.05, 1.5));
        let sheen = look.sheen.expect("sheen");
        assert_eq!((sheen.weight, sheen.roughness), (0.4, 0.6));
        assert_eq!(sheen.color, [0.9, 0.9, 1.0]);
        let emission = look.emission.expect("emission");
        assert_eq!(
            (emission.luminance, emission.color),
            (250.0, [1.0, 0.5, 0.0])
        );
        assert_eq!(look.transmission.map(|t| t.weight), Some(0.25));
        assert!(imp.is_exact("lacquer"), "{:?}", imp.warnings);
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_open_pbr_names_map_the_same_way() {
        let imp = import_mtlx(&doc(
            r#"<open_pbr_surface name="velvet" type="surfaceshader">
              <input name="base_weight" value="1"/>
              <input name="base_color" value="0.2, 0.1, 0.3"/>
              <input name="base_metalness" value="0"/>
              <input name="specular_roughness" value="0.7"/>
              <input name="specular_ior" value="1.4"/>
              <input name="fuzz_weight" value="1"/>
              <input name="fuzz_color" value="0.8, 0.7, 0.9"/>
              <input name="fuzz_roughness" value="0.5"/>
              <input name="coat_weight" value="0.3"/>
              <input name="emission_luminance" value="20"/>
            </open_pbr_surface>"#,
        ))
        .expect("imports");
        let look = &imp.records[0].look;
        assert_eq!(look.base_color, [0.2, 0.1, 0.3]);
        assert_eq!(look.specular_ior, 1.4);
        assert_eq!(look.sheen.map(|s| s.color), Some([0.8, 0.7, 0.9]));
        // Coat roughness and ior were not written: the OpenPBR defaults (0 and 1.6).
        let coat = look.coat.expect("coat");
        assert_eq!((coat.roughness, coat.ior), (0.0, 1.6));
        assert_eq!(look.emission.map(|e| e.luminance), Some(20.0));
        assert!(imp.is_exact("velvet"), "{:?}", imp.warnings);
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_an_image_input_becomes_a_texture_slot() {
        let imp = import_mtlx(&doc(
            r#"<image name="img_base" type="color3">
              <input name="file" type="filename" value="textures/Plank Wood.PNG" colorspace="srgb_texture"/>
            </image>
            <image name="img_glow" type="color3">
              <input name="file" type="filename" value="glow.png"/>
            </image>
            <image name="img_rough" type="float">
              <input name="file" type="filename" value="rough.png"/>
            </image>
            <standard_surface name="plank">
              <input name="base" value="1"/>
              <input name="base_color" type="color3" nodename="img_base"/>
              <input name="specular_roughness" type="float" nodename="img_rough"/>
              <input name="emission" value="40"/>
              <input name="emission_color" type="color3" nodename="img_glow"/>
            </standard_surface>"#,
        ))
        .expect("imports");
        let look = &imp.records[0].look;
        assert_eq!(look.textures.base_color.as_deref(), Some("tex:Plank_Wood"));
        assert_eq!(look.textures.emissive.as_deref(), Some("tex:glow"));
        assert_eq!(look.base_color, [1.0; 3], "the image supplies the color");
        // A roughness image has no slot of its own: named, and the constant default stays.
        assert_eq!(
            imp.warnings,
            vec![Warning {
                material: "plank".to_string(),
                field: "specular_roughness".to_string(),
                kind: WarningKind::Unsupported,
            }]
        );
        assert_eq!(look.specular_roughness, 0.2);
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_unknown_and_out_of_range_inputs_are_named() {
        let imp = import_mtlx(&doc(r#"<standard_surface name="odd">
              <input name="subsurface" value="0.5"/>
              <input name="thin_film_thickness" value="300"/>
              <input name="specular_roughness" value="1.5"/>
              <input name="specular_IOR" value="9"/>
            </standard_surface>"#))
        .expect("imports");
        let named = |field: &str, kind| {
            imp.warnings
                .iter()
                .any(|w| w.field == field && w.kind == kind)
        };
        assert!(named("subsurface", WarningKind::Unsupported));
        assert!(named("thin_film_thickness", WarningKind::Unsupported));
        assert!(named("specular_roughness", WarningKind::Clamped));
        assert!(named("specular_IOR", WarningKind::Clamped));
        assert_eq!(imp.records[0].look.specular_roughness, 1.0);
        assert_eq!(imp.records[0].look.specular_ior, MAX_IOR);
        round_trips(&imp);
    }

    #[test]
    fn mtlx_import_a_nodegraph_is_refused_by_name() {
        let text = doc(r#"<nodegraph name="marble_graph">
              <noise3d name="n" type="float"/>
              <output name="out" type="color3" nodename="n"/>
            </nodegraph>
            <standard_surface name="marble">
              <input name="base_color" type="color3" nodegraph="marble_graph" output="out"/>
            </standard_surface>"#);
        assert_eq!(
            import_mtlx(&text),
            Err(MtlxError::UnsupportedNode("marble_graph".to_string()))
        );
    }

    #[test]
    fn mtlx_import_an_input_linked_to_a_procedural_node_is_refused_by_name() {
        let text = doc(r#"<fractal3d name="clouds" type="float"/>
            <standard_surface name="sky">
              <input name="metalness" type="float" nodename="clouds"/>
            </standard_surface>"#);
        assert_eq!(
            import_mtlx(&text),
            Err(MtlxError::UnsupportedNode("clouds".to_string()))
        );
        // A link to a node that is not in the file is the same refusal.
        let missing = doc(
            r#"<standard_surface name="sky"><input name="metalness" nodename="ghost"/></standard_surface>"#,
        );
        assert_eq!(
            import_mtlx(&missing),
            Err(MtlxError::UnsupportedNode("ghost".to_string()))
        );
    }

    #[test]
    fn mtlx_import_bad_files_are_refused_not_guessed() {
        let xml_at = |text: &str| matches!(import_mtlx(text), Err(MtlxError::Xml { .. }));
        assert!(xml_at("<materialx><standard_surface></materialx>"));
        assert!(xml_at(
            "<materialx><standard_surface name=brass/></materialx>"
        ));
        assert!(xml_at("<materialx><!-- never closed </materialx>"));
        assert!(xml_at("<mtlx/>"));
        assert!(xml_at("<materialx/><materialx/>"));
        assert_eq!(import_mtlx("<materialx/>"), Err(MtlxError::NoSurface));
        assert_eq!(
            import_mtlx(&doc(
                r#"<standard_surface name="x"><input name="metalness" value="shiny"/></standard_surface>"#
            )),
            Err(MtlxError::BadValue {
                input: "metalness".to_string(),
                text: "shiny".to_string()
            })
        );
        assert_eq!(
            import_mtlx(&"<".repeat(MAX_MTLX_BYTES + 1)),
            Err(MtlxError::TooLarge)
        );
        let deep = format!("{}{}", "<a>".repeat(40), "</a>".repeat(40));
        assert!(xml_at(&doc(&deep)));
    }

    #[test]
    fn mtlx_import_key_indexes_follow_the_input_table() {
        for (position, input) in INPUTS.iter().enumerate() {
            assert_eq!(input.key.index(), position, "{}", input.standard);
        }
    }

    #[test]
    fn mtlx_import_same_text_gives_the_same_records_twice() {
        let text = doc(
            r#"<standard_surface name="a"><input name="coat" value="0.5"/></standard_surface>
            <open_pbr_surface name="a"><input name="coat_weight" value="0.5"/></open_pbr_surface>"#,
        );
        let first = import_mtlx(&text).expect("imports");
        let second = import_mtlx(&text).expect("imports");
        assert_eq!(first, second);
        // Two nodes of one name stay two records.
        assert_eq!(first.records.len(), 2);
        assert_ne!(first.records[0].name, first.records[1].name);
    }
}
