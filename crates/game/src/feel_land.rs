//! FEEL7 landing plumbing: the authority's fall rows into the app's tick
//! outcome and HUD health. Presentation and mirror only; the sim already
//! charged the fight, so nothing here enters a hash.

use crate::{App, TickOutcome};

/// Copy the live sim's fall landings into the app's tick scratch, so the
/// look, camera, audio and HUD share one landing tick.
pub(super) fn fill_tick_lands(app: &mut App) {
    app.tick_lands.clear();
    app.tick_lands
        .extend(app.live.landings().iter().flatten().copied());
}

/// Move the scratch landings into `out` and mirror the fall table's charge
/// into the HUD health before the tick's combat cues fold, matching the
/// sim's fall-before-strike order.
pub(super) fn apply_landings(app: &mut App, out: &mut TickOutcome) {
    out.lands.clear();
    out.lands.append(&mut app.tick_lands);
    for landing in &out.lands {
        if landing.damage == 0 {
            continue;
        }
        if let Some(hp) = app.hp.get_mut(usize::from(landing.slot)) {
            *hp = hp.saturating_sub(landing.damage);
        }
    }
}
