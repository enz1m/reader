use super::*;

fn ch(index: usize, title: &str, text: &str) -> EpubChapter {
    EpubChapter {
        title: title.to_string(),
        heading: String::new(),
        content_html: String::new(),
        plain_text: text.to_string(),
        index,
    }
}

#[test]
fn epub_search_bounds_results_and_preserves_unicode_character_offsets() {
    {
        // A one-character query over a run of that character would emit one hit per position.
        let occurrences = super::MAX_EPUB_SEARCH_HITS + 500;
        let chapters = vec![ch(0, "one", &"a".repeat(occurrences))];
        let hits = find_hits(&chapters, "a");
        assert_eq!(
            hits.len(),
            super::MAX_EPUB_SEARCH_HITS,
            "raise the early return in find_hits_in and this count follows the input size"
        );
    }
    {
        // `İ` lowercases to two chars, so offsets taken on `to_lowercase()` drift past the
        // original `plain_text` that highlights and previews index.
        let text = "İX dog end";
        let chapters = vec![ch(0, "one", text)];
        let hits = find_hits(&chapters, "dog");
        assert_eq!(hits.len(), 1);
        let start = hits[0].char_start;
        let end = hits[0].char_end;
        let recovered: String = text.chars().skip(start).take(end - start).collect();
        assert_eq!(
            recovered, "dog",
            "char offsets must index the original text, got start={start} end={end} → {recovered:?}"
        );
    }
}
