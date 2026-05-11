//! Crate-wide error type.
//!
//! Two distinct categories live here:
//!
//! - **Internal errors** — I/O failures, parse failures, malformed
//!   invocations the binary has to handle but does not classify as a
//!   spec-level session outcome. These are flat string-style errors with
//!   enough context for a developer to debug.
//! - **Session outcomes** — the `Outcome::*` codes from spec §10.2
//!   (`CONFIG_INVALID`, `DYLIB_BUILD_FAILED`, `DYLIB_NOT_FOUND`,
//!   `TOOLCHAIN_DRIFT`, `MANIFEST_CORRUPT`, `CLEANUP_RESIDUE`). These
//!   carry an [`crate::exit::ExitCode`] mapping and a human-readable
//!   diagnostic that the binary surfaces verbatim before exiting.
//!
//! ## Why one type
//!
//! `cargo-lihaaf` is one process per invocation, with one `main` that has
//! to translate failures into exit codes per spec §10.3. Splitting into
//! per-module error types and re-exporting through `From` impls would
//! buy nothing — every call site converges on the same `main` anyway.
//! A single `enum Error` with explicit variants for the
//! session-classification path keeps the conversion one match wide.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Crate-wide error type.
///
/// `Session(_)` is the spec-level path. Everything else is an internal
/// failure that bubbles up through `?` and is rendered to stderr by
/// `main` before exiting.
#[derive(Debug)]
pub enum Error {
    /// A spec-level session outcome (§10.2). The binary's exit code is
    /// derived from the wrapped `Outcome::exit_code()`; the diagnostic
    /// is printed to stderr verbatim.
    Session(Outcome),

    /// Filesystem / I/O failure. The path that was being touched is
    /// preserved so error messages name what failed.
    Io {
        /// The OS error.
        source: io::Error,
        /// What was being attempted ("read manifest", "spawn rustc", …).
        context: String,
        /// Optional path, when one is in scope.
        path: Option<PathBuf>,
    },

    /// `Cargo.toml` could not be parsed as TOML. This is distinct from
    /// `Outcome::ConfigInvalid` because the latter assumes the TOML
    /// parsed and a specific key was wrong.
    TomlParse {
        /// The path that failed to parse.
        path: PathBuf,
        /// Human-readable parse error.
        message: String,
    },

    /// JSON decode failure (manifest reads, `cargo --message-format=json`
    /// stream, etc.). Treated as internal — the user typically did not
    /// produce the JSON.
    JsonParse {
        /// What was being parsed.
        context: String,
        /// Decoder's message.
        message: String,
    },

    /// Subprocess (`cargo`, `rustc`) could not be spawned at all
    /// (binary not found, permission denied). Distinct from a non-zero
    /// exit, which is a normal session outcome.
    SubprocessSpawn {
        /// The program name.
        program: String,
        /// The OS error.
        source: io::Error,
    },

    /// CLI argument parsing failed. Carries clap's exit code so the
    /// binary can exit transparently.
    Cli {
        /// `clap`'s recommended exit code (typically `2`).
        clap_exit_code: i32,
        /// The error message clap already printed (kept for tests).
        message: String,
    },
}

/// Spec §10.2 session outcomes.
///
/// These are the "session-level" failures: things that happen before any
/// per-fixture verdict could be reported, plus the post-session
/// `CleanupResidue` flag.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// `[package.metadata.lihaaf]` missing or invalid (§3.4).
    ConfigInvalid {
        /// Human-readable diagnostic naming the offending key + the
        /// allowed shape.
        message: String,
    },

    /// The dylib build (§4.2) returned non-zero.
    DylibBuildFailed {
        /// The cargo invocation as a single line for the user to copy.
        invocation: String,
        /// Captured stderr verbatim.
        stderr: String,
    },

    /// cargo succeeded but no `compiler-artifact` message named the
    /// dylib whose `target.name` matches `dylib_crate`.
    DylibNotFound {
        /// The cargo invocation, for repro.
        invocation: String,
        /// The crate name we were searching for.
        crate_name: String,
    },

    /// rustc release at fixture-dispatch time differs from the version
    /// captured at dylib-build time (§4.6).
    ToolchainDrift {
        /// The version captured at startup.
        original: String,
        /// The version observed at dispatch.
        current: String,
    },
}

impl Outcome {
    /// Map the outcome to its exit code per spec §10.3.
    pub fn exit_code(&self) -> crate::exit::ExitCode {
        match self {
            Self::ConfigInvalid { .. } => crate::exit::ExitCode::ConfigInvalid,
            Self::DylibBuildFailed { .. } => crate::exit::ExitCode::DylibBuildFailed,
            Self::DylibNotFound { .. } => crate::exit::ExitCode::DylibNotFound,
            Self::ToolchainDrift { .. } => crate::exit::ExitCode::ToolchainDrift,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigInvalid { message } => {
                write!(f, "lihaaf: configuration invalid:\n{message}")
            }
            Self::DylibBuildFailed { invocation, stderr } => {
                write!(
                    f,
                    "lihaaf: dylib build failed.\n  invocation: {invocation}\n  cargo stderr:\n{stderr}"
                )
            }
            Self::DylibNotFound {
                invocation,
                crate_name,
            } => {
                write!(
                    f,
                    "lihaaf: cargo succeeded but emitted no `compiler-artifact` message naming the `dylib` artifact for `{crate_name}`.\n  invocation: {invocation}\n  Re-check `dylib_crate` in `[package.metadata.lihaaf]` matches a workspace member."
                )
            }
            Self::ToolchainDrift { original, current } => {
                write!(
                    f,
                    "lihaaf: rustc version drifted mid-session.\n  original (at dylib build): {original}\n  current (at dispatch):     {current}\nRe-run `cargo lihaaf` to rebuild against the current toolchain."
                )
            }
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(o) => write!(f, "{o}"),
            Self::Io {
                source,
                context,
                path,
            } => match path {
                Some(p) => write!(f, "{context}: {} ({source})", p.display()),
                None => write!(f, "{context} ({source})"),
            },
            Self::TomlParse { path, message } => {
                write!(f, "failed to parse {}: {message}", path.display())
            }
            Self::JsonParse { context, message } => {
                write!(f, "failed to parse JSON ({context}): {message}")
            }
            Self::SubprocessSpawn { program, source } => {
                write!(f, "failed to spawn `{program}`: {source}")
            }
            Self::Cli { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::SubprocessSpawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Error {
    /// Convenience: wrap an `io::Error` with context + optional path.
    pub fn io(source: io::Error, context: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self::Io {
            source,
            context: context.into(),
            path,
        }
    }
}

impl Error {
    /// Exit code reported by the binary when this error is propagated.
    /// CLI errors carry clap's recommended code (typically 2). Other
    /// internal errors map to `CONFIG_INVALID` (64) — they happened
    /// before fixtures could run and the user has no fixture verdicts
    /// to interpret.
    pub fn exit_code(&self) -> crate::exit::ExitCode {
        match self {
            Self::Session(o) => o.exit_code(),
            Self::Cli {
                clap_exit_code: 0, ..
            } => crate::exit::ExitCode::Ok,
            _ => crate::exit::ExitCode::ConfigInvalid,
        }
    }
}

/// `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
