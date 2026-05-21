//! Mode-error matrix integration tests.
//!
//! Most tests in this file spawn `cargo-lihaaf` as a subprocess via
//! [`env!("CARGO_BIN_EXE_cargo-lihaaf")`] (so the test binary path is
//! stable across `cargo test --release` / debug, host platforms, and
//! workspace layouts) and assert:
//!
//! - For "non-compat run rejects `--compat-*` flags": exit code is 2
//!   (clap-style usage error) and stderr names the offending flag plus
//!   the `--compat` prerequisite.
//! - For "compat run rejects shadowed v0.1 flags" (`--filter`,
//!   `--manifest-path`): exit code is 2 and stderr names the
//!   `--compat-*` replacement.
//! - For "compat run requires `--compat-*` flags": exit code is 2 and
//!   stderr names the missing required flag.
//! - For `non_compat_run_unchanged`: a fully-formed v0.1 invocation
//!   reaches the non-compat session path (does NOT trip the mode-error
//!   validator). Because the synthetic manifest path is non-existent,
//!   the run fails at config-loading with the CONFIG_INVALID exit code,
//!   not at CLI parse — and stderr does NOT mention `--compat`.
//!
//! The two pass-through happy-path tests
//! (`compat_run_accepts_pass_through_flags`,
//! `compat_run_accepts_omitted_cargo_test_argv`) are asserted at the
//! Rust API layer via `Cli::try_parse_from(...)` +
//! `cli.validate_mode_consistency()`. Originally these spawned the
//! binary against `--compat-root "."` (the lihaaf repo itself); the
//! stub `run_compat` is now a full end-to-end driver, so the spawn
//! would run `cargo test` → `rustc` against lihaaf's own tree (because
//! the test passes `--compat-root .`), and the parallel fan-out OOMs
//! WSL2-class hosts. The CLI-layer assertion still locks the parser +
//! validator behavior the tests originally cared about; the
//! full-driver integration belongs in a hermetic harness that knows
//! how to stand up a synthetic compat-root.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// Absolute path to the harness binary the test runner just built.
/// Cargo defines `CARGO_BIN_EXE_<name>` at build time for every
/// `[[bin]]` entry in the crate manifest. The constant is evaluated at
/// compile time, so there is no `env::var` lookup at runtime and no
/// PATH-search ambiguity.
const CARGO_LIHAAF_BIN: &str = env!("CARGO_BIN_EXE_cargo-lihaaf");

/// Serializes subprocess spawns within this test binary. Multiple
/// test methods run in parallel (default test-threads = ncpus); each
/// spawn of `cargo-lihaaf` recursively spawns rustc per fixture. On
/// hosts with limited memory (notably WSL2) the parallel fan-out can
/// trigger a global OOM kill that takes down the VM. The lock bounds
/// in-flight subprocesses to 1 per binary; cross-binary parallelism
/// is still possible but the per-binary cap is what prevents the
/// runaway chain we observed.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

/// Spawn the harness binary with the given argv and collect the
/// process output.
///
/// The first positional argument that `cargo` would inject (`lihaaf`)
/// is NOT prepended — the binary's own argv-stripping heuristic
/// handles either shape. The current working directory is set to the
/// crate's manifest dir so any relative paths in test invocations
/// (`--compat-root .`, etc.) resolve under the lihaaf checkout, not
/// under the test runner's cwd.
///
/// Acquires [`SPAWN_LOCK`] for the lifetime of the spawn so concurrent
/// test methods do not stack subprocess chains; see the lock's docs
/// for the OOM motivation.
fn run_binary(args: &[&str]) -> std::process::Output {
    let _guard = SPAWN_LOCK.lock().expect("SPAWN_LOCK poisoned");
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

// --- Pass-through happy path (asserted at the CLI parser layer). ---

#[test]
fn compat_run_accepts_pass_through_flags() {
    // This assertion used to spawn the binary and expect exit 0 from a
    // minimal compat driver. The driver now runs end to end, so the
    // spawn would run `cargo test` -> `rustc` against lihaaf's own tree
    // (because the test passes `--compat-root .`), and the parallel
    // fan-out can exhaust constrained hosts. The assertion now operates
    // at the CLI parser layer: a
    // fully-formed compat invocation including every v0.1
    // pass-through flag must parse cleanly AND survive the
    // mode-consistency validator. That is the behavior the test
    // originally cared about; the full-driver integration belongs in
    // a hermetic harness that knows how to stand up a synthetic
    // compat-root.
    use clap::Parser;
    let cli = lihaaf::Cli::try_parse_from([
        "cargo-lihaaf",
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
    ])
    .expect("clap must accept the fully-formed compat invocation with pass-through flags");
    // Validator must accept the combination (compat: true; both
    // required compat flags present; no shadowed v0.1 flags). The
    // method is `pub(crate)` for direct callers but observable via
    // `lihaaf::run(cli)` — passing a Cli that passes the validator
    // never returns Error::Cli with a mode-error message. To assert
    // at this layer without invoking the full session, drive the
    // public `lihaaf::cli::parse_from` entry the binary uses; it
    // calls the validator internally and surfaces any mode error as
    // `Error::Cli { clap_exit_code: 2, message }`.
    let argv: Vec<String> = [
        "cargo-lihaaf",
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
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    lihaaf::cli::parse_from(argv).expect(
        "cli::parse_from must accept the fully-formed compat invocation: clap parse + \
         mode-consistency validator both green",
    );
    // Sanity on the parsed Cli to ensure each pass-through field
    // landed where expected.
    assert!(cli.compat, "compat flag must be set");
    assert!(cli.bless, "--bless must propagate");
    assert!(cli.no_cache, "--no-cache must propagate");
    assert_eq!(cli.jobs, Some(4), "--jobs must propagate");
    assert!(cli.verbose, "-v / --verbose must propagate");
}

/// **`--compat-cargo-test-argv` is optional.**
///
/// The doc comment on `Cli::compat_cargo_test_argv` previously said
/// "Required when `--compat` is set"; the actual behavior in
/// `CompatArgs::from_cli` defaults to `["cargo", "test"]` when the flag
/// is absent. The doc is now corrected to "Optional in compat mode"
/// with the default named explicitly. This integration test locks the
/// runtime behavior at the CLI parser + validator + CompatArgs::from_cli
/// chain so a future regression that tightens the validator (e.g.
/// makes the flag required) would bite here without needing to spawn
/// the binary against a real compat-root.
#[test]
fn compat_run_accepts_omitted_cargo_test_argv() {
    // CLI parse + validator must accept the invocation. Then
    // `CompatArgs::from_cli` must surface the default ["cargo",
    // "test"] argv vector for the unset flag.
    let argv: Vec<String> = [
        "cargo-lihaaf",
        "--compat",
        "--compat-root",
        ".",
        "--compat-report",
        "/tmp/lihaaf-compat-report-omitted-argv.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let cli = lihaaf::cli::parse_from(argv).expect(
        "cli::parse_from must accept compat without --compat-cargo-test-argv (validator must \
         not require the flag)",
    );
    assert!(cli.compat, "compat flag must be set");
    assert!(
        cli.compat_cargo_test_argv.is_none(),
        "--compat-cargo-test-argv must be None when omitted; got {:?}",
        cli.compat_cargo_test_argv,
    );
    // CompatArgs::from_cli applies the default. The diagnostic for a
    // regression here would name the flag, so the assert message
    // points at the default value adopters depend on.
    let args = lihaaf::CompatArgs::from_cli(cli)
        .expect("CompatArgs::from_cli must succeed when --compat-cargo-test-argv is omitted");
    // The CompatArgs fields are `pub(crate)`, so we can't read
    // `compat_cargo_test_argv` directly across the crate boundary;
    // the `from_cli` call success is the contract we lock here, and
    // the inline unit test inside `src/compat/cli.rs` covers the
    // default-argv shape byte-for-byte.
    let _ = args;
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
    // Construct the Cli via clap's `try_parse_from` (the same path
    // `cli::parse_from` takes minus the validator call). This gives us
    // a Cli with `compat: false` and `compat_root: Some(...)` —
    // exactly the inconsistent shape the validator must reject — while
    // leaving the harness-private `inner_compat_normalize` field at its
    // clap-derive default. Then call `lihaaf::run` and assert the
    // validator (invoked from inside `run`) fires.
    use clap::Parser;
    let cli = lihaaf::Cli::try_parse_from([
        "cargo-lihaaf",
        "--compat-root",
        "/tmp/lihaaf-rust-api-validator-test",
    ])
    .expect("clap parse should succeed; validation runs inside lihaaf::run");
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
