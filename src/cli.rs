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
//! the validation we'd otherwise re-implement (positive integer for
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
    /// Overwrite `.stderr` snapshots whose normalized output differs
    /// from disk. Equivalent env: `LIHAAF_OVERWRITE=1`.
    #[arg(long)]
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
/// `0`; we tighten that to "positive integer required" so the bad
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
pub fn parse_from(argv: Vec<String>) -> Result<Cli, Error> {
    use clap::error::ErrorKind;
    match Cli::try_parse_from(argv) {
        Ok(cli) => Ok(cli),
        Err(e) => {
            // For `--help` / `--version`, clap returns a "graceful" error
            // and we should let it print and exit 0.
            let kind = e.kind();
            let exit_code = match kind {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 2,
            };
            // clap prints the message itself when we call `print()`.
            let message = e.to_string();
            // Pre-print so the user sees the message even though we
            // bubble through the typed error.
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

    #[test]
    fn bless_via_env_when_flag_absent() {
        // Env reads happen at call time; we must not pollute other tests.
        // SAFETY: `set_var` is `unsafe` in 2024 edition, but tests run
        // single-threaded by default and we restore the var in `finally`.
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
