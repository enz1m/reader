use super::*;
use crate::message::Msg;
use crate::model::{Format, Overlay, Screen};
use crate::pages::PageArrangement;
use crate::plumbing::pdf_ops::{self, PdfAnnotationKind, PdfRect};

fn tmpdir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "reader-arrangement-publication-{label}-{}-{:?}",
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

fn prepared_app(source: &std::path::Path) -> App {
    let (mut app, _) = App::test_new();
    let snapshot = pdf_ops::read_document_annotations(source).unwrap();
    app.model.file_path = Some(source.to_path_buf());
    app.model.file_name = "source.pdf".into();
    app.model.format = Format::Pdf;
    app.model.screen = Screen::Reading;
    app.model.page_count = snapshot.pages.len();
    app.model.total_page_count = snapshot.pages.len();
    app.model.arrangement = PageArrangement::from_page_count(snapshot.pages.len());
    app.model.pdf_annotation_snapshot = Some(snapshot);
    app.model.overlay = Some(Overlay::PageArranger);
    app
}

#[test]
fn published_arrangements_preserve_page_content_rotation_and_annotations() {
    {
        let dir = tmpdir("roundtrip");
        let source = dir.join("source.pdf");
        let destination = dir.join("published.pdf");
        std::fs::copy(fixture(), &source).unwrap();
        pdf_ops::write_annotation_with_id(
            &source,
            1,
            PdfAnnotationKind::Comment,
            PdfRect::new(40.0, 40.0, 60.0, 60.0),
            "surviving comment",
            "yellow",
            Some("stable-survivor"),
        )
        .unwrap();

        let snapshot = pdf_ops::read_document_annotations(&source).unwrap();
        let mut arrangement = PageArrangement::from_page_count(5);
        let second = arrangement.entries()[1].id;
        arrangement.move_up(second);
        arrangement.rotate(second, true);

        let publication = pdf_ops::publish_arrangement(
            &source,
            &snapshot.document_fingerprint,
            arrangement.entries(),
            &destination,
        )
        .unwrap();

        assert_eq!(publication.page_count, 5);
        let original = lopdf::Document::load(&source).unwrap();
        let reopened = lopdf::Document::load(&destination).unwrap();
        assert_eq!(
            reopened.extract_text(&[1]).unwrap(),
            original.extract_text(&[2]).unwrap()
        );
        assert_eq!(
            reopened.extract_text(&[2]).unwrap(),
            original.extract_text(&[1]).unwrap()
        );
        assert_eq!(
            publication.annotation_snapshot.pages[0]
                .rotation_degrees
                .rem_euclid(360),
            (snapshot.pages[1].rotation_degrees + 90).rem_euclid(360)
        );
        let native = publication
            .annotation_snapshot
            .annotations
            .iter()
            .find(|annotation| annotation.native_id == "stable-survivor")
            .unwrap();
        assert_eq!(native.page, 0);
        assert_eq!(native.contents, "surviving comment");
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let dir = tmpdir("merge-all");
        let source = dir.join("source.pdf");
        let merged = dir.join("appendix.pdf");
        let destination = dir.join("published.pdf");
        std::fs::copy(fixture(), &source).unwrap();
        std::fs::copy(fixture(), &merged).unwrap();
        pdf_ops::write_annotation_with_id(
            &merged,
            4,
            PdfAnnotationKind::Comment,
            PdfRect::new(30.0, 30.0, 50.0, 50.0),
            "last merged page",
            "yellow",
            Some("merged-last"),
        )
        .unwrap();
        let source_identity = pdf_ops::read_document_annotations(&source).unwrap();
        let merged_identity = pdf_ops::read_document_annotations(&merged).unwrap();
        let mut arrangement = PageArrangement::from_page_count(5);
        arrangement.append_merged(
            &merged.to_string_lossy(),
            5,
            &merged_identity.document_fingerprint,
        );

        let publication = pdf_ops::publish_arrangement(
            &source,
            &source_identity.document_fingerprint,
            arrangement.entries(),
            &destination,
        )
        .unwrap();

        assert_eq!(publication.page_count, 10);
        assert!(
            publication
                .annotation_snapshot
                .annotations
                .iter()
                .any(|annotation| annotation.native_id == "merged-last" && annotation.page == 9)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn failed_arrangement_publication_retains_draft_and_reports_uncertainty() {
    {
        let dir = tmpdir("write-failure");
        let source = dir.join("source.pdf");
        let destination = dir.join("published.pdf");
        std::fs::copy(fixture(), &source).unwrap();
        let source_bytes = std::fs::read(&source).unwrap();
        let identity = pdf_ops::read_document_annotations(&source).unwrap();
        let mut arrangement = PageArrangement::from_page_count(5);
        let first = arrangement.entries()[0].id;
        arrangement.move_down(first);
        let draft = arrangement.clone();
        pdf_ops::save_fault::fail_with_detail("injected write failure");

        let result = pdf_ops::publish_arrangement(
            &source,
            &identity.document_fingerprint,
            arrangement.entries(),
            &destination,
        );

        let error = result.unwrap_err();
        assert!(!error.was_published());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(arrangement, draft);
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let dir = tmpdir("sync-failure");
        let source = dir.join("source.pdf");
        let destination = dir.join("published.pdf");
        std::fs::copy(fixture(), &source).unwrap();
        let mut app = prepared_app(&source);
        let first = app.model.arrangement.entries()[0].id;
        let _ = app.update(Msg::MovePageDown(first));
        let draft = app.model.arrangement.clone();
        let generation = 41;
        app.model.arrangement_publish_generation = generation;
        app.model.arrangement_publication = Some(crate::model::PageArrangementPublication {
            generation,
            source: source.clone(),
            destination: destination.clone(),
            arrangement: draft.clone(),
        });
        pdf_ops::save_fault::arm_published_failure_with_detail("injected parent sync failure");
        let identity = app.model.pdf_annotation_snapshot.as_ref().unwrap();
        let result = pdf_ops::publish_arrangement(
            &source,
            &identity.document_fingerprint,
            draft.entries(),
            &destination,
        );

        let _ = app.update(Msg::PageArrangementPublished {
            generation,
            source: source.clone(),
            destination: destination.clone(),
            arrangement: draft.clone(),
            result,
        });

        assert!(destination.exists());
        assert_eq!(app.model.file_path.as_deref(), Some(source.as_path()));
        assert_eq!(app.model.arrangement, draft);
        assert!(
            app.model
                .arrangement_publish_failure
                .as_deref()
                .is_some_and(
                    |message| message.contains(destination.to_string_lossy().as_ref())
                        && message
                            .contains("source is unchanged and the reversible draft is retained")
                )
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
