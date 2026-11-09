use super::*;

fn chapter(n: usize, words: usize) -> Vec<ReadingParagraph> {
    let mut paras = Vec::new();
    let mut offset = 0usize;
    for i in 0..n {
        let text = (0..words)
            .map(|w| format!("word{:04}", i * words + w))
            .collect::<Vec<_>>()
            .join(" ");
        let len = text.chars().count();
        paras.push(ReadingParagraph {
            chapter: 0,
            para_index: i,
            text,
            spans: Vec::new(),
            char_start: offset,
            char_end: offset + len,
        });
        offset += len + 1; // the newline between paragraphs
    }
    paras
}

fn metrics() -> PageMetrics {
    PageMetrics::for_window(720.0, 900.0, 18.0, 1.5)
}

#[test]
fn pagination_tiles_unicode_chapters_and_preserves_anchors_across_reflow() {
    {
        let paras = chapter(25, 45);
        let pages = paginate(&paras, &metrics());
        assert!(!pages.is_empty());
        assert_eq!(pages[0].char_start, paras[0].char_start);
        assert_eq!(
            pages.last().unwrap().char_end,
            paras.last().unwrap().char_end
        );
        let chars: Vec<char> = paras
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .collect();
        for page in pages.iter().skip(1) {
            assert!(
                chars[page.char_start - 1].is_whitespace()
                    || chars[page.char_start].is_whitespace()
            );
        }
        for w in pages.windows(2) {
            assert_eq!(
                w[0].char_end, w[1].char_start,
                "pages must abut: {:?} then {:?}",
                w[0], w[1]
            );
        }
    }
    {
        let paras = chapter(40, 55);
        let before = paginate(&paras, &PageMetrics::for_window(720.0, 900.0, 18.0, 1.5));
        let anchor = before[5].char_start + 20;
        let idx_before = page_for_offset(&before, anchor);
        assert_eq!(idx_before, 5);

        for (label, after_metrics) in [
            (
                "font size",
                PageMetrics::for_window(720.0, 900.0, 12.0, 1.5),
            ),
            (
                "line height",
                PageMetrics::for_window(720.0, 900.0, 18.0, 2.2),
            ),
            (
                "window width",
                PageMetrics::for_window(360.0, 900.0, 18.0, 1.5),
            ),
            (
                "window height",
                PageMetrics::for_window(720.0, 500.0, 18.0, 1.5),
            ),
        ] {
            let after = paginate_anchored(&paras, &after_metrics, anchor);
            let idx = page_for_offset(&after, anchor);
            assert!(
                after[idx].contains(anchor),
                "{label}: page {idx} ({:?}) must still contain the anchored \
                 char offset {anchor}",
                after[idx]
            );
            assert_eq!(
                after[idx].char_start, anchor,
                "{label}: the anchored passage must be at the TOP of the page"
            );
        }

        // At least one repagination moves the passage, so the containment is not trivially true.
        let smaller = paginate(&paras, &PageMetrics::for_window(720.0, 900.0, 12.0, 1.5));
        assert_ne!(
            page_for_offset(&smaller, anchor),
            idx_before,
            "a 6pt font change must renumber the pages"
        );
    }
    {
        let text = "éclair 漢字 café 🙂 résumé alpha beta gamma delta epsilon ".repeat(10);
        let chars = text.chars().count();
        let paras = vec![ReadingParagraph {
            chapter: 0,
            para_index: 0,
            text: text.clone(),
            spans: Vec::new(),
            char_start: 0,
            char_end: chars,
        }];
        let m = PageMetrics::new(180.0, 5.0 * 24.0, 16.0, 1.5);
        let first_budget = m.lines_per_page() - HEADING_RESERVE_LINES as usize;
        let break_byte = enzui_iced_widgets::text_measure::enz_text_break_offset_with(
            &text,
            m.column_width(),
            first_budget,
            m.typography(),
        );
        assert!(
            break_byte < text.len(),
            "fixture must wrap onto another page"
        );
        let expected_chars = text[..break_byte].chars().count();
        assert_ne!(
            break_byte, expected_chars,
            "fixture needs multi-byte text before the break to distinguish coordinates"
        );

        let pages = paginate(&paras, &m);
        assert_eq!(
            pages[0].char_end, expected_chars,
            "the page boundary must be the shaper byte break converted to the \
             reader/annotation character-offset space"
        );
        assert_eq!(pages.last().unwrap().char_end, chars);
        for pair in pages.windows(2) {
            assert_eq!(pair[0].char_end, pair[1].char_start);
        }
    }
}
