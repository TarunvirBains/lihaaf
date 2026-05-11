//! Exit codes per spec §10.3.
//!
//! These are part of the v0.1 stable surface (§8.5). Adding a new failure
//! mode produces a new code; an existing code's meaning does not change.
//! When multiple verdicts are present in a single run, the binary's exit
//! code is the maximum (most severe) one — see [`ExitCode::merge`].
//!
//! ## Numbering
//!
//! - `0` — all OK or BLESSED.
//! - `1–8` — fixture-level failures (least to most severe).
//! - `64+` — `<sysexits.h>`-styled session-level "couldn't even start"
//!   failures.
//!
//! `CleanupResidue` (8) sits between fixture failures and session-level
//! outcomes — disk hygiene is worse than a normal fixture failure but
//! does not invalidate the verdict signal.

/// Spec §10.3 exit codes. `repr(u8)` is the wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ExitCode {
    /// All fixtures `OK` or `BLESSED`.
    Ok = 0,
    /// One or more `EXPECTED_FAIL_BUT_PASSED`, `EXPECTED_PASS_BUT_FAILED`,
    /// or `SNAPSHOT_DIFF`.
    SnapshotDiffOrUnexpected = 1,
    /// One or more `SNAPSHOT_MISSING` (without `--bless`).
    SnapshotMissing = 2,
    /// One or more `TIMEOUT`.
    Timeout = 3,
    /// One or more `MEMORY_EXHAUSTED`.
    MemoryExhausted = 4,
    /// One or more `WORKER_CRASHED`.
    WorkerCrashed = 5,
    /// One or more `SNAPSHOT_DIFF_TOO_LARGE`.
    SnapshotDiffTooLarge = 6,
    /// One or more `MALFORMED_DIAGNOSTIC`.
    MalformedDiagnostic = 7,
    /// One or more `CLEANUP_FAILED` diagnostics.
    CleanupResidue = 8,
    /// `[package.metadata.lihaaf]` missing or invalid.
    ConfigInvalid = 64,
    /// The dylib build returned non-zero.
    DylibBuildFailed = 65,
    /// cargo succeeded but no `compiler-artifact` named the dylib.
    DylibNotFound = 66,
    /// rustc release drifted between dylib build and fixture dispatch.
    ToolchainDrift = 67,
}

impl ExitCode {
    /// Take the maximum (most severe) of two exit codes per spec §10.3.
    ///
    /// The ordering is the explicit numeric ordering: `1 < 2 < 3 < 4 <
    /// 5 < 6 < 7 < 8 < 64 < 65 < 66 < 67`. `Ok` (`0`) is the identity.
    pub fn merge(self, other: ExitCode) -> ExitCode {
        if (self as u8) >= (other as u8) {
            self
        } else {
            other
        }
    }

    /// The textual name as it appears in spec tables, for diagnostic
    /// rendering. Distinct from [`std::fmt::Debug`] only because the
    /// spec's verdict catalog uses `SCREAMING_SNAKE_CASE`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::SnapshotDiffOrUnexpected => "SNAPSHOT_DIFF_OR_UNEXPECTED",
            Self::SnapshotMissing => "SNAPSHOT_MISSING",
            Self::Timeout => "TIMEOUT",
            Self::MemoryExhausted => "MEMORY_EXHAUSTED",
            Self::WorkerCrashed => "WORKER_CRASHED",
            Self::SnapshotDiffTooLarge => "SNAPSHOT_DIFF_TOO_LARGE",
            Self::MalformedDiagnostic => "MALFORMED_DIAGNOSTIC",
            Self::CleanupResidue => "CLEANUP_RESIDUE",
            Self::ConfigInvalid => "CONFIG_INVALID",
            Self::DylibBuildFailed => "DYLIB_BUILD_FAILED",
            Self::DylibNotFound => "DYLIB_NOT_FOUND",
            Self::ToolchainDrift => "TOOLCHAIN_DRIFT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact numbering in spec §10.3 — pinned so accidental
    /// reordering of variants fails the test rather than silently
    /// shipping a renumbered surface.
    #[test]
    fn spec_numbering_is_stable() {
        assert_eq!(ExitCode::Ok as u8, 0);
        assert_eq!(ExitCode::SnapshotDiffOrUnexpected as u8, 1);
        assert_eq!(ExitCode::SnapshotMissing as u8, 2);
        assert_eq!(ExitCode::Timeout as u8, 3);
        assert_eq!(ExitCode::MemoryExhausted as u8, 4);
        assert_eq!(ExitCode::WorkerCrashed as u8, 5);
        assert_eq!(ExitCode::SnapshotDiffTooLarge as u8, 6);
        assert_eq!(ExitCode::MalformedDiagnostic as u8, 7);
        assert_eq!(ExitCode::CleanupResidue as u8, 8);
        assert_eq!(ExitCode::ConfigInvalid as u8, 64);
        assert_eq!(ExitCode::DylibBuildFailed as u8, 65);
        assert_eq!(ExitCode::DylibNotFound as u8, 66);
        assert_eq!(ExitCode::ToolchainDrift as u8, 67);
    }

    #[test]
    fn merge_takes_maximum() {
        assert_eq!(
            ExitCode::Timeout.merge(ExitCode::SnapshotDiffOrUnexpected),
            ExitCode::Timeout
        );
        assert_eq!(
            ExitCode::CleanupResidue.merge(ExitCode::WorkerCrashed),
            ExitCode::CleanupResidue
        );
        assert_eq!(
            ExitCode::ConfigInvalid.merge(ExitCode::CleanupResidue),
            ExitCode::ConfigInvalid
        );
        assert_eq!(ExitCode::Ok.merge(ExitCode::Ok), ExitCode::Ok);
    }
}
