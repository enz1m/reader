//! Worker, persistence and bounded-residency workflow fixtures.

use crate::app::App;
use crate::message::Msg;
use crate::plumbing::epub_support::{EpubChapter, EpubLoadResult, EpubMetadata};
use crate::plumbing::persistence::{self, AppState, ProgressRecord, set_progress};

use iced::Size;

const W: f32 = 1400.0;
const H: f32 = 900.0;

const LIBRARY_BOOKS: usize = 120;

const PDF_PAGES: usize = 60;

const PDF_PASSWORD: &str = "perf-1405";

pub(crate) fn build_pdf(path: &std::path::Path, pages: usize, password: Option<&str>) {
    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;
    use lopdf::{Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_ids: Vec<Object> = Vec::with_capacity(pages);
    for p in 0..pages {
        let mut ops = vec![Operation::new("BT", vec![])];
        ops.push(Operation::new("Tf", vec!["F1".into(), 11.into()]));
        ops.push(Operation::new("Td", vec![72.into(), 720.into()]));
        ops.push(Operation::new("TL", vec![14.into()]));
        for line in 0..44 {
            let text = format!(
                "Page {p} line {line}: the quick brown fox jumps over the lazy dog, again."
            );
            ops.push(Operation::new("Tj", vec![Object::string_literal(text)]));
            ops.push(Operation::new("T*", vec![]));
        }
        ops.push(Operation::new("ET", vec![]));
        let content = Content { operations: ops };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode page content"),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        page_ids.push(page_id.into());
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => pages as i64,
            "Kids" => page_ids,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    // Encryption keys derive from the trailer's file ID; without one
    // `EncryptionState::try_from` fails with `MissingFileID`.
    let file_id = Object::String(
        b"1405perffixture!".to_vec(),
        lopdf::StringFormat::Hexadecimal,
    );
    doc.trailer.set("ID", vec![file_id.clone(), file_id]);
    doc.compress();

    if let Some(password) = password {
        use lopdf::encryption::{EncryptionState, EncryptionVersion};
        let version = EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner-1405",
            user_password: password,
            key_length: 128,
            permissions: lopdf::encryption::Permissions::all(),
        };
        let state = EncryptionState::try_from(version).expect("encryption state");
        doc.encrypt(&state).expect("encrypt fixture");
    }

    doc.save(path).expect("save fixture pdf");
}

pub(crate) const LONG_PDF_PAGES: usize = 400;

pub(crate) fn long_pdf_fixture() -> std::path::PathBuf {
    pdf_fixture(LONG_PDF_PAGES)
}

pub(crate) fn pdf_fixture(pages: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("reader-1634-fixtures");
    std::fs::create_dir_all(&dir).expect("fixture sandbox");
    let path = dir.join(format!("long-{pages}.pdf"));
    if !path.exists() {
        let staging = dir.join(format!(
            "long-{pages}.{}.{:?}.tmp",
            std::process::id(),
            std::thread::current().id()
        ));
        build_pdf(&staging, pages, None);
        std::fs::rename(&staging, &path).expect("publish fixture atomically");
    }
    path
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-perf-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn heavy_state(dir: &std::path::Path) -> AppState {
    let mut state = AppState::default();
    for i in 0..LIBRARY_BOOKS {
        let path = dir.join(format!("book-{i:03}.epub"));
        std::fs::write(&path, b"not really an epub").unwrap();
        set_progress(
            &mut state,
            &path.to_string_lossy(),
            ProgressRecord {
                page: i % 20,
                total: 20,
                title: format!("Book {i:03}"),
                author: "A. Reader".into(),
                format: "epub".into(),
                last_opened: format!("2026-01-{:02}T00:00:00Z", (i % 28) + 1),
                ..Default::default()
            },
        );
    }
    state
}

fn synthetic_epub(chapters: usize) -> EpubLoadResult {
    EpubLoadResult {
        metadata: EpubMetadata {
            title: "The Heavy Book".into(),
            author: "A. Reader".into(),
            language: "en".into(),
            description: String::new(),
            publisher: String::new(),
        },
        chapters: (0..chapters)
            .map(|index| EpubChapter {
                title: format!("Chapter {index}"),
                heading: String::new(),
                content_html: format!("<p>{}</p>", "Body text. ".repeat(400)),
                plain_text: "Body text. ".repeat(400),
                index,
            })
            .collect(),
        toc: Vec::new(),
        footnotes: Vec::new(),
        footnote_refs: Vec::new(),
    }
}

pub(crate) fn heavy_app(dir: &std::path::Path) -> App {
    let (mut app, _t) = App::test_new();
    app.seed_app_state_for_test(heavy_state(dir));
    let book = dir.join("book-000.epub");
    app.open_epub_for_test(&book, synthetic_epub(24));
    app
}

mod flush_tests;
mod raster_tests;

fn persistence_state(entries: usize) -> AppState {
    let mut state = AppState::default();
    for i in 0..entries {
        let path = format!("/library/shelf-{i:05}/book-{i:05}.epub");
        set_progress(
            &mut state,
            &path,
            ProgressRecord {
                page: i % 913,
                total: 913,
                epub_font_size: Some(18),
                epub_line_spacing: Some(1.6),
                epub_dark_theme: Some(false),
                epub_theme: Some("sepia".into()),
                last_opened: format!("2026-08-{:02}T12:34:56Z", (i % 28) + 1),
                title: format!("A Realistically Named Book {i:05}"),
                author: format!("Author {i:05}"),
                format: "epub".into(),
                epub_view_mode: Some("paginated".into()),
                epub_anchor: Some(crate::plumbing::persistence::EpubProgressAnchor {
                    paragraph: i % 17,
                    char_offset: i % 113,
                }),
                reading_time_secs: Some((i as u64 + 1) * 73),
            },
        );
    }
    state
}

fn state_file(resolver: &enzui_iced_core::persist::XdgResolver) -> std::path::PathBuf {
    persistence::state_path_with_resolver(resolver)
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

fn open_long_pdf(app: &mut App) {
    let path = long_pdf_fixture();
    let _ = app.update(Msg::WindowResized(Size::new(W, H)));
    let mut pending = vec![Msg::OpenPath(path)];
    for _ in 0..8 {
        if pending.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for message in pending.drain(..) {
            next.extend(crate::app::drain_task(app.update(message)));
        }
        pending = next;
    }
    assert_eq!(
        app.model().page_count,
        LONG_PDF_PAGES,
        "the {LONG_PDF_PAGES}-page fixture must open at its real length"
    );
}

fn jump_and_settle(app: &mut App, target: usize) {
    for message in crate::app::drain_task(app.update(Msg::GoToPage(target))) {
        let _ = app.update(message);
    }
}
