//! The five build cells of the page, in hotkey order 1-5.
//!
//! One table answers the page's `houseKinds` line and turns the id it hands back
//! into a sim [`HouseKind`]. The id is the sim's own kind name, the footprint and
//! the cost are read from `kam2-sim`, and the refusal words are this page's
//! English for the sim's codes: the sim still owns every rule, this module only
//! names it. The page never re-implements a check and never guesses a cost.

use kam2_sim::{HouseKind, Refusal};

/// The build cells in hotkey order: cell `n` of the page arms `HOTKEYS[n - 1]`.
pub const HOTKEYS: [(HouseKind, &str); 5] = [
    (HouseKind::Warehouse, "Warehouse"),
    (HouseKind::Farm, "Farm"),
    (HouseKind::Mill, "Mill"),
    (HouseKind::Bakery, "Bakery"),
    (HouseKind::Woodcutter, "Woodcutter"),
];

/// The kind a page id names, or `None` when the id is not a build cell.
#[must_use]
pub fn kind_of(id: &str) -> Option<HouseKind> {
    HOTKEYS
        .iter()
        .map(|(kind, _)| *kind)
        .find(|kind| kind.name().eq_ignore_ascii_case(id))
}

/// The line for a build cell the page does not know; no sim code, because no
/// sim rule was ever asked.
pub const UNKNOWN_KIND: &str = "BadKind: that build cell does not exist";

/// The words for one answer of the sim: empty when the sim said yes.
#[must_use]
pub fn answer_line(refusal: Option<Refusal>) -> String {
    refusal.map_or_else(String::new, |refusal| refusal_line(refusal).to_string())
}

/// The five build cells as JSON: id, title, footprint and cost.
///
/// The page builds one cell per row, so the cell count, the hotkey order, the
/// hover box and the tooltip all come from this one line.
#[must_use]
pub fn house_kinds() -> String {
    let mut cells: Vec<String> = Vec::with_capacity(HOTKEYS.len());
    for (kind, label) in HOTKEYS {
        let (w, h) = kind.footprint();
        let cost = cost_text(kind);
        cells.push(format!(
            "{{\"id\":\"{}\",\"label\":\"{label}\",\"w\":{w},\"h\":{h},\"cost\":\"{cost}\"}}",
            kind.name()
        ));
    }
    format!("[{}]", cells.join(","))
}

/// The English line for one sim refusal; the sim's code stays first so a log
/// keeps the stable name and the bottom strip prints words.
#[must_use]
pub const fn refusal_line(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::OutOfBounds => "OutOfBounds: that ground is off the field",
        Refusal::NotBuildable => "NotBuildable: that ground cannot carry a house",
        Refusal::Occupied => "Occupied: that ground is taken",
        Refusal::NoRoad => "NoRoad: no road reaches that ground",
        Refusal::NotAffordable => "NotAffordable: not enough wares",
        Refusal::BadSide => "BadSide: that side does not exist",
        Refusal::NotYourUnit => "NotYourUnit: that unit is not yours",
        Refusal::NoUnit => "NoUnit: no such unit",
        Refusal::NoPath => "NoPath: no path reaches that tile",
    }
}

/// The cost in words, in cost order: `10 wood, 5 stone`.
fn cost_text(kind: HouseKind) -> String {
    let mut text = String::new();
    for (ware, n) in kind.cost() {
        if !text.is_empty() {
            text.push_str(", ");
        }
        text.push_str(&n.to_string());
        text.push(' ');
        text.push_str(ware.name());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{answer_line, house_kinds, kind_of, refusal_line, HOTKEYS, UNKNOWN_KIND};
    use kam2_sim::{HouseKind, Refusal};

    #[test]
    fn the_menu_lists_the_five_kinds_in_hotkey_order() {
        let json = house_kinds();
        assert_eq!(json.matches("\"id\":").count(), HOTKEYS.len(), "{json}");
        for (slot, (kind, label)) in HOTKEYS.into_iter().enumerate() {
            let id = format!("\"id\":\"{}\"", kind.name());
            let title = format!("\"label\":\"{label}\"");
            assert_eq!(
                json.matches(id.as_str()).count(),
                1,
                "cell {} names {} once",
                slot + 1,
                kind.name()
            );
            assert!(json.contains(title.as_str()), "{json}");
            assert_eq!(kind_of(kind.name()), Some(kind), "the id round trips");
        }
        assert!(json.starts_with("[{\"id\":\"warehouse\""), "{json}");
        assert!(json.ends_with("}]"), "{json}");
    }

    #[test]
    fn a_cell_carries_its_footprint_and_its_cost() {
        let json = house_kinds();
        assert!(
            json.contains("\"id\":\"farm\",\"label\":\"Farm\",\"w\":2,\"h\":2,\"cost\":\"4 wood\""),
            "{json}"
        );
        assert!(
            json.contains("\"cost\":\"10 wood, 5 stone\""),
            "the warehouse cost reads as the sim charges it: {json}"
        );
    }

    #[test]
    fn ids_are_case_tolerant_and_unknown_ones_are_none() {
        assert_eq!(kind_of("Farm"), Some(HouseKind::Farm));
        assert_eq!(kind_of("WOODCUTTER"), Some(HouseKind::Woodcutter));
        assert_eq!(kind_of("cottage"), None, "the cottage is not a build cell");
        assert_eq!(kind_of(""), None);
    }

    #[test]
    fn every_sim_refusal_has_words_and_keeps_its_code() {
        for refusal in [
            Refusal::OutOfBounds,
            Refusal::NotBuildable,
            Refusal::Occupied,
            Refusal::NoRoad,
            Refusal::NotAffordable,
            Refusal::BadSide,
            Refusal::NotYourUnit,
            Refusal::NoUnit,
            Refusal::NoPath,
        ] {
            let line = refusal_line(refusal);
            assert!(line.contains(": "), "{refusal:?} has no code and words");
            assert!(
                line.split_once(": ")
                    .is_some_and(|(_, words)| !words.is_empty()),
                "{refusal:?} has no words"
            );
        }
        assert_eq!(
            refusal_line(Refusal::NoRoad),
            "NoRoad: no road reaches that ground"
        );
    }

    #[test]
    fn an_answer_line_is_empty_only_when_the_sim_said_yes() {
        assert_eq!(answer_line(None), "", "the sim took the house");
        assert_eq!(
            answer_line(Some(Refusal::NotBuildable)),
            refusal_line(Refusal::NotBuildable),
            "a refusal keeps the sim's own words"
        );
        assert!(UNKNOWN_KIND.contains(": "), "even an unknown cell has words");
    }
}
