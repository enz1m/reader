//! Facing-page pairing with the cover alone on the right.
//!
//! A final unpaired page appears on the left.

/// Spread display mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SpreadMode {
    /// Single page view (default).
    #[default]
    Single,
    /// Two-page spread (facing pages).
    TwoPage,
}

impl SpreadMode {
    pub fn id(self) -> &'static str {
        match self {
            SpreadMode::Single => "single",
            SpreadMode::TwoPage => "two-page",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s {
            "two-page" => SpreadMode::TwoPage,
            _ => SpreadMode::Single,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SpreadMode::Single => "Single Page",
            SpreadMode::TwoPage => "Two-Page Spread",
        }
    }
}

/// One spread — either a single page or a left-right pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spread {
    /// Left page index (None for cover-only spread).
    pub left: Option<usize>,
    /// Right page index (None for the last odd spread).
    pub right: Option<usize>,
}

/// Compute the list of spreads for a document with `page_count` pages.
///
/// Cover handling: page 0 (cover) is displayed alone on the right side
/// of the first spread. Pages 1 and 2 face each other, 3 and 4, etc.
/// If there's an even page count, the last page is alone on the left.
///
/// # Examples
///
/// ```
/// use reader_app::spread::compute_spreads;
/// let spreads = compute_spreads(5);
/// assert_eq!(spreads.len(), 3);
/// // Cover alone
/// assert_eq!(spreads[0].left, None);
/// assert_eq!(spreads[0].right, Some(0));
/// // Pages 1-2 facing
/// assert_eq!(spreads[1].left, Some(1));
/// assert_eq!(spreads[1].right, Some(2));
/// // Pages 3-4 facing
/// assert_eq!(spreads[2].left, Some(3));
/// assert_eq!(spreads[2].right, Some(4));
/// ```
pub fn compute_spreads(page_count: usize) -> Vec<Spread> {
    if page_count == 0 {
        return Vec::new();
    }

    let mut spreads = Vec::new();

    // Cover page alone
    spreads.push(Spread {
        left: None,
        right: Some(0),
    });

    // Pair up remaining pages
    let mut i = 1;
    while i < page_count {
        if i + 1 < page_count {
            spreads.push(Spread {
                left: Some(i),
                right: Some(i + 1),
            });
            i += 2;
        } else {
            // Last page alone
            spreads.push(Spread {
                left: Some(i),
                right: None,
            });
            i += 1;
        }
    }

    spreads
}

/// Given a 0-indexed page number, find which spread it belongs to and
/// whether it's on the left or right side.
///
/// Returns `(spread_index, is_left)`.
pub fn spread_for_page(page: usize, page_count: usize) -> (usize, bool) {
    if page_count == 0 {
        return (0, false);
    }
    if page == 0 {
        return (0, false); // cover is on the right
    }
    // After cover, pages pair up starting at 1:
    // page 1-2 → spread 1, page 3-4 → spread 2, etc.
    let pair_index = (page - 1) / 2;
    let spread_index = pair_index + 1;
    let is_left = (page - 1).is_multiple_of(2); // odd pages (1, 3, 5, ...) are left
    (spread_index, is_left)
}

/// Given a spread index, return the pages to show (left, right).
/// Returns the same as `compute_spreads()[index]` but without allocating
/// the full list.
pub fn pages_for_spread(spread_index: usize, page_count: usize) -> Spread {
    if page_count == 0 || spread_index == 0 {
        return if page_count > 0 {
            Spread {
                left: None,
                right: Some(0),
            }
        } else {
            Spread {
                left: None,
                right: None,
            }
        };
    }
    let first_page = (spread_index - 1) * 2 + 1;
    let left = if first_page < page_count {
        Some(first_page)
    } else {
        None
    };
    let right = if first_page + 1 < page_count {
        Some(first_page + 1)
    } else {
        None
    };
    Spread { left, right }
}

/// Total number of spreads for a document with `page_count` pages.
pub fn spread_count(page_count: usize) -> usize {
    if page_count == 0 {
        return 0;
    }
    // Cover spread + ceil((page_count - 1) / 2) pair spreads
    1 + (page_count - 1).div_ceil(2)
}

/// Minimum window width (in logical px) to engage two-page spread mode.
/// Below this, the view forces single-page even if the user selected spread.
///
/// Reads
/// [`SizingTokens::layout_switch_wide`](enzui_iced_theme::SizingTokens::layout_switch_wide)
/// (1000, "at/above this, go fully wide"); engaging a two-page spread is going
/// fully wide. A `fn` (not a `const`) because `SizingTokens::default()` is not
/// `const`.
pub fn min_spread_width() -> f32 {
    enzui_iced_theme::SizingTokens::default().layout_switch_wide
}
