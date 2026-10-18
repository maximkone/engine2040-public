//! quality: render quality tiers as typed data (GS10 slice 1).
//!
//! Owner: engine-render. Units live on each knob (see [`unit`]).
//! Determinism: pure integer data; [`table_hash`] is FNV-1a over the fixed
//! struct field order, identical on every platform. No tick, clock or float.
//! Consumer: PLF10 game presets read [`knobs`] next slice; this slice only
//! stores the table plus its hash and cut order. WEB8 adds the `Web` tier (a browser tab:
//! single thread, wasm speed, WebGPU default limits), its pick from adapter limits and a
//! `?tier=` query ([`choose_web_tier`]) and Babylon's degrade order ([`WEB_CUT_ORDER`]).

/// Render quality tier, lowest to highest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// Low tier: smallest scale, one cascade, tightest caps.
    Potato,
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
    pub const ALL: [Quality; 4] = [
        Quality::Potato,
        Quality::Web,
        Quality::Comfort,
        Quality::High,
    ];

    /// Lowercase tier name, as the page prints it in `tier=<name>`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Potato => "potato",
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

/// One row of the quality table. All integer data, no clock, no float.
///
/// Units: `render_scale_pct` (%), `shadow_cascades` (count), `shadow_map_px`
/// (px), `gtao` (level), `bloom_half_res` (bool), `taa` (bool), `msaa` (x),
/// `aniso` (x), `mip_bias_x100` (1/100), `stream_pool_mb` (MB),
/// `particle_cap` (count), `decal_cap` (count), `crowd_cap` (cards), `physfx_bodies` (bodies),
/// `physfx_ragdolls` (ragdolls), `physfx_contacts` (pairs), `texture_max_px` (px).
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
}

/// Quality row for a tier. Bias stays at 100 hundredths on every tier in
/// this slice so the accept monotone holds flat.
#[must_use]
pub const fn knobs(q: Quality) -> Knobs {
    match q {
        Quality::Potato => Knobs {
            render_scale_pct: 67,
            shadow_cascades: 1,
            shadow_map_px: 1024,
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
        },
        Quality::Web => Knobs {
            render_scale_pct: 100,
            shadow_cascades: 2,
            shadow_map_px: 1024,
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
        },
        Quality::Comfort => Knobs {
            render_scale_pct: 100,
            shadow_cascades: 3,
            shadow_map_px: 2048,
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
        },
        Quality::High => Knobs {
            render_scale_pct: 100,
            shadow_cascades: 4,
            shadow_map_px: 4096,
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
        },
    }
}

/// Unit string for a knob name. Unknown names return "".
#[must_use]
pub fn unit(knob: &str) -> &'static str {
    match knob {
        "render_scale_pct" => "%",
        "shadow_cascades" => "count",
        "shadow_map_px" => "px",
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

/// What the browser page cuts when the frame misses its budget, first to last: Babylon's
/// SceneOptimizer order (cheap invisible savings first, the picture's own size last). The page
/// takes one step per missed second and prints `tier=<name> cut=<step>` with each.
pub const WEB_CUT_ORDER: [&str; 6] = [
    "merge_draws",
    "shadows",
    "post_and_particles",
    "texture_size",
    "render_targets",
    "render_scale",
];

/// Smallest `maxTextureDimension2D` of a device that gets the `Web` tier or better. WebGPU's
/// default limit is 8192; an adapter below 4096 is a very weak or restricted GPU and gets
/// `Potato`.
pub const WEB_MIN_TEXTURE_2D: u32 = 4096;

/// The tier the page runs and why: `"query"` (the `?tier=` value named a tier), `"limits"`
/// (the adapter's texture limit is under [`WEB_MIN_TEXTURE_2D`]) or `"default"` (the web tier).
/// An unknown `?tier=` value is ignored (`"default"` or `"limits"` decides) so a typo never
/// breaks the page. Pure: the page reads the query string and the device limits and passes them.
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

/// Highest cascade band the live frame shades with: the tier's cascade count less one (the
/// last cascade covers everything beyond it). At least band 0.
#[must_use]
pub fn max_cascade_band(k: &Knobs) -> usize {
    usize::from(k.shadow_cascades.max(1)) - 1
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

/// FNV-1a over the 17 knob fields in struct order (bools as 0/1, gtao as
/// its level byte, integers little-endian). Same tier twice gives the same
/// hash; no map order involved.
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
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_monotone_tiers() {
        let p = knobs(Quality::Potato);
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
    fn costs(k: &Knobs) -> [(&'static str, u64); 17] {
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
        assert_eq!(edges, [1024, 1024, 2048, 4096]);
        assert_eq!(shadow_edge_px(&knobs(Quality::High), 2048), 2048);
        assert_eq!(shadow_edge_px(&knobs(Quality::Web), 0), MIN_SHADOW_EDGE);
    }

    #[test]
    fn quality_cascade_band_cap_is_the_count_less_one() {
        let caps: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| max_cascade_band(&knobs(*q)))
            .collect();
        assert_eq!(caps, [0, 1, 2, 3]);
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
            assert_eq!(gtao_runs(&k, true), q != Quality::Potato, "{q:?}");
            assert!(!gtao_runs(&k, false), "{q:?} with the toggle off");
        }
    }

    #[test]
    fn quality_pool_slots_follow_the_tier_within_the_pool_bounds() {
        let particles: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| particle_pool_slots(&knobs(*q)))
            .collect();
        assert_eq!(particles, [512, 2048, 4096, 8192]);
        let decals: Vec<usize> = Quality::ALL
            .iter()
            .map(|q| decal_pool_slots(&knobs(*q)))
            .collect();
        assert_eq!(
            decals,
            [64, 128, 256, crate::decals::MAX_DECALS],
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
}
