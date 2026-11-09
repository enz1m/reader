# Contributing

Use the pinned Rust toolchain and public enzUI dependencies. Keep Cargo.lock
committed. Shared controls and platform defects belong in the framework.

```sh
cargo fmt --all -- --check
ICED_TEST_BACKEND=software cargo test --locked --all-features -- --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Retain behavior tests for document admission, reading positions, editing,
persistence custody and stale-result rejection. Consolidate overlapping cases;
avoid tests that only repeat implementation details or assert one visual default.
Keep parsing, rendering and disk work on bounded workers. Handle uncertain
publication without discarding the authority needed to reconcile it.

Use disposable documents and isolated XDG paths for native checks. Exercise
compact layouts, keyboard focus, light/dark themes and changed reading/editing
flows on the actual binary. Translation catalogs live in `assets/locales/`.

This source release does not ship installers or CI. Personal build limits and
review data belong outside tracked files. Contributions are under MIT; preserve
third-party notices for bundled fonts and other assets.
