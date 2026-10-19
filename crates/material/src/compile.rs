//! compile: a material record to one WGSL module, byte-stable.
//!
//! The flow: record -> [`Structure`] (which lobes, texture slots and
//! gameplay layers the record uses) -> WGSL text composed from named chunks
//! in one fixed order -> optional naga check ([`validate`], feature
//! `validate`). Two records with the same structure get the same bytes, so
//! they share one pipeline; a parameter edit changes only the packed
//! [`pack_uniform`] floats and never the text. A structure edit (a new lobe,
//! a new layer kind) changes the text and is the only edit that needs a new
//! pipeline. Nothing here touches a GPU, a file or a clock.
//!
//! Layout the host binds (the same for every structure):
//!
//! - group 0: binding 0 camera, 1 light, 2 [`MaterialU`] (`material`), 3
//!   per-object coverage channels (`chan`, 8 floats).
//! - group 1 (only when a texture slot is used): binding 0 sampler, then
//!   1 base color, 2 ORM, 3 normal, 4 emissive for the slots the structure
//!   names, in that order ([`Structure::texture_bindings`]).
//!
//! The lighting follows the Filament material model (Apache-2.0): GGX
//! specular with the correlated Smith visibility, Burley diffuse, a
//! clear-coat lobe over the base, Charlie sheen, anisotropic GGX, skin
//! subsurface (the terminator LUT of [`crate::skin`]) and thin transmission. The base BRDF constructs are the ones
//! `render::v1_gpu` ports from Bevy; each chunk names its source. Energy is
//! kept by construction: the diffuse takes only what the specular lobe left
//! (baked albedo tables in [`crate::dfg`]), multiple scattering is
//! compensated from the same table, a coat or sheen takes its view albedo out
//! of what lies below, and the white-furnace test pins the result.
//!
//! PORT(google/filament@v1.58:shaders/src/brdf.fs, Apache-2.0): D_Charlie,
//! V_Neubelt, D_GGX_Anisotropic, V_SmithGGXCorrelated_Anisotropic.
//! PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl, MIT or
//! Apache-2.0): D_GGX, V_SmithGGXCorrelated, F_Schlick, Fd_Burley, specular
//! multiscatter form. Frostbite (Lagarde and de Rousiers 2014) Burley energy
//! normalization: an idea, reimplemented.

use crate::record::{Channel, LayerKind, MaterialRecord};
use engine_core::Fnv64;

/// Layer cap: [`crate::record::MAX_LAYERS`].
pub const MAX_LAYERS: usize = crate::record::MAX_LAYERS;

/// Float count of the fixed head of [`MaterialU`] (12 vec4).
pub const UNIFORM_HEAD_FLOATS: usize = 48;

/// Float count of the per-object channel block (`chan`): wetness, dirt,
/// blood, strain, heat and three pad floats.
pub const CHANNEL_FLOATS: usize = 8;

/// Scene luminance per nit: emission `luminance` times this lands in the
/// same units as the light `color * intensity`.
pub const NITS_TO_SCENE: f32 = 0.001;

/// Share of the anisotropic specular lobe that is kept. The albedo tables are isotropic and the
/// lobe reads them at the same alpha product, which over-reports the albedo of an anisotropic
/// lobe at grazing views by up to 4.2% (the furnace sweep over strengths 0.3 to 0.9 and both
/// tangent rotations). Scaling the lobe by this share keeps its white furnace under the 1.03 bar
/// with margin; the cost is a brushed metal 3% darker.
pub const ANISO_ENERGY_KEEP: f32 = 0.97;

/// Filament perceptual-roughness floor (`0.089`, Apache-2.0): below it a
/// half-precision build loses the specular peak, so [`clamp_perceptual_roughness`]
/// and the generated fragment clamp to it. The one source of the number; the
/// WGSL text declares it as `ROUGHNESS_FLOOR`.
pub const PERCEPTUAL_ROUGHNESS_FLOOR: f32 = 0.089;

/// Filament anisotropic alpha floor (`0.001`, Apache-2.0): the anisotropic
/// roughness pair never reaches zero, so `D_GGX_Anisotropic` has no singular
/// denominator. The WGSL text declares it as `ANISOTROPY_FLOOR`.
pub const ANISOTROPY_FLOOR: f32 = 0.001;

/// Filament smallest half normal (`2^-14`, about `6.1e-5`): the fp16-safe
/// lower bound the Schlick power base is clamped to. `2^-14` is exact in f32
/// and the shortest literal below rounds to it, so no digit is spare (clippy
/// `excessive_precision`). The WGSL text declares it as `MIN_HALF_NORMAL`.
pub const MIN_HALF_NORMAL: f32 = 6.103_515_6e-5;

/// The perceptual-roughness clamp the generated BRDF applies:
/// [`PERCEPTUAL_ROUGHNESS_FLOOR`] below, one above. The CPU twin
/// ([`crate::twin`]) reads this same function, so removing the clamp fails the
/// white-furnace bar and the clamp test together.
///
/// Units: perceptual roughness. Determinism: pure math, no state. Errors: none.
#[must_use]
pub fn clamp_perceptual_roughness(p: f32) -> f32 {
    p.clamp(PERCEPTUAL_ROUGHNESS_FLOOR, 1.0)
}

/// Lobe bits of [`Structure::lobes`].
pub mod lobe {
    /// Clear coat (from the record's `coat`, or a wet or blood layer).
    pub const COAT: u8 = 1;
    /// Charlie sheen.
    pub const SHEEN: u8 = 2;
    /// Anisotropic GGX.
    pub const ANISOTROPY: u8 = 4;
    /// Thin transmission.
    pub const TRANSMISSION: u8 = 8;
    /// Skin subsurface: the terminator LUT and the thin transmittance ([`crate::skin`]).
    pub const SUBSURFACE: u8 = 16;
    /// Emission (from the record, or a burn layer).
    pub const EMISSION: u8 = 32;
}

/// Texture-slot bits of [`Structure::textures`].
pub mod slot {
    /// sRGB base color.
    pub const BASE_COLOR: u8 = 1;
    /// Linear occlusion / roughness / metalness.
    pub const ORM: u8 = 2;
    /// Tangent-space normal.
    pub const NORMAL: u8 = 4;
    /// sRGB emissive.
    pub const EMISSIVE: u8 = 8;
}

/// One lobe entry in the six-lobe registry: its bit plus its document name.
///
/// Why a registry: RL48-51 code to one shape instead of re-listing bits.
/// Units: bit flags plus names. Determinism: fixed array order on every
/// host. Errors: none; unknown bits resolve to `None`.
///
/// Consumer: [`lobe_def`] and [`resolve_lobes`] read it;
/// [`Structure::has_lobe`] routes the fragment build through it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LobeDef {
    /// Bit in [`Structure::lobes`].
    pub bit: u8,
    /// Document name (`coat`, `sheen`, ...).
    pub name: &'static str,
}

/// Lobe count: the registry holds exactly six entries.
pub const LOBE_COUNT: usize = 6;

/// All six lobe bits set: the mask a full structure carries.
pub const LOBE_ALL_BITS: u8 = lobe::COAT
    | lobe::SHEEN
    | lobe::ANISOTROPY
    | lobe::TRANSMISSION
    | lobe::SUBSURFACE
    | lobe::EMISSION;

/// The six lobes in bit order: the shape RL48-51 code to.
///
/// Consumer: [`lobe_def`] and [`resolve_lobes`] read it;
/// [`Structure::has_lobe`] routes the fragment build through it.
/// Units: bit flags plus names. Determinism: fixed order on every host.
/// Errors: none.
pub const LOBE_REGISTRY: [LobeDef; LOBE_COUNT] = [
    LobeDef {
        bit: lobe::COAT,
        name: "coat",
    },
    LobeDef {
        bit: lobe::SHEEN,
        name: "sheen",
    },
    LobeDef {
        bit: lobe::ANISOTROPY,
        name: "anisotropy",
    },
    LobeDef {
        bit: lobe::TRANSMISSION,
        name: "transmission",
    },
    LobeDef {
        bit: lobe::SUBSURFACE,
        name: "subsurface",
    },
    LobeDef {
        bit: lobe::EMISSION,
        name: "emission",
    },
];

/// Resolve one lobe bit to its registry entry.
///
/// Units: bit flags. Determinism: fixed order, same answer on every host.
/// Errors: none; zero, combined or unknown bits give `None`.
///
/// Consumer: [`Structure::has_lobe`] routes the fragment build through it.
#[must_use]
pub fn lobe_def(bit: u8) -> Option<&'static LobeDef> {
    LOBE_REGISTRY.iter().find(|d| d.bit == bit)
}

/// Resolve a lobe mask to its entries in registry order.
///
/// Units: bit flags. Determinism: registry order on every host.
/// Errors: none; unknown bits are ignored.
///
/// Consumer: [`Structure::has_lobe`] routes the fragment build through it.
pub fn resolve_lobes(bits: u8) -> impl Iterator<Item = &'static LobeDef> {
    LOBE_REGISTRY.iter().filter(move |d| bits & d.bit != 0)
}

/// What the shader text depends on. Equal structures give equal WGSL.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Structure {
    /// Lobe bits ([`lobe`]).
    pub lobes: u8,
    /// Texture-slot bits ([`slot`]).
    pub textures: u8,
    /// Gameplay layers in application order: kind and driving channel.
    pub layers: Vec<(LayerKind, Channel)>,
}

impl Structure {
    /// The structure of `record`.
    #[must_use]
    pub fn of(record: &MaterialRecord) -> Self {
        let look = &record.look;
        let mut lobes = 0u8;
        if look.coat.is_some() {
            lobes |= lobe::COAT;
        }
        if look.sheen.is_some() {
            lobes |= lobe::SHEEN;
        }
        if look.anisotropy.is_some() {
            lobes |= lobe::ANISOTROPY;
        }
        if look.transmission.is_some() {
            lobes |= lobe::TRANSMISSION;
        }
        if look.subsurface.is_some() {
            lobes |= lobe::SUBSURFACE;
        }
        if look.emission.is_some() {
            lobes |= lobe::EMISSION;
        }
        let layers: Vec<(LayerKind, Channel)> = record
            .layers
            .iter()
            .take(MAX_LAYERS)
            .map(|l| (l.kind, l.driver))
            .collect();
        for (kind, _) in &layers {
            match kind {
                LayerKind::Wet | LayerKind::Blood => lobes |= lobe::COAT,
                LayerKind::Burn => lobes |= lobe::EMISSION,
                LayerKind::Dirt | LayerKind::Damage => {}
            }
        }
        let mut textures = 0u8;
        if look.textures.base_color.is_some() {
            textures |= slot::BASE_COLOR;
        }
        if look.textures.orm.is_some() {
            textures |= slot::ORM;
        }
        if look.textures.normal.is_some() {
            textures |= slot::NORMAL;
        }
        if look.textures.emissive.is_some() {
            textures |= slot::EMISSIVE;
        }
        debug_assert!(
            lobes & !LOBE_ALL_BITS == 0,
            "structure builds only registry lobes"
        );
        Self {
            lobes,
            textures,
            layers,
        }
    }

    /// True when `bit` is a known lobe and this structure carries it.
    /// Routes through [`lobe_def`] plus [`resolve_lobes`] so unknown bits
    /// never count. Units: bit flags. Determinism: same answer on every
    /// host. Errors: none.
    ///
    /// Consumer: [`wgsl_for`] plus the fragment build read it; the door
    /// `material` pack and RL48-51 read it through [`compile`].
    #[must_use]
    pub fn has_lobe(&self, bit: u8) -> bool {
        lobe_def(bit).is_some() && resolve_lobes(self.lobes).any(|d| d.bit == bit)
    }

    /// True when the record's alpha is below one: the host must draw it in
    /// the blended pass.
    #[must_use]
    pub fn blended(&self) -> bool {
        self.has_lobe(lobe::TRANSMISSION)
    }

    /// Group-1 binding numbers of the used texture slots, in slot order:
    /// `(slot bit, binding)`. Binding 0 is the shared sampler.
    #[must_use]
    pub fn texture_bindings(&self) -> Vec<(u8, u32)> {
        let mut out = Vec::new();
        let mut binding = 1u32;
        for bit in [slot::BASE_COLOR, slot::ORM, slot::NORMAL, slot::EMISSIVE] {
            if self.textures & bit != 0 {
                out.push((bit, binding));
                binding += 1;
            }
        }
        out
    }

    /// Stable key over the structure: the pipeline-sharing key.
    #[must_use]
    pub fn key(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u8(self.lobes);
        hasher.write_u8(self.textures);
        for (kind, channel) in &self.layers {
            hasher.write_str(kind.name());
            hasher.write_u8(0);
            hasher.write_str(channel.name());
            hasher.write_u8(0);
        }
        hasher.finish()
    }

    /// Byte size of [`MaterialU`] for this structure.
    #[must_use]
    pub fn uniform_bytes(&self) -> usize {
        (UNIFORM_HEAD_FLOATS + 8 * self.layers.len()) * 4
    }
}

/// One compiled record: the WGSL text, its structure and its first uniform.
#[derive(Clone, PartialEq, Debug)]
pub struct Compiled {
    /// What the text depends on.
    pub structure: Structure,
    /// [`Structure::key`].
    pub key: u64,
    /// The complete WGSL module (vertex and fragment entry points `vs`, `fs`).
    pub wgsl: String,
    /// [`pack_uniform`] of the record at exposure 1.
    pub uniform: Vec<f32>,
}

/// Fold of the WGSL bytes: the number a "bytes identical x2" test pins.
#[must_use]
pub fn wgsl_digest(wgsl: &str) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_str(wgsl);
    hasher.finish()
}

/// One row of the material table: one record and the compiled structure every
/// draw of it bins under.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaterialRow {
    /// Wire id of the record ([`crate::set::MaterialSet::wire_id`]).
    pub id: u8,
    /// Record name.
    pub name: String,
    /// [`Structure::lobes`] of the row.
    pub lobes: u8,
    /// [`Structure::textures`] of the row.
    pub textures: u8,
    /// [`Structure::key`]: the pipeline-sharing key of the row.
    pub key: u64,
}

/// The one material table: every shipped record in wire order, exactly one row
/// per material id.
///
/// Why: the forward pass bins a draw by material id, and an id maps to one
/// compiled structure. Generating the rows from
/// [`crate::set::MaterialSet::shipped`] means a record added to a shipped set
/// cannot miss a row, and the row's lobes come from [`Structure::of`], so the
/// table can never disagree with [`compile`].
///
/// Units: wire id, lobe and texture bits, the FNV pipeline key. Determinism:
/// set order plus [`Structure::key`], the same bytes on every host. Errors:
/// none. Budget: [`crate::set::MAX_RECORDS`] rows.
#[must_use]
pub fn material_table() -> Vec<MaterialRow> {
    crate::set::MaterialSet::shipped()
        .records()
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            let id = u8::try_from(index).ok()?;
            let structure = Structure::of(record);
            Some(MaterialRow {
                id,
                name: record.name.clone(),
                lobes: structure.lobes,
                textures: structure.textures,
                key: structure.key(),
            })
        })
        .collect()
}

/// The row of material id `id`, or `None` when no shipped record holds it.
///
/// Units: wire id. Determinism: [`material_table`] order. Errors: none.
#[must_use]
pub fn material_row(id: u8) -> Option<MaterialRow> {
    material_table().into_iter().find(|row| row.id == id)
}

/// Compile one record. Total: a record that parsed always compiles; the
/// naga check ([`validate`]) is a separate, optional step.
#[must_use]
pub fn compile(record: &MaterialRecord) -> Compiled {
    let structure = Structure::of(record);
    let wgsl = wgsl_for(&structure);
    Compiled {
        key: structure.key(),
        uniform: pack_uniform(record, 1.0),
        structure,
        wgsl,
    }
}

/// Default tint of a layer kind that wrote none.
fn default_tint(kind: LayerKind) -> [f32; 3] {
    match kind {
        LayerKind::Dirt => [0.32, 0.27, 0.2],
        LayerKind::Blood => [0.35, 0.02, 0.02],
        LayerKind::Burn => [1.0, 0.35, 0.05],
        LayerKind::Wet | LayerKind::Damage => [0.0, 0.0, 0.0],
    }
}

/// Default darkening of a layer kind that wrote none.
fn default_darken(kind: LayerKind) -> f32 {
    match kind {
        LayerKind::Dirt => 0.1,
        LayerKind::Damage => 0.25,
        LayerKind::Burn => 0.6,
        LayerKind::Wet | LayerKind::Blood => 0.0,
    }
}

/// Pack the record's values into the [`MaterialU`] float stream: 12 vec4
/// head, then two vec4 per layer. Only values; the structure decides which
/// of them the shader reads. A lobe the record lacks packs as zeros.
#[must_use]
pub fn pack_uniform(record: &MaterialRecord, exposure: f32) -> Vec<f32> {
    let look = &record.look;
    let mut out = Vec::with_capacity(UNIFORM_HEAD_FLOATS + 8 * record.layers.len());
    // 0: base color, metalness
    out.extend_from_slice(&look.base_color);
    out.push(look.base_metalness);
    // 1: roughness, ior, normal scale, occlusion strength
    out.extend_from_slice(&[
        look.specular_roughness,
        look.specular_ior,
        look.normal_scale,
        look.occlusion_strength,
    ]);
    // 2: coat weight, roughness, ior
    let (cw, cr, ci) = look
        .coat
        .map_or((0.0, 0.0, 1.5), |c| (c.weight, c.roughness, c.ior));
    out.extend_from_slice(&[cw, cr, ci, 0.0]);
    // 3: sheen color, weight
    let (sc, sw, sr) = look
        .sheen
        .map_or(([0.0; 3], 0.0, 0.5), |s| (s.color, s.weight, s.roughness));
    out.extend_from_slice(&[sc[0], sc[1], sc[2], sw]);
    // 4: sheen roughness, anisotropy strength, rotation
    let (as_, ar) = look
        .anisotropy
        .map_or((0.0, 0.0), |a| (a.strength, a.rotation));
    out.extend_from_slice(&[sr, as_, ar, 0.0]);
    // 5: transmission attenuation color, weight
    let (tc, tw, tt) = look.transmission.map_or(([1.0; 3], 0.0, 0.0), |t| {
        (t.attenuation_color, t.weight, t.thickness)
    });
    out.extend_from_slice(&[tc[0], tc[1], tc[2], tw]);
    // 6: thickness
    out.extend_from_slice(&[tt, 0.0, 0.0, 0.0]);
    // 7: subsurface color, weight
    let (ssc, ssw, ssr) = look
        .subsurface
        .map_or(([1.0; 3], 0.0, [0.0; 3]), |s| (s.color, s.weight, s.radius));
    out.extend_from_slice(&[ssc[0], ssc[1], ssc[2], ssw]);
    // 8: subsurface radius
    out.extend_from_slice(&[ssr[0], ssr[1], ssr[2], 0.0]);
    // 9: emission color, scene luminance
    let (ec, el) = look
        .emission
        .map_or(([0.0; 3], 0.0), |e| (e.color, e.luminance * NITS_TO_SCENE));
    out.extend_from_slice(&[ec[0], ec[1], ec[2], el]);
    // 10: exposure
    out.extend_from_slice(&[exposure, 0.0, 0.0, 0.0]);
    // 11: reserved for the host (frame index, cascade): zero
    out.extend_from_slice(&[0.0; 4]);
    debug_assert_eq!(out.len(), UNIFORM_HEAD_FLOATS);
    for layer in record.layers.iter().take(MAX_LAYERS) {
        let darken = layer.darken.unwrap_or_else(|| default_darken(layer.kind));
        let (to, has_to) = layer.roughness_to.map_or((0.0, 0.0), |v| (v, 1.0));
        // layer_a: strength, darken, roughness_to, has roughness_to
        out.extend_from_slice(&[layer.strength, darken, to, has_to]);
        let tint = layer.tint.unwrap_or_else(|| default_tint(layer.kind));
        // layer_b: tint, has tint (a burn layer's tint is its ember color)
        let has_tint = f32::from(u8::from(match layer.kind {
            LayerKind::Burn => false,
            LayerKind::Dirt | LayerKind::Blood => true,
            LayerKind::Wet | LayerKind::Damage => layer.tint.is_some(),
        }));
        out.extend_from_slice(&[tint[0], tint[1], tint[2], has_tint]);
    }
    out
}

/// The one shared BRDF text every compiled structure embeds.
///
/// Why one copy: the lit part of the forward shader is generated from
/// this text, never written twice; a second copy would drift (the energy
/// term in `fd_burley` is the drift that bit once). The window forward
/// shader keeps its own staged copy until it routes through here; this
/// accessor is the material-side end of that route.
///
/// The Filament clamps are WGSL `const`s generated from
/// [`PERCEPTUAL_ROUGHNESS_FLOOR`], [`ANISOTROPY_FLOOR`] and
/// [`MIN_HALF_NORMAL`], so each number lives once and the text names it.
/// Units: none (WGSL text). Determinism: the same bytes on every host.
/// Errors: none.
///
/// Consumer: [`wgsl_for`] embeds it below; the door `material` pack and
/// the render forward bind read it through [`compile`].
#[must_use]
pub fn brdf_wgsl() -> String {
    let mut out = String::with_capacity(BRDF_CHUNK_CAPACITY);
    out.push_str("const PI: f32 = 3.141592653589793;\n");
    out.push_str(&format!(
        "const ROUGHNESS_FLOOR: f32 = {PERCEPTUAL_ROUGHNESS_FLOOR:?};\n"
    ));
    out.push_str(&format!(
        "const ANISOTROPY_FLOOR: f32 = {ANISOTROPY_FLOOR:?};\n"
    ));
    out.push_str(&format!(
        "const MIN_HALF_NORMAL: f32 = {MIN_HALF_NORMAL:?};\n"
    ));
    out.push_str(CHUNK_BRDF_BODY);
    out
}

/// The generated source for a structure. Bytes depend only on `structure`.
#[must_use]
pub fn wgsl_for(structure: &Structure) -> String {
    let mut out = String::with_capacity(8 * 1024);
    out.push_str(CHUNK_HEADER);
    push_material_struct(&mut out, structure);
    push_bindings(&mut out, structure);
    out.push_str(CHUNK_IO);
    out.push_str(&brdf_wgsl());
    out.push_str(&crate::dfg::wgsl());
    if structure.has_lobe(lobe::SHEEN) {
        out.push_str(CHUNK_SHEEN);
    }
    if structure.has_lobe(lobe::ANISOTROPY) {
        out.push_str(CHUNK_ANISO);
    }
    if structure.has_lobe(lobe::SUBSURFACE) {
        out.push_str(&crate::skin::wgsl());
    }
    if structure.textures & slot::NORMAL != 0 {
        out.push_str(CHUNK_NORMAL_MAP);
    }
    out.push_str(CHUNK_TONEMAP);
    push_fragment(&mut out, structure);
    out
}

fn push_material_struct(out: &mut String, structure: &Structure) {
    if !structure.layers.is_empty() {
        // One layer is two vec4 in a row: `a` then `b`. The host packs the
        // same interleaved stream ([`pack_uniform`], `Structure::uniform_bytes`),
        // so the shader and the buffer agree on every layer, not only the
        // first: two separate `array<vec4>` fields would read the wrong vec4.
        out.push_str(
            "struct LayerU {\n\
             \x20   a: vec4<f32>,\n\
             \x20   b: vec4<f32>,\n\
             };\n",
        );
    }
    out.push_str(
        "struct MaterialU {\n\
         \x20   base: vec4<f32>,\n\
         \x20   surface: vec4<f32>,\n\
         \x20   coat: vec4<f32>,\n\
         \x20   sheen: vec4<f32>,\n\
         \x20   sheen2: vec4<f32>,\n\
         \x20   trans: vec4<f32>,\n\
         \x20   trans2: vec4<f32>,\n\
         \x20   sss: vec4<f32>,\n\
         \x20   sss_radius: vec4<f32>,\n\
         \x20   emit: vec4<f32>,\n\
         \x20   frame: vec4<f32>,\n\
         \x20   pad: vec4<f32>,\n",
    );
    if !structure.layers.is_empty() {
        let n = structure.layers.len();
        out.push_str(&format!("    layers: array<LayerU, {n}>,\n"));
    }
    out.push_str("};\n");
    out.push_str("struct ChannelU {\n    a: vec4<f32>,\n    b: vec4<f32>,\n};\n");
}

fn push_bindings(out: &mut String, structure: &Structure) {
    out.push_str(
        "@group(0) @binding(0) var<uniform> cam: CameraU;\n\
         @group(0) @binding(1) var<uniform> light: LightU;\n\
         @group(0) @binding(2) var<uniform> material: MaterialU;\n\
         @group(0) @binding(3) var<uniform> chan: ChannelU;\n",
    );
    let bindings = structure.texture_bindings();
    if !bindings.is_empty() {
        out.push_str("@group(1) @binding(0) var tex_sampler: sampler;\n");
        for (bit, binding) in bindings {
            let name = match bit {
                slot::BASE_COLOR => "tex_base",
                slot::ORM => "tex_orm",
                slot::NORMAL => "tex_normal",
                _ => "tex_emissive",
            };
            out.push_str(&format!(
                "@group(1) @binding({binding}) var {name}: texture_2d<f32>;\n"
            ));
        }
    }
}

fn channel_expr(channel: Channel) -> &'static str {
    match channel {
        Channel::Wetness => "chan.a.x",
        Channel::Dirt => "chan.a.y",
        Channel::Blood => "chan.a.z",
        Channel::Strain => "chan.a.w",
        Channel::Heat => "chan.b.x",
    }
}

fn push_fragment(out: &mut String, structure: &Structure) {
    let has = |bit: u8| structure.lobes & bit != 0;
    out.push_str("@fragment\nfn fs(in: VOut) -> @location(0) vec4<f32> {\n");
    out.push_str(
        "    var n = normalize(in.world_n);\n\
         \x20   let v = normalize(cam.camera_pos - in.world_pos);\n\
         \x20   var base_color = material.base.rgb;\n\
         \x20   var metallic = material.base.a;\n\
         \x20   var rough = material.surface.x;\n\
         \x20   var occlusion = 1.0;\n\
         \x20   var emissive = vec3<f32>(0.0);\n",
    );
    if has(lobe::COAT) {
        out.push_str(
            "    var coat_w = material.coat.x;\n\
             \x20   var coat_r = material.coat.y;\n\
             \x20   var coat_ior = material.coat.z;\n",
        );
    }
    if has(lobe::EMISSION) {
        out.push_str("    emissive = material.emit.rgb * material.emit.w;\n");
    }
    // Texture slots multiply the factors (glTF rule).
    if structure.textures & slot::BASE_COLOR != 0 {
        out.push_str(
            "    base_color = base_color * textureSample(tex_base, tex_sampler, in.uv).rgb;\n",
        );
    }
    if structure.textures & slot::ORM != 0 {
        out.push_str(
            "    let orm = textureSample(tex_orm, tex_sampler, in.uv).rgb;\n\
             \x20   occlusion = 1.0 + material.surface.w * (orm.r - 1.0);\n\
             \x20   rough = rough * orm.g;\n\
             \x20   metallic = metallic * orm.b;\n",
        );
    }
    if structure.textures & slot::NORMAL != 0 {
        out.push_str(
            "    let nm = textureSample(tex_normal, tex_sampler, in.uv).rgb * 2.0 - 1.0;\n\
             \x20   n = perturb_normal(n, in.world_pos, in.uv, vec3<f32>(nm.xy * material.surface.z, nm.z));\n",
        );
    }
    if structure.textures & slot::EMISSIVE != 0 {
        out.push_str(
            "    emissive = emissive + textureSample(tex_emissive, tex_sampler, in.uv).rgb;\n",
        );
    }
    // Gameplay layers, in record order.
    for (i, (kind, channel)) in structure.layers.iter().enumerate() {
        let c = channel_expr(*channel);
        out.push_str(&format!(
            "    {{\n\
             \x20       let a = material.layers[{i}].a;\n\
             \x20       let b = material.layers[{i}].b;\n\
             \x20       let cover = clamp({c}, 0.0, 1.0) * a.x;\n\
             \x20       base_color = base_color * (1.0 - a.y * cover);\n\
             \x20       base_color = mix(base_color, b.rgb, b.w * cover);\n\
             \x20       rough = mix(rough, a.z, a.w * cover);\n"
        ));
        match kind {
            LayerKind::Wet => out.push_str(
                "        coat_w = max(coat_w, cover);\n\
                 \x20       coat_ior = mix(coat_ior, 1.33, cover);\n\
                 \x20       coat_r = mix(coat_r, 0.05, cover);\n",
            ),
            LayerKind::Blood => out.push_str(
                "        coat_w = max(coat_w, cover * 0.6);\n\
                 \x20       coat_r = mix(coat_r, 0.15, cover);\n",
            ),
            LayerKind::Burn => {
                out.push_str("        emissive = emissive + b.rgb * (cover * cover);\n");
            }
            LayerKind::Dirt => out.push_str("        rough = mix(rough, 1.0, cover * 0.5);\n"),
            LayerKind::Damage => {
                out.push_str("        rough = mix(rough, 1.0, cover * 0.4);\n");
            }
        }
        out.push_str("    }\n");
    }
    out.push_str(
        "    let l = normalize(light.dir_to_light);\n\
         \x20   let h = normalize(l + v);\n\
         \x20   let ndot_l = clamp(dot(n, l), 0.0, 1.0);\n\
         \x20   let ndot_v = max(dot(n, v), 0.0001);\n\
         \x20   let ndot_h = clamp(dot(n, h), 0.0, 1.0);\n\
         \x20   let ldot_h = clamp(dot(l, h), 0.0, 1.0);\n\
         \x20   let roughness = perceptual_roughness_to_roughness(rough);\n\
         \x20   let ior_f0 = pow((material.surface.y - 1.0) / (material.surface.y + 1.0), 2.0);\n\
         \x20   let f0 = mix(vec3<f32>(ior_f0), base_color, metallic);\n\
         \x20   let f = fresnel_term(f0, ldot_h);\n\
         \x20   let f90_s = clamp(dot(f0, vec3<f32>(50.0 * 0.33)), 0.0, 1.0);\n",
    );
    if has(lobe::ANISOTROPY) {
        // Filament's anisotropic GGX with a tangent built from the normal;
        // the albedo tables are read at the isotropic roughness with the
        // same alpha product.
        out.push_str(
            "    let up_ref = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.99);\n\
             \x20   let tan0 = normalize(cross(up_ref, n));\n\
             \x20   let bit0 = cross(n, tan0);\n\
             \x20   let tan_angle = material.sheen2.z * 6.283185307179586;\n\
             \x20   let tan_t = tan0 * cos(tan_angle) + bit0 * sin(tan_angle);\n\
             \x20   let tan_b = cross(n, tan_t);\n\
             \x20   let a_t = min(roughness * (1.0 + material.sheen2.y), 1.0);\n\
             \x20   let a_b = max(roughness * (1.0 - material.sheen2.y), ANISOTROPY_FLOOR);\n\
             \x20   let rough_e = sqrt(sqrt(a_t * a_b));\n\
             \x20   let spec_d = d_ggx_anisotropic(a_t, a_b, dot(tan_t, h), dot(tan_b, h), ndot_h);\n",
        );
        out.push_str(&format!(
            "    let vv = v_smith_ggx_anisotropic(a_t, a_b, dot(tan_t, v), dot(tan_b, v), dot(tan_t, l), dot(tan_b, l), ndot_v, ndot_l) * {ANISO_ENERGY_KEEP:?};\n"
        ));
    } else {
        out.push_str(
            "    let rough_e = clamp(rough, ROUGHNESS_FLOOR, 1.0);\n\
             \x20   let spec_d = d_ggx(roughness, ndot_h);\n\
             \x20   let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);\n",
        );
    }
    out.push_str(
        "    let fab = dfg_lookup(ndot_v, rough_e);\n\
         \x20   let spec_albedo = min(f0 * fab.x + vec3<f32>(f90_s * fab.y), vec3<f32>(1.0));\n\
         \x20   var diffuse_albedo = base_color * (1.0 - metallic) * (vec3<f32>(1.0) - spec_albedo);\n",
    );
    if has(lobe::TRANSMISSION) {
        out.push_str("    diffuse_albedo = diffuse_albedo * (1.0 - material.trans.w);\n");
    }
    out.push_str(
        "    var diffuse = diffuse_albedo * fd_burley(roughness, ndot_v, ndot_l, ldot_h);\n",
    );
    if has(lobe::SUBSURFACE) {
        // RL50: the diffuse cosine becomes the skin terminator LUT (red keeps light past the
        // terminator, blue stops at it), and light from behind passes a thin slab per channel.
        // `skin_extra` is added after the cosine, so the dark side can light up.
        out.push_str(&format!(
            "    var skin_extra = vec3<f32>(0.0);\n\
             \x20   {{\n\
             \x20       let w = material.sss.w;\n\
             \x20       let geo_n = normalize(in.world_n);\n\
             \x20       let curvature = clamp(length(fwidth(geo_n)) / max(length(fwidth(in.world_pos)), 0.00001) * {curv}, 0.0, 1.0);\n\
             \x20       let blurred = skin_terminator(dot(n, l), curvature);\n\
             \x20       skin_extra = diffuse * (mix(vec3<f32>(ndot_l), blurred, w) - vec3<f32>(ndot_l));\n\
             \x20       let back = pow(clamp(dot(v, -(l + n * 0.5)), 0.0, 1.0), 3.0);\n\
             \x20       let spread = (material.sss_radius.r + material.sss_radius.g + material.sss_radius.b) * (1.0 / 3.0);\n\
             \x20       let thin = skin_transmit(spread * 250.0);\n\
             \x20       skin_extra = skin_extra + diffuse_albedo * material.sss.rgb * thin * (w * back * clamp(spread * 40.0, 0.0, 1.0) / 3.141592653589793);\n\
             \x20   }}\n",
            curv = crate::skin::CURVATURE_PER_INVERSE_METER
        ));
    }
    out.push_str(
        "    var spec = specular_term(spec_d, vv, f, f0, fab);\n\
         \x20   var lit = (diffuse + spec) * ndot_l;\n",
    );
    if has(lobe::SUBSURFACE) {
        out.push_str("    lit = lit + skin_extra;\n");
    }
    if has(lobe::SHEEN) {
        out.push_str(
            "    {\n\
             \x20       let sr = clamp(material.sheen2.x, 0.3, 1.0);\n\
             \x20       let sheen_term = d_charlie(sr, ndot_h) * v_neubelt(ndot_v, ndot_l) * material.sheen.rgb * material.sheen.w;\n\
             \x20       let sheen_peak = max(max(material.sheen.r, material.sheen.g), material.sheen.b);\n\
             \x20       let scale = max(1.0 - material.sheen.w * sheen_peak * sheen_albedo_lookup(ndot_v, sr), 0.0);\n\
             \x20       lit = lit * scale + sheen_term * ndot_l;\n\
             \x20   }\n",
        );
    }
    if has(lobe::COAT) {
        out.push_str(
            "    {\n\
             \x20       let cf0 = pow((coat_ior - 1.0) / (coat_ior + 1.0), 2.0);\n\
             \x20       let fc = f_schlick_vec(vec3<f32>(cf0), 1.0, ldot_h).x * coat_w;\n\
             \x20       let cr = perceptual_roughness_to_roughness(coat_r);\n\
             \x20       let cab = dfg_lookup(ndot_v, clamp(coat_r, ROUGHNESS_FLOOR, 1.0));\n\
             \x20       let coat_view = (cf0 * cab.x + cab.y) * coat_w;\n\
             \x20       let coat_spec = d_ggx(cr, ndot_h) * v_smith_ggx_correlated(cr, ndot_v, ndot_l) * fc * ndot_l;\n\
             \x20       lit = lit * (1.0 - coat_view) + vec3<f32>(coat_spec);\n\
             \x20   }\n",
        );
    }
    out.push_str(
        "    lit = lit * light.light_color;\n\
         \x20   lit = lit * occlusion;\n\
         \x20   lit = lit + emissive;\n\
         \x20   let mapped = aces_fitted(max(lit * material.frame.x, vec3<f32>(0.0)));\n",
    );
    if has(lobe::TRANSMISSION) {
        out.push_str(
            "    let thickness = material.trans2.x;\n\
             \x20   let tint = pow(max(material.trans.rgb, vec3<f32>(0.0001)), vec3<f32>(clamp(thickness * 10.0, 0.0, 10.0)));\n\
             \x20   let tint_lum = dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722));\n\
             \x20   let alpha = 1.0 - material.trans.w * tint_lum;\n\
             \x20   return vec4<f32>(linear_to_srgb_term(mapped), clamp(alpha, 0.0, 1.0));\n",
        );
    } else {
        out.push_str("    return vec4<f32>(linear_to_srgb_term(mapped), 1.0);\n");
    }
    out.push_str("}\n");
}

const CHUNK_HEADER: &str = "// generated by engine-material compile; do not edit. Bytes depend on the structure only.\n\
struct CameraU {\n    view_proj: mat4x4<f32>,\n    camera_pos: vec3<f32>,\n    _pad0: f32,\n};\n\
struct LightU {\n    dir_to_light: vec3<f32>,\n    _pad0: f32,\n    light_color: vec3<f32>,\n    intensity: f32,\n};\n";

const CHUNK_IO: &str = r#"
struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let wp = vec4<f32>(in.pos, 1.0);
    out.world_pos = wp.xyz;
    out.world_n = in.normal;
    out.uv = in.uv;
    out.clip = cam.view_proj * wp;
    return out;
}
"#;

/// Byte capacity reserved for one BRDF chunk: the three clamp declarations
/// plus [`CHUNK_BRDF_BODY`], all ASCII.
const BRDF_CHUNK_CAPACITY: usize = 2048;

/// The BRDF body: every lit function, from the one shared text. The three
/// Filament clamps are declared by [`brdf_wgsl`] and named here.
const CHUNK_BRDF_BODY: &str = r#"
fn perceptual_roughness_to_roughness(p: f32) -> f32 {
    let c = clamp(p, ROUGHNESS_FLOOR, 1.0);
    return c * c;
}

fn d_ggx(roughness: f32, ndot_h: f32) -> f32 {
    let one_minus = 1.0 - ndot_h * ndot_h;
    let a = ndot_h * roughness;
    let k = roughness / (one_minus + a * a);
    return k * k * (1.0 / PI);
}

fn v_smith_ggx_correlated(roughness: f32, ndot_v: f32, ndot_l: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = ndot_l * sqrt((ndot_v - a2 * ndot_v) * ndot_v + a2);
    let lambda_l = ndot_v * sqrt((ndot_l - a2 * ndot_l) * ndot_l + a2);
    return 0.5 / (lambda_v + lambda_l);
}

fn f_schlick_vec(f0: vec3<f32>, f90: f32, vdot_h: f32) -> vec3<f32> {
    // MIN_HALF_NORMAL keeps the power base positive: a half-vector cosine a
    // hair above one would otherwise feed a negative base to pow.
    return f0 + (f90 - f0) * pow(max(1.0 - vdot_h, MIN_HALF_NORMAL), 5.0);
}

fn fresnel_term(f0: vec3<f32>, ldot_h: f32) -> vec3<f32> {
    let f90 = clamp(dot(f0, vec3<f32>(50.0 * 0.33)), 0.0, 1.0);
    return f_schlick_vec(f0, f90, ldot_h);
}

// Burley diffuse with the Frostbite energy normalization (bias and factor on
// roughness), so a white rough surface returns at most what it receives.
fn fd_burley(roughness: f32, ndot_v: f32, ndot_l: f32, ldot_h: f32) -> f32 {
    let f90 = 0.5 * roughness + 2.0 * roughness * ldot_h * ldot_h;
    let light_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndot_l, 5.0);
    let view_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndot_v, 5.0);
    let energy = 1.0 + (1.0 / 1.51 - 1.0) * roughness;
    return light_scatter * view_scatter * energy * (1.0 / PI);
}

fn specular_term(d: f32, v: f32, f: vec3<f32>, f0: vec3<f32>, fab: vec2<f32>) -> vec3<f32> {
    var fr = (d * v) * f;
    fr *= 1.0 + f0 * (1.0 / (fab.x + fab.y) - 1.0);
    return fr;
}
"#;

const CHUNK_SHEEN: &str = r#"
// Filament D_Charlie (Estevez and Kulla) and the Neubelt visibility.
fn d_charlie(roughness: f32, ndot_h: f32) -> f32 {
    let inv_alpha = 1.0 / (roughness * roughness);
    let cos2h = ndot_h * ndot_h;
    let sin2h = max(1.0 - cos2h, 0.0078125);
    return (2.0 + inv_alpha) * pow(sin2h, inv_alpha * 0.5) / (2.0 * PI);
}
fn v_neubelt(ndot_v: f32, ndot_l: f32) -> f32 {
    return 1.0 / (4.0 * (ndot_l + ndot_v - ndot_l * ndot_v));
}
"#;

const CHUNK_ANISO: &str = r#"
// Filament D_GGX_Anisotropic and V_SmithGGXCorrelated_Anisotropic. The
// tangent frame is built from the normal in the fragment (no vertex tangent
// is needed); `rotation` in turns turns it about the normal. Filament's
// a2^3 / d2^2 underflows in f32 at low roughness (a guard on d2 flattened the
// peak and the lobe went dark); the same value is 1 / (PI a2 s^2) with
// s = th^2/at^2 + bh^2/ab^2 + nh^2, and s is never below 1.
fn d_ggx_anisotropic(at: f32, ab: f32, th: f32, bh: f32, nh: f32) -> f32 {
    let s = (th * th) / (at * at) + (bh * bh) / (ab * ab) + nh * nh;
    return 1.0 / (PI * at * ab * s * s);
}
fn v_smith_ggx_anisotropic(at: f32, ab: f32, tv: f32, bv: f32, tl: f32, bl: f32, nv: f32, nl: f32) -> f32 {
    let lambda_v = nl * length(vec3<f32>(at * tv, ab * bv, nv));
    let lambda_l = nv * length(vec3<f32>(at * tl, ab * bl, nl));
    return 0.5 / (lambda_v + lambda_l);
}
"#;

const CHUNK_NORMAL_MAP: &str = r#"
// Cotangent frame from screen derivatives (no vertex tangents).
fn perturb_normal(n: vec3<f32>, world_pos: vec3<f32>, uv: vec2<f32>, tn: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(world_pos);
    let dp2 = dpdy(world_pos);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(max(dot(t, t), dot(b, b)), 0.0000001));
    return normalize(t * (tn.x * invmax) + b * (tn.y * invmax) + n * tn.z);
}
"#;

const CHUNK_TONEMAP: &str = r#"
fn rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}
fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let rgb_to_rrt = mat3x3<f32>(
        vec3<f32>(0.59719, 0.35458, 0.04823),
        vec3<f32>(0.07600, 0.90834, 0.01566),
        vec3<f32>(0.02840, 0.13383, 0.83777)
    );
    let odt_to_rgb = mat3x3<f32>(
        vec3<f32>(1.60475, -0.53108, -0.07367),
        vec3<f32>(-0.10208, 1.10813, -0.00605),
        vec3<f32>(-0.00327, -0.07276, 1.07602)
    );
    var c = color * rgb_to_rrt;
    c = rrt_odt_fit(c);
    c = c * odt_to_rgb;
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}
fn linear_to_srgb_term(color: vec3<f32>) -> vec3<f32> {
    var out = color;
    out = select(out, vec3<f32>(0.0), out < vec3<f32>(0.0));
    let lo = out * 12.92;
    let hi = 1.055 * pow(out, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, out <= vec3<f32>(0.0031308));
}
"#;

// ---------------------------------------------------------------------------
// naga check
// ---------------------------------------------------------------------------

/// Why generated or edited WGSL did not check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WgslError {
    /// `parse` or `validate`.
    pub stage: &'static str,
    /// naga's message with the source line of the span.
    pub message: String,
}

impl std::fmt::Display for WgslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wgsl {}: {}", self.stage, self.message)
    }
}

impl std::error::Error for WgslError {}

/// Parse and validate `wgsl` with naga. No GPU needed. Needs the
/// `validate` feature.
#[cfg(feature = "validate")]
pub fn validate(wgsl: &str) -> Result<(), WgslError> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| WgslError {
        stage: "parse",
        message: e.emit_to_string(wgsl),
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| WgslError {
        stage: "validate",
        message: e.emit_to_string(wgsl),
    })
}

/// Byte size naga gives the `MaterialU` struct, for the layout test.
#[cfg(feature = "validate")]
pub fn material_uniform_size(wgsl: &str) -> Option<u32> {
    let module = naga::front::wgsl::parse_str(wgsl).ok()?;
    for (_, ty) in module.types.iter() {
        if ty.name.as_deref() != Some("MaterialU") {
            continue;
        }
        if let naga::TypeInner::Struct { span, .. } = &ty.inner {
            return Some(*span);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::MaterialSet;

    #[test]
    fn same_record_gives_the_same_bytes_twice() {
        for record in MaterialSet::siege_kit().records() {
            let a = compile(record);
            let b = compile(record);
            assert_eq!(a.wgsl, b.wgsl);
            assert_eq!(wgsl_digest(&a.wgsl), wgsl_digest(&b.wgsl));
        }
    }

    #[test]
    fn a_value_edit_never_changes_the_text() {
        let set = MaterialSet::siege_kit();
        let marble = set.get("marble").cloned().expect("shipped");
        let mut edited = marble.clone();
        edited.look.specular_roughness = 0.08;
        edited.look.base_color = [0.1, 0.2, 0.3];
        let a = compile(&marble);
        let b = compile(&edited);
        assert_eq!(a.wgsl, b.wgsl);
        assert_eq!(a.key, b.key);
        assert_ne!(a.uniform, b.uniform);
    }

    #[test]
    fn a_structure_edit_changes_the_text_and_the_key() {
        let set = MaterialSet::siege_kit();
        let stone = set.get("stone").cloned().expect("shipped");
        let mut with_sheen = stone.clone();
        with_sheen.look.sheen = Some(crate::record::Sheen {
            weight: 0.5,
            color: [0.5; 3],
            roughness: 0.5,
        });
        let a = compile(&stone);
        let b = compile(&with_sheen);
        assert_ne!(a.wgsl, b.wgsl);
        assert_ne!(a.key, b.key);
    }

    #[test]
    fn uniform_length_follows_the_layer_count() {
        for record in MaterialSet::siege_kit().records() {
            let c = compile(record);
            assert_eq!(c.uniform.len() * 4, c.structure.uniform_bytes());
        }
    }

    /// Every layer is two vec4 in a row, in the struct and in the packed
    /// stream. Two separate `array<vec4>` fields would make the shader read
    /// the wrong vec4 from the second layer on; the swatch test proves the
    /// pixels on a device, this pins the shape without one.
    #[test]
    fn layer_values_are_one_interleaved_pair_per_layer() {
        let marble = MaterialSet::siege_kit()
            .get("marble")
            .cloned()
            .expect("shipped marble");
        let compiled = compile(&marble);
        let n = compiled.structure.layers.len();
        assert!(n >= 2, "marble carries {n} layers");
        assert!(compiled.wgsl.contains("struct LayerU {"));
        assert!(compiled
            .wgsl
            .contains(&format!("layers: array<LayerU, {n}>,")));
        assert!(!compiled.wgsl.contains("layer_a"));
        for i in 0..n {
            assert!(
                compiled.wgsl.contains(&format!("material.layers[{i}].a")),
                "layer {i} reads its a"
            );
            assert!(
                compiled.wgsl.contains(&format!("material.layers[{i}].b")),
                "layer {i} reads its b"
            );
        }
        // The stream is the head, then (a, b) per layer: the second layer's
        // `a` is the second pair of the stream, not the head of a second array.
        let head = UNIFORM_HEAD_FLOATS;
        assert_eq!(compiled.uniform.len(), head + 8 * n);
        assert_eq!(compiled.uniform[head + 8..head + 12], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            compiled.uniform[head + 12..head + 16],
            [0.35, 0.02, 0.02, 1.0]
        );
    }

    #[test]
    fn shipped_materials_share_a_few_structures() {
        let mut keys: Vec<u64> = MaterialSet::siege_kit()
            .records()
            .iter()
            .chain(MaterialSet::reference().records())
            .map(|r| compile(r).key)
            .collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert!(total >= 21);
        assert!(
            keys.len() <= 12,
            "{} structures for {total} records",
            keys.len()
        );
    }

    #[test]
    fn texture_bindings_are_dense_in_slot_order() {
        let s = Structure {
            lobes: 0,
            textures: slot::ORM | slot::EMISSIVE,
            layers: Vec::new(),
        };
        assert_eq!(
            s.texture_bindings(),
            vec![(slot::ORM, 1), (slot::EMISSIVE, 2)]
        );
        let text = wgsl_for(&s);
        assert!(text.contains("@group(1) @binding(1) var tex_orm"));
        assert!(text.contains("@group(1) @binding(2) var tex_emissive"));
        assert!(!text.contains("tex_base"));
    }

    #[test]
    fn lobe_registry_resolves_six_lobes() {
        assert_eq!(LOBE_COUNT, 6, "six lobes");
        assert_eq!(LOBE_REGISTRY.len(), LOBE_COUNT);
        assert_eq!(LOBE_ALL_BITS, 63, "1|2|4|8|16|32");
        let names: Vec<&str> = LOBE_REGISTRY.iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            vec![
                "coat",
                "sheen",
                "anisotropy",
                "transmission",
                "subsurface",
                "emission"
            ]
        );
        let bits: Vec<u8> = LOBE_REGISTRY.iter().map(|d| d.bit).collect();
        assert_eq!(
            bits,
            vec![
                lobe::COAT,
                lobe::SHEEN,
                lobe::ANISOTROPY,
                lobe::TRANSMISSION,
                lobe::SUBSURFACE,
                lobe::EMISSION
            ]
        );
        for def in &LOBE_REGISTRY {
            assert_eq!(
                lobe_def(def.bit).map(|d| d.name),
                Some(def.name),
                "{} resolves",
                def.name
            );
        }
        assert!(lobe_def(0).is_none(), "zero is not one lobe");
        assert!(lobe_def(64).is_none(), "unknown bit refuses");
        assert!(lobe_def(0xFF).is_none(), "combined bits refuse");
        assert!(
            lobe_def(lobe::COAT | lobe::SHEEN).is_none(),
            "two bits refuse"
        );
        assert_eq!(resolve_lobes(0).count(), 0, "empty mask resolves empty");
        assert_eq!(
            resolve_lobes(LOBE_ALL_BITS).count(),
            6,
            "full mask resolves six"
        );
        let full: Vec<&str> = resolve_lobes(LOBE_ALL_BITS).map(|d| d.name).collect();
        assert_eq!(full, names, "registry order");
        let two: Vec<&str> = resolve_lobes(lobe::COAT | lobe::SHEEN)
            .map(|d| d.name)
            .collect();
        assert_eq!(two, vec!["coat", "sheen"]);
        assert_eq!(resolve_lobes(0xFF).count(), 6, "unknown bits are ignored");
        for record in MaterialSet::siege_kit().records() {
            let lobes = Structure::of(record).lobes;
            assert_eq!(
                lobes & !LOBE_ALL_BITS,
                0,
                "{} builds only registry lobes",
                record.name
            );
        }
    }

    #[test]
    fn structure_has_lobe_routes_through_registry() {
        let s = Structure {
            lobes: lobe::COAT | lobe::SHEEN,
            textures: 0,
            layers: Vec::new(),
        };
        assert!(s.has_lobe(lobe::COAT));
        assert!(s.has_lobe(lobe::SHEEN));
        assert!(!s.has_lobe(lobe::ANISOTROPY));
        assert!(!s.has_lobe(lobe::TRANSMISSION));
        assert!(!s.has_lobe(lobe::SUBSURFACE));
        assert!(!s.has_lobe(lobe::EMISSION));
        assert!(!s.has_lobe(0), "zero never counts");
        assert!(!s.has_lobe(64), "unknown bit never counts");
        assert!(!s.has_lobe(0xFF), "combined mask never counts as one lobe");
        // Raw `&` would count bit 64 here; the registry refuses it.
        let unknown = Structure {
            lobes: 64,
            textures: 0,
            layers: Vec::new(),
        };
        assert!(!unknown.has_lobe(64));
        assert_eq!(resolve_lobes(unknown.lobes).count(), 0);
        assert!(!unknown.blended(), "unknown bit is not transmission");
        let glass = Structure {
            lobes: lobe::TRANSMISSION,
            textures: 0,
            layers: Vec::new(),
        };
        assert!(glass.blended(), "transmission draws blended");
        assert!(!s.blended(), "coat plus sheen stays opaque");
        // The fragment build reads through the registry: sheen adds the
        // Charlie chunk, anisotropy adds its chunk, plain adds neither.
        let plain = wgsl_for(&Structure {
            lobes: 0,
            textures: 0,
            layers: Vec::new(),
        });
        assert!(!plain.contains("d_charlie"));
        assert!(!plain.contains("d_ggx_anisotropic"));
        let sheen = wgsl_for(&Structure {
            lobes: lobe::SHEEN,
            textures: 0,
            layers: Vec::new(),
        });
        assert!(sheen.contains("d_charlie"));
        let aniso = wgsl_for(&Structure {
            lobes: lobe::ANISOTROPY,
            textures: 0,
            layers: Vec::new(),
        });
        assert!(aniso.contains("d_ggx_anisotropic"));
        // The anisotropic D has no clamp on the squared length: that guard flattened the peak
        // below roughness 0.25 (see the furnace twin).
        assert!(!aniso.contains("max(d2"));
        // A shipped record routes the same way: marble layers imply a coat.
        let marble = MaterialSet::siege_kit()
            .get("marble")
            .cloned()
            .expect("shipped marble");
        let built = Structure::of(&marble);
        assert!(built.has_lobe(lobe::COAT));
        assert!(!built.has_lobe(lobe::SHEEN));
    }

    /// One text, one compile, two slices: the FPS viewmodel slice
    /// (skin, steel, cloth) and the arena PBR slice (sand, marble,
    /// stone) share the `compile` entry. Each record compiles
    /// byte-stable twice, a value edit never forks the text, and the
    /// six structures fit the frame pipeline budget.
    #[test]
    fn one_text_compile_serves_viewmodel_and_arena_slices() {
        let set = MaterialSet::shipped();
        let viewmodel = ["skin", "steel", "cloth"];
        let arena = ["sand", "marble", "stone"];
        let mut keys = Vec::new();
        for name in viewmodel.iter().chain(arena.iter()) {
            let record = set.get(name).unwrap_or_else(|| panic!("shipped {name}"));
            // Byte-stable twice through the one entry.
            let a = compile(record);
            let b = compile(record);
            assert_eq!(a.wgsl, b.wgsl, "{name} bytes x2");
            assert_eq!(a.key, b.key, "{name} key x2");
            // A value edit moves only the uniform, never the text.
            let mut edited = record.clone();
            edited.look.base_color = [0.1, 0.2, 0.3];
            edited.look.specular_roughness = 0.5;
            let c = compile(&edited);
            assert_eq!(c.wgsl, a.wgsl, "{name} value edit keeps text");
            assert_eq!(c.key, a.key, "{name} value edit keeps key");
            assert_ne!(c.uniform, a.uniform, "{name} value edit moves uniform");
            keys.push(a.key);
        }
        keys.sort_unstable();
        keys.dedup();
        assert!(keys.len() <= 12, "{} structures for 6 records", keys.len());
        // The arena triple shares pipelines: three records, fewer structures.
        let mut arena_keys: Vec<u64> = arena
            .iter()
            .map(|name| compile(set.get(name).expect("shipped arena")).key)
            .collect();
        arena_keys.sort_unstable();
        arena_keys.dedup();
        assert!(
            arena_keys.len() < arena.len(),
            "arena shares: {} for 3",
            arena_keys.len()
        );
    }

    /// Every shipped record compiles to one non-empty module that keeps both
    /// entry points, twice byte- and key-identical, and a coat edit on a bare
    /// record moves the key and the text.
    ///
    /// The row counts the 21 siege-plus-reference record names; `shipped()`
    /// holds the 19 names the siege kit does not already own, so the loop
    /// runs over those.
    #[test]
    fn every_shipped_record_compiles_to_text_with_stable_keys() {
        let shipped = MaterialSet::shipped();
        assert!(shipped.len() >= 19, "{} shipped records", shipped.len());
        for record in shipped.records() {
            let a = compile(record);
            let b = compile(record);
            assert!(!a.wgsl.is_empty(), "{} compiles to text", record.name);
            assert!(
                a.wgsl.contains("fn vs(") && a.wgsl.contains("fn fs("),
                "{} keeps the two entry points",
                record.name
            );
            assert_eq!(a.wgsl, b.wgsl, "{} bytes x2", record.name);
            assert_eq!(a.key, b.key, "{} key x2", record.name);
        }
        // A coat is a lobe, not a texture slot: the key and the text move,
        // the group-1 bindings do not.
        let bare = shipped
            .records()
            .iter()
            .find(|r| !Structure::of(r).has_lobe(lobe::COAT))
            .expect("a shipped record without a coat");
        let mut coated = bare.clone();
        coated.look.coat = Some(crate::record::Coat {
            weight: 0.8,
            roughness: 0.1,
            ior: 1.5,
        });
        let a = compile(bare);
        let b = compile(&coated);
        assert!(
            b.structure.has_lobe(lobe::COAT),
            "{} gains the coat",
            coated.name
        );
        assert_ne!(b.key, a.key, "a coat moves the key");
        assert_ne!(b.wgsl, a.wgsl, "a coat moves the text");
        assert_eq!(
            b.structure.texture_bindings(),
            a.structure.texture_bindings(),
            "a coat adds no texture binding"
        );
    }

    /// The lit part is generated once: the shared BRDF chunk holds the
    /// seven canonical functions exactly once each, every compiled
    /// structure embeds that exact chunk once, and the module keeps one
    /// `vs` plus one `fs` entry. This is the "grep finds one BRDF text"
    /// gate for everything this crate emits: WGSL here comes only from
    /// [`brdf_wgsl`] and its siblings, and each function is defined once.
    #[test]
    fn brdf_chunk_is_the_single_lit_source() {
        let chunk = brdf_wgsl();
        for name in [
            "perceptual_roughness_to_roughness",
            "d_ggx",
            "v_smith_ggx_correlated",
            "f_schlick_vec",
            "fresnel_term",
            "fd_burley",
            "specular_term",
        ] {
            let def = format!("fn {name}(");
            assert_eq!(
                chunk.match_indices(def.as_str()).count(),
                1,
                "{name} defined once in the shared chunk"
            );
        }
        for record in MaterialSet::shipped().records() {
            let text = compile(record).wgsl;
            assert_eq!(
                text.match_indices(chunk.as_str()).count(),
                1,
                "{} embeds the shared chunk once",
                record.name
            );
            for entry in ["fn vs(", "fn fs("] {
                assert_eq!(
                    text.match_indices(entry).count(),
                    1,
                    "{} keeps one {entry}",
                    record.name,
                    entry = entry
                );
            }
        }
    }

    /// The Filament clamps are named consts with the Filament values, the
    /// generated BRDF declares each once and reads it where the literal used
    /// to be, and the CPU clamp the twin shares floors at the const. F2P: drop
    /// a clamp call or change a value and this fails; the twin's furnace then
    /// fails with it.
    #[test]
    fn the_filament_clamps_are_named_consts_the_text_reads() {
        assert_eq!(PERCEPTUAL_ROUGHNESS_FLOOR, 0.089, "roughness floor");
        assert_eq!(ANISOTROPY_FLOOR, 0.001, "anisotropy floor");
        assert_eq!(MIN_HALF_NORMAL, 6.103_515_6e-5, "2^-14 half normal");
        assert_eq!(
            MIN_HALF_NORMAL,
            2f32.powi(-14),
            "the half normal is exactly 2^-14"
        );
        let chunk = brdf_wgsl();
        for (name, value) in [
            ("ROUGHNESS_FLOOR", PERCEPTUAL_ROUGHNESS_FLOOR),
            ("ANISOTROPY_FLOOR", ANISOTROPY_FLOOR),
            ("MIN_HALF_NORMAL", MIN_HALF_NORMAL),
        ] {
            let decl = format!("const {name}: f32 = {value:?};");
            assert_eq!(
                chunk.match_indices(decl.as_str()).count(),
                1,
                "{name} declared once with the const value"
            );
        }
        // The clamp is read, not inlined: the fragment names the const where
        // the literal 0.089 stood, and the Schlick base takes the half normal.
        assert!(chunk.contains("clamp(p, ROUGHNESS_FLOOR, 1.0)"));
        assert!(chunk.contains("pow(max(1.0 - vdot_h, MIN_HALF_NORMAL), 5.0)"));
        let text = compile(&MaterialSet::siege_kit().records()[0]).wgsl;
        assert!(text.contains("clamp(rough, ROUGHNESS_FLOOR, 1.0)"));
        // The floor really clamps: a zero roughness lands on the const.
        assert_eq!(clamp_perceptual_roughness(0.0), PERCEPTUAL_ROUGHNESS_FLOOR);
        assert_eq!(clamp_perceptual_roughness(0.02), PERCEPTUAL_ROUGHNESS_FLOOR);
        assert_eq!(clamp_perceptual_roughness(0.5), 0.5);
        assert_eq!(clamp_perceptual_roughness(1.5), 1.0);
    }

    /// One table, one row per material id: every shipped record maps to
    /// exactly one row, ids are the wire ids in order, the row's lobes and key
    /// come from the same [`Structure::of`] [`compile`] reads, and a lookup by
    /// id returns that row. A record added to a shipped set cannot miss a row.
    #[test]
    fn every_material_id_maps_to_one_table_row() {
        let set = MaterialSet::shipped();
        let table = material_table();
        assert_eq!(table.len(), set.len(), "one row per shipped record");
        let mut seen: Vec<u8> = Vec::with_capacity(table.len());
        for (index, (row, record)) in table.iter().zip(set.records()).enumerate() {
            let structure = Structure::of(record);
            assert_eq!(row.id, u8::try_from(index).expect("shipped ids fit a byte"));
            assert_eq!(row.name, record.name, "wire order");
            assert_eq!(row.lobes, structure.lobes, "{} lobes", row.name);
            assert_eq!(row.textures, structure.textures, "{} textures", row.name);
            assert_eq!(row.key, structure.key(), "{} key", row.name);
            assert!(!seen.contains(&row.id), "id {} appears once", row.id);
            seen.push(row.id);
            assert_eq!(
                material_row(row.id).as_ref(),
                Some(row),
                "id {} maps back to its row",
                row.id
            );
        }
        // Every sim-side material id names a record in the table.
        for id in engine_destruction::material::MaterialId::all() {
            let row = table
                .iter()
                .find(|row| row.name == id.name())
                .unwrap_or_else(|| panic!("{} has a table row", id.name()));
            assert_eq!(row.id, id.as_u8(), "{} keeps its wire id", id.name());
        }
        assert!(material_row(u8::MAX).is_none(), "an unused id has no row");
    }

    #[cfg(feature = "validate")]
    mod naga_checks {
        use super::*;

        #[test]
        fn every_shipped_record_validates() {
            for record in MaterialSet::siege_kit()
                .records()
                .iter()
                .chain(MaterialSet::reference().records())
            {
                let c = compile(record);
                if let Err(e) = validate(&c.wgsl) {
                    panic!("{}: {e}", record.name);
                }
            }
        }

        #[test]
        fn the_uniform_layout_matches_the_packed_floats() {
            for record in MaterialSet::siege_kit().records() {
                let c = compile(record);
                assert_eq!(
                    material_uniform_size(&c.wgsl),
                    Some(u32::try_from(c.uniform.len() * 4).expect("small")),
                    "{}",
                    record.name
                );
            }
        }

        #[test]
        fn a_broken_chunk_is_refused_with_a_line() {
            let mut text = compile(&MaterialSet::siege_kit().records()[0]).wgsl;
            text.push_str("fn broken() -> f32 { return vec3<f32>(1.0); }\n");
            let err = validate(&text).expect_err("type mismatch");
            assert!(!err.message.is_empty());
        }
    }
}
