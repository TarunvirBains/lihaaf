//! NO accompanying `.stderr` file — tests the `SNAPSHOT_MISSING`
//! verdict path. The macro emits a deliberate `compile_error!` so
//! rustc exits non-zero; lihaaf observes the missing snapshot file
//! and emits `SNAPSHOT_MISSING <path>` with the captured normalized
//! stderr in the per-fixture report.

use integration_corpus::corpus_error;

corpus_error!("integration_corpus: fixture intentionally has no snapshot");

fn main() {}
