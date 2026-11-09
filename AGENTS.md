# Reader contributor notes

Read README.md and the relevant format and data documentation. Keep application
logic in Rust and reuse enzUI controls, theme tokens and native services. Keep
SDK and foundation revisions pinned together; never edit Cargo's cached sources.

Keep parsing, rendering and filesystem work off the UI thread. Bound workers,
archive expansion and decoded images. Reject stale asynchronous completions.
Preserve document authority and uncertain-publication debt through cancellation,
close and recovery; never silently overwrite originals or existing destinations.

Run formatting, relevant behavior tests and strict Clippy for code changes.
Prefer tests for reading/editing flows, data integrity, protocol and ownership
over setter or source-shape assertions. Use disposable documents and isolated
XDG paths for native checks, never personal libraries or signing credentials.

Translation catalogs live in assets/locales/. Only curated README screenshots
belong in docs/screenshots/. Keep temporary captures, scripts and review notes
under ignored AGENTS/. Personal build limits belong outside tracked files;
reuse compatible Cargo targets without copying artifacts or editing fingerprints.
Stage explicit paths and keep credentials, user content and build output out of Git.
