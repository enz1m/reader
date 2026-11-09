//! DialogStack lifecycle with Reader-owned draft and secret cleanup.

use crate::{
    app::overlay_restore_id,
    message::Msg,
    model::{Model, Overlay},
};
use enzui_iced_core::modal::DismissCause;
use enzui_iced_widgets::modal_host::DialogEvent;
use iced::Task;

/// The form carried by the stack; the stack stamps each presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogKey {
    pub overlay: Overlay,
}

pub(crate) fn open(model: &mut Model, overlay: Overlay) -> Task<Msg> {
    if let Some(key) = model.dialogs.topmost().cloned() {
        model.dialogs.close(&key);
    }
    model.overlay = Some(overlay);
    if overlay == Overlay::PageArranger {
        return Task::none();
    }
    let key = DialogKey { overlay };
    model.dialogs.open(key, overlay_restore_id(Some(overlay)))
}

fn forget(model: &mut Model) {
    let closing_password = model.overlay == Some(Overlay::Password);
    model.overlay = None;
    model.pending_page_action = None;
    model.note_editor = None;
    model.note_editor_editing = None;
    model.note_editor_position = None;
    if closing_password {
        model.clear_password();
    }
}

pub(crate) fn dismiss(model: &mut Model, cause: DismissCause) -> Task<Msg> {
    let task = if let Some(key) = model.dialogs.topmost().cloned() {
        let (closed, task) = model.dialogs.dismiss(&key, cause);
        if closed.is_empty() {
            return task;
        }
        task
    } else {
        Task::none()
    };
    forget(model);
    task
}

pub(crate) fn close_for_host(model: &mut Model) -> Task<Msg> {
    if let Some(key) = model.dialogs.topmost().cloned() {
        model.dialogs.close(&key);
    }
    model.overlay = None;
    Task::none()
}

pub(crate) fn reduce(model: &mut Model, event: DialogEvent<DialogKey>) -> Task<Msg> {
    let (closed, task) = model.dialogs.update(event);
    if !closed.is_empty() {
        forget(model);
    }
    task
}
