//! spec: the one value that names a match.
//!
//! A match is a config, the fight rules and the static world it collides
//! with. Before this module the three travelled apart: the window built
//! `LiveMatch::with_rules_world(.., V2, kit)` while the network built
//! `LiveMatch::new` (rules v1, open sand), so the match on the wire was not
//! the match on screen. [`MatchSpec`] holds all three and builds the live
//! match in one place ([`MatchSpec::live`]); the game window, the net
//! server and the net client all build from it.
//!
//! Consumer: `engine-game` (`App::spec`, the window and the LAN host) and
//! `engine-net` (`Server::from_spec`, `Client::from_spec`, the lobby build
//! id). Data owner: this module owns the identity digest; [`MatchWorld`]
//! owns the world digest. Units: meters, fixed ticks. Determinism: same
//! spec, same checkpoints; the digest is FNV over integers and `to_bits`.
//! Errors: [`MatchSpec::live`] is `None` for an invalid config, an unknown
//! rules version or a world that blocks a spawn, like
//! [`LiveMatch::with_rules_world`].

use engine_combat::tables::TABLES_VERSION_V2;
use engine_core::Fnv64;

use crate::world::kit::{kit_match_world, KitError};
use crate::world::MatchWorld;
use crate::{ArenaConfig, ArenaRules, LiveMatch};

/// Everything that decides a match besides its input tape.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSpec {
    /// Seed, length, fighter count, arena size and move-table version.
    pub config: ArenaConfig,
    /// Fight rules the sim runs under.
    pub rules: ArenaRules,
    /// Static obstacles the fighters collide with.
    pub world: MatchWorld,
    /// Spear grapple on (`docs/plan/spear-grapple.md`): the `THROW_SPEAR`
    /// button throws a spear and tethers fighters. Off by default; off leaves
    /// the match, its hash and its identity exactly as before.
    pub spear: bool,
}

impl MatchSpec {
    /// A spec from its three parts.
    #[must_use]
    pub fn new(config: ArenaConfig, rules: ArenaRules, world: MatchWorld) -> Self {
        Self {
            config,
            rules,
            world,
            spear: false,
        }
    }

    /// This spec with the spear grapple on.
    #[must_use]
    pub fn with_spear(mut self) -> Self {
        self.spear = true;
        self
    }

    /// The frozen default: rules v1 on open sand, exactly what
    /// [`LiveMatch::new`] builds.
    #[must_use]
    pub fn v1(config: ArenaConfig) -> Self {
        Self::new(config, ArenaRules::V1, MatchWorld::empty())
    }

    /// The live duel: move tables v2 under the rules v2 they select, over
    /// `world`. This is what the game window plays (`App::into_duel`).
    #[must_use]
    pub fn duel(mut config: ArenaConfig, world: MatchWorld) -> Self {
        config.table_version = TABLES_VERSION_V2;
        Self::new(config, config.rules(), world)
    }

    /// [`MatchSpec::duel`] over the embedded kit placement's walls.
    ///
    /// # Errors
    ///
    /// [`KitError`] when the embedded placement does not build a world.
    pub fn kit_duel(config: ArenaConfig) -> Result<Self, KitError> {
        Ok(Self::duel(config, kit_match_world()?))
    }

    /// Digest of the world: `0` for open sand, else [`MatchWorld::digest`].
    #[must_use]
    pub fn world_digest(&self) -> u64 {
        self.world.digest()
    }

    /// Digest of the rules: version, teams, collision, balance and every
    /// pillar. Rules v1 and v2 never share one.
    #[must_use]
    pub fn rules_digest(&self) -> u64 {
        let rules = &self.rules;
        let mut hash = Fnv64::new();
        hash.write_u32(rules.version);
        hash.write(&rules.teams);
        hash.write_u8(u8::from(rules.collision));
        hash.write_u8(u8::from(rules.balance));
        for pillar in rules.pillars.iter().flatten() {
            for lane in pillar.min_mm.into_iter().chain(pillar.max_mm) {
                hash.write(&lane.to_le_bytes());
            }
        }
        hash.finish()
    }

    /// One digest over config, rules and world: two peers with equal
    /// identities play the same match from the same tape. The lobby build id
    /// carries it, so a peer on other rules or another world is refused
    /// before tick 0.
    #[must_use]
    pub fn identity(&self) -> u64 {
        let config = &self.config;
        let mut hash = Fnv64::new();
        hash.write_u64(config.seed);
        hash.write_u32(config.ticks);
        hash.write_u8(config.fighters);
        hash.write_u32(config.half_extent_m.to_bits());
        hash.write_u32(config.table_version);
        hash.write_u64(self.rules_digest());
        hash.write_u64(self.world_digest());
        // Mixed in only when on, so every spear-less identity is unchanged.
        if self.spear {
            hash.write_u8(0x5A);
        }
        hash.finish()
    }

    /// Fresh live match at tick 0 under this spec; `None` when the config is
    /// invalid, the rules version is unknown or the world blocks a spawn.
    #[must_use]
    pub fn live(&self) -> Option<LiveMatch> {
        LiveMatch::from_spec(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Obstacle;

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 0x5EC0,
            ticks: 240,
            fighters: 4,
            half_extent_m: 12.0,
            table_version: 1,
        }
    }

    /// Hash after `ticks` empty-input steps, one entry per tick.
    fn hashes(mut live: LiveMatch, ticks: u32) -> Vec<u64> {
        (0..ticks)
            .map(|_| live.step(&[]).expect("step").checkpoint.hash)
            .collect()
    }

    #[test]
    fn spec_default_is_the_live_new_match_byte_for_byte() {
        let by_spec = hashes(MatchSpec::v1(config()).live().expect("spec"), 240);
        let by_new = hashes(LiveMatch::new(&config()).expect("new"), 240);
        assert_eq!(by_spec, by_new);
    }

    #[test]
    fn spec_duel_is_rules_v2_with_v2_tables() {
        let spec = MatchSpec::duel(config(), MatchWorld::empty());
        assert_eq!(spec.config.table_version, TABLES_VERSION_V2);
        assert_eq!(spec.rules, spec.config.rules());
        assert_ne!(spec.rules, ArenaRules::V1);
        let live = spec.live().expect("duel builds");
        assert_eq!(live.rules(), spec.rules);
    }

    #[test]
    fn spec_live_equals_with_rules_world() {
        let spec = MatchSpec::kit_duel(config()).expect("kit world");
        let direct = LiveMatch::with_rules_world(&spec.config, spec.rules, spec.world.clone())
            .expect("direct");
        assert_eq!(hashes(spec.live().expect("spec"), 120), hashes(direct, 120));
    }

    #[test]
    fn spec_kit_duel_plays_another_match_than_the_default() {
        let v1 = hashes(MatchSpec::v1(config()).live().expect("v1"), 60);
        let duel = hashes(
            MatchSpec::duel(config(), MatchWorld::empty())
                .live()
                .expect("duel"),
            60,
        );
        let kit = hashes(
            MatchSpec::kit_duel(config())
                .expect("kit")
                .live()
                .expect("kit"),
            60,
        );
        assert_ne!(v1, duel, "rules v2 is not rules v1");
        assert_ne!(duel, kit, "the kit walls are part of the match");
    }

    #[test]
    fn spear_spec_identity_moves_only_when_on() {
        let base = MatchSpec::duel(config(), MatchWorld::empty());
        let armed = base.clone().with_spear();
        assert!(armed.spear && !base.spear);
        assert_ne!(
            base.identity(),
            armed.identity(),
            "a peer without it is refused"
        );
        assert_eq!(armed.identity(), armed.clone().identity());
        // A spear-less spec digests exactly as it did before the flag existed.
        let c = base.config;
        let mut old = Fnv64::new();
        old.write_u64(c.seed);
        old.write_u32(c.ticks);
        old.write_u8(c.fighters);
        old.write_u32(c.half_extent_m.to_bits());
        old.write_u32(c.table_version);
        old.write_u64(base.rules_digest());
        old.write_u64(base.world_digest());
        assert_eq!(base.identity(), old.finish());
    }

    #[test]
    fn spec_identity_moves_with_config_rules_and_world() {
        let base = MatchSpec::duel(config(), MatchWorld::empty());
        let mut seeded = base.clone();
        seeded.config.seed ^= 1;
        let mut ruled = base.clone();
        ruled.rules = ArenaRules::V1;
        let mut walled = base.clone();
        walled.world =
            MatchWorld::try_new(vec![Obstacle::aligned([6.0, 6.0], [0.5, 0.5])]).expect("world");
        let ids = [
            base.identity(),
            seeded.identity(),
            ruled.identity(),
            walled.identity(),
        ];
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "identities {ids:016x?}");
            }
        }
        assert_eq!(base.identity(), base.clone().identity(), "stable");
        assert_eq!(base.world_digest(), 0, "open sand digests to zero");
        assert_ne!(walled.world_digest(), 0);
    }
}
