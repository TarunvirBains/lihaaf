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
                  consumer's Cargo.toml."
)]
pub struct Cli {
    /// Overwrite mismatched `.stderr` snapshots from the current rustc
    /// output (destructive). Requires `--filter <SUBSTR>` (use
    /// `--filter ""` to opt into bulk blessing). Equivalent env:
    /// `LIHAAF_OVERWRITE=1`.
    ///
    /// `--bless` is destructive. It overwrites your checked-in snapshot
    /// files in-place with whatever the current rustc emits. The
    /// classical failure mode is:
    ///
    ///   1. A real regression breaks the snapshot.
    ///   2. The author runs `--bless` to "fix" the test.
    ///   3. The new snapshot reflects the broken output. The test passes.
    ///   4. The regression ships.
    ///
    /// This is especially dangerous when an AI agent is running the
    /// test suite and treats `--bless` as a way to make red tests green.
    ///
    /// Before passing `--bless`, verify the new output is correct by:
    ///
    ///   - `cargo lihaaf --filter <fixture>` (without `--bless`, see
    ///     the diff)
    ///   - manually reading the diff against the expected output
    ///   - confirming the fixture's `.rs` file has a change that
    ///     justifies the snapshot drift (lihaaf will REJECT `--bless`
    ///     on a fixture whose `.rs` file is unchanged from `HEAD` and
    ///     emit a `BLESS_SKIPPED` verdict instead)
    ///
    /// `--bless` requires `--filter` to bound the blast radius.
    /// Bulk-blessing is intentionally hard: pass `--filter ""` to
    /// match every fixture.
    #[arg(
        long,
        help = "Overwrite mismatched snapshots from current rustc output (destructive). Requires --filter.",
        long_help = "Overwrite mismatched .stderr snapshots from the current rustc output.\n\
                     \n\
                     WARNING: --bless is destructive. It overwrites your checked-in\n\
                     snapshot files in-place with whatever the current rustc emits. The\n\
                     classical failure mode is:\n\
                     \n  \
                       1. A real regression breaks the snapshot.\n  \
                       2. The author runs --bless to \"fix\" the test.\n  \
                       3. The new snapshot reflects the broken output. The test passes.\n  \
                       4. The regression ships.\n\
                     \n\
                     This is especially dangerous when an AI agent is running the test\n\
                     suite and treats --bless as a way to make red tests green.\n\
                     \n\
                     Before passing --bless, verify the new output is correct by:\n  \
                       - cargo lihaaf --filter <fixture>    # without --bless, see the diff\n  \
                       - manually reading the diff against the expected output\n  \
                       - confirming the fixture's .rs file has a change that justifies the\n    \
                         snapshot drift (lihaaf will REJECT --bless on a fixture whose .rs\n    \
                         file is unchanged from HEAD and emit a BLESS_SKIPPED verdict\n    \
                         instead).\n\
                     \n\
                     --bless requires --filter to bound the blast radius. Bulk-blessing is\n\
                     intentionally hard; pass --filter \"\" to opt into matching every\n\
                     fixture."
    )]
    pub bless: bool,

    /// Run only fixtures whose relative path contains the substring.
    /// Multiple `--filter` flags are OR'd. Substring match is
    /// case-sensitive.
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

    /// Skip the lihaaf-managed dylib copy; create a symbolic link
    /// instead. Saves ~30 MB disk + ~few hundred ms; the caller asserts
    /// no concurrent cargo activity will modify `target/`.
    #[arg(long)]
    pub use_symlink: bool,

    /// Preserve per-fixture work directories after verdict capture.
    /// Local-development escape hatch only — never set in CI.
    #[arg(long)]
    pub keep_output: bool,
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
/// Post-parse validation enforces the `--bless` blast-radius rule:
/// `--bless` without a `--filter` is rejected via
/// [`Cli::validate_bless_requires_filter`]. See that method's rustdoc
/// for the rationale and the empty-string-filter escape hatch.
pub fn parse_from(argv: Vec<String>) -> Result<Cli, Error> {
    use clap::error::ErrorKind;
    match Cli::try_parse_from(argv) {
        Ok(cli) => {
            cli.validate_bless_requires_filter()?;
            Ok(cli)
        }
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
            Err(Error::Cli {
                clap_exit_code: exit_code,
                message,
            })
        }
    }
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

    /// Reject the combination `--bless` (or `LIHAAF_OVERWRITE=1`) with
    /// no `--filter`. Bulk-blessing the entire fixture suite by accident
    /// is the classical lazy-bless failure mode (a real regression
    /// breaks the snapshot, the author runs `--bless` to "fix" it, the
    /// regression ships). Requiring an explicit `--filter` forces the
    /// caller to name the subset they intend to bless.
    ///
    /// The escape hatch for genuine bulk blessing is `--filter ""`
    /// (empty-string filter, which matches every relative path). The
    /// awkward form is intentional — the caller has to type out "yes,
    /// I want to bless everything" rather than getting it from a
    /// default.
    ///
    /// Exit code on failure is `2` (clap-style usage error).
    pub fn validate_bless_requires_filter(&self) -> Result<(), Error> {
        if self.effective_bless() && self.filter.is_empty() {
            let message = bless_without_filter_diagnostic();
            // Mirror clap's behavior for usage errors: print the
            // diagnostic so the user sees it even when the typed error
            // is the only return path.
            eprintln!("{message}");
            return Err(Error::Cli {
                clap_exit_code: 2,
                message,
            });
        }
        Ok(())
    }
}

/// Diagnostic body for `--bless` without `--filter`. Pulled out as a
/// free function so tests can compare against the exact string without
/// constructing a `Cli`.
fn bless_without_filter_diagnostic() -> String {
    "\
error: --bless requires --filter <SUBSTR> (or the LIHAAF_FILTER env)
note: --bless overwrites checked-in .stderr snapshots. To avoid
      accidentally rewriting unrelated fixtures, lihaaf requires you
      to name the subset you intend to bless via --filter.
note: to bless every fixture in a suite, pass an empty-string filter:
      `cargo lihaaf --bless --filter \"\"`
      This is intentionally awkward — the empty-string filter forces
      you to explicitly type out \"yes, I want to bless everything\"
      rather than getting it from a default."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: parse argv via `parse_from`, asserting success.
    ///
    /// IMPORTANT: callers must hold [`env_guard`] for the duration of
    /// the call so concurrent env-mutating tests (e.g.,
    /// `bless_via_env_when_flag_absent`) cannot race the
    /// `effective_bless()` env read inside
    /// `validate_bless_requires_filter`. The helper itself does NOT
    /// acquire the guard — `Mutex` is not re-entrant, and several
    /// tests both set the env and call `parse_from`, so the lock
    /// belongs at the test-function level. Tests that don't touch
    /// the env still must hold the guard while running because peer
    /// tests in this module mutate the same global env vars.
    fn parse(args: &[&str]) -> Cli {
        let argv: Vec<String> = std::iter::once("cargo-lihaaf".to_owned())
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        parse_from(argv).expect("parse must succeed")
    }

    #[test]
    fn defaults_are_safe_posture() {
        // Hold env_guard + scrub LIHAAF_OVERWRITE so a parent shell
        // setting it does not trip the new bless-filter validator
        // inside `parse_from`.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);
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
        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn filter_accumulates() {
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);
        let c = parse(&["--filter", "phase7", "--filter", "phase8"]);
        assert_eq!(c.filter, vec!["phase7".to_string(), "phase8".to_string()]);
        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn jobs_short_long() {
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);
        assert_eq!(parse(&["-j", "4"]).jobs, Some(4));
        assert_eq!(parse(&["--jobs", "8"]).jobs, Some(8));
        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn jobs_zero_is_rejected_per_spec_section_5_2() {
        // `-j 0` is rejected. The clap value parser hard-fails rather
        // than silently coercing.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);
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
        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn jobs_long_form_zero_also_rejected() {
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);
        let argv: Vec<String> = ["cargo-lihaaf", "--jobs", "0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_from(argv).is_err());
        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn bless_via_env_when_flag_absent() {
        // Env reads happen at call time; pollution across tests must be avoided.
        // SAFETY: `set_var` is `unsafe` in 2024 edition. The shared
        // env-guard mutex below serializes every test in this module
        // that mutates `LIHAAF_OVERWRITE`, so we hold an exclusive lock
        // for the duration of the parse + assertion + restore window.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        unsafe {
            std::env::set_var("LIHAAF_OVERWRITE", "1");
        }
        // Provide an explicit `--filter` to clear the
        // `validate_bless_requires_filter` guard: this test exercises
        // the env-driven `effective_bless()` path, not the filter
        // requirement (which has its own dedicated test below).
        let c = parse(&["--filter", "any"]);
        assert!(c.effective_bless());
        // Restore.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("LIHAAF_OVERWRITE", v),
                None => std::env::remove_var("LIHAAF_OVERWRITE"),
            }
        }
    }

    /// Test-only mutex serializing every test in this module that
    /// mutates `LIHAAF_OVERWRITE`. Rust tests run multi-threaded by
    /// default; without serialization, parallel reads of the env var
    /// race against `set_var` / `remove_var` and produce flakes.
    fn env_guard() -> &'static std::sync::Mutex<()> {
        use std::sync::{Mutex, OnceLock};
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        GUARD.get_or_init(|| Mutex::new(()))
    }

    /// Helper: scrub `LIHAAF_OVERWRITE` so a parent shell setting it
    /// (e.g., a CI job running with bless on by default) does not
    /// silently flip the `effective_bless()` path inside tests that
    /// expect the flag-only behavior.
    ///
    /// SAFETY: tests holding `env_guard()` hold an exclusive lock for
    /// the entire env-touching window.
    fn scrub_overwrite_env_under_guard(prev: &Option<String>) {
        unsafe {
            std::env::remove_var("LIHAAF_OVERWRITE");
        }
        // `prev` is captured by the caller before scrubbing so the
        // restore step at the end of each test can put it back.
        let _ = prev;
    }

    fn restore_overwrite_env_under_guard(prev: Option<String>) {
        unsafe {
            match prev {
                Some(v) => std::env::set_var("LIHAAF_OVERWRITE", v),
                None => std::env::remove_var("LIHAAF_OVERWRITE"),
            }
        }
    }

    #[test]
    fn bless_without_filter_is_rejected_with_directed_diagnostic() {
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);

        let argv: Vec<String> = ["cargo-lihaaf", "--bless"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse_from(argv).expect_err("--bless without --filter must be rejected");
        match err {
            Error::Cli {
                clap_exit_code,
                message,
            } => {
                assert_eq!(clap_exit_code, 2, "exit code must be clap usage code 2");
                assert!(
                    message.contains("--bless requires --filter"),
                    "diagnostic must name --filter: {message}"
                );
                assert!(
                    message.contains("--filter \"\""),
                    "diagnostic must show the empty-string-filter escape hatch: {message}"
                );
                assert!(
                    message.contains("lazy")
                        || message.contains("accidentally rewriting")
                        || message.contains("intentionally awkward"),
                    "diagnostic must explain the rationale, not just syntax: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }

        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn bless_with_filter_is_accepted() {
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);

        let c = parse(&["--bless", "--filter", "phase7"]);
        assert!(c.bless);
        assert!(c.effective_bless());
        assert_eq!(c.filter, vec!["phase7".to_string()]);

        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn bless_with_empty_string_filter_is_accepted_explicit_bulk_override() {
        // The escape hatch: `--filter ""` matches every relative path
        // (every string contains the empty string as a substring). It
        // is intentionally awkward to type, but it remains a path for
        // adopters who genuinely need to bless an entire suite.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        scrub_overwrite_env_under_guard(&prev);

        let c = parse(&["--bless", "--filter", ""]);
        assert!(c.bless);
        assert!(c.effective_bless());
        // `filter` is non-empty as a Vec (it contains the empty string
        // as a single entry), which clears the
        // `validate_bless_requires_filter` guard.
        assert_eq!(c.filter, vec![String::new()]);
        assert!(
            !c.filter.is_empty(),
            "filter Vec must be non-empty even when the substring is empty — \
             that's the entire point of the escape hatch"
        );

        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn bless_with_env_var_and_empty_filter_is_rejected() {
        // The env-set variant of the guard: `LIHAAF_OVERWRITE=1` with
        // no `--filter` must be rejected the same way `--bless` with no
        // `--filter` is rejected.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        unsafe {
            std::env::set_var("LIHAAF_OVERWRITE", "1");
        }

        let argv: Vec<String> = ["cargo-lihaaf"].iter().map(|s| s.to_string()).collect();
        let err =
            parse_from(argv).expect_err("LIHAAF_OVERWRITE=1 without --filter must be rejected");
        match err {
            Error::Cli {
                clap_exit_code,
                message,
            } => {
                assert_eq!(clap_exit_code, 2);
                assert!(
                    message.contains("--bless requires --filter"),
                    "diagnostic must name --filter even when bless came via env: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }

        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn effective_bless_via_env_still_subject_to_filter_guard() {
        // Distinct from the rejection test above: this test pins the
        // semantic that `effective_bless()` returning true is the
        // canonical trigger for the guard — the source of the bless
        // signal (flag vs env) is irrelevant.
        let _guard = env_guard().lock().expect("env guard mutex poisoned");
        let prev = std::env::var("LIHAAF_OVERWRITE").ok();
        unsafe {
            std::env::set_var("LIHAAF_OVERWRITE", "1");
        }

        // Manually construct a Cli with env-driven bless and empty
        // filter; the validator should reject regardless of how bless
        // was signaled.
        let argv: Vec<String> = ["cargo-lihaaf"].iter().map(|s| s.to_string()).collect();
        let parse_result = parse_from(argv);
        assert!(
            parse_result.is_err(),
            "parse must fail when env-driven bless lacks --filter"
        );

        // With a filter, the same env-driven invocation succeeds.
        let argv2: Vec<String> = ["cargo-lihaaf", "--filter", "any"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = parse_from(argv2).expect("env-driven bless + --filter must succeed");
        assert!(c.effective_bless());
        assert!(!c.bless, "--bless flag itself is unset; bless came via env");

        restore_overwrite_env_under_guard(prev);
    }

    #[test]
    fn bless_help_long_warns_about_destructive_overwrite() {
        // The long-form `--help` text must include the classical
        // failure-mode warning. This pins the user-visible warning so
        // a future refactor does not accidentally drop it.
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        assert!(
            help.contains("WARNING") || help.contains("destructive"),
            "long help must warn about destructive overwrite: {help}"
        );
        assert!(
            help.contains("regression"),
            "long help must describe the regression-ships failure mode: {help}"
        );
        assert!(
            help.contains("BLESS_SKIPPED") || help.contains("REJECT"),
            "long help must mention the unchanged-fixture guard: {help}"
        );
    }
}
