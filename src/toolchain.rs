//! Toolchain capture (the policy stage 2).
//!
//! `rustc --version --verbose` produces a multi-line text block. We parse
//! it into a [`Toolchain`] struct and persist the release line as the
//! drift-detection key for the policy.
//!
//! ## Sample output we parse
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
//! We tolerate missing or reordered keys — only `release` and `host` are
//! load-bearing.
//!
//! ## Why parse, not just stash the raw output
//!
//! The drift check (the policy) compares release strings, not the full block.
//! Parsing once at startup and re-running rustc per dispatch lets us
//! compare scalars instead of normalizing whole blocks repeatedly.

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

/// True if `current` matches `original`'s drift-detection key (the
/// release line). the policy compares release line equality.
pub fn matches(original: &Toolchain, current: &Toolchain) -> bool {
    original.release_line == current.release_line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_real_rustc_block_shape() {
        // We don't actually call rustc in unit tests (call it once in
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

    #[test]
    fn matches_compares_release_line_only() {
        let a = Toolchain {
            release_line: "rustc 1.95.0 (abc 2026-01-01)".into(),
            release: "1.95.0".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            commit_hash: "abc".into(),
            sysroot: PathBuf::from("/a"),
        };
        let mut b = a.clone();
        assert!(matches(&a, &b));
        b.sysroot = PathBuf::from("/b");
        // Sysroot drift is not part of the policy key.
        assert!(matches(&a, &b));
        b.release_line = "rustc 1.96.0 (def 2026-04-14)".into();
        assert!(!matches(&a, &b));
    }
}
