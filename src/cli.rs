//! CLI argument parsing.
//!
//! Spec §8.2 lists every flag in the v0.1 stable surface. The struct
//! below covers each one with its spec-mandated semantics. Each field's
//! rustdoc cites the spec section it derives from.
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
/// Each field maps 1:1 to a spec §8.2 flag. Defaults preserve the
/// "non-`--bless`, non-`--keep-output`, non-`--use-symlink`" posture
/// that the spec calls out as the default safe operating mode.
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
    /// from disk. Equivalent env: `LIHAAF_OVERWRITE=1`. Spec §7.3.
    #[arg(long)]
    pub bless: bool,

    /// Run only fixtures whose relative path contains the substring.
    /// Multiple `--filter` flags are OR'd. Substring match
    /// (case-sensitive). Spec §8.2.
    #[arg(long)]
    pub filter: Vec<String>,

    /// Override the worker parallelism cap. The harness still applies
    /// the RAM cap on top — the override does not bypass it. Spec §5.2.
    #[arg(short = 'j', long = "jobs")]
    pub jobs: Option<u32>,

    /// Force a fresh dylib build, ignoring any existing manifest.
    /// Equivalent to deleting `target/lihaaf/manifest.json` before
    /// invocation. Spec §8.2.
    #[arg(long)]
    pub no_cache: bool,

    /// Override the consumer `Cargo.toml` location. Default is cargo's
    /// normal "current directory + parent walk" lookup.
    #[arg(long, value_name = "PATH")]
    pub manifest_path: Option<PathBuf>,

    /// Print the fixtures the harness would run, one relative path per
    /// line, and exit 0. Does not build the dylib, does not invoke
    /// rustc. Composable with `--filter`. Spec §8.2.
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
    /// no concurrent cargo activity will modify `target/`. Spec §4.3.
    #[arg(long)]
    pub use_symlink: bool,

    /// Preserve per-fixture work directories after verdict capture.
    /// Local-development escape hatch only — never set in CI.
    /// Spec §5.3.
    #[arg(long)]
    pub keep_output: bool,
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
    /// equivalent to `--bless` (spec §7.3).
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
