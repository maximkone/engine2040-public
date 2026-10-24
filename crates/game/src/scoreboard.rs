//! scoreboard: the fight's numbers, folded once and shown at the bell.
//!
//! Consumer: the result overlay reads [`Scoreboard`] after the last tick; the
//! sim never reads it. Data owner: this module; the arena hands over the
//! `FightEvent` tape it already records. Units: hits, blocks and kills are
//! counts, wounds in permille, chain in hits. Determinism: one pass over the
//! tape, rows ordered by wounds then by fighter id, so the same tape always
//! prints the same table. Errors: none; an empty tape gives an empty table.
//! Budget: one pass, one row per fighter, no heap in the fold beyond the rows.

use engine_combat::fight::FightEvent;

/// The largest roster a scoreboard prints, mirroring the arena slots.
pub const MAX_ROWS: usize = 8;

/// One fighter's line: what the tape says it did and what it took.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScoreRow {
    /// Fighter slot, the tie-breaker.
    pub fighter: u8,
    /// Full-damage hits this fighter landed.
    pub hits: u16,
    /// Full damage this fighter dealt, in permille.
    pub wounds: u32,
    /// Hits this fighter blocked or parried.
    pub blocks: u16,
    /// Knockdowns this fighter scored.
    pub kills: u16,
    /// Longest run of consecutive hits this fighter landed.
    pub longest_chain: u16,
}

/// The fight's numbers, ready to print.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Scoreboard {
    rows: Vec<ScoreRow>,
    chain: (u8, u16),
}

impl Scoreboard {
    /// Fold a fight's event tape into one ranked table.
    ///
    /// A `Hit` counts for its attacker (one hit, its damage in wounds); a
    /// `Blocked` or `Parried` counts a block for the victim; a `Knockdown`
    /// counts a kill for its attacker. A chain is a run of `Hit`s by the same
    /// attacker with no `Hit` by anyone else between them.
    #[must_use]
    pub fn fold(events: &[FightEvent]) -> Self {
        let mut board = Self::default();
        for event in events {
            board.push(*event);
        }
        board.rank();
        board
    }

    /// Fold one more event into the running table; the order is fixed by
    /// [`Scoreboard::rank`], so a tick-by-tick feed and one [`Scoreboard::fold`]
    /// over the whole tape print the same table.
    pub fn push(&mut self, event: FightEvent) {
        match event {
            FightEvent::Hit {
                attacker, damage, ..
            } => {
                self.chain = if self.chain.0 == attacker {
                    (attacker, self.chain.1.saturating_add(1))
                } else {
                    (attacker, 1)
                };
                let row = row_of(&mut self.rows, attacker);
                row.hits = row.hits.saturating_add(1);
                row.wounds = row.wounds.saturating_add(u32::from(damage));
                let longest = self.chain.1;
                let row = row_of(&mut self.rows, attacker);
                row.longest_chain = row.longest_chain.max(longest);
            }
            FightEvent::Blocked { victim, .. } | FightEvent::Parried { victim, .. } => {
                let row = row_of(&mut self.rows, victim);
                row.blocks = row.blocks.saturating_add(1);
                self.chain.1 = 0;
            }
            FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => self.chain.1 = 0,
            FightEvent::Knockdown { attacker, .. } => {
                let row = row_of(&mut self.rows, attacker);
                row.kills = row.kills.saturating_add(1);
                self.chain.1 = 0;
            }
        }
    }

    /// Rank the rows: most damage first, then the lower fighter id, so the
    /// order never depends on how the tape was built.
    pub fn rank(&mut self) {
        self.rows
            .sort_by(|a, b| b.wounds.cmp(&a.wounds).then(a.fighter.cmp(&b.fighter)));
        self.rows.truncate(MAX_ROWS);
    }

    /// The ranked rows, best first.
    #[must_use]
    pub fn rows(&self) -> &[ScoreRow] {
        &self.rows
    }

    /// The table as printed lines: a header, then one line per fighter.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        lines.push("FIGHTER  HITS  WOUNDS  BLOCKS  KILLS  CHAIN".to_string());
        for row in &self.rows {
            lines.push(format!(
                "{:>7}  {:>4}  {:>6}  {:>6}  {:>5}  {:>5}",
                row.fighter, row.hits, row.wounds, row.blocks, row.kills, row.longest_chain
            ));
        }
        lines
    }
}

/// The row for `fighter`, created at zero the first time it is seen.
fn row_of(rows: &mut Vec<ScoreRow>, fighter: u8) -> &mut ScoreRow {
    let at = match rows.iter().position(|row| row.fighter == fighter) {
        Some(at) => at,
        None => {
            rows.push(ScoreRow {
                fighter,
                hits: 0,
                wounds: 0,
                blocks: 0,
                kills: 0,
                longest_chain: 0,
            });
            rows.len() - 1
        }
    };
    &mut rows[at]
}

#[cfg(test)]
mod tests {
    use super::*;
    use FightEvent::{Blocked, Hit, Knockdown, Parried};

    /// A clean two-fighter exchange: 2 lands, 0 blocks, 1 knockdown for it.
    #[test]
    fn a_clean_exchange_ranks_the_landed_hits() {
        let tape = [
            Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            Hit {
                attacker: 0,
                victim: 1,
                damage: 90,
            },
            Knockdown {
                attacker: 0,
                victim: 1,
            },
        ];
        let board = Scoreboard::fold(&tape);
        assert_eq!(board.rows().len(), 1);
        assert_eq!(
            board.rows()[0],
            ScoreRow {
                fighter: 0,
                hits: 2,
                wounds: 210,
                blocks: 0,
                kills: 1,
                longest_chain: 2,
            }
        );
        assert_eq!(
            board.lines(),
            vec![
                "FIGHTER  HITS  WOUNDS  BLOCKS  KILLS  CHAIN".to_string(),
                "      0     2     210       0      1      2".to_string(),
            ]
        );
    }

    /// Blocks break a chain and count for the blocker; the blocked fighter
    /// still ranks second on damage.
    #[test]
    fn blocks_break_a_chain_and_count_for_the_blocker() {
        let tape = [
            Hit {
                attacker: 1,
                victim: 0,
                damage: 60,
            },
            Hit {
                attacker: 1,
                victim: 0,
                damage: 60,
            },
            Blocked {
                attacker: 1,
                victim: 0,
                chip: 10,
            },
            Parried {
                attacker: 1,
                victim: 0,
            },
            Hit {
                attacker: 0,
                victim: 1,
                damage: 200,
            },
        ];
        let board = Scoreboard::fold(&tape);
        let rows = board.rows();
        assert_eq!(rows.len(), 2);
        // Ranked by damage: fighter 0 landed 200, fighter 1 landed 120.
        assert_eq!(
            rows[0],
            ScoreRow {
                fighter: 0,
                hits: 1,
                wounds: 200,
                blocks: 2,
                kills: 0,
                longest_chain: 1,
            }
        );
        assert_eq!(
            rows[1],
            ScoreRow {
                fighter: 1,
                hits: 2,
                wounds: 120,
                blocks: 0,
                kills: 0,
                longest_chain: 2,
            }
        );
        assert_eq!(
            board.lines()[1],
            "      0     1     200       2      0      1"
        );
        assert_eq!(
            board.lines()[2],
            "      1     2     120       0      0      2"
        );
    }

    /// A knockdown breaks the chain, and equal damage ties break by the lower
    /// fighter id.
    #[test]
    fn equal_damage_ties_by_fighter_id_and_chains_restart() {
        let tape = [
            Hit {
                attacker: 3,
                victim: 2,
                damage: 75,
            },
            Hit {
                attacker: 2,
                victim: 3,
                damage: 75,
            },
            Knockdown {
                attacker: 3,
                victim: 2,
            },
            Hit {
                attacker: 3,
                victim: 2,
                damage: 75,
            },
        ];
        let board = Scoreboard::fold(&tape);
        let rows = board.rows();
        assert_eq!(
            (rows[0].fighter, rows[1].fighter),
            (3, 2),
            "more damage first"
        );
        assert_eq!((rows[0].wounds, rows[1].wounds), (150, 75));
        assert_eq!(rows[0].longest_chain, 1, "the knockdown broke the chain");
        assert_eq!(rows[0].kills, 1);
        assert_eq!(Scoreboard::fold(&tape), board, "fold twice equal");

        let tied = Scoreboard::fold(&tape[..2]);
        let tied_rows = tied.rows();
        assert_eq!(
            (tied_rows[0].fighter, tied_rows[1].fighter),
            (2, 3),
            "equal wounds: the lower id ranks first"
        );
        assert_eq!(tied_rows[0].wounds, tied_rows[1].wounds);
    }
}
