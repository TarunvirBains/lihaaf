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
//! - A small set of crate-root re-exports for adopters who do want to
//!   drive lihaaf from Rust: [`Cli`], [`Config`], [`Verdict`],
//!   [`ExitCode`], [`Error`], [`Outcome`], [`run`], [`Report`].
//!
//! All other modules are `pub(crate)` and may evolve freely across
//! v0.1.x point releases. Adopters who want to drive lihaaf from Rust
//! today should prefer the crate-root re-exports above (or
//! subprocess-spawn `cargo lihaaf`); the internal module paths are not
//! part of any v0.1 stability contract.
//!
//! ## What lives where
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`cli`] | `clap` argument parsing, flag-to-action mapping. |
//! | [`config`] | Parse + validate `[package.metadata.lihaaf]`. |
//! | `toolchain` | Capture `rustc --version --verbose` for drift checks. (pub(crate)) |
//! | `dylib` | `cargo rustc --crate-type=dylib` invocation, copy mechanic. (pub(crate)) |
//! | `manifest` | `target/lihaaf/manifest.json` schema + atomic write. (pub(crate)) |
//! | `freshness` | Per-dispatch the policy invariant re-check (mtime / SHA-256 / rustc). (pub(crate)) |
//! | `lock` | Session-wide advisory file lock on `target/lihaaf/.session.lock`. (pub(crate)) |
//! | `discovery` | Walk `fixture_dirs`, classify pass/fail, sort. (pub(crate)) |
//! | `worker` | Per-fixture `rustc` spawn, RSS sampling, OOM, timeout. (pub(crate)) |
//! | `normalize` | Stderr normalization (fixed-string, byte-level). (pub(crate)) |
//! | `diff` | Hand-rolled Myers diff with line granularity. (pub(crate)) |
//! | `snapshot` | `.stderr` file I/O + `--bless` semantics. (pub(crate)) |
//! | [`verdict`] | Per-fixture verdict + session reporter. |
//! | [`exit`] | Exit-code mapping per the policy. |
//! | `session` | Lifecycle orchestration (stages 1–9 of the policy). (pub(crate)) |
//! | `error` | Crate-wide error type. (pub(crate)) |
//! | `util` | Atomic file write + sha256 helpers. (pub(crate)) |

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
// A practical constraint: no regex engine in this crate. The dependency
// surface stays small via fixed-string handling and a tight normalization
// pass. Enforcement is convention + dependency checks in CI.

pub mod cli;
pub(crate) mod compat;
pub mod config;
pub(crate) mod diff;
pub(crate) mod discovery;
pub(crate) mod dylib;
pub(crate) mod error;
pub mod exit;
pub(crate) mod freshness;
pub(crate) mod lock;
pub(crate) mod manifest;
pub(crate) mod normalize;
pub(crate) mod session;
pub(crate) mod snapshot;
pub(crate) mod toolchain;
pub(crate) mod util;
pub mod verdict;
pub(crate) mod worker;

// Crate-root re-exports — the v0.1 stable Rust callable surface.
//
// `Outcome` is re-exported alongside `Error` because `Error::Session(Outcome)`
// is a public variant of `Error`; Rust requires every type appearing in a
// public enum's variants to be at least as visible as the enum itself
// (E0446). Re-exporting `Outcome` ratifies that surface explicitly.
pub use cli::Cli;
pub use config::Config;
pub use error::{Error, Outcome};
pub use exit::ExitCode;
pub use session::{Report, run};
pub use verdict::Verdict;

// Compat-mode entry point. Re-exported because the
// `cargo-lihaaf` binary lives in a separate crate (`src/bin/`) and
// cannot reach `pub(crate)` items directly. Adopters should NOT
// drive compat mode from Rust; the supported entry is `cargo lihaaf
// --compat`. The Rust surface here exists for the binary and for
// future integration tests; the path and signature are not part of
// any v0.1 stability contract.
#[doc(hidden)]
pub use compat::cli::CompatArgs;
#[doc(hidden)]
pub use compat::run as run_compat;

// Compat-mode overlay (Phase 2 of compat mode, GH #11). Re-exported
// for the same reason as `CompatArgs` above — `tests/compat/
// overlay_determinism.rs` lives in a separate test crate and reaches
// the overlay module through this `#[doc(hidden)]` re-export. The
// stability contract is the same: NOT part of any v0.1 surface.
//
// Only `materialize_overlay` is re-exported because that is what the
// integration tests exercise; the canonicalizer, key-order helper, and
// serializer are unit-tested inline within `src/compat/overlay.rs`.
// Phase 4 reaches them through `crate::compat::overlay::*`, not through
// a crate-root re-export.
#[doc(hidden)]
pub use compat::overlay::materialize_overlay as compat_overlay_materialize;

/// The semver-stable lihaaf release the binary identifies as.
///
/// This is the value that lands in `manifest.json`'s `lihaaf_version`
/// field. It must track `Cargo.toml`'s `package.version` exactly. Tests
/// pin the value so a forgotten bump fails CI rather than shipping a
/// stale stamp.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Self-test marker for the multi-suite end-to-end corpus. Exposed
/// only when the `suite_demo` Cargo feature is enabled — the named
/// `[[package.metadata.lihaaf.suite]]` entry in this crate's own
/// `Cargo.toml` enables that feature for its dedicated fixture
/// directory, and the `tests/lihaaf/compile_pass_suite_demo/`
/// fixture references this const. If feature propagation regresses
/// (the dylib build skips the feature, or the per-fixture rustc
/// invocation drops `--cfg feature="suite_demo"`), the fixture
/// fails to link with `unresolved import lihaaf::SUITE_DEMO_MARKER`
/// and lihaaf's own CI run fails — the test case bites without
/// needing a downstream adopter.
///
/// Not part of any public API contract.
#[cfg(feature = "suite_demo")]
pub const SUITE_DEMO_MARKER: &str = "lihaaf::SUITE_DEMO_MARKER";
