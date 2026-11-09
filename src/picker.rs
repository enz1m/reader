//! Tagged native file choosers with retained Wayland parent exports.

use std::path::PathBuf;

use enzui_iced_widgets::file_chooser::{
    CancelReason, ChooserOptions, ChooserParent, ChooserSession, FileFilter, FileResult,
    PortalOperation, PortalOutcome, PortalWake,
};

/// Which affordance opened the chooser, so its terminal file grant is routed
/// to either document loading or arranged-PDF publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    /// The "Open File" affordance: choose a document to open.
    OpenBook,
    /// Choose a portal-authorised destination for an arranged PDF.
    SaveArrangement,
    /// Choose a PDF whose pages are appended in the page arranger.
    MergePdf,
    /// Choose a PKCS #12 certificate to sign with.
    SigningCertificate,
    /// Choose where the signed copy is written.
    SaveSigned,
}

impl PickerTarget {
    /// The portal operation this target asks for: a single existing file.
    pub fn operation(self) -> PortalOperation {
        match self {
            Self::OpenBook | Self::MergePdf | Self::SigningCertificate => PortalOperation::OpenFile,
            Self::SaveArrangement | Self::SaveSigned => PortalOperation::SaveFile,
        }
    }

    /// The chooser's window title.
    pub fn title(self) -> String {
        crate::actions::translated_label(match self {
            Self::OpenBook => "Open a document",
            Self::SaveArrangement => "Save arranged PDF",
            Self::MergePdf => "Add pages from a PDF",
            Self::SigningCertificate => "Choose a certificate",
            Self::SaveSigned => "Save signed PDF",
        })
    }

    /// The dropdown filters: a default combined "Documents" entry (including CBZ) plus PDF
    /// and EPUB entries.
    pub fn filters(self) -> (FileFilter, Vec<FileFilter>) {
        match self {
            Self::OpenBook => {
                let documents = FileFilter::new(crate::actions::translated_label("Documents"))
                    .glob("*.pdf")
                    .glob("*.epub")
                    .glob("*.cbz");
                let pdf = FileFilter::new("PDF").glob("*.pdf");
                let epub = FileFilter::new("EPUB").glob("*.epub");
                (documents.clone(), vec![documents, pdf, epub])
            }
            Self::SaveArrangement | Self::MergePdf | Self::SaveSigned => {
                let pdf = FileFilter::new("PDF").glob("*.pdf");
                (pdf.clone(), vec![pdf])
            }
            Self::SigningCertificate => {
                let p12 =
                    FileFilter::new(crate::actions::translated_label("Certificates (PKCS #12)"))
                        .glob("*.p12")
                        .glob("*.pfx");
                (p12.clone(), vec![p12])
            }
        }
    }

    fn suggested_name(self) -> Option<&'static str> {
        match self {
            Self::OpenBook | Self::MergePdf | Self::SigningCertificate => None,
            Self::SaveArrangement => Some("arranged-document.pdf"),
            Self::SaveSigned => Some("signed.pdf"),
        }
    }
}

/// `source` renamed with `suffix` before its extension, in its own folder:
/// `report.pdf` with `_signed` is `report_signed.pdf`.
pub fn beside_with_suffix(source: &std::path::Path, suffix: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .map_or_else(|| "document".into(), |stem| stem.to_string_lossy());
    let extension = source
        .extension()
        .map_or_else(|| "pdf".into(), |ext| ext.to_string_lossy());
    source.with_file_name(format!("{stem}{suffix}.{extension}"))
}

/// A human, actionable sentence for a non-success outcome.
///
/// Every failure arm is named, so a new `PortalOutcome` variant cannot render as a
/// generic failure. [`PortalOutcome::Completed`] and [`PortalOutcome::Cancelled`] return
/// `None`.
pub fn failure_text_for(
    target: PickerTarget,
    outcome: &PortalOutcome<FileResult>,
) -> Option<String> {
    if target == PickerTarget::SaveArrangement {
        let text = match outcome {
            PortalOutcome::Completed(_) | PortalOutcome::Cancelled(_) => return None,
            PortalOutcome::Unavailable => {
                "No file chooser on this system, so the arranged PDF was not saved."
            }
            PortalOutcome::Unsupported => {
                "This desktop's file portal cannot choose a save destination."
            }
            PortalOutcome::NotDeclared => {
                "This build is not permitted to open a save destination chooser."
            }
            PortalOutcome::Denied => "The save destination chooser was refused permission.",
            PortalOutcome::ParentUnavailable => {
                "The save destination chooser could not be attached to this window."
            }
            PortalOutcome::Failed => "The save destination chooser failed to open.",
        };
        return Some(crate::actions::translated_label(text));
    }
    let mut message = failure_text(outcome)?;
    if target == PickerTarget::OpenBook
        && matches!(
            outcome,
            PortalOutcome::Unavailable | PortalOutcome::Unsupported | PortalOutcome::NotDeclared
        )
    {
        message.push(' ');
        message.push_str(&crate::actions::translated_label(
            "Pass a document path on the command line instead.",
        ));
    }
    Some(message)
}

pub fn failure_text(outcome: &PortalOutcome<FileResult>) -> Option<String> {
    enzui_iced_widgets::file_chooser::failure_text(outcome)
}

/// The first local file from a completed result, if any.
///
/// A non-local or empty result yields `None`, and the caller reports a visible failure.
pub fn first_local_file(result: &FileResult) -> Option<PathBuf> {
    result.local_paths().into_iter().next()
}

/// The app-side portal chooser state.
pub struct Picker {
    session: ChooserSession<PickerTarget>,
    /// The file name and folder the next save chooser starts at, in place
    /// of the target's generic name.
    next_save_as: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker")
            .field("pending", &self.session.is_pending())
            .finish()
    }
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    /// A picker with no backend built and nothing in flight.
    pub fn new() -> Self {
        Self {
            session: ChooserSession::new(),
            next_save_as: None,
        }
    }

    #[cfg(test)]
    pub fn with_backend(backend: Box<dyn enzui_iced_widgets::file_chooser::PortalBackend>) -> Self {
        Self {
            session: ChooserSession::with_backend(backend),
            next_save_as: None,
        }
    }

    /// Whether a request is in flight. The completion subscription is gated on
    /// this, so an app with no chooser open holds no portal wake source.
    pub fn is_pending(&self) -> bool {
        self.session.is_pending()
    }

    /// Start the next save chooser at `path`: its folder, and its name.
    pub fn suggest_save_as(&mut self, path: std::path::PathBuf) {
        self.next_save_as = Some(path);
    }

    fn save_as_options(&mut self, target: PickerTarget, options: ChooserOptions) -> ChooserOptions {
        let suggested = self.next_save_as.take();
        match suggested
            .as_deref()
            .and_then(|path| Some((path.parent()?, path.file_name()?)))
        {
            Some((folder, name)) => options
                .current_folder(folder)
                .current_name(name.to_string_lossy()),
            None => match target.suggested_name() {
                Some(name) => options.current_name(name),
                None => options,
            },
        }
    }

    /// Opens the host's chooser for `target`.
    ///
    /// Returns `Err(outcome)` for a request that never started; it is a terminal outcome
    /// handled like a late one.
    pub fn start(&mut self, target: PickerTarget) -> Result<(), PortalOutcome<FileResult>> {
        if self.is_pending() {
            // One dialog at a time, as a real portal enforces per parent.
            return Ok(());
        }

        let (current, filters) = target.filters();
        let mut options = ChooserOptions::new()
            .title(target.title())
            .current_filter(current);
        for filter in filters {
            options = options.filter(filter);
        }
        options = self.save_as_options(target, options);

        self.session.start(target, target.operation(), options)
    }

    /// Opens the host chooser with a live exported parent.
    ///
    /// The lease remains owned until [`Self::pump`] observes a terminal outcome.
    /// A stale parent returns [`PortalOutcome::ParentUnavailable`] before launch.
    pub fn start_parented(
        &mut self,
        target: PickerTarget,
        window: iced::window::Id,
        export: enzui_iced_wayland::ParentExport,
        is_live: impl Fn(iced::window::Id) -> bool,
    ) -> Result<(), PortalOutcome<FileResult>> {
        if self.is_pending() {
            return Ok(());
        }

        let (current, filters) = target.filters();
        let mut options = ChooserOptions::new()
            .title(target.title())
            .current_filter(current);
        for filter in filters {
            options = options.filter(filter);
        }
        options = self.save_as_options(target, options);

        self.session.start_parented(
            target,
            target.operation(),
            options,
            ChooserParent::Wayland { window, export },
            is_live,
        )
    }

    /// The event-driven wake source for the live request.
    ///
    /// `Subscription::none()` unless a request is pending with an asynchronous backend, so
    /// a settled app holds no portal wake source.
    pub fn completion_subscription(&self) -> iced::Subscription<PortalWake> {
        self.session.subscription()
    }

    /// Returns one terminal outcome, retaining parent custody until settlement.
    pub fn pump(&mut self) -> Option<(PickerTarget, PortalOutcome<FileResult>)> {
        let mut captured = None;
        let _task: iced::Task<()> = self.session.pump(|target, outcome| {
            captured = Some((target, outcome));
        });
        captured
    }

    /// Cancels and settles the chooser before releasing its parent export.
    pub fn cancel(&mut self, reason: CancelReason) {
        let _ = self.session.cancel(reason);
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.cancel(CancelReason::Application);
    }
}

#[cfg(test)]
pub mod tests {
    //! An in-process [`PortalBackend`] the reducer tests inject, so the outcome flow runs
    //! with no D-Bus or dialog.

    use std::sync::{Arc, Mutex};

    use enzui_iced_widgets::file_chooser::{
        BackendError, BackendReply, CancelReason, ChooserOptions, ParentHandle, PortalBackend,
        PortalOperation, PortalResponse, RequestId,
    };

    /// What the backend was asked to do, for assertions.
    #[derive(Debug, Default)]
    pub struct StartLog {
        pub started: Vec<PortalOperation>,
        pub parents: Vec<String>,
        pub titles: Vec<Option<String>>,
        pub filter_names: Vec<Vec<String>>,
        /// Each chooser's starting file name and folder.
        pub names: Vec<Option<String>>,
        pub folders: Vec<Option<std::path::PathBuf>>,
        pub closes: usize,
    }

    /// A scripted, in-process portal backend.
    pub struct ScriptedBackend {
        log: Arc<Mutex<StartLog>>,
        reply: Option<BackendReply>,
        start_error: Option<BackendError>,
        next_id: u64,
    }

    impl ScriptedBackend {
        pub fn silent() -> Self {
            Self {
                log: Arc::new(Mutex::new(StartLog::default())),
                reply: None,
                start_error: None,
                next_id: 0,
            }
        }

        pub fn replying(response: PortalResponse, uris: Vec<String>) -> Self {
            Self {
                reply: Some(BackendReply { response, uris }),
                ..Self::silent()
            }
        }

        pub fn failing(e: BackendError) -> Self {
            Self {
                start_error: Some(e),
                ..Self::silent()
            }
        }

        pub fn log(&self) -> Arc<Mutex<StartLog>> {
            Arc::clone(&self.log)
        }
    }

    impl PortalBackend for ScriptedBackend {
        fn start(&mut self, operation: PortalOperation) -> Result<RequestId, BackendError> {
            self.start_with_options(operation, &ChooserOptions::new(), &ParentHandle::none())
        }

        fn start_with_options(
            &mut self,
            operation: PortalOperation,
            options: &ChooserOptions,
            parent: &ParentHandle,
        ) -> Result<RequestId, BackendError> {
            if let Some(e) = self.start_error {
                return Err(e);
            }
            self.next_id += 1;
            let mut log = self.log.lock().unwrap();
            log.started.push(operation);
            log.parents.push(parent.as_str().to_string());
            log.titles.push(options.title_str().map(str::to_string));
            log.names.push(options.name().map(str::to_string));
            log.folders
                .push(options.folder().map(std::path::Path::to_path_buf));
            log.filter_names.push(
                options
                    .filters()
                    .iter()
                    .map(|f| f.name().to_string())
                    .collect(),
            );
            Ok(RequestId::new(self.next_id))
        }

        fn poll_reply(&mut self, _id: RequestId) -> Option<BackendReply> {
            self.reply.take()
        }

        fn close(&mut self, _id: RequestId, _reason: CancelReason) {
            self.log.lock().unwrap().closes += 1;
        }

        fn describe(&self) -> String {
            "scripted (in-process, no D-Bus)".to_string()
        }
    }
}
