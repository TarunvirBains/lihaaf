//! Baseline compile_pass: invokes the no-op proc-macro AND references
//! the non-macro `SUITE_MARKER` const so the dylib boundary is
//! exercised on both paths in one fixture. Verdict: OK.

use integration_corpus::{corpus_noop, SUITE_MARKER};

corpus_noop!();

fn main() {
    let _ = SUITE_MARKER;
}
