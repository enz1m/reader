//! Zoom and quarter-turn display geometry.
//!
//! Quarter turns swap page axes. Non-finite zoom uses ZOOM_FIT; finite values
//! are clamped to ZOOM_MIN and ZOOM_MAX.

/// The smallest zoom factor: below a quarter scale the page is a grey smear.
///
/// A dimensionless interaction bound, not a size, so this and the factors below are
/// literals rather than `SizingTokens`.
pub const ZOOM_MIN: f32 = 0.25;
/// The largest zoom factor. Five times a 1000px raster is 5000px of bitmap
/// per page, which is where memory, not usefulness, becomes the limit.
pub const ZOOM_MAX: f32 = 5.0;
/// The identity factor: the page exactly fills its fit-width box.
pub const ZOOM_FIT: f32 = 1.0;
/// One press of `Zoom in` / `Zoom out`. An additive step keeps `zoom_in().zoom_out()` an
/// identity except at the clamped ends.
pub const ZOOM_STEP: f32 = 0.25;

/// How the page surface derives its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZoomMode {
    /// The page fills the available content width (minus gutters), capped at
    /// the readable maximum. The default, and what `Fit width` returns to.
    #[default]
    FitWidth,
    /// The entire rotated page fits the reading viewport.
    FitPage,
    /// The user has picked an explicit factor with `Zoom in` / `Zoom out`.
    Custom,
}

/// How rendered document pages occupy the reading pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageViewMode {
    #[default]
    Continuous,
    SinglePage,
}

/// The page zoom: a clamped factor plus the mode that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    factor: f32,
    mode: ZoomMode,
}

impl Default for Zoom {
    fn default() -> Self {
        Zoom {
            factor: ZOOM_FIT,
            mode: ZoomMode::FitWidth,
        }
    }
}

impl Zoom {
    /// The current factor. Always finite and always within
    /// `[ZOOM_MIN, ZOOM_MAX]` — there is no constructor that can produce
    /// anything else.
    pub fn factor(self) -> f32 {
        self.factor
    }

    pub fn mode(self) -> ZoomMode {
        self.mode
    }

    // Clamp `factor` into the honoured range. Non-finite input falls back to
    // [`ZOOM_FIT`] (see the module docs) rather than propagating `NaN`.
    fn clamp(factor: f32) -> f32 {
        if !factor.is_finite() {
            return ZOOM_FIT;
        }
        factor.clamp(ZOOM_MIN, ZOOM_MAX)
    }

    /// Set an explicit factor (`Custom` mode). `0.0` is expressible intent —
    /// it means "as small as this reader goes" and clamps to [`ZOOM_MIN`];
    /// it is neither ignored nor an error.
    pub fn set(&mut self, factor: f32) {
        self.factor = Self::clamp(factor);
        self.mode = ZoomMode::Custom;
    }

    /// `Zoom in`: one [`ZOOM_STEP`] up, saturating at [`ZOOM_MAX`].
    pub fn zoom_in(&mut self) {
        self.set(self.factor + ZOOM_STEP);
    }

    /// `Zoom out`: one [`ZOOM_STEP`] down, saturating at [`ZOOM_MIN`].
    pub fn zoom_out(&mut self) {
        self.set(self.factor - ZOOM_STEP);
    }

    /// `Fit width`: back to the identity factor AND back to `FitWidth` mode.
    /// Resetting only the factor would leave the mode reading `Custom`, so a
    /// later window resize would keep the stale explicit width.
    pub fn fit_width(&mut self) {
        self.factor = ZOOM_FIT;
        self.mode = ZoomMode::FitWidth;
    }

    /// Fit the whole page within the reading viewport.
    pub fn fit_page(&mut self) {
        self.factor = ZOOM_FIT;
        self.mode = ZoomMode::FitPage;
    }

    /// Whether `Zoom in` can still do anything. Drives the control's
    /// `enabled` flag, so a user is never offered a button that is already at
    /// its stop.
    pub fn can_zoom_in(self) -> bool {
        self.factor < ZOOM_MAX
    }

    pub fn can_zoom_out(self) -> bool {
        self.factor > ZOOM_MIN
    }

    /// The rounded zoom percentage, or `Fit page` for viewport fitting.
    pub fn label(self) -> String {
        if self.mode == ZoomMode::FitPage {
            return "Fit page".into();
        }
        format!("{}%", (self.factor * 100.0).round() as i32)
    }
}

/// Page rotation in quarter turns clockwise.
///
/// Quarter turns rather than degrees, so an unreachable angle such as 45° is
/// unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PageRotation(u8);

impl PageRotation {
    /// Unrotated.
    pub const NONE: PageRotation = PageRotation(0);

    /// Build from a quarter-turn count, wrapping. `4` is `NONE`.
    pub fn from_quarters(q: u32) -> Self {
        PageRotation((q % 4) as u8)
    }

    /// Quarter turns clockwise, `0..=3`.
    pub fn quarters(self) -> u8 {
        self.0
    }

    /// `Rotate` / `Rotate right`: one quarter turn clockwise, wrapping at a
    /// full turn back to [`Self::NONE`].
    pub fn clockwise(self) -> Self {
        PageRotation((self.0 + 1) % 4)
    }

    /// `Rotate left`: one quarter turn anticlockwise, wrapping.
    pub fn counter_clockwise(self) -> Self {
        PageRotation((self.0 + 3) % 4)
    }

    /// Degrees clockwise: `0`, `90`, `180`, `270`.
    pub fn degrees(self) -> u16 {
        self.0 as u16 * 90
    }

    /// The angle in radians, for `iced::Rotation`.
    pub fn radians(self) -> f32 {
        self.degrees() as f32 * std::f32::consts::PI / 180.0
    }

    /// Whether this rotation exchanges width and height. True at 90° and 270°
    /// only — this is the property that makes the rotation *visible* in layout
    /// rather than a field nobody reads.
    pub fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }

    /// Compose quarter turns modulo a full turn; page and global view rotations commute.
    #[must_use]
    pub fn compose(self, other: Self) -> Self {
        PageRotation((self.0 + other.0) % 4)
    }

    /// The status line a rotation press produces (`Rotated 90°`). At a full
    /// turn back to zero it says so explicitly, because "Rotated 0°" reads as
    /// a control that failed.
    pub fn status(self) -> String {
        if self.0 == 0 {
            "Rotation reset".to_string()
        } else {
            format!("Rotated {}°", self.degrees())
        }
    }
}

/// The on-screen box for a page, in logical pixels.
///
/// * `native` is the rendered bitmap's `(width, height)`.
/// * `fit_width` is the width a `FitWidth` page would occupy (the caller has
///   already subtracted gutters and applied its readable-width cap).
/// * `zoom` scales that width.
/// * `rotation` swaps the axes at 90° / 270°.
///
/// Returns `(width, height)`. A degenerate bitmap (zero width) falls back to
/// the A4 aspect ratio rather than dividing by zero.
pub fn page_box(
    native: (u32, u32),
    fit_width: f32,
    zoom: Zoom,
    rotation: PageRotation,
) -> (f32, f32) {
    let aspect = if native.0 > 0 {
        native.1 as f32 / native.0 as f32
    } else {
        1.414 // A4 portrait
    };
    // Zoom the unrotated box first; zooming after the swap would scale a rotated page's
    // height.
    let w = (fit_width * zoom.factor()).max(1.0);
    let h = (w * aspect).max(1.0);
    if rotation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    }
}
