//! Seventh reducer batch: notes, PDF comments, menus.

use super::*;

use iced::Task;

use crate::message::PageThumbnail;

impl App {
    pub(crate) fn batch7_open_note_editor(&mut self) -> Task<Msg> {
        self.model.pending_pdf_comment = None;
        self.model.note_editor = Some(enzui_iced_widgets::form_dialog::FormPresenter::new(
            crate::view::note_editor::note_form_model(""),
        ));
        self.model.note_editor_editing = None;
        self.model.note_editor_page = self.model.current_position();
        self.model.note_editor_position = None;
        crate::overlay_focus::open(&mut self.model, model::Overlay::NoteEditor)
    }

    pub(crate) fn batch7_open_note_editor_for_edit(&mut self, id: u64) -> Task<Msg> {
        let Some(note) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|a| a.id == id)
        else {
            return Task::none();
        };
        let draft = if note.kind == "highlight" && note.pdf_binding.is_some() {
            note.note.as_str()
        } else {
            note.text.as_str()
        };
        self.model.note_editor = Some(enzui_iced_widgets::form_dialog::FormPresenter::new(
            crate::view::note_editor::note_form_model(draft),
        ));
        self.model.pending_pdf_comment = None;
        self.model.note_editor_editing = Some(id);
        self.model.note_editor_page = note.page;
        self.model.note_editor_position = None;
        crate::overlay_focus::open(&mut self.model, model::Overlay::NoteEditor)
    }

    pub(crate) fn batch7_note_editor_action(
        &mut self,
        action: iced::widget::text_editor::Action,
    ) -> Task<Msg> {
        // No live presenter means the dialog is closed; drop the action.
        if let Some(presenter) = self.model.note_editor.as_mut() {
            let (_outcome, task) = presenter.update::<Msg>(
                enzui_iced_widgets::form_dialog::FormDialogEvent::EditorAction(
                    crate::view::note_editor::note_field_id(),
                    action,
                ),
                None,
            );
            return task;
        }
        Task::none()
    }

    pub(crate) fn batch7_commit_note_editor(&mut self) -> Task<Msg> {
        // A blank draft is declined without closing the dialog, so an
        // accidental keyboard commit does not discard the draft.
        let Some(text) = crate::view::note_editor::committed_note(&self.model) else {
            return Task::none();
        };
        if text.is_empty() {
            return Task::none();
        }
        if let Some(selection) = self.model.pending_pdf_comment.clone() {
            // Admit optimistically; the sidecar update lands on `Msg::PdfMutationCompleted`.
            let (admit, result) = self.create_pdf_annotation(
                plumbing::pdf_ops::PdfAnnotationKind::Comment,
                selection,
                &text,
                "",
            );
            if matches!(result, AdmissionResult::Admitted) {
                self.model.note_editor = None;
                self.model.pending_pdf_comment = None;
                let dismiss = crate::overlay_focus::dismiss(
                    &mut self.model,
                    enzui_iced_core::modal::DismissCause::CloseButton,
                );
                return Task::batch([admit, dismiss]);
            }
            // Queued or Refused: the composer stays open so the draft is not lost.
            return admit;
        }
        if let Some(id) = self.model.note_editor_editing {
            match self.edit_pdf_annotation_comment(id, &text) {
                EditCommentOutcome::Native {
                    task,
                    result: AdmissionResult::Admitted,
                } => {
                    self.model.note_editor = None;
                    self.model.note_editor_editing = None;
                    let dismiss = crate::overlay_focus::dismiss(
                        &mut self.model,
                        enzui_iced_core::modal::DismissCause::CloseButton,
                    );
                    return Task::batch([task, dismiss]);
                }
                EditCommentOutcome::Native { task, .. } => {
                    // Queued or Refused: the composer stays open so the draft is not lost.
                    return task;
                }
                EditCommentOutcome::Failed => return Task::none(),
                EditCommentOutcome::NoChange => {
                    self.model.note_editor = None;
                    self.model.note_editor_editing = None;
                    return crate::overlay_focus::dismiss(
                        &mut self.model,
                        enzui_iced_core::modal::DismissCause::CloseButton,
                    );
                }
                EditCommentOutcome::NonNative => {}
            }
            self.model.note_editor = None;
            self.model.note_editor_editing = None;
            self.model.editing_annotation = Some((id, text));
            let commit = self.reduce(Msg::CommitEditAnnotation);
            let dismiss = crate::overlay_focus::dismiss(
                &mut self.model,
                enzui_iced_core::modal::DismissCause::CloseButton,
            );
            return Task::batch([commit, dismiss]);
        }
        self.model.note_editor = None;
        let page = self.model.note_editor_page;
        let position = self
            .model
            .note_editor_position
            .take()
            .unwrap_or(persistence::AnnotationPosition::None);
        let id = persistence::add_annotation_with_position(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            page,
            "note",
            &text,
            "",
            position,
        );
        self.record_add_annotation(id);
        self.model.announce("Note added");
        self.save_annotations();
        crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::CloseButton,
        )
    }

    pub(crate) fn batch7_add_pdf_comment(&mut self) -> Task<Msg> {
        self.open_pdf_comment_editor()
    }

    pub(crate) fn batch7_add_pdf_highlight(&mut self) -> Task<Msg> {
        // Check the document kind first, so an EPUB never asks for a PDF selection.
        if self.model.format != Format::Pdf {
            self.model.refuse("PDF annotations need a PDF.");
            return Task::none();
        }
        let Some(selection) = self.model.pdf_selection.clone() else {
            self.model.refuse(crate::actions::SELECT_TEXT_FIRST);
            return Task::none();
        };
        let (task, result) = self.create_pdf_annotation(
            plumbing::pdf_ops::PdfAnnotationKind::Highlight,
            selection,
            "",
            "yellow",
        );
        if matches!(result, AdmissionResult::Admitted) {
            // Cleared only on admission; Queued or Refused leaves it for a retry.
            self.model.pdf_selection = None;
        }
        task
    }

    pub(crate) fn batch7_pdf_text_selected(
        &mut self,
        page: usize,
        range: std::ops::Range<usize>,
        finished: bool,
    ) -> Task<Msg> {
        self.set_pdf_selection(page, range);
        self.model.pdf_selection_actions = finished && self.model.pdf_selection.is_some();
        Task::none()
    }

    pub(crate) fn batch7_clear_pdf_selection(&mut self) -> Task<Msg> {
        self.model.pdf_selection = None;
        self.model.pdf_selection_actions = false;
        // A page click also releases the selected annotation and its ring.
        self.model.selected_annotation = None;
        Task::none()
    }

    pub(crate) fn batch7_copy_pdf_selection(&mut self) -> Task<Msg> {
        let Some(selection) = self.model.pdf_selection.as_ref() else {
            return Task::none();
        };
        self.model.pdf_selection_actions = false;
        iced::clipboard::write(selection.text.clone()).map(Msg::SelectionCopied)
    }

    pub(crate) fn batch7_find_selection(&mut self) -> Task<Msg> {
        let text = match (&self.model.pdf_selection, &self.model.active_selection) {
            (Some(selection), _) => selection.text.clone(),
            (None, Some(selection)) => selection.text.clone(),
            (None, None) => return Task::none(),
        };
        let query = text.split_whitespace().collect::<Vec<_>>().join(" ");
        self.model.pdf_selection_actions = false;
        if self.model.active_selection.take().is_some() {
            self.bump_selection_clear();
        }
        // The same panel state the Find control opens, seeded through the
        // debounced off-thread path.
        let open = self.update(Msg::OpenSearch);
        let seed = self.update(Msg::SearchQueryChanged(query));
        Task::batch([open, seed])
    }

    pub(crate) fn batch7_find_similar(&mut self, id: u64) -> Task<Msg> {
        // The query is that row's own text, not the PDF selection.
        let Some(text) = self
            .model
            .annotations
            .annotations
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.text.clone())
        else {
            return Task::none();
        };
        // Same panel state as the Find control, seeded through the debounced off-thread
        // path.
        let open = self.update(Msg::OpenSearch);
        let seed = self.update(Msg::SearchQueryChanged(text));
        Task::batch([open, seed])
    }

    pub(crate) fn batch7_remove_from_recents(&mut self, path: String) -> Task<Msg> {
        // The removed card's button held focus, so focus moves to the successor. Snapshot
        // the position in the current order before the rebuild.
        let removed_pos = self.model.recents.iter().position(|b| b.path == path);
        if persistence::remove_recent(std::sync::Arc::make_mut(&mut self.model.app_state), &path) {
            self.library_state_revision = self.library_state_revision.saturating_add(1);
            if self.model.library_loading {
                self.library_removed_during_restore
                    .insert(persistence::canonical_key(&path));
            }
            // Coalesced like any in-memory progress mutation; CloseWindow performs the
            // durable final save.
            self.progress_dirty = true;
            self.model.recent_thumbnails.remove(&path);
            // Evict cached probe data so an in-flight scan cannot resurrect stale rows.
            self.model.availability.remove(&path);
            self.model.bookmark_counts.remove(&path);
            // Reproject now, since the focus arithmetic reads the rebuilt order.
            self.rebuild_projection_now();
            // A fresh scan drops stale results, so re-adding the path cannot inherit them.
            let rescan = self.request_availability_scan();
            // `recents_focus_after_removal` clamps for a tail removal and returns `None`
            // for an emptied list.
            if let Some(target) = removed_pos.and_then(|pos| self.recents_focus_after_removal(pos))
            {
                return Task::batch([rescan, restore_focus_task(target)]);
            }
            return rescan;
        }
        Task::none()
    }

    pub(crate) fn batch7_recent_thumbnail_rendered(
        &mut self,
        path: String,
        thumb: Option<PageThumbnail>,
    ) -> Task<Msg> {
        // `None` leaves the card on its generic glyph; Busy routes to
        // `Msg::RecentThumbnailRefusedBusy` instead.
        if let Some(t) = thumb {
            let handle = iced::widget::image::Handle::from_rgba(t.width, t.height, t.rgba);
            self.model.recent_thumbnails.insert(path.clone(), handle);
        }
        self.preview_in_flight.remove(&path);
        Task::batch([self.drain_preview_pending(), self.pump_busy_retry()])
    }

    pub(crate) fn batch7_availability_batch_done(
        &mut self,
        generation: u64,
        next_index: usize,
        total: usize,
        outcome: Option<Vec<crate::message::AvailabilityResult>>,
    ) -> Task<Msg> {
        self.apply_availability_batch(generation, next_index, total, outcome)
    }

    pub(crate) fn batch7_open_context_menu(
        &mut self,
        target: crate::context_menu::ContextTarget,
        at: iced::Point,
    ) -> Task<Msg> {
        self.model.context_menu_generation = self
            .model
            .context_menu_generation
            .checked_add(1)
            .expect("menu showing counter exhausted");
        self.model.context_menu = crate::context_menu::OpenContextMenu::new(
            target,
            at,
            &self.model,
            self.model.context_menu_generation,
        );
        Task::none()
    }

    pub(crate) fn batch7_context_menu_event(
        &mut self,
        generation: u64,
        event: enzui_iced_widgets::context_menu_host::ContextMenuEvent,
    ) -> Task<Msg> {
        let Some(open) = self.model.context_menu.as_mut() else {
            return Task::none();
        };
        if open.generation != generation {
            return Task::none();
        }
        let document_generation = open.document_generation;
        let (chosen, focus) = open.menu.update(&event);
        if !open.menu.is_open() {
            self.model.context_menu = None;
        }
        if let Some((target, action)) = chosen
            && (matches!(target, crate::context_menu::ContextTarget::LibraryBook(_))
                || document_generation == self.model.pdf_annotation_generation)
            && crate::context_menu::items_for(&target, &self.model).contains(&action)
            && let Some(message) = crate::context_menu::message_for(&target, action, &self.model)
        {
            return Task::batch([focus, self.update(message)]);
        }
        focus
    }

    pub(crate) fn batch7_open_shortcuts(&mut self) -> Task<Msg> {
        crate::overlay_focus::open(&mut self.model, model::Overlay::Shortcuts)
    }

    pub(crate) fn batch7_close_shortcuts(&mut self) -> Task<Msg> {
        crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::CloseButton,
        )
    }

    pub(crate) fn batch7_toggle_toolbar_overflow(&mut self) -> Task<Msg> {
        self.model.toolbar_overflow_open = !self.model.toolbar_overflow_open;
        Task::none()
    }

    pub(crate) fn batch7_noop(&mut self) -> Task<Msg> {
        Task::none()
    }
}
