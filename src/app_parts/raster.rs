//! Page scroll, rasters, thumbnails, and EPUB finish.

use super::*;

use iced::Task;

impl App {
    pub(crate) fn apply_pages_scrolled(
        &mut self,
        generation: u64,
        offset: f32,
        viewport_height: f32,
        viewport_width: f32,
    ) -> Task<Msg> {
        if generation != self.model.position_restore_generation
            || !matches!(self.model.format, Format::Pdf | Format::Cbz)
            || !offset.is_finite()
            || !viewport_height.is_finite()
            || !viewport_width.is_finite()
            || viewport_width < 0.0
            || self.model.page_extents.is_empty()
        {
            return Task::none();
        }
        self.model.pages_viewport_width = viewport_width;
        let offset = offset.max(0.0);
        let viewport_height = viewport_height.max(0.0);
        if let Some(model::PendingPositionRestore::Pages {
            generation: pending_generation,
            page,
            expected_offset: pending_offset,
            operation_issued,
            mut bootstrap_events_remaining,
        }) = self.model.pending_position_restore
        {
            if pending_generation != generation {
                return Task::none();
            }
            let expected_offset = self.page_offset_of(page);
            let observed_page = self.page_at_offset(offset, viewport_height);
            if offset <= 1.0 && bootstrap_events_remaining > 0 {
                bootstrap_events_remaining -= 1;
                self.model.pages_viewport_height = viewport_height;
                self.model.pending_position_restore = Some(model::PendingPositionRestore::Pages {
                    generation,
                    page,
                    expected_offset,
                    operation_issued,
                    bootstrap_events_remaining,
                });
                return Task::none();
            }
            if (offset - pending_offset).abs() <= 1.0 || observed_page == page {
                self.model.pending_position_restore = None;
                self.model.pages_scroll_offset = offset;
                self.model.pages_viewport_height = viewport_height;
                self.model.current_page = page;
                self.save_position();
                return Task::none();
            }
            self.cancel_position_restore();
        }

        self.model.pages_scroll_offset = offset;
        self.model.pages_viewport_height = viewport_height;
        let page = self.page_at_offset(offset, viewport_height);
        if page != self.model.current_page {
            self.model.current_page = page;
            self.save_position();
        }
        Task::none()
    }

    pub(crate) fn apply_epub_scrolled(&mut self, generation: u64, fraction: f32) -> Task<Msg> {
        if generation != self.model.position_restore_generation || self.model.format != Format::Epub
        {
            return Task::none();
        }
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if let Some(model::PendingPositionRestore::Epub {
            generation: pending_generation,
            chapter,
            anchor,
            expected_fraction,
            operation_issued,
            mut bootstrap_events_remaining,
        }) = self.model.pending_position_restore
        {
            if pending_generation != generation {
                return Task::none();
            }
            if fraction <= 0.001 && bootstrap_events_remaining > 0 {
                bootstrap_events_remaining -= 1;
                self.model.pending_position_restore = Some(model::PendingPositionRestore::Epub {
                    generation,
                    chapter,
                    anchor,
                    expected_fraction,
                    operation_issued,
                    bootstrap_events_remaining,
                });
                return Task::none();
            }
            if (fraction - expected_fraction).abs() <= 0.001 {
                self.model.pending_position_restore = None;
                self.model.epub_scroll_fraction = fraction;
                self.save_position();
                return Task::none();
            }
            self.cancel_position_restore();
        }
        self.model.epub_scroll_fraction = fraction;
        self.save_position();
        Task::none()
    }

    pub(crate) fn finish_pages(
        &mut self,
        path: PathBuf,
        pages: Vec<message::PageBitmap>,
        total_pages: usize,
        extents: Vec<crate::raster::PageExtent>,
        text: Vec<crate::pdf::PdfPageText>,
    ) -> Task<Msg> {
        self.model.format = Format::Pdf;
        self.model.cbz_entries = std::sync::Arc::new(Vec::new());
        self.finish_page_document(path, pages, total_pages, extents, text, "pdf")
    }

    pub(crate) fn finish_cbz(
        &mut self,
        path: PathBuf,
        load: plumbing::cbz_support::CbzLoadResult,
    ) -> Task<Msg> {
        let total_pages = load.entries.len();
        let extents = load.entries.iter().map(|entry| entry.extent).collect();
        self.model.format = Format::Cbz;
        if self.model.sidebar_tab == model::SidebarTab::Thumbnails {
            self.model.sidebar_tab = model::SidebarTab::Bookmarks;
        }
        self.model.cbz_entries = std::sync::Arc::new(load.entries);
        self.finish_page_document(path, load.pages, total_pages, extents, Vec::new(), "cbz")
    }

    pub(crate) fn finish_page_document(
        &mut self,
        path: PathBuf,
        pages: Vec<message::PageBitmap>,
        total_pages: usize,
        extents: Vec<crate::raster::PageExtent>,
        text: Vec<crate::pdf::PdfPageText>,
        format_id: &str,
    ) -> Task<Msg> {
        // A fresh document starts with no selection, so the previous book's rectangle can
        // never arm this book's writer.
        self.model.pdf_selection = None;
        self.model.pdf_annotation_snapshot = None;
        self.model.pdf_annotation_states.clear();
        self.model.pdf_overlay_annotations.clear();
        self.model.selected_annotation = None;
        self.model.pending_pdf_comment = None;
        // The length is `total_pages`, not the rasters held.
        self.model.page_count = total_pages.max(extents.len());
        self.model.total_page_count = self.model.page_count;
        // One text slot per page, addressable by source index as pages arrive.
        self.model.pdf_text = vec![crate::pdf::PdfPageText::default(); self.model.page_count];
        self.model.page_extents = extents;
        self.model.page_extents.resize(
            self.model.page_count,
            crate::raster::PageExtent {
                width: 1000,
                height: 1294,
            },
        );
        self.model.pages.clear();
        self.model.rasterising.clear();
        self.model.raster_failed.clear();
        self.model.pages_scroll_offset = 0.0;
        self.model.pages_viewport_height = 0.0;
        self.model.pages_viewport_width = 0.0;
        let allocations = self.absorb_rasters(pages, text, 0..=0);
        let key = persistence::canonical_key(&path.to_string_lossy());
        let saved_page = self
            .model
            .app_state
            .files
            .get(&key)
            .filter(|record| {
                record.format.is_empty() || record.format.eq_ignore_ascii_case(format_id)
            })
            .map(|record| record.page);
        self.model.current_page = saved_page
            .map(|page| page.min(self.model.page_count.saturating_sub(1)))
            .unwrap_or(0);
        let recovered = saved_page.is_some_and(|page| page != self.model.current_page);
        if self.model.sidebar_tab == model::SidebarTab::Thumbnails {
            self.model.thumbnail_scroll_offset =
                self.model.current_page as f32 * crate::raster::THUMBNAIL_ROW_HEIGHT;
        }
        self.model.pages_scroll_offset = self.page_offset_of(self.model.current_page);
        self.model.pending_position_restore = saved_page
            .filter(|_| self.model.current_page > 0)
            .map(|_| model::PendingPositionRestore::Pages {
                generation: self.model.position_restore_generation,
                page: self.model.current_page,
                expected_offset: self.model.pages_scroll_offset,
                operation_issued: false,
                bootstrap_events_remaining: 1,
            });
        self.model.screen = Screen::Reading;

        // Undo history is per-document. Startup-open completion reaches here without
        // `reset_document`, so clear it again.
        self.model.undo_stack.clear();
        // Rebuild the arranger from the opened document's real length (`page_tree_len`);
        // CBZ stays read-only through `page_editing_supported`.
        self.model.arrangement =
            crate::pages::PageArrangement::from_page_count(self.page_tree_len());
        drop(crate::overlay_focus::close_for_host(&mut self.model));
        self.model.pending_page_action = None;
        if recovered {
            self.model
                .refuse("Reading position adjusted because the document changed.");
        }

        if self.model.pending_position_restore.is_none() {
            let title = self.model.file_name.clone();
            self.persist_progress(&path, title, String::new(), format_id);
        }
        allocations
    }

    pub(crate) fn scroll_to_current_page(&self) -> Task<Msg> {
        iced::widget::operation::scroll_to(
            crate::view::reading::pages_scroll_id(),
            iced::widget::scrollable::AbsoluteOffset {
                x: 0.0,
                y: self.model.pages_scroll_offset,
            },
            iced::advanced::widget::operation::Animation::Instant,
        )
    }

    pub(crate) fn scroll_to_epub_position(&self) -> Task<Msg> {
        if self.model.epub_view_mode != EpubViewMode::Scrolled
            || self.model.epub_scroll_fraction <= 0.0
        {
            return Task::none();
        }
        iced::widget::operation::snap_to(
            crate::view::reading::epub_scroll_id(),
            iced::widget::scrollable::RelativeOffset {
                x: 0.0,
                y: self.model.epub_scroll_fraction,
            },
            iced::advanced::widget::operation::Animation::Instant,
        )
    }

    // Admit bitmaps within the cache budget and retain their native GPU allocations.
    pub(crate) fn absorb_rasters(
        &mut self,
        pages: Vec<message::PageBitmap>,
        text: Vec<crate::pdf::PdfPageText>,
        protect: std::ops::RangeInclusive<usize>,
    ) -> Task<Msg> {
        let mut swaps = Vec::new();
        for (position, bitmap) in pages.into_iter().enumerate() {
            let index = bitmap.index;
            if index >= self.model.page_count {
                continue;
            }
            if let Some(page_text) = text.get(position)
                && let Some(slot) = self.model.pdf_text.get_mut(index)
            {
                slot.absorb(page_text.clone());
            }
            self.model.page_extents[index] = crate::raster::PageExtent {
                width: bitmap.width,
                height: bitmap.height,
            };
            let page = crate::raster::RenderedPage {
                index,
                handle: iced::widget::image::Handle::from_rgba(
                    bitmap.width,
                    bitmap.height,
                    bitmap.rgba,
                ),
                width: bitmap.width,
                height: bitmap.height,
                allocation: None,
            };
            if self.model.pages.get(index).is_some() {
                self.model.pages.mark_refreshing(index);
            } else {
                self.model.pages.insert(page.clone(), protect.clone());
            }
            if self.model.pages.get(index).is_none() {
                continue;
            }
            let generation = self.model.raster_generation;
            swaps.push(
                iced::widget::image::allocate(page.handle.clone()).map(move |allocation| {
                    Msg::PageRasterAllocated {
                        generation,
                        page: Box::new(page.clone()),
                        allocation: allocation.ok(),
                    }
                }),
            );
        }
        Task::batch(swaps)
    }

    // Fold a completed band into the working set.
    //
    // Stale generations are dropped, so a previous document's band never paints here.
    pub(crate) fn finish_rasterised_band(
        &mut self,
        generation: u64,
        requested: Vec<usize>,
        pages: Vec<message::PageBitmap>,
        text: Vec<crate::pdf::PdfPageText>,
    ) -> Task<Msg> {
        if generation != self.model.raster_generation {
            return Task::none();
        }
        // Every requested page leaves the in-flight set, or an empty one would never retry.
        for index in &requested {
            self.model.rasterising.remove(index);
            if !pages.iter().any(|page| page.index == *index) {
                // Recorded so `ensure_page_rasters` does not ask again (see
                // `Model::raster_failed`).
                self.model.raster_failed.insert(*index);
            }
        }
        let protect = self
            .visible_band()
            .unwrap_or(self.model.current_page..=self.model.current_page);
        let swaps = self.absorb_rasters(pages, text, protect);
        if let Some(snapshot) = self.model.pdf_annotation_snapshot.clone() {
            self.reconcile_native_pdf_annotations(snapshot);
        }
        swaps
    }

    pub(crate) fn page_offset_of(&self, page: usize) -> f32 {
        crate::view::reading::page_scroll_offset(
            &self.model,
            self.themes.theme(),
            self.reading_content_width(),
            page,
        )
    }

    // The first page of the row after (`forward`) or before the one holding
    // `page`: the next page, or in a spread the next pair's first.
    pub(crate) fn adjacent_page_of(&self, page: usize, forward: bool) -> Option<usize> {
        crate::view::reading::adjacent_row_page(
            &self.model,
            self.reading_content_width(),
            page,
            forward,
        )
    }

    pub(crate) fn page_at_offset(&self, offset: f32, viewport_height: f32) -> usize {
        crate::view::reading::page_at_scroll_offset(
            &self.model,
            self.themes.theme(),
            if self.model.pages_viewport_width > 0.0 {
                self.model.pages_viewport_width
            } else {
                self.reading_content_width()
            },
            offset,
            viewport_height,
        )
    }

    pub(crate) fn visible_band(&self) -> Option<std::ops::RangeInclusive<usize>> {
        crate::view::reading::visible_page_band(
            &self.model,
            self.themes.theme(),
            self.reading_content_width(),
            self.window_height,
        )
    }

    fn raster_admission_paused(&self) -> bool {
        // Refusal messages also run the post-update ensures; admission waits for the retry pump.
        self.busy_retry_pending || self.close_state.pauses_expendable_work()
    }

    pub(crate) fn ensure_page_rasters(&mut self) -> Task<Msg> {
        if self.raster_admission_paused() {
            return Task::none();
        }
        if !matches!(self.model.format, Format::Pdf | Format::Cbz)
            || self.model.screen != Screen::Reading
            || self.model.page_extents.is_empty()
        {
            return Task::none();
        }
        let Some(path) = self.model.file_path.clone() else {
            return Task::none();
        };
        let Some(band) = self.visible_band() else {
            return Task::none();
        };
        let want: Vec<usize> = band
            .clone()
            .filter(|index| {
                !self.model.rasterising.contains(index) && !self.model.raster_failed.contains(index)
            })
            .collect();
        let missing = self.model.pages.missing(&want);
        if missing.is_empty() {
            return Task::none();
        }
        for index in &missing {
            self.model.rasterising.insert(*index);
        }
        let generation = self.model.raster_generation;
        let format = self.model.format;
        let password = self.model.pdf_password.clone();
        let cbz_entries = std::sync::Arc::clone(&self.model.cbz_entries);
        let requested = missing.clone();
        self.dispatch_blocking(move || {
            rasterise_band_blocking(path, format, password, cbz_entries, missing)
        })
        .map(move |result| match result {
            Ok((pages, text)) => Msg::PagesRasterised {
                generation,
                requested: requested.clone(),
                pages,
                text,
            },
            Err(BlockingTaskError::Busy) => Msg::RasterBandRefusedBusy {
                generation,
                indices: requested.clone(),
            },
            // Empty terminal bands mark requested pages failed instead of retrying.
            Err(_terminal) => Msg::PagesRasterised {
                generation,
                requested: requested.clone(),
                pages: Vec::new(),
                text: Vec::new(),
            },
        })
    }

    fn arranger_thumbnail_pages(&self) -> Vec<usize> {
        let theme = self.themes.theme();
        let content = self.reading_content_width();
        let gap = theme.spacing.md();
        let columns = crate::view::page_arranger::grid_columns(
            crate::view::page_arranger::grid_width(content, theme),
            gap,
        );
        let range = crate::view::page_arranger::visible_entries(&self.model, columns, gap);
        self.model.arrangement.entries()[range]
            .iter()
            .filter_map(|entry| match entry.source {
                crate::pages::PageSource::Original(page) => Some(page),
                crate::pages::PageSource::Merged { .. } => None,
            })
            .filter(|page| !self.model.thumbnail_rasters.contains(*page))
            .take(crate::raster::THUMBNAIL_BATCH_LIMIT)
            .collect()
    }

    pub(crate) fn ensure_pdf_thumbnails(&mut self) -> Task<Msg> {
        if self.raster_admission_paused() {
            return Task::none();
        }
        let arranging = self.model.overlay == Some(model::Overlay::PageArranger);
        if self.model.format != Format::Pdf
            || self.model.screen != Screen::Reading
            || (self.model.sidebar_tab != model::SidebarTab::Thumbnails && !arranging)
            || self.model.page_count == 0
        {
            return Task::none();
        }
        let Some(path) = self.model.file_path.clone() else {
            return Task::none();
        };
        // The arranger's tiles in view, else the sidebar's band.
        let band: Vec<usize> = if arranging {
            self.arranger_thumbnail_pages()
        } else {
            let Some(band) = crate::raster::thumbnail_band(
                self.model.page_count,
                self.model.thumbnail_scroll_offset,
                self.model.thumbnail_viewport_height.max(240.0),
            ) else {
                return Task::none();
            };
            band.collect()
        };
        let wanted = band
            .into_iter()
            .filter(|page| {
                !self.model.thumbnail_rasterising.contains(page)
                    && !self.model.thumbnail_failed.contains(page)
            })
            .collect::<Vec<_>>();
        let missing = self.model.thumbnail_rasters.missing(&wanted);
        if missing.is_empty() {
            return Task::none();
        }
        for page in &missing {
            self.model.thumbnail_rasterising.insert(*page);
        }
        let generation = self.model.thumbnail_generation;
        let password = self.model.pdf_password.clone();
        let requested = missing.clone();
        self.dispatch_blocking(move || {
            crate::pdf::render_page_thumbnails(
                &path.to_string_lossy(),
                password.as_deref(),
                &missing,
            )
            .unwrap_or_default()
        })
        .map(move |result| match result {
            Ok(pages) => Msg::PdfThumbnailsRendered {
                generation,
                requested: requested.clone(),
                pages,
            },
            Err(BlockingTaskError::Busy) => Msg::PdfThumbnailsRefusedBusy {
                generation,
                indices: requested.clone(),
            },
            // Empty terminal bands mark requested pages failed instead of retrying.
            Err(_terminal) => Msg::PdfThumbnailsRendered {
                generation,
                requested: requested.clone(),
                pages: Vec::new(),
            },
        })
    }

    pub(crate) fn finish_epub(&mut self, path: PathBuf, load: EpubLoadResult) {
        self.model.page_count = load.chapters.len();
        let start_chapter = first_readable_chapter(&load.chapters);
        self.model.epub_current_chapter = start_chapter;
        let title = display_title(&load.metadata.title, &self.model.file_name);
        // The resolved title the recents card and library record use also heads the
        // titlebar.
        self.model.display_title = title.clone();
        let author = load.metadata.author.clone();
        // Built once at load so per-keystroke search never rebuilds it.
        self.model.search_index = crate::search::SearchIndex::build(&load.chapters);
        self.model.epub = Some(EpubDoc {
            load,
            chapter_paragraphs: HashMap::new(),
            highlighted_chapter: None,
        });
        self.model.screen = Screen::Reading;
        let key = persistence::canonical_key(&path.to_string_lossy());
        let saved = self
            .model
            .app_state
            .files
            .get(&key)
            .filter(|record| record.format.is_empty() || record.format.eq_ignore_ascii_case("epub"))
            .cloned();
        if let Some(rec) = &saved {
            if let Some(size) = rec.epub_font_size {
                self.model.epub_font_size = size;
                self.model.clamp_font_size();
            }
            if let Some(spacing) = rec.epub_line_spacing {
                self.model.epub_line_spacing = spacing.clamp(1.0, 2.5);
            }
            // Older records carry only the two-way dark-theme flag.
            self.model.reading_theme = match rec.epub_theme.as_deref() {
                Some(id) => ReadingTheme::from_id(id),
                None => ReadingTheme::from_legacy_bool(rec.epub_dark_theme.unwrap_or(false)),
            };
            self.model.epub_view_mode = rec
                .epub_view_mode
                .as_deref()
                .map(EpubViewMode::from_id)
                .unwrap_or(EpubViewMode::Scrolled);
        }
        let recovered = match saved.as_ref() {
            Some(record) => self.restore_epub_position(record),
            None => {
                self.enter_chapter(start_chapter);
                false
            }
        };
        if recovered {
            self.model
                .refuse("Reading position adjusted because the document changed.");
        }
        self.model.pending_position_restore = if saved.is_some()
            && self.model.epub_view_mode == EpubViewMode::Scrolled
            && self.model.epub_scroll_fraction > 0.0
        {
            self.current_epub_progress_anchor()
                .map(|anchor| model::PendingPositionRestore::Epub {
                    generation: self.model.position_restore_generation,
                    chapter: self.model.epub_current_chapter,
                    anchor,
                    expected_fraction: self.model.epub_scroll_fraction,
                    operation_issued: false,
                    bootstrap_events_remaining: 1,
                })
        } else {
            None
        };

        // Undo history is per-document; see the note in `finish_pages`.
        self.model.undo_stack.clear();

        // Record/refresh this book in the library once its viewport agrees.
        if self.model.pending_position_restore.is_none() {
            self.persist_progress(&path, title, author, "epub");
        }
    }
}
