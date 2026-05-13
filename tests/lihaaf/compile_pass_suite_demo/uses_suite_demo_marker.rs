//! Multi-suite end-to-end coverage. Compile_pass fixture for the
//! `[[package.metadata.lihaaf.suite]]` named "suite_demo".
//!
//! `lihaaf::SUITE_DEMO_MARKER` is only exposed when lihaaf is built
//! with `--features suite_demo`. The `suite_demo` named suite enables
//! that feature for its dedicated dylib build AND propagates `--cfg
//! feature="suite_demo"` to this fixture's per-fixture rustc
//! invocation, so the body below compiles and links against the
//! const.
//!
//! Regression guard: if a future change drops the `--features` flag on
//! the per-suite dylib build OR fails to propagate the per-fixture cfg,
//! this fixture fails to compile with one of:
//!
//!   error[E0432]: unresolved import `lihaaf::SUITE_DEMO_MARKER`
//!   error[E0425]: cannot find value `SUITE_DEMO_MARKER` in crate `lihaaf`
//!
//! and lihaaf's own CI run fails — the multi-suite capability has a
//! self-bite test without needing a downstream adopter.

#[cfg(feature = "suite_demo")]
fn main() {
    let _ = lihaaf::SUITE_DEMO_MARKER;
}

// Without the cfg gate, the fixture would fail under the default suite's
// no-feature build (where SUITE_DEMO_MARKER is not exported). Lihaaf's
// disjoint-fixture-dirs invariant ensures this file only ever runs in the
// suite that enables the feature, but the cfg keeps it honest if a future
// adopter copy-pastes the fixture and forgets to point it at a feature-
// enabled suite.
#[cfg(not(feature = "suite_demo"))]
fn main() {}
