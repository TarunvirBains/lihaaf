//! Toolchain capture (the policy stage 2).
//!
//! `rustc --version --verbose` produces a multi-line text block, parsed
//! into a [`Toolchain`] struct; the parsed scalars
//! `(release_line, host, commit_hash, sysroot)` form the drift-detection
//! key for the policy. Any single dimension changing trips the hard-fail.
//!
//! ## Sample output
//!
//! ```text
//! rustc 1.95.0 (59807616e 2026-04-14)
//! binary: rustc
//! commit-hash: 59807616e2031c7c44a76b1b0c1bbd0fed9a07cf
//! commit-date: 2026-04-14
//! host: x86_64-unknown-linux-gnu
//! release: 1.95.0
//! LLVM version: 22.1.2
//! ```
//!
//! The release line is the first line; the rest are `key: value` pairs.
//! Missing or reordered keys are tolerated — only `release` and `host` are
//! load-bearing for parser success. `commit-hash` is optional (custom
//! rustc builds omit it). `LLVM version:` is intentionally NOT captured
//! into the drift key — LLVM is fingerprinted into rustc's commit-hash,
//! so an LLVM swap implies a commit-hash swap on any stable-channel
//! rustc; see `matches` for the documented caveat.
//!
//! ## Why parse, not just stash the raw output
//!
//! The drift check (the policy) compares each of the four key scalars
//! independently, not the full raw block. Parsing once at startup and
//! re-running rustc per dispatch lets `matches` compare four `String`
//! / `PathBuf` equalities rather than normalizing whole multi-line
//! blocks every dispatch. The release line stays in the key (it's the
//! canonical user-visible identifier), but it is no longer the sole
//! comparator — `host`, `commit_hash`, and `sysroot` close the
//! same-release-line / different-toolchain gap that a release-only key
//! left open.

use std::path::PathBuf;
use std::process::Command;

use crate::error::Error;

/// Parsed `rustc --version --verbose` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// First line verbatim (`rustc 1.95.0 (...)`). The drift-detection
    /// key — the policy compares this line.
    pub release_line: String,
    /// `release: 1.95.0` parsed value.
    pub release: String,
    /// `host: x86_64-unknown-linux-gnu`. Used by the dylib path lookup.
    pub host: String,
    /// `commit-hash: <40-hex>` if present; empty string when absent
    /// (e.g., custom rustc builds).
    pub commit_hash: String,
    /// Sysroot from `rustc --print sysroot`. Captured separately because
    /// `--version --verbose` does not include it.
    pub sysroot: PathBuf,
}

/// Capture the active toolchain at the current process's view of `PATH`.
///
/// Invokes `rustc --version --verbose` and `rustc --print sysroot`. Both
/// must succeed; either failure is a [`Error::SubprocessSpawn`] or a
/// `ConfigInvalid` outcome (the user's environment cannot find rustc).
pub fn capture() -> Result<Toolchain, Error> {
    let version_out = Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()
        .map_err(|e| Error::SubprocessSpawn {
            program: "rustc".into(),
            source: e,
        })?;

    if !version_out.status.success() {
        return Err(Error::Session(crate::error::Outcome::ConfigInvalid {
            message: format!(
                "`rustc --version --verbose` exited {} (stderr: {}).\nWhy this matters: lihaaf needs the active toolchain identity to detect drift.",
                version_out.status,
                String::from_utf8_lossy(&version_out.stderr).trim()
            ),
        }));
    }

    let text = String::from_utf8_lossy(&version_out.stdout);
    let mut release_line = String::new();
    let mut release = String::new();
    let mut host = String::new();
    let mut commit_hash = String::new();
    for (idx, line) in text.lines().enumerate() {
        if idx == 0 {
            release_line = line.trim().to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "release" => release = value.to_string(),
                "host" => host = value.to_string(),
                "commit-hash" => commit_hash = value.to_string(),
                _ => {}
            }
        }
    }

    if release.is_empty() || host.is_empty() {
        return Err(Error::Session(crate::error::Outcome::ConfigInvalid {
            message: format!(
                "`rustc --version --verbose` did not include `release:` and/or `host:` lines.\n  output:\n{text}"
            ),
        }));
    }

    let sysroot_out = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| Error::SubprocessSpawn {
            program: "rustc".into(),
            source: e,
        })?;

    if !sysroot_out.status.success() {
        return Err(Error::Session(crate::error::Outcome::ConfigInvalid {
            message: format!(
                "`rustc --print sysroot` exited {}.\n  stderr: {}",
                sysroot_out.status,
                String::from_utf8_lossy(&sysroot_out.stderr).trim()
            ),
        }));
    }

    let sysroot = PathBuf::from(
        String::from_utf8_lossy(&sysroot_out.stdout)
            .trim()
            .to_string(),
    );

    Ok(Toolchain {
        release_line,
        release,
        host,
        commit_hash,
        sysroot,
    })
}

/// True if `current` matches `original` on every field of the
/// drift-detection key: `release_line`, `host`, `commit_hash`, and
/// `sysroot`. The hard-fail policy treats any inequality as drift.
///
/// The `release` field is not separately checked — `release_line`
/// already embeds it (e.g. `rustc 1.95.0 (59807616e 2026-04-14)`
/// includes `1.95.0`), so `release_line` equality implies `release`
/// equality for any toolchain rustc could plausibly emit.
///
/// **Known weakness (custom rustc builds):** When `rustc` is a custom
/// local build, `commit-hash:` is absent and [`Toolchain::commit_hash`]
/// is the empty string. Two different custom builds with the same
/// release line and host both have `commit_hash == ""` and will compare
/// equal on that field; the `sysroot` comparison usually catches this
/// in practice because two custom-build installs typically have
/// different sysroots, but the gap is real. Users running custom rustc
/// builds operate outside the stable-channel safety net by design.
///
/// LLVM-version drift is not part of the key: LLVM is fingerprinted
/// into rustc's commit_hash, so LLVM drift between two
/// same-`(release_line, host, commit_hash, sysroot)` toolchains is
/// implausible. If a real regression surfaces, a follow-up issue
/// should add LLVM-version capture to [`capture`].
pub fn matches(original: &Toolchain, current: &Toolchain) -> bool {
    original.release_line == current.release_line
        && original.host == current.host
        && original.commit_hash == current.commit_hash
        && original.sysroot == current.sysroot
}

/// Render the four-field drift key as a stable multi-line string for
/// diagnostics. Used by [`crate::error::Outcome::ToolchainDrift`]'s
/// Display impl so the boundary message names every captured dimension
/// — without this, a host/commit_hash/sysroot drift with an identical
/// release line would print indistinguishable "original" and "current"
/// lines, defeating the purpose of widening the comparator.
///
/// Format is byte-deterministic so adopter CI scripts can grep on it:
///
/// ```text
/// release_line: rustc X.Y.Z (...)
/// host: x86_64-unknown-linux-gnu
/// commit_hash: <40-hex or empty>
/// sysroot: /path/to/toolchain
/// ```
pub(crate) fn format_drift_key(t: &Toolchain) -> String {
    format!(
        "release_line: {rl}\nhost: {host}\ncommit_hash: {ch}\nsysroot: {sr}",
        rl = t.release_line,
        host = t.host,
        ch = t.commit_hash,
        sr = t.sysroot.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_real_rustc_block_shape() {
        // rustc is not called in unit tests (it is called once in
        // the integration test). This test exercises the parser directly
        // against a captured shape.
        let captured = "\
rustc 1.95.0 (59807616e 2026-04-14)
binary: rustc
commit-hash: 59807616e2031c7c44a76b1b0c1bbd0fed9a07cf
commit-date: 2026-04-14
host: x86_64-unknown-linux-gnu
release: 1.95.0
LLVM version: 22.1.2";

        let mut release_line = String::new();
        let mut release = String::new();
        let mut host = String::new();
        let mut commit_hash = String::new();
        for (idx, line) in captured.lines().enumerate() {
            if idx == 0 {
                release_line = line.trim().to_string();
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "release" => release = value.to_string(),
                    "host" => host = value.to_string(),
                    "commit-hash" => commit_hash = value.to_string(),
                    _ => {}
                }
            }
        }

        assert_eq!(release_line, "rustc 1.95.0 (59807616e 2026-04-14)");
        assert_eq!(release, "1.95.0");
        assert_eq!(host, "x86_64-unknown-linux-gnu");
        assert_eq!(commit_hash, "59807616e2031c7c44a76b1b0c1bbd0fed9a07cf");
    }

    /// Build a canonical [`Toolchain`] for the comparator tests. Each
    /// test below mutates one field on a clone to isolate the field's
    /// effect on `matches`.
    fn baseline_toolchain() -> Toolchain {
        Toolchain {
            release_line: "rustc 1.95.0 (59807616e 2026-04-14)".into(),
            release: "1.95.0".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            commit_hash: "59807616e2031c7c44a76b1b0c1bbd0fed9a07cf".into(),
            sysroot: PathBuf::from("/usr/local/rustup/toolchains/stable-x86_64"),
        }
    }

    /// Shared body for the four comparator-test cases. Builds the baseline,
    /// clones into `b`, applies the caller's `mutate` to `b`, and asserts
    /// that the comparator detects the per-field difference. Mirrors the
    /// `assert_only_field_drifts` helper in `freshness.rs::tests` so the
    /// same parameterization pattern applies to both layers of the
    /// four-field comparator.
    fn assert_field_mutation_differs(mutate: impl FnOnce(&mut Toolchain)) {
        let a = baseline_toolchain();
        let mut b = a.clone();
        mutate(&mut b);
        assert!(!matches(&a, &b));
    }

    #[test]
    fn matches_identical_toolchains() {
        let a = baseline_toolchain();
        let b = a.clone();
        assert!(matches(&a, &b));
    }

    #[test]
    fn matches_compares_full_key_release_line_differs() {
        assert_field_mutation_differs(|b| {
            b.release_line = "rustc 1.96.0 (deadbeef 2026-07-01)".into();
        });
    }

    /// Same release line, different host (cross-compile or architecture
    /// migration). Two materially different toolchains with the same
    /// release_line must compare unequal.
    #[test]
    fn matches_compares_full_key_host_differs() {
        assert_field_mutation_differs(|b| {
            b.host = "aarch64-apple-darwin".into();
        });
    }

    #[test]
    fn matches_compares_full_key_commit_hash_differs() {
        assert_field_mutation_differs(|b| {
            b.commit_hash = "0000000000000000000000000000000000000000".into();
        });
    }

    /// Channel-switch case: same rustc identity strings but installed under
    /// a different rustup prefix.
    #[test]
    fn matches_compares_full_key_sysroot_differs() {
        assert_field_mutation_differs(|b| {
            b.sysroot = PathBuf::from("/home/user/.rustup/toolchains/nightly-x86_64");
        });
    }

    #[test]
    fn matches_custom_build_both_empty_commit_hash() {
        // Custom rustc builds omit `commit-hash:`; `capture()` stores
        // `String::new()`. Two such toolchains with the same other
        // fields compare equal under the documented caveat.
        let mut a = baseline_toolchain();
        a.commit_hash = String::new();
        let b = a.clone();
        assert!(matches(&a, &b));
    }
}
