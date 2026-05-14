//! `cargo-lihaaf` — Cargo subcommand entry point.
//!
//! Cargo discovers binaries on `PATH` named `cargo-<subcommand>` and
//! invokes them with the subcommand string as the first positional
//! argument. That argument is stripped, then the remainder is handed to
//! [`lihaaf::cli`] for parsing.
//!
//! ## Why a binary, not a library entry point
//!
//! This crate ships as a Cargo subcommand binary because users interact through
//! CLI anyway; keeping that as the primary entry keeps compatibility and semver
//! expectations straightforward.
//!
//! ## Exit codes
//!
//! Exit-code mapping lives in [`lihaaf::exit::ExitCode`]. This binary turns an
//! [`lihaaf::Error`] into the matching session exit code.
//! Per-fixture verdict aggregation runs inside [`lihaaf::run`]
//! and is returned as part of the success path’s report.
//! [`lihaaf::Report`].

use std::process::ExitCode as ProcessExitCode;

use lihaaf::cli;
use lihaaf::{CompatArgs, Error, ExitCode, run, run_compat};

fn main() -> ProcessExitCode {
    // Cargo passes `lihaaf` as the first positional. Strip it if present.
    // Direct invocation (`cargo-lihaaf --help`) without the prefix also
    // works — the heuristic is "if argv[1] equals `lihaaf` literally,
    // strip it; otherwise leave the argv alone."
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 2 && argv[1] == "lihaaf" {
        argv.remove(1);
    }

    let parsed = match cli::parse_from(argv) {
        Ok(p) => p,
        Err(Error::Cli { clap_exit_code, .. }) => {
            // clap (and `validate_mode_consistency`) print their own
            // diagnostic to stderr; `parse_from` re-prints the
            // validator message via `eprintln!`. The exit code is
            // propagated verbatim — `0` for `--help` / `--version`,
            // `2` for clap usage errors and mode-error rejections.
            // This is the documented promise in `Error::exit_code`'s
            // doc comment: "CLI errors carry clap's recommended code
            // (typically 2)." The map to `u8` is safe because
            // `clap_exit_code` is always a small non-negative value
            // assigned by `parse_from` (currently `0` or `2`).
            return ProcessExitCode::from(clap_exit_code.clamp(0, 255) as u8);
        }
        Err(e) => {
            // Defensive fallback: any non-CLI error from `parse_from`
            // means the parser surfaced something unexpected. Map to
            // CONFIG_INVALID so the operator sees the failure rather
            // than a silent exit 0.
            eprintln!("lihaaf: {e}");
            return ProcessExitCode::from(ExitCode::ConfigInvalid as u8);
        }
    };

    // Branch on `--compat` after parsing. The validator inside
    // `cli::parse_from` already guarantees that when `cli.compat` is
    // true, the required `--compat*` flags are present, so
    // `CompatArgs::from_cli` only fails on a malformed
    // `--compat-cargo-test-argv` JSON value.
    if parsed.compat {
        let args = match CompatArgs::from_cli(parsed) {
            Ok(a) => a,
            Err(Error::Cli {
                clap_exit_code,
                message,
            }) => {
                eprintln!("{message}");
                return ProcessExitCode::from(clap_exit_code.clamp(0, 255) as u8);
            }
            Err(e) => {
                eprintln!("lihaaf: {e}");
                return ProcessExitCode::from(ExitCode::ConfigInvalid as u8);
            }
        };
        return match run_compat(args) {
            Ok(()) => ProcessExitCode::from(ExitCode::Ok as u8),
            Err(Error::Session(outcome)) => {
                eprintln!("{outcome}");
                ProcessExitCode::from(outcome.exit_code() as u8)
            }
            Err(other) => {
                eprintln!("lihaaf: {other}");
                ProcessExitCode::from(ExitCode::ConfigInvalid as u8)
            }
        };
    }

    match run(parsed) {
        Ok(report) => ProcessExitCode::from(report.exit_code() as u8),
        Err(Error::Session(outcome)) => {
            // Pre-v0.1.0-alpha.3 this branch was silent — the exit code
            // told the story but the diagnostic body was dropped on the
            // floor. The multi-suite work expanded the surface area of
            // session-level config errors (`--suite NAME` mismatch,
            // duplicate suite names, fixture_dirs collisions across
            // suites), so the diagnostic message is now printed before
            // the exit-code mapping. The Display impl on Outcome
            // already produces a fully-rendered, byte-deterministic
            // message — see `src/error.rs`.
            eprintln!("{outcome}");
            ProcessExitCode::from(outcome.exit_code() as u8)
        }
        Err(other) => {
            eprintln!("lihaaf: {other}");
            ProcessExitCode::from(ExitCode::ConfigInvalid as u8)
        }
    }
}
