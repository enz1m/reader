//! Shared metadata for inline and overflow actions.

use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "Mirrors the shared action metadata and inline control contract"
)]
pub(crate) fn metadata_action<'a>(
    descriptor: Descriptor,
    id: &'static str,
    label: impl Into<String>,
    icon: &'static str,
    group: &'static str,
    message: Msg,
    enabled: bool,
    build: impl Fn() -> Element<'a, Msg> + 'a,
) -> ActionItem<'a, Msg> {
    let label = label.into();
    ActionItem::with_inline(
        descriptor,
        Action::new(id, message)
            .with_label(crate::actions::translated_label(&label))
            .with_icon(Icon::named(icon))
            .with_enabled(enabled),
        move |_, _| build(),
    )
    .group(group)
}
