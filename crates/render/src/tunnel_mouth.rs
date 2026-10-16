//! tunnel_mouth: tunnel-mouth exposure split as bind glue.
//!
//! Owner: render lane; consumer is the window forward bind plus the capture
//! bundle (before vs after on the reference GPU), which reads one baked
//! frame holding sunlit sand and the dim gate-tunnel mouth together.
//! The frozen sky bake feeds the day rig ([`crate::arena_day`]) and the
//! fog ramp ([`crate::fog`]); this file adds no sky, fog or pass code,
//! it only binds the three frozen readers into one mouth/sand split.
//! Units: unit-free directions, 0..=1 albedo, stops for the mouth depth,
//! linear radiance triples, meters for the fog bind. Determinism: fixed
//! evaluation order with f32 arithmetic; the same rig baked twice gives
//! bit-identical bytes (pixels stay presentation, never the sim hash).
//! Errors are typed [`TunnelMouthError`].
//!
//! Cost (base tier bars, enforced on boosted release clocks): sky 0.4 ms,
//! height fog 0.2 ms, contact 0.3 ms; this split adds no pass and no
//! transient bytes, it only feeds the existing forward uniforms.
//!
//! ```rust
//! let frame = engine_render::tunnel_mouth::bake_tunnel_mouth().expect("mouth bakes");
//! let sand = engine_render::tunnel_mouth::shade_sand(
//!     [1.0, 0.95, 0.88],
//!     1.0,
//!     1.0,
//!     [0.42, 0.32, 0.20],
//!     frame.sand_ambient(),
//! )
//! .expect("sand shades");
//! let mouth = engine_render::tunnel_mouth::shade_mouth([0.42, 0.32, 0.20], frame.mouth_ambient())
//!     .expect("mouth shades");
//! assert!(sand[0] > mouth[0], "sunlit sand holds above the dim mouth");
//! assert!(mouth.iter().all(|c| *c > 0.0), "mouth keeps ambient, never black");
//! ```

use std::fmt;

use crate::arena_day::{bake_arena_day, ArenaDayRig};
use crate::fog::{ContactConfig, FogParams};
use crate::gtao::LookFrame;

/// Mouth depth in stops under the sunlit sand: the gate-tunnel mouth reads
/// ~2.5 stops dimmer than the open sand under the same EV100 anchor.
pub const TUNNEL_MOUTH_STOPS_DOWN: f32 = 2.5;
/// Mouth dim factor: `2^-2.5`, the occluded-bounce share of the day ambient
/// the mouth keeps while the direct sun stays outside.
pub const TUNNEL_MOUTH_DIM: f32 = 0.176_776_7;
/// Largest mouth depth accepted in stops; deeper would push the mouth floor
/// toward black and break the one-frame hold.
pub const TUNNEL_MOUTH_MAX_STOPS_DOWN: f32 = 8.0;
/// Default look viewport width in pixels for the mouth bind check.
pub const TUNNEL_LOOK_W_PX: u32 = 1280;
/// Default look viewport height in pixels for the mouth bind check.
pub const TUNNEL_LOOK_H_PX: u32 = 720;

/// Tunnel-mouth bind failures. Bad bind input fails loudly; device faults
/// never arise here because the bake is the std-only sky headless twin and
/// the real encode stays behind the `window` submitter.
#[derive(Clone, PartialEq, Debug)]
pub enum TunnelMouthError {
    /// Bind input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for TunnelMouthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "tunnel mouth input: {e}"),
        }
    }
}

impl std::error::Error for TunnelMouthError {}

fn invalid(what: &str) -> TunnelMouthError {
    TunnelMouthError::Invalid(what.into())
}

/// Range-check one scalar in `0..=1`: ndotl, cascade shade and the mouth
/// dim ride this one loop with a static message per site, so the message
/// names the field without formatting on any path.
fn check_unit(v: f32, bad: &'static str) -> Result<(), TunnelMouthError> {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(TunnelMouthError::Invalid(bad.into()));
    }
    Ok(())
}

/// Range-check one linear triple in `lo..=hi`: albedo, sun and ambient
/// triples ride this one loop with a static message per site, so the
/// message names the field without formatting on any path.
fn check_triplet(v: [f32; 3], lo: f32, hi: f32, bad: &'static str) -> Result<(), TunnelMouthError> {
    for c in v {
        if !c.is_finite() || !(lo..=hi).contains(&c) {
            return Err(TunnelMouthError::Invalid(bad.into()));
        }
    }
    Ok(())
}

fn check_factor(name: &str, v: f32, lo: f32, hi: f32) -> Result<(), TunnelMouthError> {
    if !v.is_finite() {
        return Err(invalid(&format!("tunnel mouth {name} is not finite")));
    }
    if !(lo..=hi).contains(&v) {
        return Err(invalid(&format!(
            "tunnel mouth {name} {v} outside {lo}..={hi}"
        )));
    }
    Ok(())
}

/// Mouth gain for `stops` stops down: `2^-stops`, so 0 stops reads 1.0 and
/// deeper mouths shrink toward the ambient floor. Stops must be finite in
/// `0..=TUNNEL_MOUTH_MAX_STOPS_DOWN`.
pub fn mouth_gain_for_stops(stops: f32) -> Result<f32, TunnelMouthError> {
    check_factor("stops", stops, 0.0, TUNNEL_MOUTH_MAX_STOPS_DOWN)?;
    Ok(2.0f32.powf(-stops))
}

/// Preset mouth dim: [`mouth_gain_for_stops`] at
/// [`TUNNEL_MOUTH_STOPS_DOWN`], the share of day ambient and fog color the
/// mouth keeps while the direct sun stays outside.
pub fn mouth_dim() -> f32 {
    2.0f32.powf(-TUNNEL_MOUTH_STOPS_DOWN)
}

/// One baked mouth frame: the frozen day rig plus the mouth readers ramped
/// from the same sky capture, so sand, mouth and fog can never drift apart.
/// The exposure anchor stays shared: the split rides the light (direct sun
/// outside, dim bounce inside), never a second exposure scale.
#[derive(Clone, PartialEq, Debug)]
pub struct TunnelMouthFrame {
    /// Frozen day rig this split rode (sky bake, look readers, fog ramp).
    pub rig: ArenaDayRig,
    /// Mouth dim factor in 0..=1, always [`mouth_dim`] for the preset bake.
    pub mouth_dim: f32,
    /// Mouth ambient in linear HDR: occluded share of the day ambient.
    pub mouth_ambient: [f32; 3],
    /// Mouth fog color in linear HDR: same dim share of the day fog ring.
    pub mouth_fog: [f32; 3],
}

impl TunnelMouthFrame {
    /// Sunlit sand ambient in linear HDR: the full day mean.
    pub fn sand_ambient(&self) -> [f32; 3] {
        self.rig.ibl_ambient
    }

    /// Mouth ambient in linear HDR: the occluded bounce share.
    pub fn mouth_ambient(&self) -> [f32; 3] {
        self.mouth_ambient
    }

    /// Day fog color in linear HDR over the open sand.
    pub fn fog_color(&self) -> [f32; 3] {
        self.rig.fog_color
    }

    /// Mouth fog color in linear HDR inside the gate mouth.
    pub fn mouth_fog(&self) -> [f32; 3] {
        self.mouth_fog
    }

    /// Fog params bound for the open sand: the frozen day-rig ramp.
    pub fn fog(&self) -> &FogParams {
        &self.rig.fog
    }

    /// Fog params for the mouth bind: day extinction with the dimmed mouth
    /// color, so the same air reads darker inside the gate.
    pub fn mouth_fog_params(&self) -> FogParams {
        FogParams {
            fog_color: self.mouth_fog,
            density_per_m: self.rig.fog.density_per_m,
            falloff_m: self.rig.fog.falloff_m,
            base_height_m: self.rig.fog.base_height_m,
            max_distance_m: self.rig.fog.max_distance_m,
            exposure_ev100: self.rig.fog.exposure_ev100,
        }
    }

    /// Look frame for the shared bind: default viewport under the rig sun
    /// and anchor exposure, validated before any device bind.
    pub fn look_frame(&self) -> Result<LookFrame, TunnelMouthError> {
        look_frame(TUNNEL_LOOK_W_PX, TUNNEL_LOOK_H_PX, &self.rig)
    }

    /// Deterministic little-endian bytes: rig bytes, mouth dim, mouth
    /// ambient triple, mouth fog triple. Same preset baked twice gives
    /// bit-identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.rig.to_bytes();
        out.extend_from_slice(&self.mouth_dim.to_le_bytes());
        for c in self.mouth_ambient.iter().chain(self.mouth_fog.iter()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out
    }
}

/// Bake the one mouth frame: frozen day rig in, sand/mouth split out. The
/// mouth readers take the preset [`mouth_dim`] share of the day ambient
/// and fog color. Fails loudly when the frozen bake refuses the preset.
pub fn bake_tunnel_mouth() -> Result<TunnelMouthFrame, TunnelMouthError> {
    let rig = bake_arena_day().map_err(|e| invalid(&format!("tunnel mouth day rig: {e}")))?;
    split_from_rig(&rig, mouth_dim())
}

/// Bind any day rig into a mouth frame at `dim` (0..=1 finite): the same
/// one-bake path the preset takes, so ad-hoc callers cannot fork a second
/// mouth truth. The ambient and fog triples ride the frozen bake, which
/// already pins them finite and nonnegative, so only the dim is checked.
/// Fails loudly on a bad dim.
pub fn split_from_rig(rig: &ArenaDayRig, dim: f32) -> Result<TunnelMouthFrame, TunnelMouthError> {
    check_factor("mouth_dim", dim, 0.0, 1.0)?;
    let mouth_ambient = [
        rig.ibl_ambient[0] * dim,
        rig.ibl_ambient[1] * dim,
        rig.ibl_ambient[2] * dim,
    ];
    let mouth_fog = [
        rig.fog_color[0] * dim,
        rig.fog_color[1] * dim,
        rig.fog_color[2] * dim,
    ];
    Ok(TunnelMouthFrame {
        rig: rig.clone(),
        mouth_dim: dim,
        mouth_ambient,
        mouth_fog,
    })
}

/// Look frame for the shared bind over `rig`: viewport validated against
/// the look bound, exposure anchor and sun carried from the rig so the
/// mouth bind can never drift from the sky bake. The sun and exposure ride
/// the frozen sky-param validation, mapped to a static message so this
/// per-frame bind path allocates nothing.
pub fn look_frame(
    width_px: u32,
    height_px: u32,
    rig: &ArenaDayRig,
) -> Result<LookFrame, TunnelMouthError> {
    if width_px == 0 || height_px == 0 {
        return Err(invalid("tunnel mouth viewport is zero"));
    }
    if width_px > crate::gtao::MAX_LOOK_DIM || height_px > crate::gtao::MAX_LOOK_DIM {
        return Err(invalid("tunnel mouth viewport exceeds the look bound"));
    }
    if rig.params.validate().is_err() {
        return Err(invalid("tunnel mouth day params refused"));
    }
    Ok(LookFrame {
        frame_index: 0,
        viewport_px: [width_px, height_px],
        exposure_ev100: rig.params.exposure_ev100,
        sun_dir: rig.params.sun_dir,
    })
}

/// Shade sunlit sand: `out = albedo * (sun_color * ndotl * shade + ambient)`.
/// The direct term carries the cascade shade the frozen sun path baked;
/// the ambient term is the full day mean. Units: 0..=1 `ndotl`/`shade`/
/// `albedo`, linear triples in and out. Pure arithmetic, no allocation.
pub fn shade_sand(
    sun_color: [f32; 3],
    ndotl: f32,
    cascade_shade: f32,
    albedo: [f32; 3],
    ambient: [f32; 3],
) -> Result<[f32; 3], TunnelMouthError> {
    check_unit(ndotl, "tunnel mouth ndotl outside 0..=1")?;
    check_unit(cascade_shade, "tunnel mouth cascade_shade outside 0..=1")?;
    check_triplet(albedo, 0.0, 1.0, "tunnel mouth albedo outside 0..=1")?;
    check_triplet(
        sun_color,
        0.0,
        f32::MAX,
        "tunnel mouth sun_color is not finite nonnegative",
    )?;
    check_triplet(
        ambient,
        0.0,
        f32::MAX,
        "tunnel mouth ambient is not finite nonnegative",
    )?;
    let direct = ndotl * cascade_shade;
    Ok([
        albedo[0] * (sun_color[0] * direct + ambient[0]),
        albedo[1] * (sun_color[1] * direct + ambient[1]),
        albedo[2] * (sun_color[2] * direct + ambient[2]),
    ])
}

/// Shade the gate-tunnel mouth: `out = albedo * mouth_ambient`. No direct
/// sun reaches inside, so the mouth holds the dim bounce share instead of
/// clipping or going black. Units: 0..=1 `albedo`, linear triples in and
/// out. Pure arithmetic, no allocation.
pub fn shade_mouth(
    albedo: [f32; 3],
    mouth_ambient: [f32; 3],
) -> Result<[f32; 3], TunnelMouthError> {
    check_triplet(albedo, 0.0, 1.0, "tunnel mouth albedo outside 0..=1")?;
    check_triplet(
        mouth_ambient,
        0.0,
        f32::MAX,
        "tunnel mouth mouth_ambient is not finite nonnegative",
    )?;
    Ok([
        albedo[0] * mouth_ambient[0],
        albedo[1] * mouth_ambient[1],
        albedo[2] * mouth_ambient[2],
    ])
}

/// Window wiring over the frozen fog pass: validate the mouth frame plus the
/// contact config, declare fog plus contact plus forward, compile, and
/// check the compiled order runs both fog passes before forward. Returns
/// the compiled graph the window submit records on the device. Fails
/// loudly on bad configs; adds no pass of its own.
pub fn compile_tunnel_fog(
    frame: &TunnelMouthFrame,
    contact: &ContactConfig,
) -> Result<crate::graph::CompiledGraph, TunnelMouthError> {
    frame
        .rig
        .fog
        .validate()
        .map_err(|e| invalid(&format!("tunnel mouth fog: {e}")))?;
    contact
        .validate()
        .map_err(|e| invalid(&format!("tunnel mouth contact: {e}")))?;
    crate::fog::compile_fog_for_forward(&frame.rig.fog, contact)
        .map_err(|e| invalid(&format!("tunnel mouth fog graph: {e}")))
}

/// One window-bind bundle for the forward submitter. Consumer: the window
/// forward bind in `crates/game` staging through `present.rs`, which records
/// this compiled fog order on the device and shades sand and mouth from the
/// same baked frame. UNLOCKS: window forward bind (no in-tree caller yet;
/// this bundle is the single entry the staging path calls).
#[derive(Debug)]
pub struct TunnelMouthBind {
    /// Baked mouth frame both shades ride.
    pub frame: TunnelMouthFrame,
    /// Shared look frame: viewport validated, exposure anchor from the rig.
    pub look: LookFrame,
    /// Frozen fog order the window submit records: fog plus contact plus
    /// forward, checked at bind time so pass drift fails loudly here.
    pub graph: crate::graph::CompiledGraph,
    /// Reference sunlit-sand shade at full sun (`ndotl` 1, cascade 1).
    pub sand_ref: [f32; 3],
    /// Reference gate-mouth shade from the same albedo, dim bounce only.
    pub mouth_ref: [f32; 3],
}

impl TunnelMouthBind {
    /// Exposure anchor both ends share: the rig EV100 carried into the look
    /// frame, so the split never rides a second exposure scale.
    pub fn exposure_ev100(&self) -> f32 {
        self.look.exposure_ev100
    }
}

/// Bake once and bind for the window forward submitter: mouth frame, shared
/// look frame, frozen fog order plus the reference sand/mouth shades proving
/// one frame holds both ends. `sun_color` is linear HDR, `albedo` is the
/// 0..=1 sand albedo both shades share. Fails loudly on a bad viewport, a
/// bad contact config, a refused bake or fog-order drift. Allocates (bake
/// plus graph); the per-face shade path stays allocation-free.
pub fn bind_window_frame(
    width_px: u32,
    height_px: u32,
    sun_color: [f32; 3],
    albedo: [f32; 3],
    contact: &ContactConfig,
) -> Result<TunnelMouthBind, TunnelMouthError> {
    let frame = bake_tunnel_mouth()?;
    let look = look_frame(width_px, height_px, &frame.rig)?;
    let graph = compile_tunnel_fog(&frame, contact)?;
    if !graph.order_names_match(&[crate::fog::FOG_PASS, crate::fog::CONTACT_PASS, "forward"]) {
        return Err(invalid(
            "tunnel mouth fog order drifted from frozen forward",
        ));
    }
    let sand_ref = shade_sand(sun_color, 1.0, 1.0, albedo, frame.sand_ambient())?;
    let mouth_ref = shade_mouth(albedo, frame.mouth_ambient())?;
    Ok(TunnelMouthBind {
        frame,
        look,
        graph,
        sand_ref,
        mouth_ref,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAND_ALBEDO: [f32; 3] = [0.42, 0.32, 0.20];
    const SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.88];

    #[test]
    fn preset_dim_matches_two_and_a_half_stops() {
        let gain = mouth_gain_for_stops(TUNNEL_MOUTH_STOPS_DOWN).expect("stops map");
        assert!(
            (gain - TUNNEL_MOUTH_DIM).abs() < 1e-6,
            "const {TUNNEL_MOUTH_DIM} quotes 2^-2.5, got {gain}"
        );
        assert!(
            (mouth_dim() - gain).abs() < 1e-9,
            "preset dim rides the stops map"
        );
        assert_eq!(
            mouth_gain_for_stops(0.0).expect("zero stops"),
            1.0,
            "open sand keeps full light"
        );
    }

    #[test]
    fn bake_twice_gives_identical_bytes() {
        let a = bake_tunnel_mouth().expect("first bake holds");
        let b = bake_tunnel_mouth().expect("second bake holds");
        assert_eq!(a.to_bytes(), b.to_bytes(), "mouth bytes x2");
        assert!(
            (a.mouth_dim - TUNNEL_MOUTH_DIM).abs() < 1e-6,
            "preset dim pinned"
        );
    }

    #[test]
    fn mouth_readers_ride_the_same_backing() {
        let frame = bake_tunnel_mouth().expect("mouth bakes");
        for c in 0..3 {
            let want_ambient = frame.rig.ibl_ambient[c] * frame.mouth_dim;
            let want_fog = frame.rig.fog_color[c] * frame.mouth_dim;
            assert!(
                (frame.mouth_ambient[c] - want_ambient).abs() < 1e-6,
                "mouth ambient rides the day mean"
            );
            assert!(
                (frame.mouth_fog[c] - want_fog).abs() < 1e-6,
                "mouth fog rides the day ring"
            );
            assert!(
                frame.mouth_ambient[c] < frame.rig.ibl_ambient[c],
                "mouth stays dimmer than sand"
            );
        }
        assert!(
            frame.mouth_ambient.iter().all(|c| *c > 0.0),
            "mouth keeps ambient, never black"
        );
        assert_eq!(
            frame.mouth_fog_params().density_per_m,
            frame.rig.fog.density_per_m,
            "mouth keeps day extinction"
        );
    }

    #[test]
    fn one_frame_holds_sand_and_mouth() {
        let frame = bake_tunnel_mouth().expect("mouth bakes");
        let sand = shade_sand(SUN_COLOR, 1.0, 1.0, SAND_ALBEDO, frame.sand_ambient())
            .expect("sand shades");
        let mouth = shade_mouth(SAND_ALBEDO, frame.mouth_ambient()).expect("mouth shades");
        for c in 0..3 {
            assert!(sand[c] > mouth[c], "sand holds above the mouth");
            assert!(mouth[c] > 0.0, "mouth never black");
            assert!(sand[c].is_finite() && mouth[c].is_finite(), "both finite");
        }
    }

    #[test]
    fn bad_inputs_fail_loudly() {
        assert!(mouth_gain_for_stops(f32::NAN).is_err(), "NaN stops refused");
        assert!(
            mouth_gain_for_stops(-1.0).is_err(),
            "negative stops refused"
        );
        assert!(
            mouth_gain_for_stops(TUNNEL_MOUTH_MAX_STOPS_DOWN + 1.0).is_err(),
            "mouth deeper than the hold refused"
        );
        assert!(
            shade_sand(SUN_COLOR, 2.0, 1.0, SAND_ALBEDO, [0.2; 3]).is_err(),
            "ndotl above 1 refused"
        );
        assert!(
            shade_sand(SUN_COLOR, 1.0, 1.0, [2.0, 0.3, 0.2], [0.2; 3]).is_err(),
            "albedo above 1 refused"
        );
        assert!(
            shade_mouth([f32::NAN, 0.3, 0.2], [0.2; 3]).is_err(),
            "NaN albedo refused"
        );
        let rig = bake_arena_day().expect("rig bakes");
        assert!(split_from_rig(&rig, 1.5).is_err(), "dim above 1 refused");
        assert!(look_frame(0, 64, &rig).is_err(), "zero viewport refused");
        assert!(
            look_frame(crate::gtao::MAX_LOOK_DIM + 1, 64, &rig).is_err(),
            "viewport past the look bound refused"
        );
    }

    #[test]
    fn frozen_fog_graph_compiles_in_order() {
        let frame = bake_tunnel_mouth().expect("mouth bakes");
        let contact = ContactConfig::small();
        let compiled = compile_tunnel_fog(&frame, &contact).expect("fog compiles");
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [crate::fog::FOG_PASS, crate::fog::CONTACT_PASS, "forward"],
            "frozen fog passes run before forward"
        );
    }

    #[test]
    fn split_shading_allocates_nothing() {
        let frame = bake_tunnel_mouth().expect("mouth bakes");
        let ambient = frame.sand_ambient();
        let mouth = frame.mouth_ambient();
        let (sums, heap) = engine_core::alloc_probe::measure(|| {
            let mut acc = [0.0f32; 3];
            for i in 0..64 {
                let n = i as f32 / 63.0;
                let sand =
                    shade_sand(SUN_COLOR, n, 1.0, SAND_ALBEDO, ambient).expect("sand shades");
                let dim = shade_mouth(SAND_ALBEDO, mouth).expect("mouth shades");
                for c in 0..3 {
                    acc[c] += sand[c] + dim[c];
                }
            }
            acc
        });
        std::hint::black_box(sums);
        assert_eq!(heap.calls, 0, "per-face split shades nothing");
    }

    #[test]
    fn window_bind_holds_sand_and_mouth() {
        let contact = ContactConfig::small();
        let bind = bind_window_frame(
            TUNNEL_LOOK_W_PX,
            TUNNEL_LOOK_H_PX,
            SUN_COLOR,
            SAND_ALBEDO,
            &contact,
        )
        .expect("window bind holds");
        assert_eq!(
            bind.exposure_ev100(),
            bind.frame.rig.params.exposure_ev100,
            "split shares one exposure anchor"
        );
        assert_eq!(
            bind.look.viewport_px,
            [TUNNEL_LOOK_W_PX, TUNNEL_LOOK_H_PX],
            "look frame carries the window viewport"
        );
        for c in 0..3 {
            assert!(
                bind.sand_ref[c] > bind.mouth_ref[c],
                "sand holds above mouth"
            );
            assert!(bind.mouth_ref[c] > 0.0, "mouth never black");
            assert!(
                bind.sand_ref[c].is_finite() && bind.mouth_ref[c].is_finite(),
                "both ends finite"
            );
        }
        assert!(
            (bind.frame.mouth_dim - TUNNEL_MOUTH_DIM).abs() < 1e-6,
            "bind rides the preset dim"
        );
    }

    #[test]
    fn window_bind_refuses_bad_inputs() {
        let contact = ContactConfig::small();
        assert!(
            bind_window_frame(0, TUNNEL_LOOK_H_PX, SUN_COLOR, SAND_ALBEDO, &contact).is_err(),
            "zero width refused"
        );
        assert!(
            bind_window_frame(
                crate::gtao::MAX_LOOK_DIM + 1,
                TUNNEL_LOOK_H_PX,
                SUN_COLOR,
                SAND_ALBEDO,
                &contact
            )
            .is_err(),
            "viewport past the look bound refused"
        );
        let bad_contact = ContactConfig {
            radius_px: 0.0,
            ..ContactConfig::small()
        };
        assert!(
            bind_window_frame(
                TUNNEL_LOOK_W_PX,
                TUNNEL_LOOK_H_PX,
                SUN_COLOR,
                SAND_ALBEDO,
                &bad_contact
            )
            .is_err(),
            "zero contact radius refused"
        );
        assert!(
            bind_window_frame(
                TUNNEL_LOOK_W_PX,
                TUNNEL_LOOK_H_PX,
                [f32::NAN, 0.95, 0.88],
                SAND_ALBEDO,
                &contact
            )
            .is_err(),
            "NaN sun refused"
        );
        assert!(
            bind_window_frame(
                TUNNEL_LOOK_W_PX,
                TUNNEL_LOOK_H_PX,
                SUN_COLOR,
                [2.0, 0.32, 0.20],
                &contact
            )
            .is_err(),
            "albedo above 1 refused"
        );
    }
}
