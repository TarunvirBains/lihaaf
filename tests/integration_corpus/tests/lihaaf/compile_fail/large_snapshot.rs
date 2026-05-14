//! Emits ~10100 lines of `compile_error!` output, exceeding
//! `crate::diff::SOFT_LINE_CEILING` (10_000) but staying under
//! `HARD_LINE_CEILING` (100_000). The committed `.stderr` has one
//! line deliberately mutated near the middle so the diff is
//! non-empty — that flips the verdict from `Ok` to `SnapshotDiff`,
//! which is the path that carries the `LARGE_SNAPSHOT` warning
//! (`src/diff.rs::unified_diff` sets `warn = n > SOFT_LINE_CEILING`
//! on the `Diff` arm; the `NoChange` arm has no warn field).

use integration_corpus::corpus_error_with_n_lines;

corpus_error_with_n_lines!(10100);

fn main() {}
