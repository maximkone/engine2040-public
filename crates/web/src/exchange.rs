//! exchange: what the played page shows of the v2 exchange (WEB16).
//!
//! The duel the page plays already runs the v2 rules: a parry window, a
//! locked riposte, a same-tick clash, poise and stamina. Until now none of it
//! left a mark on the picture: every cue was dropped, so a perfect parry
//! looked like a plain hit and the HUD had no stamina and no poise.
//!
//! This module holds the decisions as plain data, so they test without a GPU
//! and without a browser: which flash a cue paints, which one-word call the
//! tick speaks, and how wide the stamina bar and the poise pips are. The page
//! reads the cues and the sim's own numbers and hands them here.
//!
//! Names stay neutral (seat, body, exchange), so the page does not grow a
//! genre assumption: the crate's own ledger lists the words it may use.

use engine_audio::FightCue;

/// Ticks a fight cue keeps its flash on the page. The page draws at 30 Hz
/// and steps up to two ticks per frame, so a flash shorter than a few ticks
/// would never be seen; these run long enough to land in a captured frame.
pub const FLASH_TICKS: u32 = 8;

/// Ticks the one-word call stays on the page. A call is the loudest thing on
/// screen, so it holds a little longer than its flash.
pub const CALL_TICKS: u32 = 24;

/// Ticks the locked riposte window stays open after a parry, read from the
/// v2 combat tables (`PARRY_STAGGER_TICKS`). The riposte is the answer that
/// lands inside that window, so the page counts it from the parry.
pub const RIPOSTE_WINDOW_TICKS: u32 = 12;

/// Poise points of a full meter under the shipped v2 rows (`DEFAULT_MAX_POISE`).
pub const POISE_MAX: u16 = 100;

/// Stamina points of a full bar under the shipped rows (`MAX_STAMINA`).
pub const STAMINA_MAX: u16 = 100;

/// How the page paints one exchange event on the body it happened to.
///
/// One kind per cue, so a parry and a hit do not look alike: the row asks
/// for a parry ring, a block spark, a hit red and a dodge after-image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flash {
    /// A blow landed: the body flashes red.
    Hit,
    /// A held block soaked the blow: the body flashes a cold blue.
    Block,
    /// A timed parry caught the blow: the body flashes gold.
    Parry,
    /// A dodge slipped the blow: the body flashes a pale cyan after-image.
    Dodge,
    /// A blow dropped the fighter: the body flashes deep red.
    Down,
    /// The fighter swung with no stamina left: the body dims.
    Tired,
}

impl Flash {
    /// The page's word for this flash kind.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Hit => "HIT",
            Self::Block => "BLOCK",
            Self::Parry => "PARRY",
            Self::Dodge => "DODGE",
            Self::Down => "DOWN",
            Self::Tired => "TIRED",
        }
    }

    /// The body colour this flash paints, as a linear RGB triple.
    ///
    /// The kinds are far apart in hue and in brightness, so a capture pair
    /// tells them apart: a parry is gold, a hit is red, a block is blue and
    /// a dodge is a pale cyan.
    #[must_use]
    pub fn color(self) -> [f32; 3] {
        match self {
            Self::Hit => [1.0, 0.16, 0.10],
            Self::Block => [0.24, 0.52, 1.0],
            Self::Parry => [1.0, 0.82, 0.20],
            Self::Dodge => [0.70, 0.95, 1.0],
            Self::Down => [0.75, 0.05, 0.05],
            Self::Tired => [0.35, 0.35, 0.38],
        }
    }

    /// How bright the flash paints, as the material's exposure. A parry is
    /// the brightest thing on the page; a tired body only dims.
    #[must_use]
    pub fn exposure(self) -> f32 {
        match self {
            Self::Parry => 2.4,
            Self::Hit => 1.8,
            Self::Down => 1.6,
            Self::Block => 1.5,
            Self::Dodge => 1.4,
            Self::Tired => 0.6,
        }
    }
}

/// The flash a cue paints on the body it names, or `None` for a cue that
/// paints nothing on its own.
#[must_use]
pub fn flash_for(cue: &FightCue) -> Option<Flash> {
    match *cue {
        FightCue::Hit { .. } => Some(Flash::Hit),
        FightCue::Blocked { .. } => Some(Flash::Block),
        FightCue::Parried { .. } => Some(Flash::Parry),
        FightCue::Dodged { .. } => Some(Flash::Dodge),
        FightCue::StaminaOut { .. } => Some(Flash::Tired),
        FightCue::Knockdown { .. } => Some(Flash::Down),
    }
}

/// The body a cue happened to: the victim for a blow, the fighter for a
/// stamina-out. `None` for a cue that names nobody.
#[must_use]
pub fn cue_body(cue: &FightCue) -> Option<u8> {
    match *cue {
        FightCue::Hit { victim, .. }
        | FightCue::Blocked { victim, .. }
        | FightCue::Parried { victim, .. }
        | FightCue::Dodged { victim, .. }
        | FightCue::Knockdown { victim, .. } => Some(victim),
        FightCue::StaminaOut { fighter } => Some(fighter),
    }
}

/// The fighter a cue came from, or `None` for a cue with no attacker.
#[must_use]
pub fn cue_attacker(cue: &FightCue) -> Option<u8> {
    match *cue {
        FightCue::Hit { attacker, .. }
        | FightCue::Blocked { attacker, .. }
        | FightCue::Parried { attacker, .. }
        | FightCue::Dodged { attacker, .. }
        | FightCue::Knockdown { attacker, .. } => Some(attacker),
        FightCue::StaminaOut { .. } => None,
    }
}

/// The one-word call the page speaks for a tick.
///
/// The words are the exchange's own: `PARRY` for a timed parry, `RIPOSTE`
/// for the answer that lands inside the parry's window, `CLASH` for two
/// bodies landing on the same tick, `BREAK` for a poise break, and the plain
/// `HIT`/`BLOCK`/`DODGE`/`DOWN`/`TIRED` for the rest.
///
/// The order is fixed, loudest first: a poise break speaks over a clash,
/// a clash over a riposte, a riposte over the parry that opened it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// A poise meter ran out: the body broke.
    Break,
    /// Two bodies landed on the same tick.
    Clash,
    /// The answer landed inside a parry's window.
    Riposte,
    /// A timed parry caught the blow.
    Parry,
    /// A blow dropped the fighter.
    Down,
    /// A held block soaked the blow.
    Block,
    /// A blow landed.
    Hit,
    /// A dodge slipped the blow.
    Dodge,
    /// A press with no stamina left.
    Tired,
}

impl Call {
    /// The page's word for this call.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Break => "BREAK",
            Self::Clash => "CLASH",
            Self::Riposte => "RIPOSTE",
            Self::Parry => "PARRY",
            Self::Down => "DOWN",
            Self::Block => "BLOCK",
            Self::Hit => "HIT",
            Self::Dodge => "DODGE",
            Self::Tired => "TIRED",
        }
    }

    /// How loud this call is; a bigger number wins the tick.
    fn rank(self) -> u8 {
        match self {
            Self::Break => 9,
            Self::Clash => 8,
            Self::Riposte => 7,
            Self::Parry => 6,
            Self::Down => 5,
            Self::Block => 4,
            Self::Hit => 3,
            Self::Dodge => 2,
            Self::Tired => 1,
        }
    }
}

/// What the page remembers between ticks to name the v2 exchange.
///
/// The page reads the tick's cues and the sim's poise meters; this state
/// turns them into the words `RIPOSTE` and `CLASH` need, which no single
/// cue carries.
#[derive(Clone, Copy, Debug, Default)]
pub struct Exchange {
    /// The fighter that parried, and the tick the parry landed on.
    parried: Option<(u8, u32)>,
    /// The fighter that parried: the answer comes from it.
    riposte_from: u8,
    /// Whether a poise meter tripped on the last tick.
    broke: bool,
}

impl Exchange {
    /// A fresh exchange state: nothing happened yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The call for the tick, from its cues and the poise reading.
    ///
    /// `poise_broke` says a meter tripped on this tick, `tick` is the tick
    /// the cues belong to.
    #[must_use]
    pub fn call(&self, cues: &[FightCue], poise_broke: bool, tick: u32) -> Option<Call> {
        let mut best: Option<Call> = None;
        let mut take = |call: Call| {
            if best.is_none_or(|current| call.rank() > current.rank()) {
                best = Some(call);
            }
        };
        if poise_broke {
            take(Call::Break);
        }
        if self.is_clash(cues) {
            take(Call::Clash);
        }
        if self.is_riposte(cues, tick) {
            take(Call::Riposte);
        }
        for cue in cues {
            take(match *cue {
                FightCue::Parried { .. } => Call::Parry,
                FightCue::Knockdown { .. } => Call::Down,
                FightCue::Blocked { .. } => Call::Block,
                FightCue::Hit { .. } => Call::Hit,
                FightCue::Dodged { .. } => Call::Dodge,
                FightCue::StaminaOut { .. } => Call::Tired,
            });
        }
        best
    }

    /// The state after a tick: the parry that opened a riposte window, and
    /// whether a meter broke.
    #[must_use]
    pub fn after(&self, cues: &[FightCue], poise_broke: bool, tick: u32) -> Self {
        let mut next = *self;
        next.broke = poise_broke;
        for cue in cues {
            if let FightCue::Parried { victim, .. } = *cue {
                next.parried = Some((victim, tick));
                next.riposte_from = victim;
            }
        }
        next
    }

    /// True when a poise meter broke on the last tick.
    #[must_use]
    pub fn broke(&self) -> bool {
        self.broke
    }

    /// True when the tick's cues are a same-tick clash: two bodies landed a
    /// blow on each other, which the v2 rules resolve symmetrically.
    fn is_clash(&self, cues: &[FightCue]) -> bool {
        let mut landed: Option<(u8, u8)> = None;
        for cue in cues {
            let FightCue::Hit {
                attacker, victim, ..
            } = *cue
            else {
                continue;
            };
            match landed {
                None => landed = Some((attacker, victim)),
                Some((other_attacker, other_victim)) => {
                    if other_attacker == victim && other_victim == attacker {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// True when the tick's cues are the locked riposte: the fighter that
    /// parried lands a blow inside the parry's stagger window.
    fn is_riposte(&self, cues: &[FightCue], tick: u32) -> bool {
        let Some((parried_by, parry_tick)) = self.parried else {
            return false;
        };
        if tick.saturating_sub(parry_tick) > RIPOSTE_WINDOW_TICKS {
            return false;
        }
        cues.iter().any(|cue| {
            matches!(
                *cue,
                FightCue::Hit { attacker, .. } | FightCue::Blocked { attacker, .. }
                    if attacker == parried_by
            )
        })
    }
}

/// The stamina bar's width in percent, from the sim's own points. A bar
/// with no reading (a bad slot) is empty, never full.
#[must_use]
pub fn stamina_percent(points: Option<u16>) -> u32 {
    let points = u32::from(points.unwrap_or(0));
    points.min(u32::from(STAMINA_MAX)) * 100 / u32::from(STAMINA_MAX)
}

/// The poise pips for one meter: filled pips for the points left, hollow
/// ones for the rest. The bar is the meter's own cap, so a v1 meter (never
/// drained) shows full and a v2 meter shows what is left of it.
#[must_use]
pub fn poise_pips(current: Option<(u16, u16)>) -> String {
    let Some((current, max)) = current else {
        return String::new();
    };
    let max = u32::from(max);
    if max == 0 {
        return String::new();
    }
    let filled = u32::from(current).min(max) * 4 / max;
    let mut out = String::with_capacity(8);
    for _ in 0..filled {
        out.push('●');
    }
    for _ in filled..4 {
        out.push('○');
    }
    out
}

/// The body flash a seat shows, from the tick's cues: the loudest flash that
/// happened to that body, or `None` when nothing happened to it.
///
/// A parry flashes the fighter that parried; the attacker it staggered is
/// not flashed, so the page shows who answered.
#[must_use]
pub fn seat_flash(cues: &[FightCue], seat: u8) -> Option<Flash> {
    let mut best: Option<Flash> = None;
    for cue in cues {
        if cue_body(cue) != Some(seat) {
            continue;
        }
        let Some(flash) = flash_for(cue) else {
            continue;
        };
        if best.is_none_or(|current| flash_rank(flash) > flash_rank(current)) {
            best = Some(flash);
        }
    }
    best
}

/// How loud a flash is; a bigger number wins the seat.
fn flash_rank(flash: Flash) -> u8 {
    match flash {
        Flash::Parry => 6,
        Flash::Down => 5,
        Flash::Hit => 4,
        Flash::Block => 3,
        Flash::Dodge => 2,
        Flash::Tired => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(victim: u8) -> FightCue {
        FightCue::Hit {
            attacker: 1,
            victim,
            damage: 120,
        }
    }

    fn parried(attacker: u8, victim: u8) -> FightCue {
        FightCue::Parried { attacker, victim }
    }

    #[test]
    fn every_cue_kind_paints_its_own_flash() {
        assert_eq!(flash_for(&hit(0)), Some(Flash::Hit));
        assert_eq!(
            flash_for(&FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 24
            }),
            Some(Flash::Block)
        );
        assert_eq!(flash_for(&parried(1, 0)), Some(Flash::Parry));
        assert_eq!(
            flash_for(&FightCue::Dodged {
                attacker: 1,
                victim: 0
            }),
            Some(Flash::Dodge)
        );
        assert_eq!(
            flash_for(&FightCue::StaminaOut { fighter: 0 }),
            Some(Flash::Tired)
        );
        assert_eq!(
            flash_for(&FightCue::Knockdown {
                attacker: 1,
                victim: 0
            }),
            Some(Flash::Down)
        );
    }

    #[test]
    fn a_parry_and_a_hit_do_not_look_alike() {
        let parry = Flash::Parry;
        let blow = Flash::Hit;
        assert_ne!(parry.word(), blow.word());
        assert_ne!(parry.color(), blow.color());
        assert_ne!(parry.exposure(), blow.exposure());
        // Green is what separates them: a parry is lit gold (the green
        // channel carries the light), a hit is lit red (green nearly
        // dark), so a capture pair reads the two apart at any exposure.
        assert!(
            parry.color()[1] > 2.0 * blow.color()[1],
            "the green channel separates the gold from the red: {:?} vs {:?}",
            parry.color(),
            blow.color()
        );
    }

    #[test]
    fn the_cue_names_the_body_it_happened_to() {
        assert_eq!(cue_body(&hit(1)), Some(1));
        assert_eq!(cue_body(&parried(0, 1)), Some(1));
        assert_eq!(cue_attacker(&parried(0, 1)), Some(0));
        assert_eq!(cue_attacker(&FightCue::StaminaOut { fighter: 0 }), None);
    }

    #[test]
    fn a_seat_shows_the_loudest_flash_that_happened_to_it() {
        let cues = [parried(1, 0), hit(0)];
        assert_eq!(seat_flash(&cues, 0), Some(Flash::Parry));
        assert_eq!(seat_flash(&cues, 1), None);
    }

    #[test]
    fn a_parry_speaks_and_a_plain_hit_does_not_speak_over_it() {
        let exchange = Exchange::new();
        let call = exchange.call(&[parried(1, 0)], false, 10);
        assert_eq!(call, Some(Call::Parry));
        assert_eq!(call.map(Call::word), Some("PARRY"));
    }

    #[test]
    fn the_answer_inside_the_parry_window_is_a_riposte() {
        let mut exchange = Exchange::new();
        let cues = [parried(1, 0)];
        exchange = exchange.after(&cues, false, 10);
        // The parried fighter answers with a blow inside the window.
        let answer = [FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }];
        assert_eq!(exchange.call(&answer, false, 12), Some(Call::Riposte));
        assert_eq!(
            exchange.call(&answer, false, 12).map(Call::word),
            Some("RIPOSTE")
        );
    }

    #[test]
    fn an_answer_outside_the_window_is_a_plain_hit() {
        let mut exchange = Exchange::new();
        exchange = exchange.after(&[parried(1, 0)], false, 10);
        let answer = [FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }];
        let late = 10 + RIPOSTE_WINDOW_TICKS + 1;
        assert_eq!(exchange.call(&answer, false, late), Some(Call::Hit));
    }

    #[test]
    fn two_bodies_landing_on_the_same_tick_are_a_clash() {
        let exchange = Exchange::new();
        let cues = [
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: 120,
            },
        ];
        assert_eq!(exchange.call(&cues, false, 5), Some(Call::Clash));
        assert_eq!(
            exchange.call(&cues, false, 5).map(Call::word),
            Some("CLASH")
        );
    }

    #[test]
    fn one_blow_is_not_a_clash() {
        let exchange = Exchange::new();
        assert_eq!(exchange.call(&[hit(1)], false, 5), Some(Call::Hit));
    }

    #[test]
    fn a_poise_break_speaks_over_everything() {
        let exchange = Exchange::new();
        let cues = [parried(1, 0)];
        assert_eq!(exchange.call(&cues, true, 5), Some(Call::Break));
        assert_eq!(exchange.call(&cues, true, 5).map(Call::word), Some("BREAK"));
    }

    #[test]
    fn a_quiet_tick_speaks_nothing() {
        let exchange = Exchange::new();
        assert_eq!(exchange.call(&[], false, 5), None);
    }

    #[test]
    fn the_stamina_bar_reads_the_sim_points() {
        assert_eq!(stamina_percent(Some(100)), 100);
        assert_eq!(stamina_percent(Some(50)), 50);
        assert_eq!(stamina_percent(Some(0)), 0);
        assert_eq!(stamina_percent(None), 0);
        assert_eq!(stamina_percent(Some(200)), 100);
    }

    #[test]
    fn the_poise_pips_show_what_is_left_of_the_meter() {
        assert_eq!(poise_pips(Some((100, 100))), "●●●●");
        assert_eq!(poise_pips(Some((50, 100))), "●●○○");
        assert_eq!(poise_pips(Some((0, 100))), "○○○○");
        assert_eq!(poise_pips(None), "");
        assert_eq!(poise_pips(Some((5, 0))), "");
    }
}
