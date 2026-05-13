//! Multi-suite end-to-end coverage. Compile_pass fixture for the
//! `[[package.metadata.lihaaf.suite]]` named "suite_demo".
//!
//! Two-sided regression guard. Each side fails independently:
//!
//! 1. **Per-fixture `--cfg feature="suite_demo"` propagation**
//!    (`worker::spawn_and_monitor` → `apply_feature_cfgs`). If the
//!    per-fixture rustc invocation does NOT receive
//!    `--cfg feature="suite_demo"`, the `#[cfg(feature = "suite_demo")]`
//!    branch is elided and the `#[cfg(not(feature = "suite_demo"))]`
//!    `compile_error!` below fires with a fixed, greppable message —
//!    failing compilation of this compile_pass fixture and flipping CI red.
//!
//! 2. **Dylib feature propagation** (`dylib::build` →
//!    `BuildParams::features`). `lihaaf::SUITE_DEMO_MARKER` is exported
//!    only when the dylib was built with `--features suite_demo`
//!    (see `src/lib.rs` — the const is `#[cfg(feature = "suite_demo")]`).
//!    If the dylib build omits the feature, the marker is absent from
//!    the dylib's symbol table and rustc emits
//!    `error[E0432]: unresolved import lihaaf::SUITE_DEMO_MARKER` (or
//!    `error[E0425]: cannot find value ...`). The fixture fails and
//!    CI flips red.
//!
//! Either failure surfaces without needing a downstream adopter to
//! report the regression. The two `#[cfg]` branches are deliberately
//! complementary so the file ALWAYS produces a compile-time signal:
//! the only way for this fixture to compile is for BOTH halves to be
//! intact (correct dylib build AND correct per-fixture cfg
//! propagation).
//!
//! A source-level shape check in `tests/lihaaf_fixture_shape.rs`
//! pins the dual-bite design itself so a future maintainer cannot
//! silently revert this file to a one-sided guard.

#[cfg(feature = "suite_demo")]
fn main() {
    let _ = lihaaf::SUITE_DEMO_MARKER;
}

// Per-fixture cfg propagation regression guard. Fires when this
// fixture's rustc invocation is missing `--cfg feature="suite_demo"`.
// The fixed message is greppable by CI / triage scripts.
#[cfg(not(feature = "suite_demo"))]
compile_error!(
    "lihaaf multi-suite regression: this fixture must be compiled with \
     --cfg feature=\"suite_demo\". The suite_demo named suite \
     ([[package.metadata.lihaaf.suite]] in lihaaf's own Cargo.toml) is \
     responsible for propagating the suite's features to per-fixture \
     rustc invocations. If you see this error, the propagation has \
     regressed — re-check worker::spawn_and_monitor and \
     apply_feature_cfgs."
);
