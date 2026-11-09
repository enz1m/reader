//! App shell: view and subscription bodies.

use super::classify::classify_event_in;
use super::*;

impl App {
    pub(crate) fn render_view(&self) -> Element<'_, Msg> {
        let _scope = enzui_iced_theme::text_defaults::TextDefaultsScope::enter(self.themes.theme());
        let shell = view::view(
            &self.model,
            self.themes.theme(),
            self.window_width,
            self.window_height,
        );
        // Keep the feedback corner mounted outside modal inertness so alerts remain
        // accessible without reparenting the app. Standing notices stack above the
        // transient toasts, bottom-right.
        let notices = view::shell::notices(&self.model);
        let corner = (!notices.is_empty() || !self.model.toasts.is_empty()).then(|| {
            let mut corner = iced::widget::column![]
                .spacing(self.themes.theme().spacing.xs())
                .align_x(iced::Alignment::End);
            if !notices.is_empty() {
                corner = corner.push(
                    enzui_iced_widgets::notice_stack::enz_notice_stack(notices).into_element(),
                );
            }
            if !self.model.toasts.is_empty() {
                corner = corner.push(self.model.toasts.view().map(Msg::Toast));
            }
            corner.into()
        });
        enz_toast_layer(shell, corner, ToastOverlayPlacement::default())
    }

    #[cfg(test)]
    pub(crate) fn test_new() -> (Self, Task<Msg>) {
        Self::test_new_with_probe(enzui_iced_core::theme_file::ProbeOverride::None)
    }

    #[cfg(test)]
    pub(crate) fn test_new_with_probe(
        probe: enzui_iced_core::theme_file::ProbeOverride,
    ) -> (Self, Task<Msg>) {
        let themes = test_theme_integration(probe);
        let mut model = Model::new();
        // Same synchronous seed as production boot; async XDG/OS events change chrome
        // only, never this content palette.
        model.reading_theme = reading_theme_for_mode(themes.theme().mode());
        refresh_projection(&mut model);

        let (__test_blocking_owner, __test_blocking_dispatcher) = new_blocking_owner();
        let app = Self {
            model,
            themes,
            window_width: 0.0,
            window_height: 0.0,
            persist_enabled: false,
            xdg_resolver: XdgResolver::default(),
            app_data_resolver: AppDataResolver::default(),
            // The test thread's annotation root, carried to the blocking owner's workers.
            annotations_root: persistence::thread_annotations_root(),
            annotation_probe_observer: None,
            library_store: persistence::app_state_store_with_resolver(&XdgResolver::default()),
            annotation_store: None,
            blocking_tasks: __test_blocking_owner,
            blocking_dispatcher: __test_blocking_dispatcher,
            blocking_generation: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "center")]
            center: None,
            close_state: CloseState::Idle,
            native_drop: None,
            preview_in_flight: std::collections::HashSet::new(),
            preview_pending: std::collections::VecDeque::new(),
            busy_retry_pending: false,
            pdf_mutation_active: None,
            pdf_mutation_busy_retry: None,
            pdf_mutation_pending: None,
            pdf_mutation_uncertain: None,
            pdf_mutation_sequence: 0,
            pdf_mutation_uncertain_retry_sequence: 0,
            pdf_mutation_writer_running: None,
            pdf_mutation_writer_authority_slot: None,
            pdf_mutation_retry_running: None,
            pdf_mutation_retry_authority_slot: None,
            arrangement_publication_slot: None,
            arrangement_uncertain_authority: None,
            extract_publication_debt: None,
            load_controller: persistence::BlockingLoadController::default(),
            document_load_controller: persistence::BlockingLoadController::default(),
            pending_document_load: None,
            active_load_origin: None,
            pending_load: None,
            library_recovery_generation: 0,
            library_state_revision: 0,
            library_removed_during_restore: HashSet::new(),
            annotation_recovery_generation: 0,
            annotation_state_revision: 0,
            persistence_queue: persistence::PersistenceQueue::default(),
            persistence_debt: HashMap::new(),
            progress_dirty: false,
            annotations_dirty: false,
            picker: crate::picker::Picker::new(),
            instance: None,
            pending_launch_document: None,
            recovery_location_opener: crate::recovery_location::RecoveryLocationOpener::new(),
            main_window: None,
            pending_pick: None,
            next_pick_generation: 0,
            save_arrangement_pick_source: None,
            metadata: std::sync::Arc::new(FsMetadataBackend),
            availability: AvailabilityScan::default(),
            projection_dirty: false,
        };
        (app, Task::none())
    }

    #[cfg(test)]
    pub(crate) fn model(&self) -> &Model {
        &self.model
    }

    #[cfg(test)]
    pub(crate) fn model_mut(&mut self) -> &mut Model {
        &mut self.model
    }

    #[cfg(test)]
    pub(crate) fn enable_persist(&mut self, resolver: XdgResolver) {
        self.enable_persist_with_data(resolver, AppDataResolver::default());
    }

    #[cfg(test)]
    pub(crate) fn enable_persist_with_data(
        &mut self,
        resolver: XdgResolver,
        app_data_resolver: AppDataResolver,
    ) {
        let (library_store, library_load) = persistence::load_state_authority(&resolver);
        self.library_store = library_store;
        self.model
            .set_library_persistence_unavailable(library_load.is_unavailable(), true);
        self.annotation_store = None;
        self.load_controller = persistence::BlockingLoadController::default();
        self.active_load_origin = None;
        self.pending_load = None;
        self.library_recovery_generation = self.library_recovery_generation.wrapping_add(1);
        self.library_state_revision = 0;
        self.library_removed_during_restore.clear();
        self.annotation_recovery_generation = self.annotation_recovery_generation.wrapping_add(1);
        self.annotation_state_revision = 0;
        self.xdg_resolver = resolver;
        self.app_data_resolver = app_data_resolver;
        self.persist_enabled = true;
    }

    #[cfg(test)]
    pub(crate) fn annotation_authority_path(&self) -> Option<&Path> {
        self.annotation_store
            .as_ref()
            .map(|custody| custody.destination.as_path())
    }

    #[cfg(test)]
    pub(crate) fn observe_annotation_probe(
        &mut self,
        observer: impl FnMut(&Path) + Send + 'static,
    ) {
        let observer: Box<dyn FnMut(&Path) + Send> = Box::new(observer);
        self.annotation_probe_observer = Some(std::sync::Arc::new(std::sync::Mutex::new(observer)));
    }

    #[cfg(test)]
    pub(crate) fn clear_annotation_probe_observer(&mut self) {
        self.annotation_probe_observer = None;
    }

    #[cfg(test)]
    pub(crate) fn set_metadata_backend(&mut self, backend: std::sync::Arc<dyn MetadataBackend>) {
        self.metadata = backend;
    }

    #[cfg(test)]
    pub(crate) fn availability_active(&self) -> bool {
        self.availability.active
    }

    #[cfg(test)]
    pub(crate) fn open_epub_for_test(
        &mut self,
        path: &Path,
        load: crate::plumbing::epub_support::EpubLoadResult,
    ) {
        self.model.file_path = Some(path.to_path_buf());
        self.model.file_name = services::file_name(path);
        self.model.format = Format::Epub;
        self.model.screen = Screen::Reading;
        self.finish_epub(path.to_path_buf(), load);
    }

    #[cfg(test)]
    pub(crate) fn arm_password_prompt_for_test(&mut self, path: &Path, password: &str) {
        self.model.password_prompt = Some(path.to_path_buf());
        self.model.password_input = password.to_string();
        drop(crate::overlay_focus::open(
            &mut self.model,
            model::Overlay::Password,
        ));
        self.model.screen = Screen::Library;
    }

    pub(crate) fn document_focused(&self) -> bool {
        self.model.screen == crate::model::Screen::Reading && !self.modal_owns_keyboard()
    }

    // Point-anchored popovers keep their own dismissal without owning shell input.
    pub(crate) fn modal_owns_keyboard(&self) -> bool {
        self.model.overlay.is_some() || self.model.pending_page_action.is_some()
    }

    #[cfg(test)]
    pub(crate) fn seed_app_state_for_test(
        &mut self,
        state: crate::plumbing::persistence::AppState,
    ) {
        self.model.app_state = std::sync::Arc::new(state);
        let paths: Vec<String> = self.model.app_state.files.keys().cloned().collect();
        for path in paths {
            let exists = self.metadata.exists(&path);
            self.model.availability.insert(path, exists);
        }
        self.rebuild_projection_now();
    }

    #[cfg(test)]
    pub(crate) fn seed_highlight_for_test(&mut self, page: usize, text: &str, color: &str) -> u64 {
        let id = persistence::add_annotation(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            page,
            "highlight",
            text,
            color,
        );
        self.record_add_annotation(id);
        self.save_annotations();
        id
    }

    #[cfg(test)]
    pub(crate) fn seed_note_for_test(&mut self, page: usize, text: &str) -> u64 {
        let id = persistence::add_annotation(
            std::sync::Arc::make_mut(&mut self.model.annotations),
            page,
            "note",
            text,
            "",
        );
        self.record_add_annotation(id);
        self.save_annotations();
        id
    }

    // Types through the production `NoteEditorAction` route (not a seeded
    // draft); the composer must be open first.
    #[cfg(test)]
    pub(crate) fn type_into_note_composer_for_test(&mut self, text: &str) -> Task<Msg> {
        use iced::widget::text_editor::{Action, Edit};
        let mut tasks = Vec::new();
        for c in text.chars() {
            let action = if c == '\n' {
                Action::Edit(Edit::Enter)
            } else {
                Action::Edit(Edit::Insert(c))
            };
            tasks.push(self.update(Msg::NoteEditorAction(action)));
        }
        Task::batch(tasks)
    }

    // Derive post-removal focus from the successor path after projection refresh, never from a
    // stale row index.
    pub(crate) fn recents_focus_after_removal(
        &self,
        removed_pos: usize,
    ) -> Option<iced::advanced::widget::Id> {
        enzui_iced_core::focus_after_removal::focus_pos_after_removal(
            removed_pos,
            self.model.recents.len(),
        )
        .and_then(|pos| self.model.recents.get(pos))
        .map(|book| crate::view::library::remove_from_recents_id(&book.path))
    }

    pub(crate) fn build_subscription(&self) -> Subscription<Msg> {
        // Keyboard subscription callbacks cannot capture the model; document commands and
        // exact-window close resolve in the reducer.
        let events = iced::event::listen_with(|event, status, id| {
            if let Some(request) =
                enzui_iced_core::close_guard::CloseRequest::from_event(&event, id)
            {
                return Some(Msg::CloseWindow(Some(request.window())));
            }
            classify_event_in(&event, status)
        });
        // `DbusBackend` is not `Send`, so `update` pumps portal replies on this wake; the
        // framework gates it on a live request, so an idle reader holds no wake source.
        let mut recipes = vec![events];
        if let Some(instance) = &self.instance {
            recipes.push(instance.subscription(|_| Msg::InstanceWake));
        }
        if (self.progress_dirty
            && !self.model.library_persistence_unavailable
            && !self.model.library_loading)
            || (self.annotations_dirty
                && !self.model.annotations_unavailable
                && !self.model.annotations_loading)
        {
            // The tick exists only while a write is pending, so an idle reader has no timer.
            recipes.push(iced::time::every(PERSIST_FLUSH_INTERVAL).map(|_| Msg::FlushPersistence));
        }
        let portal_wake = self.picker.completion_subscription();
        recipes.push(portal_wake.map(|_| Msg::PortalTick));
        recipes.push(
            self.recovery_location_opener
                .completion_subscription()
                .map(|_| Msg::RecoveryLocationTick),
        );
        // One earliest-deadline wake while a toast can expire; the deadline-keyed identity
        // keeps unrelated messages from restarting it.
        if let Some((deadline, wait)) = self.model.toast_expiry.armed() {
            recipes.push(
                iced::time::every(wait)
                    .with(deadline)
                    .map(|(_deadline, now)| Msg::Toast(ToastEvent::Tick(now))),
            );
        }
        if self.model.search_pending && !self.close_state.pauses_expendable_work() {
            recipes.push(
                iced::time::every(std::time::Duration::from_millis(120)).map(|_| Msg::SearchTick),
            );
        }
        if matches!(self.close_state, CloseState::Draining { .. }) {
            recipes.push(iced::time::every(CLOSE_DRAIN_TICK).map(|_| Msg::CloseDrainTick));
        }
        // Saturating work may have no Reader completion message to trigger a retry.
        if self.busy_retry_pending && !self.close_state.pauses_expendable_work() {
            recipes.push(iced::time::every(BUSY_RETRY_INTERVAL).map(|_| Msg::BusyRetryTick));
        }
        // The shared-theme host's XDG watch and OS feed; the app adds no watcher of its own.
        recipes.push(
            self.themes
                .subscription()
                .map(|event| Msg::Theme(Box::new(event))),
        );
        // Always subscribed, so an AccessKit request can wake a quiescent Reader to drain
        // queued actions.
        recipes.push(enzui_iced_core::app_runtime::accessibility_wake_subscription(Msg::A11yWake));
        #[cfg(feature = "center")]
        recipes.push(self.center_subscription());
        Subscription::batch(recipes)
    }
}

#[cfg(test)]
use super::classify::new_blocking_owner;

#[cfg(test)]
use super::classify::refresh_projection;

#[cfg(test)]
use iced::Task;
