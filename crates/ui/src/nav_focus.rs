//! nav_focus: directional focus navigation as pure data.
//!
//! Consumer: the menu state machine ([`crate::menus::Menu`]), which keeps
//! `menu_id + focus` and asks one question per press: which node is in the
//! direction the stick went. Data owner: `engine-ui`; nothing here knows about
//! pads, geometry or the renderer. Units: node positions are menu-space floats
//! (one unit per button pitch), ids are `u16`. Determinism: index order and
//! plain float compares, no hash order, no clock; the same nodes and request
//! pick the same id twice. Errors: none; a request that finds nothing returns
//! `None` and the caller keeps its highlight.
//!
//! Rules: a move lands on the nearest focusable node strictly inside the
//! direction's half-plane, ties to the smaller squared distance (then the
//! smaller id). `wrap` re-runs the same test from a cursor projected one menu
//! extent past the edge, so a row wraps end to end and a single-column list
//! (extent `0` across) never wraps sideways. `Blocked` and `Inert` nodes are
//! never chosen.

/// Navigation direction. `Up` is `+y`, `Down` is `-y`, `Right` is `+x`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Dir {
    /// Towards `+y`.
    Up,
    /// Towards `-y`.
    Down,
    /// Towards `+x`.
    Right,
    /// Towards `-x`.
    Left,
}

/// One navigation request; the donor's `NavRequest` shape.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NavRequest {
    /// Move the focus one step in `dir`.
    Move(Dir),
    /// Activate the focused node (a sub-menu entry point for a parent node).
    Action,
    /// Leave a sub-menu: focus the parent node again.
    Cancel,
    /// Focus this id directly, when it can take focus.
    FocusOn(u16),
    /// Unlock a blocked focus (a modal closing).
    Unlock,
}

/// Focus state of one node.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FocusState {
    /// Decoration only: never focused, never a target.
    Inert,
    /// First choice of the owning menu.
    Prioritized,
    /// The active sub-menu's active child.
    Active,
    /// The node holding focus now.
    Focused,
    /// Disabled or covered: never a target.
    Blocked,
}

impl FocusState {
    /// True when a move may land on this node.
    #[must_use]
    pub const fn focusable(self) -> bool {
        matches!(self, Self::Prioritized | Self::Active | Self::Focused)
    }
}

/// One focusable node: stable id plus its menu-space anchor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusNode {
    /// Stable id (the menu's row index today, a widget id later).
    pub id: u16,
    /// Menu-space anchor `[x, y]`.
    pub pos: [f32; 2],
    /// Focus state.
    pub state: FocusState,
}

/// Along-axis distance and squared distance from `from` to `to`, or `None`
/// when `to` is not strictly inside the direction's half-plane.
fn offsets(from: [f32; 2], dir: Dir, to: [f32; 2]) -> Option<(f32, f32)> {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let along = match dir {
        Dir::Right => dx,
        Dir::Left => -dx,
        Dir::Up => dy,
        Dir::Down => -dy,
    };
    if along <= 0.0 {
        return None;
    }
    Some((along, dx * dx + dy * dy))
}

/// Nearest focusable node in the direction's half-plane, ties to the smaller
/// squared distance and then to the smaller id. The node under `cursor` is
/// never a target (a press moves away from it, and a wrapped press must not
/// land back on the row it started from).
fn nearest(from: [f32; 2], cursor: [f32; 2], dir: Dir, nodes: &[FocusNode]) -> Option<u16> {
    let mut best: Option<(f32, f32, u16)> = None;
    for node in nodes {
        if !node.state.focusable() || node.pos == cursor {
            continue;
        }
        let Some((along, squared)) = offsets(from, dir, node.pos) else {
            continue;
        };
        let better = match best {
            None => true,
            Some((best_along, best_squared, best_id)) => {
                (along, squared, node.id) < (best_along, best_squared, best_id)
            }
        };
        if better {
            best = Some((along, squared, node.id));
        }
    }
    best.map(|(_, _, id)| id)
}

/// Resolve one move from `from` in `dir` over `nodes`. `edge` is the menu's
/// extent in the same units as the positions: with `wrap` the cursor is
/// projected one extent past the edge and the same test runs again, so a row
/// of three wraps from the rightmost to the leftmost node. A single-column
/// list passes a zero width across and never wraps sideways.
///
/// `None` means nothing lies that way (the caller keeps its highlight).
#[must_use]
pub fn resolve_2d(
    from: [f32; 2],
    dir: Dir,
    nodes: &[FocusNode],
    wrap: bool,
    edge: [f32; 2],
) -> Option<u16> {
    if let Some(id) = nearest(from, from, dir, nodes) {
        return Some(id);
    }
    if !wrap {
        return None;
    }
    let projected = match dir {
        Dir::Right => [from[0] - edge[0], from[1]],
        Dir::Left => [from[0] + edge[0], from[1]],
        Dir::Up => [from[0], from[1] - edge[1]],
        Dir::Down => [from[0], from[1] + edge[1]],
    };
    nearest(projected, from, dir, nodes)
}

/// The node a freshly focused menu should land on: the `Focused` node, else
/// the first `Prioritized`, else the first `Active`, else nothing. `Inert` and
/// `Blocked` nodes are skipped. (The donor walks a menu tree for its root and
/// active-child steps; this module holds no tree, so the flat order is Focused,
/// Prioritized, Active.)
#[must_use]
pub fn pick_first(nodes: &[FocusNode]) -> Option<u16> {
    for state in [
        FocusState::Focused,
        FocusState::Prioritized,
        FocusState::Active,
    ] {
        if let Some(node) = nodes.iter().find(|node| node.state == state) {
            return Some(node.id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x3 grid: id `row * 3 + col`, position `[col, row]`.
    fn grid() -> Vec<FocusNode> {
        (0..9u16)
            .map(|id| FocusNode {
                id,
                pos: [f32::from(id % 3), f32::from(id / 3)],
                state: FocusState::Prioritized,
            })
            .collect()
    }

    /// One row of three at `y = 0`, ids 0..=2.
    fn row() -> Vec<FocusNode> {
        (0..3u16)
            .map(|id| FocusNode {
                id,
                pos: [f32::from(id), 0.0],
                state: FocusState::Prioritized,
            })
            .collect()
    }

    const CENTRE: [f32; 2] = [1.0, 1.0];

    #[test]
    fn nav_moves_to_the_neighbour_in_the_pressed_direction() {
        let nodes = grid();
        // From the centre (id 4) each press lands on the orthogonal neighbour.
        for (dir, want) in [
            (Dir::Right, 5),
            (Dir::Left, 3),
            (Dir::Up, 7),
            (Dir::Down, 1),
        ] {
            assert_eq!(
                resolve_2d(CENTRE, dir, &nodes, false, [3.0, 3.0]),
                Some(want),
                "{dir:?} from the centre"
            );
        }
        // Two opposite requests come back to the centre in one press each.
        for (first, second) in [
            (Dir::Right, Dir::Left),
            (Dir::Left, Dir::Right),
            (Dir::Up, Dir::Down),
            (Dir::Down, Dir::Up),
        ] {
            let away = resolve_2d(CENTRE, first, &nodes, false, [3.0, 3.0]).expect("a neighbour");
            let at = nodes.iter().find(|node| node.id == away).expect("node");
            assert_eq!(
                resolve_2d(at.pos, second, &nodes, false, [3.0, 3.0]),
                Some(4),
                "{first:?} then {second:?} returns to the centre"
            );
        }
        // A press that nothing lies behind keeps the highlight: None.
        assert_eq!(
            resolve_2d([0.0, 0.0], Dir::Left, &nodes, false, [3.0, 3.0]),
            None
        );
        assert_eq!(
            resolve_2d([0.0, 0.0], Dir::Down, &nodes, false, [3.0, 3.0]),
            None
        );
    }

    #[test]
    fn wrap_only_on_a_wrapping_menu() {
        let nodes = row();
        let right_edge = [2.0, 0.0];
        // Plain row at the right edge: nothing further right.
        assert_eq!(
            resolve_2d(right_edge, Dir::Right, &nodes, false, [3.0, 1.0]),
            None
        );
        // The same request on a wrapping menu lands on the leftmost node.
        assert_eq!(
            resolve_2d(right_edge, Dir::Right, &nodes, true, [3.0, 1.0]),
            Some(0)
        );
        // And leftwards from the leftmost lands on the rightmost.
        assert_eq!(
            resolve_2d([0.0, 0.0], Dir::Left, &nodes, true, [3.0, 1.0]),
            Some(2)
        );
        // A single-column list (zero width across) never wraps sideways.
        let column: Vec<FocusNode> = (0..3u16)
            .map(|id| FocusNode {
                id,
                pos: [0.0, f32::from(id)],
                state: FocusState::Prioritized,
            })
            .collect();
        assert_eq!(
            resolve_2d([0.0, 1.0], Dir::Right, &column, true, [0.0, 3.0]),
            None
        );
        // Inside the row a press still moves one step, wrap or not.
        assert_eq!(
            resolve_2d([1.0, 0.0], Dir::Right, &nodes, true, [3.0, 1.0]),
            Some(2)
        );
        assert_eq!(
            resolve_2d([1.0, 0.0], Dir::Right, &nodes, false, [3.0, 1.0]),
            Some(2)
        );
    }

    #[test]
    fn blocked_nodes_are_never_chosen() {
        let mut nodes = grid();
        // Block the right-hand neighbour of the centre: the press skips it.
        nodes[5].state = FocusState::Blocked;
        // The two nodes one step right off the centre line tie on distance;
        // the smaller id wins, never the blocked one.
        assert_eq!(
            resolve_2d(CENTRE, Dir::Right, &nodes, false, [3.0, 3.0]),
            Some(2)
        );
        // Inert nodes are skipped the same way.
        nodes[2].state = FocusState::Inert;
        assert_eq!(
            resolve_2d(CENTRE, Dir::Right, &nodes, false, [3.0, 3.0]),
            Some(8)
        );
        // A menu where every direction is blocked resolves to nothing.
        for node in nodes.iter_mut() {
            if node.id != 4 {
                node.state = FocusState::Blocked;
            }
        }
        for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            assert_eq!(resolve_2d(CENTRE, dir, &nodes, true, [3.0, 3.0]), None);
        }
    }

    #[test]
    fn pick_first_prefers_focused_then_prioritized_then_active() {
        let node = |id: u16, state: FocusState| FocusNode {
            id,
            pos: [f32::from(id), 0.0],
            state,
        };
        let nodes = [
            node(0, FocusState::Inert),
            node(1, FocusState::Prioritized),
            node(2, FocusState::Focused),
            node(3, FocusState::Active),
            node(4, FocusState::Blocked),
        ];
        assert_eq!(pick_first(&nodes), Some(2), "the focused node wins");
        let without_focused = [nodes[0], nodes[1], nodes[3], nodes[4]];
        assert_eq!(pick_first(&without_focused), Some(1), "then prioritized");
        assert_eq!(pick_first(&[nodes[0], nodes[3]]), Some(3), "then active");
        assert_eq!(
            pick_first(&[nodes[0], nodes[4]]),
            None,
            "never inert or blocked"
        );
        assert_eq!(pick_first(&[]), None);
    }
}
