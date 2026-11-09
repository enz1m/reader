//! Route controls for retained document authority in the merge, extract, Save As,
//! metadata and native-annotation paths.
//!
//! Verification reads through the retained `DocumentAuthority`, never the destination
//! pathname, so a substitution after publication cannot alter the bytes checked.

use super::*;
use crate::message::Msg;
use crate::model::{Format, Overlay, Screen};
use crate::pages::{PageArrangement, PageSource};
use crate::plumbing::pdf_ops;
use enzui_iced_core::modal::DismissCause;
use enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent;

fn tmpdir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "reader-retained-authority-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf")
}

fn app_in_arranger(dir: &std::path::Path) -> (App, std::path::PathBuf) {
    let pdf = dir.join("source.pdf");
    std::fs::copy(fixture(), &pdf).unwrap();
    let (mut app, _t) = App::test_new();
    let snapshot = pdf_ops::read_document_annotations(&pdf).unwrap();
    app.model.file_path = Some(pdf.clone());
    app.model.file_name = "source.pdf".to_string();
    app.model.format = Format::Pdf;
    app.model.screen = Screen::Reading;
    app.model.page_count = snapshot.pages.len();
    app.model.total_page_count = snapshot.pages.len();
    app.model.arrangement = PageArrangement::from_page_count(snapshot.pages.len());
    app.model.pdf_annotation_snapshot = Some(snapshot);
    app.model.overlay = Some(Overlay::PageArranger);
    (app, pdf)
}

// Gives the pre-publication identity check a fingerprint drift to refuse.
fn write_single_page_pdf(path: &std::path::Path) {
    use lopdf::{Document, Object, dictionary};
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "Resources" => dictionary! {},
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1_i64,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc.prune_objects();
    doc.renumber_objects();
    doc.save(path).unwrap();
}

// A single retained read makes the count and fingerprint describe the same file.
#[test]
fn publication_rejects_substituted_sources_and_existing_destinations() {
    {
        let dir = tmpdir("merge-substituted");
        let (mut app, source) = app_in_arranger(&dir);
        let appendix = dir.join("appendix.pdf");
        std::fs::copy(fixture(), &appendix).unwrap();

        let _ = app.update(Msg::MergePathChanged(appendix.display().to_string()));
        let _ = app.update(Msg::MergePdf);
        assert_eq!(app.model.arrangement.len(), 10, "5 original + 5 merged");
        let recorded_identity = match &app.model.arrangement.entries()[5].source {
            PageSource::Merged {
                file,
                index,
                document_fingerprint,
            } => {
                assert_eq!(file.as_str(), appendix.to_string_lossy().as_ref());
                assert_eq!(*index, 0);
                document_fingerprint.clone()
            }
            other => panic!("expected merged source, got {other:?}"),
        };
        assert!(!recorded_identity.is_empty());

        let _ = std::fs::remove_file(&appendix);
        write_single_page_pdf(&appendix);
        let substituted_identity = pdf_ops::read_document_annotations(&appendix)
            .unwrap()
            .document_fingerprint;
        assert_ne!(
            substituted_identity, recorded_identity,
            "substitution must produce a different retained fingerprint"
        );

        let destination = dir.join("published.pdf");
        let source_identity = app
            .model
            .pdf_annotation_snapshot
            .as_ref()
            .unwrap()
            .document_fingerprint
            .clone();
        let result = pdf_ops::publish_arrangement(
            &source,
            &source_identity,
            app.model.arrangement.entries(),
            &destination,
        );
        let error = result.unwrap_err();
        assert!(
            !error.was_published(),
            "no publication for a substituted merge source"
        );
        assert!(!destination.exists(), "no destination file was written");
        assert!(
            error
                .to_string()
                .contains(appendix.to_string_lossy().as_ref())
                && error.to_string().contains("changed after it was added"),
            "identity-drift refusal must name the substituted appendix: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        let dir = tmpdir("save-as-collision");
        let (mut app, source) = app_in_arranger(&dir);
        let destination = dir.join("published.pdf");
        std::fs::write(&destination, b"pre-existing file at Save As destination").unwrap();
        let pre_existing = std::fs::read(&destination).unwrap();

        let first = app.model.arrangement.entries()[0].id;
        let _ = app.update(Msg::MovePageDown(first));
        let draft = app.model.arrangement.clone();

        let identity = app
            .model
            .pdf_annotation_snapshot
            .as_ref()
            .unwrap()
            .document_fingerprint
            .clone();
        let source_bytes = std::fs::read(&source).unwrap();
        let result =
            pdf_ops::publish_arrangement(&source, &identity, draft.entries(), &destination);
        let error = result.unwrap_err();
        assert!(
            error.is_already_exists(),
            "refusal must be typed AlreadyExists"
        );
        assert!(!error.was_published(), "no publication for a collision");
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            pre_existing,
            "the pre-existing destination file was not truncated"
        );
        assert_eq!(
            std::fs::read(&source).unwrap(),
            source_bytes,
            "the source is unchanged when Save As refuses on collision"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn split_second_output_failure_retains_first_plus_uncertain_second_and_gates_close() {
    let dir = tmpdir("split-second-fault");
    let (mut app, source) = app_in_arranger(&dir);

    // The fault on call 2 moves the second output into `debt.uncertain`.
    pdf_ops::save_fault::arm_published_failure_on_call(
        2,
        "injected split second-output durability failure",
    );
    let _ = app.update(Msg::RequestSplitPdf);
    let _ = app.update(Msg::PageConfirmEvent(ConfirmDialogEvent::Resolved(
        DismissCause::DefaultAction,
    )));

    let debt = app
        .extract_publication_debt
        .as_ref()
        .expect("failing split installs a retained debt on App");
    assert_eq!(
        debt.completed.len(),
        1,
        "the first output is retained on debt.completed",
    );
    assert!(
        matches!(
            debt.uncertain.as_ref(),
            Some(pdf_ops::ArrangementUncertainAuthority::Durable(_)),
        ),
        "the failing second output is retained on debt.uncertain as Durable",
    );

    let summary = app
        .model
        .extract_uncertain_publication
        .as_ref()
        .expect("install populates the Model summary");
    assert_eq!(summary.completed_count, 1);
    assert!(
        matches!(
            summary.uncertain_kind,
            pdf_ops::ExtractUncertainKind::Durable
        ),
        "summary classifies the failing output as Durable",
    );
    let source_stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    assert!(
        !summary.message.contains(source.to_string_lossy().as_ref()),
        "summary must not expose the source pathname: {}",
        summary.message
    );
    assert!(
        !summary.message.contains(source_stem) || source_stem.is_empty(),
        "summary must not expose the source stem: {}",
        summary.message
    );
    assert!(
        summary.message.contains("retained output(s)"),
        "summary names the retained-output count: {}",
        summary.message
    );
    assert!(
        summary.message.contains("durability uncertain"),
        "summary names the failing output's kind: {}",
        summary.message
    );

    let veto = app.compose_close_veto_summary();
    assert_eq!(
        veto, summary.message,
        "close veto surfaces the paths-free summary"
    );

    let _ = app.update(Msg::DismissExtractUncertainPublication);
    assert!(
        app.extract_publication_debt.is_none(),
        "Dismiss drops the retained per-output descriptors"
    );
    assert!(
        app.model.extract_uncertain_publication.is_none(),
        "Dismiss clears the Model summary"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
