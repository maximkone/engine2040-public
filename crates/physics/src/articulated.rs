//! articulated: winch, rope and load solved as one constraint graph (NS1).
//!
//! Consumer: none yet. A portcullis lift or a chariot trace builds one [`Graph`] from node, link
//! and winch definitions, steps it once per tick and reads the node positions back. Data owner:
//! this module owns the solve; masses, rest lengths and winch targets come from the caller's
//! tables. Units: integer millimeters on the wire, Q8 millimeters inside (positions and rest
//! lengths carry eight fractional bits), kilograms, winch speeds in millimeters per second.
//! Determinism: integer `+ - * /` with `i128` intermediates and an integer square root, no float,
//! no clock. Definitions may arrive in any order: nodes are sorted by id, links by their id pair
//! and winches by their link, so the solve order is canonical and the same graph gives the same
//! ticks and the same [`Graph::hash`] however it was assembled. Errors: [`GraphError`] at build
//! time, nothing at step time. Budget: `ITERATIONS * links` constraint solves per tick, no
//! allocation after the build.
//!
//! The solve is position based: each tick the winches move their link's rest length toward its
//! target by at most their speed, free nodes take a damped Verlet step under gravity, then
//! every link, in canonical order, pulls its two ends back together when it is longer than its
//! rest length (a rope pulls and never pushes), splitting the move by inverse mass. A pinned node
//! (mass zero) never moves, so an anchor holds and a heavy load barely shifts a light neighbor.
#![forbid(unsafe_code)]

use engine_core::fixed::isqrt;
use engine_core::Fnv64;

/// Most nodes in one graph.
pub const MAX_NODES: usize = 64;
/// Most links in one graph.
pub const MAX_LINKS: usize = 128;
/// Heaviest node, kilograms.
pub const MAX_MASS_KG: u32 = 100_000;
/// Largest coordinate or rest length, millimeters.
pub const MAX_RANGE_MM: i32 = 1_000_000;
/// Constraint sweeps per tick.
pub const ITERATIONS: usize = 48;

/// Fractional bits of positions and rest lengths.
const FRAC: u32 = 8;
/// Gravity per tick squared in Q8 millimeters: 9.81 m/s^2 at 60 Hz is 2.725 mm.
const GRAVITY_Q8: i64 = 698;
/// Inverse mass of a 1 kg node, Q16.
const W_ONE: i64 = 1 << 16;
/// A free node keeps 63/64 of its velocity each tick.
const KEEP: (i64, i64) = (63, 64);

/// A body in the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeDef {
    /// Caller's id; unique.
    pub id: u32,
    /// Start position, millimeters.
    pub pos_mm: [i32; 3],
    /// Mass in kilograms; zero pins the node.
    pub mass_kg: u32,
}

/// A rope or chain segment between two nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkDef {
    /// One end's node id.
    pub a: u32,
    /// The other end's node id.
    pub b: u32,
    /// Length at which the link goes taut, millimeters.
    pub rest_mm: i32,
}

/// A winch on a link: reels the link's rest length toward `target_mm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinchDef {
    /// One end's node id of the link it drives.
    pub a: u32,
    /// The other end's node id of the link it drives.
    pub b: u32,
    /// Rest length it servos to, millimeters.
    pub target_mm: i32,
    /// Fastest it reels in or pays out, millimeters per second.
    pub speed_mm_s: i32,
}

/// Why a graph was refused. Nothing is built on refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// More than [`MAX_NODES`] nodes.
    TooManyNodes,
    /// More than [`MAX_LINKS`] links.
    TooManyLinks,
    /// Two nodes share an id.
    DuplicateNode(u32),
    /// A link or winch names a node that does not exist.
    UnknownNode(u32),
    /// A link joins a node to itself.
    SelfLink(u32),
    /// Two links join the same pair.
    DuplicateLink(u32, u32),
    /// A node's mass is past [`MAX_MASS_KG`] or a position is past [`MAX_RANGE_MM`].
    OutOfRange(u32),
    /// A rest length or winch target is negative or past [`MAX_RANGE_MM`], or a speed is negative.
    BadLength(u32, u32),
    /// A winch names a pair that no link joins.
    NoSuchLink(u32, u32),
    /// Two winches drive the same link.
    DuplicateWinch(u32, u32),
}

impl core::fmt::Display for GraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyNodes => write!(f, "articulated: more than {MAX_NODES} nodes"),
            Self::TooManyLinks => write!(f, "articulated: more than {MAX_LINKS} links"),
            Self::DuplicateNode(id) => write!(f, "articulated: two nodes share id {id}"),
            Self::UnknownNode(id) => write!(f, "articulated: no node {id}"),
            Self::SelfLink(id) => write!(f, "articulated: link joins node {id} to itself"),
            Self::DuplicateLink(a, b) => write!(f, "articulated: two links join {a} and {b}"),
            Self::OutOfRange(id) => write!(f, "articulated: node {id} is out of range"),
            Self::BadLength(a, b) => write!(f, "articulated: bad length on {a}-{b}"),
            Self::NoSuchLink(a, b) => write!(f, "articulated: no link joins {a} and {b}"),
            Self::DuplicateWinch(a, b) => write!(f, "articulated: two winches drive {a}-{b}"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    rest_q8: i64,
}

#[derive(Clone, Copy, Debug)]
struct Winch {
    link: usize,
    target_q8: i64,
    speed_q8: i64,
}

/// The constraint graph: nodes, links and winches in canonical order.
#[derive(Clone, Debug)]
pub struct Graph {
    ids: Vec<u32>,
    pos: Vec<[i64; 3]>,
    prev: Vec<[i64; 3]>,
    inv_mass: Vec<i64>,
    links: Vec<Link>,
    winches: Vec<Winch>,
    ticks: u64,
}

/// Millimeters to Q8.
fn q8(mm: i32) -> i64 {
    i64::from(mm) << FRAC
}

fn check_len(a: u32, b: u32, mm: i32) -> Result<(), GraphError> {
    if (0..=MAX_RANGE_MM).contains(&mm) {
        Ok(())
    } else {
        Err(GraphError::BadLength(a, b))
    }
}

impl Graph {
    /// Builds a graph from definitions in any order. Nodes are kept sorted by id, links by
    /// `(low id, high id)` and winches by the link they drive.
    ///
    /// # Errors
    /// Any [`GraphError`].
    pub fn build(
        nodes: &[NodeDef],
        links: &[LinkDef],
        winches: &[WinchDef],
    ) -> Result<Self, GraphError> {
        if nodes.len() > MAX_NODES {
            return Err(GraphError::TooManyNodes);
        }
        if links.len() > MAX_LINKS {
            return Err(GraphError::TooManyLinks);
        }
        let mut sorted = nodes.to_vec();
        sorted.sort_unstable_by_key(|n| n.id);
        if let Some(pair) = sorted.windows(2).find(|w| w[0].id == w[1].id) {
            return Err(GraphError::DuplicateNode(pair[0].id));
        }
        for n in &sorted {
            let in_range = n.pos_mm.iter().all(|c| c.abs() <= MAX_RANGE_MM);
            if !in_range || n.mass_kg > MAX_MASS_KG {
                return Err(GraphError::OutOfRange(n.id));
            }
        }
        let ids: Vec<u32> = sorted.iter().map(|n| n.id).collect();
        let index = |id: u32| {
            ids.binary_search(&id)
                .map_err(|_| GraphError::UnknownNode(id))
        };
        let mut built: Vec<Link> = Vec::with_capacity(links.len());
        let mut keys: Vec<(u32, u32)> = Vec::with_capacity(links.len());
        for l in links {
            if l.a == l.b {
                return Err(GraphError::SelfLink(l.a));
            }
            let key = (l.a.min(l.b), l.a.max(l.b));
            check_len(key.0, key.1, l.rest_mm)?;
            built.push(Link {
                a: index(key.0)?,
                b: index(key.1)?,
                rest_q8: q8(l.rest_mm),
            });
            keys.push(key);
        }
        let mut order: Vec<usize> = (0..built.len()).collect();
        order.sort_unstable_by_key(|i| keys[*i]);
        if let Some(pair) = order.windows(2).find(|w| keys[w[0]] == keys[w[1]]) {
            let (a, b) = keys[pair[0]];
            return Err(GraphError::DuplicateLink(a, b));
        }
        let link_keys: Vec<(u32, u32)> = order.iter().map(|i| keys[*i]).collect();
        let built: Vec<Link> = order.iter().map(|i| built[*i]).collect();
        let mut driven: Vec<Winch> = Vec::with_capacity(winches.len());
        for w in winches {
            let key = (w.a.min(w.b), w.a.max(w.b));
            check_len(key.0, key.1, w.target_mm)?;
            if w.speed_mm_s < 0 {
                return Err(GraphError::BadLength(key.0, key.1));
            }
            let link = link_keys
                .binary_search(&key)
                .map_err(|_| GraphError::NoSuchLink(key.0, key.1))?;
            driven.push(Winch {
                link,
                target_q8: q8(w.target_mm),
                speed_q8: i64::from(w.speed_mm_s) * 256 / 60,
            });
        }
        driven.sort_unstable_by_key(|w| w.link);
        if let Some(pair) = driven.windows(2).find(|w| w[0].link == w[1].link) {
            let (a, b) = link_keys[pair[0].link];
            return Err(GraphError::DuplicateWinch(a, b));
        }
        let pos: Vec<[i64; 3]> = sorted.iter().map(|n| n.pos_mm.map(q8)).collect();
        Ok(Self {
            ids,
            prev: pos.clone(),
            pos,
            inv_mass: sorted
                .iter()
                .map(|n| {
                    if n.mass_kg == 0 {
                        0
                    } else {
                        W_ONE / i64::from(n.mass_kg)
                    }
                })
                .collect(),
            links: built,
            winches: driven,
            ticks: 0,
        })
    }

    /// Ticks stepped so far.
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Position of node `id` in millimeters (rounded down), or `None` for an unknown id.
    pub fn pos_mm(&self, id: u32) -> Option<[i32; 3]> {
        let i = self.ids.binary_search(&id).ok()?;
        let mm = |q: i64| i32::try_from(q.div_euclid(1 << FRAC)).ok();
        Some([
            mm(self.pos[i][0])?,
            mm(self.pos[i][1])?,
            mm(self.pos[i][2])?,
        ])
    }

    /// Current length between the two ends of the link joining `a` and `b`, millimeters (rounded
    /// down), or `None` when no link joins them.
    pub fn link_len_mm(&self, a: u32, b: u32) -> Option<i64> {
        let (ia, ib) = (
            self.ids.binary_search(&a).ok()?,
            self.ids.binary_search(&b).ok()?,
        );
        let (lo, hi) = (ia.min(ib), ia.max(ib));
        self.links.iter().find(|l| (l.a, l.b) == (lo, hi))?;
        i64::try_from(self.len_q8(lo, hi) >> FRAC).ok()
    }

    /// Current rest length of the link joining `a` and `b`, millimeters (rounded down).
    pub fn rest_mm(&self, a: u32, b: u32) -> Option<i64> {
        let (ia, ib) = (
            self.ids.binary_search(&a).ok()?,
            self.ids.binary_search(&b).ok()?,
        );
        let (lo, hi) = (ia.min(ib), ia.max(ib));
        let link = self.links.iter().find(|l| (l.a, l.b) == (lo, hi))?;
        Some(link.rest_q8 >> FRAC)
    }

    fn len_q8(&self, a: usize, b: usize) -> u128 {
        let d = [
            self.pos[b][0] - self.pos[a][0],
            self.pos[b][1] - self.pos[a][1],
            self.pos[b][2] - self.pos[a][2],
        ];
        let sq: i128 = d.iter().map(|c| i128::from(*c) * i128::from(*c)).sum();
        isqrt(u128::try_from(sq).unwrap_or(0))
    }

    /// One tick: winches reel, free nodes fall, then every link pulls taut ends back together.
    pub fn step(&mut self) {
        for w in &self.winches {
            let rest = &mut self.links[w.link].rest_q8;
            *rest += (w.target_q8 - *rest).clamp(-w.speed_q8, w.speed_q8);
        }
        for i in 0..self.pos.len() {
            let cur = self.pos[i];
            if self.inv_mass[i] != 0 {
                let mut next = [0i64; 3];
                for k in 0..3 {
                    next[k] = cur[k] + (cur[k] - self.prev[i][k]) * KEEP.0 / KEEP.1;
                }
                next[1] -= GRAVITY_Q8;
                self.pos[i] = next;
            }
            self.prev[i] = cur;
        }
        for _ in 0..ITERATIONS {
            for l in 0..self.links.len() {
                self.pull(l);
            }
        }
        self.ticks += 1;
    }

    /// Pulls the ends of link `l` together when it is longer than its rest length, each end by
    /// its share of the inverse mass.
    fn pull(&mut self, l: usize) {
        let Link { a, b, rest_q8 } = self.links[l];
        let len = i128::try_from(self.len_q8(a, b)).unwrap_or(0);
        let excess = len - i128::from(rest_q8);
        let wsum = i128::from(self.inv_mass[a] + self.inv_mass[b]);
        if excess <= 0 || wsum == 0 {
            return;
        }
        for k in 0..3 {
            let d = i128::from(self.pos[b][k] - self.pos[a][k]);
            let share =
                |w: i64| i64::try_from(d * excess * i128::from(w) / (len * wsum)).unwrap_or(0);
            self.pos[a][k] += share(self.inv_mass[a]);
            self.pos[b][k] -= share(self.inv_mass[b]);
        }
    }

    /// FNV-1a over the whole graph state, for tapes and checkpoints.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(self.ticks);
        for (i, id) in self.ids.iter().enumerate() {
            h.write_u32(*id);
            for k in 0..3 {
                h.write_i64(self.pos[i][k]);
                h.write_i64(self.prev[i][k]);
            }
        }
        for l in &self.links {
            h.write_i64(l.rest_q8);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    const PLATFORM: u32 = 100;
    const ANCHORS: [(u32, i32); 3] = [(1, -1_000), (2, 0), (3, 1_000)];

    /// A portcullis on three chains: each runs from an anchor in the ceiling through two 40 kg
    /// links to a 200 kg platform, and a winch reels the first link of each chain in by half.
    fn lift() -> (Vec<NodeDef>, Vec<LinkDef>, Vec<WinchDef>) {
        let platform = [0, 3_000, 0];
        let mut nodes = vec![NodeDef {
            id: PLATFORM,
            pos_mm: platform,
            mass_kg: 200,
        }];
        let (mut links, mut winches) = (Vec::new(), Vec::new());
        for (anchor, x) in ANCHORS {
            let top = [x, 6_000, 0];
            nodes.push(NodeDef {
                id: anchor,
                pos_mm: top,
                mass_kg: 0,
            });
            // Two chain nodes at a third and two thirds of the way down, then the platform.
            let at = |third: i32| {
                [
                    top[0] + (platform[0] - top[0]) * third / 3,
                    top[1] + (platform[1] - top[1]) * third / 3,
                    0,
                ]
            };
            let ids = [anchor, anchor * 10 + 1, anchor * 10 + 2, PLATFORM];
            for (third, id) in [(1, ids[1]), (2, ids[2])] {
                nodes.push(NodeDef {
                    id,
                    pos_mm: at(third),
                    mass_kg: 40,
                });
            }
            let dx = f64::from(x);
            let seg = ((dx * dx + 3_000.0 * 3_000.0).sqrt() / 3.0).round() as i32;
            for pair in ids.windows(2) {
                links.push(LinkDef {
                    a: pair[0],
                    b: pair[1],
                    rest_mm: seg,
                });
            }
            winches.push(WinchDef {
                a: ids[0],
                b: ids[1],
                target_mm: seg / 2,
                speed_mm_s: 300,
            });
        }
        (nodes, links, winches)
    }

    fn shuffled<T: Copy>(items: &[T], rng: &mut Rng) -> Vec<T> {
        let mut out = items.to_vec();
        for i in (1..out.len()).rev() {
            let j = usize::try_from(rng.next_u64() % u64::try_from(i + 1).expect("small"))
                .expect("small");
            out.swap(i, j);
        }
        out
    }

    /// Runs the lift until the platform is 300 mm up (or 600 ticks) and returns the tick, the
    /// final platform height after 400 ticks and the state hash.
    fn run(nodes: &[NodeDef], links: &[LinkDef], winches: &[WinchDef]) -> (u64, i32, u64) {
        let mut graph = Graph::build(nodes, links, winches).expect("builds");
        let mut risen = 0;
        for _ in 0..400 {
            graph.step();
            let y = graph.pos_mm(PLATFORM).expect("platform")[1];
            if risen == 0 && y >= 3_300 {
                risen = graph.ticks();
            }
        }
        (
            risen,
            graph.pos_mm(PLATFORM).expect("platform")[1],
            graph.hash(),
        )
    }

    #[test]
    fn articulated_three_chain_lift_raises_the_platform_the_same_way_x2_shuffled() {
        let (nodes, links, winches) = lift();
        let (risen, height, hash) = run(&nodes, &links, &winches);
        // The side chains bind: with their first links halved the platform sits where a rope of
        // 2 x 1054 + 527 mm from a ceiling point a meter off to the side ends, about 3562 mm.
        assert!(risen > 0, "the platform never rose 300 mm");
        assert!(
            (3_480..=3_640).contains(&height),
            "platform settled at {height} mm"
        );
        assert_eq!(
            (risen, height, hash),
            run(&nodes, &links, &winches),
            "replay x2"
        );
        let mut rng = Rng::new(2040);
        for round in 0..8 {
            let got = run(
                &shuffled(&nodes, &mut rng),
                &shuffled(&links, &mut rng),
                &shuffled(&winches, &mut rng),
            );
            assert_eq!((risen, height, hash), got, "shuffle {round}");
        }
    }

    #[test]
    fn the_chains_carry_the_load_without_stretching_past_five_percent() {
        let (nodes, links, winches) = lift();
        let mut graph = Graph::build(&nodes, &links, &winches).expect("builds");
        for _ in 0..400 {
            graph.step();
        }
        for l in &links {
            let rest = graph.rest_mm(l.a, l.b).expect("rest");
            let len = graph.link_len_mm(l.a, l.b).expect("len");
            assert!(
                len <= rest * 105 / 100,
                "{}-{} is {len} mm on a {rest} mm rope",
                l.a,
                l.b
            );
        }
    }

    #[test]
    fn a_winch_reels_at_its_speed_and_stops_at_the_target() {
        let nodes = [
            NodeDef {
                id: 1,
                pos_mm: [0, 2_000, 0],
                mass_kg: 0,
            },
            NodeDef {
                id: 2,
                pos_mm: [0, 0, 0],
                mass_kg: 50,
            },
        ];
        let links = [LinkDef {
            a: 2,
            b: 1,
            rest_mm: 2_000,
        }];
        let winch = [WinchDef {
            a: 1,
            b: 2,
            target_mm: 1_500,
            speed_mm_s: 600,
        }];
        let mut graph = Graph::build(&nodes, &links, &winch).expect("builds");
        graph.step();
        assert_eq!(
            graph.rest_mm(1, 2),
            Some(1_990),
            "10 mm per tick at 600 mm/s"
        );
        for _ in 0..60 {
            graph.step();
        }
        assert_eq!(graph.rest_mm(1, 2), Some(1_500));
        // The same winch pays a shorter rope out toward a longer target at the same speed.
        let short = [LinkDef {
            a: 2,
            b: 1,
            rest_mm: 1_000,
        }];
        let mut paid = Graph::build(&nodes, &short, &winch).expect("builds");
        paid.step();
        assert_eq!(
            paid.rest_mm(1, 2),
            Some(1_010),
            "pays out toward the longer target"
        );
    }

    #[test]
    fn a_slack_rope_does_not_pull() {
        let nodes = [
            NodeDef {
                id: 1,
                pos_mm: [0, 1_000, 0],
                mass_kg: 0,
            },
            NodeDef {
                id: 2,
                pos_mm: [0, 900, 0],
                mass_kg: 0,
            },
        ];
        let links = [LinkDef {
            a: 1,
            b: 2,
            rest_mm: 500,
        }];
        let mut graph = Graph::build(&nodes, &links, &[]).expect("builds");
        graph.step();
        assert_eq!(graph.pos_mm(2), Some([0, 900, 0]), "pinned ends stay put");
        let nodes = [
            NodeDef {
                id: 1,
                pos_mm: [0, 1_000, 0],
                mass_kg: 0,
            },
            NodeDef {
                id: 2,
                pos_mm: [0, 900, 0],
                mass_kg: 10,
            },
        ];
        let mut graph = Graph::build(&nodes, &links, &[]).expect("builds");
        graph.step();
        let y = graph.pos_mm(2).expect("node")[1];
        assert!(
            y < 900,
            "a slack rope lets the weight fall, not hold it: {y}"
        );
    }

    #[test]
    fn bad_graphs_are_refused_by_name() {
        let n = |id| NodeDef {
            id,
            pos_mm: [0; 3],
            mass_kg: 1,
        };
        let l = |a, b| LinkDef { a, b, rest_mm: 100 };
        assert_eq!(
            Graph::build(&[n(1), n(1)], &[], &[]).unwrap_err(),
            GraphError::DuplicateNode(1)
        );
        assert_eq!(
            Graph::build(&[n(1)], &[l(1, 2)], &[]).unwrap_err(),
            GraphError::UnknownNode(2)
        );
        assert_eq!(
            Graph::build(&[n(1)], &[l(1, 1)], &[]).unwrap_err(),
            GraphError::SelfLink(1)
        );
        assert_eq!(
            Graph::build(&[n(1), n(2)], &[l(1, 2), l(2, 1)], &[]).unwrap_err(),
            GraphError::DuplicateLink(1, 2)
        );
        assert_eq!(
            Graph::build(
                &[NodeDef {
                    mass_kg: MAX_MASS_KG + 1,
                    ..n(1)
                }],
                &[],
                &[]
            )
            .unwrap_err(),
            GraphError::OutOfRange(1)
        );
        let long = LinkDef {
            rest_mm: -1,
            ..l(1, 2)
        };
        assert_eq!(
            Graph::build(&[n(1), n(2)], &[long], &[]).unwrap_err(),
            GraphError::BadLength(1, 2)
        );
        let w = |a, b| WinchDef {
            a,
            b,
            target_mm: 10,
            speed_mm_s: 10,
        };
        assert_eq!(
            Graph::build(&[n(1), n(2)], &[l(1, 2)], &[w(1, 3)]).unwrap_err(),
            GraphError::NoSuchLink(1, 3)
        );
        assert_eq!(
            Graph::build(&[n(1), n(2)], &[l(1, 2)], &[w(1, 2), w(2, 1)]).unwrap_err(),
            GraphError::DuplicateWinch(1, 2)
        );
        let crowd: Vec<NodeDef> = (0..=u32::try_from(MAX_NODES).expect("small"))
            .map(n)
            .collect();
        assert_eq!(
            Graph::build(&crowd, &[], &[]).unwrap_err(),
            GraphError::TooManyNodes
        );
    }

    #[test]
    fn isqrt_rounds_down_on_edges() {
        for n in [
            0u128,
            1,
            2,
            3,
            4,
            8,
            9,
            15,
            16,
            17,
            1 << 100,
            (1 << 100) - 1,
            u128::MAX >> 8,
        ] {
            let r = isqrt(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "isqrt({n}) = {r}");
        }
    }
}
