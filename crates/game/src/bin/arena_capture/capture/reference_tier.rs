//! reference_tier: the reference-set tier flag and its headless capture run.
//! Owner: B8. Units: pixels. Determinism: the fixed seed and camera path; the
//! pixels are evidence only and never enter a sim hash.

use std::path::Path;

use engine_game::break_window::BreakScene;
use engine_game::windowed::CameraPath;
use engine_game::{App, GameConfig, HUMAN_SLOT};
use engine_input::FighterId;
use engine_render::present::Presenter;
use engine_render::quality::{self, Quality};

use super::{
    reference_config, reference_fragment_json, reference_stage, reference_tick, KitDraws,
    ReferenceArgs, ReferenceShot, REFERENCE_FRAGMENT, REFERENCE_POTATO_SIZE, REFERENCE_SIZE,
    REFERENCE_TICKS,
};

/// The tiers the arena binary shoots. The browser tier is drawn by
/// `tools/reference-web.mjs`, never by this offscreen twin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceTier {
    /// The desktop window canvas, drawn at the presenter's default quality.
    Native,
    /// The low tier canvas, drawn under [`Quality::Potato`].
    Potato,
}

impl ReferenceTier {
    /// Parse a `--reference-tier` value; an unknown name is refused by the
    /// caller, never silently treated as native.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "native" => Some(Self::Native),
            "potato" => Some(Self::Potato),
            _ => None,
        }
    }

    /// Manifest tier name and the folder the shots are filed in.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Potato => "potato",
        }
    }

    /// Capture canvas, pixels: the manifest's declared size.
    pub(crate) const fn size(self) -> (u32, u32) {
        match self {
            Self::Native => REFERENCE_SIZE,
            Self::Potato => REFERENCE_POTATO_SIZE,
        }
    }

    /// Quality tier applied before the shot is staged. Native keeps the
    /// presenter's default, so its existing bytes stay equal; potato applies
    /// the low tier's shadow, particle, decal and texture caps.
    pub(crate) const fn quality(self) -> Option<Quality> {
        match self {
            Self::Native => None,
            Self::Potato => Some(Quality::Potato),
        }
    }
}

/// Reference-set mode: the ten captures of one tier headless on the offscreen
/// twin, one per [`REFERENCE_TICKS`] offset on the fixed seed-2040 `arena600`
/// orbit. Native draws at 1280x720 with the presenter's default quality;
/// potato draws at 320x180 under the low tier's knobs. Real frames only: a
/// machine without an adapter refuses loudly instead of falling back to the
/// synth pixels or the recipe painter.
pub(crate) fn run_reference_mode(config: &GameConfig, reference: &ReferenceArgs) -> i32 {
    let tier = reference.tier;
    let (width, height) = tier.size();
    let camera = CameraPath::Arena600;
    let config = reference_config(config);
    let mut presenter = match reference_presenter(tier) {
        Ok(presenter) => presenter,
        Err(code) => return code,
    };
    let kit_draws = match reference_stage(&mut presenter, &config, tier.size()) {
        Ok(kit_draws) => kit_draws,
        Err(err) => {
            tell!("arena reference: {err}");
            return 1;
        }
    };
    let mut app = match App::new(config) {
        Ok(app) => app,
        Err(err) => {
            tell!("arena reference: {err}");
            return 1;
        }
    };
    let mut crate_scene = BreakScene::new(&app.live_match().config());
    let Some(human) = FighterId::new(HUMAN_SLOT) else {
        tell!("arena reference: fighter slot");
        return 1;
    };
    let dir = reference.dir.join(tier.name());
    if let Err(err) = std::fs::create_dir_all(&dir) {
        tell!("arena reference: dir {}: {err}", dir.display());
        return 2;
    }
    let (shots, adapter, driver) = match reference_shots(
        &mut presenter,
        &mut app,
        &mut crate_scene,
        &kit_draws,
        human,
        tier,
        &dir,
    ) {
        Ok(shots) => shots,
        Err(err) => {
            tell!("arena reference: {err}");
            return 1;
        }
    };
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    let json = reference_fragment_json(
        tier.name(),
        camera.id(),
        config.seed,
        width,
        height,
        &adapter,
        &driver,
        &command,
        &shots,
    );
    let fragment = dir.join(REFERENCE_FRAGMENT);
    if let Err(err) = std::fs::write(&fragment, json.as_bytes()) {
        tell!("arena reference: {}: {err}", fragment.display());
        return 2;
    }
    say!(
        "arena reference: tier={} shots={} camera={} seed={} size={width}x{height} adapter=\"{adapter}\" driver=\"{driver}\" fragment=\"{}\"",
        tier.name(),
        shots.len(),
        camera.id(),
        config.seed,
        fragment.display()
    );
    0
}

/// Bring up the offscreen twin and apply the tier's quality before any texture
/// is staged. A machine with no adapter is a loud refusal, never a fake frame.
fn reference_presenter(tier: ReferenceTier) -> Result<Presenter, i32> {
    let mut presenter = match Presenter::init_offscreen_with_preset(false) {
        Ok(presenter) => presenter,
        Err(engine_render::present::PresentError::NoAdapter) => {
            tell!(
                "arena reference: no wgpu adapter on this machine; the set is drawn by a real GPU only"
            );
            return Err(1);
        }
        Err(err) => {
            tell!("arena reference: {err}");
            return Err(1);
        }
    };
    if let Some(quality) = tier.quality() {
        presenter.apply_quality(&quality::knobs(quality));
    }
    Ok(presenter)
}

/// Advance the fixed tick and keep the ten camera-path shots, in order.
fn reference_shots(
    presenter: &mut Presenter,
    app: &mut App,
    crate_scene: &mut Option<BreakScene>,
    kit_draws: &KitDraws,
    human: FighterId,
    tier: ReferenceTier,
    dir: &Path,
) -> Result<(Vec<ReferenceShot>, String, String), engine_game::windowed::WinError> {
    let last = REFERENCE_TICKS[REFERENCE_TICKS.len() - 1];
    let mut shots = Vec::with_capacity(REFERENCE_TICKS.len());
    let mut adapter = String::new();
    let mut driver = String::new();
    for tick in 0..=last {
        if let Some((shot, frame_adapter, frame_driver)) = reference_tick(
            presenter,
            app,
            crate_scene,
            kit_draws,
            human,
            tier,
            dir,
            tick,
        )? {
            adapter = frame_adapter;
            driver = frame_driver;
            shots.push(shot);
        }
    }
    Ok((shots, adapter, driver))
}
