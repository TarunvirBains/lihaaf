//! Compat-mode normalizer (§3.2.2) — cross-module integration tests
//! for the `NormalizationContext::compat_short_cargo` flag.
//!
//! These tests sit against the `NormalizationContext` / `normalize`
//! surface re-exported at the crate root behind `#[doc(hidden)]`
//! (mirrors the other compat re-exports in `src/lib.rs`), parallel to
//! the inline unit cases (a)-(d) inside `src/normalize.rs`.
//! The integration variants cover the structural-rejection edge cases
//! the inline tests do not: uppercase hex, wrong hex length, and a
//! missing trailing slash.
//!
//! ## Why no filesystem
//!
//! Every test constructs a `NormalizationContext` by hand with a
//! fictitious `/home/u/.cargo/registry` so the assertions are
//! deterministic — no `$CARGO_HOME` lookup, no real registry layout
//! on disk. The normalizer is pure (input string → output string),
//! so these tests carry zero filesystem or environment dependencies.

use std::path::PathBuf;

use lihaaf::{NormalizationContext, normalize};

/// Build a compat-mode context with `compat_short_cargo = true`.
fn ctx_compat() -> NormalizationContext {
    NormalizationContext {
        workspace_root: PathBuf::from("/p"),
        sysroot: PathBuf::from("/sysroot"),
        cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
        compat_short_cargo: true,
        extra_substitutions: Vec::new(),
        strip_lines: Vec::new(),
        strip_line_prefixes: Vec::new(),
        keep_foreign_span_bodies: false,
    }
}

/// Build a v0.1-stable context with `compat_short_cargo = false`.
/// Identical shape to `ctx_compat` apart from the flag.
fn ctx_non_compat() -> NormalizationContext {
    NormalizationContext {
        workspace_root: PathBuf::from("/p"),
        sysroot: PathBuf::from("/sysroot"),
        cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
        compat_short_cargo: false,
        extra_substitutions: Vec::new(),
        strip_lines: Vec::new(),
        strip_line_prefixes: Vec::new(),
        keep_foreign_span_bodies: false,
    }
}

#[test]
fn compat_mode_emits_short_cargo_for_index_crates_io() {
    let input = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
    let out = normalize(input, &ctx_compat(), &PathBuf::from("/p/x"));
    assert_eq!(out, "  --> $CARGO/foo-1.0.0/src/lib.rs");
}

#[test]
fn compat_mode_emits_short_cargo_for_github_com() {
    let input =
        "  --> /home/u/.cargo/registry/src/github.com-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
    let out = normalize(input, &ctx_compat(), &PathBuf::from("/p/x"));
    assert_eq!(out, "  --> $CARGO/foo-1.0.0/src/lib.rs");
}

#[test]
fn compat_mode_leaves_non_registry_paths_unchanged() {
    // A line that mentions no `/registry/src/...` segment passes
    // through the post-pass untouched. The other three placeholder
    // substitutions ($DIR / $WORKSPACE / $RUST) still apply per the
    // longest-prefix-wins rule.
    let input = "  --> /p/tests/foo.rs:3:1\n";
    let out = normalize(input, &ctx_compat(), &PathBuf::from("/p/tests"));
    assert_eq!(out, "  --> $DIR/foo.rs:3:1");
}

#[test]
fn non_compat_mode_collapses_registry_hash_and_strips_foreign_tail() {
    // Non-compat mode: the literal-prefix substitution rewrites the
    // registry path to $CARGO/registry, then the Class K-fix collapses
    // the volatile `<host>-<16hex>` hash segment to $CARGO_HASH. The
    // :LINE:COL tail is stripped by D-3a for foreign pointers.
    let input = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
    let out = normalize(input, &ctx_non_compat(), &PathBuf::from("/p/x"));
    assert_eq!(
        out,
        "  --> $CARGO/registry/src/$CARGO_HASH/foo-1.0.0/src/lib.rs"
    );
}

#[test]
fn compat_mode_rejects_uppercase_hex() {
    // Trybuild's recognizer matches lowercase hex only (`b'0'..=b'9'`
    // and `b'a'..=b'f'`). Uppercase A-F prevents recognition and the
    // line falls through to whatever the literal-prefix loop did —
    // in compat mode that's nothing for the registry, so the
    // `/home/u/.cargo/registry/src/INDEX...` prefix stays verbatim.
    let input = "  --> /home/u/.cargo/registry/src/INDEX.crates.io-1234567890ABCDEF/foo-1.0.0/src/lib.rs:3:1\n";
    let out = normalize(input, &ctx_compat(), &PathBuf::from("/p/x"));
    assert_eq!(
        out,
        "  --> /home/u/.cargo/registry/src/INDEX.crates.io-1234567890ABCDEF/foo-1.0.0/src/lib.rs:3:1"
    );
    // Make the non-match observable in the assertion: no `$CARGO`
    // placeholder is emitted.
    assert!(
        !out.contains("$CARGO"),
        "uppercase hex must not match: {out:?}"
    );
}

#[test]
fn compat_mode_rejects_wrong_hex_length() {
    // 15 hex chars: too short — no match.
    let too_short = "  --> /home/u/.cargo/registry/src/index.crates.io-123456789abcdef/foo-1.0.0/src/lib.rs:3:1\n";
    let out_short = normalize(too_short, &ctx_compat(), &PathBuf::from("/p/x"));
    assert!(
        !out_short.contains("$CARGO"),
        "15-hex (too short) must not match: {out_short:?}"
    );
    assert_eq!(
        out_short,
        "  --> /home/u/.cargo/registry/src/index.crates.io-123456789abcdef/foo-1.0.0/src/lib.rs:3:1"
    );

    // 17 hex chars: with a literal hex byte where the trybuild source
    // requires `/`. The post-pass checks byte-17 for `/` and rejects
    // when it isn't — so a 17-hex input is rejected as missing the
    // structural slash, NOT because of length per se. The behavior is
    // the same: no `$CARGO` rewrite.
    let too_long = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef0/foo-1.0.0/src/lib.rs:3:1\n";
    let out_long = normalize(too_long, &ctx_compat(), &PathBuf::from("/p/x"));
    assert!(
        !out_long.contains("$CARGO"),
        "17-hex (slash check fails) must not match: {out_long:?}"
    );
    assert_eq!(
        out_long,
        "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef0/foo-1.0.0/src/lib.rs:3:1"
    );
}

#[test]
fn compat_mode_rejects_missing_trailing_slash() {
    // `index.crates.io-1234567890abcdef-suffix` — the byte after the
    // 16-hex run is `-`, not `/`. Structural condition (3) fails and
    // the post-pass leaves the line alone.
    let input =
        "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef-suffix/whatever\n";
    let out = normalize(input, &ctx_compat(), &PathBuf::from("/p/x"));
    assert!(
        !out.contains("$CARGO"),
        "missing `/` after 16-hex must not match: {out:?}"
    );
    assert_eq!(
        out,
        "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef-suffix/whatever"
    );
}
