//! Phase 9 of compat mode (§3.4 of `docs/compatibility-plan.md`) — active
//! toolchain capture for the §3.3 envelope's `results.lihaaf.toolchain`
//! field.
//!
//! ## What this module owns
//!
//! - [`capture_active_toolchain`] — the public entry that the Phase 10
//!   driver calls AFTER changing cwd to `--compat-root`. Invokes `rustup
//!   show active-toolchain`, returns the trimmed first line, or falls
//!   back to the rustc release line on rustup absence / subprocess error.
//! - [`parse_active_toolchain`] — `pub(crate)` byte-level parser that
//!   isolates the "first line, trim at first ASCII space" rule from the
//!   subprocess plumbing so the rule is unit-testable without spawning.
//! - [`capture_with_program`] — `pub(crate)` variant of the public entry
//!   that takes the program name as a parameter. The public entry calls
//!   it with `"rustup"`; the integration tests call it with a path that
//!   does not exist to exercise the fallback path without mutating
//!   `PATH`.
//!
//! ## Why subprocess `rustup`, not `cargo --version` or manifest parsing
//!
//! Locked decision 1 of Phase 9 (plan doc): §3.4 of the compatibility
//! plan names `rustup show active-toolchain` specifically. Parsing
//! `rust-toolchain.toml` directly would re-implement rustup's resolution
//! rules (the `channel` / `path` / `components` keys, the upward search,
//! the `RUSTUP_TOOLCHAIN` override) and drift from rustup's behavior.
//! `cargo --version` reports the lihaaf-build toolchain, not the active
//! one. The subprocess is cheap (rustup reads one manifest file and
//! prints two short lines) and stays correct as rustup evolves.
//!
//! ## Why the rustc release-line fallback
//!
//! Locked decision 2 of Phase 9: the §3.3 envelope schema (Phase 8)
//! requires a non-empty `toolchain` field. Adopters who run lihaaf
//! without rustup installed (e.g. distro rustc, custom CI images) must
//! still produce a parseable envelope. The rustc release line is what
//! the v0.1 freshness machinery already captures (see
//! `crate::toolchain::Toolchain::release_line`), so the fallback adds
//! zero new subprocess shape.

use std::process::Command;

use crate::error::Error;

/// Capture the active toolchain via `rustup show active-toolchain`.
///
/// Returns the trimmed first line of rustup's stdout with any trailing
/// ` (default)` suffix dropped — e.g. `"stable-x86_64-unknown-linux-gnu
/// (default)"` becomes `"stable-x86_64-unknown-linux-gnu"`. On rustup
/// absence (spawn error) or a non-zero exit, falls back to the rustc
/// release line via `crate::toolchain::capture` so the §3.3 envelope's
/// `results.lihaaf.toolchain` field is always non-empty.
///
/// ## cwd contract
///
/// The Phase 10 driver changes cwd to `--compat-root` BEFORE calling
/// this function so `rustup show` resolves against the fork's pinned
/// `rust-toolchain.toml`. This function does not set `current_dir`
/// itself; the caller owns that.
///
/// ## Errors
///
/// Returns the propagated [`Error`] from `crate::toolchain::capture`
/// only when BOTH rustup subprocess and rustc subprocess fail — the
/// caller's environment has neither toolchain manager available, which
/// is fatal at the v0.1 freshness layer regardless of compat mode.
pub fn capture_active_toolchain() -> Result<String, Error> {
    capture_with_program("rustup")
}

/// Internal variant of [`capture_active_toolchain`] that takes the
/// program name as a parameter. The public entry calls this with
/// `"rustup"`; integration tests call it with a path that does not
/// exist to exercise the fallback path without mutating `PATH`.
pub fn capture_with_program(program: &str) -> Result<String, Error> {
    let output = Command::new(program)
        .args(["show", "active-toolchain"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            if let Some(parsed) = parse_active_toolchain(&out.stdout) {
                return Ok(parsed);
            }
            fallback_release_line()
        }
        _ => fallback_release_line(),
    }
}

/// Extract the active-toolchain identifier from rustup's stdout.
///
/// Returns the first non-empty line trimmed of leading/trailing ASCII
/// whitespace and truncated at the first ASCII space — the trim
/// drops the `(default)` annotation rustup appends when the active
/// toolchain is the user's `rustup default`. Returns `None` if every
/// line is empty after trimming (a defensive guard against future
/// rustup output formats).
pub fn parse_active_toolchain(stdout: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(stdout).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let head = match trimmed.find(' ') {
            Some(idx) => &trimmed[..idx],
            None => trimmed,
        };
        if head.is_empty() {
            continue;
        }
        return Some(head.to_string());
    }
    None
}

fn fallback_release_line() -> Result<String, Error> {
    let toolchain = crate::toolchain::capture()?;
    Ok(toolchain.release_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_default_suffix() {
        let out = b"stable-x86_64-unknown-linux-gnu (default)\n";
        assert_eq!(
            parse_active_toolchain(out).as_deref(),
            Some("stable-x86_64-unknown-linux-gnu"),
        );
    }

    #[test]
    fn parse_handles_bare_identifier_without_default_suffix() {
        let out = b"1.95.0-x86_64-unknown-linux-gnu\n";
        assert_eq!(
            parse_active_toolchain(out).as_deref(),
            Some("1.95.0-x86_64-unknown-linux-gnu"),
        );
    }

    #[test]
    fn parse_returns_none_on_empty_output() {
        assert_eq!(parse_active_toolchain(b""), None);
        assert_eq!(parse_active_toolchain(b"\n\n  \n"), None);
    }

    #[test]
    fn parse_returns_none_on_non_utf8() {
        let bad: &[u8] = &[0xff, 0xfe, b'\n'];
        assert_eq!(parse_active_toolchain(bad), None);
    }

    #[test]
    fn parse_takes_first_non_empty_line() {
        let out = b"\n  \nnightly-2026-04-01-x86_64-unknown-linux-gnu\n";
        assert_eq!(
            parse_active_toolchain(out).as_deref(),
            Some("nightly-2026-04-01-x86_64-unknown-linux-gnu"),
        );
    }
}
