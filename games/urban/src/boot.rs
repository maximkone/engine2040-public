//! Urban-war pilot corner boot through the stock engine-app pipeline.
//!
//! Headless only. Loads `content/urban/pilot.json` (CT56 corner selection
//! as data), boots the unmodified `engine-app` lifecycle with hash-law
//! config, runs a fixed number of headless frames, and returns a boot
//! receipt. Zero engine edits: only the stock app plus core publics,
//! never the arena or game facades. Ordering and
//! extraction never enter sim hashes; they are receipts.

use engine_app::{AppBuilder, AppConfig, FrameCtx, Phase, QualityTier};
use engine_core::TICK_HZ;

/// Pinned pilot file, bound at compile time so boot and data cannot skew.
pub const PILOT_JSON: &str = include_str!("../../../content/urban/pilot.json");

/// Corner this boot is pinned to (CT56.a selection).
pub const EXPECTED_CORNER: &str = "district-corner-01";
/// Variant this boot is pinned to (CT56.a selection).
pub const EXPECTED_VARIANT: &str = "district-corner";
/// Corner actor count (CT56.a probe: 3 actors, 0 findings).
pub const EXPECTED_ACTORS: usize = 3;
/// Cooked LOD0 total from the frozen CT9 intake (CT56.b).
pub const EXPECTED_LOD0: u32 = 5376;
/// Hash-law tick rate; any other value refuses before startup.
pub const EXPECTED_TICK_HZ: u64 = TICK_HZ;

/// One corner actor: placement data only, never geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PilotActor {
    /// Placement name from the corner selection.
    pub name: String,
    /// Frozen CT9 kit asset id.
    pub asset: String,
    /// Center in meters.
    pub x: f64,
    /// Center height in meters (CT9 values reused).
    pub y: f64,
    /// Center depth in meters.
    pub z: f64,
    /// Full extents in meters.
    pub sx: f64,
    /// Full height in meters.
    pub sy: f64,
    /// Full depth in meters.
    pub sz: f64,
}

/// Validated pilot: what the headless boot runs under.
#[derive(Clone, Debug, PartialEq)]
pub struct Pilot {
    /// Deterministic seed in, hashes out.
    pub seed: u64,
    /// Fixed steps per second; always [`TICK_HZ`].
    pub tick_hz: u64,
    /// Headless frames to run after handover.
    pub frames: u64,
    /// Verify ticks the startup frame runs twice.
    pub verify_ticks: u32,
    /// Corner id from the selection.
    pub corner: String,
    /// Placed actors from the selection.
    pub actors: Vec<PilotActor>,
}

/// Headless boot receipt: every number a checker needs, nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootReceipt {
    /// Seed the boot ran under.
    pub seed: u64,
    /// Verify-frame checkpoint hash, agreed twice at startup.
    pub verify_hash: u64,
    /// Ticks in the verify frame.
    pub verify_ticks: u32,
    /// Headless frames run.
    pub frames: u64,
    /// Fixed ticks elapsed through the accumulator.
    pub sim_ticks: u64,
    /// Extraction tick of the last frame.
    pub extraction_tick: u64,
    /// Extraction frame of the last frame.
    pub extraction_frame: u64,
    /// Build hash stamped on every extraction.
    pub build_hash: u64,
    /// Extraction schema version stamped on every extraction.
    pub extraction_schema: u32,
    /// Scene nodes staged (stock boot stages none).
    pub scene_nodes: usize,
    /// Corner actors carried as data.
    pub actors: usize,
    /// Cooked LOD0 total carried as data.
    pub lod0_tris: u32,
    /// Placement findings (probe verdict: 0).
    pub placement_findings: usize,
    /// True when two consecutive boots agree bit-for-bit.
    pub identical_twice: bool,
}

/// Typed boot failure: what refused, and which step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootError {
    /// The pilot file is missing a field or carries a bad value.
    Pilot(String),
    /// The stock config refuses (never half-starts).
    Config(String),
    /// Startup (registries, phase order, verify frame) refuses.
    Startup(String),
    /// A headless frame refuses.
    Frame(String),
    /// Two consecutive boots diverge.
    Mismatch(String),
    /// Shutdown drain refuses.
    Shutdown(String),
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pilot(reason) => write!(f, "pilot refused: {reason}"),
            Self::Config(reason) => write!(f, "config refused: {reason}"),
            Self::Startup(reason) => write!(f, "startup refused: {reason}"),
            Self::Frame(reason) => write!(f, "frame refused: {reason}"),
            Self::Mismatch(reason) => write!(f, "replay diverged: {reason}"),
            Self::Shutdown(reason) => write!(f, "shutdown refused: {reason}"),
        }
    }
}

impl std::error::Error for BootError {}

fn pilot_fail(msg: &str) -> BootError {
    BootError::Pilot(msg.to_string())
}

fn finite(value: f64, what: &str) -> Result<f64, BootError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(pilot_fail(&format!("{what} not finite")))
    }
}

/// Parse plus range-check one pilot file. Refuses before anything starts.
pub fn load_pilot(bytes: &[u8]) -> Result<Pilot, BootError> {
    let text = std::str::from_utf8(bytes).map_err(|_| pilot_fail("not utf-8"))?;
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| pilot_fail(&error.to_string()))?;
    let get = |key: &str| {
        root.get(key)
            .ok_or_else(|| pilot_fail(&format!("missing {key}")))
    };
    if get("game")?.as_str() != Some("urban-war") {
        return Err(pilot_fail("game != urban-war"));
    }
    if get("version")?.as_u64() != Some(1) {
        return Err(pilot_fail("version != 1"));
    }
    if get("corner")?.as_str() != Some(EXPECTED_CORNER) {
        return Err(pilot_fail("corner drift"));
    }
    if get("variant")?.as_str() != Some(EXPECTED_VARIANT) {
        return Err(pilot_fail("variant drift"));
    }
    if get("pipeline")?.as_str() != Some("engine-app") {
        return Err(pilot_fail("pipeline != engine-app"));
    }
    if get("engine_fork")?.as_str() != Some("none") {
        return Err(pilot_fail("engine fork claimed"));
    }
    let runtime = get("runtime")?;
    let seed = runtime
        .get("seed")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| pilot_fail("missing runtime.seed"))?;
    let tick_hz = runtime
        .get("tick_hz")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| pilot_fail("missing runtime.tick_hz"))?;
    if tick_hz != EXPECTED_TICK_HZ {
        return Err(pilot_fail("tick_hz must be 60"));
    }
    let frames = runtime
        .get("frames")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| pilot_fail("missing runtime.frames"))?;
    if frames == 0 || frames > 600 {
        return Err(pilot_fail("frames outside 1..=600"));
    }
    let verify_ticks = runtime
        .get("verify_ticks")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| pilot_fail("missing runtime.verify_ticks"))?;
    let verify_ticks =
        u32::try_from(verify_ticks).map_err(|_| pilot_fail("verify_ticks too large"))?;
    if verify_ticks != engine_app::VERIFY_TICKS {
        return Err(pilot_fail("verify_ticks drift"));
    }
    let cook = get("cook")?;
    let lod0 = cook
        .get("lod0_tris")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| pilot_fail("missing cook.lod0_tris"))?;
    if lod0 != u64::from(EXPECTED_LOD0) {
        return Err(pilot_fail("cook.lod0_tris drift"));
    }
    let placement = get("placement")?;
    if placement
        .get("findings")
        .and_then(serde_json::Value::as_u64)
        != Some(0)
    {
        return Err(pilot_fail("placement findings != 0"));
    }
    let raw_actors = get("actors")?;
    let list = raw_actors
        .as_array()
        .ok_or_else(|| pilot_fail("actors not a list"))?;
    if list.len() != EXPECTED_ACTORS {
        return Err(pilot_fail("actors != 3"));
    }
    let mut actors = Vec::with_capacity(list.len());
    for entry in list {
        let str_field = |key: &str| {
            entry
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| pilot_fail(&format!("actor missing {key}")))
        };
        let num_field = |key: &str| {
            entry
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| pilot_fail(&format!("actor missing {key}")))
        };
        let actor = PilotActor {
            name: str_field("name")?.to_string(),
            asset: str_field("asset")?.to_string(),
            x: finite(num_field("x")?, "x")?,
            y: finite(num_field("y")?, "y")?,
            z: finite(num_field("z")?, "z")?,
            sx: finite(num_field("sx")?, "sx")?,
            sy: finite(num_field("sy")?, "sy")?,
            sz: finite(num_field("sz")?, "sz")?,
        };
        if actor.sx <= 0.0 || actor.sy <= 0.0 || actor.sz <= 0.0 {
            return Err(pilot_fail("actor extents must be positive"));
        }
        actors.push(actor);
    }
    Ok(Pilot {
        seed,
        tick_hz,
        frames,
        verify_ticks,
        corner: EXPECTED_CORNER.to_string(),
        actors,
    })
}

fn noop(_ctx: &mut FrameCtx) {}

fn register_stock(builder: AppBuilder) -> Result<AppBuilder, BootError> {
    builder
        .register(
            "urban_setup",
            engine_app::SystemKind::Setup,
            Phase::Startup,
            noop,
        )
        .and_then(|step| {
            step.register(
                "urban_poll",
                engine_app::SystemKind::Poll,
                Phase::PreUpdate,
                noop,
            )
        })
        .and_then(|step| {
            step.register(
                "urban_step",
                engine_app::SystemKind::MatchStep,
                Phase::Update,
                noop,
            )
        })
        .and_then(|step| {
            step.register(
                "urban_extract",
                engine_app::SystemKind::Extract,
                Phase::PostUpdate,
                noop,
            )
        })
        .map_err(|error| BootError::Config(error.to_string()))
}

/// Boot once: stock config, startup verify, fixed headless frames, drain.
pub fn boot_headless(pilot: &Pilot) -> Result<BootReceipt, BootError> {
    let config = AppConfig {
        seed: pilot.seed,
        tick_hz: pilot.tick_hz,
        quality: QualityTier::Medium,
        ..AppConfig::default()
    };
    config
        .validate()
        .map_err(|error| BootError::Config(error.to_string()))?;
    let dt = config.dt();
    let mut app = register_stock(AppBuilder::new().with_config(config))?
        .build()
        .map_err(|error| BootError::Config(error.to_string()))?;
    let startup = app
        .startup()
        .map_err(|error| BootError::Startup(error.to_string()))?;
    if startup.verify_ticks != pilot.verify_ticks {
        return Err(BootError::Startup("verify_ticks drift".to_string()));
    }
    let mut last = None;
    for _ in 0..pilot.frames {
        let receipt = app
            .run_frame(dt)
            .map_err(|error| BootError::Frame(error.to_string()))?;
        last = Some(receipt.extraction);
    }
    let extraction = last.ok_or_else(|| BootError::Frame("no frame ran".to_string()))?;
    app.shutdown()
        .map_err(|error| BootError::Shutdown(error.to_string()))?;
    Ok(BootReceipt {
        seed: pilot.seed,
        verify_hash: startup.verify_hash,
        verify_ticks: startup.verify_ticks,
        frames: pilot.frames,
        sim_ticks: pilot.frames,
        extraction_tick: extraction.tick,
        extraction_frame: extraction.frame,
        build_hash: extraction.build_hash,
        extraction_schema: extraction.schema_version,
        scene_nodes: startup.scene_nodes,
        actors: pilot.actors.len(),
        lod0_tris: EXPECTED_LOD0,
        placement_findings: 0,
        identical_twice: false,
    })
}

/// Boot twice from the same bytes; divergence names the failure.
pub fn boot_twice_identical(bytes: &[u8]) -> Result<BootReceipt, BootError> {
    let pilot = load_pilot(bytes)?;
    let first = boot_headless(&pilot)?;
    let second = boot_headless(&pilot)?;
    if first != second {
        return Err(BootError::Mismatch("headless receipts diverge".to_string()));
    }
    Ok(BootReceipt {
        identical_twice: true,
        ..first
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_pilot_loads() {
        let pilot = load_pilot(PILOT_JSON.as_bytes()).unwrap();
        assert_eq!(pilot.seed, 2040);
        assert_eq!(pilot.tick_hz, TICK_HZ);
        assert_eq!(pilot.corner, EXPECTED_CORNER);
        assert_eq!(pilot.actors.len(), EXPECTED_ACTORS);
        assert_eq!(pilot.actors[0].name, "insula_a");
        assert_eq!(pilot.actors[1].asset, "market_stall");
        assert_eq!(pilot.actors[2].name, "city_wall");
    }

    #[test]
    fn boot_agrees_twice() {
        let receipt = boot_twice_identical(PILOT_JSON.as_bytes()).unwrap();
        assert_eq!(receipt.seed, 2040);
        assert_eq!(receipt.verify_ticks, engine_app::VERIFY_TICKS);
        assert_eq!(receipt.frames, 2);
        assert_eq!(receipt.sim_ticks, 2);
        assert_eq!(receipt.extraction_tick, 2);
        assert_eq!(receipt.extraction_frame, 2);
        assert_eq!(receipt.actors, EXPECTED_ACTORS);
        assert_eq!(receipt.lod0_tris, EXPECTED_LOD0);
        assert_eq!(receipt.placement_findings, 0);
        assert!(receipt.identical_twice);
    }

    #[test]
    fn bad_pilots_refuse() {
        assert!(load_pilot(b"").is_err());
        assert!(load_pilot(b"{}").is_err());
        let drifted = PILOT_JSON.replace("\"tick_hz\": 60", "\"tick_hz\": 59");
        assert!(load_pilot(drifted.as_bytes()).is_err());
        let forked =
            PILOT_JSON.replace("\"engine_fork\": \"none\"", "\"engine_fork\": \"one hunk\"");
        assert!(load_pilot(forked.as_bytes()).is_err());
    }
}
