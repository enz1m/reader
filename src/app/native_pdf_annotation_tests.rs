use super::*;
use enzui_iced_core::modal::DismissCause;
use enzui_iced_widgets::confirm_dialog::ConfirmDialogEvent;

fn tmpdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "reader-native-annotation-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open_pdf(label: &str) -> (App, std::path::PathBuf, std::path::PathBuf) {
    let dir = tmpdir(label);
    let path = dir.join("document.pdf");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf"),
        &path,
    )
    .unwrap();
    let render = crate::pdf::render_pdf(&path.to_string_lossy(), None).unwrap();
    let (mut app, _) = App::test_new();
    app.model.file_path = Some(path.clone());
    app.model.file_name = "document.pdf".to_string();
    app.model.format = Format::Pdf;
    app.model.screen = Screen::Reading;
    let _ = app.finish_pages(
        path.clone(),
        render.pages,
        render.total_pages,
        render.extents,
        render.text,
    );
    let scan = app.request_native_pdf_annotations();
    for message in crate::app::drain_task(scan) {
        app.drive_pdf_mutation(message);
    }
    assert!(
        app.model.pdf_annotation_snapshot.is_some(),
        "the native annotation scan landed"
    );
    app.model.undo_stack.clear();
    (app, path, dir)
}

fn select_first_line(app: &mut App, page: usize) -> model::PdfSelection {
    let path = app.model.file_path.clone().unwrap();
    app.model.pdf_text = crate::pdf::page_text(&path.to_string_lossy(), None);
    let first = app.model.pdf_text[page].lines[0].text.chars().count();
    let _ = app.update(Msg::PdfTextSelected {
        page,
        range: 0..first,
        finished: true,
    });
    app.model.pdf_selection.clone().unwrap()
}

#[test]
fn native_highlights_preserve_source_coordinates_and_survive_reopen() {
    {
        let (mut app, path, dir) = open_pdf("create");
        let selection = select_first_line(&mut app, 0);
        let raster_before = app.model.pages.get(0).map(|page| page.handle.id());

        app.drive_pdf_mutation(Msg::AddPdfHighlight);

        let native = crate::plumbing::pdf_ops::read_annotations(&path, 0).unwrap();
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].subtype, "Highlight");
        assert_eq!(native[0].contents, "");
        assert_eq!(app.model.annotations.annotations.len(), 1);
        assert_eq!(app.model.annotations.annotations[0].text, selection.text);
        assert!(app.model.annotations.annotations[0].pdf_binding.is_some());
        let raster_after = app.model.pages.get(0).map(|page| page.handle.id());
        assert!(
            raster_after.is_some() && raster_after != raster_before,
            "the changed page must be rerendered: before {raster_before:?}, after {raster_after:?}"
        );
        assert_eq!(
            app.model.pdf_annotation_states[&app.model.annotations.annotations[0].id],
            model::PdfAnnotationState::Verified
        );
        assert_eq!(
            app.model.pdf_overlay_annotations[&0].len(),
            1,
            "the verified sidecar geometry must reach the shared page overlay"
        );
        let id = app.model.annotations.annotations[0].id;
        let _ = app.update(Msg::GoToAnnotation(id));
        assert_eq!(app.model.selected_annotation, Some(id));

        let snapshot = crate::plumbing::pdf_ops::read_document_annotations(&path).unwrap();
        app.reconcile_native_pdf_annotations(snapshot);
        assert_eq!(
            app.model.annotations.annotations.len(),
            1,
            "re-importing the native annotation must not duplicate its sidecar row"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let dir = tmpdir("mid-line");
        let path = dir.join("lines.pdf");
        crate::perf_tests::build_pdf(&path, 1, None);
        let render = crate::pdf::render_pdf(&path.to_string_lossy(), None).unwrap();
        let (mut app, _) = App::test_new();
        app.model.file_path = Some(path.clone());
        app.model.file_name = "lines.pdf".to_string();
        app.model.format = Format::Pdf;
        app.model.screen = Screen::Reading;
        let _ = app.finish_pages(
            path.clone(),
            render.pages,
            render.total_pages,
            render.extents,
            render.text,
        );
        let scan = app.request_native_pdf_annotations();
        for message in crate::app::drain_task(scan) {
            app.drive_pdf_mutation(message);
        }

        let glyphs = &app.model.pdf_text[0].glyphs;
        let chars: String = glyphs.glyphs().iter().map(|g| g.ch).collect();
        let char_index = |needle: &str| {
            let byte = chars
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} on the page"));
            chars[..byte].chars().count()
        };
        let start = char_index("line 3: the ") + "line 3: the ".len();
        let end = char_index("line 5: the quick brown") + "line 5: the quick brown".len();
        let _ = app.update(Msg::PdfTextSelected {
            page: 0,
            range: start..end,
            finished: true,
        });
        let selection = app.model.pdf_selection.clone().expect("selected");
        assert!(selection.text.starts_with("quick"), "{:?}", selection.text);
        assert!(selection.text.ends_with("brown"), "{:?}", selection.text);
        assert_eq!(selection.quads.len(), 3, "lines 3, 4 and 5");
        let line3 = &app.model.pdf_text[0].lines[3];
        assert!(
            selection.quads[0].x0 > line3.rect[0] + 20.0,
            "the first quad starts at 'quick', not at the line start: {:?} vs {:?}",
            selection.quads[0],
            line3.rect
        );
        assert!(
            selection.quads[2].x1 < line3.rect[2] - 20.0,
            "the last quad ends at 'brown': {:?}",
            selection.quads[2]
        );

        app.drive_pdf_mutation(Msg::AddPdfHighlight);
        let document = lopdf::Document::load(&path).unwrap();
        let page = *document.get_pages().get(&1).unwrap();
        let annots = document
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .and_then(lopdf::Object::as_array)
            .unwrap()
            .clone();
        let highlight = annots
            .iter()
            .filter_map(|a| a.as_reference().ok())
            .filter_map(|id| document.get_dictionary(id).ok())
            .find(|d| d.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Highlight"))
            .expect("a highlight was written");
        let quads = highlight
            .get(b"QuadPoints")
            .and_then(lopdf::Object::as_array)
            .unwrap();
        assert_eq!(quads.len(), 3 * 8, "eight numbers per line quad");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn native_comments_and_deletion_roundtrip_through_undo() {
    {
        let (mut app, path, dir) = open_pdf("comment-edit");
        select_first_line(&mut app, 0);
        let _ = app.update(Msg::AddPdfComment);
        let _ = app.type_into_note_composer_for_test("Original comment");
        app.drive_pdf_mutation(Msg::CommitNoteEditor);
        let id = app.model.annotations.annotations[0].id;
        app.model.undo_stack.clear();

        let _ = app.update(Msg::OpenNoteEditorForEdit(id));
        app.model.note_editor = Some(enzui_iced_widgets::form_dialog::FormPresenter::new(
            crate::view::note_editor::note_form_model("Revised comment"),
        ));
        app.drive_pdf_mutation(Msg::CommitNoteEditor);
        assert_eq!(app.model.annotations.annotations[0].text, "Revised comment");
        assert_eq!(
            crate::plumbing::pdf_ops::read_annotations(&path, 0).unwrap()[0].contents,
            "Revised comment"
        );

        app.drive_pdf_mutation(Msg::Undo);
        assert_eq!(
            app.model.annotations.annotations[0].text,
            "Original comment"
        );
        assert_eq!(
            crate::plumbing::pdf_ops::read_annotations(&path, 0).unwrap()[0].contents,
            "Original comment"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let (mut app, path, dir) = open_pdf("delete-undo");
        select_first_line(&mut app, 0);
        app.drive_pdf_mutation(Msg::AddPdfHighlight);
        let id = app.model.annotations.annotations[0].id;

        let _ = app.update(Msg::RemoveAnnotation(id));
        assert_eq!(
            app.model.pending_page_action,
            Some(model::PendingPageAction::DeleteAnnotation(id))
        );
        assert_eq!(app.model.annotations.annotations.len(), 1);
        assert_eq!(
            crate::plumbing::pdf_ops::read_annotations(&path, 0)
                .unwrap()
                .len(),
            1
        );
        app.drive_pdf_mutation(Msg::PageConfirmEvent(ConfirmDialogEvent::Resolved(
            DismissCause::CancelAction,
        )));
        assert_eq!(app.model.annotations.annotations.len(), 1);
        assert_eq!(
            crate::plumbing::pdf_ops::read_annotations(&path, 0)
                .unwrap()
                .len(),
            1,
            "declining leaves both stores untouched"
        );

        let _ = app.update(Msg::RemoveAnnotation(id));
        app.drive_pdf_mutation(Msg::PageConfirmEvent(ConfirmDialogEvent::Resolved(
            DismissCause::DefaultAction,
        )));
        assert!(app.model.annotations.annotations.is_empty());
        assert!(
            crate::plumbing::pdf_ops::read_annotations(&path, 0)
                .unwrap()
                .is_empty()
        );

        app.drive_pdf_mutation(Msg::Undo);
        assert_eq!(app.model.annotations.annotations.len(), 1);
        let restored = crate::plumbing::pdf_ops::read_annotations(&path, 0).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored[0].native_id,
            app.model.annotations.annotations[0]
                .pdf_binding
                .as_ref()
                .unwrap()
                .native_id
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
