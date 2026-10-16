// Donor: bnjjj/twelf, MIT, https://github.com/bnjjj/twelf
// Fresh local twelf-style prefixed env-override layer for game config.
// Layer precedence: defaults < file map < prefixed-env map. Per-field
// env/default declaration below; resolution is per field with fall-through.

use std::collections::HashMap;

/// Precedence layer, ordered low to high: defaults < file < prefixed env.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Defaults,
    File,
    PrefixedEnv,
}

/// Env prefix isolating game config from the rest of the process env.
const ENV_PREFIX: &str = "GAME_";

// Per-field declaration: default value, file-map key, prefixed env key.
const TICK_HZ_DEFAULT: u32 = 60;
const TICK_HZ_FILE_KEY: &str = "tick_hz";
const TICK_HZ_ENV_KEY: &str = "GAME_TICK_HZ";

const ARENA_NAME_DEFAULT: &str = "arena";
const ARENA_NAME_FILE_KEY: &str = "arena_name";
const ARENA_NAME_ENV_KEY: &str = "GAME_ARENA_NAME";

const MAX_FIGHTERS_DEFAULT: u32 = 2;
const MAX_FIGHTERS_FILE_KEY: &str = "max_fighters";
const MAX_FIGHTERS_ENV_KEY: &str = "GAME_MAX_FIGHTERS";

/// Fresh local game config struct (intentionally not layered_extract).
#[derive(Debug, Clone, PartialEq, Eq)]
struct GameConfig {
    tick_hz: u32,
    arena_name: String,
    max_fighters: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            tick_hz: TICK_HZ_DEFAULT,
            arena_name: ARENA_NAME_DEFAULT.to_owned(),
            max_fighters: MAX_FIGHTERS_DEFAULT,
        }
    }
}

fn parse_u32_layered(
    file: &HashMap<String, String>,
    env: &HashMap<String, String>,
    file_key: &str,
    env_key: &str,
    default: u32,
) -> u32 {
    if let Some(raw) = env.get(env_key) {
        if let Ok(v) = raw.parse::<u32>() {
            return v;
        }
    }
    if let Some(raw) = file.get(file_key) {
        if let Ok(v) = raw.parse::<u32>() {
            return v;
        }
    }
    default
}

fn parse_string_layered(
    file: &HashMap<String, String>,
    env: &HashMap<String, String>,
    file_key: &str,
    env_key: &str,
    default: &str,
) -> String {
    if let Some(raw) = env.get(env_key) {
        if !raw.is_empty() {
            return raw.clone();
        }
    }
    if let Some(raw) = file.get(file_key) {
        if !raw.is_empty() {
            return raw.clone();
        }
    }
    default.to_owned()
}

impl GameConfig {
    /// Resolve one config from the three layers. Only keys under
    /// ENV_PREFIX are read from the env map; anything else is ignored.
    fn from_layers(file: &HashMap<String, String>, env: &HashMap<String, String>) -> Self {
        let prefixed: HashMap<&str, &String> = env
            .iter()
            .filter(|(k, _)| k.starts_with(ENV_PREFIX))
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        let _ = &prefixed;
        Self {
            tick_hz: parse_u32_layered(
                file,
                env,
                TICK_HZ_FILE_KEY,
                TICK_HZ_ENV_KEY,
                TICK_HZ_DEFAULT,
            ),
            arena_name: parse_string_layered(
                file,
                env,
                ARENA_NAME_FILE_KEY,
                ARENA_NAME_ENV_KEY,
                ARENA_NAME_DEFAULT,
            ),
            max_fighters: parse_u32_layered(
                file,
                env,
                MAX_FIGHTERS_FILE_KEY,
                MAX_FIGHTERS_ENV_KEY,
                MAX_FIGHTERS_DEFAULT,
            ),
        }
    }
}

fn file_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn defaults_win_when_all_layers_empty() {
    assert!(Layer::Defaults < Layer::File);
    assert!(Layer::File < Layer::PrefixedEnv);
    let cfg = GameConfig::from_layers(&HashMap::new(), &HashMap::new());
    assert_eq!(cfg, GameConfig::default());
}

#[test]
fn file_beats_defaults() {
    let file = file_map(&[
        ("tick_hz", "90"),
        ("arena_name", "colosseum"),
        ("max_fighters", "4"),
    ]);
    let cfg = GameConfig::from_layers(&file, &HashMap::new());
    assert_eq!(cfg.tick_hz, 90);
    assert_eq!(cfg.arena_name, "colosseum");
    assert_eq!(cfg.max_fighters, 4);
}

#[test]
fn env_beats_file() {
    let file = file_map(&[
        ("tick_hz", "90"),
        ("arena_name", "colosseum"),
        ("max_fighters", "4"),
    ]);
    let env = env_map(&[
        ("GAME_TICK_HZ", "120"),
        ("GAME_ARENA_NAME", "forum"),
        ("GAME_MAX_FIGHTERS", "8"),
    ]);
    let cfg = GameConfig::from_layers(&file, &env);
    assert_eq!(cfg.tick_hz, 120);
    assert_eq!(cfg.arena_name, "forum");
    assert_eq!(cfg.max_fighters, 8);
}

#[test]
fn env_beats_defaults_without_file() {
    let env = env_map(&[("GAME_TICK_HZ", "144")]);
    let cfg = GameConfig::from_layers(&HashMap::new(), &env);
    assert_eq!(cfg.tick_hz, 144);
    assert_eq!(cfg.arena_name, ARENA_NAME_DEFAULT);
    assert_eq!(cfg.max_fighters, MAX_FIGHTERS_DEFAULT);
}

#[test]
fn prefix_isolation_ignores_unprefixed_and_wrong_prefix() {
    let file = file_map(&[("tick_hz", "90")]);
    let env = env_map(&[
        ("TICK_HZ", "120"),
        ("OTHER_TICK_HZ", "150"),
        ("GAMEPLAY_TICK_HZ", "180"),
    ]);
    let cfg = GameConfig::from_layers(&file, &env);
    assert_eq!(cfg.tick_hz, 90);
    assert_eq!(cfg.arena_name, ARENA_NAME_DEFAULT);
}

#[test]
fn missing_falls_through_per_field() {
    let file = file_map(&[("arena_name", "colosseum")]);
    let env = env_map(&[("GAME_MAX_FIGHTERS", "6")]);
    let cfg = GameConfig::from_layers(&file, &env);
    assert_eq!(cfg.tick_hz, TICK_HZ_DEFAULT);
    assert_eq!(cfg.arena_name, "colosseum");
    assert_eq!(cfg.max_fighters, 6);
}
