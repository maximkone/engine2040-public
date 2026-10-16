//! Segment casts over caller-passed boxes in integer millimetres.
//!
//! Owner: physics lane. Units: box corners, segment endpoints, entry and exit
//! points are whole `i32` millimetres; travel distances are `u64`
//! millimetres. Determinism: integer math only (fixed-width `+ - * /` plus
//! `isqrt`); the oracle walks the caller slice in order and the index splits
//! at the median with collider-id tiebreaks, so the same inputs give the
//! same bytes on every machine and both paths return identical hits.
//! Errors: typed [`QueryError`]; refused input yields `None` or an error,
//! never a silent clamp.
//!
//! The caller owns the world. Boxes arrive as integer-millimetre corners
//! (see [`BoxMm::from_mm`]) or as capsule endpoints with a
//! radius (see [`BoxMm::from_capsule_mm`], conservative by construction).
//! This module adds no engine edge; it depends only on `engine-core` owners.
//!
//! Consumer: the moving-blade swing via [`QueryBvh::sweep_box_into`] (spike
//! caller `Dispatcher` in `tests/rb_query_kinds.rs`); later rows adapt the
//! index to the combat segment-cast trait and the perception verifier.
//! Until then the oracles [`brute_cast_into`] and [`brute_sweep_box_into`]
//! prove each index path hit for hit in tests.
//!
//! Budget: [`QueryBvh::build`] allocates once per index; [`QueryBvh::cast_into`]
//! and [`QueryBvh::sweep_box_into`] perform zero heap allocations
//! (caller-owned output buffer, fixed stack, in-place sort).
//!
//! ```
//! use engine_physics::query::{BoxMm, HitMm, QueryBvh, SegmentMm};
//!
//! let boxes = [BoxMm::from_mm([0, 0, 0], [1000, 1000, 1000], 7, 3)
//!     .expect("corners ordered")];
//! let index = QueryBvh::build(&boxes);
//! let seg = SegmentMm {
//!     from_mm: [-500, 500, 500],
//!     to_mm: [1500, 500, 500],
//! };
//! let mut out = [HitMm::EMPTY; 4];
//! let n = index.cast_into(&boxes, seg, &mut out).expect("buffer fits");
//! assert_eq!(n, 1);
//! assert_eq!(out[0].collider, 7);
//! assert_eq!(out[0].material, 3);
//! assert_eq!(out[0].entry_mm, [0, 500, 500]);
//! assert_eq!(out[0].exit_mm, [1000, 500, 500]);
//! ```

/// Maximum boxes per index leaf; the median split keeps depth logarithmic.
pub const LEAF_BOXES: usize = 4;

/// Traversal stack slots. Median-split depth stays below 62 for any input
/// length, so this bound is never reached; see the push invariant.
const STACK_SLOTS: usize = 64;

/// One caller-owned collider as an integer-millimetre box.
///
/// Capsules enter through [`BoxMm::from_capsule_mm`] as conservative boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoxMm {
    /// Minimum corner in whole millimetres, per axis.
    pub min_mm: [i32; 3],
    /// Maximum corner in whole millimetres, per axis.
    pub max_mm: [i32; 3],
    /// Caller collider id; the second sort key, so ties break identically.
    pub collider: u32,
    /// Caller material id, carried untouched into every hit.
    pub material: u16,
}

impl BoxMm {
    /// Box from integer-millimetre corners; `None` when any minimum exceeds
    /// its maximum.
    #[must_use]
    pub fn from_mm(
        min_mm: [i32; 3],
        max_mm: [i32; 3],
        collider: u32,
        material: u16,
    ) -> Option<Self> {
        for axis in 0..3 {
            if min_mm[axis] > max_mm[axis] {
                return None;
            }
        }
        Some(Self {
            min_mm,
            max_mm,
            collider,
            material,
        })
    }

    /// Conservative box around the capsule from `a_mm` to `b_mm` with
    /// `radius_mm`; `None` when the grown corners leave the `i32` range.
    ///
    /// Both endpoints plus the full radius sit inside the box by
    /// construction, so a segment that misses the box misses the capsule.
    #[must_use]
    pub fn from_capsule_mm(
        a_mm: [i32; 3],
        b_mm: [i32; 3],
        radius_mm: u32,
        collider: u32,
        material: u16,
    ) -> Option<Self> {
        let grown = i64::from(radius_mm);
        let mut min_mm = [0i32; 3];
        let mut max_mm = [0i32; 3];
        for axis in 0..3 {
            let low = i64::from(a_mm[axis]).min(i64::from(b_mm[axis])) - grown;
            let high = i64::from(a_mm[axis]).max(i64::from(b_mm[axis])) + grown;
            min_mm[axis] = i32::try_from(low).ok()?;
            max_mm[axis] = i32::try_from(high).ok()?;
        }
        Self::from_mm(min_mm, max_mm, collider, material)
    }
}

/// One directed segment in whole millimetres, from `from_mm` to `to_mm`.
///
/// A zero-length segment is a point query: it hits the boxes containing it
/// with distance zero and entry equal to exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SegmentMm {
    /// Segment start in whole millimetres.
    pub from_mm: [i32; 3],
    /// Segment end in whole millimetres.
    pub to_mm: [i32; 3],
}

/// One box hit, ordered by `(dist_mm, collider)`.
///
/// Entry and exit are the clamped segment parameters quantized to the
/// nearest millimetre; a segment starting inside the box reports entry
/// equal to its start with distance zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HitMm {
    /// Collider id copied from the hit box.
    pub collider: u32,
    /// Material id copied from the hit box.
    pub material: u16,
    /// Travel from the segment start to the entry point, whole millimetres.
    pub dist_mm: u64,
    /// Entry point in whole millimetres.
    pub entry_mm: [i32; 3],
    /// Exit point in whole millimetres.
    pub exit_mm: [i32; 3],
}

impl HitMm {
    /// Filler for caller-owned output buffers; every stored hit overwrites it.
    pub const EMPTY: Self = Self {
        collider: 0,
        material: 0,
        dist_mm: 0,
        entry_mm: [0; 3],
        exit_mm: [0; 3],
    };
}

impl Ord for HitMm {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (self.dist_mm, self.collider).cmp(&(other.dist_mm, other.collider))
    }
}

impl PartialOrd for HitMm {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Typed cast failures; refused input never writes partial hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// The caller buffer holds `capacity` hits but `needed` exist; resize
    /// to `needed` and cast again.
    BufferTooSmall { needed: usize, capacity: usize },
    /// The index was built over `built` boxes but the cast passed `given`;
    /// rebuild over the current slice before casting.
    StaleBuild { built: usize, given: usize },
}

impl core::fmt::Display for QueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BufferTooSmall { needed, capacity } => write!(
                f,
                "hit buffer too small: {needed} hits need {capacity} slots"
            ),
            Self::StaleBuild { built, given } => {
                write!(f, "index built over {built} boxes but cast over {given}")
            }
        }
    }
}

impl std::error::Error for QueryError {}

/// Median-split index over a caller box slice.
///
/// Built once per world change with [`QueryBvh::build`], then cast with
/// [`QueryBvh::cast_into`]. The index stores positions into the slice it
/// was built over; casting over a different-length slice is refused with
/// [`QueryError::StaleBuild`].
#[derive(Clone, Debug)]
pub struct QueryBvh {
    nodes: Vec<BvhNode>,
    order: Vec<usize>,
    count: usize,
}

impl QueryBvh {
    /// Index over `boxes`; empty input yields an empty index that hits nothing.
    ///
    /// Allocates once (nodes plus one permutation); casts never allocate.
    /// The split axis is the longest bounds axis with ties toward x, the
    /// split point is the median, and order ties break by collider id, so
    /// the build is stable for identical inputs.
    #[must_use]
    pub fn build(boxes: &[BoxMm]) -> Self {
        let mut bvh = Self {
            nodes: Vec::new(),
            order: (0..boxes.len()).collect(),
            count: boxes.len(),
        };
        if !boxes.is_empty() {
            split(boxes, &mut bvh.order, &mut bvh.nodes, 0, boxes.len());
        }
        bvh
    }

    /// Box count this index was built over.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// True when the index was built over no boxes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Cast `seg` over `boxes`, writing hits sorted by `(dist_mm, collider)`
    /// into `out`; returns the hit count.
    ///
    /// Determinism: identical inputs give identical bytes; the result equals
    /// [`brute_cast_into`] hit for hit. Allocation: none (fixed stack,
    /// caller buffer, in-place sort). Errors: [`QueryError::StaleBuild`]
    /// when `boxes` differs in length from the build slice,
    /// [`QueryError::BufferTooSmall`] naming the required size.
    pub fn cast_into(
        &self,
        boxes: &[BoxMm],
        seg: SegmentMm,
        out: &mut [HitMm],
    ) -> Result<usize, QueryError> {
        if boxes.len() != self.count {
            return Err(QueryError::StaleBuild {
                built: self.count,
                given: boxes.len(),
            });
        }
        if self.nodes.is_empty() {
            return Ok(0);
        }
        let mut fill = Fill {
            out,
            stored: 0,
            total: 0,
        };
        let cast = Cast::of(&seg);
        let mut stack = [0usize; STACK_SLOTS];
        let mut top = 1usize;
        stack[0] = self.nodes.len() - 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top]];
            // Descent prunes on bounding-box overlap only: a disjoint node
            // can hold no hit, and the exact slab test still decides every
            // leaf box, so pruning stays exact while inner visits cost six
            // compares. The node slab would reject almost nothing more —
            // a segment inside a node box always passes it — at full
            // fraction-test cost.
            if !bbox_hit(&node.min_mm, &node.max_mm, &cast.lo_mm, &cast.hi_mm) {
                continue;
            }
            match node.kind {
                Kind::Leaf { start, count } => {
                    for pos in start..start + count {
                        let candidate = &boxes[self.order[pos]];
                        if !bbox_hit(
                            &candidate.min_mm,
                            &candidate.max_mm,
                            &cast.lo_mm,
                            &cast.hi_mm,
                        ) {
                            continue;
                        }
                        if let Some(hit) = test_box_i(candidate, &cast) {
                            fill.push(hit);
                        }
                    }
                }
                Kind::Inner { left, right } => {
                    // Median-split depth stays far below the fixed stack
                    // (depth halves every level), so the bound below never
                    // fires for any allocatable index; the guard keeps the
                    // cast total without a panic site.
                    if top + 2 > STACK_SLOTS {
                        debug_assert!(false, "median-split index depth fits the fixed stack");
                        continue;
                    }
                    stack[top] = left;
                    stack[top + 1] = right;
                    top += 2;
                }
            }
        }
        fill.finish()
    }

    /// Swept box cast for the moving-blade kind: the box centred on `center`
    /// with half extents `half_mm` travels from `center.from_mm` to
    /// `center.to_mm`, writing hits sorted by `(dist_mm, collider)` into
    /// `out`; returns the hit count.
    ///
    /// Units: `center` corners in whole millimetres, `half_mm` per-axis half
    /// extents in whole millimetres; entry, exit and distance report the
    /// centre travel in whole millimetres. A zero half extent is the thin
    /// segment cast, hit for hit identical to [`QueryBvh::cast_into`]; a
    /// non-zero half extent is the Minkowski growth of every static box, so
    /// a thick blade hits where the thin ray misses.
    ///
    /// Determinism: integer math only; identical inputs give identical
    /// bytes and the result equals [`brute_sweep_box_into`] hit for hit.
    /// Allocation: none (fixed stack, caller buffer, in-place sort).
    /// Errors: [`QueryError::StaleBuild`] on length mismatch,
    /// [`QueryError::BufferTooSmall`] naming the required size.
    ///
    /// ```
    /// use engine_physics::query::{BoxMm, HitMm, QueryBvh, SegmentMm};
    ///
    /// let boxes = [BoxMm::from_mm([0, 0, 0], [1000, 1000, 1000], 7, 3)
    ///     .expect("corners ordered")];
    /// let index = QueryBvh::build(&boxes);
    /// // The thin centre line flies 100 mm above the box: a miss.
    /// let skim = SegmentMm {
    ///     from_mm: [-500, 1100, 500],
    ///     to_mm: [1500, 1100, 500],
    /// };
    /// let mut thin = [HitMm::EMPTY; 4];
    /// assert_eq!(index.cast_into(&boxes, skim, &mut thin).expect("fits"), 0);
    /// // The 250 mm blade around the same line clips the box: a hit.
    /// let mut swept = [HitMm::EMPTY; 4];
    /// let n = index
    ///     .sweep_box_into(&boxes, skim, [250, 250, 250], &mut swept)
    ///     .expect("fits");
    /// assert_eq!(n, 1);
    /// assert_eq!(swept[0].entry_mm, [-250, 1100, 500]);
    /// assert_eq!(swept[0].dist_mm, 250);
    /// ```
    pub fn sweep_box_into(
        &self,
        boxes: &[BoxMm],
        center: SegmentMm,
        half_mm: [u32; 3],
        out: &mut [HitMm],
    ) -> Result<usize, QueryError> {
        if boxes.len() != self.count {
            return Err(QueryError::StaleBuild {
                built: self.count,
                given: boxes.len(),
            });
        }
        if self.nodes.is_empty() {
            return Ok(0);
        }
        let cast = Cast::of(&center);
        let mut query_lo = [0i32; 3];
        let mut query_hi = [0i32; 3];
        for axis in 0..3 {
            query_lo[axis] = widen_low(cast.lo_mm[axis], half_mm[axis]);
            query_hi[axis] = widen_high(cast.hi_mm[axis], half_mm[axis]);
        }
        let mut fill = Fill {
            out,
            stored: 0,
            total: 0,
        };
        let mut stack = [0usize; STACK_SLOTS];
        let mut top = 1usize;
        stack[0] = self.nodes.len() - 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top]];
            // The grown query box bounds every position of the moving box,
            // so a node disjoint from it can hold no hit.
            if !bbox_hit(&node.min_mm, &node.max_mm, &query_lo, &query_hi) {
                continue;
            }
            match node.kind {
                Kind::Leaf { start, count } => {
                    for pos in start..start + count {
                        let candidate = &boxes[self.order[pos]];
                        if !bbox_hit(&candidate.min_mm, &candidate.max_mm, &query_lo, &query_hi) {
                            continue;
                        }
                        if let Some(hit) = test_swept_box_i(candidate, &cast, &half_mm) {
                            fill.push(hit);
                        }
                    }
                }
                Kind::Inner { left, right } => {
                    // Same fixed-stack bound as the thin cast: median-split
                    // depth halves every level, so the guard below never
                    // fires; it keeps the cast total without a panic site.
                    if top + 2 > STACK_SLOTS {
                        debug_assert!(false, "median-split index depth fits the fixed stack");
                        continue;
                    }
                    stack[top] = left;
                    stack[top + 1] = right;
                    top += 2;
                }
            }
        }
        fill.finish()
    }
}

/// Oracle: linear scan with the same ordering and error semantics as
/// [`QueryBvh::cast_into`]. Tests assert hit-for-hit equality; callers with
/// tiny box counts may use it directly.
///
/// Determinism: slice order decides ties beyond `(dist_mm, collider)` only
/// through the final sort, so any permutation of `boxes` gives the same
/// bytes. Allocation: none.
pub fn brute_cast_into(
    boxes: &[BoxMm],
    seg: SegmentMm,
    out: &mut [HitMm],
) -> Result<usize, QueryError> {
    let mut fill = Fill {
        out,
        stored: 0,
        total: 0,
    };
    let cast = Cast::of(&seg);
    for candidate in boxes {
        if let Some(hit) = test_box_i(candidate, &cast) {
            fill.push(hit);
        }
    }
    fill.finish()
}

/// Swept oracle: linear scan with the same ordering and error semantics as
/// [`QueryBvh::sweep_box_into`]. Tests assert hit-for-hit equality; a zero
/// half extent agrees with [`brute_cast_into`] hit for hit.
///
/// Determinism: slice order decides ties beyond `(dist_mm, collider)` only
/// through the final sort, so any permutation of `boxes` gives the same
/// bytes. Allocation: none.
pub fn brute_sweep_box_into(
    boxes: &[BoxMm],
    center: SegmentMm,
    half_mm: [u32; 3],
    out: &mut [HitMm],
) -> Result<usize, QueryError> {
    let mut fill = Fill {
        out,
        stored: 0,
        total: 0,
    };
    let cast = Cast::of(&center);
    for candidate in boxes {
        if let Some(hit) = test_swept_box_i(candidate, &cast, &half_mm) {
            fill.push(hit);
        }
    }
    fill.finish()
}

/// One index node: shared bounds plus a leaf range or two child indices.
#[derive(Clone, Copy, Debug)]
struct BvhNode {
    min_mm: [i32; 3],
    max_mm: [i32; 3],
    kind: Kind,
}

/// Leaf holds an `order` range; inner holds child indices with the parent
/// pushed last, so the root is the final node.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Leaf { start: usize, count: usize },
    Inner { left: usize, right: usize },
}

/// One cast with integer endpoints pre-widened: built once per cast so the
/// per-box slab test only reads them, never converts. The segment bounding
/// box rejects far-apart nodes and boxes with six integer compares before
/// the exact slab test runs; a disjoint pair can never hit, so pruning is
/// exact and the oracle still agrees hit for hit.
struct Cast {
    from_mm: [i32; 3],
    from: [i64; 3],
    edge: [i64; 3],
    lo_mm: [i32; 3],
    hi_mm: [i32; 3],
}

impl Cast {
    fn of(seg: &SegmentMm) -> Self {
        let mut cast = Self {
            from_mm: [0; 3],
            from: [0; 3],
            edge: [0; 3],
            lo_mm: [0; 3],
            hi_mm: [0; 3],
        };
        for axis in 0..3 {
            cast.from_mm[axis] = seg.from_mm[axis];
            cast.from[axis] = i64::from(seg.from_mm[axis]);
            cast.edge[axis] = i64::from(seg.to_mm[axis]) - cast.from[axis];
            if seg.from_mm[axis] <= seg.to_mm[axis] {
                cast.lo_mm[axis] = seg.from_mm[axis];
                cast.hi_mm[axis] = seg.to_mm[axis];
            } else {
                cast.lo_mm[axis] = seg.to_mm[axis];
                cast.hi_mm[axis] = seg.from_mm[axis];
            }
        }
        cast
    }
}

/// Single-pass caller-buffer writer: stores while the buffer fits, counts
/// every hit, sorts only when nothing was dropped.
struct Fill<'o> {
    out: &'o mut [HitMm],
    stored: usize,
    total: usize,
}

impl Fill<'_> {
    fn push(&mut self, hit: HitMm) {
        if self.stored < self.out.len() {
            self.out[self.stored] = hit;
            self.stored += 1;
        }
        self.total += 1;
    }

    fn finish(self) -> Result<usize, QueryError> {
        if self.total > self.out.len() {
            return Err(QueryError::BufferTooSmall {
                needed: self.total,
                capacity: self.out.len(),
            });
        }
        self.out[..self.stored].sort_unstable();
        Ok(self.stored)
    }
}

/// Recursive median split of `order[start..start + count]`; returns the new
/// node index. Depth halves every level, so any input length fits the stack.
fn split(
    boxes: &[BoxMm],
    order: &mut [usize],
    nodes: &mut Vec<BvhNode>,
    start: usize,
    count: usize,
) -> usize {
    let mut min_mm = [i32::MAX; 3];
    let mut max_mm = [i32::MIN; 3];
    for pos in start..start + count {
        let candidate = &boxes[order[pos]];
        for axis in 0..3 {
            if candidate.min_mm[axis] < min_mm[axis] {
                min_mm[axis] = candidate.min_mm[axis];
            }
            if candidate.max_mm[axis] > max_mm[axis] {
                max_mm[axis] = candidate.max_mm[axis];
            }
        }
    }
    if count <= LEAF_BOXES {
        nodes.push(BvhNode {
            min_mm,
            max_mm,
            kind: Kind::Leaf { start, count },
        });
        return nodes.len() - 1;
    }
    let mut axis = 0usize;
    let mut widest = i64::from(max_mm[0]) - i64::from(min_mm[0]);
    for rest in 1..3 {
        let extent = i64::from(max_mm[rest]) - i64::from(min_mm[rest]);
        if extent > widest {
            widest = extent;
            axis = rest;
        }
    }
    order[start..start + count].sort_unstable_by(|&x, &y| {
        let left = &boxes[x];
        let right = &boxes[y];
        let mid_left = i64::from(left.min_mm[axis]) + i64::from(left.max_mm[axis]);
        let mid_right = i64::from(right.min_mm[axis]) + i64::from(right.max_mm[axis]);
        mid_left
            .cmp(&mid_right)
            .then(left.collider.cmp(&right.collider))
    });
    let mid = count / 2;
    let left = split(boxes, order, nodes, start, mid);
    let right = split(boxes, order, nodes, start + mid, count - mid);
    nodes.push(BvhNode {
        min_mm,
        max_mm,
        kind: Kind::Inner { left, right },
    });
    nodes.len() - 1
}

/// Integer bounding-box overlap on whole millimetres: disjoint boxes share
/// no point, so a segment boxed by one side can never hit the other.
/// Six compares, no multiplies; the exact slab test still decides hits.
fn bbox_hit(a_min: &[i32; 3], a_max: &[i32; 3], b_min: &[i32; 3], b_max: &[i32; 3]) -> bool {
    for axis in 0..3 {
        if a_max[axis] < b_min[axis] || b_max[axis] < a_min[axis] {
            return false;
        }
    }
    true
}

/// Entry/exit segment parameters as fractions with positive denominators.
#[derive(Clone, Copy, Debug)]
struct Span {
    entry_num: i64,
    entry_den: i64,
    exit_num: i64,
    exit_den: i64,
}

/// Integer slab test: the entry/exit parameters of the pre-widened cast
/// against the box, each as a fraction with a positive denominator.
/// `None` on a miss. Cross products run in `i128` so the full `i32`
/// millimetre cube stays exact.
fn slab_i(min_mm: &[i32; 3], max_mm: &[i32; 3], cast: &Cast) -> Option<Span> {
    let mut span = Span {
        entry_num: 0,
        entry_den: 1,
        exit_num: 1,
        exit_den: 1,
    };
    for axis in 0..3 {
        let from = cast.from[axis];
        let edge = cast.edge[axis];
        let low = i64::from(min_mm[axis]) - from;
        let high = i64::from(max_mm[axis]) - from;
        if edge == 0 {
            if from < i64::from(min_mm[axis]) || from > i64::from(max_mm[axis]) {
                return None;
            }
            continue;
        }
        // Denominators stay positive: flip both signs when the edge runs
        // backward. Negations fit easily (values stay near `i32` range).
        let (near_num, near_den, far_num, far_den) = if edge > 0 {
            (low, edge, high, edge)
        } else {
            (-high, -edge, -low, -edge)
        };
        if i128::from(near_num) * i128::from(span.entry_den)
            > i128::from(span.entry_num) * i128::from(near_den)
        {
            span.entry_num = near_num;
            span.entry_den = near_den;
        }
        if i128::from(far_num) * i128::from(span.exit_den)
            < i128::from(span.exit_num) * i128::from(far_den)
        {
            span.exit_num = far_num;
            span.exit_den = far_den;
        }
    }
    if i128::from(span.entry_num) * i128::from(span.exit_den)
        > i128::from(span.exit_num) * i128::from(span.entry_den)
    {
        return None;
    }
    Some(span)
}

/// Segment coordinate on one axis at parameter `num / den`, rounded to the
/// nearest millimetre (half away from zero). The caller passes the clamped
/// span, so `0 <= num <= den` with `den > 0`: the scaled product fits `u64`
/// and the division is one hardware step.
fn locate_i(from: i64, edge: i64, num: i64, den: i64) -> i32 {
    let ahead = edge >= 0;
    let mag = edge.unsigned_abs();
    // Clamped parameters keep `num <= den`, so this product fits `u64`.
    // The fallbacks below are unreachable on the clamped span; each keeps
    // the function total without a panic site and debug-asserts in tests.
    let step = u64::try_from(num).unwrap_or_else(|_| {
        debug_assert!(false, "clamped parameter is non-negative");
        0
    });
    let span = u64::try_from(den).unwrap_or_else(|_| {
        debug_assert!(false, "span denominators stay positive");
        1
    });
    let scaled = mag * step;
    // Round half up without a remainder branch: adding half the span can
    // exceed `u64` for the largest edges, so the biased sum widens to
    // `u128`. The quotient stays below the edge magnitude plus one.
    let biased = u128::from(scaled) + u128::from(span) / 2;
    let whole = u64::try_from(biased / u128::from(span)).unwrap_or_else(|_| {
        debug_assert!(false, "rounded offset fits the edge");
        0
    });
    // The rounded offset never exceeds the edge magnitude.
    let off = i64::try_from(whole).unwrap_or_else(|_| {
        debug_assert!(false, "blend offset fits the edge");
        0
    });
    let coord = if ahead { from + off } else { from - off };
    // A clamped blend of two `i32` corners, rounded by under a millimetre,
    // always lands back inside the `i32` range.
    i32::try_from(coord).unwrap_or_else(|_| {
        debug_assert!(false, "segment-box blend stays inside i32 range");
        if ahead {
            i32::MAX
        } else {
            i32::MIN
        }
    })
}

/// Whole-millimetre travel from `from_mm` to `at_mm` via integer `isqrt`.
fn dist_norm(from_mm: &[i32; 3], at_mm: &[i32; 3]) -> u64 {
    let mut sum = 0u128;
    for axis in 0..3 {
        let edge = i64::from(at_mm[axis]) - i64::from(from_mm[axis]);
        // An `i32` difference fits `u32`, so its square fits `u64`.
        let limb = edge.unsigned_abs();
        sum += u128::from(limb * limb);
    }
    // The norm of an `i32`-cube diagonal stays far below `u64::MAX`.
    u64::try_from(sum.isqrt()).unwrap_or_else(|_| {
        debug_assert!(false, "integer-mm norm fits u64");
        u64::MAX
    })
}

/// Slab-test one box, attaching the caller ids on a hit.
fn test_box_i(candidate: &BoxMm, cast: &Cast) -> Option<HitMm> {
    let span = slab_i(&candidate.min_mm, &candidate.max_mm, cast)?;
    let mut entry_mm = [0i32; 3];
    let mut exit_mm = [0i32; 3];
    for axis in 0..3 {
        entry_mm[axis] = locate_i(
            cast.from[axis],
            cast.edge[axis],
            span.entry_num,
            span.entry_den,
        );
        exit_mm[axis] = locate_i(
            cast.from[axis],
            cast.edge[axis],
            span.exit_num,
            span.exit_den,
        );
    }
    Some(HitMm {
        collider: candidate.collider,
        material: candidate.material,
        dist_mm: dist_norm(&cast.from_mm, &entry_mm),
        entry_mm,
        exit_mm,
    })
}

/// Lower grown corner, saturating at the `i32` floor so extreme boxes near
/// the range edge stay conservative instead of refusing the cast.
fn widen_low(value: i32, half: u32) -> i32 {
    let grown = i64::from(value) - i64::from(half);
    let clamped = grown.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(clamped).unwrap_or_else(|_| {
        debug_assert!(false, "widened corner stays inside the i32 range");
        i32::MIN
    })
}

/// Upper grown corner, saturating at the `i32` ceiling; see [`widen_low`].
fn widen_high(value: i32, half: u32) -> i32 {
    let grown = i64::from(value) + i64::from(half);
    let clamped = grown.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(clamped).unwrap_or_else(|_| {
        debug_assert!(false, "widened corner stays inside the i32 range");
        i32::MAX
    })
}

/// Slab-test one grown box: the Minkowski sum of `candidate` with the moving
/// half extents, cast by the centre segment. Entry, exit and distance report
/// the centre travel; a zero half extent reduces to [`test_box_i`] exactly.
fn test_swept_box_i(candidate: &BoxMm, cast: &Cast, half_mm: &[u32; 3]) -> Option<HitMm> {
    let mut grown_min = [0i32; 3];
    let mut grown_max = [0i32; 3];
    for axis in 0..3 {
        grown_min[axis] = widen_low(candidate.min_mm[axis], half_mm[axis]);
        grown_max[axis] = widen_high(candidate.max_mm[axis], half_mm[axis]);
    }
    let span = slab_i(&grown_min, &grown_max, cast)?;
    let mut entry_mm = [0i32; 3];
    let mut exit_mm = [0i32; 3];
    for axis in 0..3 {
        entry_mm[axis] = locate_i(
            cast.from[axis],
            cast.edge[axis],
            span.entry_num,
            span.entry_den,
        );
        exit_mm[axis] = locate_i(
            cast.from[axis],
            cast.edge[axis],
            span.exit_num,
            span.exit_den,
        );
    }
    Some(HitMm {
        collider: candidate.collider,
        material: candidate.material,
        dist_mm: dist_norm(&cast.from_mm, &entry_mm),
        entry_mm,
        exit_mm,
    })
}

/// Slide tuning for the cast-driven move below, all integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideConfig {
    /// Highest repeats; rest travel in whole millimetres.
    pub max_iterations: u8,
    pub max_residual_mm: u32,
    /// Tallest stepped rise in whole millimetres.
    pub step_height_mm: i32,
    /// Walkable floor gate, Q16 normal height.
    pub walkable_y_q16: i32,
}

/// Remainder of `vel_mm` after removing approach into `normal_mm`.
/// Integer only with widened dots; a zero normal or a separating
/// velocity passes through untouched.
pub fn project_velocity_mm(vel_mm: [i32; 3], normal_mm: [i32; 3]) -> [i32; 3] {
    let mut dot = 0i64;
    let mut norm2 = 0i64;
    for axis in 0..3 {
        dot += i64::from(vel_mm[axis]) * i64::from(normal_mm[axis]);
        norm2 += i64::from(normal_mm[axis]) * i64::from(normal_mm[axis]);
    }
    if dot >= 0 || norm2 <= 0 {
        return vel_mm;
    }
    let mut kept = [0i32; 3];
    for axis in 0..3 {
        let cut = i64::from(normal_mm[axis]) * dot / norm2;
        kept[axis] = narrow(i64::from(vel_mm[axis]) - cut);
    }
    kept
}

/// Push `pos_mm` out of overlapped boxes, smallest face shift.
///
/// Integer only saturated at the `i32` edge; touching faces stay put.
/// Callers pass id-sorted slices so ties decide identically everywhere.
pub fn depenetrate_point_mm(
    mut pos_mm: [i32; 3],
    half_mm: [u32; 3],
    boxes: &[BoxMm],
    skin_mm: i32,
) -> [i32; 3] {
    for candidate in boxes {
        let mut pen = [0i64; 3];
        let mut hit = true;
        for axis in 0..3 {
            let lo = i64::from(pos_mm[axis]) - i64::from(half_mm[axis]);
            let hi = i64::from(pos_mm[axis]) + i64::from(half_mm[axis]);
            if hi <= i64::from(candidate.min_mm[axis]) || lo >= i64::from(candidate.max_mm[axis]) {
                hit = false;
                break;
            }
            pen[axis] = (hi - i64::from(candidate.min_mm[axis]))
                .min(i64::from(candidate.max_mm[axis]) - lo);
        }
        if !hit {
            continue;
        }
        let mut axis = 0usize;
        for rest in 1..3 {
            if pen[rest] < pen[axis] {
                axis = rest;
            }
        }
        let lo = i64::from(pos_mm[axis]) - i64::from(half_mm[axis]);
        let hi = i64::from(pos_mm[axis]) + i64::from(half_mm[axis]);
        let near_min =
            hi - i64::from(candidate.min_mm[axis]) <= i64::from(candidate.max_mm[axis]) - lo;
        let push = if near_min {
            -(pen[axis] + i64::from(skin_mm))
        } else {
            pen[axis] + i64::from(skin_mm)
        };
        let at = i64::from(pos_mm[axis]) + push;
        pos_mm[axis] = narrow(at);
    }
    pos_mm
}

fn narrow(v: i64) -> i32 {
    i32::try_from(v).unwrap_or_else(|_| if v < 0 { i32::MIN } else { i32::MAX })
}

/// Grown face holding `entry_mm`: per axis -1 on the low face, 1 on the
/// high face, 0 inside. Integer only; the caller decides the interior case.
fn grown_face_mm(candidate: &BoxMm, entry_mm: &[i32; 3], half_mm: &[u32; 3]) -> [i32; 3] {
    let mut face = [0i32; 3];
    for axis in 0..3 {
        let glo = i64::from(candidate.min_mm[axis]) - i64::from(half_mm[axis]);
        let ghi = i64::from(candidate.max_mm[axis]) + i64::from(half_mm[axis]);
        if i64::from(entry_mm[axis]) == glo {
            face[axis] = -1;
        } else if i64::from(entry_mm[axis]) == ghi {
            face[axis] = 1;
        }
    }
    face
}

/// True when the centre already rests on the grown face and the rest of the
/// move runs along it or away from it, so the contact must not block.
/// Integer dot only; approaching faces stay false and still win by distance.
fn rests_without_approach(
    candidate: &BoxMm,
    entry_mm: &[i32; 3],
    dest_mm: &[i32; 3],
    half_mm: &[u32; 3],
) -> bool {
    let face = grown_face_mm(candidate, entry_mm, half_mm);
    if face == [0, 0, 0] {
        return false;
    }
    let mut dot = 0i64;
    for axis in 0..3 {
        dot += (i64::from(dest_mm[axis]) - i64::from(entry_mm[axis])) * i64::from(face[axis]);
    }
    dot >= 0
}

/// Cast-driven move: depenetrate, sweep to contact, project, repeat.
///
/// Integer only over the slab helpers above; `hook` sees each contact
/// with its face normal. Low rises step, tall faces slide.
pub fn slide_move_mm(
    boxes: &[BoxMm],
    cfg: &SlideConfig,
    from_mm: [i32; 3],
    vel_mm: [i32; 3],
    half_mm: [u32; 3],
    hook: &mut impl FnMut(&HitMm, [i32; 3]),
) -> [i32; 3] {
    let mut pos_mm = depenetrate_point_mm(from_mm, half_mm, boxes, 1);
    let mut remaining = vel_mm;
    let mut done = 0u8;
    while done < cfg.max_iterations {
        if dist_norm(&[0, 0, 0], &remaining) <= u64::from(cfg.max_residual_mm) {
            break;
        }
        let mut dest = [0i32; 3];
        for axis in 0..3 {
            dest[axis] = narrow(i64::from(pos_mm[axis]) + i64::from(remaining[axis]));
        }
        let cast = Cast::of(&SegmentMm {
            from_mm: pos_mm,
            to_mm: dest,
        });
        let mut best: Option<HitMm> = None;
        for candidate in boxes {
            if let Some(hit) = test_swept_box_i(candidate, &cast, &half_mm) {
                // A resting centre must not block tangential travel: keeping
                // the entry-point hit would pin the drive on every repeat.
                if hit.entry_mm == pos_mm
                    && rests_without_approach(candidate, &hit.entry_mm, &dest, &half_mm)
                {
                    continue;
                }
                let take = match best {
                    None => true,
                    Some(prev) => hit < prev,
                };
                if take {
                    best = Some(hit);
                }
            }
        }
        let hit = match best {
            None => {
                pos_mm = dest;
                break;
            }
            Some(found) => found,
        };
        let mut face = [0i32; 3];
        let mut top = 0i32;
        let mut found_box = false;
        for candidate in boxes {
            if candidate.collider == hit.collider {
                top = candidate.max_mm[1];
                found_box = true;
                face = grown_face_mm(candidate, &hit.entry_mm, &half_mm);
                break;
            }
        }
        if !found_box {
            pos_mm = hit.entry_mm;
            break;
        }
        if face == [0, 0, 0] {
            face = [0, 1, 0];
        }
        hook(&hit, face);
        let mut rest = [0i32; 3];
        for axis in 0..3 {
            rest[axis] = narrow(i64::from(dest[axis]) - i64::from(hit.entry_mm[axis]));
        }
        let feet = i64::from(pos_mm[1]) - i64::from(half_mm[1]);
        let rise = i64::from(top) - feet;
        let walkable = i64::from(face[1]) * 65536 >= i64::from(cfg.walkable_y_q16);
        if face[1] == 0 && !walkable && rise > 0 && rise <= i64::from(cfg.step_height_mm) {
            let lifted = i64::from(top) + i64::from(half_mm[1]) + 1;
            pos_mm = [hit.entry_mm[0], narrow(lifted), hit.entry_mm[2]];
            rest[1] = 0;
            remaining = rest;
        } else {
            pos_mm = hit.entry_mm;
            remaining = project_velocity_mm(rest, face);
        }
        done += 1;
    }
    depenetrate_point_mm(pos_mm, half_mm, boxes, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use engine_core::{Fnv64, Rng};

    fn one_box() -> [BoxMm; 1] {
        [BoxMm::from_mm([0, 0, 0], [1000, 1000, 1000], 7, 3).expect("test corners ordered")]
    }

    fn through() -> SegmentMm {
        SegmentMm {
            from_mm: [-500, 500, 500],
            to_mm: [1500, 500, 500],
        }
    }

    fn draw_mm(rng: &mut Rng, low: i32, high: i32) -> i32 {
        let span = i64::from(high) - i64::from(low);
        assert!(span > 0, "test range ordered");
        let draw = rng.next_u64() % u64::try_from(span).expect("span fits u64");
        i32::try_from(i64::from(low) + i64::try_from(draw).expect("draw fits i64"))
            .expect("drawn value fits i32")
    }

    fn city_boxes(rng: &mut Rng, buildings: u64, per_building: u64) -> Vec<BoxMm> {
        let mut out = Vec::new();
        let mut id = 0u32;
        for _ in 0..buildings {
            let base_x = draw_mm(rng, -60_000, 60_000);
            let base_z = draw_mm(rng, -60_000, 60_000);
            for _ in 0..per_building {
                let size_x = draw_mm(rng, 2_000, 12_000);
                let size_y = draw_mm(rng, 2_000, 30_000);
                let size_z = draw_mm(rng, 2_000, 12_000);
                let corner_x = base_x + draw_mm(rng, -15_000, 15_000);
                let corner_y = draw_mm(rng, 0, 40_000);
                let corner_z = base_z + draw_mm(rng, -15_000, 15_000);
                let material = u16::try_from(id % 8).expect("table slot fits u16");
                out.push(
                    BoxMm::from_mm(
                        [corner_x, corner_y, corner_z],
                        [corner_x + size_x, corner_y + size_y, corner_z + size_z],
                        id,
                        material,
                    )
                    .expect("grown corners stay ordered"),
                );
                id += 1;
            }
        }
        out
    }

    fn city_segs(rng: &mut Rng, count: usize) -> Vec<SegmentMm> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(SegmentMm {
                from_mm: [
                    draw_mm(rng, -60_000, 60_000),
                    draw_mm(rng, 0, 40_000),
                    draw_mm(rng, -60_000, 60_000),
                ],
                to_mm: [
                    draw_mm(rng, -60_000, 60_000),
                    draw_mm(rng, 0, 40_000),
                    draw_mm(rng, -60_000, 60_000),
                ],
            });
        }
        out
    }

    /// Tick-step round flight over the district below: short segments from
    /// anywhere a fighter can stand with a 2..20 m travel. One segment per
    /// round per tick is the per-tick caller pattern, so the budget leg
    /// mirrors it instead of spanning segments that cross the whole fixture.
    fn district_segs(rng: &mut Rng, count: usize) -> Vec<SegmentMm> {
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let from = [
                draw_mm(rng, -300_000, 300_000),
                draw_mm(rng, 0, 40_000),
                draw_mm(rng, -240_000, 240_000),
            ];
            let step = [
                draw_mm(rng, -20_000, 20_000),
                draw_mm(rng, -20_000, 20_000),
                draw_mm(rng, -20_000, 20_000),
            ];
            let mut len2 = 0i64;
            for limb in step {
                let edge = i64::from(limb);
                len2 += edge * edge;
            }
            if !(2000i64 * 2000..=20_000i64 * 20_000).contains(&len2) {
                continue;
            }
            out.push(SegmentMm {
                from_mm: from,
                to_mm: [from[0] + step[0], from[1] + step[1], from[2] + step[2]],
            });
        }
        out
    }

    /// Budget-district boxes: 20 buildings on a jittered 90 m grid over a
    /// ~600 x ~480 m district, 12 part-boxes each, streets between them.
    /// Packing all 20 buildings into one 120 m block (as an early draft of
    /// this fixture did) stacks every footprint onto its neighbours, so
    /// each segment bbox-overlaps dozens of boxes and the leg prices a
    /// wall instead of the per-tick pattern; the district spacing prices
    /// what callers actually cast through.
    fn district_boxes(rng: &mut Rng, buildings: u64, per_building: u64) -> Vec<BoxMm> {
        let mut out = Vec::new();
        let mut id = 0u32;
        for serial in 0..buildings {
            let grid_x = i32::try_from(serial % 5).expect("grid column fits i32") - 2;
            let grid_z = i32::try_from(serial / 5).expect("grid row fits i32") - 1;
            let base_x = grid_x * 90_000 + draw_mm(rng, -20_000, 20_000);
            let base_z = grid_z * 90_000 + draw_mm(rng, -20_000, 20_000);
            for _ in 0..per_building {
                let size_x = draw_mm(rng, 2_000, 12_000);
                let size_y = draw_mm(rng, 2_000, 30_000);
                let size_z = draw_mm(rng, 2_000, 12_000);
                let corner_x = base_x + draw_mm(rng, -15_000, 15_000);
                let corner_y = draw_mm(rng, 0, 40_000);
                let corner_z = base_z + draw_mm(rng, -15_000, 15_000);
                let material = u16::try_from(id % 8).expect("table slot fits u16");
                out.push(
                    BoxMm::from_mm(
                        [corner_x, corner_y, corner_z],
                        [corner_x + size_x, corner_y + size_y, corner_z + size_z],
                        id,
                        material,
                    )
                    .expect("grown corners stay ordered"),
                );
                id += 1;
            }
        }
        out
    }

    fn sweep_hash(index: &QueryBvh, boxes: &[BoxMm], segs: &[SegmentMm]) -> (u64, usize) {
        let mut buf = [HitMm::EMPTY; 256];
        let mut oracle = [HitMm::EMPTY; 256];
        let mut hasher = Fnv64::new();
        let mut total = 0usize;
        for seg in segs {
            let found = index
                .cast_into(boxes, *seg, &mut buf)
                .expect("fixture fits the caller buffer");
            let wanted =
                brute_cast_into(boxes, *seg, &mut oracle).expect("fixture fits the caller buffer");
            assert_eq!(found, wanted, "index and oracle agree on hit count");
            assert_eq!(
                &buf[..found],
                &oracle[..wanted],
                "index and oracle agree on hits"
            );
            for hit in &buf[..found] {
                hasher.write_u32(hit.collider);
                hasher.write_u16(hit.material);
                hasher.write_u64(hit.dist_mm);
                for axis in 0..3 {
                    hasher.write_i32(hit.entry_mm[axis]);
                    hasher.write_i32(hit.exit_mm[axis]);
                }
            }
            total += found;
        }
        (hasher.finish(), total)
    }

    #[test]
    fn axis_entry_exit_and_material() {
        let boxes = one_box();
        let index = QueryBvh::build(&boxes);
        let mut out = [HitMm::EMPTY; 4];
        let n = index
            .cast_into(&boxes, through(), &mut out)
            .expect("buffer fits");
        assert_eq!(n, 1, "{out:?}");
        assert_eq!(out[0].collider, 7);
        assert_eq!(out[0].material, 3);
        assert_eq!(out[0].dist_mm, 500);
        assert_eq!(out[0].entry_mm, [0, 500, 500]);
        assert_eq!(out[0].exit_mm, [1000, 500, 500]);
        let back = SegmentMm {
            from_mm: [1500, 500, 500],
            to_mm: [-500, 500, 500],
        };
        let m = index
            .cast_into(&boxes, back, &mut out)
            .expect("buffer fits");
        assert_eq!(m, 1, "{out:?}");
        assert_eq!(out[0].entry_mm, [1000, 500, 500]);
        assert_eq!(out[0].dist_mm, 500);
    }

    #[test]
    fn grazing_parallel_point_and_inside_start() {
        let boxes = one_box();
        let index = QueryBvh::build(&boxes);
        let mut out = [HitMm::EMPTY; 4];
        let above = SegmentMm {
            from_mm: [-500, 1001, 500],
            to_mm: [1500, 1001, 500],
        };
        assert_eq!(index.cast_into(&boxes, above, &mut out).expect("fits"), 0);
        let point_in = SegmentMm {
            from_mm: [250, 250, 250],
            to_mm: [250, 250, 250],
        };
        let n = index.cast_into(&boxes, point_in, &mut out).expect("fits");
        assert_eq!(n, 1, "{out:?}");
        assert_eq!(out[0].dist_mm, 0);
        assert_eq!(out[0].entry_mm, [250, 250, 250]);
        assert_eq!(out[0].exit_mm, [250, 250, 250]);
        let point_out = SegmentMm {
            from_mm: [2500, 250, 250],
            to_mm: [2500, 250, 250],
        };
        assert_eq!(
            index.cast_into(&boxes, point_out, &mut out).expect("fits"),
            0
        );
        let inside = SegmentMm {
            from_mm: [250, 500, 500],
            to_mm: [1500, 500, 500],
        };
        let k = index.cast_into(&boxes, inside, &mut out).expect("fits");
        assert_eq!(k, 1, "{out:?}");
        assert_eq!(out[0].dist_mm, 0);
        assert_eq!(out[0].entry_mm, [250, 500, 500]);
        assert_eq!(out[0].exit_mm, [1000, 500, 500]);
    }

    #[test]
    fn adapters_refuse_and_convert() {
        assert!(BoxMm::from_mm([1, 0, 0], [0, 1, 1], 0, 0).is_none());
        let capsule =
            BoxMm::from_capsule_mm([0, 0, 0], [1000, 0, 0], 250, 5, 1).expect("grown corners fit");
        assert_eq!(capsule.min_mm, [-250, -250, -250]);
        assert_eq!(capsule.max_mm, [1250, 250, 250]);
        assert!(BoxMm::from_capsule_mm([0, 0, 0], [0, 0, 0], u32::MAX, 0, 0).is_none());
    }

    #[test]
    fn small_buffer_names_needed_and_stale_build_refused() {
        let boxes = [
            BoxMm::from_mm([0, 0, 0], [1000, 1000, 1000], 1, 0).expect("ordered"),
            BoxMm::from_mm([2000, 0, 0], [3000, 1000, 1000], 2, 0).expect("ordered"),
        ];
        let index = QueryBvh::build(&boxes);
        let seg = SegmentMm {
            from_mm: [-500, 500, 500],
            to_mm: [3500, 500, 500],
        };
        let mut tight = [HitMm::EMPTY; 1];
        let err = index
            .cast_into(&boxes, seg, &mut tight)
            .expect_err("one slot cannot hold two hits");
        assert_eq!(
            err,
            QueryError::BufferTooSmall {
                needed: 2,
                capacity: 1
            }
        );
        let mut roomy = [HitMm::EMPTY; 2];
        let n = index.cast_into(&boxes, seg, &mut roomy).expect("fits");
        assert_eq!(n, 2);
        assert_eq!([roomy[0].collider, roomy[1].collider], [1, 2]);
        let stale = index
            .cast_into(&boxes[..1], seg, &mut roomy)
            .expect_err("length mismatch refused");
        assert_eq!(stale, QueryError::StaleBuild { built: 2, given: 1 });
    }

    #[test]
    fn sorted_by_distance_then_collider() {
        // Near, far and twin boxes: the twins share an entry distance, so
        // the collider id decides their order.
        let boxes = [
            BoxMm::from_mm([4000, 0, 0], [5000, 1000, 1000], 30, 0).expect("ordered"),
            BoxMm::from_mm([1000, 0, 0], [2000, 1000, 1000], 20, 0).expect("ordered"),
            BoxMm::from_mm([1000, 0, 0], [2000, 1000, 1000], 10, 0).expect("ordered"),
        ];
        let index = QueryBvh::build(&boxes);
        let seg = SegmentMm {
            from_mm: [-500, 500, 500],
            to_mm: [6000, 500, 500],
        };
        let mut out = [HitMm::EMPTY; 8];
        let n = index.cast_into(&boxes, seg, &mut out).expect("fits");
        assert_eq!(n, 3);
        assert_eq!(
            [out[0].collider, out[1].collider, out[2].collider],
            [10, 20, 30]
        );
        assert_eq!(out[0].dist_mm, out[1].dist_mm);
        assert!(out[1].dist_mm < out[2].dist_mm);
    }

    #[test]
    fn slide_matrix_steps_slopes_platforms_hash_x2() {
        let cfg = SlideConfig {
            max_iterations: 4,
            max_residual_mm: 1,
            step_height_mm: 500,
            walkable_y_q16: 46340,
        };
        let half = [150u32, 800u32, 150u32];
        let step_box = BoxMm::from_mm([2000, 0, -1000], [3000, 250, 1000], 1, 0).expect("ordered");
        let wall_box = BoxMm::from_mm([2000, 0, -1000], [3000, 2000, 1000], 2, 0).expect("ordered");
        let floor = BoxMm::from_mm([-4000, -500, -4000], [4000, 0, 4000], 3, 0).expect("ordered");
        let mut hook = |_: &HitMm, _: [i32; 3]| {};
        let stepped = slide_move_mm(
            &[step_box],
            &cfg,
            [0, 800, 0],
            [3000, 0, 0],
            half,
            &mut hook,
        );
        assert_eq!(stepped, [3000, 1051, 0], "low rise steps, {stepped:?}");
        let mut face_seen = [0i32; 3];
        let mut wall_hook = |_: &HitMm, normal: [i32; 3]| {
            face_seen = normal;
        };
        let blocked = slide_move_mm(
            &[wall_box],
            &cfg,
            [0, 800, 0],
            [3000, 0, 0],
            half,
            &mut wall_hook,
        );
        assert_eq!(face_seen, [-1, 0, 0], "wall face");
        assert_eq!(blocked, [1850, 800, 0], "tall face slides, {blocked:?}");
        let floored = slide_move_mm(
            &[floor],
            &cfg,
            [0, 1500, 0],
            [2000, -1500, 0],
            half,
            &mut hook,
        );
        assert_eq!(floored, [2000, 800, 0], "floor keeps drive, {floored:?}");
        let steep = slide_move_mm(
            &[wall_box],
            &cfg,
            [0, 800, 0],
            [3000, -500, 0],
            half,
            &mut hook,
        );
        assert_eq!(steep, [1850, 300, 0], "over-steep keeps falling, {steep:?}");
        let plats = [
            BoxMm::from_mm([1000, 0, -500], [2000, 1000, 500], 11, 0).expect("ordered"),
            BoxMm::from_mm([3000, 0, -500], [4000, 1000, 500], 22, 0).expect("ordered"),
            BoxMm::from_mm([5000, 0, -500], [6000, 1000, 500], 33, 0).expect("ordered"),
        ];
        let seg = SegmentMm {
            from_mm: [-500, 500, 0],
            to_mm: [6500, 500, 0],
        };
        let index = QueryBvh::build(&plats);
        let mut out = [HitMm::EMPTY; 8];
        let n = index.cast_into(&plats, seg, &mut out).expect("fits");
        assert_eq!(n, 3);
        assert_eq!(
            [out[0].collider, out[1].collider, out[2].collider],
            [11, 22, 33]
        );
        let rev = [plats[2], plats[1], plats[0]];
        let rebuilt = QueryBvh::build(&rev);
        let mut again = [HitMm::EMPTY; 8];
        let m = rebuilt.cast_into(&rev, seg, &mut again).expect("fits");
        assert_eq!(m, 3);
        assert_eq!(
            [again[0].collider, again[1].collider, again[2].collider],
            [11, 22, 33]
        );
        let world = [step_box, wall_box, floor];
        let run = |fork: u64| {
            let seed = Rng::new(2040);
            let mut rng = seed.fork(fork);
            let mut hasher = Fnv64::new();
            let mut contacts = 0u32;
            let mut i = 0usize;
            while i < 1000 {
                let from = [
                    draw_mm(&mut rng, -4000, 4000),
                    draw_mm(&mut rng, 0, 3000),
                    draw_mm(&mut rng, -2000, 2000),
                ];
                let vel = [
                    draw_mm(&mut rng, -3000, 3000),
                    draw_mm(&mut rng, -3000, 3000),
                    draw_mm(&mut rng, -500, 500),
                ];
                let mut hook = |hit: &HitMm, normal: [i32; 3]| {
                    contacts += 1;
                    hasher.write_u32(hit.collider);
                    for axis in 0..3 {
                        hasher.write_i32(normal[axis]);
                    }
                };
                let end = slide_move_mm(&world, &cfg, from, vel, half, &mut hook);
                for axis in 0..3 {
                    hasher.write_i32(end[axis]);
                }
                i += 1;
            }
            (hasher.finish(), contacts)
        };
        let (h1, c1) = run(31);
        let (h2, c2) = run(31);
        assert_eq!((h1, c1), (h2, c2), "seeded matrix hashes twice");
        assert!(c1 > 100, "matrix must land contacts, got {c1}");
    }

    #[test]
    fn empty_build_hits_nothing() {
        let boxes: [BoxMm; 0] = [];
        let index = QueryBvh::build(&boxes);
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        let mut out = [HitMm::EMPTY; 4];
        assert_eq!(
            index.cast_into(&boxes, through(), &mut out).expect("fits"),
            0
        );
    }

    #[test]
    fn index_matches_oracle_10k_and_hashes_twice() {
        let seed = Rng::new(2040);
        let boxes = city_boxes(&mut seed.fork(11), 20, 12);
        assert_eq!(boxes.len(), 240);
        let segments = city_segs(&mut seed.fork(12), 10_000);
        let index = QueryBvh::build(&boxes);
        let (first, total) = sweep_hash(&index, &boxes, &segments);
        assert!(total > 5_000, "fixture must land hits, got {total}");
        let rebuilt = QueryBvh::build(&boxes);
        let (second, total_again) = sweep_hash(&rebuilt, &boxes, &segments);
        assert_eq!(total, total_again, "rebuild repeats the sweep");
        assert_eq!(first, second, "same inputs hash twice");
        println!("query parity: 10000 segs over 240 boxes, {total} hits, hash {first:016x}");
    }

    #[test]
    fn release_budget_2048_casts() {
        let seed = Rng::new(2040);
        let boxes = district_boxes(&mut seed.fork(21), 20, 12);
        let segments = district_segs(&mut seed.fork(22), 2048);
        let index = QueryBvh::build(&boxes);
        let mut buf = [HitMm::EMPTY; 256];
        for seg in segments.iter().take(64) {
            index
                .cast_into(&boxes, *seg, &mut buf)
                .expect("fixture fits the caller buffer");
        }
        let (hits, allocs) = alloc_probe::measure(|| {
            let mut hits = 0usize;
            for seg in &segments {
                hits += index
                    .cast_into(&boxes, *seg, &mut buf)
                    .expect("fixture fits the caller buffer");
            }
            hits
        });
        assert_eq!(allocs.calls, 0, "casts allocate nothing, got {allocs:?}");
        assert!(
            hits > 50,
            "fixture must exercise buildings, got {hits} hits"
        );
    }

    #[test]
    #[ignore = "timing: run under proof --perf"]
    #[expect(
        clippy::disallowed_methods,
        reason = "release budget probe reads wall time; the reading never enters simulation state or hashes"
    )]
    #[expect(
        clippy::disallowed_types,
        reason = "release budget probe reads wall time; the reading never enters simulation state or hashes"
    )]
    fn release_budget_2048_casts_under_1ms() {
        let seed = Rng::new(2040);
        let boxes = district_boxes(&mut seed.fork(21), 20, 12);
        let segments = district_segs(&mut seed.fork(22), 2048);
        let index = QueryBvh::build(&boxes);
        let mut buf = [HitMm::EMPTY; 256];
        for seg in segments.iter().take(64) {
            index
                .cast_into(&boxes, *seg, &mut buf)
                .expect("fixture fits the caller buffer");
        }
        let start = std::time::Instant::now();
        let mut hits = 0usize;
        for seg in &segments {
            hits += index
                .cast_into(&boxes, *seg, &mut buf)
                .expect("fixture fits the caller buffer");
        }
        let elapsed = start.elapsed();
        let ms = elapsed.as_secs_f64() * 1000.0;
        println!("query budget: 2048 tick steps over 240 boxes, {hits} hits, {ms:.3} ms");
        if !cfg!(debug_assertions) {
            assert!(ms <= 1.0, "2048 casts exceed the 1 ms budget: {ms:.3} ms");
        }
    }
}
