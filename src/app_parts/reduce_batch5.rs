//! Fifth reducer batch: focus, metadata, page actions.

use super::*;

use iced::Task;

use crate::plumbing::pdf_ops::MetadataField;

impl App {
    pub(crate) fn batch5_focus_previous(&mut self) -> Task<Msg> {
        operate(traverse::<Msg>(Traversal::Previous))
    }

    pub(crate) fn batch5_undo(&mut self) -> Task<Msg> {
        self.replay_history(false)
    }

    pub(crate) fn batch5_redo(&mut self) -> Task<Msg> {
        self.replay_history(true)
    }

    pub(crate) fn batch5_open_page_arranger(&mut self) -> Task<Msg> {
        // The titlebar button toggles: pressed again, it returns to the PDF.
        if self.model.overlay == Some(model::Overlay::PageArranger) {
            return self.batch5_close_page_arranger();
        }
        if self.model.page_editing_supported() {
            // Rebuild only for a fresh document, so re-entering keeps the user's moves and
            // their undo history.
            if self.model.arrangement.is_empty() {
                self.model.arrangement =
                    crate::pages::PageArrangement::from_page_count(self.page_tree_len());
            }
            return crate::overlay_focus::open(&mut self.model, model::Overlay::PageArranger);
        }
        Task::none()
    }

    pub(crate) fn batch5_close_page_arranger(&mut self) -> Task<Msg> {
        let dismissed = crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::CloseButton,
        );
        self.back_from_page_arranger(dismissed)
    }

    pub(crate) fn batch5_close_overlay(&mut self) -> Task<Msg> {
        self.model.pending_pdf_comment = None;
        let was_arranging = self.model.overlay == Some(model::Overlay::PageArranger);
        let dismissed = crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::CloseButton,
        );
        if was_arranging {
            return self.back_from_page_arranger(dismissed);
        }
        dismissed
    }

    // The reading surface is rebuilt when the arranger closes, so it comes
    // back at the top; return it to the page the reader was on.
    fn back_from_page_arranger(&mut self, dismissed: Task<Msg>) -> Task<Msg> {
        if self.model.overlay == Some(model::Overlay::PageArranger) {
            return dismissed;
        }
        self.model.pages_scroll_offset = self.page_offset_of(self.model.current_page);
        Task::batch([dismissed, self.scroll_to_current_page()])
    }

    pub(crate) fn batch5_shell_dialog_event(
        &mut self,
        event: enzui_iced_widgets::modal_host::DialogEvent<crate::overlay_focus::DialogKey>,
    ) -> Task<Msg> {
        let task = crate::overlay_focus::reduce(&mut self.model, event);
        if self.model.overlay != Some(model::Overlay::NoteEditor) {
            self.model.pending_pdf_comment = None;
        }
        task
    }

    pub(crate) fn batch5_move_page_up(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        if let Some((from, to)) = self.model.arrangement.move_up(id) {
            self.model
                .undo_stack
                .record(UndoableAction::MovePage { id, from, to });
            self.model
                .announce(format!("Moved page to position {}", to + 1));
        }
        Task::none()
    }

    pub(crate) fn batch5_move_page_down(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        if let Some((from, to)) = self.model.arrangement.move_down(id) {
            self.model
                .undo_stack
                .record(UndoableAction::MovePage { id, from, to });
            self.model
                .announce(format!("Moved page to position {}", to + 1));
        }
        Task::none()
    }

    pub(crate) fn batch5_page_dropped(
        &mut self,
        request: enzui_iced_core::drag::DropRequest,
    ) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        // One `DropRequest` of stable ids and a destination gap, resolved exactly once.
        if let Some(item) = request.items.first()
            && let Some(id) = crate::pages::id_from_key(item.as_str())
            && let Some((from, to)) = self
                .model
                .arrangement
                .move_to_slot(id, request.slot.index())
        {
            self.model
                .undo_stack
                .record(UndoableAction::MovePage { id, from, to });
            self.model
                .announce(format!("Moved page to position {}", to + 1));
        }
        Task::none()
    }

    pub(crate) fn batch5_open_page(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        let Some(index) = self.model.arrangement.position(id) else {
            return Task::none();
        };
        self.model.current_page = index;
        crate::overlay_focus::dismiss(
            &mut self.model,
            enzui_iced_core::modal::DismissCause::DefaultAction,
        )
    }

    pub(crate) fn batch5_request_delete_page(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        if self.model.arrangement.len() <= 1 {
            self.model
                .refuse("A PDF must keep at least one page; nothing was deleted.");
            return Task::none();
        }
        self.ask(model::PendingPageAction::DeletePage(id));
        Task::none()
    }

    pub(crate) fn request_delete_pages(&mut self, ids: Vec<u64>) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() || ids.is_empty() {
            return Task::none();
        }
        if ids.len() >= self.model.arrangement.len() {
            self.model
                .refuse("A PDF must keep at least one page; nothing was deleted.");
            return Task::none();
        }
        self.ask(model::PendingPageAction::DeletePages(ids));
        Task::none()
    }

    pub(crate) fn batch5_request_extract_page(&mut self, id: u64) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        self.ask(model::PendingPageAction::ExtractPage(id));
        Task::none()
    }

    pub(crate) fn batch5_apply_page_arrangement(&mut self) -> Task<Msg> {
        let Some(destination) = self.model.file_path.clone() else {
            return Task::none();
        };
        self.begin_arrangement_publication(destination)
    }

    pub(crate) fn batch5_save_page_arrangement_as(&mut self) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        let Some(source) = self.model.file_path.clone() else {
            self.model
                .refuse("No source PDF is open; the draft was retained.");
            return Task::none();
        };
        self.picker
            .suggest_save_as(crate::picker::beside_with_suffix(&source, "_arranged"));
        self.save_arrangement_pick_source = Some(source);
        if let Some(window) = self.main_window {
            return self.begin_parented_pick(crate::picker::PickerTarget::SaveArrangement, window);
        }
        self.open_pick_unparented(crate::picker::PickerTarget::SaveArrangement)
    }

    pub(crate) fn batch5_page_arrangement_published(
        &mut self,
        generation: u64,
        source: PathBuf,
        destination: PathBuf,
        arrangement: crate::pages::PageArrangement,
        result: Result<
            crate::plumbing::pdf_ops::ArrangementPublication,
            crate::plumbing::pdf_ops::PdfWriteError,
        >,
    ) -> Task<Msg> {
        self.finish_arrangement_publication(generation, source, destination, arrangement, result)
    }

    pub(crate) fn batch5_request_merge_pdf(&mut self) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        // Choose the PDF in the file chooser; its path then merges the same
        // way a typed one did.
        if let Some(window) = self.main_window {
            return self.begin_parented_pick(crate::picker::PickerTarget::MergePdf, window);
        }
        self.open_pick_unparented(crate::picker::PickerTarget::MergePdf)
    }

    pub(crate) fn batch5_merge_path_changed(&mut self, text: String) -> Task<Msg> {
        self.model.merge_path = text;
        Task::none()
    }

    pub(crate) fn batch5_merge_pdf(&mut self) -> Task<Msg> {
        if self.model.arrangement_publication.is_some() {
            return Task::none();
        }
        self.merge_pdf();
        Task::none()
    }

    pub(crate) fn batch5_page_confirm_event(
        &mut self,
        event: enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent,
    ) -> Task<Msg> {
        self.on_page_confirm(event)
    }

    pub(crate) fn batch5_open_metadata(&mut self) -> Task<Msg> {
        if self.model.metadata_supported()
            && let Some(path) = self.model.file_path.clone()
        {
            if self
                .model
                .metadata_save_failure
                .as_ref()
                .is_some_and(|failure| failure.path == path)
            {
                return crate::overlay_focus::open(&mut self.model, model::Overlay::Metadata);
            }
            match plumbing::pdf_ops::read_metadata(&path) {
                Ok(meta) => {
                    self.model.metadata_baseline = meta.clone();
                    self.model.metadata = meta;
                    self.model.reset_metadata_date_ui();
                    self.model.metadata_save_failure = None;
                    return crate::overlay_focus::open(&mut self.model, model::Overlay::Metadata);
                }
                Err(diagnostic) => {
                    eprintln!("reader-app PDF metadata load failure: diagnostic={diagnostic:?}");
                    self.model.error = Some("PDF metadata could not be loaded.".into());
                }
            }
        }
        Task::none()
    }

    pub(crate) fn batch5_open_properties(&mut self) -> Task<Msg> {
        if self.model.metadata_supported()
            && let Some(path) = self.model.file_path.clone()
        {
            match plumbing::pdf_ops::read_properties(&path) {
                Ok(props) => {
                    self.model.properties = props;
                    return crate::overlay_focus::open(&mut self.model, model::Overlay::Properties);
                }
                Err(diagnostic) => {
                    eprintln!("reader-app PDF properties load failure: diagnostic={diagnostic:?}");
                    self.model.error = Some("PDF properties could not be loaded.".into());
                }
            }
        }
        Task::none()
    }

    pub(crate) fn batch5_metadata_field_changed(
        &mut self,
        field: MetadataField,
        value: String,
    ) -> Task<Msg> {
        self.model.edit_metadata(field, value);
        Task::none()
    }

    pub(crate) fn batch5_metadata_date_picker(
        &mut self,
        field: MetadataField,
        event: enzui_iced_widgets::date_picker::DatePickerEvent,
    ) -> Task<Msg> {
        use enzui_iced_widgets::date_picker::DatePickerEvent;
        let Some(slot) = model::metadata_date_slot(field) else {
            return Task::none();
        };
        let current = plumbing::pdf_ops::pdf_date_parts(field.get(&self.model.metadata));
        let picker = &mut self.model.metadata_dates[slot].picker;
        let trigger = picker.trigger_id();
        let restore = match event {
            DatePickerEvent::TriggerPressed => {
                picker.toggle(current.as_ref().map(|(date, _, _)| *date), Some(trigger))
            }
            DatePickerEvent::Calendar(event) => {
                let (picked, restore) = picker.apply_calendar_event_and_close(event);
                if let Some(date) = picked {
                    // A new day keeps the time and zone already written, or
                    // the time typed before there was a day.
                    let typed = model::metadata_time_format()
                        .parse(&self.model.metadata_dates[slot].time_draft)
                        .ok();
                    let (time, zone) = match current {
                        Some((_, time, zone)) => (time, zone),
                        None => (
                            typed.unwrap_or(enzui_iced_core::time_of_day::TimeOfDay::MIDNIGHT),
                            String::new(),
                        ),
                    };
                    self.model.metadata_dates[slot].time_draft =
                        model::metadata_time_format().format(time);
                    self.model.edit_metadata(
                        field,
                        plumbing::pdf_ops::pdf_date_from_parts(date, time, &zone),
                    );
                }
                restore
            }
            DatePickerEvent::Dismissed => {
                picker
                    .apply_dismissal(enzui_iced_core::dismissal::Dismissal::Dismiss)
                    .restore_focus
            }
        };
        restore.map_or_else(Task::none, restore_focus_task)
    }

    pub(crate) fn batch5_metadata_time(
        &mut self,
        field: MetadataField,
        event: enzui_iced_widgets::time_field::TimeFieldEvent,
    ) -> Task<Msg> {
        use enzui_iced_widgets::time_field::TimeFieldEvent;
        let Some(slot) = model::metadata_date_slot(field) else {
            return Task::none();
        };
        match event {
            TimeFieldEvent::Drafted(text) => self.model.metadata_dates[slot].time_draft = text,
            TimeFieldEvent::Committed(time) => {
                self.model.metadata_dates[slot].time_draft =
                    model::metadata_time_format().format(time);
                // A time with no day yet is kept in the field until a day
                // is picked.
                if let Some((date, _, zone)) =
                    plumbing::pdf_ops::pdf_date_parts(field.get(&self.model.metadata))
                {
                    self.model.edit_metadata(
                        field,
                        plumbing::pdf_ops::pdf_date_from_parts(date, time, &zone),
                    );
                }
            }
            TimeFieldEvent::Refused(_) => {}
        }
        Task::none()
    }

    pub(crate) fn batch5_save_metadata(&mut self) -> Task<Msg> {
        self.save_metadata(plumbing::pdf_ops::MetadataDateRemoval::default());
        Task::none()
    }

    pub(crate) fn batch5_retry_metadata_save(&mut self) -> Task<Msg> {
        if let Some(retry) = self.model.metadata_save_failure.clone()
            && self.model.file_path.as_ref() == Some(&retry.path)
        {
            self.save_metadata(retry.confirmed_removal);
        }
        Task::none()
    }
}
