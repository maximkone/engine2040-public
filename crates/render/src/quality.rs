//! quality: render quality tiers as typed data (GS10 slice 1).
//!
//! Owner: engine-render. Units live on each knob (see [`unit()`]).
//! Determinism: pure integer data; [`table_hash`] is FNV-1a over the fixed
//! struct field order, identical on every platform. No tick, clock or float.
//! Consumer: PLF10 game presets read [`knobs`] next slice; this slice only
//! stores the table plus its hash and cut order. WEB8 adds the `Web` tier (a browser tab:
//! single thread, wasm speed, WebGPU default limits), its pick from adapter limits and a
//! `?tier=` query ([`choose_web_tier`]) and Babylon's degrade order ([`WEB_CUT_ORDER`]).
//! BG8 adds the `Mobile` tier (a phone or tablet: one cascade, no AO, the device pixel
//! ratio capped at 2), its pick from the device class and the adapter limits
//! ([`choose_device_tier`]) and the device class hint ([`DeviceClass`]).

/// Render quality tier, lowest to highest (the derived order is the declaration order).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Quality {
    /// Low tier: smallest scale, one cascade, tightest caps.
    Potato,
    /// A phone or tablet (BG8): one cascade, no AO, tight caps, the device pixel ratio
    /// capped at 2 so a 3x panel does not cost three times the pixels.
    Mobile,
    /// A browser tab (WEB8): full scale (the ladder cuts it last), two cascades, half-res AO, no
    /// TAA, web-sized caps.
    Web,
    /// Middle tier: full scale, three cascades, mid caps.
    Comfort,
    /// Top tier: full scale, four cascades, largest caps.
    High,
}

impl Quality {
    /// Every tier, weakest first.
    pub const ALL: [Quality; 5] = [
        Quality::Potato,
        Quality::Mobile,
        Quality::Web,
        Quality::Comfort,
        Quality::High,
    ];

    /// Lowercase tier name, as the page prints it in `tier=<name>`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Potato => "potato",
            Self::Mobile => "mobile",
            Self::Web => "web",
            Self::Comfort => "comfort",
            Self::High => "high",
        }
    }

    /// The tier named `name` (any letter case), `None` for anything else.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|q| q.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// Ambient occlusion level for a tier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gtao {
    /// No ambient occlusion.
    Off,
    /// Half-resolution ambient occlusion.
    Half,
    /// Full-resolution ambient occlusion.
    Full,
}

impl Gtao {
    /// Numeric level: 0, 1, 2. Used by the table hash.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Half => 1,
            Self::Full => 2,
        }
    }
}

/// Sun-shadow compare tap count and pattern for a tier (SW92): fewer taps are
/// fewer shadow-map fetches on every lit pixel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShadowFilter {
    /// 3x3 texel grid: nine compares, the reference picture.
    Pcf3x3,
    /// Four taps on a 1x1 texel box rotated by the 4x4 Bayer angle: nine
    /// compares become four, and the rotation keeps the pattern from banding.
    Rotated4,
    /// Two taps along one fixed diagonal, no per-pixel rotation: the web arm,
    /// where a noisy pattern would survive a missing TAA pass.
    Tap2,
}

impl ShadowFilter {
    /// Every filter, softest first.
    pub const ALL: [ShadowFilter; 3] = [
        ShadowFilter::Pcf3x3,
        ShadowFilter::Rotated4,
        ShadowFilter::Tap2,
    ];

    /// Pipeline-constant word the forward shader switches on.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Pcf3x3 => 0,
            Self::Rotated4 => 1,
            Self::Tap2 => 2,
        }
    }

    /// Compare taps this filter spends per lit pixel.
    #[must_use]
    pub const fn taps(self) -> u32 {
        match self {
            Self::Pcf3x3 => 9,
            Self::Rotated4 => 4,
            Self::Tap2 => 2,
        }
    }
}

/// One row of the quality table. All integer data, no clock, no float.
///
/// Units: `render_scale_pct` (%), `shadow_cascades` (count), `shadow_map_px`
/// (px), `gtao` (level), `bloom_half_res` (bool), `taa` (bool), `msaa` (x),
/// `aniso` (x), `mip_bias_x100` (1/100), `stream_pool_mb` (MB),
/// `particle_cap` (count), `decal_cap` (count), `crowd_cap` (cards), `physfx_bodies` (bodies),
/// `physfx_ragdolls` (ragdolls), `physfx_contacts` (pairs), `texture_max_px` (px),
/// `dpr_cap` (device pixels per CSS pixel).
///
/// The `physfx_*` caps mirror the physical layer's own table (`crates/physfx/src/caps.rs`,
/// `Tier::caps`: bodies, ragdolls, contacts); render cannot depend on physfx, so the numbers
/// are repeated here and the page reads them from this table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Knobs {
    /// Output scale in percent (%).
    pub render_scale_pct: u8,
    /// Shadow cascade count (count).
    pub shadow_cascades: u8,
    /// Shadow map edge length (px).
    pub shadow_map_px: u16,
    /// Sun depth cadence: frame period per cascade (frames). Cascade `i`
    /// re-renders when `frame % period[i] == 0` or its light matrix moved, so
    /// the far cascades may run slower than the near one. `0` reads as `1`,
    /// so a bad row never freezes a map. Never hashed: it picks which frames
    /// draw a map, not the picture the tier asks for.
    pub shadow_update_period: [u8; 3],
    /// Ambient occlusion level (level).
    pub gtao: Gtao,
    /// Bloom at half resolution (bool).
    pub bloom_half_res: bool,
    /// Temporal antialiasing on (bool).
    pub taa: bool,
    /// Multisample count (x).
    pub msaa: u8,
    /// Anisotropy level (x).
    pub aniso: u8,
    /// Mip bias in hundredths (1/100); 100 holds every tier flat.
    pub mip_bias_x100: i16,
    /// Streaming pool size (MB).
    pub stream_pool_mb: u16,
    /// Particle budget (count).
    pub particle_cap: u32,
    /// Decal budget (count).
    pub decal_cap: u32,
    /// Crowd card budget (cards).
    pub crowd_cap: u32,
    /// Physical-layer dynamic bodies alive at once (bodies).
    pub physfx_bodies: u16,
    /// Physical-layer ragdolls alive at once (ragdolls).
    pub physfx_ragdolls: u8,
    /// Physical-layer contact pairs a step may pay for (pairs).
    pub physfx_contacts: u16,
    /// Largest texture edge kept resident; bigger textures load a smaller mip (px).
    pub texture_max_px: u16,
    /// The largest device pixel ratio the tier renders at: how many device pixels
    /// one CSS pixel gets (device pixels per CSS pixel). `1` on the potato tier
    /// (no supersampling), `2` on every other tier: a 3x phone panel would cost
    /// three times the pixels for no visible gain.
    pub dpr_cap: u8,
    /// The look shaders' potato guard: true switches the fp32-only grazing
    /// clamp off and keeps the fp16 floors (bool). Presentation only, never
    /// hashed: it picks a shader constant, not a picture budget.
    pub look_potato_guard: bool,
    /// HDR working-target format this tier pays per pixel (SW6). Not hashed:
    /// every pinned `table_hash` receipt stays, and the tier row already
    /// names the format.
    pub hdr_format: HdrFormat,
    /// Torch sum rate this tier pays (SW5). Not hashed: it picks a shader entry,
    /// not a picture budget.
    pub torch_lighting: TorchLighting,
    /// Bits the output store dithers against (SW53): 8 on the SDR present path,
    /// 10 where the target carries more. Not hashed.
    pub output_dither_px_bits: u8,
}

/// HDR working-target format a tier pays per pixel (SW6).
///
/// The top tier keeps half float, so a tone-mapped highlight above 1.0 and the
/// bloom it feeds survive; the rows that run no bloom (`Potato`, `Mobile`) drop
/// to a packed unorm, and the mid rows keep float headroom in 4 bytes. Units:
/// bytes per pixel. Presentation only, never hashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HdrFormat {
    /// Half float RGBA: 8 bytes per pixel (the top-tier default).
    Rgba16Float,
    /// 10-bit RGB plus 2-bit alpha, unorm: 4 bytes per pixel, clamps at 1.0.
    Rgb10a2Unorm,
    /// 11+11+10-bit float: 4 bytes per pixel, keeps the headroom above 1.0.
    Rg11B10Ufloat,
}

impl HdrFormat {
    /// Short name for the page and logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rgba16Float => "rgba16float",
            Self::Rgb10a2Unorm => "rgb10a2unorm",
            Self::Rg11B10Ufloat => "rg11b10float",
        }
    }

    /// Bytes one pixel costs in this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba16Float => 8,
            Self::Rgb10a2Unorm | Self::Rg11B10Ufloat => 4,
        }
    }
}

/// How the forward pass sums the frame torches (SW5).
///
/// The potato tier pays the 8-light sum once per vertex and interpolates it
/// across the triangle; every other tier keeps the shipped per-fragment loop.
/// Presentation only, never hashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TorchLighting {
    /// Per fragment: the shipped 8-torch loop (`forward.wgsl` `fs`).
    Pixel,
    /// Per vertex: the same sum at each vertex, interpolated
    /// (`shaders/forward_torch_vertex.wgsl` `vs_torch`/`fs_torch`).
    Vertex,
}

impl TorchLighting {
    /// Short name for the page and logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pixel => "pixel",
            Self::Vertex => "vertex",
        }
    }
}

/// Quality row for a tier. Bias stays at 100 hundredths on every tier in
/// this slice so the accept monotone holds flat.
///
/// The two presentation extras (SW6's HDR format, SW5's torch rate) ride
/// [`hdr_format_for_tier`] and [`torch_lighting_for_tier`] instead of a line
/// per row, so the row table stays inside the function cap.
#[must_use]
pub const fn knobs(q: Quality) -> Knobs {
    with_presentation(
        match q {
            Quality::Potato => Knobs {
                render_scale_pct: 67,
                shadow_cascades: 1,
                shadow_map_px: 1024,
                shadow_update_period: [1, 2, 4],
                gtao: Gtao::Off,
                bloom_half_res: false,
                taa: false,
                msaa: 0,
                aniso: 0,
                mip_bias_x100: 100,
                stream_pool_mb: 200,
                particle_cap: 512,
                decal_cap: 64,
                crowd_cap: 1500,
                physfx_bodies: 96,
                physfx_ragdolls: 2,
                physfx_contacts: 256,
                texture_max_px: 512,
                dpr_cap: 1,
                look_potato_guard: true,
            },
            Quality::Mobile => mobile_knobs(),
            Quality::Web => Knobs {
                render_scale_pct: 100,
                shadow_cascades: 2,
                shadow_map_px: 1024,
                shadow_update_period: [1, 2, 2],
                gtao: Gtao::Half,
                bloom_half_res: true,
                taa: false,
                msaa: 0,
                aniso: 2,
                mip_bias_x100: 100,
                stream_pool_mb: 350,
                particle_cap: 2048,
                decal_cap: 128,
                crowd_cap: 3000,
                physfx_bodies: 192,
                physfx_ragdolls: 4,
                physfx_contacts: 512,
                texture_max_px: 1024,
                dpr_cap: 2,
                look_potato_guard: false,
            },
            Quality::Comfort => Knobs {
                render_scale_pct: 100,
                shadow_cascades: 3,
                shadow_map_px: 2048,
                shadow_update_period: [1, 1, 1],
                gtao: Gtao::Half,
                bloom_half_res: true,
                taa: true,
                msaa: 0,
                aniso: 4,
                mip_bias_x100: 100,
                stream_pool_mb: 700,
                particle_cap: 4096,
                decal_cap: 256,
                crowd_cap: 5000,
                physfx_bodies: 384,
                physfx_ragdolls: 8,
                physfx_contacts: 1024,
                texture_max_px: 2048,
                dpr_cap: 2,
                look_potato_guard: false,
            },
            Quality::High => Knobs {
                render_scale_pct: 100,
                shadow_cascades: 4,
                shadow_map_px: 4096,
                shadow_update_period: [1, 1, 1],
                gtao: Gtao::Full,
                bloom_half_res: true,
                taa: true,
                msaa: 4,
                aniso: 8,
                mip_bias_x100: 100,
                stream_pool_mb: 1400,
                particle_cap: 8192,
                decal_cap: 512,
                crowd_cap: 10000,
                physfx_bodies: 768,
                physfx_ragdolls: 16,
                physfx_contacts: 2048,
                texture_max_px: 4096,
                dpr_cap: 2,
                look_potato_guard: false,
            },
        },
        q,
    )
}

/// The two presentation extras over one row: SW6's HDR working format and
/// SW5's torch sum rate. They ride here, not a line per row, so the table stays
/// inside the function cap.
const fn with_presentation(base: Knobs, q: Quality) -> Knobs {
    Knobs {
        hdr_format: hdr_format_for_tier(q),
        torch_lighting: torch_lighting_for_tier(q),
        output_dither_px_bits: dither_bits_for_tier(q),
        ..base
    }
}

/// SW6: the HDR working-target format a tier pays per pixel. The rows that run
/// no bloom take the packed unorm, the mid rows keep float headroom in four
/// bytes, and the top row keeps half float.
#[must_use]
pub const fn hdr_format_for_tier(q: Quality) -> HdrFormat {
    match q {
        Quality::Potato | Quality::Mobile => HdrFormat::Rgb10a2Unorm,
        Quality::Web | Quality::Comfort => HdrFormat::Rg11B10Ufloat,
        Quality::High => HdrFormat::Rgba16Float,
    }
}

/// SW53: the integer bits the 8-bit output store dithers against (8 on the
/// SDR present path, 10 where the target carries more). Not hashed.
pub const fn dither_bits_for_tier(_q: Quality) -> u8 {
    crate::dither::DITHER_BITS
}

/// SW5: the torch sum rate a tier pays. Only the potato row runs it per vertex.
#[must_use]
pub const fn torch_lighting_for_tier(q: Quality) -> TorchLighting {
    match q {
        Quality::Potato => TorchLighting::Vertex,
        Quality::Mobile | Quality::Web | Quality::Comfort | Quality::High => TorchLighting::Pixel,
    }
}

/// The phone row (BG8). It lives beside the table so the tier match stays
/// inside the function cap: one cascade, no AO, no TAA, the device pixel ratio
/// capped at 2, and phone-sized caps under every desktop row.
const fn mobile_knobs() -> Knobs {
    with_presentation(
        Knobs {
            render_scale_pct: 100,
            shadow_cascades: 1,
            shadow_map_px: 1024,
            shadow_update_period: [1, 2, 4],
            gtao: Gtao::Off,
            bloom_half_res: false,
            taa: false,
            msaa: 0,
            aniso: 1,
            mip_bias_x100: 100,
            stream_pool_mb: 250,
            particle_cap: 1024,
            decal_cap: 64,
            crowd_cap: 1500,
            physfx_bodies: 128,
            physfx_ragdolls: 3,
            physfx_contacts: 384,
            texture_max_px: 1024,
            dpr_cap: 2,
            look_potato_guard: false,
        },
        Quality::Mobile,
    )
}

/// Unit string for a knob name. Unknown names return "".
#[must_use]
pub fn unit(knob: &str) -> &'static str {
    match knob {
        "render_scale_pct" => "%",
        "shadow_cascades" => "count",
        "shadow_map_px" => "px",
        "shadow_update_period" => "frames",
        "gtao" => "level",
        "bloom_half_res" => "bool",
        "taa" => "bool",
        "msaa" => "x",
        "aniso" => "x",
        "mip_bias_x100" => "1/100",
        "stream_pool_mb" => "MB",
        "particle_cap" => "count",
        "decal_cap" => "count",
        "crowd_cap" => "cards",
        "physfx_bodies" => "bodies",
        "physfx_ragdolls" => "ragdolls",
        "physfx_contacts" => "pairs",
        "texture_max_px" => "px",
        "dpr_cap" => "device px per CSS px",
        "hdr_format" => "bytes/px",
        "torch_lighting" => "rate",
        _ => "",
    }
}

/// Cut order: what to drop first, fixed.
pub const CUT_ORDER: [&str; 8] = [
    "crowd",
    "particles",
    "aniso",
    "motion_blur",
    "ssr",
    "ao",
    "shadow_cascades",
    "render_scale",
];

/// Every shader file that shades a lit look term, relative to `src/shaders`
/// (QL5). Each one calls [`LOOK_GUARD_FN`] and declares the
/// [`LOOK_GUARD_OVERRIDE`] the preset switches; a new lit file joins this list
/// or `quality_look_guard...` fails. Staged files that no pipeline compiles yet
/// (`forward_maps.wgsl`, `skin.wgsl`) stay out until they join a frame.
pub const LOOK_SHADERS: [&str; 1] = ["forward.wgsl"];

/// The one fp16 and specular-aliasing guard each look shader calls.
pub const LOOK_GUARD_FN: &str = "look_guard";

/// WGSL `override` that carries the potato preset into the look shaders: true
/// switches the fp32-only grazing clamp off and keeps the fp16 floors.
pub const LOOK_GUARD_OVERRIDE: &str = "POTATO_GUARD";

/// True when this row turns the fp32-only clamp off: the potato preset alone,
/// the tier an iGPU or a 2 GB card gets (`tier_rules`).
#[must_use]
pub const fn look_potato_guard(k: &Knobs) -> bool {
    k.look_potato_guard
}

/// Pipeline constants for a tier's forward pipelines: one `override` value,
/// 1.0 on the potato preset and 0.0 everywhere else (the fp32 default).
#[must_use]
pub fn look_guard_constants(k: &Knobs) -> [(&'static str, f64); 1] {
    [(
        LOOK_GUARD_OVERRIDE,
        if look_potato_guard(k) { 1.0 } else { 0.0 },
    )]
}

/// Smallest `maxTextureDimension2D` of a device that gets the `Web` tier or better. WebGPU's
/// default limit is 8192; an adapter below 4096 is a very weak or restricted GPU and gets
/// `Potato`.
pub const WEB_MIN_TEXTURE_2D: u32 = 4096;

/// The tier the page runs and why: `"query"` (the `?tier=` value named a tier), `"limits"`
/// (the adapter's texture limit is under [`WEB_MIN_TEXTURE_2D`]) or `"default"` (the web tier).
/// An unknown `?tier=` value is ignored (`"default"` or `"limits"` decides) so a typo never
/// breaks the page. Pure: the page reads the query string and the device limits and passes them.
///
/// The limits are bucketed: Chrome rounds what an adapter reports so that it cannot tell two cards
/// apart, and one bucket holds weak and strong cards alike. So a limit is a floor and never a
/// measurement: it can only LOWER the tier (an adapter under [`WEB_MIN_TEXTURE_2D`] gets
/// `Potato`), no limit raises it, and the startup probe ([`choose_web_tier_probed`]) can lower it
/// further but never lift it above what the limits allow.
#[must_use]
pub fn choose_web_tier(query: Option<&str>, max_texture_2d: u32) -> (Quality, &'static str) {
    if let Some(tier) = query.and_then(Quality::from_name) {
        return (tier, "query");
    }
    if max_texture_2d < WEB_MIN_TEXTURE_2D {
        return (Quality::Potato, "limits");
    }
    (Quality::Web, "default")
}

/// What the host says about the device it runs on. The hint comes from the
/// platform (a phone build, a tablet, a desktop), never from the GPU: the
/// adapter limits cut the tier down, the class says how far down it starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceClass {
    /// A phone: the tightest budget and the mobile tier.
    Phone,
    /// A tablet: the mobile tier on a bigger screen.
    Tablet,
    /// A desktop: the comfortable tier.
    Desktop,
}

impl DeviceClass {
    /// Every class, weakest device first.
    pub const ALL: [DeviceClass; 3] = [
        DeviceClass::Phone,
        DeviceClass::Tablet,
        DeviceClass::Desktop,
    ];

    /// Lowercase class name, as the startup line prints it in `device=<name>`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Desktop => "desktop",
        }
    }

    /// The class named `name` (any letter case), `None` for anything else.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// Smallest `maxTextureDimension2D` a device must report to get the `Mobile`
/// tier: the mobile row holds a 1024 px texture and a 1024 px shadow map, so
/// an adapter under 2048 cannot hold it and drops to `Potato`.
pub const MOBILE_MIN_TEXTURE_2D: u32 = 2048;

/// The tier a device runs and why: `"phone"`, `"tablet"` or `"desktop"` for
/// the class it is, `"limits"` when the adapter is too weak to hold the
/// mobile row.
///
/// The class sets the ceiling and the limits only cut down (the web tier's
/// rule): a phone gets [`Quality::Mobile`], a tablet the same row, a desktop
/// [`Quality::Comfort`], and any adapter under [`MOBILE_MIN_TEXTURE_2D`]
/// gets [`Quality::Potato`]. Pure: the host reads the class and the device
/// limits and passes them.
#[must_use]
pub fn choose_device_tier(class: DeviceClass, max_texture_2d: u32) -> (Quality, &'static str) {
    if max_texture_2d < MOBILE_MIN_TEXTURE_2D {
        return (Quality::Potato, "limits");
    }
    match class {
        DeviceClass::Phone => (Quality::Mobile, "phone"),
        DeviceClass::Tablet => (Quality::Mobile, "tablet"),
        DeviceClass::Desktop => (Quality::Comfort, "desktop"),
    }
}

/// Adapter text fragments (lowercase) whose match sends the page to `Potato` with the reason
/// `"denylist"` (see [`apply_denylist`]). Empty on purpose: an entry goes in only for an adapter
/// that was measured or reported unusable, and the comment above it names that source (a run id
/// or a bug report). The text is what the page prints as `adapter=`.
pub const WEB_DENYLIST: &[&str] = &[];

/// `pick` (from [`choose_web_tier`]) unless the adapter text contains a fragment of `denylist`
/// (case-insensitive): then `Potato` with the reason `"denylist"`. A `?tier=` query keeps its say,
/// it is the owner asking. An empty fragment matches nothing. An empty list changes nothing.
#[must_use]
pub fn apply_denylist(
    pick: (Quality, &'static str),
    adapter: &str,
    denylist: &[&str],
) -> (Quality, &'static str) {
    let adapter = adapter.to_ascii_lowercase();
    let listed = pick.1 != "query"
        && denylist
            .iter()
            .any(|f| !f.is_empty() && adapter.contains(&f.to_ascii_lowercase()));
    if listed {
        (Quality::Potato, "denylist")
    } else {
        pick
    }
}

/// The value of `tier` in a URL query string (`?a=1&tier=web` or `a=1&tier=web`), `None` when
/// absent. No percent-decoding: tier names are plain letters.
#[must_use]
pub fn tier_from_query(search: &str) -> Option<&str> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("tier="))
}

/// Smallest shadow map edge the live frame builds, texels.
pub const MIN_SHADOW_EDGE: u32 = 256;

/// Edge of the sun shadow map the tier asks for: `shadow_map_px`, within what the adapter can
/// hold (`max_texture_2d`, the device's `maxTextureDimension2D`) and not under
/// [`MIN_SHADOW_EDGE`]. The presenter builds its depth texture at this size.
#[must_use]
pub fn shadow_edge_px(k: &Knobs, max_texture_2d: u32) -> u32 {
    u32::from(k.shadow_map_px)
        .min(max_texture_2d)
        .max(MIN_SHADOW_EDGE)
}

/// Smallest texture edge the cap hands an upload, texels: an adapter that reports no usable
/// limit still gets a 1 px edge, never a zero-size texture.
pub const MIN_TEXTURE_EDGE: u32 = 1;

/// Largest texture edge the tier's uploads keep resident: `texture_max_px`, cut to what the
/// adapter can hold (`max_texture_2d`, the device's `maxTextureDimension2D`) and never under
/// [`MIN_TEXTURE_EDGE`]. The loader caps every upload to this edge, so a 4096 tier value on a
/// 2048 adapter loads the 2048 mip instead of failing the upload. QT9: the page prints the
/// result as `tex_max=`.
#[must_use]
pub fn texture_edge_px(k: &Knobs, max_texture_2d: u32) -> u32 {
    u32::from(k.texture_max_px)
        .min(max_texture_2d)
        .max(MIN_TEXTURE_EDGE)
}

/// Highest cascade band the live frame shades with: the tier's cascade count less one (the
/// last cascade covers everything beyond it). At least band 0.
#[must_use]
pub fn max_cascade_band(k: &Knobs) -> usize {
    usize::from(k.shadow_cascades.max(1)) - 1
}

/// Sun depth cadence for a tier: frame period per cascade, near to far. Cascade `i` re-renders
/// when `frame % period[i] == 0` or its light matrix moved (`shadow_cascade::CascadeCadence`).
/// Presentation only, never hashed.
#[must_use]
pub const fn cascade_periods(k: &Knobs) -> [u8; 3] {
    k.shadow_update_period
}

/// Sun-shadow compare filter for a tier (SW92): 9 taps, 4 rotated taps, or 2.
#[must_use]
pub const fn shadow_filter(q: Quality) -> ShadowFilter {
    match q {
        Quality::Potato | Quality::Mobile => ShadowFilter::Rotated4,
        Quality::Web => ShadowFilter::Tap2,
        Quality::Comfort | Quality::High => ShadowFilter::Pcf3x3,
    }
}

/// Pipeline constants the forward pipeline sets so one shader compiles every
/// filter: `SHADOW_FILTER` carries [`ShadowFilter::as_u32`].
#[must_use]
pub fn shadow_filter_constants(q: Quality) -> [(&'static str, f64); 1] {
    [("SHADOW_FILTER", f64::from(shadow_filter(q).as_u32()))]
}

/// True when the live GTAO pass runs: the look toggle is on and the tier has ambient occlusion.
/// The live kernel is half resolution, so `Half` and `Full` both run it.
#[must_use]
pub fn gtao_runs(k: &Knobs, row_on: bool) -> bool {
    row_on && k.gtao != Gtao::Off
}

/// Slots of the particle pool the tier allows, within the pool's hard bound.
#[must_use]
pub fn particle_pool_slots(k: &Knobs) -> usize {
    usize::try_from(k.particle_cap)
        .unwrap_or(usize::MAX)
        .clamp(1, crate::particles_gpu::POOL_CAP_HARD)
}

/// Slots of the decal pool the tier allows, within the pool's hard bound.
#[must_use]
pub fn decal_pool_slots(k: &Knobs) -> usize {
    usize::try_from(k.decal_cap)
        .unwrap_or(usize::MAX)
        .clamp(1, crate::decals::MAX_DECALS)
}

/// Frames the startup probe draws (WEB8): enough for a stable median, few enough to be done
/// before the page says it is ready.
pub const PROBE_FRAMES: usize = 30;
/// The probe stops after this many milliseconds even when fewer than [`PROBE_FRAMES`] frames
/// were drawn (a very slow adapter must not stall the loader).
pub const PROBE_MAX_MS: f64 = 4000.0;
/// A probe median over this (under 30 frames per second) drops the page to `Potato`.
pub const PROBE_SLOW_MS: f64 = 1000.0 / 30.0;

/// Median of the finite, positive samples (mean of the two middle ones for an even count),
/// `None` when there is none.
#[must_use]
pub fn probe_median_ms(samples: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = samples
        .iter()
        .copied()
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
}

/// [`choose_web_tier`] with the startup probe's median frame time (`None`: no probe ran). A
/// `?tier=` query and the limits keep their say; only the default pick (`Web`) steps down to
/// `Potato` with reason `"probe"` when the median is over [`PROBE_SLOW_MS`]. The fps-miss ladder
/// on the page stays on top of this.
#[must_use]
pub fn choose_web_tier_probed(
    query: Option<&str>,
    max_texture_2d: u32,
    probe_p50_ms: Option<f64>,
) -> (Quality, &'static str) {
    let (tier, why) = choose_web_tier(query, max_texture_2d);
    if why == "default" && probe_p50_ms.is_some_and(|ms| ms > PROBE_SLOW_MS) {
        return (Quality::Potato, "probe");
    }
    (tier, why)
}

/// FNV-1a over the 18 hashed knob fields in struct order (bools as 0/1, gtao
/// as its level byte, integers little-endian). Same tier twice gives the same
/// hash; no map order involved. `look_potato_guard` stays out: it picks a
/// shader constant, not a picture budget.
#[must_use]
pub fn table_hash(q: Quality) -> u64 {
    let k = knobs(q);
    let mut h = engine_core::Fnv64::new();
    h.write_u8(k.render_scale_pct);
    h.write_u8(k.shadow_cascades);
    h.write_u16(k.shadow_map_px);
    h.write_u8(k.gtao.as_u8());
    h.write_bool(k.bloom_half_res);
    h.write_bool(k.taa);
    h.write_u8(k.msaa);
    h.write_u8(k.aniso);
    h.write(&k.mip_bias_x100.to_le_bytes());
    h.write_u16(k.stream_pool_mb);
    h.write_u32(k.particle_cap);
    h.write_u32(k.decal_cap);
    h.write_u32(k.crowd_cap);
    h.write_u16(k.physfx_bodies);
    h.write_u8(k.physfx_ragdolls);
    h.write_u16(k.physfx_contacts);
    h.write_u16(k.texture_max_px);
    h.write_u8(k.dpr_cap);
    h.finish()
}

/// Detail-cull threshold for a tier: a mesh whose projected diameter is under
/// this many thousandths of the screen height never reaches the vertex stage
/// (`gpu_cull::detail_cull_flags_into`). Cheaper tiers drop more, because
/// their pixels are fewer and their vertex budget smaller; the top tier keeps
/// everything at 1/500 of the screen height. `0` would disable the drop.
/// Presentation only, never hashed: it picks which draws reach the vertex
/// stage, not the picture the tier asks for.
#[must_use]
pub const fn detail_cull_permille(q: Quality) -> u16 {
    match q {
        Quality::Potato => 8,
        Quality::Mobile => 7,
        Quality::Web => 6,
        Quality::Comfort => 4,
        Quality::High => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probed tier pick: the function pointer type the test pins.
    ///
    /// A named alias, not a bare signature: the pointer type is complex
    /// enough that `clippy::type_complexity` refused it, and the lint was
    /// debt that blocked the clippy proof of every crate that depends on
    /// render.
    type ProbedTierPick = fn(Option<&str>, u32, Option<f64>) -> (Quality, &'static str);

    #[test]
    fn quality_monotone_tiers() {
        let p = knobs(Quality::Potato);
        let m = knobs(Quality::Mobile);
        let c = knobs(Quality::Comfort);
        let h = knobs(Quality::High);
        assert_eq!(p.render_scale_pct, 67);
        assert_eq!(p.shadow_cascades, 1);
        assert_eq!(p.shadow_map_px, 1024);
        assert_eq!(p.gtao, Gtao::Off);
        assert!(!p.bloom_half_res);
        assert!(!p.taa);
        assert_eq!(p.msaa, 0);
        assert_eq!(p.aniso, 0);
        assert_eq!(p.mip_bias_x100, 100);
        assert_eq!(p.stream_pool_mb, 200);
        assert_eq!(p.particle_cap, 512);
        assert_eq!(p.decal_cap, 64);
        assert_eq!(p.crowd_cap, 1500);
        assert_eq!(
            (p.physfx_bodies, p.physfx_ragdolls, p.physfx_contacts),
            (96, 2, 256)
        );
        assert_eq!(p.texture_max_px, 512);
        assert_eq!(p.dpr_cap, 1);
        assert_eq!(m.render_scale_pct, 100);
        assert_eq!(m.shadow_cascades, 1);
        assert_eq!(m.shadow_map_px, 1024);
        assert_eq!(m.gtao, Gtao::Off);
        assert!(!m.bloom_half_res);
        assert!(!m.taa);
        assert_eq!(m.msaa, 0);
        assert_eq!(m.aniso, 1);
        assert_eq!(m.mip_bias_x100, 100);
        assert_eq!(m.stream_pool_mb, 250);
        assert_eq!(m.particle_cap, 1024);
        assert_eq!(m.decal_cap, 64);
        assert_eq!(m.crowd_cap, 1500);
        assert_eq!(
            (m.physfx_bodies, m.physfx_ragdolls, m.physfx_contacts),
            (128, 3, 384)
        );
        assert_eq!(m.texture_max_px, 1024);
        assert_eq!(m.dpr_cap, 2);
        assert_eq!(c.render_scale_pct, 100);
        assert_eq!(c.shadow_cascades, 3);
        assert_eq!(c.shadow_map_px, 2048);
        assert_eq!(c.gtao, Gtao::Half);
        assert!(c.bloom_half_res);
        assert!(c.taa);
        assert_eq!(c.msaa, 0);
        assert_eq!(c.aniso, 4);
        assert_eq!(c.mip_bias_x100, 100);
        assert_eq!(c.stream_pool_mb, 700);
        assert_eq!(c.particle_cap, 4096);
        assert_eq!(c.decal_cap, 256);
        assert_eq!(c.crowd_cap, 5000);
        assert_eq!(
            (c.physfx_bodies, c.physfx_ragdolls, c.physfx_contacts),
            (384, 8, 1024)
        );
        assert_eq!(c.texture_max_px, 2048);
        assert_eq!(h.render_scale_pct, 100);
        assert_eq!(h.shadow_cascades, 4);
        assert_eq!(h.shadow_map_px, 4096);
        assert_eq!(h.gtao, Gtao::Full);
        assert!(h.bloom_half_res);
        assert!(h.taa);
        assert_eq!(h.msaa, 4);
        assert_eq!(h.aniso, 8);
        assert_eq!(h.mip_bias_x100, 100);
        assert_eq!(h.stream_pool_mb, 1400);
        assert_eq!(h.particle_cap, 8192);
        assert_eq!(h.decal_cap, 512);
        assert_eq!(h.crowd_cap, 10000);
        assert_eq!(
            (h.physfx_bodies, h.physfx_ragdolls, h.physfx_contacts),
            (768, 16, 2048)
        );
        assert_eq!(h.texture_max_px, 4096);
        assert!(p.render_scale_pct <= c.render_scale_pct);
        assert!(c.render_scale_pct <= h.render_scale_pct);
        assert!(p.shadow_cascades <= c.shadow_cascades);
        assert!(c.shadow_cascades <= h.shadow_cascades);
        assert!(p.shadow_map_px <= c.shadow_map_px);
        assert!(c.shadow_map_px <= h.shadow_map_px);
        assert!(p.gtao.as_u8() <= c.gtao.as_u8());
        assert!(c.gtao.as_u8() <= h.gtao.as_u8());
        assert!(p.msaa <= c.msaa);
        assert!(c.msaa <= h.msaa);
        assert!(p.aniso <= c.aniso);
        assert!(c.aniso <= h.aniso);
        assert!(p.mip_bias_x100 <= c.mip_bias_x100);
        assert!(c.mip_bias_x100 <= h.mip_bias_x100);
        assert!(p.stream_pool_mb <= c.stream_pool_mb);
        assert!(c.stream_pool_mb <= h.stream_pool_mb);
        assert!(p.particle_cap <= c.particle_cap);
        assert!(c.particle_cap <= h.particle_cap);
        assert!(p.decal_cap <= c.decal_cap);
        assert!(c.decal_cap <= h.decal_cap);
        assert!(p.crowd_cap <= c.crowd_cap);
        assert!(c.crowd_cap <= h.crowd_cap);
    }

    /// Every cost knob in one list, as numbers, so the ordering test covers a knob by adding
    /// it here and nowhere else.
    fn costs(k: &Knobs) -> [(&'static str, u64); 18] {
        [
            ("render_scale_pct", u64::from(k.render_scale_pct)),
            ("shadow_cascades", u64::from(k.shadow_cascades)),
            ("shadow_map_px", u64::from(k.shadow_map_px)),
            ("gtao", u64::from(k.gtao.as_u8())),
            ("bloom_half_res", u64::from(k.bloom_half_res)),
            ("taa", u64::from(k.taa)),
            ("msaa", u64::from(k.msaa)),
            ("aniso", u64::from(k.aniso)),
            ("mip_bias_x100", u64::from(k.mip_bias_x100.unsigned_abs())),
            ("stream_pool_mb", u64::from(k.stream_pool_mb)),
            ("particle_cap", u64::from(k.particle_cap)),
            ("decal_cap", u64::from(k.decal_cap)),
            ("crowd_cap", u64::from(k.crowd_cap)),
            ("physfx_bodies", u64::from(k.physfx_bodies)),
            ("physfx_ragdolls", u64::from(k.physfx_ragdolls)),
            ("physfx_contacts", u64::from(k.physfx_contacts)),
            ("texture_max_px", u64::from(k.texture_max_px)),
            ("dpr_cap", u64::from(k.dpr_cap)),
        ]
    }

    #[test]
    fn quality_web_tier_sits_between_potato_and_comfort_on_every_cost_knob() {
        let rows: Vec<_> = Quality::ALL.iter().map(|q| costs(&knobs(*q))).collect();
        for pair in rows.windows(2) {
            for (lo, hi) in pair[0].iter().zip(pair[1].iter()) {
                assert_eq!(lo.0, hi.0);
                assert!(
                    lo.1 <= hi.1,
                    "{} is not ordered: {} then {}",
                    lo.0,
                    lo.1,
                    hi.1
                );
            }
        }
        // The web tier is a real step, not a copy: it differs from both neighbours.
        assert_ne!(table_hash(Quality::Potato), table_hash(Quality::Web));
        assert_ne!(table_hash(Quality::Web), table_hash(Quality::Comfort));
        assert_ne!(table_hash(Quality::Web), table_hash(Quality::High));
    }

    #[test]
    fn quality_web_tier_values() {
        let w = knobs(Quality::Web);
        assert_eq!(
            w.render_scale_pct, 100,
            "the ladder cuts the render scale last, so the tier starts at full"
        );
        assert_eq!(w.shadow_cascades, 2);
        assert_eq!(w.shadow_map_px, 1024);
        assert_eq!(w.gtao, Gtao::Half);
        assert!(!w.taa, "no TAA on the web: it needs motion vectors");
        assert_eq!(w.msaa, 0);
        assert_eq!(w.particle_cap, 2048);
        assert_eq!(w.decal_cap, 128);
        assert_eq!(w.texture_max_px, 1024);
        assert_eq!(
            (w.physfx_bodies, w.physfx_ragdolls, w.physfx_contacts),
            (192, 4, 512),
            "the physfx caps repeat crates/physfx/src/caps.rs Tier::Web"
        );
    }

    /// BG8: the mobile tier is picked from the device class and the adapter
    /// limits. The class sets the ceiling, the limits only cut down.
    #[test]
    fn mobile_tier_is_picked_from_the_device_class_and_the_adapter_limits() {
        assert_eq!(
            choose_device_tier(DeviceClass::Phone, 4096),
            (Quality::Mobile, "phone")
        );
        assert_eq!(
            choose_device_tier(DeviceClass::Tablet, 4096),
            (Quality::Mobile, "tablet")
        );
        assert_eq!(
            choose_device_tier(DeviceClass::Desktop, 8192),
            (Quality::Comfort, "desktop")
        );
        // A weak adapter drops the phone to the floor, whatever the class says.
        assert_eq!(
            choose_device_tier(DeviceClass::Phone, MOBILE_MIN_TEXTURE_2D - 1),
            (Quality::Potato, "limits")
        );
        assert_eq!(
            choose_device_tier(DeviceClass::Desktop, 0),
            (Quality::Potato, "limits")
        );
        // The floor itself is the mobile tier's row.
        assert_eq!(
            choose_device_tier(DeviceClass::Phone, MOBILE_MIN_TEXTURE_2D),
            (Quality::Mobile, "phone")
        );
        // The class hint round trips and a typo is nothing.
        for class in DeviceClass::ALL {
            assert_eq!(DeviceClass::from_name(class.name()), Some(class));
            assert_eq!(
                DeviceClass::from_name(&class.name().to_uppercase()),
                Some(class)
            );
        }
        assert_eq!(DeviceClass::from_name(" phone "), Some(DeviceClass::Phone));
        assert_eq!(DeviceClass::from_name("watch"), None);
        assert_eq!(DeviceClass::from_name(""), None);
    }

    /// BG8: every cap of the mobile row is under the desktop row, and the
    /// mobile row is a real step: it caps the device pixel ratio at 2, shades
    /// one cascade and runs no ambient occlusion.
    #[test]
    fn mobile_tier_stays_under_the_desktop_row_on_every_cap() {
        let mobile = knobs(Quality::Mobile);
        let desktop = knobs(Quality::Comfort);
        let low_costs = costs(&mobile);
        let high_costs = costs(&desktop);
        for (low, high) in low_costs.iter().zip(high_costs.iter()) {
            assert_eq!(low.0, high.0);
            assert!(low.1 <= high.1, "{} is over the desktop row", low.0);
        }
        assert_eq!(
            mobile.dpr_cap, 2,
            "a 3x panel would cost three times the pixels"
        );
        assert_eq!(mobile.shadow_cascades, 1);
        assert_eq!(mobile.gtao, Gtao::Off);
        assert!(!gtao_runs(&mobile, true));
        assert_eq!(mobile.texture_max_px, 1024);
        assert_eq!(
            (
                mobile.physfx_bodies,
                mobile.physfx_ragdolls,
                mobile.physfx_contacts
            ),
            (128, 3, 384)
        );
        assert_ne!(table_hash(Quality::Mobile), table_hash(Quality::Potato));
        assert_ne!(table_hash(Quality::Mobile), table_hash(Quality::Web));
    }

    #[test]
    fn quality_names_round_trip_and_unknown_names_are_none() {
        for q in Quality::ALL {
            assert_eq!(Quality::from_name(q.name()), Some(q));
            assert_eq!(Quality::from_name(&q.name().to_uppercase()), Some(q));
        }
        assert_eq!(Quality::from_name(" web "), Some(Quality::Web));
        assert_eq!(Quality::from_name("ultra"), None);
        assert_eq!(Quality::from_name(""), None);
    }

    #[test]
    fn quality_web_pick_reads_the_query_first_then_the_limits() {
        assert_eq!(choose_web_tier(None, 8192), (Quality::Web, "default"));
        assert_eq!(
            choose_web_tier(Some("high"), 8192),
            (Quality::High, "query")
        );
        assert_eq!(
            choose_web_tier(Some("potato"), 16384),
            (Quality::Potato, "query")
        );
        // A query beats the limits (the owner asked for it).
        assert_eq!(
            choose_web_tier(Some("comfort"), 2048),
            (Quality::Comfort, "query")
        );
        // A weak adapter without a query drops to potato.
        assert_eq!(choose_web_tier(None, 2048), (Quality::Potato, "limits"));
        assert_eq!(
            choose_web_tier(None, WEB_MIN_TEXTURE_2D),
            (Quality::Web, "default")
        );
        // A typo is ignored, never a crash.
        assert_eq!(
            choose_web_tier(Some("webb"), 8192),
            (Quality::Web, "default")
        );
        assert_eq!(
            choose_web_tier(Some("webb"), 1024),
            (Quality::Potato, "limits")
        );
    }

    #[test]
    fn quality_tier_query_is_found_among_other_parameters() {
        assert_eq!(tier_from_query("?tier=web"), Some("web"));
        assert_eq!(tier_from_query("?square&tier=high&x=1"), Some("high"));
        assert_eq!(tier_from_query("tier=potato"), Some("potato"));
        assert_eq!(tier_from_query(""), None);
        assert_eq!(tier_from_query("?tiers=web"), None);
    }

    #[test]
    fn quality_web_cut_order_is_babylons_and_ends_with_render_scale() {
        assert_eq!(
            WEB_CUT_ORDER,
            [
                "merge_draws",
                "shadows",
                "post_and_particles",
                "texture_size",
                "render_targets",
                "render_scale"
            ]
        );
    }

    #[test]
    fn quality_units_nonempty() {
        for name in [
            "render_scale_pct",
            "shadow_cascades",
            "shadow_map_px",
            "gtao",
            "bloom_half_res",
            "taa",
            "msaa",
            "aniso",
            "mip_bias_x100",
            "stream_pool_mb",
            "particle_cap",
            "decal_cap",
            "crowd_cap",
            "physfx_bodies",
            "physfx_ragdolls",
            "physfx_contacts",
            "texture_max_px",
            "dpr_cap",
        ] {
            assert!(!unit(name).is_empty(), "unit missing for {name}");
        }
        assert_eq!(unit("render_scale_pct"), "%");
        assert_eq!(unit("shadow_cascades"), "count");
        assert_eq!(unit("shadow_map_px"), "px");
        assert_eq!(unit("gtao"), "level");
        assert_eq!(unit("bloom_half_res"), "bool");
        assert_eq!(unit("taa"), "bool");
        assert_eq!(unit("msaa"), "x");
        assert_eq!(unit("aniso"), "x");
        assert_eq!(unit("mip_bias_x100"), "1/100");
        assert_eq!(unit("stream_pool_mb"), "MB");
        assert_eq!(unit("particle_cap"), "count");
        assert_eq!(unit("decal_cap"), "count");
        assert_eq!(unit("crowd_cap"), "cards");
        assert_eq!(unit("physfx_bodies"), "bodies");
        assert_eq!(unit("physfx_ragdolls"), "ragdolls");
        assert_eq!(unit("physfx_contacts"), "pairs");
        assert_eq!(unit("texture_max_px"), "px");
        assert_eq!(unit("dpr_cap"), "device px per CSS px");
        assert_eq!(unit("no_such_knob"), "");
        assert_eq!(unit(""), "");
    }

    #[test]
    fn quality_cut_order_fixed() {
        assert_eq!(
            CUT_ORDER,
            [
                "crowd",
                "particles",
                "aniso",
                "motion_blur",
                "ssr",
                "ao",
                "shadow_cascades",
                "render_scale"
            ]
        );
    }

    #[test]
    fn quality_shadow_edge_follows_the_tier_within_the_adapter() {
        let edges: Vec<u32> = Quality::ALL
            .iter()
            .map(|q| shadow_edge_px(&knobs(*q), 16384))
            .collect();
        assert_eq!(edges, [1024, 1024, 1024, 2048, 4096]);
        assert_eq!(shadow_edge_px(&knobs(Quality::High), 2048), 2048);
        assert_eq!(shadow_edge_px(&knobs(Quality::Web), 0), MIN_SHADOW_EDGE);
    }

    #[test]
    fn quality_texture_edge_is_the_tier_value_within_the_adapter() {
        let edges: Vec<u32> = Quality::ALL
            .iter()
            .map(|q| texture_edge_px(&knobs(*q), 16384))
            .collect();
        assert_eq!(edges, [512, 1024, 1024, 2048, 4096]);
        assert_eq!(
            texture_edge_px(&knobs(Quality::High), 2048),
            2048,
            "a 4096 tier value on a 2048 adapter loads the 2048 mip"
        );
        assert_eq!(
            texture_edge_px(&knobs(Quality::High), 8192),
            4096,
            "a wider adapter keeps the tier's 4096"
        );
        assert_eq!(
            texture_edge_px(&knobs(Quality::Web), 0),
            MIN_TEXTURE_EDGE,
            "an adapter with no usable limit still gets one texel"
        );
    }

    #[test]
    fn quality_cascade_band_cap_is_the_count_less_one() {
        let caps: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| max_cascade_band(&knobs(*q)))
            .collect();
        assert_eq!(caps, [0, 0, 1, 2, 3]);
        let mut none = knobs(Quality::Potato);
        none.shadow_cascades = 0;
        assert_eq!(
            max_cascade_band(&none),
            0,
            "a zero count still shades band 0"
        );
    }

    #[test]
    fn quality_gtao_runs_only_when_the_tier_has_it_and_the_toggle_is_on() {
        for q in Quality::ALL {
            let k = knobs(q);
            let has_ao = q != Quality::Potato && q != Quality::Mobile;
            assert_eq!(gtao_runs(&k, true), has_ao, "{q:?}");
            assert!(!gtao_runs(&k, false), "{q:?} with the toggle off");
        }
    }

    #[test]
    fn quality_pool_slots_follow_the_tier_within_the_pool_bounds() {
        let particles: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| particle_pool_slots(&knobs(*q)))
            .collect();
        assert_eq!(particles, [512, 1024, 2048, 4096, 8192]);
        let decals: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| decal_pool_slots(&knobs(*q)))
            .collect();
        assert_eq!(
            decals,
            [64, 64, 128, 256, crate::decals::MAX_DECALS],
            "the high tier asks for 512 and the pool holds 256"
        );
    }

    #[test]
    fn quality_probe_median_ignores_bad_samples() {
        assert_eq!(probe_median_ms(&[]), None);
        assert_eq!(probe_median_ms(&[f64::NAN, -1.0, 0.0]), None);
        assert_eq!(probe_median_ms(&[9.0, 1.0, 5.0]), Some(5.0));
        assert_eq!(probe_median_ms(&[4.0, f64::NAN, 2.0, 8.0, 6.0]), Some(5.0));
        assert_eq!(PROBE_FRAMES, 30);
    }

    #[test]
    fn quality_probe_steps_only_the_default_pick_down() {
        let slow = Some(PROBE_SLOW_MS + 1.0);
        let fast = Some(8.0);
        assert_eq!(
            choose_web_tier_probed(None, 8192, slow),
            (Quality::Potato, "probe")
        );
        assert_eq!(
            choose_web_tier_probed(None, 8192, fast),
            (Quality::Web, "default")
        );
        assert_eq!(
            choose_web_tier_probed(None, 8192, Some(PROBE_SLOW_MS)),
            (Quality::Web, "default"),
            "on the budget is not over it"
        );
        assert_eq!(
            choose_web_tier_probed(None, 8192, None),
            (Quality::Web, "default")
        );
        // A query and a weak adapter keep their own reason.
        assert_eq!(
            choose_web_tier_probed(Some("high"), 8192, slow),
            (Quality::High, "query")
        );
        assert_eq!(
            choose_web_tier_probed(None, 2048, slow),
            (Quality::Potato, "limits")
        );
    }

    #[test]
    fn quality_probe_never_lifts_the_tier_above_the_limit_floor() {
        let probes = [
            None,
            Some(1.0),
            Some(PROBE_SLOW_MS),
            Some(PROBE_SLOW_MS + 0.1),
            Some(500.0),
        ];
        for query in [
            None,
            Some("web"),
            Some("high"),
            Some("potato"),
            Some("webb"),
        ] {
            for limit in [
                1024,
                WEB_MIN_TEXTURE_2D - 1,
                WEB_MIN_TEXTURE_2D,
                8192,
                16384,
            ] {
                let floor = choose_web_tier(query, limit).0;
                for probe in probes {
                    let (tier, why) = choose_web_tier_probed(query, limit, probe);
                    assert!(
                        tier <= floor,
                        "{query:?} {limit} {probe:?}: {tier:?} ({why})"
                    );
                }
            }
        }
        // A fast probe above the floor: a weak adapter stays on the floor, a good one on the web tier.
        assert_eq!(
            choose_web_tier_probed(None, 2048, Some(1.0)),
            (Quality::Potato, "limits")
        );
        assert_eq!(
            choose_web_tier_probed(None, 8192, Some(1.0)),
            (Quality::Web, "default")
        );
        // A slow probe below the floor lowers it, and says why.
        assert_eq!(
            choose_web_tier_probed(None, 8192, Some(PROBE_SLOW_MS + 1.0)),
            (Quality::Potato, "probe")
        );
    }

    #[test]
    fn quality_tier_pick_takes_no_adapter_name() {
        // The pick reads the query and the limits and nothing else. The adapter text is a hint
        // for the receipt; only the denylist, a separate step, reads it.
        let pick: fn(Option<&str>, u32) -> (Quality, &'static str) = choose_web_tier;
        let probed: ProbedTierPick = choose_web_tier_probed;
        assert_eq!(pick(None, 8192), (Quality::Web, "default"));
        assert_eq!(probed(None, 8192, None), (Quality::Web, "default"));
    }

    #[test]
    fn quality_tiers_order_from_potato_up() {
        assert!(Quality::Potato < Quality::Mobile && Quality::Mobile < Quality::Web);
        assert!(Quality::Web < Quality::Comfort);
        assert!(Quality::Comfort < Quality::High);
        assert!(Quality::ALL.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn quality_denylist_sends_a_listed_adapter_to_potato_and_an_empty_list_changes_nothing() {
        let web = (Quality::Web, "default");
        assert_eq!(apply_denylist(web, "nvidia/ampere", &[]), web);
        assert_eq!(
            apply_denylist(web, "Vendor/Old_Card_9000", &["old_card"]),
            (Quality::Potato, "denylist"),
            "the match ignores case"
        );
        assert_eq!(apply_denylist(web, "intel/gen-12", &["old_card"]), web);
        assert_eq!(
            apply_denylist(web, "anything", &[""]),
            web,
            "an empty fragment matches nothing"
        );
        // The owner's ?tier= keeps its say; the limits' Potato just gets the denylist reason.
        let asked = (Quality::High, "query");
        assert_eq!(apply_denylist(asked, "old_card", &["old_card"]), asked);
        assert_eq!(
            apply_denylist((Quality::Potato, "limits"), "old_card", &["old_card"]),
            (Quality::Potato, "denylist")
        );
        // The shipped list: lowercase, no empty entry, and every entry really sends its adapter down.
        for fragment in WEB_DENYLIST {
            assert!(!fragment.is_empty() && *fragment == fragment.to_ascii_lowercase());
            assert_eq!(
                apply_denylist(web, fragment, WEB_DENYLIST),
                (Quality::Potato, "denylist")
            );
        }
    }

    /// The number after `field:` in the `Tier::<name> => Caps { ... }` arm of the physfx table.
    fn physfx_cap(table: &str, tier: &str, field: &str) -> usize {
        let arm = table
            .split(&format!("Tier::{tier} => Caps {{"))
            .nth(1)
            .unwrap_or_else(|| panic!("no Tier::{tier} arm in physfx caps.rs"));
        let arm = arm.split('}').next().unwrap_or(arm);
        let after = arm
            .split(&format!("{field}:"))
            .nth(1)
            .unwrap_or_else(|| panic!("no {field} in the {tier} arm"));
        after
            .trim_start()
            .split(',')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("{field} of {tier} is not a number"))
    }

    #[test]
    fn quality_physfx_caps_repeat_the_physfx_table() {
        let table = include_str!("../../physfx/src/caps.rs");
        for q in Quality::ALL {
            let name = match q {
                Quality::Potato => "Potato",
                Quality::Mobile => "Mobile",
                Quality::Web => "Web",
                Quality::Comfort => "Comfort",
                Quality::High => "High",
            };
            let k = knobs(q);
            assert_eq!(
                usize::from(k.physfx_bodies),
                physfx_cap(table, name, "bodies"),
                "{name} bodies"
            );
            assert_eq!(
                usize::from(k.physfx_ragdolls),
                physfx_cap(table, name, "ragdolls"),
                "{name} ragdolls"
            );
            assert_eq!(
                usize::from(k.physfx_contacts),
                physfx_cap(table, name, "contacts"),
                "{name} contacts"
            );
        }
    }

    #[test]
    fn quality_hash_stable_x2() {
        for tier in Quality::ALL {
            assert_eq!(table_hash(tier), table_hash(tier));
        }
        assert_ne!(table_hash(Quality::Potato), table_hash(Quality::High));
    }

    /// The look guard a look shader must carry: the function, the preset
    /// override, the two QL1 floors and the fp32-only grazing clamp, each read
    /// where the literal stood. `None` when the text carries all of it;
    /// otherwise the piece that is missing.
    fn look_guard_gap(name: &str, source: &str) -> Option<String> {
        let need =
            |piece: &str| (!source.contains(piece)).then(|| format!("{name}: missing `{piece}`"));
        need(&format!("fn {LOOK_GUARD_FN}("))
            .or_else(|| need(&format!("override {LOOK_GUARD_OVERRIDE}: bool = false;")))
            // The guard is applied, not just declared: one declaration plus a
            // call from the roughness path.
            .or_else(|| {
                (source.matches(&format!("{LOOK_GUARD_FN}(")).count() < 2)
                    .then(|| format!("{name}: {LOOK_GUARD_FN} is declared but never called"))
            })
            .or_else(|| need("const LOOK_ROUGHNESS_FLOOR: f32 = 0.089;"))
            .or_else(|| need("const LOOK_MIN_HALF_NORMAL: f32 = 6.103515625e-5;"))
            .or_else(|| need("const LOOK_ANALYTIC_GRAZING: f32 = 0.045;"))
            .or_else(|| {
                need(&format!(
                    "clamp({LOOK_GUARD_FN}(p, LOOK_ROUGHNESS_FLOOR, false), 0.0, 1.0)"
                ))
            })
            .or_else(|| {
                need(&format!(
                    "{LOOK_GUARD_FN}(max(dot(n, v), 0.0001), LOOK_MIN_HALF_NORMAL, true)"
                ))
            })
            .or_else(|| need("pow(max(1.0 - vdot_h, LOOK_MIN_HALF_NORMAL), 5.0)"))
    }

    fn look_shader_source(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("shaders")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    /// QL5: every look shader carries the one guard, the potato override and
    /// the floors QL1 named. F2P: strip the guard call (or the override) and
    /// this names the gap; the CQ23 naga gate then refuses the file too.
    #[test]
    fn look_guard_is_declared_and_applied_in_every_look_shader() {
        assert_eq!(
            LOOK_GUARD_FN, "look_guard",
            "the fixed name the shader text must carry"
        );
        for name in LOOK_SHADERS {
            let source = look_shader_source(name);
            assert_eq!(look_guard_gap(name, &source), None, "{name}");
        }
        // The checker really fails: the shipped text with the guard call
        // removed is a gap.
        let stripped = look_shader_source(LOOK_SHADERS[0])
            .replace("look_guard(p, LOOK_ROUGHNESS_FLOOR, false)", "p");
        assert!(
            look_guard_gap(LOOK_SHADERS[0], &stripped).is_some(),
            "a look shader that skips the guard is a gap"
        );
    }

    /// The preset: 1.0 (`POTATO_GUARD` on) on the potato tier alone, 0.0 (the
    /// fp32 default) on the other three.
    #[test]
    fn look_guard_constants_switch_on_the_potato_tier_alone() {
        for tier in Quality::ALL {
            let k = knobs(tier);
            let expected = if tier == Quality::Potato { 1.0 } else { 0.0 };
            assert_eq!(look_potato_guard(&k), tier == Quality::Potato, "{tier:?}");
            assert_eq!(
                look_guard_constants(&k),
                [(LOOK_GUARD_OVERRIDE, expected)],
                "{tier:?}"
            );
        }
    }

    /// QL5 F2P over the CQ23 naga gate: a look shader that lost its guard is
    /// refused by name, so an edit cannot quietly drop it. Needs
    /// `shader-validate`; the text gate above is the default check.
    #[cfg(feature = "shader-validate")]
    #[test]
    fn look_guard_removal_fails_the_naga_gate() {
        let name = LOOK_SHADERS[0];
        let source = look_shader_source(name);
        crate::shader_gate::validate_source(name, &source).expect("the shipped guard validates");
        // The guard's definition goes, its callers stay: no definition in
        // scope. The gate must name the missing identifier.
        let stripped = source.replace("fn look_guard(", "fn look_guard_dropped(");
        let err = crate::shader_gate::validate_source(name, &stripped)
            .expect_err("the gate refuses a look shader without the guard");
        assert!(err.message.contains(LOOK_GUARD_FN), "{err}");
    }

    /// SW6 F2P: the HDR working target is one format per tier. The rows that
    /// run no bloom drop to a packed unorm, the mid rows keep float headroom in
    /// four bytes, and the top row keeps half float; bytes per pixel never fall
    /// as the tier rises.
    #[test]
    fn hdr_format_follows_the_tier() {
        assert_eq!(
            knobs(Quality::Potato).hdr_format,
            HdrFormat::Rgb10a2Unorm,
            "the potato row pays 4 bytes per pixel"
        );
        assert_eq!(
            knobs(Quality::Mobile).hdr_format,
            HdrFormat::Rgb10a2Unorm,
            "the phone row pays 4 bytes per pixel too"
        );
        assert_eq!(
            knobs(Quality::Web).hdr_format,
            HdrFormat::Rg11B10Ufloat,
            "the web row keeps headroom above 1.0 in 4 bytes"
        );
        assert_eq!(
            knobs(Quality::Comfort).hdr_format,
            HdrFormat::Rg11B10Ufloat,
            "the middle tier keeps headroom above 1.0 in 4 bytes"
        );
        assert_eq!(
            knobs(Quality::High).hdr_format,
            HdrFormat::Rgba16Float,
            "the top row keeps half float"
        );
        let mut last = 0_u32;
        for tier in Quality::ALL {
            let bytes = knobs(tier).hdr_format.bytes_per_pixel();
            assert!(
                bytes >= last,
                "{tier:?} pays {bytes} bytes per pixel under a weaker row"
            );
            assert!(bytes == 4 || bytes == 8, "{tier:?} pays {bytes}");
            last = bytes;
        }
        // One word per row, so the page prints a distinct name for each.
        let mut names: Vec<&str> = Quality::ALL
            .iter()
            .map(|tier| knobs(*tier).hdr_format.name())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names, ["rg11b10float", "rgb10a2unorm", "rgba16float"]);
        assert_eq!(unit("hdr_format"), "bytes/px");
    }

    /// SW6 accept numbers: at 1280x720 the HDR target drops from 7.4 MB
    /// (Rgba16Float, 8 bytes per pixel) to 3.7 MB (Rgb10a2Unorm, 4 bytes), so
    /// the potato row pays exactly half the bandwidth of the top row.
    #[test]
    fn potato_hdr_target_is_half_the_bytes() {
        let voxels = 1280_u64 * 720;
        let bytes = |tier: Quality| u64::from(knobs(tier).hdr_format.bytes_per_pixel()) * voxels;
        assert_eq!(bytes(Quality::High), 7_372_800, "7.4 MB at the top row");
        assert_eq!(bytes(Quality::Potato), 3_686_400, "3.7 MB on potato");
        assert_eq!(bytes(Quality::Potato) * 2, bytes(Quality::High));
        assert_eq!(bytes(Quality::Web), 3_686_400, "the mid row is 4 bytes too");
    }
}
