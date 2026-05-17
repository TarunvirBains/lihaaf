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
//!    (`patch_tables_preserved_verbatim` — `[patch.crates-io]`
//!    `git`/`branch`/`tag`/`rev` fields pass through verbatim;
//!    `absolutizes_patch_path_entries` — `[patch.crates-io.X].path`
//!    is absolutized like `[dependencies.X].path` so the cxx-pilot
//!    `cxx = { path = "." }` / `cxx-build = { path = "gen/build" }` shapes
//!    work correctly from the staged manifest dir).
//! 7. Workspace key classes (FIX class B): `[package].workspace`,
//!    `[workspace].default-members`, `[workspace.dependencies.*].path`.
//! 8. Richer cargo-build regression (`cargo_accepts_rich_overlay_for_dylib_build`)
//!    exercising path-dep + `[patch.crates-io]` path entry + relative
//!    `--compat-root` — the production failure shapes from the Round-2 panel.
//!
//! ## Why every test is hermetic
//!
//! Each test owns a `tempfile::TempDir` and operates exclusively within
//! it. The overlay generator writes the staged manifest under
//! `<tempdir>/target/lihaaf-overlay/Cargo.toml` (the same shape every
//! production compat run produces), so the tempdir layout mirrors a
//! real fork checkout without polluting the lihaaf source tree.

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

/// Helper: read the staged overlay bytes as a UTF-8 string.
fn read_overlay(sibling_path: &Path) -> String {
    let bytes = std::fs::read(sibling_path)
        .expect("staged overlay `target/lihaaf-overlay/Cargo.toml` must exist");
    String::from_utf8(bytes).expect("staged overlay must be valid UTF-8")
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
fn virtual_workspace_with_inherited_workspace_package_is_rejected() {
    // Cargo's "inherited package metadata" pattern: a virtual workspace
    // hosts a `[workspace.package]` table for members to inherit via
    // `package.version.workspace = true`. The manifest itself is STILL a
    // virtual workspace (no top-level `[package]`) — cargo cannot build
    // it as a library, so the overlay must reject it just like the
    // bare-virtual-workspace case. Regression for round-2 review BLOCK:
    // earlier logic exempted virtual workspaces that carried
    // `[workspace.package]`, treating the inherited-metadata table as if
    // it made the root manifest buildable. It does not.
    let input = r#"[workspace]
members = ["crate-a", "crate-b"]

[workspace.package]
version = "0.1.0"
edition = "2021"
"#;
    let (_tmp, upstream) = write_upstream(input);
    let err = materialize_overlay(&upstream)
        .expect_err("virtual workspace with [workspace.package] must be rejected");
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
    //
    // **Path-absolutization caveat.** After the PR #34 redesign, the
    // staged overlay carries absolute paths in its `[lib] path` and
    // path-dep entries (resolved against the upstream crate dir). The
    // corpus expected files hold a `__UPSTREAM_DIR__` placeholder that
    // this test substitutes with the real tempdir before comparing —
    // the determinism guarantee is "same upstream dir → byte-identical
    // overlay", which the substitution restores.
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
        let expected_template = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("reading corpus expected {expected_path:?}: {e}"));

        let (_tmp, upstream) = write_upstream(&input);
        let plan = materialize_overlay(&upstream).expect("overlay must succeed");
        let actual = read_overlay(&plan.sibling_manifest);

        // Substitute the `__UPSTREAM_DIR__` placeholder in the expected
        // template with the real tempdir (forward-slash form, matching
        // the overlay code's `to_forward_slash` call). The placeholder
        // is a fixed-string substitution — no regex, per spec §6.1.
        let upstream_dir = upstream
            .parent()
            .expect("upstream manifest has a parent dir")
            .to_string_lossy()
            .replace('\\', "/");
        let expected = expected_template.replace("__UPSTREAM_DIR__", &upstream_dir);

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
    // Per spec §3.2.3: `[patch]` `git`/`branch`/`tag`/`rev` fields must
    // pass through verbatim — the overlay must NOT rewrite remote-source
    // fields.  (The `path` sub-key IS absolutized; see
    // `absolutizes_patch_path_entries` below.)  We assert two properties:
    //
    // 1. The `[patch.crates-io]` git/branch fields reach the output unchanged.
    // 2. The `[patch]` ordering relative to the canonical key sequence is
    //    honored (`patch` lands after `features` per `canonical_key_order()`).
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

/// `sibling_manifest` filename must be `Cargo.toml` — not
/// `Cargo.lihaaf.toml` or any other variant — so that
/// `cargo rustc --manifest-path <path>` accepts it without error.
///
/// Cargo validates the `--manifest-path` filename at startup and rejects
/// any path whose last component is not literally `Cargo.toml` (exit
/// code 1, ~43 ms, before any compilation work). This test pins the
/// contract at the overlay level so a future rename of the staged path
/// breaks here rather than silently causing every compat pilot to fail
/// with an opaque cargo error.
///
/// Regression: before this fix the overlay was staged as
/// `Cargo.lihaaf.toml` (a sibling of the upstream `Cargo.toml`), which
/// caused all four stage-2 pilot forks (cxx, serde-json, anyhow,
/// thiserror) to fail with `error_type: lihaaf_session_failed` / detail
/// "the manifest-path must be a path to a Cargo.toml file" on every CI
/// run — run https://github.com/TarunvirBains/lihaaf/actions/runs/25994537438.
#[test]
fn sibling_manifest_filename_is_cargo_toml_for_cargo_compat() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");

    let filename = plan
        .sibling_manifest
        .file_name()
        .and_then(|n| n.to_str())
        .expect("sibling_manifest must have a filename component");

    assert_eq!(
        filename, "Cargo.toml",
        "`sibling_manifest` filename must be `Cargo.toml` so cargo accepts \
         `--manifest-path`; got `{filename}`. Cargo rejects any manifest-path \
         whose last component is not literally `Cargo.toml`."
    );

    // Belt-and-suspenders: the staged overlay must live under
    // `target/lihaaf-overlay/` so it is implicitly cargo-ignored and
    // never pollutes the fork's worktree.
    let path_str = plan.sibling_manifest.to_string_lossy();
    assert!(
        path_str.contains("lihaaf-overlay"),
        "`sibling_manifest` must be staged under `target/lihaaf-overlay/`; \
         got `{path_str}`"
    );

    // The overlay content must be readable and contain the expected
    // crate-type canonicalization — verifies the staged file is well-formed.
    let content = std::fs::read_to_string(&plan.sibling_manifest)
        .expect("staged overlay must exist and be readable");
    assert!(
        content.contains(r#"crate-type = ["dylib", "rlib"]"#),
        "staged overlay must contain canonical crate-type; got:\n{content}"
    );
}

/// **Cargo actually accepts the staged overlay and builds the dylib.**
///
/// This test codifies the manual repro strict-swe Opus ran in the PR #34
/// adversarial review: build a synthetic single-crate fork with
/// `<upstream>/Cargo.toml` + `<upstream>/src/lib.rs`, materialize the
/// overlay, then invoke `cargo rustc --manifest-path <staged>
/// --crate-type=dylib --lib` and assert exit 0. Without the
/// path-absolutization fix landed in this PR, cargo emits
/// `can't find library "demo", rename file to "src/lib.rs" or specify
/// lib.path` and exits non-zero — every stage-2 pilot would then fail
/// with `lihaaf_session_failed` in the §3.3 envelope.
///
/// **Why this is gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS`.** The test
/// spawns a real `cargo rustc` invocation against a freshly-staged
/// crate, which downloads no deps but still costs ~5–10 s of wall-clock
/// and a few hundred MB of disk for the new target dir under
/// `<tempdir>/target/`. Local CI on RAM-limited boxes (Arch / WSL2 with
/// 4 GB cap) OOMs when this runs alongside `cargo test --all-features`;
/// authoritative verification happens in GitHub Actions, which sets the
/// env var. The gate fails the test loudly when the env var is set but
/// cargo is unavailable, so CI can never accidentally skip the bite.
///
/// **What this test bites.** Three independent regression vectors:
///
/// 1. A future overlay rewrite that drops the `[lib] path` absolutization
///    (BLOCKER class 1 from the panel review) — cargo's auto-discovery
///    would search `<staged_manifest_dir>/src/lib.rs` and fail.
/// 2. A future cargo version that surfaces a hard error for the
///    "empty bin/test/example/bench auto-discovery dir" case — the
///    overlay disables auto-discovery for non-lib targets, so this
///    test pins that behavior.
/// 3. A future change to the staged-path shape that drops the
///    `target/lihaaf-overlay/` parent — cargo's `--manifest-path`
///    filename check would then reject the overlay.
#[test]
fn cargo_accepts_staged_overlay_for_dylib_build() {
    // CI gate: this test is only authoritative under the green-button
    // CI lane that has the env var set. Local boxes opt-in by exporting
    // the variable; the default-skip keeps RAM-limited dev machines
    // safe.
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_staged_overlay_for_dylib_build: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this \
             automatically)"
        );
        return;
    }

    // Build a synthetic single-crate fork: upstream Cargo.toml + src/lib.rs.
    // The crate is intentionally minimal so the build is fast and
    // deterministic; the test bites the manifest-resolution path, not
    // the actual library code.
    let tmp = tempfile::tempdir().expect("creating tempdir for cargo build test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "// minimal library so cargo has something to compile.\npub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Sanity: the staged overlay is at <upstream>/target/lihaaf-overlay/Cargo.toml.
    let expected_staged = upstream_dir
        .join("target")
        .join("lihaaf-overlay")
        .join("Cargo.toml");
    assert_eq!(
        plan.sibling_manifest, expected_staged,
        "overlay must be staged at <upstream>/target/lihaaf-overlay/Cargo.toml"
    );

    // The acid test: invoke cargo rustc against the staged manifest.
    // The flags mirror `dylib::build()`'s production invocation (see
    // `src/dylib.rs`):
    //
    //   cargo rustc -p <crate> --lib --release --crate-type=dylib
    //          --manifest-path <staged> --target-dir <isolated>
    //
    // We point `--target-dir` at a sibling of the staged overlay so
    // build artifacts don't collide with the staged dir itself.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("demo")
        .arg("--lib")
        .arg("--release")
        .arg("--crate-type=dylib")
        .arg("--manifest-path")
        .arg(&plan.sibling_manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        // `-C prefer-dynamic` mirrors production; absence wouldn't
        // change build success on this minimal crate but keeps the
        // invocation faithful.
        .env("RUSTFLAGS", "-C prefer-dynamic")
        .output()
        .expect("spawning cargo rustc; CI must have cargo on PATH");

    assert!(
        output.status.success(),
        "cargo rustc must succeed against the staged overlay; got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// **Path-absolutization fixed-point: the staged overlay contains
/// absolute `[lib] path` pointing at the upstream `src/lib.rs`.**
///
/// This is the unit-style cousin of `cargo_accepts_staged_overlay_for_dylib_build`
/// — instead of running cargo, it inspects the staged overlay bytes
/// directly. The unit-style form runs on every CI lane (no env-var
/// gate) and bites the same BLOCKER class 1 regression: a future
/// refactor that drops or downgrades the path-absolutization step
/// would leave `[lib] path` either absent or relative, and cargo would
/// then fail to find the library.
///
/// The check is byte-level on the serialized overlay so it bites
/// regardless of the exact TOML serialization shape (`[lib] path =
/// "..."` inline vs `[lib]\npath = "..."` block form).
#[test]
fn staged_overlay_carries_absolute_lib_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");
    let content = read_overlay(&plan.sibling_manifest);

    // The expected absolute path uses forward-slash form (matches
    // production: the overlay code calls `to_forward_slash` on every
    // absolutized path so Windows backslashes never reach the TOML).
    let expected_lib_path = upstream_dir.join("src").join("lib.rs");
    let expected_forward = expected_lib_path.to_string_lossy().replace('\\', "/");

    assert!(
        content.contains(&format!(r#"path = "{expected_forward}""#)),
        "staged overlay must carry an absolute `[lib] path` pointing at \
         the upstream `src/lib.rs`; expected substring `path = \"{expected_forward}\"`, \
         got overlay:\n{content}"
    );

    // Auto-discovery for non-lib targets must be disabled so cargo
    // does not search the empty staged dir for `src/bin/`, `tests/`,
    // `examples/`, `benches/`. The overlay always writes all four to
    // make the lib-only intent explicit.
    for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
        assert!(
            content.contains(&format!("{key} = false")),
            "staged overlay must declare `{key} = false` to disable cargo's \
             auto-discovery of non-lib targets in the (empty) staged dir; \
             got overlay:\n{content}"
        );
    }
}

/// **Path-bearing dependency entries are absolutized.**
///
/// Workspace-style pilots (cxx's `cxx-build`/`cxx-gen`/etc, thiserror's
/// `thiserror-impl = { path = "impl" }`) declare relative path-deps
/// pointing at sibling crates. Without absolutization, the staged
/// overlay would tell cargo to look under
/// `<upstream>/target/lihaaf-overlay/impl/Cargo.toml`, which doesn't
/// exist — every workspace-style pilot would fail.
///
/// This test pins the rewrite across the three dependency tables
/// (`dependencies`, `dev-dependencies`, `build-dependencies`). The
/// platform-conditional `[target.*.dependencies]` table is covered
/// implicitly because the same `absolutize_deps_paths` helper handles
/// every dependency-table shape; explicit coverage there is unit-tested
/// inside `src/compat/overlay.rs` to keep this integration test focused
/// on the user-visible production shape.
#[test]
fn staged_overlay_absolutizes_path_dependencies() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
inner = { path = "inner" }

[dev-dependencies]
inner-dev = { path = "inner-dev" }

[build-dependencies]
inner-build = { path = "inner-build" }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");
    let content = read_overlay(&plan.sibling_manifest);

    let expected_inner = upstream_dir
        .join("inner")
        .to_string_lossy()
        .replace('\\', "/");
    let expected_inner_dev = upstream_dir
        .join("inner-dev")
        .to_string_lossy()
        .replace('\\', "/");
    let expected_inner_build = upstream_dir
        .join("inner-build")
        .to_string_lossy()
        .replace('\\', "/");

    assert!(
        content.contains(&format!(r#"path = "{expected_inner}""#)),
        "staged overlay must absolutize `[dependencies.inner].path`; expected \
         `path = \"{expected_inner}\"`, got overlay:\n{content}"
    );
    assert!(
        content.contains(&format!(r#"path = "{expected_inner_dev}""#)),
        "staged overlay must absolutize `[dev-dependencies.inner-dev].path`; \
         expected `path = \"{expected_inner_dev}\"`, got overlay:\n{content}"
    );
    assert!(
        content.contains(&format!(r#"path = "{expected_inner_build}""#)),
        "staged overlay must absolutize `[build-dependencies.inner-build].path`; \
         expected `path = \"{expected_inner_build}\"`, got overlay:\n{content}"
    );

    // Defense-in-depth: no relative `path = "inner..."` survives. A
    // refactor that absolutized only the first table found would slip
    // a relative entry through one of the others.
    for relative_form in [
        r#"path = "inner""#,
        r#"path = "inner-dev""#,
        r#"path = "inner-build""#,
    ] {
        assert!(
            !content.contains(relative_form),
            "no relative `{relative_form}` may survive in the overlay; got:\n{content}"
        );
    }
}

/// **`[patch.<registry>.X].path` entries are absolutized in the staged
/// overlay.**
///
/// Regression for Round-2 strict-swe Opus BLOCK-1 (cxx pilot): cxx's
/// `Cargo.toml` carries `[patch.crates-io] cxx = { path = "." }` and
/// `cxx-build = { path = "gen/build" }`.  After staging the overlay two
/// dirs deeper, those paths resolved against the staged manifest dir:
/// `"."` became a self-reference to the staged dir and `"gen/build"`
/// pointed at a nonexistent subdir.  This test pins the fix — path-form
/// patch entries are absolutized like `[dependencies.X].path`.
///
/// The `git`/`branch` form (`serde = { git = "..." }`) must still pass
/// through verbatim (covered by `patch_tables_preserved_verbatim`).
#[test]
fn absolutizes_patch_path_entries() {
    let tmp = tempfile::tempdir().expect("tempdir for patch path test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Mirrors the cxx pilot: two path-form entries + one git-form entry.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "cxx-demo"
version = "0.1.0"

[dependencies]
serde = "1"

[patch.crates-io]
cxx-demo = { path = "." }
cxx-demo-build = { path = "gen/build" }
serde = { git = "https://example.com/serde", branch = "main" }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");
    let content = read_overlay(&plan.sibling_manifest);

    // Parse the output to make assertions structure-level, not byte-level,
    // and tolerate inline-to-explicit-table canonicalization.
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must parse as TOML");
    let crates_io = parsed
        .get("patch")
        .and_then(|v| v.get("crates-io"))
        .expect("[patch.crates-io] must survive overlay");

    // cxx-demo path = "." → absolute upstream dir.
    let cxx_path = crates_io
        .get("cxx-demo")
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .expect("[patch.crates-io.cxx-demo].path must survive overlay");
    assert!(
        Path::new(cxx_path).is_absolute(),
        "[patch.crates-io.cxx-demo].path must be absolute after overlay; got `{cxx_path}`"
    );

    // cxx-demo-build path = "gen/build" → absolute upstream/gen/build.
    let build_path = crates_io
        .get("cxx-demo-build")
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .expect("[patch.crates-io.cxx-demo-build].path must survive overlay");
    assert!(
        Path::new(build_path).is_absolute(),
        "[patch.crates-io.cxx-demo-build].path must be absolute after overlay; got `{build_path}`"
    );
    assert!(
        build_path.ends_with("gen/build"),
        "[patch.crates-io.cxx-demo-build].path must end with `gen/build`; got `{build_path}`"
    );

    // serde git-form entry must pass through with git/branch fields intact.
    let serde_patch = crates_io
        .get("serde")
        .expect("[patch.crates-io.serde] must survive overlay");
    assert_eq!(
        serde_patch.get("git").and_then(|v| v.as_str()),
        Some("https://example.com/serde"),
        "patch git URL must be unchanged by overlay"
    );
    assert_eq!(
        serde_patch.get("branch").and_then(|v| v.as_str()),
        Some("main"),
        "patch branch must be unchanged by overlay"
    );

    // Defense-in-depth: no relative path survives in the [patch.*] region.
    let patch_start = content
        .find("[patch")
        .expect("[patch] section must be present in overlay");
    let patch_region = &content[patch_start..];
    assert!(
        !patch_region.contains(r#"path = ".""#) && !patch_region.contains(r#"path = "gen/build""#),
        "no relative path must survive in [patch.*]; got patch region:\n{patch_region}"
    );
}

/// **Workspace key classes (FIX class B): `[package].workspace`,
/// `[workspace].default-members`, and `[workspace.dependencies.*].path`
/// are absolutized in the staged overlay.**
///
/// Regression test for three key classes that `docs/compatibility-plan.md`
/// claimed "every path-bearing key" covered but the Round-2 panel found
/// missing.  Each class is exercised in the same manifest so a single
/// overlay run verifies all three.
#[test]
fn staged_overlay_absolutizes_workspace_key_classes() {
    let tmp = tempfile::tempdir().expect("tempdir for workspace key test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");

    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "member"
version = "0.1.0"
workspace = "../"

[workspace]
members = ["crate-a"]
default-members = ["crate-a", "crate-b"]

[workspace.dependencies]
shared-utils = { path = "utils" }

[dependencies]
serde = "1"
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");
    let content = read_overlay(&plan.sibling_manifest);

    // Parse to make assertions structure-level.
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must parse as TOML");

    // [package].workspace must be absolute.
    let pkg_ws = parsed
        .get("package")
        .and_then(|v| v.get("workspace"))
        .and_then(|v| v.as_str())
        .expect("[package].workspace must survive overlay");
    assert!(
        Path::new(pkg_ws).is_absolute(),
        "[package].workspace must be absolutized in the staged overlay; got `{pkg_ws}`"
    );

    // [workspace].default-members must have all-absolute entries.
    let dm = parsed
        .get("workspace")
        .and_then(|v| v.get("default-members"))
        .and_then(|v| v.as_array())
        .expect("[workspace].default-members must survive overlay");
    for entry in dm {
        let s = entry
            .as_str()
            .expect("default-members entry must be a string");
        assert!(
            Path::new(s).is_absolute(),
            "[workspace].default-members entry must be absolute; got `{s}`"
        );
    }

    // [workspace.dependencies.shared-utils].path must be absolute.
    let utils_path = parsed
        .get("workspace")
        .and_then(|v| v.get("dependencies"))
        .and_then(|v| v.get("shared-utils"))
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .expect("[workspace.dependencies.shared-utils].path must survive overlay");
    assert!(
        Path::new(utils_path).is_absolute(),
        "[workspace.dependencies.shared-utils].path must be absolutized; got `{utils_path}`"
    );
    assert!(
        utils_path.ends_with("utils"),
        "absolutized path must end with `utils` component; got `{utils_path}`"
    );

    // Defense-in-depth: relative workspace pointer must not survive.
    assert!(
        !content.contains(r#"workspace = "../""#),
        "relative [package].workspace must not survive in overlay; got:\n{content}"
    );
}

/// **Richer cargo-build regression: path-dep + `[patch.crates-io]` path
/// entry + relative `--compat-root` form.**  (FIX class D)
///
/// The existing `cargo_accepts_staged_overlay_for_dylib_build` test uses a
/// minimal crate that would NOT have caught FIX class A (relative
/// `--compat-root`) because `tempfile::tempdir()` returns an absolute path,
/// and would NOT have caught FIX class C (`[patch.crates-io.X].path`) because
/// it has no `[patch]` block.
///
/// This test exercises the three production failure shapes the Round-2 panel
/// surfaced:
///
/// 1. A path-dep (`demo-impl = { path = "impl" }`) — catches any regression
///    to `[dependencies.X].path` absolutization.
/// 2. A `[patch.crates-io]` block with `path = "."` (cxx pattern) — catches
///    FIX class C regressions.
/// 3. The manifest path is given to `materialize_overlay` as an absolute path
///    (reflecting what `CompatArgs::from_cli` now guarantees after FIX class A),
///    but `upstream_dir` is confirmed absolute before use — this is the
///    structural guard that FIX class A is exercised via the CLI layer (see
///    `CompatArgs::from_cli` absolutization) and the overlay layer handles only
///    the manifest path it is given.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` for the same OOM reason as
/// `cargo_accepts_staged_overlay_for_dylib_build`.
#[test]
fn cargo_accepts_rich_overlay_for_dylib_build() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_rich_overlay_for_dylib_build: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    // Build a synthetic fork with:
    //   - a path-dep pointing at <upstream>/impl/ (mirrors thiserror pattern)
    //   - a [patch.crates-io] entry with path = "." (mirrors cxx pattern)
    //
    // The path-dep crate is a stub with its own Cargo.toml + src/lib.rs.
    // The patch entry references the same crate; cargo will resolve the
    // patch to the real upstream dir after absolutization.
    let tmp = tempfile::tempdir().expect("creating tempdir for rich cargo build test");
    let upstream_dir = tmp.path();

    // Guarantee upstream_dir is absolute — mirrors what FIX class A
    // ensures at the CLI layer (CompatArgs::from_cli absolutizes compat_root).
    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute; FIX class A ensures compat_root is absolute before overlay"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Write the impl sub-crate.
    let impl_dir = upstream_dir.join("impl");
    std::fs::create_dir_all(impl_dir.join("src")).expect("creating impl/src/");
    std::fs::write(
        impl_dir.join("Cargo.toml"),
        r#"[package]
name = "rich-demo-impl"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("writing impl/Cargo.toml");
    std::fs::write(
        impl_dir.join("src").join("lib.rs"),
        "// impl stub\npub fn helper() {}\n",
    )
    .expect("writing impl/src/lib.rs");

    // Write the upstream manifest.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "rich-demo"
version = "0.1.0"
edition = "2021"

[dependencies]
rich-demo-impl = { path = "impl" }

[patch.crates-io]
rich-demo = { path = "." }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "// rich-demo stub\npub fn api() -> u32 { 42 }\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Sanity: overlay staged in expected location.
    let expected_staged = upstream_dir
        .join("target")
        .join("lihaaf-overlay")
        .join("Cargo.toml");
    assert_eq!(
        plan.sibling_manifest, expected_staged,
        "overlay must be staged at <upstream>/target/lihaaf-overlay/Cargo.toml"
    );

    // Verify that neither [dependencies.rich-demo-impl].path nor
    // [patch.crates-io.rich-demo].path carries a relative value.
    let content = read_overlay(&plan.sibling_manifest);
    assert!(
        !content.contains(r#"path = "impl""#),
        "relative [dependencies.rich-demo-impl].path must not survive; overlay:\n{content}"
    );
    assert!(
        !content.contains(r#"path = ".""#),
        "relative [patch.crates-io.rich-demo].path must not survive; overlay:\n{content}"
    );

    // The acid test: cargo rustc against the staged overlay.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("rich-demo")
        .arg("--lib")
        .arg("--release")
        .arg("--crate-type=dylib")
        .arg("--manifest-path")
        .arg(&plan.sibling_manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .env("RUSTFLAGS", "-C prefer-dynamic")
        .output()
        .expect("spawning cargo rustc; CI must have cargo on PATH");

    assert!(
        output.status.success(),
        "cargo rustc must succeed against the rich staged overlay; got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
