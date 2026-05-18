//! Crate-wide error type.
//!
//! Two categories are represented:
//!
//! - **Internal errors** — I/O failures, parse failures, and malformed
//!   invocations that are not fixture-level outcomes. These keep enough
//!   context for a developer to debug quickly.
//! - **Session outcomes** — the `Outcome::*` values surfaced to users
//!   (`CONFIG_INVALID`, `DYLIB_BUILD_FAILED`, `DYLIB_NOT_FOUND`,
//!   `TOOLCHAIN_DRIFT`, `MANIFEST_CORRUPT`, `CLEANUP_RESIDUE`).
//!   These map to an [`crate::exit::ExitCode`] and a stable diagnostic message
//!   that `main` prints before exiting.
//!
//! ## Why one type
//!
//! `cargo-lihaaf` is one process per invocation with one `main` that has to
//! convert failures into exit codes. Splitting into per-module types adds plumbing
//! without simplifying control flow. A single `enum Error` keeps the conversion
//! straightforward and explicit.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Crate-wide error type.
///
/// `Session(_)` is the session-level path. Everything else is an internal
/// failure that bubbles up through `?` and is rendered to stderr by
/// `main` before exiting.
#[derive(Debug)]
pub enum Error {
    /// A session-level outcome. The binary's exit code is
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
    /// `Outcome::ConfigInvalid` because the latter assumes TOML parsing
    /// succeeded and points to a specific key-level issue.
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

    /// The upstream `Cargo.toml` declared a `[patch.crates-io.<self>]`
    /// entry that compat-mode's Option H self-patch policy cannot
    /// safely overwrite — the entry targets a vendored fork (`path`
    /// resolves to a non-root directory), a git source, or both.
    ///
    /// Surfaced by the compat overlay materializer's Rule 4 (REJECT)
    /// branch. The §3.3 envelope renders this as an overlay-stage
    /// error; the operator sees the structured message verbatim so
    /// they can resolve by either reshaping their upstream's
    /// `[patch.crates-io]` table or waiting for the v0.2/v1.1
    /// `--compat-allow-patch-override` escape hatch.
    ///
    /// See `src/compat/overlay.rs::apply_self_patch_policy` for the
    /// 4-rule decision tree and issues #40 / #47 for the failure
    /// shapes this policy resolves.
    CompatPatchOverrideConflict {
        /// The crate name (the upstream's `[package].name`) keying the
        /// rejected `[patch.crates-io.<crate_name>]` entry.
        crate_name: String,
        /// A `Debug` rendering of the upstream's existing entry, so the
        /// operator can see exactly what compat-mode refused to
        /// overwrite.
        upstream_entry: String,
        /// Human-readable explanation of what compat-mode would have
        /// done (Rule 1 INJECT / Rule 2 REMAP) and why the upstream's
        /// existing entry blocks it. References the v0.2/v1.1 escape
        /// hatch.
        expected_resolution: String,
    },

    /// Staged package-root mirror step failed.
    ///
    /// After the overlay manifest is written to
    /// `<upstream>/target/lihaaf-overlay/Cargo.toml`, the materializer
    /// creates symlinks (or copies on platforms where symlinks are
    /// unavailable) for each top-level entry in the upstream package
    /// directory so build scripts can read package-root files via
    /// `CARGO_MANIFEST_DIR` / cwd. This variant surfaces when an
    /// individual symlink, copy, or stale-state reconcile step fails
    /// during that pass.
    ///
    /// See `src/compat/overlay.rs::mirror_upstream_into_overlay` and
    /// issues #40 / #47 for the build-script file-access failure shapes
    /// the mirror resolves.
    OverlayMirrorFailed {
        /// The upstream entry path that was being mirrored (e.g.
        /// `/work/cxx/src` for `<upstream>/src`).
        upstream_entry: PathBuf,
        /// The destination path inside the staged overlay (e.g.
        /// `/work/cxx/target/lihaaf-overlay/src`).
        staged_target: PathBuf,
        /// Human-readable description of which stage failed (symlink
        /// creation, copy fallback, stale-state removal, post-condition
        /// assertion).
        stage: String,
        /// The OS-level I/O error, when one is in scope. `None` for
        /// post-condition assertion failures that report a structural
        /// mismatch rather than an `io::Error`.
        source: Option<io::Error>,
    },
}

/// Session outcomes reported before fixture verdicts can run.
///
/// These cover startup/setup failures plus the post-session cleanup residue flag.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// `[package.metadata.lihaaf]` missing or invalid.
    ConfigInvalid {
        /// Human-readable diagnostic naming the offending key + the
        /// allowed shape.
        message: String,
    },

    /// The dylib build returned non-zero.
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
        /// The crate name that was searched for.
        crate_name: String,
    },

    /// Captured rustc identity at fixture-dispatch time differs from the
    /// identity captured at dylib build time, on any field of the
    /// four-field drift key (release_line, host, commit_hash, sysroot).
    /// The `original` and `current` strings are pre-rendered multi-line
    /// keys (see `toolchain::format_drift_key`), so a host /
    /// commit_hash / sysroot drift produces a diagnostic that names
    /// the differing dimension rather than echoing identical release
    /// lines.
    ToolchainDrift {
        /// Rendered four-field key captured at startup.
        original: String,
        /// Rendered four-field key observed at dispatch.
        current: String,
    },

    /// One of the tracked freshness invariants drifted between the
    /// per-session snapshot and a dispatch. It maps onto `TOOLCHAIN_DRIFT`
    /// because both indicate stale-cache or stale-toolchain state.
    /// All invariants point to "the dylib or the
    /// toolchain built against is no longer the one about to
    /// be linked"). The `invariant` label names which of the four drifted
    /// in stable form (`managed_dylib_path` / `dylib_mtime` /
    /// `dylib_sha256` / `rustc_release`); the `detail` is a
    /// pre-rendered diagnostic body.
    FreshnessDrift {
        /// Stable identifier for the invariant that drifted.
        invariant: String,
        /// Pre-rendered diagnostic body — see
        /// `freshness::FreshnessFailure::detail` for the upstream renderer.
        detail: String,
    },
}

impl Outcome {
    /// Map the outcome to its exit code.
    pub fn exit_code(&self) -> crate::exit::ExitCode {
        match self {
            Self::ConfigInvalid { .. } => crate::exit::ExitCode::ConfigInvalid,
            Self::DylibBuildFailed { .. } => crate::exit::ExitCode::DylibBuildFailed,
            Self::DylibNotFound { .. } => crate::exit::ExitCode::DylibNotFound,
            Self::ToolchainDrift { .. } => crate::exit::ExitCode::ToolchainDrift,
            // Freshness drift maps to the same exit code as TOOLCHAIN_DRIFT:
            // both indicate the dylib/toolchain state changed mid-session and
            // CI scripts usually treat this as "rebuild cache and re-run."
            Self::FreshnessDrift { .. } => crate::exit::ExitCode::ToolchainDrift,
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
                // The `original` / `current` strings are multi-line
                // four-field keys. Indent every line by four spaces so
                // the rendered output keeps a clear visual hierarchy
                // and CI grep patterns can anchor on the labels rather
                // than the line shape.
                let indent = |s: &str| {
                    s.lines()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                write!(
                    f,
                    "lihaaf: rustc toolchain drifted mid-session.\n  original (at dylib build):\n{}\n  current (at dispatch):\n{}\nRe-run `cargo lihaaf` to rebuild against the current toolchain.",
                    indent(original),
                    indent(current),
                )
            }
            Self::FreshnessDrift { invariant, detail } => {
                write!(
                    f,
                    "lihaaf: freshness invariant `{invariant}` drifted mid-session.\n  {detail}\nRe-run `cargo lihaaf` to rebuild against the current dylib + toolchain."
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
            Self::CompatPatchOverrideConflict {
                crate_name,
                upstream_entry,
                expected_resolution,
            } => {
                write!(
                    f,
                    "compat overlay: upstream [patch.crates-io.{crate_name}] entry {upstream_entry} cannot be safely overwritten.\n  {expected_resolution}"
                )
            }
            Self::OverlayMirrorFailed {
                upstream_entry,
                staged_target,
                stage,
                source,
            } => match source {
                Some(e) => write!(
                    f,
                    "compat overlay mirror failed in stage `{stage}` mirroring `{}` → `{}` ({e})",
                    upstream_entry.display(),
                    staged_target.display()
                ),
                None => write!(
                    f,
                    "compat overlay mirror failed in stage `{stage}` mirroring `{}` → `{}`",
                    upstream_entry.display(),
                    staged_target.display()
                ),
            },
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::SubprocessSpawn { source, .. } => Some(source),
            Self::OverlayMirrorFailed {
                source: Some(s), ..
            } => Some(s),
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

    /// Convenience constructor for [`Error::OverlayMirrorFailed`].
    ///
    /// `stage` is a short human-readable label naming which step failed
    /// (e.g. `"symlink"`, `"copy-fallback"`, `"stale-removal"`,
    /// `"post-condition-assertion"`); see
    /// `src/compat/overlay.rs::mirror_upstream_into_overlay` for the
    /// enumerated stages.
    pub fn overlay_mirror_failed(
        upstream_entry: PathBuf,
        staged_target: PathBuf,
        stage: impl Into<String>,
        source: Option<io::Error>,
    ) -> Self {
        Self::OverlayMirrorFailed {
            upstream_entry,
            staged_target,
            stage: stage.into(),
            source,
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
#[allow(dead_code)] // exported convenience alias; callers spell `Result<_, Error>` directly.
pub type Result<T> = std::result::Result<T, Error>;
