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
//! 4. `rustc --version --verbose` still produces a toolchain that matches
//!    the captured key on every dimension — release line, host triple,
//!    commit hash, and sysroot. Same comparator as the session-startup
//!    drift check (`toolchain::matches`); freshness wraps it into the
//!    per-dispatch loop so a same-release-line drift on host /
//!    commit_hash / sysroot still trips here.
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
/// outcome); only the data needed by the four invariants is copied out
/// so the snapshot is `Send + Sync + Clone` for the worker pool.
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
    /// Full parsed toolchain captured at session startup. Invariant 4 —
    /// re-runs `rustc --version --verbose` per dispatch and compares the
    /// captured key (release_line, host, commit_hash, sysroot) via
    /// `crate::toolchain::matches`. Same comparator as the session-startup
    /// boundary check; freshness wraps it into the per-dispatch loop so
    /// a same-release-line drift on host / commit_hash / sysroot still
    /// trips here.
    pub original_toolchain: toolchain::Toolchain,
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
    /// Invariant 4: captured toolchain key drifted between session
    /// startup and this dispatch. Same shape as the policy
    /// `TOOLCHAIN_DRIFT`, but fired from the per-dispatch path rather
    /// than the one-shot pre-dispatch check. Any of `release_line`,
    /// `host`, `commit_hash`, or `sysroot` may differ — the rendered
    /// detail names which dimension(s) drifted.
    RustcDrift {
        /// Full toolchain captured at session startup.
        original: Box<toolchain::Toolchain>,
        /// Toolchain observed at this dispatch. When the re-capture
        /// itself failed (e.g. rustc no longer on PATH), this is a
        /// placeholder with empty strings + empty PathBuf.
        observed: Box<toolchain::Toolchain>,
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
            Self::RustcDrift { original, observed } => {
                // Identify which of the four key fields actually drifted
                // so the diagnostic body points the adopter at the
                // dimension that changed. Order is stable (release_line,
                // host, commit_hash, sysroot) for byte-deterministic
                // output regardless of how many fields drifted.
                let mut changed: Vec<&'static str> = Vec::new();
                if original.release_line != observed.release_line {
                    changed.push("release_line");
                }
                if original.host != observed.host {
                    changed.push("host");
                }
                if original.commit_hash != observed.commit_hash {
                    changed.push("commit_hash");
                }
                if original.sysroot != observed.sysroot {
                    changed.push("sysroot");
                }
                let changed_list = if changed.is_empty() {
                    // Should not happen — check() only constructs this
                    // variant on a real inequality — but render a stable
                    // placeholder rather than an empty list so the body
                    // is never confusingly blank.
                    "(none detected)".to_string()
                } else {
                    changed.join(", ")
                };
                format!(
                    "rustc toolchain drifted (changed fields: {changed_list}; original: {orig_rl}, host: {orig_host}, commit-hash: {orig_ch}, sysroot: {orig_sr}; observed: {obs_rl}, host: {obs_host}, commit-hash: {obs_ch}, sysroot: {obs_sr})",
                    orig_rl = original.release_line,
                    orig_host = original.host,
                    orig_ch = original.commit_hash,
                    orig_sr = original.sysroot.display(),
                    obs_rl = observed.release_line,
                    obs_host = observed.host,
                    obs_ch = observed.commit_hash,
                    obs_sr = observed.sysroot.display(),
                )
            }
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
/// cost is dwarfed by the per-fixture rustc compile — providing
/// the only line of defense against an in-session toolchain swap.
///
/// Invariant 4 uses the same `(release_line, host, commit_hash,
/// sysroot)` comparator as the session-startup `toolchain::matches`
/// check, so a same-release-line drift on host / commit_hash / sysroot
/// trips here too — no shadow comparator with a narrower key.
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

    // Invariant 4: captured toolchain key unchanged. Compared with the
    // session-startup `toolchain::matches` comparator across all four
    // key fields (release_line, host, commit_hash, sysroot).
    match toolchain::capture() {
        Ok(observed) => {
            if !toolchain::matches(&snapshot.original_toolchain, &observed) {
                return Err(FreshnessFailure::RustcDrift {
                    original: Box::new(snapshot.original_toolchain.clone()),
                    observed: Box::new(observed),
                });
            }
        }
        Err(_) => {
            // A captured toolchain that can no longer be re-captured is
            // itself a drift. Surface it as RustcDrift with a
            // placeholder `observed` so the detail renderer still has
            // valid fields to compare and the user sees a clear
            // "rustc disappeared" delta rather than a silent pass.
            return Err(FreshnessFailure::RustcDrift {
                original: Box::new(snapshot.original_toolchain.clone()),
                observed: Box::new(toolchain::Toolchain {
                    release_line: String::new(),
                    release: String::new(),
                    host: String::new(),
                    commit_hash: String::new(),
                    sysroot: PathBuf::new(),
                }),
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

    /// Canonical placeholder toolchain for tests that do not exercise the
    /// rustc-drift path. The values do not have to match the real rustc
    /// running the test because every test using this helper bails out
    /// before invariant 4 (the rustc re-capture).
    fn placeholder_toolchain() -> toolchain::Toolchain {
        toolchain::Toolchain {
            release_line: "rustc 1.95.0 (abc 2026-01-01)".into(),
            release: "1.95.0".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            commit_hash: "59807616e2031c7c44a76b1b0c1bbd0fed9a07cf".into(),
            sysroot: PathBuf::from("/usr/local/rustup/toolchains/stable-x86_64"),
        }
    }

    /// Build a passing-invariants-1-3 snapshot pointing at a stub dylib
    /// in `dir`, so the rustc-drift tests below cleanly bite only
    /// invariant 4. The `original_toolchain` is a synthetic value chosen
    /// to differ from whatever the test machine's real `rustc::capture()`
    /// returns — guaranteeing the drift path fires.
    fn snapshot_with_synthetic_toolchain(
        dir: &std::path::Path,
        original_toolchain: toolchain::Toolchain,
    ) -> FreshnessSnapshot {
        let p = write_dylib_stub(dir, b"hello world");
        let meta = std::fs::metadata(&p).unwrap();
        let mtime = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let sha = crate::util::sha256_file(&p).unwrap();
        FreshnessSnapshot {
            managed_dylib_path: p,
            original_mtime_unix_secs: mtime,
            original_sha256: sha,
            original_toolchain,
        }
    }

    #[test]
    fn missing_dylib_fails_invariant_1() {
        let snap = FreshnessSnapshot {
            managed_dylib_path: PathBuf::from("/path/that/does/not/exist.so"),
            original_mtime_unix_secs: 0,
            original_sha256: "deadbeef".into(),
            original_toolchain: placeholder_toolchain(),
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
            original_toolchain: placeholder_toolchain(),
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

    /// Invariant 4 fires when the captured `release_line` differs from
    /// the live rustc. Constructed by setting `original_toolchain` to a
    /// fake `release_line` that no real rustc could emit; the live
    /// `rustc::capture()` returns the real release line, the comparator
    /// rejects the pair, `RustcDrift` is returned, and `detail()` names
    /// `release_line` in its changed-fields list.
    #[test]
    fn freshness_check_detects_release_line_drift() {
        let tmp = tempdir().unwrap();
        let mut tc = placeholder_toolchain();
        tc.release_line = "rustc 0.0.0 (fake 1970-01-01)".into();
        let snap = snapshot_with_synthetic_toolchain(tmp.path(), tc);
        let err = check(&snap).unwrap_err();
        match &err {
            FreshnessFailure::RustcDrift { .. } => {}
            other => panic!("expected RustcDrift, got {other:?}"),
        }
        assert_eq!(err.invariant_label(), "rustc_release");
        let detail = err.detail();
        assert!(
            detail.contains("release_line"),
            "detail must name release_line as changed: {detail}"
        );
    }

    /// Invariant 4 fires when only `host` differs (the previously-shadowed
    /// case): the original branch's release-line-only comparator would
    /// have silently passed this; the widened comparator must reject it.
    ///
    /// Anchored to the live rustc so `release_line`, `commit_hash`, and
    /// `sysroot` genuinely match between snapshot and check-time capture.
    /// Only `host` is mutated, so if `check()` regressed to release-line-only
    /// it would see release_line == release_line and return Ok(()), causing
    /// `unwrap_err()` to panic — biting the regression.
    #[test]
    fn freshness_check_detects_host_drift() {
        let tmp = tempdir().unwrap();
        let live = toolchain::capture().expect("rustc must be on PATH for this test");
        let mut original = live.clone();
        original.host = "fake-host-target".into();
        let snap = snapshot_with_synthetic_toolchain(tmp.path(), original);

        let err = check(&snap).unwrap_err();
        match &err {
            FreshnessFailure::RustcDrift { .. } => {}
            other => panic!("expected RustcDrift, got {other:?}"),
        }
        assert_eq!(err.invariant_label(), "rustc_release");

        let detail = err.detail();
        // Extract only the "changed fields: <names>" prefix before the
        // first semicolon so that negative assertions are not confused
        // by field values in the original/observed dump that follows.
        let changed_prefix = detail
            .split(';')
            .next()
            .filter(|s| s.contains("changed fields:"))
            .expect("changed-fields prefix must be present");
        assert!(
            changed_prefix.contains("host"),
            "changed-fields list must name host: {changed_prefix}"
        );
        // Regression-bite: the other fields genuinely matched, so they
        // must NOT appear in the changed-fields prefix.
        assert!(
            !changed_prefix.contains("release_line"),
            "release_line must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("commit_hash"),
            "commit_hash must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("sysroot"),
            "sysroot must NOT appear in changed-fields: {changed_prefix}"
        );
    }

    /// Invariant 4 fires when only `commit_hash` differs. Anchored to the
    /// live rustc so `release_line`, `host`, and `sysroot` genuinely match;
    /// only `commit_hash` is mutated. A release-line-only regression would
    /// return Ok(()), panicking at `unwrap_err()`.
    #[test]
    fn freshness_check_detects_commit_hash_drift() {
        let tmp = tempdir().unwrap();
        let live = toolchain::capture().expect("rustc must be on PATH for this test");
        let mut original = live.clone();
        original.commit_hash = "00000000000000000000000000000000fakehash".into();
        let snap = snapshot_with_synthetic_toolchain(tmp.path(), original);

        let err = check(&snap).unwrap_err();
        match &err {
            FreshnessFailure::RustcDrift { .. } => {}
            other => panic!("expected RustcDrift, got {other:?}"),
        }
        assert_eq!(err.invariant_label(), "rustc_release");

        let detail = err.detail();
        let changed_prefix = detail
            .split(';')
            .next()
            .filter(|s| s.contains("changed fields:"))
            .expect("changed-fields prefix must be present");
        assert!(
            changed_prefix.contains("commit_hash"),
            "changed-fields list must name commit_hash: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("release_line"),
            "release_line must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("host"),
            "host must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("sysroot"),
            "sysroot must NOT appear in changed-fields: {changed_prefix}"
        );
    }

    /// Invariant 4 fires when only `sysroot` differs. Anchored to the live
    /// rustc so `release_line`, `host`, and `commit_hash` genuinely match;
    /// only `sysroot` is mutated. A release-line-only regression would
    /// return Ok(()), panicking at `unwrap_err()`.
    #[test]
    fn freshness_check_detects_sysroot_drift() {
        let tmp = tempdir().unwrap();
        let live = toolchain::capture().expect("rustc must be on PATH for this test");
        let mut original = live.clone();
        original.sysroot = PathBuf::from("/nonexistent/fake/toolchains/stable");
        let snap = snapshot_with_synthetic_toolchain(tmp.path(), original);

        let err = check(&snap).unwrap_err();
        match &err {
            FreshnessFailure::RustcDrift { .. } => {}
            other => panic!("expected RustcDrift, got {other:?}"),
        }
        assert_eq!(err.invariant_label(), "rustc_release");

        let detail = err.detail();
        let changed_prefix = detail
            .split(';')
            .next()
            .filter(|s| s.contains("changed fields:"))
            .expect("changed-fields prefix must be present");
        assert!(
            changed_prefix.contains("sysroot"),
            "changed-fields list must name sysroot: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("release_line"),
            "release_line must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("host"),
            "host must NOT appear in changed-fields: {changed_prefix}"
        );
        assert!(
            !changed_prefix.contains("commit_hash"),
            "commit_hash must NOT appear in changed-fields: {changed_prefix}"
        );
    }

    /// `detail()` rendering is byte-deterministic across the new
    /// `RustcDrift` shape. Two calls produce identical strings and the
    /// changed-fields list lands in canonical (release_line, host,
    /// commit_hash, sysroot) order regardless of how many fields drift.
    #[test]
    fn rustc_drift_detail_is_byte_deterministic_and_lists_changed_fields() {
        let original = toolchain::Toolchain {
            release_line: "rustc 1.95.0 (abc 2026-01-01)".into(),
            release: "1.95.0".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            commit_hash: "59807616e2031c7c44a76b1b0c1bbd0fed9a07cf".into(),
            sysroot: PathBuf::from("/usr/local/rustup/toolchains/stable-x86_64"),
        };
        let observed = toolchain::Toolchain {
            release_line: "rustc 1.96.0 (def 2026-07-01)".into(),
            release: "1.96.0".into(),
            host: "aarch64-apple-darwin".into(),
            commit_hash: "59807616e2031c7c44a76b1b0c1bbd0fed9a07cf".into(),
            sysroot: PathBuf::from("/usr/local/rustup/toolchains/stable-x86_64"),
        };
        let f = FreshnessFailure::RustcDrift {
            original: Box::new(original),
            observed: Box::new(observed),
        };
        let a = f.detail();
        let b = f.detail();
        assert_eq!(a, b);
        // Two fields drifted; both must appear, in canonical order.
        let ri = a.find("release_line").expect("release_line in detail");
        let hi = a.find("host").expect("host in detail");
        assert!(
            ri < hi,
            "changed-fields list must list release_line before host: {a}"
        );
        // Untouched fields are not listed in the changed-fields prefix.
        // (commit_hash and sysroot DO appear later as part of the full
        // original/observed dump — we only check the changed-fields
        // section comes first by checking the changed-fields header.)
        let header = "changed fields: release_line, host;";
        assert!(
            a.contains(header),
            "expected changed-fields header `{header}`, got: {a}"
        );
    }
}
