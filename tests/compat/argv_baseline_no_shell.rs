//! Phase 3 of compat mode (issue #8) — argv-only baseline integration tests.
//!
//! The acid test is **"no shell, ever":** shell metacharacters in argv
//! entries are passed verbatim to the spawned process, never expanded
//! by `sh -c`, `bash -c`, or `cmd /c`. If a future refactor introduces
//! a shell-invocation path on the baseline runner,
//! [`shell_metacharacters_passed_as_literal_argv`] fails.
//!
//! All tests reach [`lihaaf::compat_baseline_run`] through the
//! `#[doc(hidden)]` re-exports in `src/lib.rs`. Those re-exports exist
//! exclusively for this test crate and the cargo-lihaaf binary; the
//! v0.1 supported entry to compat mode is `cargo lihaaf --compat`,
//! not the Rust API.
//!
//! ## Why every test is hermetic
//!
//! Each test owns a `tempfile::TempDir` and writes the sidecar JSON
//! into that directory. The baseline runner's only filesystem effect
//! is the sidecar write, so a leak would be visible. Spawned commands
//! that touch the filesystem (`pwd`, `printenv`) read or print only;
//! they do not write into the test's tempdir.
//!
//! ## Platform notes
//!
//! Linux + macOS use POSIX-style argv handoff via `execve`. On Windows,
//! [`std::process::Command`] dispatches through `CreateProcess` with
//! the documented Microsoft C runtime quoting rules — argv entries
//! round-trip verbatim for the characters we exercise here
//! (`$`, single quotes, backticks, `;`).

use std::path::PathBuf;

use lihaaf::compat_baseline_run as run_baseline;

/// Read the sidecar JSON written by the baseline runner.
///
/// Returns the parsed JSON value plus the raw bytes (the byte form is
/// asserted on directly by [`sidecar_records_argv_verbatim`]).
fn read_sidecar(path: &std::path::Path) -> (serde_json::Value, Vec<u8>) {
    let bytes = std::fs::read(path).expect("sidecar JSON must exist after the baseline run");
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("sidecar must parse as JSON");
    (value, bytes)
}

/// Empty-argv guard. The runner must refuse the spawn before any
/// process executes and surface a directed diagnostic naming
/// `--compat-cargo-test-argv` so the adopter can find the flag.
#[test]
fn empty_argv_is_rejected_with_directed_message() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let result = run_baseline(&[], tmp.path(), &sidecar);
    let err = result.expect_err("empty argv must be rejected before spawn");
    let rendered = format!("{err}");
    assert!(
        rendered.contains("--compat-cargo-test-argv"),
        "diagnostic must name the flag; got: {rendered}"
    );
    assert!(
        !sidecar.exists(),
        "sidecar must NOT be written when the spawn never happened"
    );
}

/// **Acid test for the "no shell, ever" invariant.**
///
/// Argv `["echo", "$HOME"]` must produce stdout `$HOME\n` (a single
/// literal dollar-sign followed by the four ASCII characters
/// `H-O-M-E`, then a newline). A shell-invocation regression (joining
/// argv into a single string and passing it to `sh -c`) would expand
/// `$HOME` to the user's home directory before the child runs. The
/// shape `^/` (absolute path) is not what `echo $HOME` produces
/// without a shell.
///
/// Two assertions back this up:
///
/// 1. The captured stdout starts with `$HOME` — direct evidence of
///    no variable expansion.
/// 2. The sidecar's `argv` array contains the literal `$HOME` token —
///    direct evidence of no argv-side tokenization or rewriting.
///
/// `echo` is a coreutils binary on every POSIX platform lihaaf
/// targets. The shell builtin can shadow it inside `bash` /
/// `zsh`, but spawning the binary directly via [`std::process`] picks
/// up `/usr/bin/echo` (PATH-resolved); the test does not depend on
/// the builtin.
#[test]
fn shell_metacharacters_passed_as_literal_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["echo".to_string(), "$HOME".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar).expect("`echo $HOME` must spawn");
    assert_eq!(
        result.exit_code, 0,
        "`echo` must exit 0; got {}",
        result.exit_code
    );

    let (value, _bytes) = read_sidecar(&sidecar);

    // Direct stdout assertion: `echo $HOME` writes the literal `$HOME`
    // string when no shell intercepts it. A shell-injection regression
    // would substitute the user's actual home directory (typically
    // `/home/...` or `/Users/...`).
    let captured_stdout = value
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .expect("sidecar `stdout` must be a string")
        .to_string();
    assert!(
        captured_stdout.starts_with("$HOME"),
        "echo must emit the literal `$HOME` string; got: {captured_stdout:?} \
         — if the captured stdout is an absolute path, a shell expanded \
         the variable before the child ran"
    );

    // Capture-side reinforcement: the argv recorded in the sidecar is
    // exactly the input vector, with no shell escaping or tokenization.
    let captured_argv: Vec<String> = value
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .expect("sidecar `argv` field must be an array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("sidecar argv elements must be strings")
                .to_string()
        })
        .collect();
    assert_eq!(
        captured_argv, argv,
        "the runner must record argv byte-for-byte; a shell-invocation \
         regression would either rewrite the entries or join them into \
         a single string"
    );
}

/// A shell-style command line that would normally expand under `sh
/// -c` is treated as a literal program name. Spawning a non-existent
/// program is a [`std::process::Command::spawn`] failure (the OS
/// returns `ENOENT` on POSIX), surfaced through
/// [`lihaaf::Error::SubprocessSpawn`]. A regression that introduced a
/// shell would instead spawn `sh -c "echo $HOME; rm -rf /"` and
/// happily run the inner commands — which would NOT produce a
/// `SubprocessSpawn` error.
#[test]
fn argv_zero_is_program_not_a_shell_command_line() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    // The whole shell-style line is the program name. On POSIX no
    // such binary exists; the spawn fails before any child runs.
    let argv = vec!["echo $HOME && rm -rf /tmp/never-touched-by-this-test".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar);
    let err = result.expect_err(
        "the runner must treat argv[0] as a program name, not a shell command line; \
         a non-existent program must fail to spawn",
    );
    // The error must be the spawn-failure variant. A `Cli` error
    // would indicate the empty-argv guard fired; an `Io` error after
    // a successful spawn would indicate the shell ran and we got past
    // the spawn step.
    match err {
        lihaaf::Error::SubprocessSpawn { program, .. } => {
            assert_eq!(
                program, argv[0],
                "spawn-failure diagnostic must name the literal argv[0]"
            );
        }
        other => panic!(
            "expected Error::SubprocessSpawn (spawn failure on a non-existent \
             program); got: {other:?} — a different error category here suggests \
             the runner went through a shell"
        ),
    }
    assert!(
        !sidecar.exists(),
        "sidecar must NOT be written when the spawn never happened"
    );
}

/// `cargo --version` succeeds because the spawned child inherits the
/// parent process's PATH (so `cargo` resolves) and other environment.
/// This is the minimum-viable "the child can find its tools"
/// assertion that Phase 4+ depends on (the real baseline `cargo test`
/// will not work otherwise).
#[test]
fn child_inherits_parent_env_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["cargo".to_string(), "--version".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar)
        .expect("`cargo --version` must spawn when PATH is inherited");
    assert_eq!(
        result.exit_code, 0,
        "`cargo --version` must exit 0; got {}",
        result.exit_code
    );

    // The captured stdout in the sidecar must contain the cargo
    // version banner. A regression that swallowed stdout (e.g. swapped
    // `Stdio::piped` for `Stdio::null`) would leave the field empty
    // even though the exit was 0.
    let (value, _) = read_sidecar(&sidecar);
    let captured_stdout = value
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .expect("sidecar `stdout` must be a string");
    assert!(
        captured_stdout.starts_with("cargo"),
        "captured stdout must begin with `cargo <version>`; got: {captured_stdout:?}"
    );
}

/// The `cwd` argument is honored. Spawning `pwd` with `cwd = tmp` must
/// print the tempdir's path; the runner does not silently fall back to
/// the lihaaf process's cwd. Path canonicalization (symlink resolution
/// on macOS `/tmp` → `/private/tmp`) is the reason the assertion uses
/// the trailing-component check rather than full-string equality.
#[test]
fn cwd_is_honored_by_spawned_process() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["pwd".to_string()];
    let result =
        run_baseline(&argv, tmp.path(), &sidecar).expect("`pwd` must spawn from a tempdir");
    assert_eq!(result.exit_code, 0, "`pwd` must exit 0");

    let (value, _) = read_sidecar(&sidecar);
    let captured_stdout = value
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .expect("sidecar `stdout` must be a string")
        .trim()
        .to_string();

    // macOS canonicalizes `/tmp` to `/private/tmp`, so a full-string
    // equality would fail on that platform. The trailing-component
    // check is portable: the tempdir's basename is unique to this
    // test invocation and must appear in the `pwd` output.
    let tmp_basename = tmp
        .path()
        .file_name()
        .expect("tempdir must have a basename")
        .to_string_lossy()
        .into_owned();
    assert!(
        captured_stdout.contains(&tmp_basename),
        "`pwd` output must contain the tempdir basename `{tmp_basename}`; \
         got: {captured_stdout:?}"
    );
}

/// `BaselineResult.exit_code` matches the child's exit code. The
/// canonical "non-zero exit" sentinel is `/usr/bin/false`, which is
/// guaranteed to exit `1` on every POSIX platform lihaaf targets.
#[test]
fn non_zero_exit_propagated_to_result() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["false".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar).expect("`false` must spawn");
    assert_ne!(
        result.exit_code, 0,
        "`false` must exit non-zero; got {}",
        result.exit_code
    );

    // Phase 3 invariant: `pass` and `fail` stay `None`; `unknown_count`
    // stays `0`. Phase 4 wires the conservative parser that flips
    // these. If a refactor populates these fields prematurely the
    // assertion bites.
    assert!(
        result.pass.is_none(),
        "Phase 3 must not populate `pass`; got Some({:?})",
        result.pass
    );
    assert!(
        result.fail.is_none(),
        "Phase 3 must not populate `fail`; got Some({:?})",
        result.fail
    );
    assert_eq!(
        result.unknown_count, 0,
        "Phase 3 must keep `unknown_count` at 0 until Phase 4 wires fixture-level parsing"
    );

    // Sidecar must reflect the same exit code so adopters reading the
    // JSON file see consistent data with the in-memory result.
    let (value, _) = read_sidecar(&sidecar);
    assert_eq!(
        value.get("exit_code").and_then(serde_json::Value::as_i64),
        Some(result.exit_code as i64),
        "sidecar `exit_code` must match the in-memory result"
    );
}

/// Wall-clock is recorded as monotonic milliseconds. The minimum
/// observable value is `0` (a very fast child on a fast machine), so
/// the assertion is `>= 0` — a `u64` is trivially non-negative; the
/// real concern is that the field is populated. The upper bound test
/// (`< 5 minutes`) catches a regression that overflows the clock or
/// hangs the wait.
#[test]
fn wall_clock_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["true".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar).expect("`true` must spawn");
    // The `u64 >= 0` shape is a tautology in Rust; the real check is
    // that the field exists and the test compiled with the field
    // present. A drift that removes the field would be a compile
    // error, not a runtime one.
    let _: u64 = result.dur_ms;
    assert!(
        result.dur_ms < 5 * 60 * 1000,
        "wall-clock for `true` must be well under 5 minutes; got {} ms — \
         a regression that hangs the wait or overflows the clock fails here",
        result.dur_ms
    );
}

/// Sidecar JSON records argv byte-for-byte. This is the structural
/// guarantee that the §3.3 envelope writer can render
/// `commands.baseline` exactly as the operator invoked it. A
/// regression that base64-encoded, escaped, or normalized argv would
/// silently fail this test.
#[test]
fn sidecar_records_argv_verbatim() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec![
        "true".to_string(),
        "--flag-with-dashes".to_string(),
        "argument with spaces".to_string(),
        "single'quote".to_string(),
        "$dollar_sign".to_string(),
    ];
    let result = run_baseline(&argv, tmp.path(), &sidecar)
        .expect("`true` must spawn regardless of trailing argv");
    assert_eq!(result.exit_code, 0);

    // In-memory result mirrors the input.
    assert_eq!(result.argv, argv);

    // Sidecar JSON mirrors the input.
    let (value, _) = read_sidecar(&sidecar);
    let captured_argv: Vec<String> = value
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .expect("sidecar `argv` field must be an array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("sidecar argv elements must be strings")
                .to_string()
        })
        .collect();
    assert_eq!(captured_argv, argv);
}

/// The sidecar path the runner returns matches the path the caller
/// supplied. This is the contract the §3.3 envelope writer relies on:
/// it must be able to construct the sidecar path from configuration
/// and trust that the runner wrote there.
#[test]
fn sidecar_path_in_result_matches_input() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["true".to_string()];
    let result = run_baseline(&argv, tmp.path(), &sidecar).expect("`true` must spawn");
    assert_eq!(
        result.sidecar_path,
        PathBuf::from(&sidecar),
        "BaselineResult.sidecar_path must match the input path byte-for-byte"
    );
    assert!(sidecar.exists(), "sidecar must be written to disk");
}
