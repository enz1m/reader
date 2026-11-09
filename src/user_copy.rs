use std::path::Path;

use crate::plumbing::pdf_ops::PdfWriteError;

const MAX_PATH_NAME_CHARS: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PdfWriteOperation {
    Split,
    Extract,
    SaveMetadata,
    ClearMetadata,
}

impl PdfWriteOperation {
    fn action(self) -> &'static str {
        match self {
            Self::Split => "split the PDF",
            Self::Extract => "extract the PDF page",
            Self::SaveMetadata => "save PDF metadata",
            Self::ClearMetadata => "clear PDF metadata",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationPhase {
    BeforePublication,
    PublishedUncertain,
}

#[derive(Debug)]
pub(crate) struct PdfWriteFailure {
    operation: PdfWriteOperation,
    phase: PublicationPhase,
    completed_outputs: usize,
    diagnostic: String,
}

impl PdfWriteFailure {
    pub(crate) fn new(operation: PdfWriteOperation, error: PdfWriteError) -> Self {
        let phase = if error.was_published() {
            PublicationPhase::PublishedUncertain
        } else {
            PublicationPhase::BeforePublication
        };
        let completed_outputs = error.completed_outputs();
        Self {
            operation,
            phase,
            completed_outputs,
            diagnostic: error.to_string(),
        }
    }

    pub(crate) fn user_summary(&self) -> String {
        match self.phase {
            PublicationPhase::BeforePublication => format!(
                "Could not {}. No output was published.",
                self.operation.action()
            ),
            PublicationPhase::PublishedUncertain if self.completed_outputs != 0 => format!(
                "Publication occurred for {} split file(s), but current contents or durability could not be confirmed.",
                self.completed_outputs
            ),
            PublicationPhase::PublishedUncertain => format!(
                "PDF publication occurred while trying to {}, but current contents or durability could not be confirmed.",
                self.operation.action()
            ),
        }
    }

    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

pub(crate) fn path_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{061c}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return "this file".to_string();
    }
    if cleaned.chars().count() <= MAX_PATH_NAME_CHARS {
        return cleaned.to_string();
    }
    let mut bounded: String = cleaned.chars().take(MAX_PATH_NAME_CHARS - 1).collect();
    bounded.push('\u{2026}');
    bounded
}
