//! Baseline compile_fail: emits a `compile_error!` via the corpus
//! macro and expects the normalized stderr to match the sibling
//! `corpus_error_basic.stderr`. The committed snapshot is blessed
//! verbatim and lives in this directory; the fixture drives a
//! straightforward `OK` verdict on the matching path.

use integration_corpus::corpus_error;

corpus_error!("integration_corpus: deliberate basic error");

fn main() {}
