//! CLI argument parsing.
//!
//! CLI parsing for the `cargo lihaaf` command.
//!
//! Each field maps directly to a subcommand flag and preserves the
//! documented default behavior.
//!
//! ## Why clap derive
//!
//! The flag set is small enough that hand-rolling argv parsing would
//! work, but `clap` carries the `--help` / `--version` rendering and
//! the validation that would otherwise need to be re-implemented (positive integer for
//! `-j`, etc.).

use std::path::PathBuf;

use clap::Parser;

use crate::error::Error;

/// Parsed CLI arguments.
///
/// Each field maps directly to a CLI flag.
/// Defaults preserve the conservative "non-`--bless`,
/// non-`--keep-output`, non-`--use-symlink`" posture.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "cargo-lihaaf",
    bin_name = "cargo lihaaf",
    version,
    about = "Fast, parallel, non-flaky proc-macro test harness",
    long_about = "Fast, parallel, non-flaky proc-macro test harness for compile-fail \
                  and compile-pass fixtures. The consumer crate is built once as a \
                  Rust dynamic library at session startup; each fixture is a \
                  per-fixture rustc invocation that links the dylib via --extern. \
                  See `target/lihaaf/manifest.json` for the dylib metadata after \
                  the first run. Configuration: `[package.metadata.lihaaf]` in the \
                  consumer's Cargo.toml.\n\n\
                  New setup: add `dylib_crate`, `extern_crates`, and `fixture_dirs` \
                  to `[package.metadata.lihaaf]`, then place `.rs` fixtures and \
                  matching `.stderr` snapshots under the configured fixture dirs. \
                  Run `cargo lihaaf --bless` once to create or refresh snapshots, \
                  then `cargo lihaaf` in CI.\n\n\
                  Trybuild migration note: most conversions only need `dev_deps`; \
                  for split metadata/dylib crates whose fixtures cannot resolve \
                  metadata-side dev-deps, set `build_targets = [\"tests\"]` on \
                  each suite that needs staged dev-dep collection. See the \
                  repository migration guide at `docs/migrating-from-trybuild.md`."
)]
pub struct Cli {
    /// Overwrite `.stderr` snapshots whose normalized output differs from disk.
    ///
    /// Use when you intentionally change a proc macro's error output or when a
    /// rustc toolchain version changes how compiler diagnostics are formatted.
    /// Always run `git diff` after blessing to verify the changes match your
    /// macro edits, rather than masking an unintended compiler regression.
    ///
    /// Equivalent env: `LIHAAF_OVERWRITE=1`.
    #[arg(long)]
    pub bless: bool,

    /// Switch the binary into compat mode.
    ///
    /// Compat mode is a migration and validation workflow for proc-macro crates
    /// that already use trybuild. It executes the existing trybuild suite via
    /// `cargo test` as a baseline, statically discovers fixtures via AST analysis,
    /// runs them under a staged lihaaf overlay, and aggregates outcomes into a
    /// byte-deterministic comparison report.
    ///
    /// This allows verifying diagnostic output and behavior parity across toolchains
    /// prior to committing to a full migration.
    ///
    /// Under compat mode, only `--compat*` flags (e.g. `--compat-root`, `--compat-report`,
    /// `--compat-filter`, `--compat-manifest`, etc.) govern manifest and fixture selection.
    /// Standard flags `--filter` and `--manifest-path` are rejected as mode errors,
    /// while formatting, caching, and execution flags (`--bless`, `--no-cache`, `-q`,
    /// `-v`, `--use-symlink`, `--keep-output`, `-j`) remain valid as pass-throughs.
    #[arg(long)]
    pub compat: bool,

    /// Optional in compat mode. JSON array passed verbatim as argv to
    /// the baseline `cargo test` invocation (no shell). Defaults to
    /// `["cargo","test"]` when not specified.
    #[arg(long, value_name = "JSON")]
    pub compat_cargo_test_argv: Option<String>,

    /// Recorded in the §3.3 envelope's `commit` field for traceability.
    #[arg(long, value_name = "SHA")]
    pub compat_commit: Option<String>,

    /// Substring filter on fixture paths in compat mode (shadows `--filter`).
    #[arg(long, value_name = "SUBSTR")]
    pub compat_filter: Vec<String>,

    /// Sibling-manifest path override (shadows `--manifest-path`).
    #[arg(long, value_name = "PATH")]
    pub compat_manifest: Option<PathBuf>,

    /// Required when `--compat` is set. §3.3 envelope output path.
    #[arg(long, value_name = "PATH")]
    pub compat_report: Option<PathBuf>,

    /// Required when `--compat` is set. Target crate checkout path.
    #[arg(long, value_name = "DIR")]
    pub compat_root: Option<PathBuf>,

    /// Compat-mode workspace-member package selector. When set, the
    /// upstream manifest is resolved from the workspace rooted at
    /// `--compat-root` by matching `<package>` against each member's
    /// `[package].name`. Required when `--compat-root` points at a
    /// workspace ROOT that declares `[workspace]` without `[package]`;
    /// rejected otherwise (see `validate_mode_consistency`).
    ///
    /// The short form `-p` mirrors cargo's `-p` convention. Multi-valued
    /// is rejected at parse time (single package per invocation); the
    /// field type `Option<String>` (not `Vec<String>`) makes clap reject
    /// a second occurrence at parse time.
    ///
    /// Mutually exclusive with `--compat-manifest` (which supplies an
    /// explicit manifest path, bypassing the workspace-member resolver).
    /// See `docs/spec/lihaaf-v0.1.md` §8.2 and `docs/compatibility-plan.md`
    /// §3.2.3 ("Workspace-member entry via `--package`") for the
    /// adopter-facing surface.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE", value_parser = parse_compat_package)]
    pub compat_package: Option<String>,

    /// Additional fully-qualified macro paths the §3.2.1 AST walk treats as
    /// aliases for `trybuild::TestCases::new()`. Repeatable; OR'd.
    #[arg(long, value_name = "PATH")]
    pub compat_trybuild_macro: Vec<String>,

    /// Run only fixtures whose relative path contains the substring.
    ///
    /// Substring match is case-sensitive. Multiple `--filter` flags are OR'd.
    ///
    /// # Examples
    ///
    /// - `--filter ui` runs fixtures containing `ui` in their relative path.
    /// - `--filter pass --filter fail` runs fixtures containing either `pass` OR `fail`.
    #[arg(long)]
    pub filter: Vec<String>,

    /// Override the worker parallelism cap. The RAM cap still applies —
    /// this override does not bypass it.
    ///
    /// `-j 0` is rejected at parse time; explicit values are required.
    /// Omit the flag to use the default.
    #[arg(short = 'j', long = "jobs", value_parser = parse_jobs)]
    pub jobs: Option<u32>,

    /// Limit the run to the named suite(s). Repeatable. Without
    /// `--suite`, every defined suite runs in declared metadata order
    /// (the implicit `default` suite first, then each
    /// `[[package.metadata.lihaaf.suite]]` entry in source order).
    ///
    /// `--suite default` selects the implicit suite built from the
    /// top-level `[package.metadata.lihaaf]` table; named suites use
    /// their declared `name`. Unknown names are rejected at session
    /// startup with the list of valid names.
    #[arg(long, value_name = "NAME")]
    pub suite: Vec<String>,

    /// Force a fresh dylib build, ignoring any existing manifest.
    /// Equivalent to deleting `target/lihaaf/manifest.json` before
    /// invocation.
    #[arg(long)]
    pub no_cache: bool,

    /// Override the consumer `Cargo.toml` location. Default is cargo's
    /// normal "current directory + parent walk" lookup.
    #[arg(long, value_name = "PATH")]
    pub manifest_path: Option<PathBuf>,

    /// Print the fixtures the harness would run, one relative path per
    /// line, and exit 0. Does not build the dylib or invoke rustc.
    /// Composable with `--filter`.
    #[arg(long)]
    pub list: bool,

    /// Suppress per-fixture progress; only the aggregate report and
    /// non-OK verdict lines print.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Print each fixture's rustc command before running it, plus
    /// captured stderr regardless of normalization outcome.
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Skip the lihaaf-managed dylib copy; create a symbolic link instead.
    ///
    /// Saves ~30 MB disk space per run and several hundred milliseconds by skipping
    /// the file copy operation.
    ///
    /// # Safety Assertion
    ///
    /// The caller must ensure that **no concurrent cargo activity** (such as IDE
    /// background compilation, cargo check/build/test in another terminal, or
    /// concurrent CI jobs) is modifying the target directory. If cargo rebuilds or
    /// overwrites the underlying dylib during test execution, parallel rustc workers
    /// may encounter compilation errors, undefined behavior, or link failures.
    #[arg(long)]
    pub use_symlink: bool,

    /// Preserve per-fixture work directories and generated staging files after verdict capture.
    ///
    /// # What is preserved
    ///
    /// - **Standard runs:** Preserves the temporary compilation work directories for each
    ///   fixture (containing compiled artifacts, dependency info, and transient objects)
    ///   under the session's temporary directory.
    /// - **Compat-mode runs:** Preserves all generated compat sidecars, staged/converted
    ///   fixture files, and the generated manifest overlay under `target/lihaaf-overlay/`
    ///   instead of cleaning them up upon completion or panic.
    ///
    /// # Debugging Use Cases
    ///
    /// Useful for local development when a fixture fails to compile or links incorrectly,
    /// allowing developers to inspect intermediate compiler outputs or manually run the
    /// generated `rustc` command in the preserved work directory.
    ///
    /// Note: This is a local-development escape hatch only — never set this flag in CI,
    /// as it prevents cleanup and will leak directories/files, consuming disk space.
    #[arg(long)]
    pub keep_output: bool,

    /// Internal flag — NOT exposed on argv. Compat mode's driver sets
    /// this to `true` when invoking the inner `session::run` so the
    /// inner session constructs a [`crate::normalize::NormalizationContext`]
    /// with `compat_short_cargo = true` and the §3.2.2 short-form
    /// `$CARGO/<crate>-<ver>/...` rewrite fires for compat snapshots.
    ///
    /// The clap `skip` attribute keeps the field off the CLI surface
    /// entirely; clap does not generate a flag for it and `parse_from`
    /// always leaves it at the default `false`. The field is set
    /// programmatically by `compat::mod::build_inner_cli` so adopters
    /// using `cargo lihaaf --compat` do not need to know it exists.
    #[arg(skip)]
    pub(crate) inner_compat_normalize: bool,
}

/// Reject empty `--package` values at parse time. clap's default
/// `Option<String>` value parser accepts
/// empty strings; we tighten to "non-empty required" so the bad
/// invocation fails immediately with a clap error rather than reaching
/// the resolver and surfacing as a no-match diagnostic that doesn't
/// name the empty-string root cause. Further package-name validation
/// (cargo's `[a-zA-Z0-9_-]+` rule) lives at resolver time; cargo would
/// itself have rejected the invalid name at workspace-load time, so an
/// adopter passing a malformed name reaches the no-match diagnostic.
fn parse_compat_package(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("`--package` requires a non-empty package name".to_string());
    }
    Ok(s.to_string())
}

/// Reject `-j 0` at parse time. The default
/// `value_parser` for `u32` accepts any non-negative integer, including
/// `0`; this is tightened to "positive integer required" so the bad
/// invocation fails immediately with a clap error rather than silently
/// being clamped downstream.
fn parse_jobs(s: &str) -> Result<u32, String> {
    let n: u32 = s
        .parse()
        .map_err(|_| format!("`{s}` is not a non-negative integer"))?;
    if n == 0 {
        return Err(
            "must be a positive integer (`-j 0` is rejected; omit `-j` to use the default)"
                .to_string(),
        );
    }
    Ok(n)
}

/// Parse `argv` (already stripped of the cargo subcommand prefix) into a
/// [`Cli`].
///
/// After clap-derive parsing succeeds, the (`pub(crate)`)
/// `validate_mode_consistency` method is run to enforce the mode-error
/// matrix for compat vs non-compat flag combinations. Both clap errors
/// and validator errors return [`Error::Cli`] with `clap_exit_code = 2`;
/// mode-error diagnostics are printed to stderr before the typed error
/// bubbles up so the user sees the directed message even in pipelines
/// that discard the returned error body.
pub fn parse_from(argv: Vec<String>) -> Result<Cli, Error> {
    use clap::error::ErrorKind;
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) => {
            // For `--help` / `--version`, clap returns a "graceful" error
            // and should print and exit 0.
            let kind = e.kind();
            let exit_code = match kind {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 2,
            };
            // clap prints the message itself when `print()` is called.
            let message = e.to_string();
            // Pre-print so the caller sees the message even when
            // bubbling through the typed error.
            let _ = e.print();
            return Err(Error::Cli {
                clap_exit_code: exit_code,
                message,
            });
        }
    };

    // Mode-error matrix. Validation runs after clap so every parsed
    // field is observable and the diagnostic can name the replacement
    // flag instead of clap's generic "cannot be used with" message.
    if let Err(e) = cli.validate_mode_consistency() {
        if let Error::Cli { message, .. } = &e {
            eprintln!("{message}");
        }
        return Err(e);
    }

    Ok(cli)
}

impl Cli {
    /// True when the env var `LIHAAF_OVERWRITE=1` should be honored as
    /// equivalent to `--bless`.
    pub fn effective_bless(&self) -> bool {
        if self.bless {
            return true;
        }
        std::env::var("LIHAAF_OVERWRITE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// Reject inconsistent mode combinations between `--compat` and the
    /// v0.1 surface.
    ///
    /// Called by [`parse_from`] after clap parsing succeeds AND by
    /// [`crate::session::run`] at the top of its dispatch — both entry
    /// points must enforce the matrix so a Rust caller that constructs
    /// `Cli` via direct field initialization cannot bypass it. The
    /// validator is idempotent (no side effects, pure inspection), so
    /// a double call from `parse_from` → `run` is safe. The validator
    /// returns directed diagnostics:
    ///
    /// - In compat mode, `--filter` and `--manifest-path` are mode
    ///   errors and the message names the replacement (`--compat-filter`
    ///   / `--compat-manifest`).
    /// - In compat mode, `--compat-root` and `--compat-report` are
    ///   required; their absence is a mode error.
    /// - Outside compat mode, any `--compat*` flag is a mode error.
    ///
    /// The implementation is a hand-rolled validator rather than clap's
    /// `requires` / `conflicts_with` annotations because the spec
    /// requires the diagnostic to name the replacement flag. clap's
    /// generic "the argument '--filter' cannot be used with '--compat'"
    /// message does not give the adopter that pointer.
    pub(crate) fn validate_mode_consistency(&self) -> Result<(), Error> {
        if self.compat {
            // In compat mode: shadowed flags are mode errors.
            if !self.filter.is_empty() {
                return Err(cli_mode_error(
                    "--filter",
                    "--compat-filter",
                    "compat mode owns the fixture-path filter surface",
                ));
            }
            if self.manifest_path.is_some() {
                return Err(cli_mode_error(
                    "--manifest-path",
                    "--compat-manifest",
                    "compat mode owns the manifest-path surface",
                ));
            }
            // In compat mode: required compat flags must be present.
            if self.compat_root.is_none() {
                return Err(missing_required_compat_flag("--compat-root"));
            }
            if self.compat_report.is_none() {
                return Err(missing_required_compat_flag("--compat-report"));
            }
            // `--package` and `--compat-manifest` are mutually exclusive.
            // The two flags address opposite ends of the same problem space:
            // `--compat-manifest` supplies an explicit manifest path
            // directly to compat mode, bypassing the workspace-member
            // resolver; `--package` invokes the resolver to find the
            // member from a workspace root. Combining them is
            // incoherent — clap's generic conflicts-with would say
            // "cannot be used with" but not explain why; we surface a
            // directed diagnostic instead.
            if self.compat_package.is_some() && self.compat_manifest.is_some() {
                return Err(Error::Cli {
                    clap_exit_code: 2,
                    message: "error: `--package` and `--compat-manifest` cannot be combined: \
                         `--compat-manifest` supplies an explicit manifest path directly to \
                         compat mode, while `--package` invokes the workspace-member resolver. \
                         Use one or the other."
                        .to_string(),
                });
            }
        } else {
            // Outside compat mode: every --compat-* flag is a mode error.
            // Order matters here only for the first-error-wins diagnostic;
            // alphabetical keeps the surface predictable.
            if self.compat_cargo_test_argv.is_some() {
                return Err(non_compat_mode_error("--compat-cargo-test-argv"));
            }
            if self.compat_commit.is_some() {
                return Err(non_compat_mode_error("--compat-commit"));
            }
            if !self.compat_filter.is_empty() {
                return Err(non_compat_mode_error("--compat-filter"));
            }
            if self.compat_manifest.is_some() {
                return Err(non_compat_mode_error("--compat-manifest"));
            }
            // `--package` outside compat mode is a mode error. Surface via
            // the existing `non_compat_mode_error` helper so the diagnostic
            // shape matches the rest of the matrix.
            if self.compat_package.is_some() {
                return Err(non_compat_mode_error("--package"));
            }
            if self.compat_report.is_some() {
                return Err(non_compat_mode_error("--compat-report"));
            }
            if self.compat_root.is_some() {
                return Err(non_compat_mode_error("--compat-root"));
            }
            if !self.compat_trybuild_macro.is_empty() {
                return Err(non_compat_mode_error("--compat-trybuild-macro"));
            }
        }
        Ok(())
    }
}

/// Build the `Error::Cli` for a compat-mode shadowed-flag rejection.
///
/// `bare_flag` is the v0.1 flag that the user passed; `compat_flag` is
/// the compat-mode replacement; `rationale` is the short phrase that
/// explains *why* the v0.1 flag is rejected. The rendered diagnostic
/// names all three so the user can fix the invocation without reading
/// the spec.
fn cli_mode_error(bare_flag: &str, compat_flag: &str, rationale: &str) -> Error {
    Error::Cli {
        clap_exit_code: 2,
        message: format!(
            "error: `{bare_flag}` cannot be combined with `--compat`: {rationale}. \
             Use `{compat_flag}` instead."
        ),
    }
}

/// Build the `Error::Cli` for a non-compat-mode rejection of a
/// `--compat*` flag.
///
/// The diagnostic names the offending flag and points the user at the
/// `--compat` switch as the prerequisite.
fn non_compat_mode_error(flag: &str) -> Error {
    Error::Cli {
        clap_exit_code: 2,
        message: format!(
            "error: `{flag}` requires `--compat` (compat-mode-only flag). \
             Pass `--compat` to switch the binary into compat mode, or remove `{flag}`."
        ),
    }
}

/// Build the `Error::Cli` for a missing required `--compat*` flag in
/// compat mode.
fn missing_required_compat_flag(flag: &str) -> Error {
    Error::Cli {
        clap_exit_code: 2,
        message: format!(
            "error: `{flag}` is required when `--compat` is set. \
             See `cargo lihaaf --help` for the compat-mode invocation shape."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let argv: Vec<String> = std::iter::once("cargo-lihaaf".to_owned())
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        parse_from(argv).expect("parse must succeed")
    }

    #[test]
    fn defaults_are_safe_posture() {
        let c = parse(&[]);
        assert!(!c.bless);
        assert!(c.filter.is_empty());
        assert!(c.jobs.is_none());
        assert!(!c.no_cache);
        assert!(c.manifest_path.is_none());
        assert!(!c.list);
        assert!(!c.quiet);
        assert!(!c.verbose);
        assert!(!c.use_symlink);
        assert!(!c.keep_output);
    }

    /// Every compat field defaults to its empty / `None` / `false`
    /// posture when no `--compat*` flag is on the command line. The
    /// v0.1 surface stays in the "compat is off" branch of the
    /// validator, so adopters who never opt in see no behavioral
    /// drift.
    #[test]
    fn defaults_for_compat_fields_are_safe_posture() {
        let c = parse(&[]);
        assert!(!c.compat);
        assert!(c.compat_cargo_test_argv.is_none());
        assert!(c.compat_commit.is_none());
        assert!(c.compat_filter.is_empty());
        assert!(c.compat_manifest.is_none());
        assert!(c.compat_package.is_none());
        assert!(c.compat_report.is_none());
        assert!(c.compat_root.is_none());
        assert!(c.compat_trybuild_macro.is_empty());
    }

    #[test]
    fn filter_accumulates() {
        let c = parse(&["--filter", "phase7", "--filter", "phase8"]);
        assert_eq!(c.filter, vec!["phase7".to_string(), "phase8".to_string()]);
    }

    #[test]
    fn jobs_short_long() {
        assert_eq!(parse(&["-j", "4"]).jobs, Some(4));
        assert_eq!(parse(&["--jobs", "8"]).jobs, Some(8));
    }

    #[test]
    fn jobs_zero_is_rejected_per_spec_section_5_2() {
        // `-j 0` is rejected. The clap value parser hard-fails rather
        // than silently coercing.
        let argv: Vec<String> = ["cargo-lihaaf", "-j", "0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse_from(argv).expect_err("`-j 0` must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message.contains("positive integer"),
                    "diagnostic must explain the requirement: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn jobs_long_form_zero_also_rejected() {
        let argv: Vec<String> = ["cargo-lihaaf", "--jobs", "0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_from(argv).is_err());
    }

    /// Short `-p` form parses into `compat_package`.
    ///
    /// The clap-derive `short = 'p'` attribute makes the CLI accept the
    /// short form. This test pins that the parsed `Cli` carries the
    /// package name on `compat_package` (not on any unrelated field) so
    /// a future refactor that drops the `short = 'p'` attribute (or
    /// renames the field) trips here immediately.
    #[test]
    fn cli_parses_short_p_flag() {
        let c = parse(&[
            "--compat",
            "--compat-root",
            "/tmp/ws",
            "--compat-report",
            "/tmp/r.json",
            "-p",
            "axum-macros",
        ]);
        assert_eq!(c.compat_package.as_deref(), Some("axum-macros"));
    }

    /// Long `--package` form parses into `compat_package`.
    /// Mirror of `cli_parses_short_p_flag` for the long form.
    #[test]
    fn cli_parses_long_package_flag() {
        let c = parse(&[
            "--compat",
            "--compat-root",
            "/tmp/ws",
            "--compat-report",
            "/tmp/r.json",
            "--package",
            "axum-macros",
        ]);
        assert_eq!(c.compat_package.as_deref(), Some("axum-macros"));
    }

    /// `--package ""` is rejected at parse time.
    ///
    /// The `parse_compat_package` value parser tightens clap's default
    /// `Option<String>` parser (which would accept the empty string) so
    /// the bad invocation fails immediately with a clap error instead
    /// of reaching the resolver and surfacing as a no-match diagnostic.
    /// The diagnostic must name `--package` so the adopter knows which
    /// flag is at fault.
    #[test]
    fn cli_rejects_empty_package_name() {
        let argv: Vec<String> = [
            "cargo-lihaaf",
            "--compat",
            "--compat-root",
            "/tmp/ws",
            "--compat-report",
            "/tmp/r.json",
            "-p",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let err = parse_from(argv).expect_err("empty `--package` must be rejected");
        match err {
            Error::Cli { message, .. } => assert!(
                message.contains("`--package` requires a non-empty package name"),
                "diagnostic must name the requirement: {message}"
            ),
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    /// `--package` outside compat mode is a mode error.
    ///
    /// Symmetric with the rest of the compat-mode matrix; the
    /// `validate_mode_consistency` validator owns the rejection per the
    /// existing pattern. The diagnostic uses `non_compat_mode_error`
    /// (same helper as `--compat-manifest`, `--compat-report`, etc.) so
    /// the message shape is uniform across the matrix.
    #[test]
    fn cli_rejects_package_outside_compat_mode() {
        let argv: Vec<String> = ["cargo-lihaaf", "-p", "axum-macros"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse_from(argv).expect_err("`--package` outside compat mode must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message.contains("`--package` requires `--compat`"),
                    "diagnostic must name the requirement: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    /// `--package` and `--compat-manifest` are mutually exclusive.
    /// Combining them is incoherent (the two flags address opposite ends
    /// of the manifest-resolution problem); the validator surfaces a
    /// directed diagnostic naming both flags.
    #[test]
    fn cli_rejects_package_with_compat_manifest() {
        let argv: Vec<String> = [
            "cargo-lihaaf",
            "--compat",
            "--compat-root",
            "/x",
            "--compat-manifest",
            "/y/Cargo.toml",
            "-p",
            "foo",
            "--compat-report",
            "/z",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let err = parse_from(argv).expect_err("`--package` + `--compat-manifest` must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message
                        .contains("cannot be combined: `--compat-manifest` supplies an explicit"),
                    "diagnostic must explain mutual exclusion: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn bless_via_env_when_flag_absent() {
        // Env reads happen at call time; pollution across tests must be avoided.
        // SAFETY: `set_var` is `unsafe` in 2024 edition, but tests run
        // single-threaded by default; the var is restored below.
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        unsafe {
            std::env::set_var("LIHAAF_OVERWRITE", "1");
        }
        let c = parse(&[]);
        assert!(c.effective_bless());
        // Restore.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("LIHAAF_OVERWRITE", v),
                None => std::env::remove_var("LIHAAF_OVERWRITE"),
            }
        }
    }
}
