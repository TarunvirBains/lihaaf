//! Phase 9 of compat mode (§3.4 of `docs/compatibility-plan.md`) —
//! integration tests for the active-toolchain capture entry point.
//!
//! Each test in this file reaches `lihaaf::compat_capture_active_toolchain`
//! (and the `_with_program` variant) through the `#[doc(hidden)]`
//! re-exports declared in `src/lib.rs`. The re-exports exist exclusively
//! for this test crate (and for the Phase 10 driver wire-up). The
//! supported entry to compat mode is `cargo lihaaf --compat`, not the
//! Rust API.
//!
//! ## Test choices
//!
//! - **`captures_active_toolchain_when_rustup_present`** calls the
//!   public entry with the integration-test crate root and asserts on
//!   the SHAPE of the returned identifier (lowercase ASCII, no
//!   whitespace, non-empty) rather than a fixed string. CI runners may
//!   have different `rustup default` values; a shape assertion bites if
//!   the parser regresses (e.g. forgets to trim the `(default)` suffix)
//!   without coupling the test to a specific toolchain pin. Gated with
//!   a runtime `rustup --version` probe; the test exits cleanly when
//!   rustup is absent.
//!
//! - **`falls_back_to_rustc_release_when_rustup_absent`** invokes the
//!   `_with_program` variant with an absolute path to a binary that
//!   does not exist. This exercises the spawn-error branch of the
//!   fallback without mutating `PATH` (a process-global mutation
//!   would race with parallel tests). Asserts the returned string is
//!   a rustc release line (starts with `"rustc "`).
//!
//! - **`respects_compat_root_param`** writes a `rust-toolchain.toml` to
//!   a tempdir, passes that path as the `compat_root` parameter to the
//!   public entry, and asserts the returned identifier reflects the pin.
//!   The fix in this commit moved `rustup show active-toolchain`'s cwd
//!   from "the test runner's cwd" to "the explicit `compat_root`
//!   parameter"; without this test bites, a regression that re-introduced
//!   the cwd-based resolution would silently re-record the wrong
//!   toolchain. The test no longer mutates process cwd, so no
//!   serialization mutex is needed. The pin uses `stable` so the test
//!   does not depend on `rustup` having `nightly` installed.
//!
//! ## Why no PATH mutation
//!
//! Mutating `PATH` (or any environment variable) inside a test is
//! process-wide and races with other tests in the same binary. The
//! `_with_program` indirection in `src/compat/rustup.rs` lets the
//! fallback test exercise the spawn-error path through a localized
//! argument override, not a global state change.

use std::path::PathBuf;
use std::process::Command;

use lihaaf::{compat_capture_active_toolchain, compat_capture_with_program};

/// True if `rustup --version` returns a successful exit. Used to gate
/// the rustup-dependent tests so CI runners without rustup still get a
/// clean pass on the rest of the file.
fn rustup_available() -> bool {
    Command::new("rustup")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn captures_active_toolchain_when_rustup_present() {
    if !rustup_available() {
        eprintln!("rustup not available; skipping");
        return;
    }
    // Use the test crate's own checkout root as the compat_root —
    // every contributor / CI machine has rustup configured here, so
    // the call resolves the same active toolchain it would for the
    // unit-test runner itself.
    let compat_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let captured = compat_capture_active_toolchain(&compat_root)
        .expect("rustup is on PATH and rustc is on PATH; capture must succeed");
    assert!(
        !captured.is_empty(),
        "rustup-present capture must return a non-empty identifier"
    );
    assert!(
        !captured.contains(char::is_whitespace),
        "first-space trim must drop the `(default)` suffix; got {captured:?}"
    );
    assert!(
        !captured.contains("(default)"),
        "first-space trim must drop the `(default)` suffix; got {captured:?}"
    );
    // The trim head is the rustup toolchain identifier, which rustup
    // always renders in lowercase (`stable`, `nightly`, `1.95.0`, with
    // a `-<host-triple>` suffix). An uppercase byte would indicate the
    // parser picked up a different line than expected.
    assert!(
        captured.bytes().all(|b| !b.is_ascii_uppercase()),
        "rustup identifiers are lowercase; got {captured:?}"
    );
}

#[test]
fn falls_back_to_rustc_release_when_rustup_absent() {
    // Use an absolute path that cannot exist so the spawn fails with
    // ENOENT (Unix) / file-not-found (Windows). This drives the spawn
    // error branch of the fallback without mutating `PATH`.
    #[cfg(unix)]
    let missing = "/lihaaf-phase9-rustup-does-not-exist";
    #[cfg(windows)]
    let missing = r"C:\lihaaf-phase9-rustup-does-not-exist.exe";

    let compat_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let captured = compat_capture_with_program(missing, &compat_root)
        .expect("rustc is on PATH; fallback to rustc release line must succeed");
    assert!(
        captured.starts_with("rustc "),
        "fallback must return the rustc release line (starts with `rustc `); got {captured:?}"
    );
    // The release line embeds a version triple like `1.95.0`; assert
    // a digit appears so a future regression that captured only the
    // word `rustc` bites here.
    assert!(
        captured.bytes().any(|b| b.is_ascii_digit()),
        "rustc release line must contain a version digit; got {captured:?}"
    );
}

#[test]
fn respects_compat_root_param() {
    if !rustup_available() {
        eprintln!("rustup not available; skipping");
        return;
    }
    // The pin uses `stable` because every rustup install on a contributor
    // or CI machine has the default toolchain ready. Pinning `nightly`
    // would skip on machines without nightly installed; pinning a fixed
    // version like `1.95.0` would break when the runner image rolls.
    let pin = "stable";

    let temp = tempfile::tempdir().expect("tempdir for rust-toolchain.toml pin");
    let manifest = temp.path().join("rust-toolchain.toml");
    std::fs::write(&manifest, format!("[toolchain]\nchannel = \"{pin}\"\n"))
        .expect("write rust-toolchain.toml");

    // Pass the tempdir as the compat_root parameter — no cwd mutation
    // required. The fix in this commit moved the rustup-resolution cwd
    // from the test runner's cwd to this explicit parameter; if a
    // regression drops the `current_dir(compat_root)` call on the
    // subprocess, the capture would record the test runner's pinned
    // toolchain instead of `stable` (the test runner pins lihaaf's own
    // toolchain, typically a fixed version like `1.95.0`).
    let captured = compat_capture_active_toolchain(temp.path())
        .expect("rustup capture under pinned compat_root must succeed");
    assert!(
        captured.starts_with(pin),
        "captured toolchain {captured:?} must start with the pinned channel {pin:?}; the \
         compat_root-based resolution did not take effect"
    );
    // `temp` is dropped here — TempDir's Drop cleans the directory.
    drop(temp);
}
