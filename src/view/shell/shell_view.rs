//! Shell root view.

use super::composition::shell_composition;
use super::content::dialog_surface;
use super::*;

pub fn view<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Msg> {
    let spacing = &theme.spacing;

    let base: Element<'a, Msg> =
        shell_composition(model, theme, spacing, window_width, window_height);

    let open = model.context_menu.as_ref();
    let generation = open.map_or(0, |open| open.generation);
    let menu = open.map_or(&model.closed_context_menu, |open| &open.menu);
    let mut host = menu
        .host(base, move |event| Msg::ContextMenuEvent(generation, event))
        .id_prefix("reader-context-menu");
    if let Some(target) = menu.target() {
        if let Some(invoker) = crate::context_menu::invoker_id(target) {
            host = host.invoker(invoker);
        }
        if let Some(label) = crate::context_menu::menu_label(target, model) {
            host = host.label(label);
        }
    }
    let base: Element<'a, Msg> = host.into_element();

    let base = model.dialogs.view_with(
        base,
        |_key| dialog_surface(model, theme).expect("presented Reader form has a surface"),
        Msg::ShellDialogEvent,
        |_key, host| host.label("Reader dialog"),
    );

    let confirmation = model.pending_page_action.as_ref().map(|action| {
        enz_confirm_dialog(
            action.title(),
            confirm_message(model, action),
            &model.page_confirm_state,
            Msg::PageConfirmEvent,
        )
        .confirm_label(action.confirm_label())
        .cancel_label("Cancel")
        .destructive(action.is_destructive())
        .alert(true)
        .id_prefix("reader-page-confirm")
    });
    enzui_iced_widgets::confirm_dialog::confirm_dialog_host(
        base,
        confirmation,
        Msg::PageConfirmEvent,
    )
}
