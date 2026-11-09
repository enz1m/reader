# Formats and editing

## PDF

Poppler-GLib renders pages and exposes text, outline and search. Reader supports
continuous, single-page and facing-page layouts, zoom, fit width/page, rotation,
bookmarks and source-coordinate text selection. Password-protected PDFs prompt
before loading.

Highlights and comments are native PDF annotations. Page arrangement supports
reordering, rotation, deletion, extraction, splitting and appending another PDF,
with undo/redo before publication. Apply writes to the selected PDF; Save As,
extract and split publish separate outputs. Published output files are not
removed by undo.

Document properties include metadata, page count, format version, size and
encryption. Metadata edits, clearing metadata and password changes are explicit
operations. Signatures support typed handwriting, an image and certificate
signing. Verification and trust depend on the recipient's PDF tools and
certificate trust configuration. Freehand ink and general PDF text editing are
not implemented.

## EPUB

Reader parses the spine and contents, follows chapter-local and cross-chapter
footnotes, and offers scrolled or paginated chapters. Reflow preserves a
character anchor. Typography and reading palettes can be changed separately
from the shell theme. EPUB facing pages are not implemented.

EPUB source files remain unchanged. Bookmarks, highlights, notes and reading
preferences are stored separately; annotations can be exported as JSON or
Markdown. Character anchors are Reader-local positions, not standard EPUB CFIs.

## CBZ

CBZ is a ZIP archive of JPEG or PNG pages. Reader orders pages by archive names,
decodes them on demand and offers the PDF-style reading layouts. The page
thumbnail sidebar currently supports PDF only. Archive entries
are not extracted to disk. CBR/RAR and other comic archive formats are unsupported.

## Data and failure behavior

Document admission bounds encoded input, archive entries, expanded bytes and
decoded images. Corrupt or oversized input fails visibly. Parsing, rendering,
search and filesystem work run off the UI thread with bounded residency and
cancellation tied to the current document generation.

Original path references remain in place when dragged between local apps.
Temporary exported documents are retained before a drop completes. Recent rows
with missing source files cannot offer a file drag. Selected document text is
exported as text rather than a file.

Saved library/progress and sidecars use atomic publication. Failed persistence
can delay close until it is reconciled or acknowledged. Corrupt and newer state
is preserved rather than silently replaced. Save As and extracted files cannot
overwrite existing destinations without explicit authority.

Localization is partial; untranslated interface text falls back to English.
