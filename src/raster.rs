//! Byte-budgeted page and thumbnail raster caches.
//!
//! Eviction drops the furthest page outside the protected visible band. That band
//! may exceed the budget so visible content remains resident.

use std::collections::HashMap;

/// The pixel size of a page's raster: what `crate::pdf::render_pages` produces
/// for that page, and what the column reserves for it before it exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageExtent {
    pub width: u32,
    pub height: u32,
}

impl PageExtent {
    /// The RGBA cost of holding this page's raster.
    ///
    /// `iced::widget::image::Handle::from_rgba` keeps the buffer, so this is the real
    /// residency of one page.
    pub fn bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// The resident-raster budget, in bytes.
///
/// Accounts for CPU RGBA buffers; native image allocations retain a corresponding
/// GPU copy until cache eviction.
pub const RASTER_BUDGET_BYTES: usize = 192 * 1024 * 1024;
pub const THUMBNAIL_BUDGET_BYTES: usize = 8 * 1024 * 1024;
pub const THUMBNAIL_BATCH_LIMIT: usize = 12;
/// A sidebar page row: the 108 px thumbnail plus an 8 px gap.
pub const THUMBNAIL_ROW_HEIGHT: f32 = 116.0;

pub fn thumbnail_band(
    page_count: usize,
    offset: f32,
    viewport_height: f32,
) -> Option<std::ops::RangeInclusive<usize>> {
    if page_count == 0 {
        return None;
    }
    let first_visible = (offset.max(0.0) / THUMBNAIL_ROW_HEIGHT).floor() as usize;
    let visible = (viewport_height.max(1.0) / THUMBNAIL_ROW_HEIGHT).ceil() as usize + 1;
    let start = first_visible.saturating_sub(2).min(page_count - 1);
    let end = first_visible
        .saturating_add(visible)
        .saturating_add(2)
        .min(page_count - 1)
        .min(start.saturating_add(THUMBNAIL_BATCH_LIMIT - 1));
    Some(start..=end)
}

/// Off-screen raster lookahead in viewport heights; jumps recompute the band at their
/// destination.
pub const LOOKAHEAD_VIEWPORTS: f32 = 1.0;

/// A rendered PDF/CBZ page image, held in state (iced `image::Handle` is
/// cloneable and `Send`, so pages move freely between tasks).
#[derive(Clone, Debug)]
pub struct RenderedPage {
    pub index: usize,
    pub handle: iced::widget::image::Handle,
    /// Native pixel size, for aspect-ratio-correct fit-width scaling.
    pub width: u32,
    pub height: u32,
    /// Keeps the image drawable while off screen; a CPU handle alone permits GPU eviction.
    pub allocation: Option<iced::widget::image::Allocation>,
}

impl RenderedPage {
    /// This page's residency, from the same arithmetic as [`PageExtent::bytes`].
    pub fn bytes(&self) -> usize {
        PageExtent {
            width: self.width,
            height: self.height,
        }
        .bytes()
    }
}

/// The rasters currently resident, bounded in bytes.
///
/// Keyed by source page index, the identity the arranger's `PageSource` and the text
/// layer share.
#[derive(Debug)]
pub struct PageRasterCache {
    entries: HashMap<usize, RenderedPage>,
    /// Pages whose raster is out of date because the file changed. They stay on screen
    /// and count as missing until a fresh raster replaces them.
    stale: std::collections::HashSet<usize>,
    bytes: usize,
    budget: usize,
    /// Cumulative page evictions caused by the residency budget.
    evictions: u64,
}

impl Default for PageRasterCache {
    fn default() -> Self {
        Self::with_budget(RASTER_BUDGET_BYTES)
    }
}

impl PageRasterCache {
    /// A cache with an explicit budget, so tests reach the bound without 192 MiB of
    /// fixture.
    pub fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            stale: std::collections::HashSet::new(),
            bytes: 0,
            budget,
            evictions: 0,
        }
    }

    /// The raster of page `index`, if it is resident.
    ///
    /// `&self` so the view can read it; eviction order comes from the protected band
    /// passed to [`Self::insert`], not from reads.
    pub fn get(&self, index: usize) -> Option<&RenderedPage> {
        self.entries.get(&index)
    }

    /// Whether page `index` has a current raster. A stale one does not count.
    pub fn contains(&self, index: usize) -> bool {
        self.entries.contains_key(&index) && !self.stale.contains(&index)
    }

    /// Keep page `index`'s raster on screen but render it again, since the document
    /// changed under it.
    pub fn mark_stale(&mut self, index: usize) {
        if self.entries.contains_key(&index) {
            self.stale.insert(index);
        }
    }

    /// Page `index`'s fresh raster has arrived and is being allocated: keep
    /// showing the old one, but stop asking for another.
    pub fn mark_refreshing(&mut self, index: usize) {
        self.stale.remove(&index);
    }

    /// How many pages are resident.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resident bytes of RGBA, the unit of the bound.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    /// How many pages have been evicted over this cache's life.
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// Resident page indices in ascending order.
    pub fn resident(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.entries.keys().copied().collect();
        out.sort_unstable();
        out
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.stale.clear();
        self.bytes = 0;
    }

    pub fn remove(&mut self, index: usize) -> bool {
        self.stale.remove(&index);
        let Some(dropped) = self.entries.remove(&index) else {
            return false;
        };
        self.bytes = self.bytes.saturating_sub(dropped.bytes());
        true
    }

    /// Which of `want` are not resident, in the order given.
    pub fn missing(&self, want: &[usize]) -> Vec<usize> {
        want.iter()
            .copied()
            .filter(|i| !self.contains(*i))
            .collect()
    }

    /// Insert `page`, then evict down to the budget.
    ///
    /// `protect` is the band the viewport can see and is never dropped (see the module
    /// docs); other pages go furthest from the band first.
    pub fn insert(&mut self, page: RenderedPage, protect: std::ops::RangeInclusive<usize>) {
        let cost = page.bytes();
        self.stale.remove(&page.index);
        if let Some(old) = self.entries.insert(page.index, page) {
            self.bytes = self.bytes.saturating_sub(old.bytes());
        }
        self.bytes = self.bytes.saturating_add(cost);
        self.evict_to_budget(&protect);
    }

    // Drop the furthest non-protected pages until the budget is met, or until
    // only protected pages remain.
    fn evict_to_budget(&mut self, protect: &std::ops::RangeInclusive<usize>) {
        let anchor = (*protect.start() + *protect.end()) / 2;
        while self.bytes > self.budget {
            let victim = self
                .entries
                .keys()
                .copied()
                .filter(|i| !protect.contains(i))
                .max_by_key(|i| i.abs_diff(anchor));
            let Some(victim) = victim else {
                // Only the visible band remains; keeping it is the stated exception.
                return;
            };
            self.stale.remove(&victim);
            if let Some(dropped) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(dropped.bytes());
                self.evictions += 1;
            }
        }
    }
}

/// The inclusive page range a viewport of `viewport_height` at scroll `offset`
/// can reach, given each page's laid-out `heights` (top to bottom), the
/// column's `top_pad` and the `gap` between pages.
///
/// A page is in the band when its box intersects the viewport widened by
/// [`LOOKAHEAD_VIEWPORTS`] above and below. Returns `None` for an empty document. Uses the
/// same heights `crate::view::reading` lays out, so band and geometry agree.
pub fn band(
    heights: &[f32],
    top_pad: f32,
    gap: f32,
    offset: f32,
    viewport_height: f32,
) -> Option<std::ops::RangeInclusive<usize>> {
    if heights.is_empty() {
        return None;
    }
    let viewport = viewport_height.max(1.0);
    let look = viewport * LOOKAHEAD_VIEWPORTS;
    let top = offset.max(0.0) - look;
    let bottom = offset.max(0.0) + viewport + look;

    let mut y = top_pad;
    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for (index, height) in heights.iter().enumerate() {
        let page_top = y;
        let page_bottom = y + height;
        if page_bottom >= top && page_top <= bottom {
            first.get_or_insert(index);
            last = index;
        } else if first.is_some() {
            break;
        }
        y = page_bottom + gap;
    }
    // An offset past the end resolves to the nearest page rather than a blank viewport.
    let first = first.unwrap_or(heights.len() - 1);
    Some(first..=last.max(first))
}
