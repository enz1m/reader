use super::*;

use iced::Task;

impl App {
    // Bound preview submissions and retain pending jobs across Busy refusals; missing previews
    // keep their glyph.
    pub(crate) fn thumbnail_tasks(&mut self) -> Task<Msg> {
        // `preview_jobs` drops rendered previews; `preview_in_flight` and
        // `preview_pending` guard against duplicates.
        let jobs = preview_jobs(&self.model);
        for (path, kind) in jobs {
            if self.preview_in_flight.contains(&path) {
                continue;
            }
            if self.preview_pending.iter().any(|(p, _)| p == &path) {
                continue;
            }
            self.preview_pending.push_back((path, kind));
        }
        self.drain_preview_pending()
    }

    pub(crate) fn drain_preview_pending(&mut self) -> Task<Msg> {
        if self.close_state.pauses_expendable_work() {
            return Task::none();
        }
        let mut tasks: Vec<Task<Msg>> = Vec::new();
        while self.preview_in_flight.len() < PREVIEW_INFLIGHT_CAP {
            let Some((path, kind)) = self.preview_pending.pop_front() else {
                break;
            };
            self.preview_in_flight.insert(path.clone());
            let render_path = path.clone();
            let busy_path = path.clone();
            let done_path = path;
            let task = self
                .dispatch_blocking(move || kind.render(&render_path))
                .map(move |result| match result {
                    Ok(thumb) => Msg::RecentThumbnailRendered {
                        path: done_path.clone(),
                        thumb,
                    },
                    Err(BlockingTaskError::Busy) => Msg::RecentThumbnailRefusedBusy {
                        path: busy_path.clone(),
                        source: kind,
                    },
                    Err(_terminal) => Msg::RecentThumbnailRendered {
                        path: done_path.clone(),
                        thumb: None,
                    },
                });
            tasks.push(task);
        }
        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    // Mark the coalesced busy-retry wake armed. See [`BUSY_RETRY_INTERVAL`].
    pub(crate) fn arm_busy_retry(&mut self) {
        self.busy_retry_pending = true;
    }

    #[cfg(test)]
    pub(crate) fn busy_retry_is_armed(&self) -> bool {
        self.busy_retry_pending
    }

    #[cfg(test)]
    pub(crate) fn preview_pending_snapshot(&self) -> Vec<(String, PreviewSource)> {
        self.preview_pending.iter().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn push_preview_pending_for_test(&mut self, path: String, source: PreviewSource) {
        self.preview_pending.push_back((path, source));
    }

    #[cfg(test)]
    pub(crate) fn replace_blocking_owner_for_test(
        &mut self,
        owner: enzui_iced_core::blocking_task::BlockingTaskOwner,
        dispatcher: enzui_iced_core::blocking_task::BlockingTaskDispatcher,
    ) -> enzui_iced_core::blocking_task::BlockingTaskOwner {
        let old = std::mem::replace(&mut self.blocking_tasks, owner);
        self.blocking_dispatcher = dispatcher;
        old
    }

    #[cfg(test)]
    pub(crate) fn close_state_for_test(&self) -> &CloseState {
        &self.close_state
    }

    #[cfg(test)]
    pub(crate) fn force_close_state_for_test(&mut self, state: CloseState) {
        self.close_state = state;
    }

    pub(crate) fn pump_busy_retry(&mut self) -> Task<Msg> {
        if !self.busy_retry_pending {
            return Task::none();
        }
        if self.close_state.pauses_expendable_work() {
            self.busy_retry_pending = false;
            return Task::none();
        }
        self.busy_retry_pending = false;
        let mut tasks: Vec<Task<Msg>> = Vec::with_capacity(5);
        tasks.push(self.drain_preview_pending());
        tasks.push(self.ensure_page_rasters());
        tasks.push(self.ensure_pdf_thumbnails());
        if self.model.search_pending && self.model.search_in_flight_generation.is_none() {
            tasks.push(Task::done(Msg::SearchTick));
        }
        tasks.push(self.release_retained_pdf_mutation());
        Task::batch(tasks)
    }
}
