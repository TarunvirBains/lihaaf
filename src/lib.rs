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

// Compat-mode baseline runner (Phase 3 of compat mode, GH #8).
// Re-exported for the same reason as the overlay above —
// `tests/compat/argv_baseline_no_shell.rs` lives in a separate test
// crate and reaches the baseline module through these
// `#[doc(hidden)]` re-exports. The stability contract is the same:
// NOT part of any v0.1 surface. Phase 4 (issue #9) layers
// fixture-level parsing on top of `BaselineResult` and bumps the
// sidecar `schema_version` from `1` to `2`; that change is additive
// and the re-export name stays put.
#[doc(hidden)]
pub use compat::baseline::BaselineResult as CompatBaselineResult;
#[doc(hidden)]
pub use compat::baseline::run_baseline as compat_baseline_run;

// Compat-mode conservative trybuild baseline extraction (Phase 4 of
// compat mode, GH #9). Re-exported so `tests/compat/
// baseline_conservative.rs` can reach the conservative parser, the
// fixture-recognition input type, the mismatch record, and the v2
// runner entry point through stable names. The stability contract
// is the same as the other compat re-exports: NOT part of any v0.1
// surface; the supported entry to compat mode is `cargo lihaaf
// --compat`.
#[doc(hidden)]
pub use compat::baseline::BaselineMismatch as CompatBaselineMismatch;
#[doc(hidden)]
pub use compat::baseline::BaselineVerdict as CompatBaselineVerdict;
#[doc(hidden)]
pub use compat::baseline::FixtureId as CompatFixtureId;
#[doc(hidden)]
pub use compat::baseline::ParsedBaseline as CompatParsedBaseline;
#[doc(hidden)]
pub use compat::baseline::parse_libtest_output as compat_parse_libtest_output;
#[doc(hidden)]
pub use compat::baseline::run_baseline_with_recognized_fixtures as compat_baseline_run_with_recognized_fixtures;

// Compat-mode fixture-invocation discovery (Phase 6 of compat mode,
// §3.2.1 of the compatibility plan). Re-exported for the same reason
// as the overlay above — `tests/compat/discovery_syn.rs` lives in a
// separate test crate and reaches the discovery types through these
// `#[doc(hidden)]` re-exports. The stability contract is the same:
// NOT part of any v0.1 surface; the supported entry to compat mode is
// `cargo lihaaf --compat`. Phase 9 (issue #13 — driver body) wires
// `discover` into `compat::run`; until then the re-exports exist
// solely to let the §3.2.1 integration tests reach the visitor.
#[doc(hidden)]
pub use compat::discovery::CallSite as CompatDiscoveryCallSite;
#[doc(hidden)]
pub use compat::discovery::DiscoveredFixture as CompatDiscoveredFixture;
#[doc(hidden)]
pub use compat::discovery::DiscoveryOutput as CompatDiscoveryOutput;
#[doc(hidden)]
pub use compat::discovery::DiscoveryUnrecognized as CompatDiscoveryUnrecognized;
#[doc(hidden)]
pub use compat::discovery::FixtureKind as CompatFixtureKind;
#[doc(hidden)]
pub use compat::discovery::discover as compat_discover;

// Compat-mode dirty-worktree cleanup (Phase 5 of compat mode, GH #10).
// Re-exported for the same reason as the overlay above —
// `tests/compat/cleanup_dirty_worktree.rs` lives in a separate test
// crate and reaches the cleanup types through these `#[doc(hidden)]`
// re-exports. The stability contract is the same: NOT part of any
// v0.1 surface; the supported entry to compat mode is `cargo lihaaf
// --compat`.
//
// `install_panic_hook` is re-exported alongside the guard types
// because Phase 9 wires it into `compat::run` and integration tests
// will eventually call it directly (in a child process so the
// process-wide hook does not perturb libtest's panic capture in the
// outer test runner).
#[doc(hidden)]
pub use compat::cleanup::CleanupGuard as CompatCleanupGuard;
#[doc(hidden)]
pub use compat::cleanup::GeneratedPath as CompatGeneratedPath;
#[doc(hidden)]
pub use compat::cleanup::GeneratedPathClass as CompatGeneratedPathClass;
#[doc(hidden)]
pub use compat::cleanup::install_panic_hook as compat_install_panic_hook;

// Compat-mode normalizer flag plumbing (Phase 7 of compat mode,
// §3.2.2 of the compatibility plan). Re-exported for the same reason
// as the other compat surfaces above — `tests/compat/
// normalizer_compat_cargo.rs` lives in a separate test crate and
// reaches the public `NormalizationContext` / `normalize` entry
// points through these `#[doc(hidden)]` re-exports. The stability
// contract is the same: NOT part of any v0.1 surface; the supported
// entry to compat mode is `cargo lihaaf --compat`.
#[doc(hidden)]
pub use normalize::{NormalizationContext, normalize};

// Compat-mode §3.3 deterministic envelope (Phase 8 of compat mode).
// Re-exported for the same reason as the other compat surfaces above —
// `tests/compat/report_determinism.rs` lives in a separate test crate
// and reaches the envelope struct + writer through these
// `#[doc(hidden)]` re-exports. The stability contract is the same: NOT
// part of any v0.1 surface; the supported entry to compat mode is
// `cargo lihaaf --compat`. Phase 9 (issue #13 — driver body) wires
// `write_envelope` into `compat::run`; until then the re-exports exist
// solely to let the Phase 8 integration tests reach the writer.
#[doc(hidden)]
pub use compat::report::BaselineCounts as CompatBaselineCounts;
#[doc(hidden)]
pub use compat::report::Commands as CompatCommands;
#[doc(hidden)]
pub use compat::report::CompatEnvelope;
#[doc(hidden)]
pub use compat::report::EnvelopeError as CompatEnvelopeError;
#[doc(hidden)]
pub use compat::report::ExcludedFixture as CompatExcludedFixture;
#[doc(hidden)]
pub use compat::report::GeneratedPath as CompatEnvelopeGeneratedPath;
#[doc(hidden)]
pub use compat::report::LihaafCounts as CompatLihaafCounts;
#[doc(hidden)]
pub use compat::report::MismatchExample as CompatMismatchExample;
#[doc(hidden)]
pub use compat::report::OverlayMetadata as CompatOverlayMetadata;
#[doc(hidden)]
pub use compat::report::Results as CompatResults;
#[doc(hidden)]
pub use compat::report::canonicalize as compat_canonicalize_envelope;
#[doc(hidden)]
pub use compat::report::generated_path_from_cleanup as compat_envelope_generated_path_from_cleanup;
#[doc(hidden)]
pub use compat::report::write_envelope as compat_write_envelope;

// Compat-mode §3.4 active-toolchain capture (Phase 9 of compat mode).
// Re-exported for the same reason as the other compat surfaces above —
// `tests/compat/toolchain_resolution.rs` lives in a separate test crate
// and reaches the capture entry point + its internal "swap the program
// name" variant through these `#[doc(hidden)]` re-exports. The
// stability contract is the same: NOT part of any v0.1 surface; the
// supported entry to compat mode is `cargo lihaaf --compat`. Phase 10
// (driver wire-up) calls `capture_active_toolchain` directly through
// `crate::compat::rustup::*`, not through this re-export.
#[doc(hidden)]
pub use compat::rustup::capture_active_toolchain as compat_capture_active_toolchain;
#[doc(hidden)]
pub use compat::rustup::capture_with_program as compat_capture_with_program;

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
