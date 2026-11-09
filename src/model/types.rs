//! Model small types: toasts, publications, enums, projections.

use super::*;

/// Stable identity prefix for the transient accessibility live region.
pub const TOAST_ID_PREFIX: &str = "reader-status-toast";

/// The channel key every transient acknowledgement shares, so a newer status replaces
/// the current line rather than stacking.
pub const READER_STATUS_COALESCE_KEY: &str = "reader-transient-status";

/// The reserved whole-document load attempt id that identifies no request.
///
/// [`Model::renew_document_load_attempt`] saturates here instead of wrapping, and
/// [`Model::admits_document_load`] refuses it, so an exhausted id space admits nothing.
pub const DOCUMENT_LOAD_ATTEMPT_EXHAUSTED: u64 = u64::MAX;

/// The exact immutable arranger draft being published off the reducer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageArrangementPublication {
    pub generation: u64,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub arrangement: PageArrangement,
}

/// Unresolved Save As publication summary; non-cloneable custody remains on App until
/// acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrangementUncertainPublication {
    /// The destination pathname that received the arranged PDF.
    pub destination: PathBuf,
    /// Number of pages the published arrangement carried when known
    /// (`0` for `PublishedButNotDurable` / `Lost` outcomes where the
    /// reopen verification did not run).
    pub page_count: usize,
    /// The bounded, user-visible sentence the shell banner and close
    /// veto summary render.
    pub message: String,
    /// Which framework custody variant the reducer holds on
    /// `crate::app::App::arrangement_uncertain_authority`.
    pub custody_kind: crate::plumbing::pdf_ops::ArrangementUncertainKind,
}

/// Uncertainty debt for a failing extract or split run whose worker
/// retained per-output custody in
/// `crate::app::App::extract_publication_debt`.
///
/// Carries counts and a classification, never pathnames, for the banner and close veto.
/// The retained descriptors live on `App`, since
/// [`enzui_iced_core::user_file::DocumentAuthority`] and
/// [`enzui_iced_core::user_file::LostCreateStage`] are not [`Clone`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractUncertainPublication {
    /// Number of outputs the run created successfully and
    /// reopen-verified through their retained descriptors before
    /// the failure.
    pub completed_count: usize,
    /// Classification of the failing output's custody.
    pub uncertain_kind: crate::plumbing::pdf_ops::ExtractUncertainKind,
    /// The bounded, user-visible sentence the shell banner and
    /// close-veto summary render. No pathname of a retained output
    /// appears in this string.
    pub message: String,
}

/// Retry authority retained with unsaved metadata after publication failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataSaveFailure {
    pub path: PathBuf,
    pub confirmed_removal: crate::plumbing::pdf_ops::MetadataDateRemoval,
    pub message: String,
}

/// Which destructive document operation a confirm dialog is currently asking
/// about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingPageAction {
    /// Delete the annotation whose stable sidecar id is carried.
    DeleteAnnotation(u64),
    /// Delete the page whose arranger entry id is carried.
    DeletePage(u64),
    /// Extract the page whose arranger entry id is carried, to a derived
    /// sibling path.
    ExtractPage(u64),
    /// Delete several arranger pages at once.
    DeletePages(Vec<u64>),
    /// Extract several pages into one new PDF beside this one.
    ExtractPages(Vec<u64>),
    /// Wipe the whole `/Info` dictionary (`Clear all metadata`).
    ClearMetadata,
    /// Remove explicitly emptied creation and modification dates.
    RemoveMetadataDates(crate::plumbing::pdf_ops::MetadataDateRemoval),
    /// `Split into pages` — write every page out as its own file. It asks
    /// because it writes N files beside the source, which for a 300-page PDF
    /// is a surprise the user should get to decline.
    SplitPdf,
}

impl PendingPageAction {
    /// The confirm dialog's title.
    pub fn title(&self) -> &'static str {
        match self {
            PendingPageAction::DeleteAnnotation(_) => "Delete annotation",
            PendingPageAction::DeletePage(_) => "Delete page",
            PendingPageAction::ExtractPage(_) => "Extract this page",
            PendingPageAction::DeletePages(_) => "Delete pages",
            PendingPageAction::ExtractPages(_) => "Extract pages",
            PendingPageAction::ClearMetadata => "Clear all metadata",
            PendingPageAction::RemoveMetadataDates(_) => "Remove PDF dates",
            PendingPageAction::SplitPdf => "Split into pages",
        }
    }

    /// The affirmative button's label.
    pub fn confirm_label(&self) -> &'static str {
        match self {
            PendingPageAction::DeleteAnnotation(_) => "Delete annotation",
            PendingPageAction::DeletePage(_) => "Delete page",
            PendingPageAction::ExtractPage(_) => "Extract",
            PendingPageAction::DeletePages(_) => "Delete pages",
            PendingPageAction::ExtractPages(_) => "Extract",
            PendingPageAction::ClearMetadata => "Clear Metadata",
            PendingPageAction::RemoveMetadataDates(_) => "Remove dates",
            PendingPageAction::SplitPdf => "Split PDF",
        }
    }

    /// Whether the affirmative action destroys user data (drives the
    /// destructive button tone). Extraction and splitting write *new* files
    /// and leave the source alone, so they are not destructive.
    pub fn is_destructive(&self) -> bool {
        !matches!(
            self,
            PendingPageAction::ExtractPage(_)
                | PendingPageAction::ExtractPages(_)
                | PendingPageAction::SplitPdf
        )
    }
}

/// Whether the password dialog puts a password on the PDF or takes it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordPurpose {
    Protect,
    Remove,
}

/// The open password dialog: its purpose, its form, and why the last
/// attempt was refused.
#[derive(Debug)]
pub struct PdfPasswordForm {
    pub purpose: PasswordPurpose,
    pub presenter: enzui_iced_widgets::form_dialog::FormPresenter,
    pub error: Option<String>,
}

/// Which overlay dialog, if any, is showing over the reading surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The page arranger (`Page arranger`), a full-surface mode rather than a
    /// floating dialog.
    PageArranger,
    /// The metadata editor (`Edit Metadata`).
    Metadata,
    /// Read-only `Document Properties`.
    Properties,
    /// The encrypted-PDF unlock prompt (`Password Required`).
    Password,
    /// `Protect with Password…` / `Remove Password…` on the open PDF.
    PdfPassword,
    /// The `New Note` / `Edit Note` composer.
    NoteEditor,
    /// Keyboard Shortcuts help dialog. The discovery surface for every
    /// binding in `shortcuts::bindings()`, reachable by F1.
    Shortcuts,
    /// `Sign…`: the name, its handwriting and an optional certificate.
    Sign,
}

/// Which document format is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Epub,
    Cbz,
}

macro_rules! screen_routes {
    ($( $(#[$docs:meta])* $route:ident ),+ $(,)?) => {
        /// Which top-level screen is showing.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Screen {
            $( $(#[$docs])* $route, )+
        }

        impl Screen {
            /// Every top-level route, generated by the enum declaration so a
            /// route-wide census cannot silently omit a newly added screen.
            pub const ALL: &'static [Self] = &[$(Self::$route),+];
        }
    };
}

screen_routes! {
    /// The library browser and home screen: every known document, with filter, sort and a
    /// "Continue reading" section. Closing a document returns here.
    Library,
    /// A document is open and being read.
    Reading,
}

/// How the library browser orders its list.
///
/// The default is [`Self::Recent`], matching the "Continue reading" order. `Title` and
/// `Author` are case-insensitive ascending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibrarySort {
    /// Most-recently-opened first (the default). Same order as recents.
    Recent,
    /// By title, A→Z, case-insensitive. Empty titles sort last.
    Title,
    /// By author, A→Z, case-insensitive. Empty authors sort last.
    Author,
}

impl LibrarySort {
    /// All sort modes in presentation (cycle) order.
    pub const ALL: &'static [LibrarySort] =
        &[LibrarySort::Recent, LibrarySort::Title, LibrarySort::Author];

    /// The sort control's label, naming the current order.
    pub fn label(self) -> &'static str {
        match self {
            LibrarySort::Recent => "Recent",
            LibrarySort::Title => "Title",
            LibrarySort::Author => "Author",
        }
    }

    /// The Sort menu row for this mode.
    pub fn menu_label(self) -> &'static str {
        match self {
            LibrarySort::Recent => "Recently Opened",
            LibrarySort::Title => "Title",
            LibrarySort::Author => "Author",
        }
    }

    pub(crate) fn menu_id(self) -> &'static str {
        match self {
            LibrarySort::Recent => "sort-recent",
            LibrarySort::Title => "sort-title",
            LibrarySort::Author => "sort-author",
        }
    }

    /// The mode a Sort menu row stands for.
    pub fn from_menu_id(id: &str) -> Option<LibrarySort> {
        Self::ALL.iter().copied().find(|sort| sort.menu_id() == id)
    }

    /// The Sort menu: every mode, the current one checked.
    pub fn menu_entries(current: LibrarySort) -> Vec<enzui_iced_core::menu_model::MenuEntryModel> {
        use enzui_iced_core::menu_model::{CheckState, MenuEntryModel};
        Self::ALL
            .iter()
            .map(|&sort| {
                MenuEntryModel::command(sort.menu_id(), sort.menu_label()).check(
                    if sort == current {
                        CheckState::Checked
                    } else {
                        CheckState::Unchecked
                    },
                )
            })
            .collect()
    }
}

/// EPUB reading sub-mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpubViewMode {
    Scrolled,
    Paginated,
}

impl EpubViewMode {
    /// Stable identifier stored in a book's progress record.
    pub fn id(self) -> &'static str {
        match self {
            Self::Scrolled => "scrolled",
            Self::Paginated => "paginated",
        }
    }

    /// Restore a persisted identifier, falling back to the documented
    /// scrolled mode for missing-era or unrecognised values.
    pub fn from_id(id: &str) -> Self {
        match id {
            "paginated" => Self::Paginated,
            _ => Self::Scrolled,
        }
    }
}

/// The reading sidebar panels, each with a live data source or a format-specific empty
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    /// Table of contents / outline.
    Contents,
    /// Page-thumbnail navigation for paged documents, shown as "Pages": the
    /// default view for a PDF or a comic.
    Thumbnails,
    /// Annotations + bookmarks list, with add-note, filter, delete, and copy.
    Annotations,
    /// Find-in-document: EPUB chapter matches or page-numbered PDF text matches.
    Search,
    /// Bookmarks, with a per-row `Remove bookmark`.
    Bookmarks,
    /// EPUB footnotes and endnotes, grouped by chapter.
    Footnotes,
}

/// Named reading theme — the colour palette for the reading surface.
///
/// Based on Foliate's themes. The ids are persisted in sidecars and must not change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReadingTheme {
    Light,
    Sepia,
    Gray,
    Grass,
    Sky,
    Solarized,
    Nord,
    Dark,
}

impl ReadingTheme {
    /// All available themes in presentation order.
    pub const ALL: &'static [ReadingTheme] = &[
        ReadingTheme::Light,
        ReadingTheme::Sepia,
        ReadingTheme::Gray,
        ReadingTheme::Grass,
        ReadingTheme::Sky,
        ReadingTheme::Solarized,
        ReadingTheme::Nord,
        ReadingTheme::Dark,
    ];

    /// Stable string identifier used in persistence.
    pub fn id(self) -> &'static str {
        match self {
            ReadingTheme::Light => "light",
            ReadingTheme::Sepia => "sepia",
            ReadingTheme::Gray => "gray",
            ReadingTheme::Grass => "grass",
            ReadingTheme::Sky => "sky",
            ReadingTheme::Solarized => "solarized",
            ReadingTheme::Nord => "nord",
            ReadingTheme::Dark => "dark",
        }
    }

    /// Human-readable display name.
    pub fn label(self) -> &'static str {
        match self {
            ReadingTheme::Light => "Light",
            ReadingTheme::Sepia => "Sepia",
            ReadingTheme::Gray => "Gray",
            ReadingTheme::Grass => "Grass",
            ReadingTheme::Sky => "Sky",
            ReadingTheme::Solarized => "Solarized",
            ReadingTheme::Nord => "Nord",
            ReadingTheme::Dark => "Dark",
        }
    }

    /// Parse from the persisted string id. Falls back to `Light`.
    pub fn from_id(s: &str) -> Self {
        match s {
            "light" => ReadingTheme::Light,
            "sepia" => ReadingTheme::Sepia,
            "gray" => ReadingTheme::Gray,
            "grass" => ReadingTheme::Grass,
            "sky" => ReadingTheme::Sky,
            "solarized" => ReadingTheme::Solarized,
            "nord" => ReadingTheme::Nord,
            "dark" => ReadingTheme::Dark,
            _ => ReadingTheme::Light,
        }
    }

    /// Migrate from the older `epub_dark_theme` boolean.
    pub fn from_legacy_bool(dark: bool) -> Self {
        if dark {
            ReadingTheme::Dark
        } else {
            ReadingTheme::Light
        }
    }

    /// Whether this theme is "dark" (light text on dark background).
    pub fn is_dark(self) -> bool {
        matches!(self, ReadingTheme::Dark)
    }

    /// The next theme in [`Self::ALL`] order, wrapping around. Used by the
    /// toolbar palette button to cycle reading themes until the swatch
    /// popover lands.
    pub fn next(self) -> ReadingTheme {
        let all = Self::ALL;
        let idx = all.iter().position(|&t| t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    /// Colour palette for the reading surface, `(bg, fg, link)` in 0xRRGGBB.
    pub fn colors(self) -> ReadingThemeColors {
        match self {
            ReadingTheme::Light => ReadingThemeColors {
                bg: 0xffffff,
                fg: 0x1a1a1a,
                link: 0x0066cc,
                card_bg: 0xffffff,
                separator: 0xe0e0e0,
                surface_bg: 0xf5f5f5,
            },
            ReadingTheme::Sepia => ReadingThemeColors {
                bg: 0xf1e8d0,
                fg: 0x5b4636,
                link: 0x008b8b,
                card_bg: 0xfbf3de,
                separator: 0xd9c9a3,
                surface_bg: 0xf0e6d0,
            },
            ReadingTheme::Gray => ReadingThemeColors {
                bg: 0xe0e0e0,
                fg: 0x222222,
                link: 0x4488cc,
                card_bg: 0xeaeaea,
                separator: 0xc0c0c0,
                surface_bg: 0xd6d6d6,
            },
            ReadingTheme::Grass => ReadingThemeColors {
                bg: 0xd7dbbd,
                fg: 0x232c16,
                link: 0x177b4d,
                card_bg: 0xe2e6cc,
                separator: 0xb8be98,
                surface_bg: 0xced2b4,
            },
            ReadingTheme::Sky => ReadingThemeColors {
                bg: 0xcedef5,
                fg: 0x262d48,
                link: 0x2d53e5,
                card_bg: 0xdbe8fa,
                separator: 0xaec0dd,
                surface_bg: 0xc4d4ee,
            },
            ReadingTheme::Solarized => ReadingThemeColors {
                bg: 0xfdf6e3,
                fg: 0x586e75,
                link: 0x268bd2,
                card_bg: 0xfdf6e3,
                separator: 0xd4cdb8,
                surface_bg: 0xf5eed9,
            },
            ReadingTheme::Nord => ReadingThemeColors {
                bg: 0xeceff4,
                fg: 0x2e3440,
                link: 0x5e81ac,
                card_bg: 0xeceff4,
                separator: 0xc8cdd5,
                surface_bg: 0xe2e6ec,
            },
            ReadingTheme::Dark => ReadingThemeColors {
                bg: 0x222222,
                fg: 0xe0e0e0,
                link: 0x77bbee,
                card_bg: 0x2a2a2a,
                separator: 0x444444,
                surface_bg: 0x181818,
            },
        }
    }
}

/// Colour components for a reading theme, `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadingThemeColors {
    /// Page/card background.
    pub bg: u32,
    /// Body text foreground.
    pub fg: u32,
    /// Link / footnote-ref colour.
    pub link: u32,
    /// EPUB chapter-card background.
    pub card_bg: u32,
    /// Separator / border colour between chapter cards.
    pub separator: u32,
    /// Surface behind the cards (scroll area background).
    pub surface_bg: u32,
}

// Sort key for a library title/author string: lowercased, with empty
// strings mapped to a sentinel that sorts after any real value, so untitled books fall
// to the end of an A→Z list.
pub(super) fn library_sort_key(s: &str) -> (u8, String) {
    let t = s.trim();
    if t.is_empty() {
        // Non-empty strings get bucket 0; empties bucket 1 (sorts last).
        (1, String::new())
    } else {
        (0, t.to_lowercase())
    }
}

/// The filtered, sorted library rows retained in application state.
///
/// `enz_virtual_list` borrows this provider every frame, so filtering and sorting run only
/// when inputs change. Rows are keyed by canonical path, which survives insertions above.
#[derive(Debug, Default)]
pub struct LibraryProjection {
    rows: Vec<RecentBook>,
}

impl LibraryProjection {
    pub(super) fn compute(library: &[RecentBook], query: &str, sort: LibrarySort) -> Self {
        let needle = query.trim().to_lowercase();
        let mut rows: Vec<RecentBook> = library
            .iter()
            .filter(|book| {
                needle.is_empty()
                    || book.title.to_lowercase().contains(&needle)
                    || book.author.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        match sort {
            LibrarySort::Recent => {}
            LibrarySort::Title => rows.sort_by_key(|book| library_sort_key(&book.title)),
            LibrarySort::Author => rows.sort_by_key(|book| library_sort_key(&book.author)),
        }
        Self { rows }
    }

    pub fn rows(&self) -> &[RecentBook] {
        &self.rows
    }

    // Update `file_missing` on matching rows from `updates` (path -> exists).
    // Returns whether any row changed. Order is untouched — availability is
    // not a sort key.
    pub(super) fn apply_availability(
        &mut self,
        updates: &std::collections::HashMap<String, bool>,
    ) -> bool {
        let mut changed = false;
        for row in &mut self.rows {
            if let Some(&exists) = updates.get(&row.path) {
                let missing = !exists;
                if row.file_missing != missing {
                    row.file_missing = missing;
                    changed = true;
                }
            }
        }
        changed
    }

    pub(super) fn apply_bookmark_counts(
        &mut self,
        updates: &std::collections::HashMap<String, usize>,
    ) -> bool {
        let mut changed = false;
        for row in &mut self.rows {
            if let Some(&count) = updates.get(&row.path)
                && row.bookmark_count != count
            {
                row.bookmark_count = count;
                changed = true;
            }
        }
        changed
    }
}

impl ItemProvider<RecentBook> for LibraryProjection {
    fn len(&self) -> u64 {
        self.rows.len() as u64
    }

    fn item(&self, index: u64) -> Option<RecentBook> {
        self.rows.get(usize::try_from(index).ok()?).cloned()
    }
}

impl KeyedProvider<RecentBook> for LibraryProjection {
    fn key(&self, index: u64) -> Option<SequenceKey> {
        self.rows
            .get(usize::try_from(index).ok()?)
            .map(|book| SequenceKey::new(book.path.clone()))
    }
}

/// A per-frame cache for the library's variable row heights.
///
/// Row pitch comes from the shaped height of each title, author, stats and missing-file
/// caption. The table is reshaped only when the key changes: the book set
/// ([`Model::library_revision`]), the content width or the typography ramp.
///
/// `view(&self)` borrows the model shared, so the cache is interior-mutable and lives in
/// [`Model`] across frames. Its two counters prove the reshape work is cache-bounded.
#[derive(Debug, Default)]
pub struct LibraryHeightCache {
    inner: std::cell::RefCell<LibraryHeightCacheState>,
    /// Total individual text-shaping calls performed while (re)building the
    /// table. A cache hit adds none.
    shape_calls: std::cell::Cell<u64>,
    /// How many times the whole table was reshaped (one per key change).
    recomputes: std::cell::Cell<u64>,
}

#[derive(Debug, Default)]
struct LibraryHeightCacheState {
    key: Option<u128>,
    heights: std::rc::Rc<Vec<f32>>,
}

impl LibraryHeightCache {
    /// Return the height table for `key`, recomputing it via `build` on a miss.
    ///
    /// `build` must bump the shaping counter once per text it shapes. On a hit neither
    /// counter moves.
    pub fn get(
        &self,
        key: u128,
        build: impl FnOnce(&std::cell::Cell<u64>) -> Vec<f32>,
    ) -> std::rc::Rc<Vec<f32>> {
        let mut state = self.inner.borrow_mut();
        if state.key != Some(key) {
            self.recomputes.set(self.recomputes.get() + 1);
            state.heights = std::rc::Rc::new(build(&self.shape_calls));
            state.key = Some(key);
        }
        std::rc::Rc::clone(&state.heights)
    }

    /// Total text-shaping calls performed so far (test instrument).
    #[must_use]
    pub fn shape_calls(&self) -> u64 {
        self.shape_calls.get()
    }

    /// Total full-table reshapes performed so far (test instrument).
    #[must_use]
    pub fn recomputes(&self) -> u64 {
        self.recomputes.get()
    }
}

/// Convert a `0xRRGGBB` value to an iced [`iced::Color`].
///
/// Reading themes carry their own raw `0xRRGGBB` hues, not `Palette` tokens, so this
/// decodes theme data with `Color::from_rgb8`.
pub fn rgb(hex: u32) -> iced::Color {
    iced::Color::from_rgb8(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}
