//! Phase 1 mode-error matrix integration tests — closes GH issue #12.
//!
//! Every test spawns `cargo-lihaaf` as a subprocess via
//! [`env!("CARGO_BIN_EXE_cargo-lihaaf")`] (so the test binary path is
//! stable across `cargo test --release` / debug, host platforms, and
//! workspace layouts) and asserts:
//!
//! - For "non-compat run rejects `--compat-*` flags": exit code is 2
//!   (clap-style usage error) and stderr names the offending flag plus
//!   the `--compat` prerequisite.
//! - For "compat run rejects shadowed v0.1 flags" (`--filter`,
//!   `--manifest-path`): exit code is 2 and stderr names the
//!   `--compat-*` replacement.
//! - For "compat run requires `--compat-*` flags": exit code is 2 and
//!   stderr names the missing required flag.
//! - For `compat_run_accepts_pass_through_flags`: a fully-formed compat
//!   invocation with the pass-through flags parses successfully; Phase 1
//!   stubs `run_compat` so the binary exits 0.
//! - For `non_compat_run_unchanged`: a fully-formed v0.1 invocation
//!   reaches the non-compat session path (does NOT trip the mode-error
//!   validator). Because the synthetic manifest path is non-existent,
//!   the run fails at config-loading with the CONFIG_INVALID exit code,
//!   not at CLI parse — and stderr does NOT mention `--compat`.

use std::path::PathBuf;
use std::process::Command;

/// Absolute path to the harness binary the test runner just built.
/// Cargo defines `CARGO_BIN_EXE_<name>` at build time for every
/// `[[bin]]` entry in the crate manifest. The constant is evaluated at
/// compile time, so there is no `env::var` lookup at runtime and no
/// PATH-search ambiguity.
const CARGO_LIHAAF_BIN: &str = env!("CARGO_BIN_EXE_cargo-lihaaf");

/// Spawn the harness binary with the given argv and collect the
/// process output.
///
/// The first positional argument that `cargo` would inject (`lihaaf`)
/// is NOT prepended — the binary's own argv-stripping heuristic
/// handles either shape. The current working directory is set to the
/// crate's manifest dir so any relative paths in test invocations
/// (`--compat-root .`, etc.) resolve under the lihaaf checkout, not
/// under the test runner's cwd.
fn run_binary(args: &[&str]) -> std::process::Output {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new(CARGO_LIHAAF_BIN)
        .args(args)
        .current_dir(&manifest_dir)
        .output()
        .expect("spawning cargo-lihaaf must succeed")
}

/// Convert the captured stderr bytes to a UTF-8 string. Test
/// invocations never produce invalid UTF-8 (diagnostics are all ASCII
/// or controlled UTF-8 from clap and the validator), so a hard panic
/// on a decoding failure is correct — it would mean the harness
/// emitted unexpected bytes and the test should fail loudly.
fn stderr_string(out: &std::process::Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr must be UTF-8")
}

/// Assert the invocation produced exit code 2 (clap-style usage error)
/// and the stderr contains every expected substring.
fn assert_cli_mode_error(out: &std::process::Output, expected_substrings: &[&str]) {
    let stderr = stderr_string(out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected CLI mode-error exit code 2 (got {:?}); stderr was:\n{stderr}",
        out.status.code(),
    );
    for needle in expected_substrings {
        assert!(
            stderr.contains(needle),
            "stderr must contain `{needle}`; got:\n{stderr}",
        );
    }
}

// --- Non-compat-mode rejections: every --compat-* flag is a mode error. ---

#[test]
fn non_compat_run_rejects_compat_root() {
    let out = run_binary(&["--compat-root", "."]);
    assert_cli_mode_error(&out, &["--compat-root", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_report() {
    let out = run_binary(&["--compat-report", "/tmp/lihaaf-compat-report-test"]);
    assert_cli_mode_error(&out, &["--compat-report", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_filter() {
    let out = run_binary(&["--compat-filter", "phase7"]);
    assert_cli_mode_error(&out, &["--compat-filter", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_manifest() {
    let out = run_binary(&["--compat-manifest", "/tmp/lihaaf-compat-manifest-test"]);
    assert_cli_mode_error(&out, &["--compat-manifest", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_cargo_test_argv() {
    let out = run_binary(&["--compat-cargo-test-argv", r#"["cargo","test"]"#]);
    assert_cli_mode_error(&out, &["--compat-cargo-test-argv", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_commit() {
    let out = run_binary(&["--compat-commit", "deadbeef"]);
    assert_cli_mode_error(&out, &["--compat-commit", "--compat"]);
}

#[test]
fn non_compat_run_rejects_compat_trybuild_macro() {
    let out = run_binary(&["--compat-trybuild-macro", "crate::aliased_tests"]);
    assert_cli_mode_error(&out, &["--compat-trybuild-macro", "--compat"]);
}

// --- Compat-mode shadowed-flag rejections: --filter / --manifest-path are mode errors. ---

#[test]
fn compat_run_rejects_bare_filter() {
    let out = run_binary(&[
        "--compat",
        "--compat-root",
        ".",
        "--compat-report",
        "/tmp/lihaaf-compat-report-test",
        "--filter",
        "phase7",
    ]);
    // Validator naming: the diagnostic must name the replacement flag
    // (`--compat-filter`) so the adopter knows where to go.
    assert_cli_mode_error(&out, &["--filter", "--compat-filter"]);
}

#[test]
fn compat_run_rejects_bare_manifest_path() {
    let out = run_binary(&[
        "--compat",
        "--compat-root",
        ".",
        "--compat-report",
        "/tmp/lihaaf-compat-report-test",
        "--manifest-path",
        "/tmp/lihaaf-compat-manifest-path-test",
    ]);
    assert_cli_mode_error(&out, &["--manifest-path", "--compat-manifest"]);
}

// --- Compat-mode required-flag rejections. ---

#[test]
fn compat_run_requires_compat_root() {
    // `--compat-report` is present; `--compat-root` is missing.
    let out = run_binary(&[
        "--compat",
        "--compat-report",
        "/tmp/lihaaf-compat-report-test",
    ]);
    assert_cli_mode_error(&out, &["--compat-root", "required"]);
}

#[test]
fn compat_run_requires_compat_report() {
    // `--compat-root` is present; `--compat-report` is missing.
    let out = run_binary(&["--compat", "--compat-root", "."]);
    assert_cli_mode_error(&out, &["--compat-report", "required"]);
}

// --- Pass-through happy path. ---

#[test]
fn compat_run_accepts_pass_through_flags() {
    // Fully-formed compat invocation including every v0.1 pass-through
    // flag. The validator must accept this combination, Phase 1's stub
    // `compat::run` must return Ok(()), and the binary must exit 0.
    let out = run_binary(&[
        "--compat",
        "--compat-root",
        ".",
        "--compat-report",
        "/tmp/lihaaf-compat-report-pass-through.json",
        "--bless",
        "--no-cache",
        "--jobs",
        "4",
        "-v",
    ]);
    let stderr = stderr_string(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "Phase 1 stub must accept the pass-through invocation (got {:?}); stderr:\n{stderr}",
        out.status.code(),
    );
    assert!(
        !stderr.contains("error:"),
        "stderr must not contain any error diagnostic; got:\n{stderr}"
    );
}

// --- Rust API also enforces mode-consistency. ---

/// A Rust caller that constructs `Cli` via direct field initialization
/// (or via `Cli::try_parse_from`) bypasses `cli::parse_from`, so the
/// validator must ALSO run at the top of `lihaaf::run`. Without this,
/// a Rust adopter could pass `--compat-root` without `--compat` and
/// the harness would silently ignore the compat-only flag — exactly
/// the shape the §3.1 mode-error matrix exists to reject.
///
/// This test constructs a `Cli` with `compat: false` and
/// `compat_root: Some(...)`, calls `lihaaf::run(cli)`, and asserts the
/// returned error is the mode-consistency diagnostic (clap_exit_code
/// 2, message names the offending flag and `--compat`).
#[test]
fn rust_api_run_enforces_mode_consistency() {
    let cli = lihaaf::Cli {
        bless: false,
        compat: false,
        compat_cargo_test_argv: None,
        compat_commit: None,
        compat_filter: vec![],
        compat_manifest: None,
        compat_report: None,
        compat_root: Some(PathBuf::from("/tmp/lihaaf-rust-api-validator-test")),
        compat_trybuild_macro: vec![],
        filter: vec![],
        jobs: None,
        suite: vec![],
        no_cache: false,
        manifest_path: None,
        list: false,
        quiet: false,
        verbose: false,
        use_symlink: false,
        keep_output: false,
    };
    let err = lihaaf::run(cli).expect_err("validator must fire on the Rust API path");
    match err {
        lihaaf::Error::Cli {
            clap_exit_code,
            message,
        } => {
            assert_eq!(clap_exit_code, 2, "mode-consistency must use exit code 2");
            assert!(
                message.contains("--compat-root"),
                "diagnostic must name the offending flag; got: {message}"
            );
            assert!(
                message.contains("--compat"),
                "diagnostic must point at --compat; got: {message}"
            );
        }
        other => panic!("expected Error::Cli, got {other:?}"),
    }
}

// --- Regression bite: non-compat surface stays byte-identical. ---

#[test]
fn non_compat_run_unchanged() {
    // The v0.1 surface — `--filter` plus `--manifest-path` — must
    // parse cleanly when `--compat` is NOT set. The synthetic manifest
    // path does not exist; the run will fail at config-loading with a
    // session-level outcome (exit 64 = CONFIG_INVALID), but the
    // failure must NOT be the CLI mode-error path (exit 2) and the
    // stderr must NOT mention `--compat`.
    let out = run_binary(&[
        "--filter",
        "phase7",
        "--manifest-path",
        "/tmp/lihaaf-non-existent-manifest.toml",
    ]);
    let stderr = stderr_string(&out);
    let code = out.status.code();
    assert_ne!(
        code,
        Some(2),
        "non-compat invocation must not be a CLI mode error; got exit {:?}, stderr:\n{stderr}",
        code,
    );
    assert!(
        !stderr.contains("--compat"),
        "non-compat stderr must not mention any --compat flag; got:\n{stderr}"
    );
}
