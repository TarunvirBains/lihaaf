//! Compat-mode argument bundle.
//!
//! [`CompatArgs`] is the typed projection of [`crate::cli::Cli`] used by
//! the compat driver. Construction goes through [`CompatArgs::from_cli`],
//! which is only valid to call after `Cli::validate_mode_consistency`
//! has returned `Ok` — by that point `compat_root` and `compat_report`
//! are known to be present and the `compat_cargo_test_argv` JSON shape
//! can be checked once eagerly so a malformed value fails with a
//! Phase 1 diagnostic instead of crashing deep inside the Phase 3
//! baseline driver.
//!
//! Pass-through v0.1 flags (`--bless`, `--no-cache`, `--list`,
//! `--quiet`, `--verbose`, `--use-symlink`, `--keep-output`, `--jobs`)
//! travel inside [`CompatArgs::inner_cli`]; the compat driver re-uses
//! that `Cli` when invoking the inner `lihaaf` session.

use std::path::PathBuf;

use crate::cli::Cli;
use crate::error::Error;

/// Typed bundle of compat-mode arguments.
///
/// The struct is `pub` because the crate's binary lives in a separate
/// crate (`src/bin/cargo-lihaaf.rs`) and must be able to name the type
/// through the `#[doc(hidden)]` re-export at the crate root. All
/// fields stay `pub(crate)` — adopters cannot construct or read the
/// bundle from outside the crate. The supported entry to compat mode is
/// `cargo lihaaf --compat`, not the Rust API.
///
/// Phases 2+ populate this struct and pass it through the compat
/// pipeline (manifest overlay, baseline runner, fixture discovery,
/// report writer).
#[derive(Debug, Clone)]
// Fields are unread in Phase 1 — the compat driver body is stubbed.
// Phase 2+ consumes them (manifest overlay, baseline runner, fixture
// discovery, report writer). The struct is populated correctly today
// so a regression in `from_cli` would fail the inline unit tests, but
// no caller reads back the fields until the driver body fills in.
#[allow(dead_code)]
pub struct CompatArgs {
    /// Target crate checkout root. Always set (validated by
    /// `validate_mode_consistency`).
    pub(crate) compat_root: PathBuf,
    /// Output path for the §3.3 envelope. Always set.
    pub(crate) compat_report: PathBuf,
    /// Parsed argv for the baseline `cargo test` invocation. Default
    /// `["cargo", "test"]` (applied when `--compat-cargo-test-argv` is
    /// not passed).
    pub(crate) compat_cargo_test_argv: Vec<String>,
    /// Sibling-manifest path override (`--compat-manifest`). When
    /// `None`, Phase 2 derives the path from `--compat-root`.
    pub(crate) compat_manifest: Option<PathBuf>,
    /// Commit SHA to record in the report envelope.
    pub(crate) compat_commit: Option<String>,
    /// Compat-mode fixture-path filter (substring; OR'd).
    pub(crate) compat_filter: Vec<String>,
    /// Additional fully-qualified macro paths the §3.2.1 AST walk
    /// treats as aliases for `trybuild::TestCases::new()`.
    pub(crate) compat_trybuild_macro: Vec<String>,
    /// The full original [`Cli`] for pass-through flag access
    /// (`--bless`, `--no-cache`, `--list`, `--quiet`, `--verbose`,
    /// `--use-symlink`, `--keep-output`, `--jobs`). Phase 2+ forwards
    /// the relevant fields into the inner session.
    pub(crate) inner_cli: Cli,
}

impl CompatArgs {
    /// Project a validated [`Cli`] into a [`CompatArgs`] bundle.
    ///
    /// **Pre-condition:** the (`pub(crate)`) `Cli::validate_mode_consistency`
    /// method has returned `Ok` for `cli`. This means `cli.compat` is
    /// `true`, `cli.compat_root` is `Some`, and `cli.compat_report` is
    /// `Some`.
    ///
    /// **Returns** `Err(Error::Cli)` if the
    /// `--compat-cargo-test-argv` JSON is malformed. The diagnostic
    /// names the flag and the parse error in a single human-readable
    /// line — adopters do not see a raw `serde_json` error.
    ///
    /// `pub` for the same reason [`CompatArgs`] itself is `pub`: the
    /// crate's binary lives in a separate crate and must reach this
    /// constructor through the `#[doc(hidden)]` re-export at the crate
    /// root.
    pub fn from_cli(cli: Cli) -> Result<Self, Error> {
        debug_assert!(
            cli.compat,
            "CompatArgs::from_cli called outside compat mode; validate_mode_consistency \
             must have been bypassed"
        );
        let compat_root = cli
            .compat_root
            .clone()
            .expect("validate_mode_consistency ensures compat_root is set");
        let compat_report = cli
            .compat_report
            .clone()
            .expect("validate_mode_consistency ensures compat_report is set");
        let compat_cargo_test_argv = parse_argv_json(
            cli.compat_cargo_test_argv
                .as_deref()
                .unwrap_or(DEFAULT_CARGO_TEST_ARGV_JSON),
        )?;
        let compat_manifest = cli.compat_manifest.clone();
        let compat_commit = cli.compat_commit.clone();
        let compat_filter = cli.compat_filter.clone();
        let compat_trybuild_macro = cli.compat_trybuild_macro.clone();
        Ok(Self {
            compat_root,
            compat_report,
            compat_cargo_test_argv,
            compat_manifest,
            compat_commit,
            compat_filter,
            compat_trybuild_macro,
            inner_cli: cli,
        })
    }
}

/// Default value for `--compat-cargo-test-argv` when the flag is not
/// passed. The string is parsed through [`parse_argv_json`] so the
/// default path and the user-supplied path share one validator and one
/// failure mode.
const DEFAULT_CARGO_TEST_ARGV_JSON: &str = r#"["cargo","test"]"#;

/// Parse `--compat-cargo-test-argv`'s JSON value into a `Vec<String>`.
///
/// The input must be a JSON array of strings (`["cargo", "test",
/// "--", "--ignored"]` etc.). Any other shape — a JSON object, a bare
/// string, a number, an array containing a non-string — is rejected
/// with [`Error::Cli`] and a diagnostic that names the expected shape.
/// Adopters never see a raw `serde_json` error; the harness owns the
/// error message.
fn parse_argv_json(s: &str) -> Result<Vec<String>, Error> {
    let value: serde_json::Value = serde_json::from_str(s).map_err(|e| Error::Cli {
        clap_exit_code: 2,
        message: format!(
            "error: `--compat-cargo-test-argv` must be a JSON array of strings \
             (e.g. `[\"cargo\",\"test\"]`); failed to parse as JSON: {e}"
        ),
    })?;

    let arr = match value {
        serde_json::Value::Array(a) => a,
        other => {
            return Err(Error::Cli {
                clap_exit_code: 2,
                message: format!(
                    "error: `--compat-cargo-test-argv` must be a JSON array of strings \
                     (e.g. `[\"cargo\",\"test\"]`); got a JSON {}",
                    json_value_kind(&other),
                ),
            });
        }
    };

    let mut argv = Vec::with_capacity(arr.len());
    for (idx, elem) in arr.into_iter().enumerate() {
        match elem {
            serde_json::Value::String(s) => argv.push(s),
            other => {
                return Err(Error::Cli {
                    clap_exit_code: 2,
                    message: format!(
                        "error: `--compat-cargo-test-argv` element at index {idx} is a JSON {} \
                         but every element must be a JSON string",
                        json_value_kind(&other),
                    ),
                });
            }
        }
    }
    Ok(argv)
}

/// Short label for a [`serde_json::Value`] variant; used in error
/// messages so adopters see "got a JSON object" instead of clap's
/// generic "invalid JSON".
fn json_value_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_argv_parses_to_cargo_test() {
        let argv = parse_argv_json(DEFAULT_CARGO_TEST_ARGV_JSON).expect("default must parse");
        assert_eq!(argv, vec!["cargo".to_string(), "test".to_string()]);
    }

    #[test]
    fn parse_argv_json_rejects_object() {
        let err = parse_argv_json(r#"{"cargo":"test"}"#).expect_err("object must be rejected");
        match err {
            Error::Cli { message, .. } => assert!(
                message.contains("JSON object"),
                "diagnostic must name the JSON kind: {message}"
            ),
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn parse_argv_json_rejects_string() {
        let err = parse_argv_json(r#""cargo test""#).expect_err("string must be rejected");
        match err {
            Error::Cli { message, .. } => assert!(
                message.contains("JSON string"),
                "diagnostic must name the JSON kind: {message}"
            ),
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn parse_argv_json_rejects_non_string_element() {
        let err = parse_argv_json(r#"["cargo", 42]"#).expect_err("number element must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message.contains("index 1"),
                    "diagnostic must name the failing index: {message}"
                );
                assert!(
                    message.contains("JSON number"),
                    "diagnostic must name the JSON kind: {message}"
                );
            }
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn parse_argv_json_rejects_malformed_json() {
        let err =
            parse_argv_json(r#"["cargo","test"#).expect_err("malformed JSON must be rejected");
        match err {
            Error::Cli { message, .. } => assert!(
                message.contains("failed to parse as JSON"),
                "diagnostic must surface the parse failure: {message}"
            ),
            other => panic!("expected Cli error, got {other:?}"),
        }
    }

    #[test]
    fn parse_argv_json_accepts_extended_argv() {
        let argv = parse_argv_json(r#"["cargo","+nightly","test","--","--ignored"]"#)
            .expect("extended argv must parse");
        assert_eq!(
            argv,
            vec![
                "cargo".to_string(),
                "+nightly".to_string(),
                "test".to_string(),
                "--".to_string(),
                "--ignored".to_string(),
            ]
        );
    }
}
