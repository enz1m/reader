//! Document parsing, annotation reconciliation, publication and persistence.

pub mod cbz_support;
pub mod epub_support;
pub mod footnotes;
pub(crate) mod pdf_annotations;
pub(crate) mod pdf_navigation;
pub mod pdf_ops;
pub mod pdf_sign;
pub mod persistence;
pub mod reading_stats;
pub mod spread;
