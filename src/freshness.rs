//! Per-dispatch freshness validation (the policy).
//!
//! the policy: "Before each fixture worker dispatches, the harness
//! re-checks four invariants against the in-memory manifest captured at
//! startup":
//!
//! 1. The lihaaf-managed dylib file at `managed_dylib_path` exists.
//! 2. Its mtime has not moved backward (a backward jump implies clock
//!    skew or external file replacement of the managed copy itself).
//! 3. Its SHA-256 still matches `dylib_sha256`.
//! 4. `rustc --version --verbose` still produces the same release line.
//!
//! the policy: "ANY divergence → blow the cache, re-run from stage 3
//! (dylib build), re-copy, re-validate, then proceed. No 'try anyway'
//! fallback."
//!
//! In practice — and per the dispatch-orchestrator brief — v0.1 hard-
//! fails with a diagnostic similar in shape to the policy `TOOLCHAIN_DRIFT`
//! rather than attempting a mid-session rebuild. The mid-session
//! rebuild is anchored deferral: it requires re-issuing the dylib
//! build under whatever rustc is currently active, re-copying, and
//! re-validating every in-flight worker; safer to refuse and let the
//! adopter re-run the session against the now-current toolchain.
//!
//! ## Why per-dispatch and not per-session
//!
//! A long-running session can outlive a `rustup update`, a sibling
//! cargo build that touches `target/lihaaf/`, or a clock skew event.
//! The freshness check is the only line of defense against silent ABI
//! mismatch (per the policy: "load-time crash (loud, survivable) or silent
//! miscompilation (quiet, catastrophic)"). The per-dispatch cost is
//! dominated by the SHA-256 over a page-cache-warm artifact (~30 ms
//! for a 10–50 MB dylib on a laptop) plus a short `rustc --version
//! --verbose` subprocess — small enough that paying it on every
//! dispatch is the right call given the blast radius of a stale dylib.

use std::path::PathBuf;

use crate::toolchain;
use crate::util;

/// Snapshot of the four invariants captured at session startup.
///
/// Re-checked per fixture dispatch via [`check`]. The snapshot is
/// constructed once per session from the data already on hand after
/// stages 2–5 of [`crate::session::run`] (`Toolchain` + dylib copy
/// outcome); we copy out only the four scalars we need so the snapshot
/// is `Send + Sync + Clone` for the worker pool.
#[derive(Debug, Clone)]
pub struct FreshnessSnapshot {
    /// Absolute path of the lihaaf-managed dylib copy. This is
    /// invariant 1 (existence) plus the input to invariants 2 + 3.
    pub managed_dylib_path: PathBuf,
    /// mtime of the managed dylib at copy time, in Unix seconds.
    /// Invariant 2 — a backward jump triggers the failure path.
    pub original_mtime_unix_secs: i64,
    /// SHA-256 of the managed dylib at copy time. Invariant 3 —
    /// 3 — a hash mismatch triggers the failure path even if mtime is
    /// stable (defensive against in-place edits that preserve the
    /// timestamp).
    pub original_sha256: String,
    /// First line of `rustc --version --verbose` captured at session
    /// startup. Invariant 4 + the policy — re-runs `rustc
    /// --version --verbose` and compares the release line. Same key
    /// as the policy mid-session check at session-startup boundary;
    /// freshness wraps it into the per-dispatch loop.
    pub original_rustc_release_line: String,
}

/// One of the four policy invariants and its drift detail.
#[derive(Debug, Clone)]
pub enum FreshnessFailure {
    /// Invariant 1: the managed dylib no longer exists at the captured
    /// path. The `path` is the absolute path lihaaf was checking; the
    /// adopter typically discovers this when an unrelated `cargo
    /// clean` ran mid-session.
    DylibMissing {
        /// Path that was expected to exist.
        path: PathBuf,
    },
    /// Invariant 2: the managed dylib's mtime moved backward relative
    /// to the captured value. Implies clock skew, an external file
    /// replacement of the managed copy, or NTP correction.
    DylibMtimeBackward {
        /// Path of the managed copy.
        path: PathBuf,
        /// mtime captured at copy time (Unix seconds).
        original_mtime: i64,
        /// mtime observed at this dispatch (Unix seconds).
        observed_mtime: i64,
    },
    /// Invariant 3: the managed dylib's SHA-256 no longer matches the
    /// captured digest. Implies in-place edit of the managed copy.
    DylibShaMismatch {
        /// Path of the managed copy.
        path: PathBuf,
        /// SHA-256 captured at copy time.
        original_sha256: String,
        /// SHA-256 observed at this dispatch.
        observed_sha256: String,
    },
    /// Invariant 4: rustc release line drifted between session startup
    /// and this dispatch. Same shape as the policy `TOOLCHAIN_DRIFT`, but
    /// fired from the per-dispatch path rather than the one-shot
    /// pre-dispatch check.
    RustcDrift {
        /// Release line captured at session startup.
        original_release_line: String,
        /// Release line observed at this dispatch.
        observed_release_line: String,
    },
}

impl FreshnessFailure {
    /// Stable identifier for the invariant that drifted. Consumed by
    /// the session-outcome diagnostic so adopters and CI can grep on a
    /// fixed token rather than a free-form message body.
    pub fn invariant_label(&self) -> &'static str {
        match self {
            Self::DylibMissing { .. } => "managed_dylib_path",
            Self::DylibMtimeBackward { .. } => "dylib_mtime",
            Self::DylibShaMismatch { .. } => "dylib_sha256",
            Self::RustcDrift { .. } => "rustc_release",
        }
    }

    /// Pre-rendered diagnostic body. Composed once at construction
    /// time so the session reporter prints byte-deterministic output.
    pub fn detail(&self) -> String {
        match self {
            Self::DylibMissing { path } => {
                format!("managed dylib no longer exists at {}", path.display())
            }
            Self::DylibMtimeBackward {
                path,
                original_mtime,
                observed_mtime,
            } => format!(
                "managed dylib mtime moved backward at {} (original: {original_mtime}, observed: {observed_mtime})",
                path.display()
            ),
            Self::DylibShaMismatch {
                path,
                original_sha256,
                observed_sha256,
            } => format!(
                "managed dylib SHA-256 changed at {} (original: {original_sha256}, observed: {observed_sha256})",
                path.display()
            ),
            Self::RustcDrift {
                original_release_line,
                observed_release_line,
            } => format!(
                "rustc release line changed (original: {original_release_line}, observed: {observed_release_line})"
            ),
        }
    }
}

/// Re-check the four policy invariants against `snapshot`. Returns
/// `Ok(())` when all four still hold; otherwise returns the first
/// invariant that drifted (checked in a fixed order: existence → mtime →
/// SHA-256 → rustc).
///
/// The check is intended for the per-dispatch path. Re-running a
/// short `rustc --version --verbose` per fixture is acceptable — the
/// cost is dwarfed by the per-fixture rustc compile — and gives us
/// the only line of defense against an in-session toolchain swap.
pub fn check(snapshot: &FreshnessSnapshot) -> Result<(), FreshnessFailure> {
    // Invariant 1: existence.
    let path = &snapshot.managed_dylib_path;
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            return Err(FreshnessFailure::DylibMissing { path: path.clone() });
        }
    };

    // Invariant 2: mtime not moved backward.
    let observed_mtime = match meta.modified() {
        Ok(t) => t
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        Err(_) => 0,
    };
    if observed_mtime < snapshot.original_mtime_unix_secs {
        return Err(FreshnessFailure::DylibMtimeBackward {
            path: path.clone(),
            original_mtime: snapshot.original_mtime_unix_secs,
            observed_mtime,
        });
    }

    // Invariant 3: SHA-256 unchanged.
    let observed_sha = match util::sha256_file(path) {
        Ok(s) => s,
        Err(_) => {
            return Err(FreshnessFailure::DylibMissing { path: path.clone() });
        }
    };
    if observed_sha != snapshot.original_sha256 {
        return Err(FreshnessFailure::DylibShaMismatch {
            path: path.clone(),
            original_sha256: snapshot.original_sha256.clone(),
            observed_sha256: observed_sha,
        });
    }

    // Invariant 4: rustc release line unchanged.
    match toolchain::capture() {
        Ok(t) => {
            if t.release_line != snapshot.original_rustc_release_line {
                return Err(FreshnessFailure::RustcDrift {
                    original_release_line: snapshot.original_rustc_release_line.clone(),
                    observed_release_line: t.release_line,
                });
            }
        }
        Err(_) => {
            // A captured toolchain that can no longer be re-captured is
            // itself a drift. We surface as RustcDrift with an empty
            // observed line — better than silently passing the check.
            return Err(FreshnessFailure::RustcDrift {
                original_release_line: snapshot.original_rustc_release_line.clone(),
                observed_release_line: String::new(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_dylib_stub(dir: &std::path::Path, contents: &[u8]) -> PathBuf {
        let p = dir.join("libstub.so");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(contents).unwrap();
        f.sync_all().unwrap();
        p
    }

    #[test]
    fn missing_dylib_fails_invariant_1() {
        let snap = FreshnessSnapshot {
            managed_dylib_path: PathBuf::from("/path/that/does/not/exist.so"),
            original_mtime_unix_secs: 0,
            original_sha256: "deadbeef".into(),
            original_rustc_release_line: "rustc 1.95.0 (abc 2026-01-01)".into(),
        };
        let r = check(&snap).unwrap_err();
        assert_eq!(r.invariant_label(), "managed_dylib_path");
    }

    #[test]
    fn sha_mismatch_fails_invariant_3() {
        let tmp = tempdir().unwrap();
        let p = write_dylib_stub(tmp.path(), b"hello world");
        let mtime = std::fs::metadata(&p)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let snap = FreshnessSnapshot {
            managed_dylib_path: p.clone(),
            original_mtime_unix_secs: mtime,
            // Wrong digest on purpose.
            original_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
            original_rustc_release_line: "rustc 1.95.0 (abc 2026-01-01)".into(),
        };
        let err = check(&snap).unwrap_err();
        match &err {
            FreshnessFailure::DylibShaMismatch { .. } => {}
            other => panic!("expected DylibShaMismatch, got {other:?}"),
        }
        assert_eq!(err.invariant_label(), "dylib_sha256");
    }

    #[test]
    fn detail_messages_are_byte_deterministic() {
        let f = FreshnessFailure::DylibShaMismatch {
            path: PathBuf::from("/p/lib.so"),
            original_sha256: "abc".into(),
            observed_sha256: "def".into(),
        };
        let a = f.detail();
        let b = f.detail();
        assert_eq!(a, b);
        assert!(a.contains("/p/lib.so"));
        assert!(a.contains("abc"));
        assert!(a.contains("def"));
    }
}
