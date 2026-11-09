use super::*;
use std::io::{Cursor, Write};

fn png(value: u8) -> Vec<u8> {
    let pixels = image::RgbaImage::from_pixel(
        3,
        4,
        image::Rgba([value, value.wrapping_add(1), value.wrapping_add(2), 255]),
    );
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn cbz(entries: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("comic.cbz");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
    (dir, path)
}

fn settle(app: &mut App, task: Task<Msg>) {
    for message in drain_task(task) {
        drop(app.update(message));
    }
}

#[test]
fn comic_open_preserves_order_progress_and_rejects_unsafe_archives() {
    {
        let (_dir, path) = cbz(&[
            ("ComicInfo.xml", b"<comic/>".to_vec()),
            ("page10.png", png(10)),
            (".DS_Store", b"finder metadata".to_vec()),
            ("page2.png", png(2)),
            ("page1.png", png(1)),
        ]);
        let (mut app, _task) = App::test_new();
        let task = app.open(path.clone());
        assert!(app.model.loading);
        settle(&mut app, task);

        assert_eq!(app.model.screen, Screen::Reading);
        assert_eq!(app.model.format, Format::Cbz);
        assert_eq!(app.model.page_count, 3);
        assert_eq!(app.model.pages.len(), 1);
        assert_eq!(
            app.model
                .cbz_entries
                .iter()
                .map(|entry| entry.archive_name.as_str())
                .collect::<Vec<_>>(),
            ["page1.png", "page2.png", "page10.png"]
        );
        assert_eq!(
            app.model.page_extents[0],
            crate::raster::PageExtent {
                width: 3,
                height: 4
            }
        );
        assert_eq!(app.model.sidebar_tab, model::SidebarTab::Bookmarks);
        assert!(app.model.error.is_none());

        let page_task = app.update(Msg::GoToPage(2));
        settle(&mut app, page_task);
        assert!(app.model.pages.contains(2));
        drop(app.update(Msg::ToggleCurrentBookmark));
        drop(app.update(Msg::GoToPage(0)));
        drop(app.update(Msg::GoToBookmark {
            format: Format::Cbz,
            document_key: crate::plumbing::persistence::canonical_key(&path.to_string_lossy()),
            position: 2,
        }));
        assert_eq!(app.model.current_page, 2);
        assert!(
            app.model
                .annotations
                .bookmarks
                .iter()
                .any(|mark| mark.page == 2)
        );

        let key = persistence::canonical_key(&path.to_string_lossy());
        let record = &app.model.app_state.files[&key];
        assert_eq!(record.format, "cbz");
        assert_eq!(record.page, 2);

        let saved_state = (*app.model.app_state).clone();
        let (mut reopened, _task) = App::test_new();
        reopened.seed_app_state_for_test(saved_state);
        let task = reopened.open(path);
        settle(&mut reopened, task);
        assert_eq!(reopened.model.format, Format::Cbz);
        assert_eq!(reopened.model.current_page, 2);
    }
    {
        let (_dir, path) = cbz(&[("../escape.png", png(1))]);
        let (mut app, _task) = App::test_new();
        let task = app.open(path);
        settle(&mut app, task);

        assert_eq!(app.model.screen, Screen::Library);
        assert_eq!(app.model.page_count, 0);
        assert!(!app.model.loading);
        assert!(
            app.model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("unsafe path"))
        );
    }
}
