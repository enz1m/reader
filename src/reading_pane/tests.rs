use super::*;
use crate::plumbing::epub_support::InlineSpan;

fn colors() -> ReadingThemeColors {
    crate::model::ReadingTheme::Light.colors()
}

fn para(text: &str, spans: Vec<InlineSpan>) -> ReadingParagraph {
    ReadingParagraph {
        chapter: 0,
        para_index: 0,
        text: text.to_string(),
        spans,
        char_start: 0,
        char_end: text.chars().count(),
    }
}

#[test]
fn highlights_preserve_unicode_boundaries_and_clamp_stale_spans() {
    {
        // Both endpoints land inside multi-byte text, so a byte-vs-char confusion panics or
        // highlights the wrong glyphs.
        let text = "café—x";
        let p = para(
            text,
            vec![InlineSpan {
                char_start: 3, // 'é'
                char_end: 5,   // exclusive, through '—'
                kind: InlineSpanKind::Highlight {
                    color: "blue".into(),
                },
            }],
        );
        let spans = paragraph_spans::<u64>(&p, &colors(), &|_| 0u64);
        let full: String = spans.iter().map(|s| s.text.to_string()).collect();
        assert_eq!(full, "café—x");
        let highlighted: String = spans
            .iter()
            .filter(|s| s.highlight.is_some())
            .map(|s| s.text.to_string())
            .collect();
        assert_eq!(highlighted, "é—", "only the multi-byte run is highlighted");
        assert_eq!(spans.first().unwrap().text.to_string(), "caf");
        assert_eq!(spans.last().unwrap().text.to_string(), "x");
    }
    {
        // Indices past the text, from a corrupt annotation or a span-builder bug, clamp to its
        // length.
        let text = "hello";
        let p = para(
            text,
            vec![InlineSpan {
                char_start: 3,
                char_end: 999,
                kind: InlineSpanKind::Highlight {
                    color: "blue".into(),
                },
            }],
        );
        let spans = paragraph_spans::<u64>(&p, &colors(), &|_| 0u64);
        let highlighted: String = spans
            .iter()
            .filter(|s| s.highlight.is_some())
            .map(|s| s.text.to_string())
            .collect();
        assert_eq!(highlighted, "lo", "clamped highlight should cover 3..len");
        let full: String = spans.iter().map(|s| s.text.to_string()).collect();
        assert_eq!(full, "hello");
    }
}
