//! Typed builder fallback vectors for the game session config.
//!
//! Consumer: packaging and quality review over [`engine_game::GameConfig`]
//! through a typed builder. Data owner: this test owns its builder and its
//! seeded rows; `engine-game` owns [`engine_game::GameConfig`] and its range
//! checks. Units: seed is an opaque `u64`, ticks and fighters are integer
//! counts, extent is meters. Determinism scope: same seed gives the same
//! twenty stacks and verdicts, no IO. Errors: bad rows fall back, never fail.
//!
//! Donor: LukasKalbertodt/confique (MIT)
//! <https://github.com/LukasKalbertodt/confique> `builder.rs`
//! `with_fallback` idea: each layer holds typed `Option` fields and an upper
//! layer fills its gaps from a lower layer. Fresh std-only Rust here; no
//! bytes copied and no new dependency.

use engine_core::splitmix64;
use engine_game::{GameConfig, MAX_TICKS};
use engine_input::MAX_PARTICIPANTS;

/// Fixture seed: same input gives the same twenty stacks.
const FIXTURE_SEED: u64 = 2040;
/// Seeded stacks: each carries exactly one bad row.
const STACKS: usize = 20;
/// Salt step spreading per-stack streams apart.
const STREAM_SALT: u64 = 0x9E37_79B9_7F4A_7C15;
/// Second-stream salt for the env layer.
const ENV_SALT: u64 = 0x517E;

/// One bad slot per stack, round-robin over the four knobs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BadSlot {
    Seed,
    Ticks,
    Fighters,
    Extent,
}

/// Typed partial layer: `None` means absent or bad, so the next lower layer
/// fills it. Pure data.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct Partial {
    seed: Option<u64>,
    ticks: Option<u32>,
    fighters: Option<u8>,
    half_extent_m: Option<f32>,
}

impl Partial {
    /// Fill every `None` field from the fallback layer. Pure.
    #[must_use]
    fn with_fallback(self, fallback: Self) -> Self {
        Self {
            seed: self.seed.or(fallback.seed),
            ticks: self.ticks.or(fallback.ticks),
            fighters: self.fighters.or(fallback.fighters),
            half_extent_m: self.half_extent_m.or(fallback.half_extent_m),
        }
    }
}

/// Typed builder in the confique shape: by-value chaining plus one
/// `with_fallback` merge per layer. Pure.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct Builder {
    inner: Partial,
}

impl Builder {
    /// Empty builder: every field falls back. Pure.
    #[must_use]
    fn new() -> Self {
        Self::default()
    }

    /// Builder prefilled from the canonical defaults. Pure.
    #[must_use]
    fn with_defaults() -> Self {
        let defaults = GameConfig::default();
        Self {
            inner: Partial {
                seed: Some(defaults.seed),
                ticks: Some(defaults.ticks),
                fighters: Some(defaults.fighters),
                half_extent_m: Some(defaults.half_extent_m),
            },
        }
    }

    /// Set the seed knob. Pure.
    #[must_use]
    fn with_seed(mut self, value: u64) -> Self {
        self.inner.seed = Some(value);
        self
    }

    /// Set the ticks knob. Pure.
    #[must_use]
    fn with_ticks(mut self, value: u32) -> Self {
        self.inner.ticks = Some(value);
        self
    }

    /// Set the fighters knob. Pure.
    #[must_use]
    fn with_fighters(mut self, value: u8) -> Self {
        self.inner.fighters = Some(value);
        self
    }

    /// Set the extent knob in meters. Pure.
    #[must_use]
    fn with_half_extent_m(mut self, value_m: f32) -> Self {
        self.inner.half_extent_m = Some(value_m);
        self
    }

    /// Fill gaps from the fallback builder. Pure.
    #[must_use]
    fn with_fallback(mut self, fallback: Self) -> Self {
        self.inner = self.inner.with_fallback(fallback.inner);
        self
    }

    /// Finish into a validated config. Gaps fall back to defaults. Pure.
    #[must_use]
    fn build(self) -> GameConfig {
        let defaults = GameConfig::default();
        GameConfig {
            seed: self.inner.seed.unwrap_or(defaults.seed),
            ticks: self.inner.ticks.unwrap_or(defaults.ticks),
            fighters: self.inner.fighters.unwrap_or(defaults.fighters),
            half_extent_m: self.inner.half_extent_m.unwrap_or(defaults.half_extent_m),
        }
    }
}

/// Parse one seed row; any `u64` goes, else a bad row. Pure.
fn parse_seed(raw: &str) -> Option<u64> {
    raw.parse().ok()
}

/// Parse one ticks row inside `1..=MAX_TICKS`; else a bad row. Pure.
fn parse_ticks(raw: &str) -> Option<u32> {
    raw.parse()
        .ok()
        .filter(|value| (1..=MAX_TICKS).contains(value))
}

/// Parse one fighters row inside `2..=MAX_PARTICIPANTS`; else bad. Pure.
fn parse_fighters(raw: &str) -> Option<u8> {
    raw.parse()
        .ok()
        .filter(|value| (2..=MAX_PARTICIPANTS).contains(value))
}

/// Parse one extent row inside `5.0..=100.0` finite meters; else bad. Pure.
fn parse_extent(raw: &str) -> Option<f32> {
    raw.parse()
        .ok()
        .filter(|value: &f32| value.is_finite() && (5.0..=100.0).contains(value))
}

/// Look up one named row; `None` when absent. Pure.
fn row_of<'x>(rows: &[(&'x str, &'x str)], key: &str) -> Option<&'x str> {
    rows.iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Parse one layer into a builder: bad or missing rows stay `None`. Pure.
fn parse_layer(rows: &[(&str, &str)]) -> Builder {
    Builder {
        inner: Partial {
            seed: row_of(rows, "seed").and_then(parse_seed),
            ticks: row_of(rows, "ticks").and_then(parse_ticks),
            fighters: row_of(rows, "fighters").and_then(parse_fighters),
            half_extent_m: row_of(rows, "half_extent_m").and_then(parse_extent),
        },
    }
}

/// Resolve `defaults < file < env` with per-row fallback. Pure, never fails.
fn resolve(
    defaults: GameConfig,
    file_rows: &[(&str, &str)],
    env_rows: &[(&str, &str)],
) -> GameConfig {
    let defaults_builder = Builder {
        inner: Partial {
            seed: Some(defaults.seed),
            ticks: Some(defaults.ticks),
            fighters: Some(defaults.fighters),
            half_extent_m: Some(defaults.half_extent_m),
        },
    };
    parse_layer(env_rows)
        .with_fallback(parse_layer(file_rows))
        .with_fallback(defaults_builder)
        .build()
}

/// Render one config as a four-knob template. Pure.
fn render_template(config: &GameConfig) -> String {
    format!(
        "seed = {}\nticks = {}\nfighters = {}\nhalf_extent_m = {}\n",
        config.seed, config.ticks, config.fighters, config.half_extent_m
    )
}

/// Default template text: the four canonical knobs. Pure.
fn default_template() -> String {
    render_template(&GameConfig::default())
}

/// Parse a template back through the builder over defaults. Pure.
fn parse_template(template: &str) -> GameConfig {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in template.lines() {
        let mut split = line.splitn(2, '=');
        let key = split.next().unwrap_or("").trim().to_owned();
        let value = split.next().unwrap_or("").trim().to_owned();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        pairs.push((key, value));
    }
    let borrowed: Vec<(&str, &str)> = pairs
        .iter()
        .map(|entry| (entry.0.as_str(), entry.1.as_str()))
        .collect();
    resolve(GameConfig::default(), &borrowed, &[])
}

/// Next draw from a chained `splitmix64` stream. Pure.
fn next_draw(state: &mut u64) -> u64 {
    let out = splitmix64(*state);
    *state = out;
    out
}

/// Valid file values for one stack index. Same index, same values.
fn file_values(index: usize) -> (u64, u32, u8, u8) {
    let slot = u64::try_from(index).unwrap_or(0);
    let mut state = FIXTURE_SEED
        .wrapping_add(slot.wrapping_mul(STREAM_SALT))
        .wrapping_add(1);
    let seed = next_draw(&mut state);
    let ticks = u32::try_from(next_draw(&mut state) % u64::from(MAX_TICKS) + 1)
        .unwrap_or(1)
        .max(1);
    let fighters = u8::try_from(next_draw(&mut state) % u64::from(MAX_PARTICIPANTS - 1) + 2)
        .unwrap_or(2)
        .max(2);
    let extent = u8::try_from(next_draw(&mut state) % 96 + 5)
        .unwrap_or(5)
        .max(5);
    (seed, ticks, fighters, extent)
}

/// Valid env values for one stack index, second stream. Same index, same values.
fn env_values(index: usize) -> (u64, u32, u8, u8) {
    let slot = u64::try_from(index).unwrap_or(0);
    let mut state = FIXTURE_SEED
        .wrapping_add(slot.wrapping_mul(STREAM_SALT))
        .wrapping_add(ENV_SALT);
    let seed = next_draw(&mut state);
    let ticks = u32::try_from(next_draw(&mut state) % u64::from(MAX_TICKS) + 1)
        .unwrap_or(1)
        .max(1);
    let fighters = u8::try_from(next_draw(&mut state) % u64::from(MAX_PARTICIPANTS - 1) + 2)
        .unwrap_or(2)
        .max(2);
    let extent = u8::try_from(next_draw(&mut state) % 96 + 5)
        .unwrap_or(5)
        .max(5);
    (seed, ticks, fighters, extent)
}

/// Bad slot for one stack index: round-robin over the four knobs. Pure.
fn bad_slot_for(index: usize) -> BadSlot {
    match index % 4 {
        0 => BadSlot::Seed,
        1 => BadSlot::Ticks,
        2 => BadSlot::Fighters,
        _ => BadSlot::Extent,
    }
}

/// Bad row text for one slot: never parses into its range. Pure.
fn bad_text_for(slot: BadSlot) -> &'static str {
    match slot {
        BadSlot::Seed => "not-a-seed",
        BadSlot::Ticks => "0",
        BadSlot::Fighters => "1",
        BadSlot::Extent => "4",
    }
}

/// Expected config for one stack: even stacks hold the bad row in env so the
/// knob falls back to file; odd stacks hold it in file so it falls back to
/// defaults. Pure.
fn expected_for(index: usize) -> GameConfig {
    let defaults = GameConfig::default();
    let (file_seed, file_ticks, file_fighters, file_extent) = file_values(index);
    let (env_seed, env_ticks, env_fighters, env_extent) = env_values(index);
    let bad = bad_slot_for(index);
    if index % 2 == 0 {
        GameConfig {
            seed: if bad == BadSlot::Seed {
                file_seed
            } else {
                env_seed
            },
            ticks: if bad == BadSlot::Ticks {
                file_ticks
            } else {
                env_ticks
            },
            fighters: if bad == BadSlot::Fighters {
                file_fighters
            } else {
                env_fighters
            },
            half_extent_m: f32::from(if bad == BadSlot::Extent {
                file_extent
            } else {
                env_extent
            }),
        }
    } else {
        GameConfig {
            seed: if bad == BadSlot::Seed {
                defaults.seed
            } else {
                file_seed
            },
            ticks: if bad == BadSlot::Ticks {
                defaults.ticks
            } else {
                file_ticks
            },
            fighters: if bad == BadSlot::Fighters {
                defaults.fighters
            } else {
                file_fighters
            },
            half_extent_m: if bad == BadSlot::Extent {
                defaults.half_extent_m
            } else {
                f32::from(file_extent)
            },
        }
    }
}

/// Resolve one seeded stack through the typed builder.
fn resolve_stack(index: usize) -> GameConfig {
    let (file_seed, file_ticks, file_fighters, file_extent) = file_values(index);
    let (env_seed, env_ticks, env_fighters, env_extent) = env_values(index);
    let bad = bad_slot_for(index);
    let bad_text = bad_text_for(bad);
    let file_seed_s = file_seed.to_string();
    let file_ticks_s = file_ticks.to_string();
    let file_fighters_s = file_fighters.to_string();
    let file_extent_s = file_extent.to_string();
    let env_seed_s = env_seed.to_string();
    let env_ticks_s = env_ticks.to_string();
    let env_fighters_s = env_fighters.to_string();
    let env_extent_s = env_extent.to_string();
    if index % 2 == 0 {
        let triple = match bad {
            BadSlot::Seed => (
                bad_text,
                env_ticks_s.as_str(),
                env_fighters_s.as_str(),
                env_extent_s.as_str(),
            ),
            BadSlot::Ticks => (
                env_seed_s.as_str(),
                bad_text,
                env_fighters_s.as_str(),
                env_extent_s.as_str(),
            ),
            BadSlot::Fighters => (
                env_seed_s.as_str(),
                env_ticks_s.as_str(),
                bad_text,
                env_extent_s.as_str(),
            ),
            BadSlot::Extent => (
                env_seed_s.as_str(),
                env_ticks_s.as_str(),
                env_fighters_s.as_str(),
                bad_text,
            ),
        };
        let file_rows: [(&str, &str); 4] = [
            ("seed", file_seed_s.as_str()),
            ("ticks", file_ticks_s.as_str()),
            ("fighters", file_fighters_s.as_str()),
            ("half_extent_m", file_extent_s.as_str()),
        ];
        let env_rows: [(&str, &str); 4] = [
            ("seed", triple.0),
            ("ticks", triple.1),
            ("fighters", triple.2),
            ("half_extent_m", triple.3),
        ];
        resolve(GameConfig::default(), &file_rows, &env_rows)
    } else {
        let triple = match bad {
            BadSlot::Seed => (
                bad_text,
                file_ticks_s.as_str(),
                file_fighters_s.as_str(),
                file_extent_s.as_str(),
            ),
            BadSlot::Ticks => (
                file_seed_s.as_str(),
                bad_text,
                file_fighters_s.as_str(),
                file_extent_s.as_str(),
            ),
            BadSlot::Fighters => (
                file_seed_s.as_str(),
                file_ticks_s.as_str(),
                bad_text,
                file_extent_s.as_str(),
            ),
            BadSlot::Extent => (
                file_seed_s.as_str(),
                file_ticks_s.as_str(),
                file_fighters_s.as_str(),
                bad_text,
            ),
        };
        let file_rows: [(&str, &str); 4] = [
            ("seed", triple.0),
            ("ticks", triple.1),
            ("fighters", triple.2),
            ("half_extent_m", triple.3),
        ];
        resolve(GameConfig::default(), &file_rows, &[])
    }
}

#[test]
fn defaults_template_holds_four_knobs() {
    let template = default_template();
    assert!(template.contains("seed = 2040"), "template holds seed");
    assert!(template.contains("ticks = 600"), "template holds ticks");
    assert!(template.contains("fighters = 2"), "template holds fighters");
    assert!(
        template.contains("half_extent_m = 20"),
        "template holds extent"
    );
    assert_eq!(template.lines().count(), 4, "template holds four knobs");
}

#[test]
fn typed_template_round_trips() {
    let built = Builder::new()
        .with_seed(777)
        .with_ticks(1000)
        .with_fighters(4)
        .with_half_extent_m(32.5)
        .with_fallback(Builder::with_defaults())
        .build();
    assert_eq!(built.seed, 777, "typed seed chains");
    assert_eq!(built.ticks, 1000, "typed ticks chains");
    assert_eq!(built.fighters, 4, "typed fighters chains");
    let template = render_template(&built);
    let parsed = parse_template(template.as_str());
    assert_eq!(parsed, built, "template round-trips");
    assert!(parsed.validate().is_ok(), "round-trip validates");
}

#[test]
fn seeded_fallback_stacks_20_of_20() {
    let mut correct = 0;
    for index in 0..STACKS {
        let got = resolve_stack(index);
        let want = expected_for(index);
        assert_eq!(got, want, "stack {index} falls back on its one bad row");
        assert!(got.validate().is_ok(), "stack {index} validates");
        correct += 1;
    }
    assert_eq!(correct, STACKS, "all twenty seeded stacks resolve");
}

#[test]
fn seeded_stacks_are_deterministic() {
    for index in 0..STACKS {
        assert_eq!(
            resolve_stack(index),
            resolve_stack(index),
            "stack {index} repeats"
        );
        assert_eq!(
            expected_for(index),
            expected_for(index),
            "expectation {index} repeats"
        );
    }
    assert_eq!(file_values(3), file_values(3), "file stream repeats");
    assert_eq!(env_values(5), env_values(5), "env stream repeats");
}

#[test]
fn typed_chaining_prefers_top_layer() {
    let top = Builder::new().with_seed(11);
    let mid = Builder::new().with_seed(22).with_ticks(700);
    let bottom = Builder::with_defaults();
    let merged = top.with_fallback(mid).with_fallback(bottom).build();
    assert_eq!(merged.seed, 11, "top seed wins");
    assert_eq!(merged.ticks, 700, "gap falls back to mid");
    assert_eq!(
        merged,
        GameConfig {
            seed: 11,
            ticks: 700,
            fighters: GameConfig::default().fighters,
            half_extent_m: GameConfig::default().half_extent_m,
        },
        "gaps fall back to defaults"
    );
}
