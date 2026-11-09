//! Fourth reducer batch: bookmarks, annotations, focus.

use super::*;

use iced::Task;

use crate::model::SidebarTab;

impl App {
    pub(crate) fn batch4_remove_bookmark(&mut self, page: usize) -> Task<Msg> {
        let existing = self
            .model
            .annotations
            .bookmarks
            .iter()
            .find(|b| b.page == page)
            .cloned();
        let removed = persistence::remove_bookmark(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            page,
        );
        if removed && let Some(record) = existing {
            self.model
                .undo_stack
                .record(UndoableAction::RemoveBookmark { record });
        }
        self.save_annotations();
        Task::none()
    }

    pub(crate) fn batch4_select_sidebar_tab(&mut self, tab: SidebarTab) -> Task<Msg> {
        self.model.sidebar_tab = tab;
        if tab == model::SidebarTab::Thumbnails {
            self.model.thumbnail_scroll_offset =
                self.model.current_page as f32 * crate::raster::THUMBNAIL_ROW_HEIGHT;
        }
        Task::none()
    }

    pub(crate) fn batch4_toggle_annotation_tools(&mut self, expanded: bool) -> Task<Msg> {
        self.model.annotation_tools_expanded = expanded;
        Task::none()
    }

    pub(crate) fn batch4_annotation_filter_changed(&mut self, text: String) -> Task<Msg> {
        self.model.annotation_filter = text;
        Task::none()
    }

    pub(crate) fn batch4_copy_citation(&mut self, id: u64) -> Task<Msg> {
        let Some(ann) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|a| a.id == id)
        else {
            return Task::none();
        };
        let citation = persistence::annotation_citation(ann, &self.model.file_name);
        iced::clipboard::write(citation).map(Msg::CitationCopied)
    }

    pub(crate) fn batch4_citation_copied(
        &mut self,
        result: Result<(), iced::clipboard::Error>,
    ) -> Task<Msg> {
        match result {
            Ok(()) => self.model.announce("Copied citation"),
            Err(_) => self
                .model
                .refuse("Couldn’t copy the citation to the clipboard."),
        }
        Task::none()
    }

    pub(crate) fn batch4_begin_edit_annotation(&mut self, id: u64) -> Task<Msg> {
        // Seed the draft with the annotation's current text so the edit
        // field opens pre-filled. No-op for an unknown id.
        if let Some(a) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|a| a.id == id)
        {
            self.model.editing_annotation = Some((id, a.text.clone()));
        }
        Task::none()
    }

    pub(crate) fn batch4_edit_annotation_changed(&mut self, text: String) -> Task<Msg> {
        if let Some((_, draft)) = self.model.editing_annotation.as_mut() {
            *draft = text;
        }
        Task::none()
    }

    pub(crate) fn batch4_commit_edit_annotation(&mut self) -> Task<Msg> {
        if let Some((id, draft)) = self.model.editing_annotation.take() {
            let text = draft.trim().to_string();
            // A blank edit is treated as "no change" rather than
            // clobbering the annotation with an empty string.
            if !text.is_empty() {
                // Capture the previous text for undo; an unchanged text records nothing.
                let previous = self
                    .model
                    .annotations
                    .annotations
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| a.text.clone());
                persistence::update_annotation_text(
                    std::sync::Arc::make_mut(&mut self.model.annotations),
                    id,
                    &text,
                );
                if let Some(previous) = previous
                    && previous != text
                {
                    self.model
                        .undo_stack
                        .record(UndoableAction::UpdateAnnotationText {
                            id,
                            previous,
                            new: text,
                        });
                }
                self.model.announce("Note updated");
                self.save_annotations();
            }
        }
        Task::none()
    }

    pub(crate) fn batch4_cancel_edit_annotation(&mut self) -> Task<Msg> {
        self.model.editing_annotation = None;
        Task::none()
    }

    pub(crate) fn batch4_copy_annotations_json(&mut self) -> Task<Msg> {
        let json = persistence::annotations_to_json(&self.model.annotations);
        iced::clipboard::write(json).map(Msg::AnnotationsJsonCopied)
    }

    pub(crate) fn batch4_annotations_json_copied(
        &mut self,
        result: Result<(), iced::clipboard::Error>,
    ) -> Task<Msg> {
        match result {
            Ok(()) => self.model.announce("Copied JSON"),
            Err(_) => self.model.refuse("Couldn’t copy JSON to the clipboard."),
        }
        Task::none()
    }

    pub(crate) fn batch4_copy_annotations_markdown(&mut self) -> Task<Msg> {
        let markdown =
            persistence::annotations_to_markdown(&self.model.annotations, &self.model.file_name);
        iced::clipboard::write(markdown).map(Msg::AnnotationsMarkdownCopied)
    }

    pub(crate) fn batch4_annotations_markdown_copied(
        &mut self,
        result: Result<(), iced::clipboard::Error>,
    ) -> Task<Msg> {
        match result {
            Ok(()) => self.model.announce("Copied Markdown"),
            Err(_) => self
                .model
                .refuse("Couldn’t copy Markdown to the clipboard."),
        }
        Task::none()
    }

    pub(crate) fn batch4_open_search(&mut self) -> Task<Msg> {
        self.model.sidebar_tab = model::SidebarTab::Search;
        // At mobile width the sidebar is an overlay, so the tab alone would be invisible.
        self.ensure_sidebar_visible();
        iced::widget::operation::focus(crate::view::search_panel::find_field_id())
    }

    pub(crate) fn batch4_search_query_changed(&mut self, q: String) -> Task<Msg> {
        // Update the query immediately and leave scanning to the debounce tick;
        // `begin_search` returns false for an unsearchable query, dropping the highlight.
        if !self.model.begin_search(q) {
            self.refresh_search_highlight();
        }
        Task::none()
    }

    pub(crate) fn batch4_search_tick(&mut self) -> Task<Msg> {
        if !self.model.search_pending || self.close_state.pauses_expendable_work() {
            return Task::none();
        }
        self.model.search_pending = false;
        let generation = self.model.search_generation;
        self.model.search_in_flight_generation = Some(generation);
        let query = self.model.search_query.clone();
        match self.model.format {
            Format::Epub => {
                let index = self.model.search_index.clone();
                self.dispatch_blocking(move || index.run(&query))
                    .map(move |result| match result {
                        Ok(hits) => Msg::SearchCompleted { generation, hits },
                        Err(BlockingTaskError::Busy) => Msg::SearchRefusedBusy {
                            generation,
                            format: Format::Epub,
                        },
                        Err(_terminal) => Msg::SearchCompleted {
                            generation,
                            hits: Vec::new(),
                        },
                    })
            }
            Format::Pdf => {
                let index = self.model.pdf_search_index.clone();
                self.dispatch_blocking(move || index.run(&query))
                    .map(move |result| match result {
                        Ok(hits) => Msg::PdfSearchCompleted { generation, hits },
                        Err(BlockingTaskError::Busy) => Msg::SearchRefusedBusy {
                            generation,
                            format: Format::Pdf,
                        },
                        Err(_terminal) => Msg::PdfSearchCompleted {
                            generation,
                            hits: Vec::new(),
                        },
                    })
            }
            Format::Cbz => Task::none(),
        }
    }

    pub(crate) fn batch4_search_completed(
        &mut self,
        generation: u64,
        hits: Vec<crate::plumbing::epub_support::EpubHit>,
    ) -> Task<Msg> {
        // Discard a superseded scan so a slow earlier scan cannot land on a later query.
        if generation != self.model.search_generation || self.model.format != Format::Epub {
            return Task::none();
        }
        self.model.search_in_flight_generation = None;
        self.model.epub_search_truncated =
            hits.len() >= crate::plumbing::epub_support::MAX_EPUB_SEARCH_HITS;
        self.model.apply_search_hits(hits);
        self.refresh_search_highlight();
        Task::batch([self.navigate_to_active_hit(), self.pump_busy_retry()])
    }

    pub(crate) fn batch4_pdf_search_completed(
        &mut self,
        generation: u64,
        hits: Vec<crate::search::PdfSearchHit>,
    ) -> Task<Msg> {
        if generation != self.model.search_generation || self.model.format != Format::Pdf {
            return Task::none();
        }
        self.model.search_in_flight_generation = None;
        self.model.pdf_search_truncated |= hits.len() == crate::search::MAX_PDF_SEARCH_HITS;
        self.model.apply_pdf_search_hits(hits);
        Task::batch([self.navigate_to_active_hit(), self.pump_busy_retry()])
    }

    pub(crate) fn batch4_search_result_activated(&mut self, index: usize) -> Task<Msg> {
        // A result row was clicked / activated by keyboard: select it,
        // jump to its passage, and move the transient highlight there.
        if index < self.model.search_hit_count() {
            self.model.search_active = index;
            self.refresh_search_highlight();
            return self.navigate_to_active_hit();
        }
        Task::none()
    }

    pub(crate) fn batch4_next_match(&mut self) -> Task<Msg> {
        self.model.step_search(true);
        self.refresh_search_highlight();
        self.navigate_to_active_hit()
    }

    pub(crate) fn batch4_prev_match(&mut self) -> Task<Msg> {
        self.model.step_search(false);
        self.refresh_search_highlight();
        self.navigate_to_active_hit()
    }

    pub(crate) fn batch4_focus_next(&mut self) -> Task<Msg> {
        operate(traverse::<Msg>(Traversal::Next))
    }
}
