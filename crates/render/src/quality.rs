//! quality: render quality tiers as typed data (GS10 slice 1).
//!
//! Owner: engine-render. Units live on each knob (see [`unit`]).
//! Determinism: pure integer data; [`table_hash`] is FNV-1a over the fixed
//! struct field order, identical on every platform. No tick, clock or float.
//! Consumer: PLF10 game presets read [`knobs`] next slice; this slice only
//! stores the table plus its hash and cut order.

/// Render quality tier, lowest to highest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// Low tier: smallest scale, one cascade, tightest caps.
    Potato,
    /// Middle tier: full scale, three cascades, mid caps.
    Comfort,
    /// Top tier: full scale, four cascades, largest caps.
    High,
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
/// `particle_cap` (count), `decal_cap` (count), `crowd_cap` (cards).
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

/// FNV-1a over the 13 knob fields in struct order (bools as 0/1, gtao as
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
        assert_eq!(p.bloom_half_res, false);
        assert_eq!(p.taa, false);
        assert_eq!(p.msaa, 0);
        assert_eq!(p.aniso, 0);
        assert_eq!(p.mip_bias_x100, 100);
        assert_eq!(p.stream_pool_mb, 200);
        assert_eq!(p.particle_cap, 512);
        assert_eq!(p.decal_cap, 64);
        assert_eq!(p.crowd_cap, 1500);
        assert_eq!(c.render_scale_pct, 100);
        assert_eq!(c.shadow_cascades, 3);
        assert_eq!(c.shadow_map_px, 2048);
        assert_eq!(c.gtao, Gtao::Half);
        assert_eq!(c.bloom_half_res, true);
        assert_eq!(c.taa, true);
        assert_eq!(c.msaa, 0);
        assert_eq!(c.aniso, 4);
        assert_eq!(c.mip_bias_x100, 100);
        assert_eq!(c.stream_pool_mb, 700);
        assert_eq!(c.particle_cap, 4096);
        assert_eq!(c.decal_cap, 256);
        assert_eq!(c.crowd_cap, 5000);
        assert_eq!(h.render_scale_pct, 100);
        assert_eq!(h.shadow_cascades, 4);
        assert_eq!(h.shadow_map_px, 4096);
        assert_eq!(h.gtao, Gtao::Full);
        assert_eq!(h.bloom_half_res, true);
        assert_eq!(h.taa, true);
        assert_eq!(h.msaa, 4);
        assert_eq!(h.aniso, 8);
        assert_eq!(h.mip_bias_x100, 100);
        assert_eq!(h.stream_pool_mb, 1400);
        assert_eq!(h.particle_cap, 8192);
        assert_eq!(h.decal_cap, 512);
        assert_eq!(h.crowd_cap, 10000);
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
    fn quality_hash_stable_x2() {
        for tier in [Quality::Potato, Quality::Comfort, Quality::High] {
            assert_eq!(table_hash(tier), table_hash(tier));
        }
        assert_ne!(table_hash(Quality::Potato), table_hash(Quality::High));
    }
}
