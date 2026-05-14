//! Phase 3 of compat mode (issue #8) — argv-only baseline command runner.
//!
//! Spawns the baseline `cargo test` invocation that fork CI compares
//! against. The exact argv vector is supplied by the caller (the
//! `--compat-cargo-test-argv` flag, parsed in Phase 1 and bundled into
//! [`crate::compat::cli::CompatArgs::compat_cargo_test_argv`]); this
//! module never tokenizes a string and never invokes a shell.
//!
//! ## Security invariant — no shell, ever
//!
//! The argv vector is handed directly to
//! [`std::process::Command::new`] + [`std::process::Command::args`], so
//! shell metacharacters (`$HOME`, `;`, `&&`, single quotes, backticks,
//! …) are passed through as literal bytes to the spawned program.
//! There is no path that constructs a single command-line string and
//! hands it to `sh -c`, `bash -c`, or `cmd /c`. The same guarantee
//! holds on Windows: [`std::process::Command`] dispatches via
//! `CreateProcess` directly rather than going through `cmd.exe`, and
//! `std`'s argv-joining round-trip uses the documented Microsoft C
//! runtime quoting so the child sees argv entries verbatim.
//!
//! See `docs/compatibility-plan.md` §3.1 — "no shell command line" is a
//! locked v0.1 invariant.
//!
//! ## Phase 3 scope vs. Phase 4 scope
//!
//! Phase 3 only captures the **coarse baseline**: the resolved argv,
//! the exit code, the wall-clock, and the raw stdout / stderr bytes of
//! the child. The [`BaselineResult::pass`] and [`BaselineResult::fail`]
//! fields stay [`None`] in Phase 3 — Phase 4 (issue #9) layers
//! conservative libtest-output parsing on top of this capture and
//! populates per-fixture pass/fail counts driven by the trybuild
//! discovery set produced in Phase 6.
//!
//! The sidecar JSON shape is intentionally minimal in Phase 3:
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "argv": ["cargo", "test", "..."],
//!   "exit_code": 0,
//!   "stdout": "<raw stdout text>",
//!   "stderr": "<raw stderr text>"
//! }
//! ```
//!
//! Phase 4 extends the shape additively (no field is removed or
//! retyped). The `schema_version` integer is the explicit hook for
//! that bump; adopters that read the sidecar today see `1` and can gate
//! on a known value.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::error::Error;
use crate::util;

/// One captured baseline run.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the crate
/// root can [`#[doc(hidden)]`] re-export this for the integration test
/// crate. Not part of any v0.1 stability contract — the supported entry
/// to compat mode is `cargo lihaaf --compat`, not the Rust API.
///
/// Fields are documented in `docs/compatibility-plan.md` §3.3 (the
/// `results.baseline` subset) plus the §3.3 envelope's
/// `commands.baseline` field for `argv`.
#[derive(Debug)]
// Fields are unread in Phase 3 — the §3.3 envelope writer (Phase 7)
// reads them when wiring `results.baseline` and `commands.baseline`.
// Carrying them today keeps the §3.3 envelope schema additive across
// Phase 3 → Phase 7 (no field renames mid-implementation) and lets the
// integration tests in `tests/compat/argv_baseline_no_shell.rs` assert
// the captured shape.
#[allow(dead_code)]
pub struct BaselineResult {
    /// Number of fixtures libtest reported as passing. Populated only
    /// when fixture-level baseline is RECOGNIZED per §1 — Phase 4
    /// (issue #9) wires the conservative parser. Phase 3 always
    /// returns `None`.
    pub pass: Option<u32>,
    /// Number of fixtures libtest reported as failing. Same nullable
    /// rule as [`Self::pass`].
    pub fail: Option<u32>,
    /// Number of fixtures whose libtest output didn't match a
    /// recognized trybuild invocation. Always populated; this is the
    /// conservative `unknown` count from #9. Phase 3 returns `0`
    /// (Phase 4 changes this when fixture-level parsing applies).
    pub unknown_count: u32,
    /// Exit code from the child process. On a signal-terminated child
    /// (no real exit code; `ExitStatus::code()` returns [`None`]) this
    /// is `-1` — the §3.3 envelope renders the signal in `errors[]`
    /// rather than overloading the exit code field.
    pub exit_code: i32,
    /// Wall-clock for the baseline run, milliseconds. EXCLUDED from
    /// determinism checks per §3.3 (timing is not byte-stable across
    /// machines).
    pub dur_ms: u64,
    /// Path to the libtest output sidecar JSON the runner wrote.
    /// Always populated even on a non-zero exit so the §3.3 envelope
    /// writer can point adopters at the raw bytes for diagnosis.
    pub sidecar_path: PathBuf,
    /// Resolved argv that was actually executed. Recorded so the §3.3
    /// envelope's `commands.baseline` field can render the exact
    /// invocation. This is a byte-for-byte copy of the input slice —
    /// no quoting, no shell-escape normalization.
    pub argv: Vec<String>,
}

/// Sidecar JSON schema version. Bumped additively across phases —
/// Phase 3 emits `1`; Phase 4 will emit `2` when fixture-level fields
/// are layered on top. Adopters parsing the sidecar should gate on a
/// known value rather than treating the file as schema-free.
const SIDECAR_SCHEMA_VERSION: u32 = 1;

/// Sentinel exit code used when the child terminated via a signal and
/// no real OS-level exit code is available. `-1` is chosen because
/// every real POSIX exit code is in `0..=255`, and Windows
/// [`std::process::ExitStatus::code`] only returns `None` on a
/// signal-style termination (rare on that platform).
const SIGNAL_TERMINATED_EXIT_SENTINEL: i32 = -1;

/// Run the baseline `cargo test` invocation.
///
/// **Argv-only.** No shell, no `sh -c`, no `cmd /c`. The first element
/// of `argv` is the program; the remaining elements are direct argv
/// entries the OS hands to the child without interpretation.
///
/// **Errors.** Returns:
///
/// - [`Error::Cli`] when `argv` is empty. The diagnostic names the
///   `--compat-cargo-test-argv` flag so the adopter knows which input
///   was malformed even when this function is called through the
///   default `["cargo", "test"]` path.
/// - [`Error::SubprocessSpawn`] when the OS refuses to spawn the
///   program (binary not found, permission denied, …). Distinct from
///   a non-zero exit, which is a normal session outcome captured in
///   [`BaselineResult::exit_code`].
/// - [`Error::Io`] on failure to wait on the child or to write the
///   sidecar JSON.
/// - [`Error::JsonParse`] when the sidecar JSON cannot be serialized.
///   In practice this is unreachable — the input is `String`s,
///   integers, and a vector of `String`s, all of which `serde_json`
///   serializes infallibly — but the error path is wired in defensively
///   so a future schema bump can fail loudly rather than panicking.
///
/// **Side effects.** Writes the sidecar JSON to `sidecar_path` via
/// `crate::util::write_file_atomic`. Creates the sidecar's parent
/// directory if it doesn't exist (matching the atomic-write helper's
/// own semantics).
pub fn run_baseline(
    argv: &[String],
    cwd: &Path,
    sidecar_path: &Path,
) -> Result<BaselineResult, Error> {
    if argv.is_empty() {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: "error: `--compat-cargo-test-argv` must contain at least one argument \
                      (the program to spawn, e.g. `\"cargo\"`)"
                .to_string(),
        });
    }

    let program = &argv[0];
    let args = &argv[1..];

    let started = Instant::now();
    // Capture stdout/stderr so the sidecar can record them. Inherit env
    // by default — the child needs PATH so `cargo` / `rustc` resolve,
    // RUSTUP_TOOLCHAIN so the +toolchain selector continues to work,
    // and CARGO_HOME so adopters with non-default cargo state get the
    // same view their `cargo test` sees outside lihaaf.
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Error::SubprocessSpawn {
            program: program.clone(),
            source: e,
        })?;

    let dur_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    // `ExitStatus::code()` is `None` on Unix signal-terminated children
    // (`SIGKILL`, `SIGTERM`, …). The §3.3 envelope's `errors[]` field
    // is where signal detail belongs; the integer exit-code slot uses
    // a sentinel so adopters consuming only the bare integer still see
    // a non-zero value rather than a misleading 0.
    let exit_code = output
        .status
        .code()
        .unwrap_or(SIGNAL_TERMINATED_EXIT_SENTINEL);

    // Libtest stdout is well-formed UTF-8 in practice (cargo + rustc
    // emit it as such, and the `--format=json` mode if used would
    // round-trip cleanly). Lossy decode tolerates a binary fixture
    // that emits non-UTF-8 noise — those bytes are lost in the
    // sidecar but the rest of the capture stays readable. The
    // alternative (base64) would add a dependency the v0.1 surface
    // does not need.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    write_sidecar(sidecar_path, argv, exit_code, &stdout, &stderr)?;

    Ok(BaselineResult {
        pass: None,
        fail: None,
        unknown_count: 0,
        exit_code,
        dur_ms,
        sidecar_path: sidecar_path.to_path_buf(),
        argv: argv.to_vec(),
    })
}

/// Serialize the sidecar JSON and write it atomically.
///
/// Split out from [`run_baseline`] for testability — the inline unit
/// test in this module exercises the serializer shape without
/// spawning a child.
fn write_sidecar(
    sidecar_path: &Path,
    argv: &[String],
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> Result<(), Error> {
    // Use `serde_json::Map` with `preserve_order` (enabled at the
    // crate level via the `preserve_order` feature on `serde_json`) so
    // the emitted JSON keeps the order this code inserts them in.
    // Adopters reading the file with `jq` see a stable shape across
    // runs.
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "schema_version".to_string(),
        serde_json::Value::from(SIDECAR_SCHEMA_VERSION),
    );
    envelope.insert(
        "argv".to_string(),
        serde_json::Value::Array(
            argv.iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    envelope.insert("exit_code".to_string(), serde_json::Value::from(exit_code));
    envelope.insert(
        "stdout".to_string(),
        serde_json::Value::String(stdout.to_string()),
    );
    envelope.insert(
        "stderr".to_string(),
        serde_json::Value::String(stderr.to_string()),
    );

    let mut bytes =
        serde_json::to_vec_pretty(&serde_json::Value::Object(envelope)).map_err(|e| {
            Error::JsonParse {
                context: "serializing compat baseline sidecar".into(),
                message: e.to_string(),
            }
        })?;
    // Trailing newline so `cat` output reads cleanly.
    bytes.push(b'\n');

    util::write_file_atomic(sidecar_path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The empty-argv guard fires before any process is spawned. The
    /// diagnostic must name `--compat-cargo-test-argv` so the adopter
    /// can find the flag in `cargo lihaaf --help`.
    #[test]
    fn empty_argv_is_rejected_with_directed_message() {
        let tmp = tempdir().unwrap();
        let sidecar = tmp.path().join("baseline_capture.json");
        let err = run_baseline(&[], tmp.path(), &sidecar).expect_err("empty argv must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message.contains("--compat-cargo-test-argv"),
                    "diagnostic must name the flag; got: {message}"
                );
                assert!(
                    message.contains("at least one argument"),
                    "diagnostic must spell out the requirement; got: {message}"
                );
            }
            other => panic!("expected Error::Cli, got {other:?}"),
        }
    }

    /// Sidecar JSON keys land in the documented order:
    /// `schema_version`, `argv`, `exit_code`, `stdout`, `stderr`. A
    /// reorder would silently break adopter `jq` pipelines that pull
    /// fields by position; the `preserve_order` feature on `serde_json`
    /// is the underlying guarantee.
    #[test]
    fn sidecar_shape_is_canonical() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        let argv = vec!["foo".to_string(), "bar".to_string()];
        write_sidecar(&path, &argv, 0, "out", "err").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();

        let i_schema = text
            .find("\"schema_version\"")
            .expect("schema_version key must be present");
        let i_argv = text.find("\"argv\"").expect("argv key must be present");
        let i_exit = text
            .find("\"exit_code\"")
            .expect("exit_code key must be present");
        let i_stdout = text.find("\"stdout\"").expect("stdout key must be present");
        let i_stderr = text.find("\"stderr\"").expect("stderr key must be present");

        assert!(
            i_schema < i_argv && i_argv < i_exit && i_exit < i_stdout && i_stdout < i_stderr,
            "sidecar JSON keys must appear in canonical order: schema_version, argv, \
             exit_code, stdout, stderr; got:\n{text}"
        );
    }

    /// Sidecar schema_version is the documented integer (`1`). A bump
    /// requires a deliberate code change; the test bites if a refactor
    /// accidentally drops the version constant or flips its type.
    #[test]
    fn sidecar_schema_version_is_one() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        write_sidecar(&path, &["x".to_string()], 0, "", "").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v.get("schema_version").and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }
}
