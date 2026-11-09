//! Reader boot constructor.

use super::classify::{new_blocking_owner, refresh_projection};
use super::*;

use iced::Task;

impl App {
    /// Construct the app and its startup work.
    ///
    /// The first frame exposes a loading state while persistence and any
    /// command-line document are restored in generation order off the UI thread.
    pub fn new() -> (Self, Task<Msg>) {
        Self::new_with_launch(enzui_iced_core::activation::launch_request_from_env().intent)
    }

    pub(crate) fn new_with_launch(
        launch: enzui_iced_core::activation::ApplicationIntent,
    ) -> (Self, Task<Msg>) {
        let xdg_resolver = XdgResolver::from_env();
        let app_data_resolver = AppDataResolver::from_env();
        let annotations_root =
            std::env::var_os(persistence::ANNOTATIONS_DIR_ENV).map(PathBuf::from);
        // File parsing and the OS snapshot return as a `Task`, so nothing blocks on I/O.
        // `boot()` captures the probe, XDG paths and OS feed once.
        let app_id = enzui_iced_core::app_data::AppId::new(crate::DESKTOP_APP_ID)
            .expect("DESKTOP_APP_ID is a portable application identifier");
        let (themes, theme_boot) = ThemeIntegration::boot_for(app_id);
        let mut model = Model::new();
        // Synchronous seed: a probe-pinned light capture opens light, otherwise dark. The
        // async XDG load and OS snapshot change chrome only.
        model.reading_theme = reading_theme_for_mode(themes.theme().mode());

        let state_location_available = persistence::state_location_available(&xdg_resolver);
        let library_store = persistence::unavailable_app_state_store();
        model.library_loading = state_location_available;
        model.set_library_persistence_unavailable(!state_location_available, true);
        refresh_projection(&mut model);

        // One bounded pool for every synchronous class the reader dispatches.
        let (blocking_tasks, blocking_dispatcher) = new_blocking_owner();

        let mut app = Self {
            model,
            themes,
            // 0 until the first size event: a larger default would show the sidebar at
            // 375px if the first paint precedes the event.
            window_width: 0.0,
            // 0 = not yet reported; `pagination_metrics` falls back to the opening size.
            window_height: 0.0,
            persist_enabled: true,
            xdg_resolver,
            app_data_resolver,
            annotations_root,
            annotation_probe_observer: None,
            library_store,
            annotation_store: None,
            blocking_tasks,
            blocking_dispatcher,
            blocking_generation: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "center")]
            center: None,
            close_state: CloseState::Idle,
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
            native_drop: None,
            active_load_origin: state_location_available
                .then_some(persistence::StateOrigin::Library),
            pending_load: None,
            library_recovery_generation: 1,
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

        let startup_open = match services::document_from_launch(launch) {
            Ok(Some(path)) => Some(path),
            Ok(None) => None,
            Err(error) => {
                app.model.picker_failure = Some(error);
                None
            }
        };

        // Read the real first size: a window that opens at mobile size may never receive
        // a resize.
        let size_task = iced::window::latest().then(|maybe_id| match maybe_id {
            Some(id) => iced::window::size(id).map(Msg::WindowResized),
            None => Task::none(),
        });

        // The theme host's boot task routes back through `Msg::Theme`.
        let theme_boot = theme_boot.map(|event| Msg::Theme(Box::new(event)));
        let generation = app.library_recovery_generation;
        let base_revision = app.library_state_revision;
        let startup_restore = if state_location_available {
            let worker_resolver = app.xdg_resolver.clone();
            let task = app
                .load_controller
                .begin(&app.blocking_dispatcher, move || {
                    persistence::load_startup_library(&worker_resolver)
                });
            let failed_open_path = startup_open.clone();
            match task {
                Ok(task) => task.map(move |settlement| Msg::StartupLibraryRestored {
                    generation,
                    base_revision,
                    open_path: startup_open.clone(),
                    settlement,
                }),
                Err(error) => Task::done(Msg::StartupLibraryRestored {
                    generation,
                    base_revision,
                    open_path: failed_open_path,
                    settlement: persistence::BlockingLoadSettlement::WorkerFailed(error),
                }),
            }
        } else {
            startup_open.map_or_else(Task::none, |path| app.open(path))
        };
        let main_window = iced::window::latest().map(Msg::MainWindowReady);
        // Production is the only constructor that may expose the app;
        // `list_library` answers `unavailable` until the library settles.
        #[cfg(feature = "center")]
        app.start_center();
        (
            app,
            Task::batch([size_task, theme_boot, main_window, startup_restore]),
        )
    }
}
