//! Toolbar actions.

use super::decorations::metadata_action;
use super::*;

pub(crate) fn toolbar_actions<'a>(
    model: &'a Model,
    theme: &'a EnzTheme,
    spacing: &SpacingScale,
) -> Vec<ActionItem<'a, Msg>> {
    let mut items: Vec<ActionItem<'a, Msg>> = Vec::new();

    if model.screen == Screen::Reading && model.format == Format::Epub {
        // Chapter navigation is pinned and enabled by content-chapter availability, so
        // it greys out exactly where stepping stops.
        let prev_enabled = model.epub_prev_content_chapter().is_some();
        items.push(ActionItem::new(
            Descriptor::new("reader.nav.prev-chapter").pinned(),
            move || {
                if prev_enabled {
                    icon_button("chevron-left", Msg::PrevChapter, "Previous chapter", None)
                } else {
                    icon_disabled("chevron-left", "Previous chapter")
                }
            },
        ));
        let next_enabled = model.epub_next_content_chapter().is_some();
        items.push(ActionItem::new(
            Descriptor::new("reader.nav.next-chapter").pinned(),
            move || {
                if next_enabled {
                    icon_button("chevron-right", Msg::NextChapter, "Next chapter", None)
                } else {
                    icon_disabled("chevron-right", "Next chapter")
                }
            },
        ));

        // Font size: priority 100, the most-reached comfort control, so it folds last.
        items.push(metadata_action(
            Descriptor::new("reader.epub.font-down").with_priority(VisibilityPriority(100)),
            "reader.epub.font-down",
            "Smaller text",
            "a-arrow-down",
            "text",
            Msg::FontSizeDown,
            true,
            move || icon_button("a-arrow-down", Msg::FontSizeDown, "Smaller text", None),
        ));
        items.push(metadata_action(
            Descriptor::new("reader.epub.font-up").with_priority(VisibilityPriority(100)),
            "reader.epub.font-up",
            "Larger text",
            "a-arrow-up",
            "text",
            Msg::FontSizeUp,
            true,
            move || icon_button("a-arrow-up", Msg::FontSizeUp, "Larger text", None),
        ));

        // The chooser anchor must remain mounted when other actions fold.
        let theme_menu_open = model.reading_theme_menu_open;
        let current_theme = model.reading_theme;
        items.push(metadata_action(
            Descriptor::new("reader.epub.theme").pinned(),
            "reader.epub.theme",
            "Reading theme",
            "palette",
            "text",
            Msg::ToggleReadingThemeMenu,
            true,
            move || reading_theme_control(theme_menu_open, current_theme, theme),
        ));

        items.push(find_action("reader.epub.find"));

        // Line spacing: priority 40.
        items.push(metadata_action(
            Descriptor::new("reader.epub.line-spacing-up")
                .with_priority(VisibilityPriority(40))
                .always_overflow(),
            "reader.epub.line-spacing-up",
            "More line spacing",
            "unfold-vertical",
            "layout",
            Msg::LineSpacingUp,
            true,
            move || {
                icon_button(
                    "unfold-vertical",
                    Msg::LineSpacingUp,
                    "More line spacing",
                    None,
                )
            },
        ));
        items.push(metadata_action(
            Descriptor::new("reader.epub.line-spacing-down")
                .with_priority(VisibilityPriority(40))
                .always_overflow(),
            "reader.epub.line-spacing-down",
            "Less line spacing",
            "fold-vertical",
            "layout",
            Msg::LineSpacingDown,
            true,
            move || {
                icon_button(
                    "fold-vertical",
                    Msg::LineSpacingDown,
                    "Less line spacing",
                    None,
                )
            },
        ));

        // View mode: priority 20. `mode_action` offers the mode not currently in use.
        let mode = crate::actions::mode_action(model.epub_view_mode);
        let mode_icon: &'static str = mode.icon();
        let mode_tooltip: &'static str = mode.tooltip();
        items.push(metadata_action(
            Descriptor::new("reader.epub.view-mode")
                .with_priority(VisibilityPriority(20))
                .always_overflow(),
            "reader.epub.view-mode",
            mode_tooltip,
            mode_icon,
            "layout",
            Msg::ToggleEpubViewMode,
            true,
            move || icon_button(mode_icon, Msg::ToggleEpubViewMode, mode_tooltip, None),
        ));
    }

    // Zoom and rotation are absent for EPUB rather than inert. Priority 100 for zoom, 90
    // for Fit width, 40 for Rotate. Comic archives have no text layer to search.
    if model.screen == crate::model::Screen::Reading && model.format == crate::model::Format::Pdf {
        items.push(find_action("reader.pdf.find"));
    }

    if model.page_view_controls_supported() {
        use crate::actions::ViewAction;
        let zoom_out = ViewAction::ZoomOut;
        let can_zoom_out = model.zoom.can_zoom_out();
        let zoom_out_tip = zoom_out.tooltip();
        let zoom_out_icon = zoom_out.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.zoom-out").with_priority(VisibilityPriority(100)),
            "reader.pdf.zoom-out",
            zoom_out_tip,
            zoom_out_icon,
            "view",
            Msg::ZoomOut,
            can_zoom_out,
            move || {
                if can_zoom_out {
                    icon_button(zoom_out_icon, Msg::ZoomOut, zoom_out_tip, None)
                } else {
                    icon_disabled(zoom_out_icon, zoom_out_tip)
                }
            },
        ));
        let fit = ViewAction::FitWidth;
        // An action, not a toggle: every document opens at fit width, so a pressed look
        // would read as stuck.
        let fit_tip = fit.tooltip();
        let fit_icon = fit.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.fit-width").with_priority(VisibilityPriority(90)),
            "reader.pdf.fit-width",
            fit_tip,
            fit_icon,
            "view",
            Msg::ZoomFitWidth,
            true,
            move || icon_button(fit_icon, Msg::ZoomFitWidth, fit_tip, None),
        ));
        let zoom_in = ViewAction::ZoomIn;
        let can_zoom_in = model.zoom.can_zoom_in();
        let zoom_in_tip = zoom_in.tooltip();
        let zoom_in_icon = zoom_in.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.zoom-in").with_priority(VisibilityPriority(100)),
            "reader.pdf.zoom-in",
            zoom_in_tip,
            zoom_in_icon,
            "view",
            Msg::ZoomIn,
            can_zoom_in,
            move || {
                if can_zoom_in {
                    icon_button(zoom_in_icon, Msg::ZoomIn, zoom_in_tip, None)
                } else {
                    icon_disabled(zoom_in_icon, zoom_in_tip)
                }
            },
        ));
        let fit_page_tip = ViewAction::FitPage.tooltip();
        let fit_page_icon = ViewAction::FitPage.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.fit-page")
                .with_priority(VisibilityPriority(90))
                .always_overflow(),
            "reader.pdf.fit-page",
            fit_page_tip,
            fit_page_icon,
            "view",
            Msg::ZoomFitPage,
            true,
            move || icon_button(fit_page_icon, Msg::ZoomFitPage, fit_page_tip, None),
        ));
        let rotate = ViewAction::Rotate;
        let rotate_active = model.rotation != crate::viewport::PageRotation::NONE;
        let rotate_tip = rotate.tooltip();
        let rotate_icon = rotate.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.rotate")
                .with_priority(VisibilityPriority(40))
                .always_overflow(),
            "reader.pdf.rotate",
            rotate_tip,
            rotate_icon,
            "view",
            Msg::RotateClockwise,
            true,
            move || {
                icon_button(
                    rotate_icon,
                    Msg::RotateClockwise,
                    rotate_tip,
                    Some(rotate_active),
                )
            },
        ));
    }

    if model.page_view_controls_supported() {
        use crate::actions::ViewAction;
        let single = model.page_view_mode == crate::viewport::PageViewMode::SinglePage;
        let mode_label = if single {
            "Continuous pages"
        } else {
            "Single page"
        };
        items.push(metadata_action(
            Descriptor::new("reader.pages.view-mode").always_overflow(),
            "reader.pages.view-mode",
            mode_label,
            "file",
            "view",
            Msg::TogglePageViewMode,
            true,
            move || icon_button("file", Msg::TogglePageViewMode, mode_label, Some(single)),
        ));
        let spread = ViewAction::TwoPages;
        let spread_on = model.spread_mode == crate::spread::SpreadMode::TwoPage;
        let spread_tip = spread.tooltip();
        let spread_icon = spread.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.two-pages")
                .with_priority(VisibilityPriority(35))
                .always_overflow(),
            "reader.pdf.two-pages",
            spread_tip,
            spread_icon,
            "view",
            Msg::ToggleSpread,
            true,
            move || icon_button(spread_icon, Msg::ToggleSpread, spread_tip, Some(spread_on)),
        ));
    }

    // PDF structure editing folds first: rare acts with their own dialogs, priority
    // 20/15/10/5. Highlight and comment live in the selection's actions strip.
    if model.page_editing_supported() {
        use crate::actions::ViewAction;
        let arranger_open = model.overlay == Some(crate::model::Overlay::PageArranger);
        items.push(metadata_action(
            Descriptor::new("reader.pdf.arranger").with_priority(VisibilityPriority(20)),
            "reader.pdf.arranger",
            super::super::page_arranger::TITLE,
            "layout-grid",
            "document",
            Msg::OpenPageArranger,
            true,
            move || {
                super::super::widgets::icon_button_with_id(
                    "layout-grid",
                    Msg::OpenPageArranger,
                    super::super::page_arranger::TITLE,
                    Some(arranger_open),
                    Some(super::super::widgets::page_arranger_button_id()),
                )
            },
        ));
        let metadata_open = model.overlay == Some(crate::model::Overlay::Metadata);
        items.push(metadata_action(
            Descriptor::new("reader.pdf.metadata")
                .with_priority(VisibilityPriority(15))
                .always_overflow(),
            "reader.pdf.metadata",
            super::super::metadata_dialog::EDIT_METADATA_ACTION,
            "tag",
            "document",
            Msg::OpenMetadata,
            true,
            move || {
                super::super::widgets::icon_button_with_id(
                    "tag",
                    Msg::OpenMetadata,
                    super::super::metadata_dialog::EDIT_METADATA_ACTION,
                    Some(metadata_open),
                    Some(super::super::widgets::metadata_button_id()),
                )
            },
        ));
        let properties_open = model.overlay == Some(crate::model::Overlay::Properties);
        items.push(metadata_action(
            Descriptor::new("reader.pdf.properties")
                .with_priority(VisibilityPriority(10))
                .always_overflow(),
            "reader.pdf.properties",
            super::super::metadata_dialog::PROPERTIES_TITLE,
            "info",
            "document",
            Msg::OpenProperties,
            true,
            move || {
                super::super::widgets::icon_button_with_id(
                    "info",
                    Msg::OpenProperties,
                    super::super::metadata_dialog::PROPERTIES_TITLE,
                    Some(properties_open),
                    Some(super::super::widgets::properties_button_id()),
                )
            },
        ));
        let split = ViewAction::SplitPdf;
        let split_tip = split.tooltip();
        let split_icon = split.icon();
        items.push(metadata_action(
            Descriptor::new("reader.pdf.split")
                .with_priority(VisibilityPriority(5))
                .always_overflow(),
            "reader.pdf.split",
            split_tip,
            split_icon,
            "document",
            Msg::RequestSplitPdf,
            true,
            move || icon_button(split_icon, Msg::RequestSplitPdf, split_tip, None),
        ));
        // A password-unlocked PDF offers removal; any other offers protection.
        let (password_tip, password_icon) = if model.pdf_password.is_some() {
            (
                super::super::pdf_password_dialog::REMOVE_ACTION,
                "lock-open",
            )
        } else {
            (super::super::pdf_password_dialog::PROTECT_ACTION, "lock")
        };
        items.push(metadata_action(
            Descriptor::new("reader.pdf.sign")
                .with_priority(VisibilityPriority(30))
                .always_overflow(),
            "reader.pdf.sign",
            "Sign\u{2026}",
            "signature",
            "document",
            Msg::OpenSignDialog,
            true,
            move || icon_button("signature", Msg::OpenSignDialog, "Sign\u{2026}", None),
        ));
        items.push(metadata_action(
            Descriptor::new("reader.pdf.password")
                .with_priority(VisibilityPriority(4))
                .always_overflow(),
            "reader.pdf.password",
            password_tip,
            password_icon,
            "document",
            Msg::OpenPdfPasswordForm,
            true,
            move || icon_button(password_icon, Msg::OpenPdfPasswordForm, password_tip, None),
        ));
    }

    // Page edits made outside the arranger (a rotation from a page's menu) wait in
    // the draft; Save writes them to the file, as the arranger's Apply does.
    let arranger_open = model.overlay == Some(crate::model::Overlay::PageArranger);
    if model.page_editing_supported()
        && !arranger_open
        && model.arrangement_publication.is_none()
        && !model
            .arrangement
            .is_identity(model.total_page_count.max(model.page_count))
    {
        items.push(ActionItem::new(
            Descriptor::new("reader.pdf.save-pages").pinned(),
            move || {
                icon_button(
                    "save",
                    Msg::ApplyPageArrangement,
                    super::super::states::SAVE_PAGE_CHANGES,
                    None,
                )
            },
        ));
    }

    // Open is pinned last, at the trailing edge.
    items.push(ActionItem::new(
        Descriptor::new("reader.file.open").pinned(),
        move || {
            super::super::widgets::icon_button_with_id(
                "folder-open",
                Msg::OpenDialog,
                "Open document",
                None,
                Some(iced::widget::Id::new("reader.open")),
            )
        },
    ));

    // `SpaceToken::Xs` (4 px) keeps the cluster one group; `spacing` stays in the
    // signature so a caller's scale is not silently ignored later.
    let _ = spacing;
    // Right alignment makes the pinned `Open document` hug the trailing edge of the
    // Fill slot; folding is measured before alignment.
    items
}

// Find in document: priority 80. Opens the search sidebar tab with its field focused.
fn find_action<'a>(id: &'static str) -> ActionItem<'a, Msg> {
    metadata_action(
        Descriptor::new(id).with_priority(VisibilityPriority(80)),
        id,
        "Find in document",
        "search",
        "find",
        Msg::OpenSearch,
        true,
        move || icon_button("search", Msg::OpenSearch, "Find in document", None),
    )
}
