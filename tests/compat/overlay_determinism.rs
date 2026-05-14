//! Phase 2 of compat mode (issue #11) — sibling-manifest overlay
//! determinism integration tests.
//!
//! Every test in this file calls into `lihaaf::compat_overlay_*`
//! re-exports (declared in `src/lib.rs`). The re-exports are
//! `#[doc(hidden)]` — they exist exclusively for this test crate. The
//! supported entry into the overlay generator is `cargo lihaaf
//! --compat`, not the Rust API.
//!
//! The suite covers:
//!
//! 1. The `[lib] crate-type` canonicalization matrix
//!    (`upstream_without_lib_section_adds_dylib_rlib` …
//!    `upstream_with_cdylib_preserved_after_pair`) — every combination
//!    listed in `docs/compatibility-plan.md` §3.2.3.
//! 2. The byte-shape invariants
//!    (`trailing_whitespace_stripped`,
//!    `line_endings_are_lf_on_every_platform`,
//!    `canonical_key_order_is_stable`).
//! 3. Comment scraping
//!    (`comments_stripped_and_recorded`).
//! 4. Idempotency under rerun
//!    (`idempotent_rerun_no_byte_change` — second run does not touch
//!    mtime).
//! 5. The cross-binary determinism corpus
//!    (`byte_identical_across_two_lihaaf_binaries_on_corpus` — five
//!    representative `Cargo.toml` shapes under
//!    `tests/compat/overlay_corpus/`, each checked against a
//!    pre-committed `*.expected.toml`).
//! 6. The §3.2.3 risk section's invariant
//!    (`patch_tables_preserved_verbatim` — `[patch.crates-io]` data is
//!    passed through unmodified except for the spec's documented inline-
//!    to-explicit-table canonicalization the `toml` crate's serializer
//!    performs).
//!
//! ## Why every test is hermetic
//!
//! Each test owns a `tempfile::TempDir` and operates exclusively within
//! it. The overlay generator writes `Cargo.lihaaf.toml` next to the
//! input it reads, so the tempdir layout mirrors a real fork checkout
//! without polluting the lihaaf source tree.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lihaaf::compat_overlay_materialize as materialize_overlay;

/// Helper: drop an input `Cargo.toml` in a fresh tempdir and return
/// the directory handle plus the path to the new manifest.
fn write_upstream(input: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("creating tempdir for overlay test");
    let path = tmp.path().join("Cargo.toml");
    std::fs::write(&path, input).expect("writing upstream Cargo.toml");
    (tmp, path)
}

/// Helper: read the sibling overlay bytes as a UTF-8 string.
fn read_overlay(sibling_path: &Path) -> String {
    let bytes = std::fs::read(sibling_path).expect("sibling Cargo.lihaaf.toml must exist");
    String::from_utf8(bytes).expect("sibling overlay must be valid UTF-8")
}

#[test]
fn idempotent_rerun_no_byte_change() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[lib]
crate-type = ["rlib"]
"#;
    let (_tmp, upstream) = write_upstream(input);

    let first = materialize_overlay(&upstream).expect("first overlay run");
    let first_bytes = std::fs::read(&first.sibling_manifest).unwrap();
    let first_mtime = std::fs::metadata(&first.sibling_manifest)
        .unwrap()
        .modified()
        .unwrap();

    // Sleep just past filesystem mtime granularity so a spurious
    // re-write would be observable. The idempotency contract is:
    // bytes match → no write → mtime preserved.
    std::thread::sleep(Duration::from_millis(50));

    let second = materialize_overlay(&upstream).expect("second overlay run");
    let second_bytes = std::fs::read(&second.sibling_manifest).unwrap();
    let second_mtime = std::fs::metadata(&second.sibling_manifest)
        .unwrap()
        .modified()
        .unwrap();

    assert_eq!(
        first_bytes, second_bytes,
        "second run must produce byte-identical overlay"
    );
    assert_eq!(
        first_mtime, second_mtime,
        "second run must not bump sibling mtime (idempotency)"
    );
}

#[test]
fn upstream_without_lib_section_adds_dylib_rlib() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);
    assert!(
        out.contains(r#"crate-type = ["dylib", "rlib"]"#),
        "expected `[\"dylib\", \"rlib\"]` in:\n{out}"
    );
    assert!(!plan.upstream_already_has_dylib);
}

#[test]
fn upstream_with_rlib_only_keeps_rlib_adds_dylib() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[lib]
crate-type = ["rlib"]
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);
    assert!(
        out.contains(r#"crate-type = ["dylib", "rlib"]"#),
        "expected `[\"dylib\", \"rlib\"]` (dylib prepended, rlib retained) in:\n{out}"
    );
    assert!(!plan.upstream_already_has_dylib);
}

#[test]
fn upstream_with_dylib_already_unchanged_semantics() {
    // Input already declares dylib; per spec §3.2.3 the sibling is
    // still written (uniform §3.3 envelope classification) and the
    // output retains `rlib` for the `cargo test` baseline.
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[lib]
crate-type = ["dylib"]
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);
    assert!(
        out.contains(r#"crate-type = ["dylib", "rlib"]"#),
        "expected `[\"dylib\", \"rlib\"]` (rlib appended) in:\n{out}"
    );
    assert!(
        plan.upstream_already_has_dylib,
        "the plan must record that upstream already declared dylib"
    );
}

#[test]
fn upstream_with_cdylib_preserved_after_pair() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[lib]
crate-type = ["cdylib"]
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);
    assert!(
        out.contains(r#"crate-type = ["dylib", "rlib", "cdylib"]"#),
        "expected `[\"dylib\", \"rlib\", \"cdylib\"]` in:\n{out}"
    );
}

#[test]
fn canonical_key_order_is_stable() {
    // Inputs intentionally shuffled — `[features]` before `[package]`,
    // `[dependencies]` between them — so we can prove the canonical
    // order is honored on output rather than just echoing input order.
    let input = r#"[features]
default = []

[dependencies]
serde = "1"

[package]
name = "demo"
version = "0.1.0"

[workspace]
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);

    let headers: Vec<&str> = out.lines().filter(|l| l.starts_with('[')).collect();
    // Per `canonical_key_order()` the expected order is:
    // package, lib (newly inserted), dependencies, features, workspace.
    assert_eq!(
        headers,
        vec![
            "[package]",
            "[lib]",
            "[dependencies]",
            "[features]",
            "[workspace]"
        ],
        "canonical order violated; output:\n{out}"
    );
}

#[test]
fn workspace_root_manifest_is_rejected_with_directed_diagnostic() {
    // `[workspace]` without `[package]` is a workspace root — cargo
    // cannot build it as a library and the lihaaf stage-3 dylib pass
    // would fail with an opaque error. Phase 2 overlay rejects this
    // upfront with a directed diagnostic pointing the adopter at a
    // member crate's Cargo.toml.
    let input = r#"[workspace]
members = ["crate-a", "crate-b"]
"#;
    let (_tmp, upstream) = write_upstream(input);
    let err = materialize_overlay(&upstream).expect_err("workspace root must be rejected");
    match err {
        lihaaf::Error::Cli {
            clap_exit_code,
            message,
        } => {
            assert_eq!(clap_exit_code, 2, "exit code must be the clap usage code");
            assert!(
                message.contains("workspace"),
                "diagnostic must name `workspace`; got: {message}"
            );
            assert!(
                message.contains("--compat-root"),
                "diagnostic must point at the flag; got: {message}"
            );
            assert!(
                message.contains("member crate"),
                "diagnostic must direct adopter to a member crate; got: {message}"
            );
        }
        other => panic!("expected Error::Cli, got {other:?}"),
    }
}

#[test]
fn manifest_without_package_or_workspace_still_tolerated() {
    // An empty / odd manifest (no `[package]`, no `[workspace]`) is
    // not the workspace-root shape — the rejection is targeted. The
    // overlay still produces a sibling without complaint, treating
    // the shape as "no library to build" (uniform §3.3 envelope
    // classification, no lib rewrite).
    let input = "# empty manifest\n";
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("non-workspace empty manifest must succeed");
    assert!(plan.sibling_manifest.exists());
}

#[test]
fn multiline_string_hash_is_not_a_comment() {
    // Regression bite for the §3.3 envelope's `overlay.dropped_comments`
    // inventory. A `#` byte that lives inside a TOML triple-quoted
    // string is content, not a comment marker — the scanner must walk
    // string state across line boundaries to honor that.
    let input = r#"[package]
name = "demo"
version = "0.1.0"
description = """
line with #notacomment
and another #alsonot
"""

[dependencies]
# real comment
serde = "1"
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    // The only dropped comment from this input is the line that reads
    // `# real comment`. The `#notacomment` and `#alsonot` lines live
    // inside a multi-line basic string and must NOT be in the inventory.
    assert!(
        plan.dropped_comments
            .iter()
            .all(|c| !c.contains("notacomment") && !c.contains("alsonot")),
        "multi-line string bodies must not surface as comments; got {:?}",
        plan.dropped_comments
    );
    assert!(
        plan.dropped_comments.iter().any(|c| c == "real comment"),
        "the genuine comment line must still be captured; got {:?}",
        plan.dropped_comments
    );
}

#[test]
fn comments_stripped_and_recorded() {
    let input = r#"# header comment
[package]
name = "demo"
version = "0.1.0"

[dependencies]
# pin reason
serde = "1" # trailing
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);

    assert!(
        !out.contains('#'),
        "overlay must not contain any `#` markers; got:\n{out}"
    );
    // The plan records the three dropped comments. Order matches
    // input order so the §3.3 envelope can render them faithfully.
    assert_eq!(
        plan.dropped_comments.len(),
        3,
        "got: {:?}",
        plan.dropped_comments
    );
    assert_eq!(plan.dropped_comments[0], "header comment");
    assert_eq!(plan.dropped_comments[1], "pin reason");
    assert_eq!(plan.dropped_comments[2], "trailing");
}

#[test]
fn byte_identical_across_two_lihaaf_binaries_on_corpus() {
    // The corpus is the cross-binary determinism guard called out in
    // spec §3.2.3. Two lihaaf binaries built against the same `toml`
    // 1.x patch level must produce the byte-identical expected output
    // for every fixture. Divergence on a `toml` patch bump trips this
    // test in CI; the careful-coder handling the bump regenerates the
    // corpus and re-asserts the `overlay_serializer_drift` envelope
    // contract (Phase 8 surfaces this).
    let corpus_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("compat")
        .join("overlay_corpus");
    let names = [
        "bare_package",
        "with_rlib_only",
        "with_cdylib",
        "with_patch_section",
        "with_comments",
    ];
    let mut checked = 0usize;
    for name in &names {
        let input_path = corpus_dir.join(format!("{name}.input.toml"));
        let expected_path = corpus_dir.join(format!("{name}.expected.toml"));
        let input = std::fs::read_to_string(&input_path)
            .unwrap_or_else(|e| panic!("reading corpus input {input_path:?}: {e}"));
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("reading corpus expected {expected_path:?}: {e}"));

        let (_tmp, upstream) = write_upstream(&input);
        let plan = materialize_overlay(&upstream).expect("overlay must succeed");
        let actual = read_overlay(&plan.sibling_manifest);

        assert_eq!(
            actual, expected,
            "corpus `{name}` drifted from expected output.\n\
             If a `toml` 1.x patch bump produced this drift, regenerate \
             `tests/compat/overlay_corpus/{name}.expected.toml` and bump \
             the spec §3.2.3 reference. Otherwise this is a bug in the \
             overlay serializer.\n\n--- expected ---\n{expected}\n--- actual ---\n{actual}"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 5,
        "corpus must include all 5 representative fixtures"
    );
}

#[test]
fn trailing_whitespace_stripped() {
    // Trailing whitespace in the input must be normalized away in the
    // output (per §3.2.3 byte-shape invariants). Note that the `toml`
    // crate will silently swallow trailing whitespace on parse, so the
    // test is most useful as a defense-in-depth check on
    // `post_process_output`.
    let input = "[package]   \nname = \"demo\"  \nversion = \"0.1.0\"\n";
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);
    for (idx, line) in out.lines().enumerate() {
        assert!(
            !line.ends_with(' ') && !line.ends_with('\t'),
            "line {idx} ({line:?}) has trailing whitespace; full output:\n{out}"
        );
    }
}

#[test]
fn line_endings_are_lf_on_every_platform() {
    // The overlay must always be LF, even when the upstream uses
    // CRLF (Windows checkouts). Any `\r` in the output is a defect.
    let input = "[package]\r\nname = \"demo\"\r\nversion = \"0.1.0\"\r\n";
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let bytes = std::fs::read(&plan.sibling_manifest).expect("sibling exists");
    assert!(
        !bytes.contains(&b'\r'),
        "overlay must contain no `\\r` bytes; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn patch_tables_preserved_verbatim() {
    // Per spec §3.2.3 risks: `[patch]` overlays are pass-through —
    // the overlay code must NOT touch them. We assert two properties:
    //
    // 1. The `[patch.crates-io]` table reaches the output unchanged
    //    in its data content (keys + values match the input).
    // 2. The `[patch]` ordering relative to the canonical key sequence
    //    is honored (`patch` lands after `features` per
    //    `canonical_key_order()`).
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[dependencies]
serde = "1"

[patch.crates-io]
serde = { git = "https://example.com/serde", branch = "main" }
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let out = read_overlay(&plan.sibling_manifest);

    // Parse the output back to make the assertion robust to inline-
    // vs-explicit table canonicalization (the `toml` 1.x serializer
    // renders `[patch.crates-io] serde = { git = "..." }` as
    // `[patch.crates-io.serde] git = "..."`; the structured data is
    // identical, and that's what `[patch]` resolution cares about).
    let parsed: toml::Value = toml::from_str(&out).expect("output must parse");
    let patch = parsed
        .get("patch")
        .expect("patch table must survive overlay")
        .get("crates-io")
        .expect("crates-io section must survive overlay")
        .get("serde")
        .expect("serde patch entry must survive overlay");
    assert_eq!(
        patch.get("git").and_then(|v| v.as_str()),
        Some("https://example.com/serde"),
        "patch git URL must match input"
    );
    assert_eq!(
        patch.get("branch").and_then(|v| v.as_str()),
        Some("main"),
        "patch branch must match input"
    );

    // The `[patch]` header must come AFTER `[features]` per canonical
    // order (features is at index 10, patch at index 11 in
    // `canonical_key_order()`). Here features is absent, so patch
    // simply lands after dependencies.
    let dep_idx = out.find("[dependencies]").expect("deps section present");
    let patch_idx = out.find("[patch").expect("patch section present");
    assert!(
        patch_idx > dep_idx,
        "[patch] must follow [dependencies] in canonical order; got:\n{out}"
    );
}
