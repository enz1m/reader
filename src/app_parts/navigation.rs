//! Picker outcomes, navigation, and EPUB projection.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn forwarded_launch(&mut self) -> Task<Msg> {
        let Some(instance) = self.instance.as_mut() else {
            return Task::none();
        };
        let (intents, raise) = instance.drain(self.main_window);
        let mut tasks = vec![raise];
        for intent in intents {
            tasks.push(self.receive_launch(intent));
        }
        Task::batch(tasks)
    }

    pub(crate) fn receive_launch(
        &mut self,
        intent: enzui_iced_core::activation::ApplicationIntent,
    ) -> Task<Msg> {
        match services::document_from_launch(intent) {
            Ok(Some(path)) => {
                if !matches!(self.close_state, CloseState::Idle)
                    || self.model.overlay.is_some()
                    || self.model.pending_page_action.is_some()
                    || self.pdf_mutation_active.is_some()
                    || self.pdf_mutation_uncertain.is_some()
                    || self.model.signing.is_some()
                    || self.model.arrangement_publication.is_some()
                {
                    self.model.picker_failure = Some(
                        "Finish the current operation before opening another document.".into(),
                    );
                } else if self.model.library_loading {
                    self.pending_launch_document = Some(path);
                } else {
                    return self.open(path);
                }
            }
            Ok(None) => {}
            Err(error) => self.model.picker_failure = Some(error),
        }
        Task::none()
    }

    // Chooser cancellation is silent; non-local success and failures produce actionable
    // errors.
    pub(crate) fn open_pick_unparented(
        &mut self,
        target: crate::picker::PickerTarget,
    ) -> Task<Msg> {
        if let Err(outcome) = self.picker.start(target) {
            return self.finish_pick(target, outcome);
        }
        Task::none()
    }

    pub(crate) fn begin_parented_pick(
        &mut self,
        target: crate::picker::PickerTarget,
        window: iced::window::Id,
    ) -> Task<Msg> {
        if let Some(prev) = self.pending_pick.take() {
            prev.cancel
                .cancel(enzui_iced_core::portal::CancelReason::Application);
        }
        let generation = self.next_pick_generation;
        self.next_pick_generation = self.next_pick_generation.wrapping_add(1);
        let cancel = enzui_iced_core::portal::cancel::PortalCancel::new();
        self.pending_pick = Some(PendingPick { generation, cancel });
        iced::window::wayland_parent_export(window).map(move |payload| Msg::PickerParentResolved {
            generation,
            target,
            window,
            payload: enzui_iced_parent_lease::Handoff::new(payload),
        })
    }

    pub(crate) fn reduce_pick_parent_resolved(
        &mut self,
        generation: u64,
        target: crate::picker::PickerTarget,
        window: iced::window::Id,
        payload: &enzui_iced_parent_lease::Handoff<Option<Box<dyn std::any::Any + Send>>>,
    ) -> Task<Msg> {
        if self.pending_pick.as_ref().map(|p| p.generation) != Some(generation) {
            return Task::none();
        }
        if self.main_window != Some(window) {
            self.pending_pick = None;
            return Task::none();
        }
        let cancel = self
            .pending_pick
            .as_ref()
            .expect("generation matched a live pending pick")
            .cancel
            .clone();
        let Some(owned) = payload
            .take()
            .and_then(enzui_iced_wayland::owned_window_from_payload)
        else {
            self.pending_pick = None;
            return self.open_pick_unparented(target);
        };
        // The Wayland round trip blocks on compositor I/O, so it runs on the bounded owner.
        self.dispatch_blocking(move || {
            let incarnation = owned.generation();
            enzui_iced_wayland::ParentExport::open(
                &owned,
                incarnation,
                &cancel,
                std::time::Duration::from_secs(5),
            )
        })
        .map(move |result| Msg::PickResolved {
            generation,
            target,
            window,
            outcome: export_result_to_outcome(result),
        })
    }

    pub(crate) fn reduce_pick_resolved(
        &mut self,
        generation: u64,
        target: crate::picker::PickerTarget,
        window: iced::window::Id,
        outcome: crate::message::PickOutcome,
    ) -> Task<Msg> {
        if self.pending_pick.as_ref().map(|p| p.generation) != Some(generation) {
            return Task::none();
        }
        self.pending_pick = None;
        match outcome {
            crate::message::PickOutcome::Aborted => Task::none(),
            crate::message::PickOutcome::Unparented => self.open_pick_unparented(target),
            crate::message::PickOutcome::Parented(export) => {
                let Some(export) = export.take() else {
                    return Task::none();
                };
                let live = self.main_window;
                match self
                    .picker
                    .start_parented(target, window, export, move |w| live == Some(w))
                {
                    Ok(()) => {
                        if let Some((t, outcome)) = self.picker.pump() {
                            return self.finish_pick(t, outcome);
                        }
                        Task::none()
                    }
                    Err(outcome) => self.finish_pick(target, outcome),
                }
            }
        }
    }

    pub(crate) fn finish_pick(
        &mut self,
        target: crate::picker::PickerTarget,
        outcome: enzui_iced_widgets::file_chooser::PortalOutcome<
            enzui_iced_widgets::file_chooser::FileResult,
        >,
    ) -> Task<Msg> {
        use enzui_iced_widgets::file_chooser::PortalOutcome;
        if target == crate::picker::PickerTarget::SaveArrangement {
            let requested_source = self.save_arrangement_pick_source.take();
            if requested_source.as_ref() != self.model.file_path.as_ref() {
                return Task::none();
            }
        }
        match &outcome {
            PortalOutcome::Completed(result) => match crate::picker::first_local_file(result) {
                Some(path) => {
                    return match target {
                        crate::picker::PickerTarget::OpenBook => self.open(path),
                        // The chosen file goes through the same two steps a
                        // typed path does.
                        crate::picker::PickerTarget::SigningCertificate => {
                            if let Some(draft) = self.model.signing.as_mut() {
                                draft.certificate = Some(path);
                                draft.error = None;
                            }
                            Task::none()
                        }
                        crate::picker::PickerTarget::SaveSigned => self.write_signed_copy(path),
                        crate::picker::PickerTarget::MergePdf => {
                            Task::done(Msg::MergePathChanged(path.to_string_lossy().into_owned()))
                                .chain(Task::done(Msg::MergePdf))
                        }
                        crate::picker::PickerTarget::SaveArrangement => {
                            self.begin_arrangement_publication(path)
                        }
                    };
                }
                None => {
                    self.model.error = None;
                    self.model.picker_failure = Some(match target {
                    crate::picker::PickerTarget::OpenBook => {
                        "That location is not a local file — the reader can only open a file on this machine."
                    }
                    crate::picker::PickerTarget::SaveArrangement => {
                        "That destination is not a local file, so the arranged PDF was not saved."
                    }
                    crate::picker::PickerTarget::MergePdf => {
                        "That PDF is not a local file, so its pages were not added."
                    }
                    crate::picker::PickerTarget::SigningCertificate => {
                        "That certificate is not a local file."
                    }
                    crate::picker::PickerTarget::SaveSigned => {
                        "That destination is not a local file, so the signed PDF was not saved."
                    }
                }
                .to_string());
                }
            },
            // A user closing the dialog is a decision, not an error — no-op.
            PortalOutcome::Cancelled(_) => {}
            // `failure_text` is total over the outcome enum, so a new variant is a compile
            // error rather than a generic banner.
            other => {
                if let Some(text) = crate::picker::failure_text_for(target, other) {
                    self.model.error = None;
                    self.model.picker_failure = Some(text);
                }
            }
        }
        Task::none()
    }

    pub(crate) fn navigate_to_active_hit(&mut self) -> Task<Msg> {
        match self.model.format {
            Format::Epub => {
                let Some(chapter) = self.model.active_hit().map(|hit| hit.chapter) else {
                    return Task::none();
                };
                if chapter < self.model.epub_chapter_count() {
                    self.enter_chapter(chapter);
                }
                Task::none()
            }
            Format::Pdf => {
                let Some(page) = self.model.active_pdf_search_hit().map(|hit| hit.page) else {
                    return Task::none();
                };
                self.model.pdf_selection = None;
                self.update(Msg::GoToPage(page))
            }
            Format::Cbz => Task::none(),
        }
    }

    pub(crate) fn current_epub_progress_anchor(&self) -> Option<persistence::EpubProgressAnchor> {
        if self.model.format != Format::Epub {
            return None;
        }
        let chapter = self.model.epub_current_chapter;
        let paragraphs = self.model.epub.as_ref()?.chapter_paragraphs.get(&chapter)?;
        if paragraphs.is_empty() {
            return Some(persistence::EpubProgressAnchor::default());
        }
        let absolute = if self.model.epub_view_mode == EpubViewMode::Paginated {
            self.model.epub_page_offset
        } else {
            self.offset_for_scroll_fraction()
        };
        let paragraph = paragraphs.iter().min_by_key(|paragraph| {
            if absolute < paragraph.char_start {
                paragraph.char_start - absolute
            } else if absolute >= paragraph.char_end {
                absolute - paragraph.char_end + 1
            } else {
                0
            }
        })?;
        let max_offset = paragraph.text.chars().count().saturating_sub(1);
        Some(persistence::EpubProgressAnchor {
            paragraph: paragraph.para_index,
            char_offset: absolute
                .saturating_sub(paragraph.char_start)
                .min(max_offset),
        })
    }

    pub(crate) fn restore_epub_position(&mut self, record: &ProgressRecord) -> bool {
        let chapter_count = self.model.epub_chapter_count();
        if chapter_count == 0 {
            self.model.epub_current_chapter = 0;
            self.model.epub_page_offset = 0;
            self.model.epub_scroll_fraction = 0.0;
            return record.page != 0
                || record
                    .epub_anchor
                    .as_ref()
                    .is_some_and(|anchor| anchor.paragraph != 0 || anchor.char_offset != 0);
        }

        let chapter = record.page.min(chapter_count - 1);
        let mut moved = chapter != record.page;
        self.enter_chapter(chapter);
        let Some(anchor) = &record.epub_anchor else {
            return moved;
        };
        let paragraphs = self
            .model
            .epub
            .as_ref()
            .and_then(|epub| epub.chapter_paragraphs.get(&chapter));
        let Some(paragraph) = paragraphs.and_then(|paragraphs| {
            paragraphs
                .iter()
                .min_by_key(|paragraph| paragraph.para_index.abs_diff(anchor.paragraph))
        }) else {
            self.model.epub_page_offset = 0;
            self.model.epub_scroll_fraction = 0.0;
            return moved || anchor.paragraph != 0 || anchor.char_offset != 0;
        };
        moved |= paragraph.para_index != anchor.paragraph;
        let max_offset = paragraph.text.chars().count().saturating_sub(1);
        let char_offset = anchor.char_offset.min(max_offset);
        moved |= char_offset != anchor.char_offset;
        self.model.epub_page_offset = paragraph.char_start.saturating_add(char_offset);
        self.model.epub_scroll_fraction = self.scroll_fraction_for_offset();
        self.repaginate();
        moved
    }

    // Reset chapter-local anchors on chapter changes so offsets from a longer chapter cannot
    // open the last page.
    pub(crate) fn enter_chapter(&mut self, chapter: usize) {
        self.model.epub_current_chapter = chapter;
        self.ensure_chapter_paragraphs();
        self.model.epub_page_offset = 0;
        self.repaginate();
    }

    pub(crate) fn reading_content_width(&self) -> f32 {
        let (w, _) = self.reading_window_size();
        crate::view::shell::content_width_for(&self.model, w)
    }

    pub(crate) fn epub_column_width(&self) -> f32 {
        crate::reading_pane::column_width_for(self.reading_content_width())
    }

    // Shape the current long chapter's geometry off the UI thread when
    // it is missing for the current metrics; the view windows over an
    // estimate until it lands.
    pub(crate) fn ensure_epub_geometry(&mut self) -> Task<Msg> {
        let Some(key) = self.model.epub_geometry_key(self.epub_column_width()) else {
            return Task::none();
        };
        if self
            .model
            .epub_geometry
            .as_ref()
            .is_some_and(|g| g.key() == key)
            || self.model.epub_geometry_pending == Some(key)
        {
            return Task::none();
        }
        let Some(texts) = self.model.epub.as_ref().and_then(|epub| {
            epub.chapter_paragraphs.get(&key.chapter).map(|paras| {
                paras
                    .iter()
                    .map(|para| para.text.clone())
                    .collect::<Vec<_>>()
            })
        }) else {
            return Task::none();
        };
        self.model.epub_geometry_pending = Some(key);
        let (font_size, spacing) = (
            self.model.epub_font_size as f32,
            self.model.epub_line_spacing,
        );
        let column_width = f32::from_bits(key.column_width_bits);
        self.dispatch_blocking(move || {
            crate::reading_pane::ChapterGeometry::measure(
                key,
                texts.iter().map(String::as_str),
                font_size,
                spacing,
                column_width,
            )
        })
        .map(move |result| Msg::EpubGeometryMeasured(key, result.ok().map(std::sync::Arc::new)))
    }

    fn reading_window_size(&self) -> (f32, f32) {
        if self.window_width > 0.0 && self.window_height > 0.0 {
            (self.window_width, self.window_height)
        } else {
            crate::pagination::DEFAULT_WINDOW
        }
    }

    pub(crate) fn pagination_metrics(&self) -> crate::pagination::PageMetrics {
        let (w, h) = self.reading_window_size();
        let content_width = crate::view::shell::content_width_for(&self.model, w);
        let column_width = crate::reading_pane::column_width_for(content_width);
        let font_size = self.model.epub_font_size as f32;
        let line_spacing = self.model.epub_line_spacing;
        let spacing = self.themes.theme().spacing;
        let metrics = match self.model.epub_page_box {
            Some(page_box) => crate::pagination::PageMetrics::new(
                column_width,
                page_box
                    - crate::view::reading::page_top_inset(&spacing)
                    - crate::reading_pane::COLUMN_PADDING,
                font_size,
                line_spacing,
            ),
            None => {
                crate::pagination::PageMetrics::for_window(column_width, h, font_size, line_spacing)
            }
        };
        let heading = self
            .model
            .epub
            .as_ref()
            .and_then(|epub| epub.load.chapters.get(self.model.epub_current_chapter))
            .map(|chapter| chapter.display_heading().to_string())
            .unwrap_or_default();
        let heading_height = enzui_iced_widgets::text_measure::enz_text_measure_with(
            &heading,
            column_width,
            enzui_iced_widgets::text_measure::MeasureTypography::new(
                enzui_iced_widgets::enz_text::TextSize::Title2,
            ),
        )
        .measured_height;
        metrics.with_heading_height(heading_height + spacing.md())
    }

    // Page turns select cached spans; metric changes reflow from the character anchor, and
    // scrolled mode clears pagination.
    pub(crate) fn repaginate(&mut self) {
        if self.model.format != Format::Epub || self.model.epub_view_mode != EpubViewMode::Paginated
        {
            self.model.epub_pages.clear();
            self.model.epub_page_content.clear();
            return;
        }
        let metrics = self.pagination_metrics();
        let chapter = self.model.epub_current_chapter;
        let offset = self.model.epub_page_offset;
        let computed = self
            .model
            .epub
            .as_ref()
            .and_then(|e| e.chapter_paragraphs.get(&chapter))
            .map(|paras| {
                // The cut forces a page boundary at the reading position, so a reflow puts
                // the current passage at the top of a page.
                let pages = crate::pagination::paginate_anchored(paras, &metrics, offset);
                let index = crate::pagination::page_for_offset(&pages, offset);
                let span = pages.get(index).copied();
                let content = span
                    .map(|s| crate::pagination::slice_paragraphs(paras, &s))
                    .unwrap_or_default();
                (pages, span, content)
            });
        match computed {
            Some((pages, span, content)) => {
                // Canonicalise a stale anchor into the painted page, so the model never
                // reports an off-screen position.
                if let Some(span) = span.filter(|s| !s.contains(offset)) {
                    self.model.epub_page_offset = span.char_start;
                }
                self.model.epub_pages = pages;
                self.model.epub_page_content = content;
            }
            None => {
                self.model.epub_pages.clear();
                self.model.epub_page_content.clear();
            }
        }
    }

    // After a page turn: slice the new span from the cached pagination without
    // shaping, and keep the continuous-mode scroll fraction in step.
    pub(crate) fn after_page_turn(&mut self) {
        let chapter = self.model.epub_current_chapter;
        let offset = self.model.epub_page_offset;
        let span = self
            .model
            .epub_pages
            .get(crate::pagination::page_for_offset(
                &self.model.epub_pages,
                offset,
            ))
            .copied();
        self.model.epub_page_content = self
            .model
            .epub
            .as_ref()
            .and_then(|epub| epub.chapter_paragraphs.get(&chapter))
            .and_then(|paras| span.map(|span| crate::pagination::slice_paragraphs(paras, &span)))
            .unwrap_or_default();
        self.model.epub_scroll_fraction = self.scroll_fraction_for_offset();
    }

    pub(crate) fn chapter_char_len(&self) -> usize {
        self.model
            .epub
            .as_ref()
            .and_then(|e| e.chapter_paragraphs.get(&self.model.epub_current_chapter))
            .and_then(|paras| paras.last().map(|p| p.char_end))
            .unwrap_or(0)
    }

    // The char offset the continuous-mode scroll position corresponds to, used on
    // entering paginated mode.
    pub(crate) fn offset_for_scroll_fraction(&self) -> usize {
        let len = self.chapter_char_len();
        if len == 0 {
            return 0;
        }
        let frac = self.model.epub_scroll_fraction.clamp(0.0, 1.0);
        ((frac * len as f32).round() as usize).min(len.saturating_sub(1))
    }

    // The continuous-mode scroll fraction the current anchor corresponds to, used on
    // leaving paginated mode.
    pub(crate) fn scroll_fraction_for_offset(&self) -> f32 {
        let len = self.chapter_char_len();
        if len == 0 {
            return 0.0;
        }
        (self.model.epub_page_offset as f32 / len as f32).clamp(0.0, 1.0)
    }

    // Persisted and transient search highlights share projection; moving search clears only
    // its transient layer.
    pub(crate) fn ensure_chapter_paragraphs(&mut self) {
        let chapter = self.model.epub_current_chapter;
        let Some(epub) = self.model.epub.as_mut() else {
            return;
        };
        // Only derived projections are evicted; source text and persisted anchors stay intact.
        epub.chapter_paragraphs
            .retain(|index, _| index.abs_diff(chapter) <= 1);
        if epub
            .highlighted_chapter
            .is_some_and(|index| !epub.chapter_paragraphs.contains_key(&index))
        {
            epub.highlighted_chapter = None;
        }
        if epub.chapter_paragraphs.contains_key(&chapter) {
            return;
        }
        // Persisted highlights first; the transient search paint is appended below.
        let mut highlights = self.model.persisted_epub_highlights(chapter);
        let search_highlight = self.model.transient_search_highlight(chapter);
        // `highlighted_chapter` tracks only the search layer: with annotations also in
        // `highlights`, a non-empty list no longer means "search paint".
        let carries_search_highlight = search_highlight.is_some();
        highlights.extend(search_highlight);
        let Some(epub) = self.model.epub.as_mut() else {
            return;
        };
        let Some(ch) = epub.load.chapters.iter().find(|c| c.index == chapter) else {
            return;
        };
        // Footnote refs render as clickable underlined spans.
        let located =
            epub_support::locate_footnote_refs(&ch.plain_text, chapter, &epub.load.footnote_refs);
        let paras: Vec<ReadingParagraph> =
            epub_support::build_chapter_reading_paragraphs(ch, &highlights, &located);
        epub.chapter_paragraphs.insert(chapter, paras);
        if carries_search_highlight {
            epub.highlighted_chapter = Some(chapter);
        } else if epub.highlighted_chapter == Some(chapter) {
            // Rebuilt without the transient layer, so a later clear must not target it.
            epub.highlighted_chapter = None;
        }
    }

    // Only consumed selections and retiring navigation advance clear_generation; ordinary
    // selection gestures retain it.
    pub(crate) fn bump_selection_clear(&mut self) {
        self.model.selection_clear_generation =
            self.model.selection_clear_generation.wrapping_add(1);
    }

    // Rebuild one chapter after its persisted annotation ranges change.
    //
    // Cached `ReadingParagraph` spans must be invalidated when a highlight is saved.
    // Paginated content is re-cut from the same character anchor.
    pub(crate) fn refresh_epub_annotation_projection(&mut self, chapter: usize) {
        if let Some(epub) = self.model.epub.as_mut() {
            epub.chapter_paragraphs.remove(&chapter);
        }
        if chapter == self.model.epub_current_chapter {
            self.ensure_chapter_paragraphs();
            self.repaginate();
        }
    }

    // Invalidate every cached EPUB projection after undo/redo, which can touch an
    // off-screen chapter; only the current chapter is rebuilt immediately.
    pub(crate) fn refresh_all_epub_annotation_projections(&mut self) {
        let is_epub = self.model.epub.is_some();
        if let Some(epub) = self.model.epub.as_mut() {
            epub.chapter_paragraphs.clear();
            epub.highlighted_chapter = None;
        }
        if is_epub {
            self.ensure_chapter_paragraphs();
            self.repaginate();
        }
    }

    // Rebuild cached chapters for the current transient search highlight, which is baked
    // into the paragraphs: drop it from the old chapter and rebuild the new one. Clearing
    // the search removes it everywhere.
    pub(crate) fn refresh_search_highlight(&mut self) {
        let new_chapter = self.model.active_hit().map(|h| h.chapter);
        if let Some(epub) = self.model.epub.as_mut() {
            if let Some(old) = epub.highlighted_chapter.take() {
                epub.chapter_paragraphs.remove(&old);
            }
            if let Some(new) = new_chapter {
                epub.chapter_paragraphs.remove(&new);
            }
        }
        self.ensure_chapter_paragraphs();
        // The paginated page slices the rebuilt paragraphs, so re-cut from the same anchor.
        self.repaginate();
    }
}
