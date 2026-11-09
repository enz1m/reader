//! Error, uncertainty and persistence notices, and the state-recovery panel.

use super::*;
use enzui_iced_widgets::notice_stack::EnzNotice;
use enzui_iced_widgets::toast::ToastTone;
use iced::widget::column;

/// The standing notices, in stack order, for the bottom-right corner they share with the
/// toasts (see `App::render_view`). Every notice can be closed.
///
/// Storage and publication failures come first, since only the first error is the live
/// region. Each failure kind keeps its own notice, so there is one alert per kind.
pub(crate) fn notices(model: &Model) -> Vec<EnzNotice<Msg>> {
    let mut notices = Vec::new();
    // Closing the persistence notice only hides it until the failure changes or a close
    // is vetoed; Retry stays the way to settle it.
    if let Some(message) = model.persistence_alert.as_deref()
        && !model.persistence_alert_hidden
    {
        notices.push(
            EnzNotice::new(
                "reader-persistence",
                "Couldn’t save your changes",
                ToastTone::Error,
            )
            .description(message)
            .action("Retry", Msg::RetryPersistence)
            .on_dismiss(Msg::HidePersistenceAlert),
        );
    }
    // Acknowledgement releases retained Save As custody and its close veto without
    // claiming destination cleanup.
    if let Some(debt) = model.arrangement_uncertain_publication.as_ref() {
        notices.push(
            EnzNotice::new("reader-arrangement-uncertain", debt.message.clone(), ToastTone::Error)
                .description("An arranged PDF publication finished with uncertain custody. Verify the destination, then Dismiss to acknowledge the write; leave the warning to keep the window open.")
                .on_dismiss(Msg::DismissArrangementUncertainPublication),
        );
    }
    // Acknowledgement abandons retained output descriptors without claiming removal of
    // published files.
    if let Some(debt) = model.extract_uncertain_publication.as_ref() {
        notices.push(
            EnzNotice::new("reader-extract-uncertain", debt.message.clone(), ToastTone::Error)
                .description("Retained extract/split output custody. Verify the outputs, then Dismiss to abandon the retained handles; Dismiss does not clean up the on-disk files. Leave the warning to keep the window open.")
                .on_dismiss(Msg::DismissExtractUncertainPublication),
        );
    }
    // Dismiss hides the warning without settling uncertainty; Retry reconciles retained
    // custody and unresolved debt vetoes close.
    if let Some(message) = model.pdf_mutation_uncertain_summary.as_deref() {
        notices.push(
            EnzNotice::new("reader-pdf-uncertain", message, ToastTone::Error)
                .description("The PDF on disk may already have changed. Retry to reconcile; Dismiss to close the warning and mark the file for reconciliation on next open.")
                .action("Retry", Msg::RetryPdfMutationUncertain)
                .on_dismiss(Msg::DismissPdfMutationUncertainWarning),
        );
    }
    // Open failures show on the welcome screen or with no pages loaded.
    if let Some(error) = model.error.as_deref()
        && (model.screen != Screen::Reading || model.page_extents.is_empty())
    {
        notices.push(
            EnzNotice::new("reader-error", error, ToastTone::Error).on_dismiss(Msg::DismissError),
        );
    }
    if let Some(failure) = model.picker_failure.as_deref() {
        notices.push(
            EnzNotice::new("reader-picker-failure", failure, ToastTone::Error)
                .on_dismiss(Msg::DismissPickerFailure),
        );
    }
    notices
}

pub(crate) fn recovery_panel<'a>(
    authority: &'a crate::plumbing::persistence::RecoveryAuthority,
    revealed: Option<&'a std::path::Path>,
    theme: &'a EnzTheme,
) -> Element<'a, Msg> {
    let origin = authority.origin.clone();
    let mut actions = row![
        enz_button("Retry", Msg::RetryStateRecovery(origin.clone()))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .into_element(),
        enz_button(
            "Open recovery location",
            Msg::RevealRecoveryLocation(origin.clone()),
        )
        .kind(ButtonKind::Secondary)
        .size(ButtonSize::Sm)
        .into_element(),
    ]
    .spacing(theme.spacing.xs());
    if authority.can_start_empty() {
        actions = actions.push(
            enz_button("Start empty", Msg::StartEmptyState(origin))
                .kind(ButtonKind::Destructive)
                .size(ButtonSize::Sm)
                .into_element(),
        );
    }
    let mut content = column![
        enz_banner(authority.title().to_string())
            .body(authority.message())
            .tone(BannerTone::Danger)
            .dismissible(false)
            .id_prefix(match &authority.origin {
                crate::plumbing::persistence::StateOrigin::Library => "reader-library-recovery",
                crate::plumbing::persistence::StateOrigin::Annotations { .. } => {
                    "reader-annotations-recovery"
                }
            })
            .into_element(),
        actions,
    ]
    .spacing(theme.spacing.xs());
    if let Some(path) = revealed {
        content = content.push(
            enz_text(path.display().to_string())
                .size(TextSize::Caption)
                .tone(TextTone::Muted),
        );
    }
    container(content)
        .padding(theme.spacing.sm())
        .width(Length::Fill)
        .into()
}
