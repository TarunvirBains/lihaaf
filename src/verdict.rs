//! Per-fixture verdict catalog (spec §10.1).
//!
//! Every fixture produces exactly one [`Verdict`]. The verdict is the
//! authoritative result for that fixture in the run. Verdict names match
//! the spec table verbatim — adopters and CI consume the printed names.
//!
//! ## Why an enum, not a string
//!
//! The verdict catalog is closed. Adding new verdicts is a v0.x decision,
//! and the exit-code merge rule (`max(per-fixture verdict mapped to exit
//! code)`) only works on a typed value. A string-typed verdict would
//! make the spec §10.3 ordering impossible to enforce in code.

use std::path::PathBuf;

use crate::exit::ExitCode;

/// Per-fixture verdicts (spec §10.1).
#[derive(Debug, Clone)]
pub enum Verdict {
    /// Fixture matched expectation. Compile_pass: rustc exited 0.
    /// Compile_fail: normalized stderr equals snapshot.
    Ok,
    /// compile_fail fixture, but rustc exited 0.
    ExpectedFailButPassed,
    /// compile_pass fixture, but rustc exited non-zero. Carries the
    /// captured stderr so the report can show what went wrong.
    ExpectedPassButFailed {
        /// Captured rustc stderr (rendered, normalized).
        stderr: String,
    },
    /// compile_fail fixture, rustc failed as expected, normalized stderr
    /// differs from snapshot. Carries the unified diff.
    SnapshotDiff {
        /// Unified diff (`--- expected\n+++ actual\n…`).
        diff: String,
    },
    /// compile_fail fixture, rustc failed as expected, no `.stderr`
    /// file on disk. Carries the actual normalized stderr so the user
    /// sees what would be blessed.
    SnapshotMissing {
        /// What rustc emitted (so user can paste into a `.stderr` if
        /// adopting manually).
        actual: String,
    },
    /// `--bless` was set and the snapshot was overwritten. Treated as
    /// `Ok` for exit-code purposes.
    Blessed {
        /// Path of the rewritten file, relative to crate root.
        snapshot_path: PathBuf,
    },
    /// Worker exceeded `fixture_timeout_secs`.
    Timeout,
    /// Worker exceeded `per_fixture_memory_mb` on both initial and serial
    /// retry.
    MemoryExhausted,
    /// Worker exited via signal or non-rustc exit code without
    /// lihaaf-initiated kill.
    WorkerCrashed {
        /// Human-readable description: `signal: SIGSEGV (11)` or
        /// `exit code: 101`.
        cause: String,
    },
    /// compile_fail fixture, normalized stderr or snapshot exceeds the
    /// 100,000-line hard ceiling.
    SnapshotDiffTooLarge {
        /// Line count of the actual side.
        actual_lines: usize,
        /// Line count of the expected side.
        expected_lines: usize,
        /// First 100 lines of actual (or fewer if shorter).
        actual_head: String,
        /// First 100 lines of expected (or fewer if shorter).
        expected_head: String,
    },
    /// Non-UTF-8 bytes detected in rustc `--error-format=json` output's
    /// `rendered` field, or in the snapshot file.
    MalformedDiagnostic {
        /// Byte offset where validation failed.
        byte_offset: usize,
        /// What was being validated.
        source: MalformedSource,
    },
}

/// Where the malformed diagnostic was detected.
#[derive(Debug, Clone)]
pub enum MalformedSource {
    /// Rustc's JSON output `rendered` field.
    RustcRendered,
    /// The on-disk `.stderr` snapshot.
    Snapshot,
}

impl Verdict {
    /// Map this verdict to its exit code per spec §10.3.
    ///
    /// `Ok` and `Blessed` both map to `ExitCode::Ok`.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Ok | Self::Blessed { .. } => ExitCode::Ok,
            Self::ExpectedFailButPassed
            | Self::ExpectedPassButFailed { .. }
            | Self::SnapshotDiff { .. } => ExitCode::SnapshotDiffOrUnexpected,
            Self::SnapshotMissing { .. } => ExitCode::SnapshotMissing,
            Self::Timeout => ExitCode::Timeout,
            Self::MemoryExhausted => ExitCode::MemoryExhausted,
            Self::WorkerCrashed { .. } => ExitCode::WorkerCrashed,
            Self::SnapshotDiffTooLarge { .. } => ExitCode::SnapshotDiffTooLarge,
            Self::MalformedDiagnostic { .. } => ExitCode::MalformedDiagnostic,
        }
    }

    /// Short label for the per-fixture progress line.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::ExpectedFailButPassed => "EXPECTED_FAIL_BUT_PASSED",
            Self::ExpectedPassButFailed { .. } => "EXPECTED_PASS_BUT_FAILED",
            Self::SnapshotDiff { .. } => "SNAPSHOT_DIFF",
            Self::SnapshotMissing { .. } => "SNAPSHOT_MISSING",
            Self::Blessed { .. } => "BLESSED",
            Self::Timeout => "TIMEOUT",
            Self::MemoryExhausted => "MEMORY_EXHAUSTED",
            Self::WorkerCrashed { .. } => "WORKER_CRASHED",
            Self::SnapshotDiffTooLarge { .. } => "SNAPSHOT_DIFF_TOO_LARGE",
            Self::MalformedDiagnostic { .. } => "MALFORMED_DIAGNOSTIC",
        }
    }

    /// True when this verdict counts as a passing outcome (`Ok` or
    /// `Blessed`).
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Ok | Self::Blessed { .. })
    }
}

/// One fixture's complete result, including any cleanup-failure
/// diagnostic appended after the fact.
#[derive(Debug, Clone)]
pub struct FixtureResult {
    /// Path of the fixture relative to the crate root, in
    /// forward-slash form (deterministic across OS).
    pub relative_path: String,
    /// The fixture's verdict.
    pub verdict: Verdict,
    /// `Some(path)` if cleanup of this fixture's workdir failed. The
    /// outer report aggregates these into a `CLEANUP_RESIDUE` outcome.
    pub cleanup_failure: Option<CleanupFailure>,
    /// Wall-clock time the worker took, for the report.
    pub wall_ms: u64,
    /// Optional non-fatal warning attached to this fixture. The
    /// canonical case is `LARGE_SNAPSHOT` — the diff exceeded the §7.2
    /// soft ceiling but stayed under the hard ceiling, so the verdict
    /// is unaffected (still `Ok` / `SnapshotDiff` / etc.) but the
    /// session reporter prints a warning line.
    pub warning: Option<FixtureWarning>,
}

/// Non-fatal per-fixture warning. Distinct from [`Verdict`] because
/// warnings ride alongside a verdict rather than replacing one — a
/// `LARGE_SNAPSHOT` fixture can still pass; the warning surfaces for
/// adopter visibility without touching the exit-code aggregation.
#[derive(Debug, Clone)]
pub enum FixtureWarning {
    /// The fixture's diff input exceeded the spec §7.2 soft ceiling
    /// (10K lines) but stayed under the hard ceiling (100K lines). The
    /// session reporter renders this as
    /// `lihaaf: LARGE_SNAPSHOT <path> (<expected>/<actual> lines)`.
    LargeSnapshot {
        /// Line count on the expected (snapshot) side.
        expected_lines: usize,
        /// Line count on the actual (rustc-emitted) side.
        actual_lines: usize,
    },
}

/// One fixture's cleanup error detail.
#[derive(Debug, Clone)]
pub struct CleanupFailure {
    /// The workdir path that could not be removed.
    pub path: PathBuf,
    /// The OS error.
    pub message: String,
}
