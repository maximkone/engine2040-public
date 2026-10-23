//! layout: integer-pixel panel layout over rows, gaps and margins.
//!
//! Consumer: the editor layout tool, the settings/menu screens and the
//! accessibility tree. A [`LayoutDoc`] is authored data; [`LayoutDoc::solve`]
//! turns it into exact [`Rect`]s for one [`Viewport`]. Data owner: `engine-ui`.
//! Units: pixels as `u32`, fractions as `num/den` weights of the space left
//! after fixed tracks, contrast in permille. Determinism: the solver walks
//! rows then panels in authored order and divides integers only; there is no
//! float accumulate, clock or RNG, so the same document and viewport give the
//! same rects and the same [`SolvedLayout::digest`]. Failure mode: bad
//! constraints, duplicate panel ids, content that does not fit, or an unknown
//! palette return [`LayoutError`]; nothing panics.
//!
//! Overflow is a typed error, never a clamp: it names the axis, the shortfall
//! in pixels, and the first panel of the caller's shrink order, which is the
//! one recovery the caller applies (shrink that panel, or widen the viewport).

use engine_core::checked::saturating_f64_to_u64;
use engine_core::Fnv64;

use super::access::hud_palettes;
use super::{contrast_ratio, HUD_TEXT_CONTRAST, MIN_CONTRAST};

/// Largest number of panels one document may hold.
pub const MAX_PANELS: usize = 64;
/// Largest number of rows one document may hold.
pub const MAX_ROWS: usize = 32;
/// Largest margin or track gap accepted, in pixels.
pub const MAX_GAP_PX: u32 = 512;

/// One axis of a layout, used to name which side overflowed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Axis {
    /// Left-to-right axis.
    Horizontal,
    /// Top-to-bottom axis.
    Vertical,
}

impl core::fmt::Display for Axis {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Horizontal => write!(f, "horizontal"),
            Self::Vertical => write!(f, "vertical"),
        }
    }
}

/// Panel identifier: the stable `u16` the solver echoes on each rect.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PanelId(pub u16);

/// How one row band or one panel spends the space it is given.
///
/// Fractions are weights: panels (or rows) that use a fraction share the
/// space left after the fixed, min/max and aspect tracks, in proportion to
/// `num`. Fractions on one axis must share a `den`, so the split stays
/// integer; different `den` values in one row are refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Constraint {
    /// Exact pixel size.
    Fixed {
        /// Size in pixels.
        px: u32,
    },
    /// At least `min_px`, at most `max_px`; grows into leftover space.
    MinMax {
        /// Smallest accepted size in pixels.
        min_px: u32,
        /// Largest size in pixels.
        max_px: u32,
    },
    /// Weight of the space left after the other tracks.
    Fraction {
        /// Weight numerator, at least 1.
        num: u32,
        /// Weight denominator, at least 1; shared by every fraction on the
        /// same axis.
        den: u32,
    },
    /// `w:h` aspect: the size on this axis follows the other axis.
    Aspect {
        /// Aspect width part.
        w: u32,
        /// Aspect height part.
        h: u32,
    },
}

impl Constraint {
    /// Size in pixels for a track that has no other axis to follow.
    fn base_px(self, total_px: u32) -> u32 {
        match self {
            Self::Fixed { px } => px,
            Self::MinMax { min_px, .. } => min_px,
            Self::Fraction { num, den } => {
                if den == 0 {
                    0
                } else {
                    total_px.saturating_mul(num) / den
                }
            }
            // Aspect tracks are refused on the row axis before this is read.
            Self::Aspect { .. } => 0,
        }
    }
}

/// Where leftover space goes when the tracks do not claim all of it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Whitespace {
    /// Leave the slack as trailing space at the right/bottom edge.
    Compact,
    /// Give slack to the fraction tracks only, split evenly.
    Balanced,
    /// Give slack to every track, split evenly.
    Spacious,
}

/// Viewport the document is solved against, in integer pixels.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Viewport {
    /// Width in pixels.
    pub width_px: u32,
    /// Height in pixels.
    pub height_px: u32,
}

impl Viewport {
    /// Viewport of `width_px` by `height_px`. Pure.
    #[must_use]
    pub const fn new(width_px: u32, height_px: u32) -> Self {
        Self {
            width_px,
            height_px,
        }
    }
}

/// Page margins in pixels; the inner box is the viewport minus these.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Margins {
    /// Left edge inset.
    pub left: u32,
    /// Top edge inset.
    pub top: u32,
    /// Right edge inset.
    pub right: u32,
    /// Bottom edge inset.
    pub bottom: u32,
}

impl Margins {
    /// No inset on any edge.
    pub const ZERO: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    /// Left plus right inset, saturating.
    #[must_use]
    pub const fn horizontal(self) -> u32 {
        self.left.saturating_add(self.right)
    }

    /// Top plus bottom inset, saturating.
    #[must_use]
    pub const fn vertical(self) -> u32 {
        self.top.saturating_add(self.bottom)
    }
}

/// One solved rectangle: top-left origin, integer pixels.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// Rect at `(x, y)` with size `(w, h)`. Pure.
    #[must_use]
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Right edge, saturating.
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.w)
    }

    /// Bottom edge, saturating.
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.h)
    }

    /// True when the two rects share at least one pixel. Touching edges do
    /// not count as overlap. Pure.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// One panel in one row: id plus the rule for its width.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PanelSpec {
    /// Identifier echoed on the solved rect.
    pub id: PanelId,
    /// Width rule on the row axis.
    pub width: Constraint,
}

/// One horizontal band: height rule, gap between panels, and its panels
/// left to right.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RowSpec {
    /// Height rule on the vertical axis.
    pub height: Constraint,
    /// Gap between neighbouring panels in this row, pixels.
    pub gap: u32,
    /// Panels left to right.
    pub panels: Vec<PanelSpec>,
}

/// Authored layout document: background, margins, row gap, whitespace taste
/// and its row bands.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LayoutDoc {
    /// Background sRGB triad the palette check measures against.
    pub background: [u8; 3],
    /// Page margins.
    pub margins: Margins,
    /// Gap between neighbouring rows, pixels.
    pub row_gap: u32,
    /// Where leftover space goes.
    pub whitespace: Whitespace,
    /// Row bands top to bottom.
    pub rows: Vec<RowSpec>,
    /// Caller drag overrides, at most one per panel; applied by [`Self::solve`]
    /// in place of the authored width. Empty after a release.
    pub pins: Vec<PanelPin>,
}

/// One panel's solved place.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PanelRect {
    /// Panel this rect belongs to.
    pub id: PanelId,
    /// Exact pixel rect.
    pub rect: Rect,
}

/// One pinned width override: a caller drag stored beside the authored
/// document, never inside it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PanelPin {
    /// Panel the override names.
    pub id: PanelId,
    /// Width rule used instead of the authored `PanelSpec::width`.
    pub width: Constraint,
}

/// Solved layout plus the edit receipt: which panels moved versus the
/// previous solve, in authored order. A solve with no previous result lists
/// every panel.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditedLayout {
    /// The solve itself.
    pub solved: SolvedLayout,
    /// Panel ids whose rect differs from the previous solve, authored order.
    /// Empty when the edit changed nothing.
    pub moved: Vec<PanelId>,
}

/// Solved layout: one rect per panel, the inner box, the slack it left and a
/// deterministic digest over all of it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SolvedLayout {
    /// Panel rects in authored order (rows top to bottom, panels left to
    /// right).
    pub panels: Vec<PanelRect>,
    /// Inner box inside the margins.
    pub inner: Rect,
    /// Unused width at the right edge, pixels.
    pub leftover_w: u32,
    /// Unused height at the bottom edge, pixels.
    pub leftover_h: u32,
    /// FNV-1a64 over the inner box, every rect and the leftover, integers
    /// only. Same document and viewport give the same digest.
    pub digest: u64,
}

impl SolvedLayout {
    /// Rect for one panel id, if the document holds it. Pure.
    #[must_use]
    pub fn rect(&self, id: PanelId) -> Option<Rect> {
        self.panels
            .iter()
            .find(|panel| panel.id == id)
            .map(|panel| panel.rect)
    }
}

/// One text role the palette gate measures on the layout background.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextRole {
    /// Body text, gated at [`HUD_TEXT_CONTRAST`] (WCAG AA normal text).
    Body,
    /// Accent text and markers, gated at [`MIN_CONTRAST`].
    Accent,
}

impl TextRole {
    /// Short role name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Accent => "accent",
        }
    }

    /// The contrast floor this role must reach, from the crate constants.
    #[must_use]
    pub fn required_permille(self) -> u32 {
        match self {
            Self::Body => permille(HUD_TEXT_CONTRAST),
            Self::Accent => permille(MIN_CONTRAST),
        }
    }
}

/// Contrast for one text role: measured ratio, the floor, and pass/fail.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RoleContrast {
    /// Role measured.
    pub role: TextRole,
    /// Measured contrast in permille (`2100` is 2.1:1).
    pub ratio_permille: u32,
    /// Floor the role must reach, in permille.
    pub required_permille: u32,
    /// True when `ratio_permille >= required_permille`.
    pub passes: bool,
}

impl RoleContrast {
    /// Measure `fg` over `bg` for `role`. Pure; one float division inside
    /// [`contrast_ratio`] and integer permille after that.
    #[must_use]
    pub fn measure(role: TextRole, fg: [u8; 3], bg: [u8; 3], required_permille: u32) -> Self {
        let ratio_permille = permille(contrast_ratio(fg, bg));
        Self {
            role,
            ratio_permille,
            required_permille,
            passes: ratio_permille >= required_permille,
        }
    }
}

/// Palette gate for one layout: the palette name, one row per text role, and
/// whether every role passes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PaletteReport {
    /// Palette name, from [`hud_palettes`].
    pub palette: &'static str,
    /// One row per role, [`TextRole::Body`] first.
    pub roles: [RoleContrast; 2],
    /// True when every role passes.
    pub passes: bool,
}

/// Layout failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LayoutError {
    /// The document holds no rows.
    NoRows,
    /// More than [`MAX_ROWS`] rows, count carried.
    TooManyRows(usize),
    /// More than [`MAX_PANELS`] panels, count carried.
    TooManyPanels(usize),
    /// Two panels share an id, culprit carried.
    DuplicatePanel(PanelId),
    /// A margin or gap is wider than [`MAX_GAP_PX`], culprit carried.
    GapTooLarge(u32),
    /// A fraction has a zero `num` or `den`; first row index carried.
    BadFraction {
        /// Row index (0-based) of the bad fraction.
        row: u16,
    },
    /// A min/max constraint has `min_px > max_px`; first row index carried.
    BadMinMax {
        /// Row index (0-based) of the bad pair.
        row: u16,
    },
    /// An aspect has a zero `w` or `h`; first row index carried.
    BadAspect {
        /// Row index (0-based) of the bad aspect.
        row: u16,
    },
    /// A row band uses an aspect constraint; there is no other axis to
    /// follow. Row index carried.
    AspectOnRowTrack(u16),
    /// Fractions on one axis disagree on `den`; row index and the offending
    /// `den` carried.
    MixedFractionDen {
        /// Row index (0-based) of the offending fraction.
        row: u16,
        /// Denominator that differs from the first fraction in that row.
        den: u32,
    },
    /// The content does not fit the viewport on one axis.
    Overflow {
        /// Axis that overflowed.
        axis: Axis,
        /// Pixels the content exceeds the inner box by.
        shortfall_px: u32,
        /// First panel of the caller's shrink order, the one recovery.
        shrink_first: Option<PanelId>,
    },
    /// Palette id outside the four [`hud_palettes`] sets, culprit carried.
    BadPalette(u8),
    /// A pin names a panel the document does not hold, culprit carried.
    UnknownPanel(PanelId),
    /// A drag asked for a vertical resize; panels carry only a width rule.
    UnsupportedResizeAxis(Axis),
}

impl core::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoRows => write!(f, "layout has no rows"),
            Self::TooManyRows(n) => write!(f, "too many rows ({n} over {MAX_ROWS})"),
            Self::TooManyPanels(n) => write!(f, "too many panels ({n} over {MAX_PANELS})"),
            Self::DuplicatePanel(id) => write!(f, "duplicate panel {}", id.0),
            Self::GapTooLarge(px) => {
                write!(f, "gap or margin {px} px over the {MAX_GAP_PX} px bound")
            }
            Self::BadFraction { row } => write!(
                f,
                "row {row}: a fraction needs num >= 1 and den >= 1"
            ),
            Self::BadMinMax { row } => {
                write!(f, "row {row}: min_px must not exceed max_px")
            }
            Self::BadAspect { row } => write!(f, "row {row}: a zero aspect is not a size"),
            Self::AspectOnRowTrack(row) => {
                write!(f, "row {row}: aspect is a panel rule, not a row height")
            }
            Self::MixedFractionDen { row, den } => write!(
                f,
                "row {row}: fraction den {den} differs from the row's other fractions"
            ),
            Self::Overflow {
                axis,
                shortfall_px,
                shrink_first,
            } => match shrink_first {
                Some(id) => write!(
                    f,
                    "{axis} overflow by {shortfall_px} px; recovery: shrink panel {} first",
                    id.0
                ),
                None => write!(
                    f,
                    "{axis} overflow by {shortfall_px} px; recovery: name a shrink order or widen the viewport"
                ),
            },
            Self::BadPalette(id) => {
                write!(f, "unknown palette {id}; legal ids are 0..=3")
            }
            Self::UnknownPanel(id) => write!(
                f,
                "pin names panel {} which this document does not hold; recovery: pin an authored panel id",
                id.0
            ),
            Self::UnsupportedResizeAxis(axis) => write!(
                f,
                "a {axis} drag is not a panel rule; recovery: resize the row height through its RowSpec"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl LayoutDoc {
    /// Document with black background, zero margins, zero row gap and
    /// [`Whitespace::Compact`]. Pure.
    #[must_use]
    pub fn new(rows: Vec<RowSpec>) -> Self {
        Self {
            background: [0, 0, 0],
            margins: Margins::ZERO,
            row_gap: 0,
            whitespace: Whitespace::Compact,
            rows,
            pins: Vec::new(),
        }
    }

    /// Pin panel `id` to `width`, replacing its authored width rule until
    /// [`Self::unpin`]. One override per panel; pinning again replaces it.
    /// The authored spec is untouched, so a release restores the old rects
    /// exactly. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::UnknownPanel`] when no panel carries `id`.
    pub fn pin(&mut self, id: PanelId, width: Constraint) -> Result<(), LayoutError> {
        if !self.has_panel(id) {
            return Err(LayoutError::UnknownPanel(id));
        }
        if let Some(slot) = self.pins.iter_mut().find(|pin| pin.id == id) {
            slot.width = width;
        } else {
            self.pins.push(PanelPin { id, width });
        }
        Ok(())
    }

    /// Release the pin on `id`; true when one was held. The next solve uses
    /// the authored width again. Pure.
    pub fn unpin(&mut self, id: PanelId) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.id != id);
        self.pins.len() != before
    }

    /// Active pins in pin order.
    #[must_use]
    pub fn pins(&self) -> &[PanelPin] {
        &self.pins
    }

    /// Store a caller drag on panel `id` as a `min_px == max_px` min/max
    /// override, so the panel keeps exactly the width the drag left it at
    /// (`px` is the drag's target width in pixels). Pure.
    ///
    /// # Errors
    ///
    /// [`LayoutError::UnsupportedResizeAxis`] for a vertical drag (a panel
    /// owns a width rule, not a height), [`LayoutError::UnknownPanel`] for an
    /// id the document does not hold.
    pub fn resize(&mut self, id: PanelId, axis: Axis, px: u32) -> Result<(), LayoutError> {
        if axis == Axis::Vertical {
            return Err(LayoutError::UnsupportedResizeAxis(axis));
        }
        self.pin(
            id,
            Constraint::MinMax {
                min_px: px,
                max_px: px,
            },
        )
    }

    /// Solve, then report which panels moved versus `previous`, in authored
    /// order. `None` means there is no previous solve, so every panel is new.
    /// The receipt never carries a panel that sat still, and two identical
    /// drag sequences give the same `moved` and digest. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] exactly as [`Self::solve`] does.
    pub fn fetch_changes(
        &self,
        viewport: Viewport,
        shrink_order: &[PanelId],
        previous: Option<&SolvedLayout>,
    ) -> Result<EditedLayout, LayoutError> {
        let solved = self.solve(viewport, shrink_order)?;
        let moved = solved
            .panels
            .iter()
            .filter(|panel| previous.and_then(|prev| prev.rect(panel.id)) != Some(panel.rect))
            .map(|panel| panel.id)
            .collect();
        Ok(EditedLayout { solved, moved })
    }

    /// True when an authored panel carries `id`.
    fn has_panel(&self, id: PanelId) -> bool {
        self.row_of(id).is_some()
    }

    /// Row index of the panel that carries `id`, if any.
    fn row_of(&self, id: PanelId) -> Option<u16> {
        self.rows
            .iter()
            .position(|row| row.panels.iter().any(|panel| panel.id == id))
            .map(row_index)
    }

    /// Solve the document against `viewport`, naming the caller's shrink
    /// order for the overflow recovery. Panels never overlap and every rect
    /// sits inside the margins. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] for bad constraints, duplicate ids, oversized
    /// gaps, or content that does not fit the viewport.
    pub fn solve(
        &self,
        viewport: Viewport,
        shrink_order: &[PanelId],
    ) -> Result<SolvedLayout, LayoutError> {
        self.validate()?;
        let inner_w = viewport.width_px.saturating_sub(self.margins.horizontal());
        let inner_h = viewport.height_px.saturating_sub(self.margins.vertical());
        let inner = Rect::new(self.margins.left, self.margins.top, inner_w, inner_h);
        let heights = self.solve_rows(inner_h, shrink_order)?;
        let mut panels: Vec<PanelRect> = Vec::new();
        let mut used_w = 0u32;
        let mut y = self.margins.top;
        for (row, height) in self.rows.iter().zip(&heights) {
            let (rects, row_used) = solve_row(
                row,
                y,
                *height,
                inner_w,
                self.margins.left,
                self.whitespace,
                shrink_order,
                &self.pins,
            )?;
            used_w = used_w.max(row_used);
            panels.extend(rects);
            y = y.saturating_add(*height).saturating_add(self.row_gap);
        }
        let used_h = heights
            .iter()
            .fold(gap_total(self.row_gap, heights.len()), |acc, h| {
                acc.saturating_add(*h)
            });
        let leftover_w = inner_w.saturating_sub(used_w);
        let leftover_h = inner_h.saturating_sub(used_h).min(inner_h);
        let digest = layout_digest(inner, &panels, leftover_w, leftover_h);
        Ok(SolvedLayout {
            panels,
            inner,
            leftover_w,
            leftover_h,
            digest,
        })
    }

    /// Contrast gate for palette `palette_id` from [`hud_palettes`]: body
    /// text and accent, each measured against this document's background.
    /// Pure.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::BadPalette`] for ids above 3.
    pub fn palette_report(&self, palette_id: u8) -> Result<PaletteReport, LayoutError> {
        let palette = hud_palettes()
            .get(usize::from(palette_id))
            .copied()
            .ok_or(LayoutError::BadPalette(palette_id))?;
        let body = RoleContrast::measure(
            TextRole::Body,
            palette.fg,
            self.background,
            TextRole::Body.required_permille(),
        );
        let accent = RoleContrast::measure(
            TextRole::Accent,
            palette.accent,
            self.background,
            TextRole::Accent.required_permille(),
        );
        Ok(PaletteReport {
            palette: palette.name,
            roles: [body, accent],
            passes: body.passes && accent.passes,
        })
    }

    /// Reject the documents the solver cannot answer honestly.
    fn validate(&self) -> Result<(), LayoutError> {
        if self.rows.is_empty() {
            return Err(LayoutError::NoRows);
        }
        if self.rows.len() > MAX_ROWS {
            return Err(LayoutError::TooManyRows(self.rows.len()));
        }
        self.check_gap(self.row_gap)?;
        for gap in [
            self.margins.left,
            self.margins.top,
            self.margins.right,
            self.margins.bottom,
        ] {
            self.check_gap(gap)?;
        }
        let mut seen: Vec<PanelId> = Vec::new();
        let mut count = 0usize;
        for (index, row) in self.rows.iter().enumerate() {
            let row_index = row_index(index);
            self.check_gap(row.gap)?;
            match row.height {
                Constraint::Aspect { .. } => {
                    return Err(LayoutError::AspectOnRowTrack(row_index));
                }
                Constraint::Fraction { num, den } => {
                    if num == 0 || den == 0 {
                        return Err(LayoutError::BadFraction { row: row_index });
                    }
                }
                Constraint::MinMax { min_px, max_px } => {
                    if min_px > max_px {
                        return Err(LayoutError::BadMinMax { row: row_index });
                    }
                }
                Constraint::Fixed { .. } => {}
            }
            // Fractions on one axis share a denominator so the split stays
            // integer; the row height and its panels are two separate axes.
            let mut row_den: Option<u32> = None;
            for panel in &row.panels {
                count = count.saturating_add(1);
                if count > MAX_PANELS {
                    return Err(LayoutError::TooManyPanels(count));
                }
                if seen.contains(&panel.id) {
                    return Err(LayoutError::DuplicatePanel(panel.id));
                }
                seen.push(panel.id);
                self.check_width(panel.width, row_index)?;
                if let Constraint::Fraction { den, .. } = panel.width {
                    if let Some(first) = row_den {
                        if first != den {
                            return Err(LayoutError::MixedFractionDen {
                                row: row_index,
                                den,
                            });
                        }
                    } else {
                        row_den = Some(den);
                    }
                }
            }
        }
        for pin in &self.pins {
            let row = self
                .row_of(pin.id)
                .ok_or(LayoutError::UnknownPanel(pin.id))?;
            self.check_width(pin.width, row)?;
        }
        Ok(())
    }

    /// Check one panel width rule; the row index only names the culprit.
    fn check_width(&self, width: Constraint, row: u16) -> Result<(), LayoutError> {
        match width {
            Constraint::Fraction { num, den } => {
                if num == 0 || den == 0 {
                    return Err(LayoutError::BadFraction { row });
                }
            }
            Constraint::Aspect { w, h } => {
                if w == 0 || h == 0 {
                    return Err(LayoutError::BadAspect { row });
                }
            }
            Constraint::MinMax { min_px, max_px } => {
                if min_px > max_px {
                    return Err(LayoutError::BadMinMax { row });
                }
            }
            Constraint::Fixed { .. } => {}
        }
        Ok(())
    }

    /// One gap check, shared by margins, the row gap and row gaps.
    fn check_gap(&self, px: u32) -> Result<(), LayoutError> {
        if px > MAX_GAP_PX {
            return Err(LayoutError::GapTooLarge(px));
        }
        Ok(())
    }

    /// First id of the caller shrink order that names a panel in this
    /// document, or `None` when the caller named none of them.
    fn first_shrink(&self, shrink_order: &[PanelId]) -> Option<PanelId> {
        shrink_order.iter().copied().find(|id| {
            self.rows
                .iter()
                .any(|row| row.panels.iter().any(|p| p.id == *id))
        })
    }

    /// Row heights: fractions share the space left after the other rows.
    fn solve_rows(&self, inner_h: u32, shrink_order: &[PanelId]) -> Result<Vec<u32>, LayoutError> {
        let count = self.rows.len();
        let gaps = gap_total(self.row_gap, count);
        let mut remaining = inner_h.saturating_sub(gaps);
        let mut fraction_sum = 0u32;
        for row in &self.rows {
            match row.height {
                Constraint::Fraction { num, .. } => {
                    fraction_sum = fraction_sum.saturating_add(num);
                }
                other => {
                    remaining = remaining.saturating_sub(other.base_px(inner_h));
                }
            }
        }
        let mut heights: Vec<u32> = Vec::with_capacity(count);
        for row in &self.rows {
            let height = match row.height {
                Constraint::Fraction { num, .. } => {
                    if fraction_sum == 0 {
                        0
                    } else {
                        remaining.saturating_mul(num) / fraction_sum
                    }
                }
                other => other.base_px(inner_h),
            };
            heights.push(height);
        }
        let used = heights.iter().fold(gaps, |acc, h| acc.saturating_add(*h));
        if used > inner_h {
            return Err(LayoutError::Overflow {
                axis: Axis::Vertical,
                shortfall_px: used - inner_h,
                shrink_first: self.first_shrink(shrink_order),
            });
        }
        let slack = inner_h - used;
        self.grow_and_spread(&mut heights, slack);
        Ok(heights)
    }

    /// Grow min/max tracks into `slack`, then place what is left by the
    /// whitespace taste.
    fn grow_and_spread(&self, sizes: &mut [u32], slack: u32) {
        let mut slack = slack;
        for (size, row) in sizes.iter_mut().zip(&self.rows) {
            if let Constraint::MinMax { max_px, .. } = row.height {
                let grow = max_px.saturating_sub(*size).min(slack);
                *size = size.saturating_add(grow);
                slack -= grow;
            }
        }
        let eligible: Vec<bool> = self
            .rows
            .iter()
            .map(|row| match self.whitespace {
                Whitespace::Compact => false,
                Whitespace::Balanced => matches!(row.height, Constraint::Fraction { .. }),
                Whitespace::Spacious => true,
            })
            .collect();
        distribute_even(sizes, slack, &eligible);
    }
}

/// Row index as `u16`; the row cap keeps the conversion exact.
fn row_index(index: usize) -> u16 {
    u16::try_from(index).unwrap_or(u16::MAX)
}

/// Total gap for `count` tracks: one gap between each neighbouring pair.
fn gap_total(gap: u32, count: usize) -> u32 {
    let pairs = count.saturating_sub(1);
    gap.saturating_mul(u32::try_from(pairs).unwrap_or(u32::MAX))
}

/// Split `slack` evenly over the eligible tracks, one extra pixel to the
/// first remainder tracks. No eligible track leaves the slack unused.
fn distribute_even(sizes: &mut [u32], slack: u32, eligible: &[bool]) {
    if slack == 0 {
        return;
    }
    let count = eligible.iter().filter(|ok| **ok).count();
    if count == 0 {
        return;
    }
    let count_u32 = u32::try_from(count).unwrap_or(u32::MAX);
    let each = slack / count_u32;
    let mut extra = slack - each.saturating_mul(count_u32);
    for (size, ok) in sizes.iter_mut().zip(eligible) {
        if !*ok {
            continue;
        }
        let share = if extra > 0 {
            extra -= 1;
            each.saturating_add(1)
        } else {
            each
        };
        *size = size.saturating_add(share);
    }
}

/// Effective width rule for one panel: the pin when held, else the authored
/// rule. Pure.
fn effective_width(panel: &PanelSpec, pins: &[PanelPin]) -> Constraint {
    pins.iter()
        .find(|pin| pin.id == panel.id)
        .map_or(panel.width, |pin| pin.width)
}

/// Solve one row band: panel widths, positions and the width it consumed.
fn solve_row(
    row: &RowSpec,
    top: u32,
    height: u32,
    inner_w: u32,
    left: u32,
    whitespace: Whitespace,
    shrink_order: &[PanelId],
    pins: &[PanelPin],
) -> Result<(Vec<PanelRect>, u32), LayoutError> {
    let count = row.panels.len();
    let gaps = gap_total(row.gap, count);
    let widths_rules: Vec<Constraint> = row
        .panels
        .iter()
        .map(|panel| effective_width(panel, pins))
        .collect();
    let mut remaining = inner_w.saturating_sub(gaps);
    let mut fraction_sum = 0u32;
    for rule in &widths_rules {
        match *rule {
            Constraint::Fraction { num, .. } => {
                fraction_sum = fraction_sum.saturating_add(num);
            }
            Constraint::Aspect { w, h } => {
                remaining = remaining.saturating_sub(height.saturating_mul(w) / h);
            }
            other => {
                remaining = remaining.saturating_sub(other.base_px(inner_w));
            }
        }
    }
    let mut widths: Vec<u32> = Vec::with_capacity(count);
    for rule in &widths_rules {
        let width = match *rule {
            Constraint::Fraction { num, .. } => {
                if fraction_sum == 0 {
                    0
                } else {
                    remaining.saturating_mul(num) / fraction_sum
                }
            }
            Constraint::Aspect { w, h } => height.saturating_mul(w) / h,
            other => other.base_px(inner_w),
        };
        widths.push(width);
    }
    let used = widths.iter().fold(gaps, |acc, w| acc.saturating_add(*w));
    if used > inner_w {
        let shrink_first = shrink_order
            .iter()
            .copied()
            .find(|id| row.panels.iter().any(|panel| panel.id == *id))
            .or_else(|| row.panels.first().map(|panel| panel.id));
        return Err(LayoutError::Overflow {
            axis: Axis::Horizontal,
            shortfall_px: used - inner_w,
            shrink_first,
        });
    }
    let slack = grow_row_panels(&mut widths, &widths_rules, inner_w - used);
    let eligible: Vec<bool> = widths_rules
        .iter()
        .map(|rule| match whitespace {
            Whitespace::Compact => false,
            Whitespace::Balanced => matches!(rule, Constraint::Fraction { .. }),
            Whitespace::Spacious => true,
        })
        .collect();
    distribute_even(&mut widths, slack, &eligible);
    let mut rects = Vec::with_capacity(count);
    let mut x = left;
    for (panel, width) in row.panels.iter().zip(&widths) {
        rects.push(PanelRect {
            id: panel.id,
            rect: Rect::new(x, top, *width, height),
        });
        x = x.saturating_add(*width).saturating_add(row.gap);
    }
    let used_w = widths.iter().fold(gaps, |acc, w| acc.saturating_add(*w));
    Ok((rects, used_w))
}

/// Grow min/max panels in one row up to their max; returns the slack left.
fn grow_row_panels(widths: &mut [u32], rules: &[Constraint], slack: u32) -> u32 {
    let mut slack = slack;
    for (width, rule) in widths.iter_mut().zip(rules) {
        if let Constraint::MinMax { max_px, .. } = *rule {
            let grow = max_px.saturating_sub(*width).min(slack);
            *width = width.saturating_add(grow);
            slack -= grow;
        }
    }
    slack
}

/// Contrast ratio as integer permille, rounded down.
fn permille(ratio: f32) -> u32 {
    let scaled = saturating_f64_to_u64(f64::from(ratio) * 1000.0);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// FNV-1a64 over the solved layout, integers only, field widths fixed.
fn layout_digest(inner: Rect, panels: &[PanelRect], leftover_w: u32, leftover_h: u32) -> u64 {
    let mut hash = Fnv64::new();
    for value in [inner.x, inner.y, inner.w, inner.h] {
        hash.write_u32(value);
    }
    for panel in panels {
        hash.write_u16(panel.id.0);
        for value in [panel.rect.x, panel.rect.y, panel.rect.w, panel.rect.h] {
            hash.write_u32(value);
        }
    }
    hash.write_u32(leftover_w);
    hash.write_u32(leftover_h);
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Palette;

    fn panel(id: u16, width: Constraint) -> PanelSpec {
        PanelSpec {
            id: PanelId(id),
            width,
        }
    }

    fn fraction(num: u32, den: u32) -> Constraint {
        Constraint::Fraction { num, den }
    }

    /// One row, one panel, both fill the box.
    fn one_by_one() -> LayoutDoc {
        LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 0,
            panels: vec![panel(0, fraction(1, 1))],
        }])
    }

    /// Eight-pixel page margin, eight-pixel gaps, four equal cells.
    fn two_by_two() -> LayoutDoc {
        let mut doc = LayoutDoc::new(vec![
            RowSpec {
                height: fraction(1, 2),
                gap: 8,
                panels: vec![panel(0, fraction(1, 2)), panel(1, fraction(1, 2))],
            },
            RowSpec {
                height: fraction(1, 2),
                gap: 8,
                panels: vec![panel(2, fraction(1, 2)), panel(3, fraction(1, 2))],
            },
        ]);
        doc.margins = Margins {
            left: 8,
            top: 8,
            right: 8,
            bottom: 8,
        };
        doc.row_gap = 8;
        doc
    }

    /// Fixed 240 px sidebar, main takes the rest with a 16 px gap.
    fn sidebar_and_main() -> LayoutDoc {
        LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 16,
            panels: vec![
                panel(0, Constraint::Fixed { px: 240 }),
                panel(1, fraction(1, 1)),
            ],
        }])
    }

    fn assert_inside(solved: &SolvedLayout) {
        let inner = solved.inner;
        for panel in &solved.panels {
            let rect = panel.rect;
            assert!(
                rect.x >= inner.x
                    && rect.y >= inner.y
                    && rect.right() <= inner.right()
                    && rect.bottom() <= inner.bottom(),
                "{:?} outside inner {:?}",
                rect,
                inner
            );
        }
    }

    fn assert_no_overlap(solved: &SolvedLayout) {
        for (index, first) in solved.panels.iter().enumerate() {
            for second in solved.panels.iter().skip(index + 1) {
                assert!(
                    !first.rect.overlaps(second.rect),
                    "{:?} overlaps {:?}",
                    first,
                    second
                );
            }
        }
    }

    fn rect_of(solved: &SolvedLayout, id: u16) -> Rect {
        solved
            .rect(PanelId(id))
            .expect("fixture panel id is solved")
    }

    #[test]
    fn one_by_one_fills_both_viewports() {
        let doc = one_by_one();
        for (width, height) in [(1280, 720), (360, 640)] {
            let solved = doc
                .solve(Viewport::new(width, height), &[])
                .expect("fraction panels always fit");
            assert_eq!(rect_of(&solved, 0), Rect::new(0, 0, width, height));
            assert_eq!(
                (solved.leftover_w, solved.leftover_h),
                (0, 0),
                "a full fraction leaves no slack"
            );
            assert_inside(&solved);
            assert_no_overlap(&solved);
        }
    }

    #[test]
    fn two_by_two_cells_are_exact_at_both_viewports() {
        let doc = two_by_two();
        let wide = doc
            .solve(Viewport::new(1280, 720), &[])
            .expect("four fractions fit");
        let expected_wide = [
            Rect::new(8, 8, 628, 348),
            Rect::new(644, 8, 628, 348),
            Rect::new(8, 364, 628, 348),
            Rect::new(644, 364, 628, 348),
        ];
        for (id, expected) in expected_wide.iter().enumerate() {
            assert_eq!(
                rect_of(&wide, u16::try_from(id).expect("four ids fit")),
                *expected,
                "1280x720 cell {id}"
            );
        }
        assert_eq!((wide.leftover_w, wide.leftover_h), (0, 0));
        assert_inside(&wide);
        assert_no_overlap(&wide);

        let tall = doc
            .solve(Viewport::new(360, 640), &[])
            .expect("four fractions fit");
        let expected_tall = [
            Rect::new(8, 8, 168, 308),
            Rect::new(184, 8, 168, 308),
            Rect::new(8, 324, 168, 308),
            Rect::new(184, 324, 168, 308),
        ];
        for (id, expected) in expected_tall.iter().enumerate() {
            assert_eq!(
                rect_of(&tall, u16::try_from(id).expect("four ids fit")),
                *expected,
                "360x640 cell {id}"
            );
        }
        assert_eq!((tall.leftover_w, tall.leftover_h), (0, 0));
        assert_inside(&tall);
        assert_no_overlap(&tall);
    }

    #[test]
    fn sidebar_plus_main_keeps_the_sidebar_fixed() {
        let doc = sidebar_and_main();
        let wide = doc
            .solve(Viewport::new(1280, 720), &[])
            .expect("a fixed sidebar and a fraction main fit");
        assert_eq!(rect_of(&wide, 0), Rect::new(0, 0, 240, 720));
        assert_eq!(rect_of(&wide, 1), Rect::new(256, 0, 1024, 720));
        assert_inside(&wide);
        assert_no_overlap(&wide);

        let tall = doc
            .solve(Viewport::new(360, 640), &[])
            .expect("a fixed sidebar and a fraction main fit");
        assert_eq!(rect_of(&tall, 0), Rect::new(0, 0, 240, 640));
        assert_eq!(rect_of(&tall, 1), Rect::new(256, 0, 104, 640));
        assert_inside(&tall);
        assert_no_overlap(&tall);
    }

    #[test]
    fn the_same_document_solves_to_the_same_digest() {
        let doc = two_by_two();
        let viewport = Viewport::new(1280, 720);
        let first = doc.solve(viewport, &[]).expect("fixture fits");
        let second = doc.solve(viewport, &[]).expect("fixture fits");
        assert_eq!(first.panels, second.panels);
        assert_eq!(first.digest, second.digest);
        let other = one_by_one().solve(viewport, &[]).expect("fixture fits");
        assert_ne!(
            first.digest, other.digest,
            "different layouts, other digest"
        );
    }

    #[test]
    fn impossible_mins_name_the_shortfall_and_one_recovery() {
        let doc = LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 0,
            panels: vec![
                panel(
                    0,
                    Constraint::MinMax {
                        min_px: 700,
                        max_px: 900,
                    },
                ),
                panel(
                    1,
                    Constraint::MinMax {
                        min_px: 700,
                        max_px: 900,
                    },
                ),
            ],
        }]);
        let error = doc
            .solve(Viewport::new(1280, 720), &[PanelId(1), PanelId(0)])
            .expect_err("1400 px of mins do not fit 1280");
        assert_eq!(
            error,
            LayoutError::Overflow {
                axis: Axis::Horizontal,
                shortfall_px: 120,
                shrink_first: Some(PanelId(1)),
            }
        );
        let text = error.to_string();
        assert!(text.contains("120"), "shortfall named: {text}");
        assert!(
            text.contains("shrink panel 1 first"),
            "recovery named: {text}"
        );
    }

    #[test]
    fn vertical_mins_overflow_on_the_vertical_axis() {
        let doc = LayoutDoc::new(vec![
            RowSpec {
                height: Constraint::Fixed { px: 300 },
                gap: 0,
                panels: vec![panel(0, fraction(1, 1))],
            },
            RowSpec {
                height: Constraint::Fixed { px: 300 },
                gap: 0,
                panels: vec![panel(1, fraction(1, 1))],
            },
            RowSpec {
                height: Constraint::Fixed { px: 300 },
                gap: 0,
                panels: vec![panel(2, fraction(1, 1))],
            },
        ]);
        assert_eq!(
            doc.solve(Viewport::new(1280, 720), &[PanelId(2)]),
            Err(LayoutError::Overflow {
                axis: Axis::Vertical,
                shortfall_px: 180,
                shrink_first: Some(PanelId(2)),
            })
        );
    }

    #[test]
    fn an_aspect_panel_takes_its_width_from_the_row_height() {
        let doc = LayoutDoc::new(vec![RowSpec {
            height: Constraint::Fixed { px: 180 },
            gap: 8,
            panels: vec![
                panel(0, Constraint::Aspect { w: 16, h: 9 }),
                panel(1, fraction(1, 1)),
            ],
        }]);
        let solved = doc
            .solve(Viewport::new(1280, 720), &[])
            .expect("aspect and main fit");
        assert_eq!(rect_of(&solved, 0), Rect::new(0, 0, 320, 180));
        assert_eq!(rect_of(&solved, 1), Rect::new(328, 0, 952, 180));
        assert_inside(&solved);
        assert_no_overlap(&solved);
    }

    #[test]
    fn whitespace_taste_moves_the_slack() {
        let rows = vec![RowSpec {
            height: Constraint::Fixed { px: 100 },
            gap: 0,
            panels: vec![
                panel(0, Constraint::Fixed { px: 100 }),
                panel(1, Constraint::Fixed { px: 100 }),
            ],
        }];
        let mut compact = LayoutDoc::new(rows.clone());
        compact.whitespace = Whitespace::Compact;
        let solved = compact
            .solve(Viewport::new(1280, 720), &[])
            .expect("fixed panels fit");
        assert_eq!(rect_of(&solved, 0).w, 100);
        assert_eq!(rect_of(&solved, 1).w, 100);
        assert_eq!(solved.leftover_w, 1080);
        assert_eq!(solved.leftover_h, 620);

        let mut spacious = LayoutDoc::new(rows);
        spacious.whitespace = Whitespace::Spacious;
        let solved = spacious
            .solve(Viewport::new(1280, 720), &[])
            .expect("fixed panels fit");
        assert_eq!(rect_of(&solved, 0).w, 640);
        assert_eq!(rect_of(&solved, 1).w, 640);
        assert_eq!(solved.leftover_w, 0);
        assert_eq!(solved.leftover_h, 0);
    }

    #[test]
    fn palette_report_gates_body_and_accent_on_the_layout_background() {
        let mut doc = one_by_one();
        let palette = Palette::default_palette();
        doc.background = palette.bg;
        let report = doc.palette_report(0).expect("palette 0 is gated");
        assert_eq!(report.palette, "default");
        assert!(
            report.passes,
            "default palette passes on its own background"
        );
        assert_eq!(report.roles[0].role, TextRole::Body);
        assert_eq!(report.roles[0].required_permille, 4_500);
        assert_eq!(report.roles[1].role, TextRole::Accent);
        assert_eq!(report.roles[1].required_permille, 3_000);
        assert_eq!(
            report.roles[0].ratio_permille,
            permille(contrast_ratio(palette.fg, palette.bg))
        );
        assert_eq!(
            report.roles[1].ratio_permille,
            permille(contrast_ratio(palette.accent, palette.bg))
        );

        let mut low = one_by_one();
        low.background = [120, 120, 120];
        let report = low.palette_report(0).expect("palette 0 is gated");
        assert!(!report.passes, "mid-grey background fails both roles");
        assert!(!report.roles[0].passes);
        assert!(!report.roles[1].passes);
        assert_eq!(doc.palette_report(4), Err(LayoutError::BadPalette(4)));
    }

    /// Fixed 300 px, then two flex panels sharing the rest (490 each at 1280).
    fn three_panel_row() -> LayoutDoc {
        LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 0,
            panels: vec![
                panel(0, Constraint::Fixed { px: 300 }),
                panel(1, fraction(1, 1)),
                panel(2, fraction(1, 1)),
            ],
        }])
    }

    #[test]
    fn a_drag_moves_the_pinned_panel_and_its_flex_neighbour_only() {
        let mut doc = three_panel_row();
        let viewport = Viewport::new(1280, 720);
        let before = doc.solve(viewport, &[]).expect("three panels fit");
        assert_eq!(rect_of(&before, 0), Rect::new(0, 0, 300, 720));
        assert_eq!(rect_of(&before, 1), Rect::new(300, 0, 490, 720));
        assert_eq!(rect_of(&before, 2), Rect::new(790, 0, 490, 720));

        // Drag the middle panel 40 px wider: 490 -> 530.
        doc.resize(PanelId(1), Axis::Horizontal, 530)
            .expect("a horizontal drag on a pinned panel");
        let drag = doc
            .fetch_changes(viewport, &[], Some(&before))
            .expect("three panels fit");
        assert_eq!(drag.moved, vec![PanelId(1), PanelId(2)]);
        assert_eq!(rect_of(&drag.solved, 0), Rect::new(0, 0, 300, 720));
        assert_eq!(rect_of(&drag.solved, 1), Rect::new(300, 0, 530, 720));
        assert_eq!(rect_of(&drag.solved, 2), Rect::new(830, 0, 450, 720));

        // The same drag again changes nothing: no moved ids, same digest.
        doc.resize(PanelId(1), Axis::Horizontal, 530)
            .expect("a horizontal drag on a pinned panel");
        let again = doc
            .fetch_changes(viewport, &[], Some(&drag.solved))
            .expect("three panels fit");
        assert!(again.moved.is_empty(), "a repeated drag is a no-op");
        assert_eq!(again.solved.digest, drag.solved.digest);

        // Release restores the authored rects byte for byte.
        assert!(doc.unpin(PanelId(1)));
        assert!(doc.pins().is_empty());
        let released = doc.solve(viewport, &[]).expect("three panels fit");
        assert_eq!(released, before);
        assert_eq!(released.digest, before.digest);
    }

    #[test]
    fn two_identical_drag_sequences_agree_on_moved_and_digest() {
        let viewport = Viewport::new(1280, 720);
        let mut first = three_panel_row();
        let mut second = three_panel_row();
        let mut first_moved = Vec::new();
        let mut second_moved = Vec::new();
        for (doc, moved) in [
            (&mut first, &mut first_moved),
            (&mut second, &mut second_moved),
        ] {
            let mut previous = doc.solve(viewport, &[]).expect("three panels fit");
            for px in [530u32, 500, 560] {
                doc.resize(PanelId(1), Axis::Horizontal, px)
                    .expect("a horizontal drag on a pinned panel");
                let edit = doc
                    .fetch_changes(viewport, &[], Some(&previous))
                    .expect("three panels fit");
                moved.push(edit.moved);
                previous = edit.solved;
            }
        }
        assert_eq!(first_moved, second_moved);
        assert_eq!(
            first.solve(viewport, &[]).expect("three panels fit").digest,
            second
                .solve(viewport, &[])
                .expect("three panels fit")
                .digest
        );
        assert_eq!(first.pins(), second.pins());
    }

    #[test]
    fn a_vertical_drag_and_an_unknown_pin_are_refused() {
        let mut doc = three_panel_row();
        assert_eq!(
            doc.resize(PanelId(1), Axis::Vertical, 200),
            Err(LayoutError::UnsupportedResizeAxis(Axis::Vertical))
        );
        assert_eq!(
            doc.pin(PanelId(9), Constraint::Fixed { px: 10 }),
            Err(LayoutError::UnknownPanel(PanelId(9)))
        );
        assert!(doc.pins().is_empty());
        // A bad pinned rule is caught by the solve, not stored as a panic.
        doc.pin(
            PanelId(1),
            Constraint::MinMax {
                min_px: 20,
                max_px: 10,
            },
        )
        .expect("the id exists");
        assert_eq!(
            doc.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::BadMinMax { row: 0 })
        );
    }

    #[test]
    fn bad_constraints_are_refused_with_the_row_index() {
        let mut gap_big = one_by_one();
        gap_big.row_gap = MAX_GAP_PX + 1;
        assert_eq!(
            gap_big.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::GapTooLarge(MAX_GAP_PX + 1))
        );

        let aspect_row = LayoutDoc::new(vec![RowSpec {
            height: Constraint::Aspect { w: 4, h: 3 },
            gap: 0,
            panels: vec![panel(0, fraction(1, 1))],
        }]);
        assert_eq!(
            aspect_row.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::AspectOnRowTrack(0))
        );

        let zero_den = LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 0),
            gap: 0,
            panels: vec![panel(0, fraction(1, 1))],
        }]);
        assert_eq!(
            zero_den.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::BadFraction { row: 0 })
        );

        let mixed = LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 0,
            panels: vec![panel(0, fraction(1, 2)), panel(1, fraction(1, 3))],
        }]);
        assert_eq!(
            mixed.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::MixedFractionDen { row: 0, den: 3 })
        );

        let duplicate = LayoutDoc::new(vec![RowSpec {
            height: fraction(1, 1),
            gap: 0,
            panels: vec![panel(0, fraction(1, 1)), panel(0, fraction(1, 1))],
        }]);
        assert_eq!(
            duplicate.solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::DuplicatePanel(PanelId(0)))
        );

        assert_eq!(
            LayoutDoc::new(Vec::new()).solve(Viewport::new(1280, 720), &[]),
            Err(LayoutError::NoRows)
        );
    }
}
