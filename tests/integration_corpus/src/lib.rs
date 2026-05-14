//! Real-adopter-shaped integration corpus for lihaaf's CI.
//!
//! The proc-macros come from the sibling `integration_corpus_macros`
//! crate; this regular library is what lihaaf builds as the dylib and
//! re-exports the macros so fixtures use a single
//! `use integration_corpus::corpus_noop;` shape. Same layout pattern as
//! `serde` re-exporting from `serde_derive`.
//!
//! This crate is `publish = false` and lives only under
//! `tests/integration_corpus/` of the lihaaf source tree. It is
//! excluded from the root crate's package by the root Cargo.toml's
//! `[package].exclude` so `cargo publish` for lihaaf itself ignores
//! the corpus.

pub use integration_corpus_macros::{
    corpus_error, corpus_error_with_n_lines, corpus_noop, corpus_oom_allocate,
    corpus_sleep_forever,
};

/// Marker referenced by `uses_corpus_noop.rs` to exercise the
/// non-macro dylib boundary alongside the proc-macro path. Keeping a
/// non-macro symbol guarantees the dylib build emits a real artifact
/// even on a toolchain where the proc-macro re-export is fully
/// optimized into a no-op at the dylib layer.
pub const SUITE_MARKER: &str = "integration_corpus::SUITE_MARKER";
