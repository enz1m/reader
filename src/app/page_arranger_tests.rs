use super::*;
use crate::message::Msg;
use crate::model::{Format, PendingPageAction, Screen};
use crate::pages::PageSource;
use crate::plumbing::pdf_ops;
use enzui_iced_core::modal::DismissCause;
use enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent;

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-arr-{}-{}-{:?}",
        label,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf")
}

fn app_in_arranger(dir: &std::path::Path) -> (App, std::path::PathBuf) {
    let pdf = dir.join("doc.pdf");
    std::fs::copy(fixture(), &pdf).unwrap();
    let (mut app, _t) = App::test_new();
    app.model.file_path = Some(pdf.clone());
    app.model.file_name = "doc.pdf".to_string();
    app.model.format = Format::Pdf;
    app.model.screen = Screen::Reading;
    app.model.page_count = 5;
    app.model.total_page_count = 5;
    app.load_annotations_for_document(&pdf);
    let _ = app.update(Msg::OpenPageArranger);
    (app, pdf)
}

fn labels(app: &App) -> Vec<String> {
    app.model
        .arrangement
        .entries()
        .iter()
        .map(|e| e.label.clone())
        .collect()
}

#[test]
fn page_merge_and_delete_are_reversible() {
    {
        let dir = tmpdir("confirm-del");
        let (mut app, _p) = app_in_arranger(&dir);

        let victim = app.model.arrangement.entries()[1].clone();
        let _ = app.update(Msg::RequestDeletePage(victim.id));
        let _ = app.update(Msg::PageConfirmEvent(ConfirmDialogEvent::Resolved(
            DismissCause::DefaultAction,
        )));
        assert_eq!(
            labels(&app),
            vec!["Page 1", "Page 3", "Page 4", "Page 5"],
            "exactly page 2 is gone"
        );
        assert_eq!(
            crate::live_feedback::announced(&app).as_deref(),
            Some("Deleted Page 2"),
        );

        let _ = app.update(Msg::Undo);
        assert_eq!(app.model.arrangement.len(), 5, "count restored");
        let restored = &app.model.arrangement.entries()[1];
        assert_eq!(restored.id, victim.id, "the SAME page came back, by id");
        assert_eq!(restored.label, victim.label);
        assert_eq!(restored.source, PageSource::Original(1));
        assert_eq!(
            labels(&app),
            vec!["Page 1", "Page 2", "Page 3", "Page 4", "Page 5"],
            "and at the position it was removed from"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        let dir = tmpdir("merge");
        let (mut app, _p) = app_in_arranger(&dir);
        let other = dir.join("appendix.pdf");
        std::fs::copy(fixture(), &other).unwrap();

        let _ = app.update(Msg::RequestMergePdf);
        let _ = app.update(Msg::MergePathChanged(other.display().to_string()));
        let _ = app.update(Msg::MergePdf);

        assert_eq!(app.model.arrangement.len(), 10, "5 + 5");
        assert_eq!(
            crate::live_feedback::announced(&app).as_deref(),
            Some("Merged 5 pages from appendix.pdf"),
        );
        match &app.model.arrangement.entries()[5].source {
            PageSource::Merged {
                file,
                index,
                document_fingerprint,
            } => {
                assert_eq!(file.as_str(), other.to_string_lossy().as_ref());
                assert_eq!(*index, 0);
                assert!(!document_fingerprint.is_empty());
            }
            other => panic!("expected merged source, got {other:?}"),
        }
        assert!(
            app.model.merge_path.is_empty(),
            "the field clears after a successful merge"
        );

        let _ = app.update(Msg::Undo);
        assert_eq!(app.model.arrangement.len(), 5, "undo removes the merge");
        assert!(app.model.arrangement.is_identity(5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn ids(app: &App) -> Vec<u64> {
    app.model
        .arrangement
        .entries()
        .iter()
        .map(|e| e.id)
        .collect()
}

fn confirm(app: &mut App) {
    let _ = app.update(Msg::PageConfirmEvent(ConfirmDialogEvent::Resolved(
        DismissCause::DefaultAction,
    )));
}

#[test]
fn bulk_delete_requires_confirmation_and_extract_preserves_the_source() {
    {
        let dir = tmpdir("grid-delete");
        let (mut app, _p) = app_in_arranger(&dir);
        let all = ids(&app);
        let _ = app.update(Msg::RequestDeletePages(vec![all[1], all[3]]));
        assert_eq!(
            app.model.pending_page_action,
            Some(PendingPageAction::DeletePages(vec![all[1], all[3]])),
            "it asks first"
        );
        confirm(&mut app);
        assert_eq!(labels(&app), vec!["Page 1", "Page 3", "Page 5"]);

        let _ = app.update(Msg::RequestDeletePages(ids(&app)));
        assert_eq!(app.model.pending_page_action, None, "every page: refused");
        assert_eq!(labels(&app).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
    {
        let dir = tmpdir("grid-extract");
        let (mut app, pdf) = app_in_arranger(&dir);
        let all = ids(&app);
        let _ = app.update(Msg::RequestExtractPages(vec![all[4], all[1]]));
        confirm(&mut app);
        let out = dir.join("doc-pages-2-5.pdf");
        assert!(out.exists(), "wrote {out:?}");
        assert_eq!(pdf_ops::page_count(&out).unwrap(), 2);
        assert_eq!(
            pdf_ops::page_count(&pdf).unwrap(),
            5,
            "the source is unchanged"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
