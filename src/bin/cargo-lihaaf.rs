//! `cargo-lihaaf` — Cargo subcommand entry point.
//!
//! Cargo discovers binaries on `PATH` named `cargo-<subcommand>` and
//! invokes them with the subcommand string as the first positional
//! argument. We strip that argument, then hand the remainder to
//! [`lihaaf::cli`] for parsing.
//!
//! ## Why a binary, not a library entry point
//!
//! Spec §8.1 commits to the cargo-subcommand convention. The semver story
//! (§8.5) only covers the CLI surface, not any Rust API; shipping a
//! `cargo-lihaaf` binary keeps the contract on the surface adopters
//! actually consume.
//!
//! ## Exit codes
//!
//! All exit-code mapping lives in [`lihaaf::exit::ExitCode`]. This binary
//! is responsible only for converting an [`lihaaf::error::Error`] (a
//! session-level startup failure) into the corresponding session-outcome
//! exit code. Per-fixture verdict aggregation runs inside
//! [`lihaaf::session::run`] and bubbles up as part of the success path's
//! [`lihaaf::session::Report`].

use std::process::ExitCode as ProcessExitCode;

use lihaaf::cli;
use lihaaf::error::Error;
use lihaaf::exit::ExitCode;
use lihaaf::session;

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
        Err(e) => {
            // clap prints its own diagnostic on `--help` / `--version`
            // and on parse errors; we just propagate the exit code.
            return ProcessExitCode::from(e.exit_code() as u8);
        }
    };

    match session::run(parsed) {
        Ok(report) => ProcessExitCode::from(report.exit_code() as u8),
        Err(Error::Session(outcome)) => ProcessExitCode::from(outcome.exit_code() as u8),
        Err(other) => {
            eprintln!("lihaaf: {other}");
            ProcessExitCode::from(ExitCode::ConfigInvalid as u8)
        }
    }
}
