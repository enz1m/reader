//! Second reducer batch: thumbnails, windows, persistence.

use super::*;

use iced::Task;

use crate::message::PageBitmap;

impl App {
    pub(crate) fn batch2_pdf_thumbnails_scrolled(
        &mut self,
        generation: u64,
        offset: f32,
        viewport_height: f32,
    ) -> Task<Msg> {
        if generation != self.model.thumbnail_generation {
            return Task::none();
        }
        self.model.thumbnail_scroll_offset = offset.max(0.0);
        self.model.thumbnail_viewport_height = viewport_height.max(0.0);
        Task::none()
    }

    pub(crate) fn batch2_pdf_thumbnails_rendered(
        &mut self,
        generation: u64,
        requested: Vec<usize>,
        pages: Vec<PageBitmap>,
    ) -> Task<Msg> {
        if generation != self.model.thumbnail_generation || self.model.format != Format::Pdf {
            return Task::none();
        }
        let returned = pages
            .iter()
            .map(|page| page.index)
            .collect::<std::collections::HashSet<_>>();
        for index in requested {
            self.model.thumbnail_rasterising.remove(&index);
            if !returned.contains(&index) {
                self.model.thumbnail_failed.insert(index);
            }
        }
        let protect = crate::raster::thumbnail_band(
            self.model.page_count,
            self.model.thumbnail_scroll_offset,
            self.model.thumbnail_viewport_height.max(240.0),
        )
        .unwrap_or(0..=0);
        for page in pages {
            let rendered = crate::raster::RenderedPage {
                index: page.index,
                handle: iced::widget::image::Handle::from_rgba(page.width, page.height, page.rgba),
                width: page.width,
                height: page.height,
                // Thumbnails are small enough to upload in the frame.
                allocation: None,
            };
            self.model
                .thumbnail_rasters
                .insert(rendered, protect.clone());
        }
        self.pump_busy_retry()
    }

    pub(crate) fn batch2_document_key_command(
        &mut self,
        command: crate::shortcuts::Command,
    ) -> Task<Msg> {
        if !self.document_focused() {
            return Task::none();
        }
        self.model.toolbar_overflow_open = false;
        let inner = crate::shortcuts::command_message(command);
        self.update(inner)
    }

    pub(crate) fn batch2_shell_key_command(
        &mut self,
        command: crate::shortcuts::Command,
    ) -> Task<Msg> {
        let arranger_history = self.model.overlay == Some(model::Overlay::PageArranger)
            && self.model.pending_page_action.is_none()
            && matches!(
                command,
                crate::shortcuts::Command::Undo | crate::shortcuts::Command::Redo
            );
        if self.modal_owns_keyboard() && !arranger_history {
            return Task::none();
        }
        self.model.toolbar_overflow_open = false;
        let inner = crate::shortcuts::command_message(command);
        self.update(inner)
    }

    pub(crate) fn batch2_next_page(&mut self) -> Task<Msg> {
        // On a paginated EPUB the next page moves the anchor, not `current_page`; `PageDown`
        // and the toolbar share this arm.
        if self.model.epub_is_paginated() {
            self.model.epub_page_offset = crate::pagination::next_page_offset(
                &self.model.epub_pages,
                self.model.epub_page_offset,
            );
            self.after_page_turn();
            self.save_position();
            return Task::none();
        }
        // In a spread the next page is the next pair's first.
        match self.adjacent_page_of(self.model.current_page, true) {
            Some(next) => self.update(Msg::GoToPage(next)),
            None => Task::none(),
        }
    }

    pub(crate) fn batch2_prev_page(&mut self) -> Task<Msg> {
        if self.model.epub_is_paginated() {
            self.model.epub_page_offset = crate::pagination::previous_page_offset(
                &self.model.epub_pages,
                self.model.epub_page_offset,
            );
            self.after_page_turn();
            self.save_position();
            return Task::none();
        }
        match self.adjacent_page_of(self.model.current_page, false) {
            Some(previous) => self.update(Msg::GoToPage(previous)),
            None => Task::none(),
        }
    }

    pub(crate) fn batch2_font_size_up(&mut self) -> Task<Msg> {
        self.model.epub_font_size += 2;
        self.model.clamp_font_size();
        // A font change moves page breaks; `repaginate` keeps the anchored passage.
        self.repaginate();
        self.save_position();
        Task::none()
    }

    pub(crate) fn batch2_font_size_down(&mut self) -> Task<Msg> {
        self.model.epub_font_size -= 2;
        self.model.clamp_font_size();
        self.repaginate();
        self.save_position();
        Task::none()
    }

    pub(crate) fn batch2_line_spacing_up(&mut self) -> Task<Msg> {
        self.model.epub_line_spacing = (self.model.epub_line_spacing + 0.1).min(2.5);
        self.repaginate();
        self.save_position();
        Task::none()
    }

    pub(crate) fn batch2_line_spacing_down(&mut self) -> Task<Msg> {
        self.model.epub_line_spacing = (self.model.epub_line_spacing - 0.1).max(1.0);
        self.repaginate();
        self.save_position();
        Task::none()
    }

    pub(crate) fn batch2_set_reading_theme(&mut self, t: ReadingTheme) -> Task<Msg> {
        self.model.reading_theme = t;
        // A theme does not change page metrics, but repaginating keeps one path producing
        // the page slice.
        self.repaginate();
        let was_open = std::mem::replace(&mut self.model.reading_theme_menu_open, false);
        self.save_position();
        if was_open {
            self.restore_reading_theme_trigger()
        } else {
            Task::none()
        }
    }

    fn restore_reading_theme_trigger(&self) -> Task<Msg> {
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::focus(
            crate::view::widgets::reading_theme_trigger_id(),
        ))
    }

    pub(crate) fn batch2_toggle_reading_theme_menu(&mut self) -> Task<Msg> {
        self.model.reading_theme_menu_open = !self.model.reading_theme_menu_open;
        if self.model.reading_theme_menu_open {
            iced::advanced::widget::operate(enzui_iced_widgets::popover::focus_operation(
                crate::view::widgets::reading_theme_scope_id(),
            ))
        } else {
            self.restore_reading_theme_trigger()
        }
    }

    pub(crate) fn batch2_close_reading_theme_menu(&mut self) -> Task<Msg> {
        if std::mem::replace(&mut self.model.reading_theme_menu_open, false) {
            self.restore_reading_theme_trigger()
        } else {
            Task::none()
        }
    }

    pub(crate) fn batch2_footnote_clicked(&mut self, target_id: String) -> Task<Msg> {
        let note = self.model.epub.as_ref().and_then(|epub| {
            epub.load
                .resolve_footnote(self.model.epub_current_chapter, &target_id)
                .cloned()
        });
        match note {
            Some(note) => {
                self.model.active_selection = None;
                self.bump_selection_clear();
                self.model.active_footnote = Some(note);
            }
            None => {
                self.model.active_footnote = None;
                self.model.refuse("This footnote is unavailable.");
            }
        }
        Task::none()
    }

    pub(crate) fn batch2_dismiss_footnote(&mut self) -> Task<Msg> {
        self.model.active_footnote = None;
        Task::none()
    }

    pub(crate) fn batch2_copied_selection(&mut self, text: String) -> Task<Msg> {
        iced::clipboard::write(text).map(Msg::SelectionCopied)
    }

    pub(crate) fn batch2_selection_copied(
        &mut self,
        result: Result<(), iced::clipboard::Error>,
    ) -> Task<Msg> {
        match result {
            Ok(()) => self.model.announce("Copied selection"),
            Err(_) => self
                .model
                .refuse("Couldn’t copy the selection to the clipboard."),
        }
        Task::none()
    }

    pub(crate) fn batch2_selection_changed(
        &mut self,
        sel: Option<enzui_iced_widgets::selection_scope::ScopeSelection>,
    ) -> Task<Msg> {
        // Stored so the view can anchor contextual actions to a live selection.
        self.model.active_selection = sel.filter(|s| !s.text.trim().is_empty());
        Task::none()
    }

    pub(crate) fn batch2_highlight_selection(&mut self) -> Task<Msg> {
        let Some((chapter, char_start, char_end, text)) = self.model.active_epub_selection_anchor()
        else {
            self.model.active_selection = None;
            self.bump_selection_clear();
            return Task::none();
        };
        let id = persistence::add_annotation_full(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            chapter,
            "highlight",
            &text,
            "yellow",
            persistence::AnnotationPosition::EpubCfi {
                chapter,
                char_start,
                char_end,
            },
            "highlight",
            "",
        );
        self.record_add_annotation(id);
        self.model.active_selection = None;
        self.bump_selection_clear();
        self.model.announce("Highlight added");
        self.refresh_epub_annotation_projection(chapter);
        self.save_annotations();
        Task::none()
    }

    pub(crate) fn batch2_add_note_to_selection(&mut self) -> Task<Msg> {
        let Some((chapter, char_start, char_end, _)) = self.model.active_epub_selection_anchor()
        else {
            self.model.active_selection = None;
            self.bump_selection_clear();
            return Task::none();
        };
        self.model.note_editor = Some(enzui_iced_widgets::form_dialog::FormPresenter::new(
            crate::view::note_editor::note_form_model(""),
        ));
        self.model.note_editor_editing = None;
        self.model.note_editor_page = chapter;
        self.model.note_editor_position = Some(persistence::AnnotationPosition::EpubCfi {
            chapter,
            char_start,
            char_end,
        });
        self.model.active_selection = None;
        self.bump_selection_clear();
        crate::overlay_focus::open(&mut self.model, model::Overlay::NoteEditor)
    }

    pub(crate) fn batch2_copy_selection(&mut self) -> Task<Msg> {
        if self.model.pdf_selection.is_some() {
            return self.batch7_copy_pdf_selection();
        }
        let Some(selection) = self.model.active_selection.take() else {
            return Task::none();
        };
        self.bump_selection_clear();
        iced::clipboard::write(selection.text).map(Msg::SelectionCopied)
    }

    pub(crate) fn batch2_dismiss_selection_actions(&mut self) -> Task<Msg> {
        self.model.active_selection = None;
        self.bump_selection_clear();
        Task::none()
    }

    pub(crate) fn batch2_toggle_sidebar(&mut self) -> Task<Msg> {
        // Toggling flips the inline preference or opens the drawer depending on the
        // presentation, resolved by the same `nav_split::resolve_for_width` the widget uses.
        let presentation = self.sidebar_presentation();
        self.model.nav.toggle_for_presentation(presentation);
        Task::none()
    }

    pub(crate) fn batch2_dismiss_sidebar(&mut self) -> Task<Msg> {
        self.model.nav.dismiss_overlay();
        Task::none()
    }

    pub(crate) fn batch2_window_resized(&mut self, size: iced::Size) -> Task<Msg> {
        self.window_width = size.width;
        self.window_height = size.height;
        self.model.pages_viewport_height =
            (size.height - crate::view::widgets::TITLEBAR_HEIGHT).max(1.0);
        // A new page box moves the boundaries; repaginating keeps the anchored passage.
        self.repaginate();
        if self.model.epub_is_paginated() {
            // A paginated surface has no scroll offset to restore, and the `snap_to` target
            // is not in the tree.
            return Task::none();
        }
        if matches!(self.model.format, Format::Pdf | Format::Cbz)
            && self.model.screen == Screen::Reading
            && self.model.current_page > 0
        {
            self.model.pages_scroll_offset = self.page_offset_of(self.model.current_page);
            return self.scroll_to_current_page();
        }
        // `nav_split::resolve_for_width` owns collapse; `overlay_open` survives a width
        // change because `effective_overlay_open` gates it. A reflow invalidates the EPUB
        // scroll offset, so snap back to the relative fraction.
        if self.model.format == model::Format::Epub
            && self.model.screen == model::Screen::Reading
            && self.model.epub.is_some()
        {
            let frac = self.model.epub_scroll_fraction;
            return iced::widget::operation::snap_to(
                crate::view::reading::epub_scroll_id(),
                iced::widget::scrollable::RelativeOffset { x: 0.0, y: frac },
                iced::advanced::widget::operation::Animation::Instant,
            );
        }
        Task::none()
    }
}
