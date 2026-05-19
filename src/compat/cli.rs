//! Compat-mode argument bundle.
//!
//! [`CompatArgs`] is the typed projection of [`crate::cli::Cli`] used by
//! the compat driver. Construction goes through [`CompatArgs::from_cli`],
//! which is only valid to call after `Cli::validate_mode_consistency`
//! has returned `Ok` — by that point `compat_root` and `compat_report`
//! are known to be present and the `compat_cargo_test_argv` JSON shape
//! can be checked once eagerly so a malformed value fails with a
//! CLI-layer diagnostic instead of crashing deep inside the baseline
//! driver.
//!
//! Pass-through v0.1 flags (`--bless`, `--no-cache`, `--list`,
//! `--quiet`, `--verbose`, `--use-symlink`, `--keep-output`, `--jobs`)
//! travel inside [`CompatArgs::inner_cli`]; the compat driver re-uses
//! that `Cli` when invoking the inner `lihaaf` session.

use std::path::PathBuf;

use crate::cli::Cli;
use crate::error::Error;

/// Absolutize a `compat_root` path (or `compat_manifest` path) that may have
/// arrived as a relative value from the CLI.
///
/// **Why this is necessary.** The production shape in
/// `compat/templates/pilot-stage2.yml` is `--compat-root .`, which is a
/// relative path evaluated in the context of the CI checkout directory.
/// Every downstream consumer in `overlay.rs` and `mod.rs` joins paths against
/// `compat_root` (e.g. `upstream_dir`, `converted_fixtures_root`).  If
/// `compat_root` is `"."` those joins produce relative strings like
/// `./target/lihaaf-compat-converted/`, which cargo resolves against the
/// staged manifest dir (`<upstream>/target/lihaaf-overlay/`) — not the crate
/// root — causing the double-`target/` nonexistent-path failure.
///
/// The fix: absolutize ONCE here at the CLI boundary so all downstream code
/// receives an absolute path.  We use `current_dir().join()` rather than
/// `canonicalize()` because the directory may not exist yet (e.g. an
/// operator-controlled path that will be created by a preceding checkout step)
/// and `canonicalize` fails on non-existent paths on most platforms.
fn absolutize_optional_path(base: Option<PathBuf>) -> Result<Option<PathBuf>, Error> {
    match base {
        None => Ok(None),
        Some(p) if p.is_absolute() => Ok(Some(p)),
        Some(p) => {
            let cwd = std::env::current_dir().map_err(|e| Error::Io {
                source: e,
                context: "obtaining cwd to absolutize --compat-root / --compat-manifest"
                    .to_string(),
                path: None,
            })?;
            Ok(Some(cwd.join(p)))
        }
    }
}

/// Absolutize a required path that is guaranteed non-`None` by
/// `validate_mode_consistency`. Extracts and absolutizes in one call so the
/// caller at the `compat_root` and `compat_report` sites stays one line each.
fn absolutize_required_path(base: PathBuf) -> Result<PathBuf, Error> {
    if base.is_absolute() {
        return Ok(base);
    }
    let cwd = std::env::current_dir().map_err(|e| Error::Io {
        source: e,
        context: "obtaining cwd to absolutize --compat-root / --compat-report".to_string(),
        path: None,
    })?;
    Ok(cwd.join(base))
}

/// Typed bundle of compat-mode arguments.
///
/// The struct is `pub` because the crate's binary lives in a separate
/// crate (`src/bin/cargo-lihaaf.rs`) and must be able to name the type
/// through the `#[doc(hidden)]` re-export at the crate root. All
/// fields stay `pub(crate)` — adopters cannot construct or read the
/// bundle from outside the crate. The supported entry to compat mode is
/// `cargo lihaaf --compat`, not the Rust API.
///
/// Every field is read by the compat driver ([`crate::compat::run`])
/// — `compat_root` / `compat_report` route the overlay + envelope I/O,
/// `compat_cargo_test_argv` drives the baseline runner,
/// `compat_manifest` / `compat_commit` flow into envelope fields,
/// `compat_filter` translates into `--filter` on the inner Cli,
/// `compat_trybuild_macro` extends the §3.2.1 discovery alias set, and
/// `inner_cli` provides the pass-through v0.1 flags.
#[derive(Debug, Clone)]
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
    /// `None`, the compat driver derives the path from `--compat-root`
    /// (the upstream manifest sits at `<compat_root>/Cargo.toml`).
    pub(crate) compat_manifest: Option<PathBuf>,
    /// Workspace-member package selector forwarded from `--package`
    /// (issue #53). Resolved to a member-manifest path inside the
    /// compat driver via [`crate::compat::overlay::resolve_workspace_member_manifest`].
    /// Mutually exclusive with `compat_manifest` (enforced at validator
    /// time — see `crate::cli::Cli::validate_mode_consistency`).
    pub(crate) compat_package: Option<String>,
    /// Commit SHA to record in the report envelope.
    pub(crate) compat_commit: Option<String>,
    /// Compat-mode fixture-path filter (substring; OR'd).
    pub(crate) compat_filter: Vec<String>,
    /// Additional fully-qualified macro paths the §3.2.1 AST walk
    /// treats as aliases for `trybuild::TestCases::new()`.
    pub(crate) compat_trybuild_macro: Vec<String>,
    /// The full original [`Cli`] for pass-through flag access
    /// (`--bless`, `--no-cache`, `--list`, `--quiet`, `--verbose`,
    /// `--use-symlink`, `--keep-output`, `--jobs`). The compat driver
    /// forwards the relevant fields into the inner session.
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
        // Absolutize compat_root at the CLI entry boundary. Production usage
        // passes `--compat-root .` (see `compat/templates/pilot-stage2.yml:209`),
        // which is a relative path. Every downstream consumer joins additional
        // sub-paths against compat_root; if compat_root stays as `"."` those
        // joins produce relative paths like `./target/lihaaf-compat-converted/`
        // that cargo resolves against the staged manifest dir
        // (`<upstream>/target/lihaaf-overlay/`) — not the crate root — causing
        // the double-`target/` nonexistent-path failure first caught in the
        // Round-2 panel. Fix once here so all consumers see an absolute path.
        let compat_root = absolutize_required_path(
            cli.compat_root
                .clone()
                .expect("validate_mode_consistency ensures compat_root is set"),
        )?;
        let compat_report = absolutize_required_path(
            cli.compat_report
                .clone()
                .expect("validate_mode_consistency ensures compat_report is set"),
        )?;
        let compat_cargo_test_argv = parse_argv_json(
            cli.compat_cargo_test_argv
                .as_deref()
                .unwrap_or(DEFAULT_CARGO_TEST_ARGV_JSON),
        )?;
        // --compat-manifest is optional; absolutize it too so callers in
        // overlay.rs never receive a relative manifest path.
        let compat_manifest = absolutize_optional_path(cli.compat_manifest.clone())?;
        // --package is a string identifier; no path absolutization
        // applies. The validator (`Cli::validate_mode_consistency`)
        // already enforced mutual exclusion with `--compat-manifest`.
        let compat_package = cli.compat_package.clone();
        let compat_commit = cli.compat_commit.clone();
        let compat_filter = cli.compat_filter.clone();
        let compat_trybuild_macro = cli.compat_trybuild_macro.clone();
        Ok(Self {
            compat_root,
            compat_report,
            compat_cargo_test_argv,
            compat_manifest,
            compat_package,
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
    use std::sync::Mutex;

    /// Global mutex that serializes any test that mutates `std::env::current_dir`.
    ///
    /// Rust's test runner executes unit tests in parallel within a single process.
    /// `set_current_dir` writes process-global state, so concurrent mutations
    /// would race. Tests that call `set_current_dir` must hold this lock for
    /// the duration of the mutation + restore cycle.
    static CWD_MUTEX: Mutex<()> = Mutex::new(());

    /// **`CompatArgs::from_cli` absolutizes a relative `--compat-root`.**
    ///
    /// Production CI invokes compat mode with `--compat-root .` (see
    /// `compat/templates/pilot-stage2.yml`). This test exercises
    /// `CompatArgs::from_cli` end-to-end with a relative path and asserts
    /// that the resulting `compat_root` field is absolute, proving that a
    /// future regression removing the `absolutize_required_path` call would
    /// break this test.
    ///
    /// **Test design.** We create a tempdir, `cd` into it, pass a relative
    /// sub-directory name (just the basename) as `--compat-root`, and assert
    /// `compat_root.is_absolute()` on the resulting `CompatArgs`. We also
    /// assert that the absolute path ends with the subdir basename so the
    /// test is not trivially satisfied by an unrelated absolute path.
    #[test]
    fn from_cli_absolutizes_relative_compat_root() {
        let tmp = tempfile::tempdir().expect("creating tempdir for cli absolutize test");
        let subdir_name = "my-crate-root";
        let subdir = tmp.path().join(subdir_name);
        std::fs::create_dir_all(&subdir).expect("creating subdir inside tempdir");

        let original_cwd = std::env::current_dir().expect("getting cwd before test");

        let result = {
            // Scope the mutex guard tightly: acquire, mutate cwd, run
            // `from_cli`, restore cwd, release.  Panic-safety: if
            // `from_cli` panics the guard is poisoned, not worse.
            let _guard = CWD_MUTEX
                .lock()
                .expect("CWD_MUTEX lock should not be poisoned");
            std::env::set_current_dir(tmp.path()).expect("cd into tempdir");

            // Build a minimal Cli with compat_root set to the RELATIVE subdir name.
            // compat_report is also required by validate_mode_consistency but the
            // test only exercises compat_root; give it an absolute path to avoid
            // a second relative-path interaction.
            let cli = crate::cli::Cli {
                bless: false,
                compat: true,
                compat_cargo_test_argv: None,
                compat_commit: None,
                compat_filter: Vec::new(),
                compat_manifest: None,
                compat_package: None,
                compat_report: Some(tmp.path().join("report.json")),
                compat_root: Some(std::path::PathBuf::from(subdir_name)),
                compat_trybuild_macro: Vec::new(),
                filter: Vec::new(),
                jobs: None,
                suite: Vec::new(),
                no_cache: false,
                manifest_path: None,
                list: false,
                quiet: false,
                verbose: false,
                use_symlink: false,
                keep_output: false,
                inner_compat_normalize: false,
            };

            let r = CompatArgs::from_cli(cli);

            // Restore cwd before releasing the lock so other tests see a
            // clean process state even if the assertion below panics.
            std::env::set_current_dir(&original_cwd)
                .expect("restoring original cwd after absolutize test");

            r
        };

        let args = result.expect("from_cli must succeed with a valid relative compat_root");
        assert!(
            args.compat_root.is_absolute(),
            "from_cli must absolutize --compat-root; got `{}`",
            args.compat_root.display()
        );
        assert!(
            args.compat_root
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == subdir_name),
            "absolutized compat_root must end with `{subdir_name}`; got `{}`",
            args.compat_root.display()
        );
    }

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

    /// **Issue #53 — `CompatArgs::from_cli` carries `compat_package`.**
    ///
    /// The projection plumbing is a single-line clone. This test pins
    /// the contract so a future refactor that drops the carry trips
    /// immediately; the resolver in `compat::run` cannot succeed
    /// without the field on `CompatArgs`.
    #[test]
    fn compat_args_from_cli_carries_compat_package() {
        let tmp = tempfile::tempdir().expect("creating tempdir for projection test");
        let cli = crate::cli::Cli {
            bless: false,
            compat: true,
            compat_cargo_test_argv: None,
            compat_commit: None,
            compat_filter: Vec::new(),
            compat_manifest: None,
            compat_package: Some("axum-macros".to_string()),
            compat_report: Some(tmp.path().join("report.json")),
            compat_root: Some(tmp.path().to_path_buf()),
            compat_trybuild_macro: Vec::new(),
            filter: Vec::new(),
            jobs: None,
            suite: Vec::new(),
            no_cache: false,
            manifest_path: None,
            list: false,
            quiet: false,
            verbose: false,
            use_symlink: false,
            keep_output: false,
            inner_compat_normalize: false,
        };
        let args = CompatArgs::from_cli(cli).expect("from_cli must succeed with valid Cli");
        assert_eq!(
            args.compat_package.as_deref(),
            Some("axum-macros"),
            "from_cli must carry compat_package through unchanged"
        );
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
