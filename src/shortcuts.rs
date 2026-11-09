//! Keyboard bindings shared by dispatch, help and menu labels.
//!
//! Document commands require content focus; modal presentations take precedence.

use crate::message::Msg;
use enzui_iced_core::shortcut::{Modifier, Shortcut};
use enzui_iced_core::{ShortcutBinding, dispatch_shortcut_command, shortcut_help};
use enzui_iced_widgets::shortcut_list::ShortcutEntry;
use iced::keyboard::{self, key::Named};
use std::sync::LazyLock;

/// A command the shortcut catalog can dispatch. `Msg` carries payloads a static catalog
/// cannot know, so [`command_message`] resolves them from the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Redo,
    ToggleBookmark,
    CopySelection,
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    ToggleSidebar,
    Metadata,
    RotateClockwise,
    NextPage,
    PreviousPage,
    /// Focus the Find-in-document field.
    FindInDocument,
    /// Step to the next find-in-document match; the find panel has no Prev/Next buttons,
    /// so this is what produces `Msg::NextMatch`.
    FindNext,
    /// Step to the previous find-in-document match.
    FindPrevious,
    /// Open the file picker.
    OpenFile,
}

impl Command {
    /// The label shown in the Keyboard Shortcuts dialog, naming the action rather than
    /// the chord.
    pub const fn description(self) -> &'static str {
        match self {
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::ToggleBookmark => "Toggle bookmark",
            Command::CopySelection => "Copy selected text",
            Command::ZoomIn => "Zoom in",
            Command::ZoomOut => "Zoom out",
            Command::FitWidth => "Fit width",
            Command::FitPage => "Fit page",
            Command::ToggleSidebar => "Toggle sidebar",
            Command::Metadata => "Edit PDF metadata",
            Command::RotateClockwise => "Rotate clockwise",
            Command::NextPage => "Next page",
            Command::PreviousPage => "Previous page",
            Command::FindInDocument => "Find in document",
            Command::FindNext => "Find next",
            Command::FindPrevious => "Find previous",
            Command::OpenFile => "Open file",
        }
    }
}

/// reader-app's catalog binding: a typed [`Command`] with no context predicate.
///
/// The context is `()`: the one model-dependent gate, pagination requiring a focused
/// document, needs the live model, so it stays in the reducer (see
/// [`is_document_command`]).
pub type ReaderBinding = ShortcutBinding<Command>;

fn binding(
    id: &'static str,
    command: Command,
    primary: Shortcut,
    alternates: impl IntoIterator<Item = Shortcut>,
) -> ReaderBinding {
    ShortcutBinding::new(id, command, primary)
        .with_description(command.description())
        .with_alternates(alternates)
}

/// Command catalog in help order; PageUp/Down, Space/Shift+Space and horizontal arrows turn
/// pages, while vertical arrows scroll.
pub static ALL_BINDINGS: LazyLock<Vec<ReaderBinding>> = LazyLock::new(|| {
    vec![
        binding(
            "edit.undo",
            Command::Undo,
            Shortcut::new(Modifier::CTRL, 'z'),
            [],
        ),
        binding(
            "edit.redo",
            Command::Redo,
            Shortcut::new(Modifier::CTRL | Modifier::SHIFT, 'z'),
            // The Windows-style redo, established across Linux document apps.
            [Shortcut::new(Modifier::CTRL, 'y')],
        ),
        binding(
            "reader.toggle-bookmark",
            Command::ToggleBookmark,
            Shortcut::new(Modifier::CTRL, 'd'),
            [],
        ),
        binding(
            "reader.copy-selection",
            Command::CopySelection,
            Shortcut::new(Modifier::CTRL, 'c'),
            [],
        ),
        // `Ctrl+=` rather than `Ctrl++`: the unshifted key is what the compositor
        // publishes.
        binding(
            "view.zoom-in",
            Command::ZoomIn,
            Shortcut::new(Modifier::CTRL, '='),
            [],
        ),
        binding(
            "view.zoom-out",
            Command::ZoomOut,
            Shortcut::new(Modifier::CTRL, '-'),
            [],
        ),
        binding(
            "view.fit-width",
            Command::FitWidth,
            Shortcut::new(Modifier::CTRL, '1'),
            [],
        ),
        binding(
            "view.fit-page",
            Command::FitPage,
            Shortcut::new(Modifier::CTRL, '2'),
            [],
        ),
        binding(
            "view.sidebar",
            Command::ToggleSidebar,
            Shortcut::new(Modifier::NONE, Named::F9),
            [],
        ),
        binding(
            "document.metadata",
            Command::Metadata,
            Shortcut::new(Modifier::CTRL, 'i'),
            [],
        ),
        binding(
            "view.rotate-clockwise",
            Command::RotateClockwise,
            Shortcut::new(Modifier::CTRL, 'r'),
            [],
        ),
        binding(
            "reader.next-page",
            Command::NextPage,
            Shortcut::new(Modifier::NONE, Named::PageDown),
            [
                Shortcut::new(Modifier::NONE, Named::Space),
                Shortcut::new(Modifier::NONE, Named::ArrowRight),
            ],
        ),
        binding(
            "reader.previous-page",
            Command::PreviousPage,
            Shortcut::new(Modifier::NONE, Named::PageUp),
            [
                Shortcut::new(Modifier::SHIFT, Named::Space),
                Shortcut::new(Modifier::NONE, Named::ArrowLeft),
            ],
        ),
        binding(
            "reader.find-in-document",
            Command::FindInDocument,
            Shortcut::new(Modifier::CTRL, 'f'),
            [],
        ),
        // F3 / Shift+F3 with Ctrl+G / Ctrl+Shift+G alternates. Non-text keys are not
        // captured by a focused Find field (`crate::app::classify_event_in`), so stepping
        // works while the query holds the caret.
        binding(
            "reader.find-next",
            Command::FindNext,
            Shortcut::new(Modifier::NONE, Named::F3),
            [Shortcut::new(Modifier::CTRL, 'g')],
        ),
        binding(
            "reader.find-previous",
            Command::FindPrevious,
            Shortcut::new(Modifier::SHIFT, Named::F3),
            [Shortcut::new(Modifier::CTRL | Modifier::SHIFT, 'g')],
        ),
        binding(
            "file.open",
            Command::OpenFile,
            Shortcut::new(Modifier::CTRL, 'o'),
            [],
        ),
    ]
});

/// The one binding catalog.
pub fn bindings() -> &'static [ReaderBinding] {
    ALL_BINDINGS.as_slice()
}

/// The catalog row for `command`, if any.
#[must_use]
pub fn binding_for(command: Command) -> Option<&'static ReaderBinding> {
    bindings().iter().find(|b| *b.command() == command)
}

/// Resolve a real iced key event against the catalog through the framework matcher.
///
/// Returns `None` for no match, an OS key-repeat, or a non-press event. The
/// document-focus gate applies later, in the reducer (see [`is_document_command`]).
#[must_use]
pub fn dispatch(event: &keyboard::Event) -> Option<Command> {
    dispatch_shortcut_command(bindings(), event, &())
}

/// The primary chord bound to `command`, rendered as e.g. `Ctrl+D`, or empty when the
/// command is unbound.
#[must_use]
pub fn chord_display(command: Command) -> String {
    binding_for(command)
        .map(|b| b.primary().to_string())
        .unwrap_or_default()
}

/// Help rows in catalog order; each badge includes the command aliases.
#[must_use]
pub fn shortcut_dialog_entries_from(bindings: &[ReaderBinding]) -> Vec<ShortcutEntry> {
    shortcut_help(bindings.iter())
        .into_iter()
        .map(|row| {
            let chords = row
                .shortcuts()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            ShortcutEntry::new(row.section(), chords, row.description()).stable_key(row.id())
        })
        .collect()
}

/// The shortcuts dialog's one row per command, aliases included in that row.
#[must_use]
pub fn shortcut_dialog_entries() -> Vec<ShortcutEntry> {
    shortcut_dialog_entries_from(bindings())
}

/// Whether `command` needs an open document under the caret to mean anything.
///
/// Pagination and the document commands are only meaningful on the reading
/// surface; see `crate::app::classify_event` for where this gates dispatch.
pub const fn is_document_command(command: Command) -> bool {
    match command {
        Command::NextPage | Command::PreviousPage | Command::CopySelection => true,
        // Stepping matches moves the document, so F3 in a modal must not move it; the
        // Find field is not an overlay, so the gate stays open there.
        Command::FindNext | Command::FindPrevious => true,
        Command::Undo
        | Command::Redo
        | Command::ToggleBookmark
        | Command::ZoomIn
        | Command::ZoomOut
        | Command::FitWidth
        | Command::FitPage
        | Command::ToggleSidebar
        | Command::Metadata
        | Command::RotateClockwise => false,
        // Find and Open work without an open document.
        Command::FindInDocument | Command::OpenFile => false,
    }
}

/// Resolve payload-free catalog commands; the reducer supplies live model state such as the
/// current bookmark page.
pub fn command_message(command: Command) -> Msg {
    match command {
        Command::Undo => Msg::Undo,
        Command::Redo => Msg::Redo,
        Command::ToggleBookmark => Msg::ToggleCurrentBookmark,
        Command::CopySelection => Msg::CopySelection,
        Command::ZoomIn => Msg::ZoomIn,
        Command::ZoomOut => Msg::ZoomOut,
        Command::FitWidth => Msg::ZoomFitWidth,
        Command::FitPage => Msg::ZoomFitPage,
        Command::ToggleSidebar => Msg::ToggleSidebar,
        Command::Metadata => Msg::OpenMetadata,
        Command::RotateClockwise => Msg::RotateClockwise,
        Command::NextPage => Msg::NextPage,
        Command::PreviousPage => Msg::PrevPage,
        Command::FindInDocument => Msg::OpenSearch,
        Command::FindNext => Msg::NextMatch,
        Command::FindPrevious => Msg::PrevMatch,
        Command::OpenFile => Msg::OpenDialog,
    }
}

#[cfg(test)]
pub(crate) fn event_for_shortcut(shortcut: &Shortcut) -> iced::Event {
    event_for_shortcut_repeat(shortcut, false)
}

#[cfg(test)]
pub(crate) fn event_for_shortcut_repeat(shortcut: &Shortcut, repeat: bool) -> iced::Event {
    let mut modifiers = iced::keyboard::Modifiers::default();
    let m = shortcut.modifiers();
    if m.contains(Modifier::CTRL) {
        modifiers |= iced::keyboard::Modifiers::CTRL;
    }
    if m.contains(Modifier::SHIFT) {
        modifiers |= iced::keyboard::Modifiers::SHIFT;
    }
    if m.contains(Modifier::ALT) {
        modifiers |= iced::keyboard::Modifiers::ALT;
    }
    if m.contains(Modifier::SUPER) {
        modifiers |= iced::keyboard::Modifiers::LOGO;
    }
    let key = match shortcut.key().as_character() {
        Some(c) => iced::keyboard::Key::Character(c.into()),
        None => iced::keyboard::Key::Named(
            shortcut
                .key()
                .as_named()
                .expect("a shortcut key is either a character or a named key"),
        ),
    };
    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        modified_key: key.clone(),
        key,
        // The dispatcher matches the logical key, so this code decides nothing.
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyZ),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat,
    })
}
