//! Progress records and recent books.

use super::*;

/// The map key under which a document's progress is stored: its canonicalised
/// absolute path, so the same file opened via different relative paths or
/// symlinks resolves to one library entry. Falls back to the raw path when the
/// file cannot be canonicalised (e.g. it no longer exists).
pub fn canonical_key(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Look up a document's saved progress by path (via [`canonical_key`]).
pub fn get_progress(state: &AppState, path: &str) -> Option<ProgressRecord> {
    state.files.get(&canonical_key(path)).cloned()
}

/// Record (or overwrite) a document's progress, keyed by [`canonical_key`].
pub fn set_progress(state: &mut AppState, path: &str, record: ProgressRecord) {
    state.files.insert(canonical_key(path), record);
}

/// A summary of one recently-opened book — everything the welcome UI needs
/// to render a "Continue reading" card without reloading the source file.
///
/// `path` is the canonical path used to reopen. `progress_percentage` is a 0..1 float
/// derived from `(page + 1) / total`.
#[derive(Clone, Debug, PartialEq)]
pub struct RecentBook {
    pub path: String,
    pub title: String,
    pub author: String,
    pub page: usize,
    pub total: usize,
    pub progress_percentage: f32,
    pub last_opened: String,
    pub bookmark_count: usize,
    pub format: String,
    /// True when the file no longer exists at `path`; the UI greys the card and offers
    /// removal.
    pub file_missing: bool,
    /// Accumulated reading time in seconds.
    pub reading_time_secs: u64,
}

/// Return the `limit` most-recently-opened books from `state`, sorted
/// newest-first by `last_opened`. Records with an empty `last_opened`
/// timestamp sort last.
///
/// `bookmark_count_of` is a callback so this function stays IO-free (the
/// caller passes `|path| load_annotations(path).bookmarks.len()`). Same
/// contract for `file_exists`, so tests can inject a fake fs.
pub fn recent_books<F, G>(
    state: &AppState,
    limit: usize,
    mut bookmark_count_of: F,
    mut file_exists: G,
) -> Vec<RecentBook>
where
    F: FnMut(&str) -> usize,
    G: FnMut(&str) -> bool,
{
    if limit == 0 {
        return Vec::new();
    }
    let mut entries: Vec<(&String, &ProgressRecord)> = state.files.iter().collect();
    // Sort by last_opened DESC. Empty strings sort last.
    entries.sort_by(|a, b| {
        let ka = if a.1.last_opened.is_empty() {
            ""
        } else {
            &a.1.last_opened
        };
        let kb = if b.1.last_opened.is_empty() {
            ""
        } else {
            &b.1.last_opened
        };
        kb.cmp(ka)
    });
    entries
        .into_iter()
        .take(limit)
        .map(|(path, rec)| {
            // Progress: percentage complete based on 1-indexed page position.
            // Reader-app stores `page` 0-indexed; the "N / M" reader convention
            // is 1-indexed so a fresh file on page 0 reads as "1 / N".
            let percentage = if rec.total > 0 {
                ((rec.page + 1) as f32 / rec.total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            RecentBook {
                path: path.clone(),
                title: rec.title.clone(),
                author: rec.author.clone(),
                page: rec.page,
                total: rec.total,
                progress_percentage: percentage,
                last_opened: rec.last_opened.clone(),
                bookmark_count: bookmark_count_of(path),
                format: rec.format.clone(),
                file_missing: !file_exists(path),
                reading_time_secs: rec.reading_time_secs.unwrap_or(0),
            }
        })
        .collect()
}

/// Remove a book from the recents list. No-op if the path is unknown.
/// Returns true when a record was removed.
pub fn remove_recent(state: &mut AppState, path: &str) -> bool {
    state.files.remove(&canonical_key(path)).is_some()
}
