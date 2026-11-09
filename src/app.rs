//! The iced `App`: state, reducer, and the async document-load tasks.
//!

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use enzui_iced_core::app_data::AppDataResolver;
use enzui_iced_core::blocking_task::BlockingTaskError;
use enzui_iced_core::cancellation;
use enzui_iced_core::theme_file::integration::ThemeIntegration;
use enzui_iced_core::{
    focus::{Traversal, traversal_operation as traverse},
    persist::XdgResolver,
};
use enzui_iced_theme::EnzThemeMode;
use enzui_iced_widgets::toast_host::{ToastEvent, ToastOverlayPlacement, enz_toast_layer};
use iced::{Element, Subscription, Task, advanced::widget::operate};

use crate::message::{LoadOutcome, Msg};
use crate::model::{EpubDoc, EpubViewMode, Format, Model, ReadingTheme, Screen};
use crate::plumbing::epub_support::{self, EpubLoadResult, ReadingParagraph};
use crate::plumbing::persistence::{self, ProgressRecord};
use crate::undo::{self, UndoableAction};
use crate::{message, model, pdf, plumbing, services, view};

#[path = "app_parts/app_state.rs"]
mod app_state;
#[path = "app_parts/classify.rs"]
mod classify;
#[path = "app_parts/reduce_batch1.rs"]
mod reduce_batch1;
#[path = "app_parts/reduce_batch2.rs"]
mod reduce_batch2;
#[path = "app_parts/reduce_batch3.rs"]
mod reduce_batch3;
#[path = "app_parts/reduce_batch4.rs"]
mod reduce_batch4;
#[path = "app_parts/reduce_batch5.rs"]
mod reduce_batch5;
#[path = "app_parts/reduce_batch6.rs"]
mod reduce_batch6;
#[path = "app_parts/reduce_batch7.rs"]
mod reduce_batch7;
#[path = "app_parts/shell.rs"]
mod shell;
#[path = "app_parts/support.rs"]
mod support;
pub use app_state::App;
pub(crate) use classify::AVAILABILITY_BATCH;
#[cfg(test)]
pub(crate) use classify::refresh_projection;
pub use classify::{classify_event, classify_event_in};
pub(crate) use support::{
    AdmissionResult, AnnotationCustody, AvailabilityScan, BUSY_RETRY_INTERVAL,
    BlockingOccupancyGuard, CLOSE_DRAIN_BUDGET, CLOSE_DRAIN_TICK, DebtStatus, EditCommentOutcome,
    FsMetadataBackend, HISTORY_STACK_DRIFT_SUMMARY, HistoryDecision, MetadataBackend,
    PERSIST_FLUSH_INTERVAL, PREVIEW_INFLIGHT_CAP, PdfEditContentsIntent, PdfMutationInFlight,
    PdfMutationPending, PdfReconciliationClass, PdfRemoveIntent, PdfUpdateColorIntent,
    PendingDocumentLoad, PendingLoad, PendingPersistence, PendingPick, PersistenceSnapshot,
    RECONCILIATION_BLOCKED_ADMIT_SUMMARY, RETAINED_SLOT_CANCELLED_BY_DISMISS_SUMMARY,
    RETRY_OCCUPANCY_BLOCKED_ADMIT_SUMMARY, UNCERTAIN_DISMISSED_SUMMARY,
    UNCERTAIN_PUBLICATION_SUMMARY, UNCERTAIN_RETRY_CLEARED_SUMMARY,
    UNCERTAIN_RETRY_EXHAUSTED_SUMMARY, UNCERTAIN_RETRY_FAILED_SUMMARY,
    UNCERTAIN_RETRY_MISMATCH_SUMMARY, UNCERTAIN_RETRY_NOT_APPLIED_SUMMARY, UncertainPublication,
    WRITER_OCCUPANCY_BLOCKED_ADMIT_SUMMARY, classify_reconciliation_via_authority,
    export_result_to_outcome, native_annotation_rect_and_subtype, overlay_restore_id,
    pdf_worker_terminal_message, reading_theme_for_mode, restore_focus_task,
};
pub use support::{CloseState, PdfCreateIntent, PdfMutationIntent};
#[path = "app_parts/annotations.rs"]
mod annotations;
#[path = "app_parts/arrangement.rs"]
mod arrangement;
#[path = "app_parts/boot.rs"]
mod boot;
#[path = "app_parts/close.rs"]
mod close;
#[path = "app_parts/loading.rs"]
mod loading;
#[path = "app_parts/mutation_a.rs"]
mod mutation_a;
#[path = "app_parts/mutation_b.rs"]
mod mutation_b;
#[path = "app_parts/mutation_c.rs"]
mod mutation_c;
#[path = "app_parts/navigation.rs"]
mod navigation;
#[path = "app_parts/persist.rs"]
mod persist;
#[path = "app_parts/preview.rs"]
mod preview;
#[path = "app_parts/raster.rs"]
mod raster;
#[path = "app_parts/reduce.rs"]
mod reduce;
#[path = "app_parts/signing.rs"]
mod signing;

impl App {
    // Toggle and render use the same width-dependent sidebar presentation.
    fn sidebar_presentation(&self) -> enzui_iced_widgets::nav_split::SidebarPresentation {
        crate::view::sidebar_presentation(&self.model, self.window_width)
    }

    // Ensure the sidebar is visible at the current width so a targeted panel is on screen.
    //
    // Inline needs nothing; Overlay opens the drawer without touching the durable
    // preference; Hidden restores `Shown`, which resolves to a drawer or inline by width.
    fn ensure_sidebar_visible(&mut self) {
        use enzui_iced_widgets::nav_split::{SidebarPreference, SidebarPresentation};
        match self.sidebar_presentation() {
            SidebarPresentation::Inline => { /* already visible */ }
            SidebarPresentation::Overlay => {
                self.model.nav.overlay_open = true;
            }
            SidebarPresentation::Hidden => {
                self.model.nav.preference = SidebarPreference::Shown;
                // At a narrow width `Shown` resolves to Overlay, so the drawer
                // must also be opened for the panel to appear immediately.
                if !crate::view::sidebar_presentation(&self.model, self.window_width).is_inline() {
                    self.model.nav.overlay_open = true;
                }
            }
        }
    }

    /// The shared XDG/OS theme for chrome and shell-following reading palettes.
    pub fn theme(&self) -> iced::Theme {
        self.themes.iced_theme()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        self.render_view()
    }

    /// Window-event subscription driving the responsive breakpoints.
    pub fn subscription(&self) -> Subscription<Msg> {
        self.build_subscription()
    }
}

/// Preview jobs for available documents whose thumbnails are not cached.
pub fn preview_jobs(model: &Model) -> Vec<(String, PreviewSource)> {
    model
        .recents
        .iter()
        .filter(|book| !book.file_missing)
        .filter(|book| !model.recent_thumbnails.contains_key(&book.path))
        .filter_map(|book| {
            PreviewSource::for_format(&book.format).map(|kind| (book.path.clone(), kind))
        })
        .collect()
}

/// Preview renderer selected by document format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSource {
    /// Rasterise page 0 (PDF).
    FirstPageRaster,
    /// Decode the archive's declared cover image (EPUB).
    CoverImage,
    /// Decode the first naturally ordered page image (CBZ).
    FirstComicPage,
}

impl PreviewSource {
    /// The preview source for a library `format` string, or `None` when the
    /// format has no preview in this build.
    ///
    /// Case-insensitive: the persisted format is whatever the loader recorded
    /// (`"epub"`, `"EPUB"`).
    pub fn for_format(format: &str) -> Option<Self> {
        if format.eq_ignore_ascii_case("pdf") {
            Some(Self::FirstPageRaster)
        } else if format.eq_ignore_ascii_case("epub") {
            Some(Self::CoverImage)
        } else if format.eq_ignore_ascii_case("cbz") {
            Some(Self::FirstComicPage)
        } else {
            None
        }
    }

    /// Produce the thumbnail, or `None` when this document has none to show.
    pub fn render(self, path: &str) -> Option<crate::message::PageThumbnail> {
        match self {
            Self::FirstPageRaster => pdf::render_first_page_thumbnail(path),
            Self::CoverImage => {
                plumbing::epub_support::cover_thumbnail(path, pdf::THUMBNAIL_WIDTH as u32).map(
                    |c| crate::message::PageThumbnail {
                        rgba: c.rgba,
                        width: c.width,
                        height: c.height,
                    },
                )
            }
            Self::FirstComicPage => plumbing::cbz_support::render_first_page_thumbnail(
                path,
                pdf::THUMBNAIL_WIDTH as u32,
            ),
        }
    }
}

/// The library and persistence display title: the EPUB's trimmed `<dc:title>` when
/// present, otherwise the file name, so an untitled book never shows as a blank entry.
pub fn display_title(metadata_title: &str, file_name: &str) -> String {
    let trimmed = metadata_title.trim();
    if trimmed.is_empty() {
        file_name.to_string()
    } else {
        // Trimmed: fixture titles carry trailing spaces that would reach the titlebar and
        // the library record.
        trimmed.to_string()
    }
}

pub fn first_readable_chapter(chapters: &[plumbing::epub_support::EpubChapter]) -> usize {
    // Shares `content_chapter_indices`' definition of content, so the first open lands on
    // the chapter the counter labels "Chapter 1"; falls back to 0.
    plumbing::epub_support::content_chapter_indices(chapters)
        .first()
        .copied()
        .unwrap_or(0)
}

fn load_document_blocking(path: PathBuf, format: Format) -> LoadOutcome {
    match format {
        Format::Epub => match epub_support::load_epub(&path.to_string_lossy()) {
            Ok(load) => LoadOutcome::Epub(Box::new(load)),
            Err(summary) => LoadOutcome::Refused(summary),
        },
        // `render_pdf` fails only with the classifier's bounded summary.
        Format::Pdf => match pdf::render_pdf(&path.to_string_lossy(), None) {
            Ok(render) => LoadOutcome::Pages {
                pages: render.pages,
                total_pages: render.total_pages,
                extents: render.extents,
                text: render.text,
            },
            Err(summary) => LoadOutcome::Refused(summary),
        },
        Format::Cbz => match plumbing::cbz_support::load_cbz(&path) {
            Ok(load) => LoadOutcome::Cbz(Box::new(load)),
            Err(error) => LoadOutcome::Refused(error.to_string()),
        },
    }
}

// Retry an encrypted PDF without retaining its password in a message.
fn unlock_pdf_blocking(path: PathBuf, password: String) -> LoadOutcome {
    match pdf::render_pdf(&path.to_string_lossy(), Some(&password)) {
        Ok(render) => LoadOutcome::Pages {
            pages: render.pages,
            total_pages: render.total_pages,
            extents: render.extents,
            text: render.text,
        },
        Err(summary) => LoadOutcome::Refused(summary),
    }
}

// Rasterise one band of PDF or CBZ pages off the render thread.
fn rasterise_band_blocking(
    path: PathBuf,
    format: Format,
    password: Option<String>,
    cbz_entries: std::sync::Arc<Vec<plumbing::cbz_support::CbzPage>>,
    indices: Vec<usize>,
) -> (Vec<message::PageBitmap>, Vec<pdf::PdfPageText>) {
    match format {
        Format::Pdf => {
            match pdf::render_pages(&path.to_string_lossy(), password.as_deref(), &indices) {
                Ok(render) => (render.pages, render.text),
                Err(_) => (Vec::new(), Vec::new()),
            }
        }
        Format::Cbz => match plumbing::cbz_support::render_pages(&path, &cbz_entries, &indices) {
            Ok(pages) => {
                let text = vec![pdf::PdfPageText::default(); pages.len()];
                (pages, text)
            }
            Err(_) => (Vec::new(), Vec::new()),
        },
        Format::Epub => (Vec::new(), Vec::new()),
    }
}

#[cfg(test)]
pub(crate) fn test_theme_integration(
    probe: enzui_iced_core::theme_file::ProbeOverride,
) -> ThemeIntegration {
    use enzui_iced_core::file_watch::InMemoryBackend;
    use enzui_iced_core::theme_file::ThemePaths;
    use enzui_iced_core::theme_file::integration::ThemeIntegrationConfig;
    ThemeIntegration::boot_with(ThemeIntegrationConfig::injected(
        ThemePaths::new(
            Some("/nonexistent/reader-theme"),
            Vec::<&str>::new(),
            None::<&str>,
        ),
        std::sync::Arc::new(InMemoryBackend::new()),
        probe,
        EnzThemeMode::Dark,
    ))
    .0
}

#[cfg(test)]
pub(crate) fn test_xdg_resolver(root: &Path) -> XdgResolver {
    XdgResolver::new(
        None::<PathBuf>,
        Some(root.to_path_buf()),
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

/// Drive a full find-in-document search to completion the way the runtime would: type
/// the query, fire the debounce tick, run the off-thread scan, and feed the completion
/// back in. Test-only: production never scans synchronously.
#[cfg(test)]
impl App {
    pub(crate) fn run_search_to_completion(&mut self, query: &str) {
        // Drain every task each reducer arm returns, transitively, to a fixed point, so
        // a task added to any search arm runs here as it would at runtime.
        let mut pending: std::collections::VecDeque<Msg> = std::collections::VecDeque::new();
        pending.push_back(Msg::SearchQueryChanged(query.to_string()));
        pending.push_back(Msg::SearchTick);
        // Bound so a message cycle fails instead of hanging the test.
        let mut budget = 256usize;
        while let Some(msg) = pending.pop_front() {
            budget = budget.checked_sub(1).expect(
                "run_search_to_completion exceeded its message budget: a reducer \
                 arm is producing a message cycle",
            );
            let task = self.update(msg);
            pending.extend(drain_task(task));
        }
    }

    pub(crate) fn drive_pdf_mutation(&mut self, message: Msg) {
        let mut pending: std::collections::VecDeque<Msg> = std::collections::VecDeque::new();
        pending.push_back(message);
        let mut budget = 32usize;
        while let Some(msg) = pending.pop_front() {
            budget = budget.checked_sub(1).expect(
                "drive_pdf_mutation exceeded its message budget: a reducer arm \
                 is producing a message cycle",
            );
            let task = self.update(msg);
            pending.extend(drain_task(task));
        }
    }

    pub(crate) fn test_new_settled() -> Self {
        let (mut app, boot) = Self::test_new();
        for message in drain_task(boot) {
            app.drive_pdf_mutation(message);
        }
        app
    }

    pub(crate) fn open_settled_for_test(&mut self, path: std::path::PathBuf) {
        self.drive_pdf_mutation(Msg::OpenPath(path));
        assert!(
            !self.model.annotations_loading,
            "the annotation restore for the opened document must have landed"
        );
    }
}

#[cfg(test)]
pub(crate) fn drain_task(task: Task<Msg>) -> Vec<Msg> {
    task_observer::observe(task).messages
}

#[cfg(test)]
mod held_owner;
#[cfg(test)]
mod task_observer;

#[cfg(test)]
mod reading_position_tests;

#[cfg(test)]
mod cbz_tests;

#[cfg(test)]
mod modal_key_scope_tests;

#[cfg(test)]
mod annotation_authority_route_tests;
#[cfg(test)]
mod native_pdf_annotation_tests;

#[cfg(test)]
mod persistence_failure_tests;

#[cfg(test)]
mod state_recovery_tests;

#[cfg(test)]
mod document_load_generation_tests;

#[cfg(test)]
mod search_tests;

#[cfg(test)]
mod long_chapter_tests;

#[cfg(test)]
mod pdf_password_tests;

#[cfg(test)]
mod footnote_popover_tests;

#[cfg(test)]
mod page_arranger_tests;

#[cfg(test)]
mod page_arrangement_publication_tests;

#[cfg(test)]
mod retained_authority_tests;

#[cfg(test)]
mod pdf_navigation_tests;

#[cfg(test)]
mod viewer_chrome_tests;

#[cfg(test)]
mod picker_tests;

#[cfg(test)]
mod close_routing_tests;

#[cfg(test)]
mod launch_tests;
