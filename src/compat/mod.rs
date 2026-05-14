//! Compat mode driver. Activated by `cargo lihaaf --compat`.
//!
//! Implements `docs/compatibility-plan.md` §3 end-to-end (Phases 2+ in
//! `docs/superpowers/plans/2026-05-13-compat-mode-implementation-plan.md`).
//! Phase 1 only stubs the entry point: the CLI surface, the mode-error
//! validator, and the `CompatArgs` bundle are in place; the driver body
//! is filled by later phases (manifest overlay, baseline runner, fixture
//! discovery, normalizer flag, report writer, cleanup, toolchain wiring,
//! CI gate).
//!
//! Adopters opt in via `cargo lihaaf --compat --compat-root <DIR>
//! --compat-report <PATH>`. The Rust API is not part of the v0.1
//! stability contract; treat `pub(crate) fn run` as private.

pub(crate) mod baseline;
pub(crate) mod cleanup;
pub(crate) mod cli;
pub(crate) mod discovery;
pub(crate) mod overlay;
pub(crate) mod report;

/// Top-level compat-mode entry. Called from `cargo-lihaaf.rs` when
/// `cli.compat` is true.
///
/// Phase 1 is a stub that returns `Ok(())` immediately — the CLI parses,
/// the validator runs, and the binary exits 0 so the
/// `compat_run_accepts_pass_through_flags` test can observe the full
/// parse-and-dispatch path without the driver body being wired. Later
/// phases fill the body.
///
/// This is `pub` so the crate's binary (`src/bin/cargo-lihaaf.rs`) and
/// out-of-tree integration tests can reach it through the re-export at
/// the crate root. It is `#[doc(hidden)]` at the re-export site —
/// adopters should drive compat mode through `cargo lihaaf --compat`,
/// not through the Rust API.
pub fn run(_args: cli::CompatArgs) -> Result<(), crate::error::Error> {
    Ok(())
}
