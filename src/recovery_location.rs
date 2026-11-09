use std::hash::{Hash, Hasher};
use std::path::Path;

use enzui_iced_core::portal::completion::{CompletionSignal, PortalWake};
use enzui_iced_core::portal::open_uri::{OpenUri, OpenUriBackend, OpenUriRequest, PendingOpenUri};
use enzui_iced_core::portal::{CancelReason, PortalOutcome, PortalParent};
use enzui_iced_widgets::file_chooser::DbusBackend;

#[derive(Clone)]
struct CompletionKey {
    request_generation: u64,
    signal: CompletionSignal,
}

impl Hash for CompletionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.request_generation.hash(state);
    }
}

pub struct RecoveryLocationOpener {
    backend: Option<Box<dyn OpenUriBackend>>,
    pending: Option<PendingOpenUri>,
    request_generation: u64,
}

impl Default for RecoveryLocationOpener {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoveryLocationOpener {
    pub fn new() -> Self {
        Self {
            backend: None,
            pending: None,
            request_generation: 0,
        }
    }

    #[cfg(test)]
    pub fn with_backend(backend: Box<dyn OpenUriBackend>) -> Self {
        Self {
            backend: Some(backend),
            pending: None,
            request_generation: 0,
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        if self
            .pending
            .as_ref()
            .is_some_and(PendingOpenUri::is_unsettled)
        {
            self.cancel();
        }
        self.request_generation = self.request_generation.wrapping_add(1);
        let uri = OpenUri::from_file_path(path)
            .map_err(|_| "The recovery location is not an absolute local path.".to_string())?;
        let Some(backend) = self.backend_mut() else {
            return Err("The desktop could not open the recovery location.".into());
        };
        match OpenUriRequest::new(uri, PortalParent::None).start(&mut **backend, |_| true) {
            Ok(pending) => {
                self.pending = Some(pending);
                Ok(())
            }
            Err(_) => Err("The desktop could not open the recovery location.".into()),
        }
    }

    pub fn completion_subscription(&self) -> iced::Subscription<PortalWake> {
        if !self
            .pending
            .as_ref()
            .is_some_and(PendingOpenUri::is_unsettled)
        {
            return iced::Subscription::none();
        }
        let Some(signal) = self
            .backend
            .as_deref()
            .and_then(OpenUriBackend::open_uri_completion_signal)
        else {
            return iced::Subscription::none();
        };
        iced::Subscription::run_with(
            CompletionKey {
                request_generation: self.request_generation,
                signal,
            },
            |key| {
                let signal = key.signal.clone();
                iced::futures::stream::unfold((signal, None::<u64>), async |(signal, seen)| {
                    match seen {
                        None => {
                            let generation = signal.generation();
                            Some((PortalWake { generation }, (signal, Some(generation))))
                        }
                        Some(seen) => {
                            let generation = signal.wait_after(seen).await;
                            Some((PortalWake { generation }, (signal, Some(generation))))
                        }
                    }
                })
            },
        )
    }

    pub fn generation(&self) -> u64 {
        self.request_generation
    }

    pub fn pump(&mut self) -> Option<(u64, PortalOutcome<()>)> {
        let pending = self.pending.as_mut()?;
        let generation = self.request_generation;
        let backend = self.backend.as_deref_mut()?;
        let mut outcome = pending.poll(backend);
        if outcome.is_none()
            && let Some(reason) = pending.cancel_reason()
        {
            outcome = Some(PortalOutcome::Cancelled(reason));
        }
        let _ = pending.settle(backend);
        if outcome.is_some() {
            self.pending = None;
        }
        outcome.map(|outcome| (generation, outcome))
    }

    pub fn cancel(&mut self) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if let Some(backend) = self.backend.as_deref_mut() {
            let _ = pending.cancel(CancelReason::Application);
            let _ = pending.settle(backend);
        }
        self.pending = None;
    }

    fn backend_mut(&mut self) -> Option<&mut Box<dyn OpenUriBackend>> {
        if self.backend.is_none() {
            match DbusBackend::new() {
                Ok(backend) => self.backend = Some(Box::new(backend)),
                Err(_) => return None,
            }
        }
        self.backend.as_mut()
    }
}
