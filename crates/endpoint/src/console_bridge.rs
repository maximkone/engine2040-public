//! EG26 console bridge: the dev console's read-only door over the endpoint
//! session's live play slot.
//!
//! `engine-game` owns the [`ConsoleDoor`] trait so the game never depends on
//! `engine-endpoint`; this file is the endpoint half. The live bout is opened
//! and stepped by `play.step` / `sim.step`, and the console only reads it.
//! Units: fixed ticks and `u64` state hashes. No clock, no writes, no tick.

use engine_game::console::ConsoleDoor;

use crate::mcp::ServerState;

impl ConsoleDoor for ServerState {
    fn hash(&self) -> u64 {
        self.play.current().map_or(0, |bout| bout.observe(0).digest)
    }

    fn tick(&self) -> u32 {
        self.play.current().map_or(0, |bout| bout.observe(0).tick)
    }

    fn stats(&self) -> String {
        match self.play.current() {
            Some(bout) => {
                let view = bout.observe(0);
                let spec = bout.spec();
                format!(
                    "session={} rev={} seed={} fighters={} tick={} hash={:016x}",
                    self.session,
                    self.doc.revision,
                    spec.seed,
                    view.fighters.len(),
                    view.tick,
                    view.digest
                )
            }
            None => format!(
                "session={} rev={} live=none",
                self.session, self.doc.revision
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::play_session::BoutSpec;
    use engine_arena::BoutTape;
    use engine_editor::capability::Capability;

    #[test]
    fn console_bridge_reads_the_live_endpoint_slot() {
        let mut state = ServerState::new(7, Capability::unrestricted(7));
        assert_eq!(state.hash(), 0, "zero before the first tick");
        assert_eq!(state.tick(), 0);
        assert!(state.stats().contains("live=none"));

        let digest = {
            let bout = state
                .play
                .advance_to(BoutSpec::new(2040, 2, 20.0), &BoutTape::default(), 4)
                .expect("four ticks of the proof bout");
            bout.observe(0).digest
        };
        assert_ne!(digest, 0, "a stepped bout carries a state hash");
        assert_eq!(state.tick(), 4);
        assert_eq!(state.hash(), digest);
        let stats = state.stats();
        assert!(stats.contains("tick=4"), "{stats}");
        assert!(stats.contains("fighters=2"), "{stats}");
        assert!(stats.contains("seed=2040"), "{stats}");
    }
}
