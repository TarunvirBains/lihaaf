//! # lihaaf — fast, parallel, non-flaky proc-macro test harness
//!
//! `lihaaf` ("quilt", Urdu) is a Rust proc-macro test harness. It compiles
//! a single consumer crate as a Rust dynamic library once at session
//! startup, then dispatches each fixture file as a per-fixture `rustc`
//! invocation that links the dylib via `--extern`. The architectural win
//! is documented in `docs/spec/lihaaf-v0.1.md` Section 2: per-fixture cost
//! drops from cargo's full per-project rebuild (~5–15 minutes on a
//! 200-fixture corpus) to seconds because fixtures don't rebuild the
//! consumer; they link to it.
//!
//! ## Public surface
//!
//! v0.1's public surface is intentionally CLI-shaped:
//!
//! - The `cargo-lihaaf` binary (Cargo subcommand convention; invoked as
//!   `cargo lihaaf [OPTIONS]`).
//! - `[package.metadata.lihaaf]` schema in the consumer's `Cargo.toml`
//!   (see [`config::Config`]).
//! - Verdict catalog (see [`verdict::Verdict`]) and exit codes
//!   (see [`exit::ExitCode`]) — both part of the v0.1 stable surface.
//!
//! Library callers exist (the binary itself, and v0.x integration tests)
//! but the surface is pre-1.0 — module paths and helper signatures may
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
//! | [`freshness`] | Per-dispatch §4.5 invariant re-check (mtime / SHA-256 / rustc). |
//! | [`discovery`] | Walk `fixture_dirs`, classify pass/fail, sort. |
//! | [`worker`] | Per-fixture `rustc` spawn, RSS sampling, OOM, timeout. |
//! | [`normalize`] | Stderr normalization (no regex; stdlib only). |
//! | [`diff`] | Hand-rolled Myers diff with line granularity. |
//! | [`snapshot`] | `.stderr` file I/O + `--bless` semantics. |
//! | [`verdict`] | Per-fixture verdict + session reporter. |
//! | [`exit`] | Exit-code mapping per spec §10.3. |
//! | [`session`] | Lifecycle orchestration (stages 1–9 of spec §4.1). |
//! | [`error`] | Crate-wide error type. |
//! | [`util`] | Atomic file write + sha256 helpers. |

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
// Spec §6.1: no regex engine of any kind. Enforced by convention and
// dep-tree audit (cargo tree | grep -i regex must return nothing). The
// `clippy::disallowed_types` lint would be the natural place to encode
// the rule but requires a per-crate clippy.toml; CI grep is sufficient
// for a single-crate harness.

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
