//! # lihaaf — fast, parallel compile-fail/compile-pass harness
//!
//! `lihaaf` ("quilt", Urdu) is a practical Rust test harness for
//! running compile-fail and compile-pass fixtures with less friction.
//! It compiles the consumer crate once as a dynamic library, then runs each
//! fixture as a standalone `rustc` invocation that links the prebuilt
//! dylib via `--extern`. In many real projects this is the difference
//! between waiting forever and getting quick feedback.
//!
//! ## Public surface
//!
//! The exposed surface is intentionally CLI-shaped:
//!
//! - The `cargo-lihaaf` binary (Cargo subcommand convention; invoked as
//!   `cargo lihaaf [OPTIONS]`).
//! - `[package.metadata.lihaaf]` schema in the consumer's `Cargo.toml`
//!   (see [`config::Config`]).
//! - Verdict catalog (see [`verdict::Verdict`]) and exit codes
//!   (see [`exit::ExitCode`]).
//!
//! Library callers exist (the binary itself, and v0.x integration tests),
//! but this is pre-1.0: module paths and helper signatures may
//! evolve before v1.0. Adopters who want to drive lihaaf from Rust today
//! should subprocess-spawn `cargo lihaaf`.
//!
//! ## What lives where
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`cli`] | `clap` argument parsing, flag-to-action mapping. |
//! | [`config`] | Parse + validate `[package.metadata.lihaaf]`. |
//! | [`toolchain`] | Capture `rustc --version --verbose` for drift checks. |
//! | [`dylib`] | `cargo rustc --crate-type=dylib` invocation, copy mechanic. |
//! | [`manifest`] | `target/lihaaf/manifest.json` schema + atomic write. |
//! | [`freshness`] | Per-dispatch the policy invariant re-check (mtime / SHA-256 / rustc). |
//! | [`discovery`] | Walk `fixture_dirs`, classify pass/fail, sort. |
//! | [`worker`] | Per-fixture `rustc` spawn, RSS sampling, OOM, timeout. |
//! | [`normalize`] | Stderr normalization (fixed-string, byte-level). |
//! | [`diff`] | Hand-rolled Myers diff with line granularity. |
//! | [`snapshot`] | `.stderr` file I/O + `--bless` semantics. |
//! | [`verdict`] | Per-fixture verdict + session reporter. |
//! | [`exit`] | Exit-code mapping per the policy. |
//! | [`session`] | Lifecycle orchestration (stages 1–9 of the policy). |
//! | [`error`] | Crate-wide error type. |
//! | [`util`] | Atomic file write + sha256 helpers. |

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
// A practical constraint: no regex engine in this crate. We keep the
// dependency surface small by using fixed-string handling and a tight
// normalization pass. Enforcement is convention + dependency checks in CI.

pub mod cli;
pub mod config;
pub mod diff;
pub mod discovery;
pub mod dylib;
pub mod error;
pub mod exit;
pub mod freshness;
pub mod manifest;
pub mod normalize;
pub mod session;
pub mod snapshot;
pub mod toolchain;
pub mod util;
pub mod verdict;
pub mod worker;

/// The semver-stable lihaaf release the binary identifies as.
///
/// This is the value that lands in `manifest.json`'s `lihaaf_version`
/// field. It must track `Cargo.toml`'s `package.version` exactly. Tests
/// pin the value so a forgotten bump fails CI rather than shipping a
/// stale stamp.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
