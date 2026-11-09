//! Launch the Reader library application.

#![deny(unconditional_recursion)]

fn main() -> iced::Result {
    enzui_iced_core::crash_report::CrashReporter::for_app("reader-app", env!("CARGO_PKG_VERSION"));
    reader_app::run()
}
