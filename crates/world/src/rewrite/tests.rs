use super::*;

const MAZE: &str = include_str!("../../../../content/maps/rewrite/maze_streets.rules");
const INSULA: &str = include_str!("../../../../content/maps/rewrite/insula_block.rules");
const RUIN: &str = include_str!("../../../../content/maps/rewrite/ruined_wall.rules");

fn run(text: &str, seed: u64) -> Grid {
    Program::parse(text)
        .expect("test rules parse")
        .run(seed)
        .expect("test rules settle")
}

fn refused(text: &str) -> RulesError {
    Program::parse(text).expect_err("the rules must be refused")
}

// ----------------------------------------------------------- exact grids

#[test]
fn all_grows_a_row_one_cell_per_round_without_overlap() {
    let g = run(
        "size 5 1\nalphabet BW\nput 2 0 W\nall { rule WB = WW sym=none }\n",
        1,
    );
    assert_eq!(g.render(), "BBWWW\n");
    assert_eq!(g.steps(), 3, "the put, then two rounds");
}

#[test]
fn prl_applies_every_match_of_the_round_start_at_once() {
    let g = run(
        "size 4 1\nalphabet BW\nput 0 0 W\nprl { rule WB = WW sym=none }\n",
        9,
    );
    assert_eq!(g.render(), "WWWW\n");
    assert_eq!(g.steps(), 4, "the put, then three rounds");
}

#[test]
fn one_with_full_symmetry_floods_a_square() {
    let g = run(
        "size 3 3\nalphabet BW\nput 1 1 W\none { rule WB = WW }\n",
        5,
    );
    assert_eq!(g.render(), "WWW\nWWW\nWWW\n");
    assert_eq!(g.steps(), 9, "the put, then eight single steps");
}

#[test]
fn a_known_rule_pair_gives_an_exact_grid_whatever_the_seed() {
    // One head walks right, leaving a trail, and a second node turns the trail
    // into a wall: every step has exactly one match, so no draw can change it.
    let text = "size 6 2\nalphabet .RTX\nput 0 0 R\n\
                one { rule R. = TR sym=none }\n\
                one { rule T = X }\n";
    let want = "XXXXXR\n......\n";
    for seed in [0, 1, 99, u64::MAX] {
        assert_eq!(run(text, seed).render(), want, "seed {seed}");
    }
}

#[test]
fn wildcards_match_any_symbol_and_keep_the_cell() {
    // `*` in IN matches anything; `*` in OUT leaves the cell: only the middle changes.
    let text = "size 3 1\nalphabet ABC\nput 0 0 B\nput 2 0 C\none { rule *A* = *C* sym=none }\n";
    assert_eq!(run(text, 3).render(), "BCC\n");
}

#[test]
fn markov_restarts_at_the_first_child_after_every_step() {
    // Child 0 may take one step in all; child 1 then finishes the rest.
    let text = "size 5 1\nalphabet ABC\n\
                markov {\n one steps=1 { rule A = B sym=none }\n one { rule A = C sym=none }\n}\n";
    for seed in 0..6 {
        let g = run(text, seed);
        assert_eq!((g.count('B'), g.count('C'), g.count('A')), (1, 4, 0));
    }
}

#[test]
fn sequence_finishes_each_child_before_the_next() {
    let text = "size 4 1\nalphabet ABC\nsequence {\n one { rule A = B }\n one { rule B = C }\n}\n";
    assert_eq!(run(text, 2).render(), "CCCC\n");
}

#[test]
fn percent_chance_thins_a_parallel_round() {
    let g = run(
        "size 20 20\nalphabet AB\nprl steps=1 { rule A = B p=50 }\n",
        4,
    );
    let b = g.count('B');
    assert!((140..=260).contains(&b), "about half of 400 cells, got {b}");
}

#[test]
fn symmetry_expands_to_the_distinct_orientations() {
    let n = |sym: &str| {
        let text = format!("size 4 4\nalphabet ABC\none {{ rule ABC = CBA sym={sym} }}\n");
        Program::parse(&text).expect("parses").variant_count()
    };
    assert_eq!((n("none"), n("flip"), n("rot"), n("all")), (1, 2, 4, 4));
    let square = "size 4 4\nalphabet AB\none { rule A = B }\n";
    assert_eq!(Program::parse(square).expect("parses").variant_count(), 1);
    let lopsided = "size 4 4\nalphabet AB\none { rule AA/AB = BB/BB }\n";
    assert_eq!(Program::parse(lopsided).expect("parses").variant_count(), 4);
}

// ---------------------------------------------------------------- bounds

#[test]
fn a_rule_set_that_never_settles_is_refused_not_cut_short() {
    let text = "size 2 2\nalphabet AB\none { rule A = B\n rule B = A }\n";
    let program = Program::parse(text).expect("parses");
    assert_eq!(program.run(1), Err(RunError::StepCap));
}

#[test]
fn a_node_step_cap_stops_a_node_cleanly() {
    let g = run("size 2 2\nalphabet AB\none steps=2 { rule A = B }\n", 1);
    assert_eq!(g.count('B'), 2);
    assert_eq!(g.steps(), 2);
}

// ---------------------------------------------------------- refused rules

#[test]
fn bad_rules_are_refused_with_their_line_number() {
    let bad_shape = RulesErrorKind::BadPattern("IN and OUT differ in shape");
    let cases: [(&str, usize, RulesErrorKind); 14] = [
        (
            "size 5 5\nalphabet AB\none {\n  rule AX = BB\n}\n",
            4,
            RulesErrorKind::UnknownSymbol('X'),
        ),
        (
            "size 5 5\nalphabet AB\n\none {\n rule AB/A = BB/BB\n}\n",
            5,
            RulesErrorKind::BadPattern("rows differ in length"),
        ),
        (
            "size 5 5\nalphabet AB\none {\n rule A = A\n}\n",
            4,
            RulesErrorKind::NoChange,
        ),
        (
            "size 5 5\nalphabet AB\none {\n rule AB = B\n}\n",
            4,
            bad_shape,
        ),
        (
            "size 5 5\nalphabet AB\n# open\none {\n rule A = B\n",
            4,
            RulesErrorKind::Unclosed,
        ),
        (
            "size 5 5\nalphabet AB\none { rule A = B }\n}\n",
            4,
            RulesErrorKind::StrayClose,
        ),
        (
            "size 5 5\nalphabet AB\nrule A = B\n",
            3,
            RulesErrorKind::Misplaced("`rule` belongs inside one, all or prl"),
        ),
        (
            "size 5 5\nalphabet AB\none {\n}\n",
            3,
            RulesErrorKind::EmptyNode,
        ),
        (
            "size 500 5\nalphabet AB\n",
            1,
            RulesErrorKind::OutOfRange("size (1..=128 per side)"),
        ),
        (
            "size 5 5\nalphabet AB\nalphabet AB\n",
            3,
            RulesErrorKind::Duplicate("alphabet"),
        ),
        (
            "size 5 5\nalphabet AB\nfly 1 2\n",
            3,
            RulesErrorKind::UnknownStatement("fly".to_owned()),
        ),
        (
            "one {\n rule A = B\n}\n",
            1,
            RulesErrorKind::NeedHeader("size"),
        ),
        (
            "size 5 5\nalphabet AB\none {\n rule A = B p=0\n}\n",
            4,
            RulesErrorKind::OutOfRange("p (1..=100)"),
        ),
        (
            "size 5 5\nalphabet AB\nrect 3 3 4 4 A\n",
            3,
            RulesErrorKind::OutOfRange("rectangle (must lie inside the grid)"),
        ),
    ];
    for (text, line, kind) in cases {
        let e = refused(text);
        assert_eq!((e.line, &e.kind), (line, &kind), "{text:?}");
    }
}

#[test]
fn more_refusals_name_the_line_and_read_clearly() {
    let e = refused("size 5 5\nalphabet AB\nsequence steps=3 {\n one { rule A = B }\n}\n");
    assert_eq!(
        (e.line, e.kind),
        (3, RulesErrorKind::BadAttr("steps=3".to_owned()))
    );
    let e = refused("size 5 5\nalphabet AB\nput 1 1 Z\n");
    assert_eq!((e.line, e.kind), (3, RulesErrorKind::UnknownSymbol('Z')));
    let e = refused("size 5 5\nalphabet AB\n");
    assert_eq!((e.line, e.kind), (0, RulesErrorKind::NoNodes));
    let e = refused("size 5 5\nalphabet A*\n");
    assert_eq!(e.line, 2);
    let e = refused("size 5 5\nalphabet AB\none {\n rule A = B\n put 1 1 A\n}\n");
    assert_eq!(e.line, 5);
    assert_eq!(
        refused("size 5 5\nalphabet AB\none {\n rule AX = BB\n}\n").to_string(),
        "rules line 4: symbol `X` is not in the alphabet"
    );
}

// --------------------------------------------------------------- examples

/// `(cells reached from the first open cell, open cells in all)`.
fn flood(g: &Grid, open: char) -> (usize, usize) {
    let (w, h) = (usize::from(g.width()), usize::from(g.height()));
    let is_open = |i: usize| g.symbol_at((i % w) as u16, (i / w) as u16) == Some(open);
    let total = (0..w * h).filter(|&i| is_open(i)).count();
    let Some(start) = (0..w * h).find(|&i| is_open(i)) else {
        return (0, 0);
    };
    let mut seen = vec![false; w * h];
    let mut stack = vec![start];
    seen[start] = true;
    let mut reached = 0;
    while let Some(i) = stack.pop() {
        reached += 1;
        let (x, y) = (i % w, i / w);
        let mut next = Vec::new();
        if x > 0 {
            next.push(i - 1);
        }
        if x + 1 < w {
            next.push(i + 1);
        }
        if y > 0 {
            next.push(i - w);
        }
        if y + 1 < h {
            next.push(i + w);
        }
        for n in next {
            if !seen[n] && is_open(n) {
                seen[n] = true;
                stack.push(n);
            }
        }
    }
    (reached, total)
}

#[test]
fn every_example_terminates_and_reproduces_byte_identically() {
    for (name, text) in [("maze", MAZE), ("insula", INSULA), ("ruin", RUIN)] {
        let program = Program::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        for seed in [0u64, 1, 2040] {
            let a = program
                .run(seed)
                .unwrap_or_else(|e| panic!("{name} seed {seed}: {e}"));
            let b = program.run(seed).expect("second run settles");
            assert_eq!(a.cells(), b.cells(), "{name} seed {seed}");
            assert_eq!(a.hash(), b.hash(), "{name} seed {seed}");
            assert_eq!(a.render(), b.render(), "{name} seed {seed}");
            // A fresh parse is the same program.
            assert_eq!(Program::parse(text).expect("parses"), program, "{name}");
        }
        let hashes: std::collections::BTreeSet<u64> = (0..8)
            .map(|s| program.run(s).expect("settles").hash())
            .collect();
        assert!(hashes.len() >= 4, "{name}: seeds barely change the grid");
    }
}

#[test]
fn maze_streets_are_one_connected_walled_block() {
    for seed in 0..6 {
        let g = run(MAZE, seed);
        assert_eq!((g.count('G'), g.count('R')), (0, 0), "no digging left over");
        // 225 junctions, 224 connectors of the spanning tree, 12 loop openings.
        assert_eq!(g.count('W'), 225 + 224 + 12, "seed {seed}");
        let (reached, total) = flood(&g, 'W');
        assert_eq!(reached, total, "seed {seed}: streets split in two");
        let (w, h) = (g.width(), g.height());
        for i in 0..w {
            for (x, y) in [(i, 0), (i, h - 1), (0, i), (w - 1, i)] {
                assert_eq!(g.symbol_at(x, y), Some('B'), "outer ring is wall");
            }
        }
    }
}

#[test]
fn insula_has_a_courtyard_and_doors_onto_the_street() {
    for seed in 0..6 {
        let g = run(INSULA, seed);
        assert_eq!(g.count('C'), 8 * 6, "courtyard untouched");
        assert!(
            g.count('D') >= 8,
            "4 street doors, 3 courtyard doors, 1+ divider door"
        );
        let at = |x: i32, y: i32| {
            let (x, y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
            g.symbol_at(x, y)
        };
        let street_door = (0..i32::from(g.height())).any(|y| {
            (0..i32::from(g.width())).any(|x| {
                at(x, y) == Some('D')
                    && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dy)| at(x + dx, y + dy) == Some('S'))
            })
        });
        assert!(street_door, "seed {seed}: no door opens onto the street");
        assert_eq!(g.symbol_at(0, 0), Some('S'), "the margin stays street");
    }
}

#[test]
fn ruined_wall_has_gaps_and_rubble() {
    for seed in 0..6 {
        let g = run(RUIN, seed);
        assert_eq!(g.count('H'), 3 + 30, "three breaches grown 30 cells");
        assert!(g.count('W') <= 36 * 3 - 33, "wall lost its breached cells");
        assert!(g.count('R') > 0, "seed {seed}: no rubble");
    }
}

#[test]
fn the_maze_hash_is_pinned() {
    // The grid hash of `maze_streets.rules` at seed 2040. A change here means
    // the rules, the symmetry order or the draw order moved: re-pin on purpose.
    assert_eq!(run(MAZE, 2040).hash(), 17_662_186_103_125_026_066);
}
