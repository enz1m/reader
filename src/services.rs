//! All I/O: document loading, library state, format detection.
//!
//! Nothing here renders; `update` calls these off-thread via `Task::perform`
//! and folds the result back into the model.

use std::path::Path;

use crate::model::Format;

/// Detect the document format from the file extension.
pub fn detect_format(path: &Path) -> Option<Format> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    match ext.as_str() {
        "pdf" => Some(Format::Pdf),
        "epub" => Some(Format::Epub),
        "cbz" => Some(Format::Cbz),
        _ => None,
    }
}

/// Human file name for the titlebar.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

pub(crate) fn document_from_launch(
    intent: enzui_iced_core::activation::ApplicationIntent,
) -> Result<Option<std::path::PathBuf>, String> {
    use enzui_iced_core::activation::ApplicationIntent;
    match intent {
        ApplicationIntent::Activate => Ok(None),
        ApplicationIntent::Open(uris) if uris.len() == 1 => {
            enzui_iced_widgets::file_chooser::FileUri::new(uris[0].clone())
                .to_path()
                .map(Some)
                .map_err(|_| "That location is not a local file.".into())
        }
        ApplicationIntent::Open(_) => Err("Open one local document at a time.".into()),
        ApplicationIntent::ActivateAction { .. } => {
            Err("This desktop action is not supported.".into())
        }
    }
}

#[cfg(test)]
mod launch_tests {
    use super::*;
    use enzui_iced_core::activation::ApplicationIntent;

    #[test]
    fn launches_decode_local_uris_and_reject_remote_or_ambiguous_requests() {
        assert_eq!(
            document_from_launch(ApplicationIntent::Open(vec![
                "file:///tmp/Annual%20report.epub".into()
            ]))
            .unwrap(),
            Some("/tmp/Annual report.epub".into())
        );
        assert!(
            document_from_launch(ApplicationIntent::Open(vec![
                "file://remote/book.pdf".into()
            ]))
            .is_err()
        );
        assert!(
            document_from_launch(ApplicationIntent::Open(vec![
                "https://example.com/book.pdf".into()
            ]))
            .is_err()
        );
        assert!(
            document_from_launch(ApplicationIntent::Open(vec![
                "file:///tmp/a.pdf".into(),
                "file:///tmp/b.pdf".into()
            ]))
            .is_err()
        );
    }
}
