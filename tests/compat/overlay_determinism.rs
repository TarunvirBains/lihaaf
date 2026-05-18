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
//!    (`byte_identical_across_two_lihaaf_binaries_on_corpus` — eight
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
//! 7. Workspace key classes (low-level): `[package].workspace`,
//!    `[workspace].default-members`, `[workspace.dependencies.*].path`
//!    absolutization is covered by the unit-level tests in
//!    `src/compat/overlay.rs::tests`. At the integration level, the
//!    workspace-identity fix (issue #36) overrides those values
//!    unconditionally — see test bullet 9 below.
//! 8. Richer cargo-build regression (`cargo_accepts_rich_overlay_for_dylib_build`)
//!    exercising path-dep + `[patch.crates-io]` path entry + relative
//!    `--compat-root` — the production failure shapes from the Round-2 panel.
//! 9. Workspace-identity fix for issue #36 + R2 inheritance-preservation
//!    fixup (PR #37 Codex + Gemini BLOCK):
//!    - `staged_overlay_overrides_upstream_workspace_inheritance`
//!      pins the R2 selective-rewrite contract: the staged overlay
//!      strips only `members` / `exclude` / `default-members` from
//!      `[workspace]` and preserves `dependencies`, `package`,
//!      `lints`, `metadata`, `resolver`, plus any unknown
//!      `[workspace.X]`.
//!    - `staged_overlay_rejects_workspace_member_manifest` pins the
//!      Option-C decision: `[package].workspace = "<path>"` manifests
//!      are REJECTED (out-of-scope for v0.1.0-beta.6).
//!    - `staged_overlay_rejects_implicit_workspace_member_manifest`
//!      is the R3 extension (PR #37 Codex BLOCK fixup): manifests
//!      that lack a local `[workspace]` but carry any
//!      `{ workspace = true }` inheritance reference are ALSO
//!      rejected — they would otherwise produce a manifest with
//!      stranded inheritance refs that cargo fails to parse with
//!      "workspace inheritance was specified but `[workspace.X]`
//!      was not defined".
//!    - `staged_overlay_rejects_manifest_with_ancestor_workspace`
//!      is the R4 extension (PR #37 R3 Codex BLOCK fixup):
//!      manifests with no local `[workspace]` AND no
//!      `{ workspace = true }` references are ALSO rejected when
//!      an ancestor `Cargo.toml` on the filesystem walk-up carries
//!      `[workspace]` — the ancestor may carry `[patch]` /
//!      `[replace]` / `[profile]` / `resolver` /
//!      `[workspace.dependencies]` tables that produce a divergent
//!      cargo dependency graph between baseline (walks up, sees the
//!      ancestor) and overlay (terminates the walk-up at the staged
//!      manifest, skips the ancestor entirely) — and therefore
//!      false compat verdicts.
//!    - `staged_overlay_allows_root_with_local_workspace_and_inheritance_refs`
//!      is the negative-case companion: a workspace ROOT carrying
//!      both `[workspace]` and `{ workspace = true }` refs MUST
//!      succeed, since the refs resolve LOCALLY.
//!    - `cargo_accepts_workspace_style_overlay_for_dylib_build` is
//!      the cargo-level proof for the membership-stripping case.
//!    - `cargo_accepts_workspace_inheritance_reference_in_overlay`
//!      is the cargo-level proof for the R2 inheritance-preservation
//!      contract (the Codex repro that BLOCKed R1).
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
    // package, lib (newly inserted), dependencies, features,
    // patch.crates-io.<self> (newly INJECTED by the Option H Rule 1
    // self-patch policy — issues #40 / #47 — because `[package].name`
    // is set and no upstream `[patch.crates-io.demo]` exists),
    // workspace.
    assert_eq!(
        headers,
        vec![
            "[package]",
            "[lib]",
            "[dependencies]",
            "[features]",
            "[patch.crates-io.demo]",
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
        "with_replace_section",
        // Two new fixtures pin the issue #40/#47 Option H self-patch
        // policy emission: Rule 1 INJECT for clean upstreams (the
        // anyhow / thiserror / serde_json shape) and Rule 2 REMAP
        // for upstreams that already carry `[patch.crates-io.<self>]
        // = { path = "." }` (the cxx shape). Both rules emit the
        // same byte form (absolutized staged-overlay-dir); the
        // distinction is in the INPUT.
        "with_self_patch_injected",
        "with_self_patch_remapped",
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
        checked, 8,
        "corpus must include all 8 representative fixtures \
         (6 existing + 2 new for the issue #40/#47 Option H \
         Rule 1 INJECT and Rule 2 REMAP policy)"
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

/// **Workspace identity in the staged overlay: membership keys stripped,
/// inheritance tables PRESERVED (issues #36 + PR #37 R2 fixup).**
///
/// History:
/// - v0.1.0-beta.5 (PR #34): no `[workspace]` override at all → cargo
///   walked UP and attached the overlay to the upstream workspace,
///   failing every workspace-style pilot with `package <X> is a member
///   of the wrong workspace`.
/// - v0.1.0-beta.6 R1 (PR #37 commit 1f6520b): replaced `[workspace]`
///   with an empty table → fixed the "wrong workspace" error but
///   stranded `{ workspace = true }` inheritance references (Codex +
///   Gemini panel BLOCK: "workspace inheritance was specified but
///   `[workspace.dependencies]` was not defined").
/// - v0.1.0-beta.6 R2 (this commit): selective rewrite — strip ONLY
///   the membership keys (`members`, `exclude`, `default-members`)
///   from `[workspace]`, preserve every other table.
///
/// This test pins the R2 invariant on a workspace-ROOT upstream
/// (`[package]` + `[workspace]` in the same Cargo.toml — the actual
/// shape of cxx / serde-json / anyhow / thiserror upstreams).
///
/// **Why this replaces the prior `…absolutizes_workspace_key_classes`
/// test.** That test verified an intermediate behavior: workspace
/// key-class paths were absolutized and survived. R1 clobbered them.
/// R2 preserves the inheritance tables (`workspace.dependencies`,
/// `workspace.package`, `workspace.lints`, `workspace.metadata`,
/// `workspace.resolver`, plus any unknown `[workspace.X]`) while
/// stripping only the membership keys. The low-level absolutization
/// step (`absolutize_path_bearing_keys`) still runs and is still
/// covered by the unit-level tests in
/// `src/compat/overlay.rs::tests`. The earlier-pass
/// absolutization of `[workspace.dependencies.X].path` is now
/// LOAD-BEARING (R2-preserved tables carry it through).
///
/// **The workspace-MEMBER case (`[package].workspace = "../"`) is
/// covered by a separate test** —
/// `staged_overlay_rejects_workspace_member_manifest` — because R2
/// rejects that shape rather than overlaying it. See the module-level
/// "Workspace-inheritance override" section in `src/compat/overlay.rs`
/// for the full rationale.
#[test]
fn staged_overlay_overrides_upstream_workspace_inheritance() {
    let tmp = tempfile::tempdir().expect("tempdir for workspace override test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Upstream manifest is the workspace-ROOT case — `[package]` +
    // `[workspace]` in the SAME file. Carries:
    //   - `[workspace] members = [...]`           (stripped)
    //   - `[workspace] exclude = [...]`           (stripped)
    //   - `[workspace] default-members`           (stripped)
    //   - `[workspace.dependencies.X].path = "..."` (PRESERVED — R2)
    //   - `[workspace.package].edition`           (PRESERVED — R2)
    //   - `[workspace.lints].rust.unsafe_code`    (PRESERVED — R2)
    //   - `[workspace.metadata].my-tool.key`      (PRESERVED — R2)
    //   - `[workspace.resolver]`                  (PRESERVED — R2)
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "ws-root"
version = "0.1.0"

[workspace]
members = ["crate-a"]
exclude = ["scratch"]
default-members = ["crate-a"]
resolver = "2"

[workspace.dependencies]
shared-utils = { path = "utils" }

[workspace.package]
edition = "2021"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.metadata.my-tool]
key = "value"

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

    // Parse to make assertions structure-level (and to verify the
    // overlay still parses as valid TOML after the override).
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must parse as TOML");

    // 1. `[workspace]` MUST exist (so cargo treats the overlay as its
    //    own workspace root; the walk-up terminates here).
    let ws = parsed
        .get("workspace")
        .and_then(|v| v.as_table())
        .expect("overlay must declare `[workspace]` (even if only inheritance tables) to be a workspace root");

    // 2. Membership keys MUST be stripped. These are the keys that
    //    caused the v0.1.0-beta.5 "wrong workspace" failure — claiming
    //    the upstream's path-dep crates as overlay members.
    for stripped_key in ["members", "exclude", "default-members"] {
        assert!(
            !ws.contains_key(stripped_key),
            "overlay's `[workspace].{stripped_key}` MUST be stripped to avoid \
             cross-workspace membership conflicts; got entries: {:?}",
            ws.keys().collect::<Vec<_>>()
        );
    }

    // 3. Inheritance tables MUST survive — this is the R2 invariant
    //    that the R1 attempt got wrong. Each of these tables backs
    //    a `{ workspace = true }` reference shape in cargo.
    assert!(
        ws.contains_key("dependencies"),
        "overlay must preserve `[workspace.dependencies]` (R2 — used by \
         `{{ workspace = true }}` references in `[dependencies]`); got entries: {:?}",
        ws.keys().collect::<Vec<_>>()
    );
    assert!(
        ws.contains_key("package"),
        "overlay must preserve `[workspace.package]` (R2 — used by \
         `{{ workspace = true }}` references in `[package]`); got entries: {:?}",
        ws.keys().collect::<Vec<_>>()
    );
    assert!(
        ws.contains_key("lints"),
        "overlay must preserve `[workspace.lints]` (R2 — used by \
         `{{ workspace = true }}` references in `[lints]`); got entries: {:?}",
        ws.keys().collect::<Vec<_>>()
    );
    assert!(
        ws.contains_key("metadata"),
        "overlay must preserve `[workspace.metadata]` (R2 — passes through \
         tool-owned namespaced metadata); got entries: {:?}",
        ws.keys().collect::<Vec<_>>()
    );
    assert!(
        ws.contains_key("resolver"),
        "overlay must preserve `[workspace.resolver]` (R2 — cargo's deps resolver \
         version); got entries: {:?}",
        ws.keys().collect::<Vec<_>>()
    );

    // 4. R2 fix-specific: `[workspace.dependencies.shared-utils]` must
    //    SURVIVE (R1 stripped it). The earlier `absolutize_path_bearing_keys`
    //    pass has rewritten the `path = "utils"` to an absolute path; we
    //    verify the dep entry is there.
    let ws_deps = ws
        .get("dependencies")
        .and_then(|v| v.as_table())
        .expect("`[workspace.dependencies]` must be a table");
    assert!(
        ws_deps.contains_key("shared-utils"),
        "R2 invariant: `[workspace.dependencies.shared-utils]` MUST survive \
         (R1 stripped it; this is the regression Codex + Gemini caught); \
         got workspace.dependencies entries: {:?}",
        ws_deps.keys().collect::<Vec<_>>()
    );

    // 5. R2 fix-specific: `[workspace.package].edition` must SURVIVE so
    //    a future `[package] edition.workspace = true` reference would
    //    resolve correctly.
    let ws_pkg = ws
        .get("package")
        .and_then(|v| v.as_table())
        .expect("`[workspace.package]` must be a table");
    assert_eq!(
        ws_pkg.get("edition").and_then(|v| v.as_str()),
        Some("2021"),
        "R2 invariant: `[workspace.package].edition` MUST survive so \
         `{{ workspace = true }}` in `[package].edition` resolves; got: {:?}",
        ws_pkg
    );

    // 6. R2 fix-specific: `[workspace.lints]` content must SURVIVE.
    let ws_lints = ws
        .get("lints")
        .and_then(|v| v.as_table())
        .expect("`[workspace.lints]` must be a table");
    assert!(
        ws_lints.contains_key("rust"),
        "R2 invariant: `[workspace.lints.rust]` MUST survive so `{{ workspace = true }}` \
         in `[lints.rust]` resolves; got: {:?}",
        ws_lints
    );

    // 7. The non-workspace part of the overlay (the actual buildable
    //    crate identity) MUST be preserved.
    let pkg_table = parsed
        .get("package")
        .and_then(|v| v.as_table())
        .expect("overlay must preserve `[package]` table");
    assert_eq!(
        pkg_table.get("name").and_then(|v| v.as_str()),
        Some("ws-root"),
        "overlay must preserve `[package].name`"
    );
    assert_eq!(
        pkg_table.get("version").and_then(|v| v.as_str()),
        Some("0.1.0"),
        "overlay must preserve `[package].version`"
    );

    // 8. The regular `[dependencies]` table must also pass through (the
    //    override only targets `[workspace]`, not deps).
    let deps = parsed
        .get("dependencies")
        .and_then(|v| v.as_table())
        .expect("overlay must preserve `[dependencies]` table");
    assert!(
        deps.contains_key("serde"),
        "overlay must preserve `[dependencies.serde]`"
    );

    // 9. Defense-in-depth on the serialized bytes — neither the
    //    `members` nor `default-members` keys must survive in any form
    //    (the absolutization-then-strip path could in principle have a
    //    leak; this catches it). `exclude` would have been absolutized
    //    to a path starting with the tempdir prefix; we check the bare
    //    key, which still appears in the absolutized array form.
    //
    //    Note: we cannot blindly `!contains("members")` because
    //    `default-members` would match; we use precise key syntax.
    assert!(
        !content.contains("\nmembers ="),
        "[workspace] members must not survive in overlay; got:\n{content}"
    );
    assert!(
        !content.contains("\ndefault-members ="),
        "[workspace] default-members must not survive in overlay; got:\n{content}"
    );
    assert!(
        !content.contains("\nexclude ="),
        "[workspace] exclude must not survive in overlay; got:\n{content}"
    );
}

/// **Workspace-MEMBER manifest is REJECTED (PR #37 R2 — Option C).**
///
/// When the overlay manifest itself declares `[package].workspace =
/// "<path>"`, it is a member of an ANCESTOR workspace — and the
/// ancestor (not the manifest itself) is where the actual
/// `[workspace.dependencies]` / `[workspace.package]` / `[workspace.lints]`
/// tables live. To overlay this shape and keep inheritance working,
/// we would need to read the ancestor's `Cargo.toml` and copy those
/// tables down. That cross-manifest read is out-of-scope for
/// v0.1.0-beta.6 (the R2 fix).
///
/// All four Round-1 pilots (cxx, serde-json, anyhow, thiserror) invoke
/// lihaaf from the upstream ROOT (workspace-root shape: `[package]` +
/// `[workspace]` in the same file). NONE invoke from a workspace-member
/// sub-crate. So this rejection does not affect any currently-enrolled
/// pilot. The follow-up to enable workspace-member overlays will land
/// separately.
///
/// **What this test pins:** when `[package].workspace = "../"` is
/// present in the upstream manifest, `materialize_overlay` returns an
/// `Error::Cli` with a directed diagnostic — NOT a silently-stripped
/// pointer (R1's behavior) or a silently-overlayed manifest.
///
/// **R3 tightening (PR #37, strict-swe Finding 1):** the rejection
/// MUST surface as `Error::Cli { clap_exit_code: 2, message }`, not
/// as a different `Error` variant that happens to have a Debug repr
/// containing "workspace member". The earlier
/// `format!("{err:?}") + .contains(...)` shape was loose
/// family-completeness with the adjacent
/// `workspace_root_manifest_is_rejected_with_directed_diagnostic`
/// test pattern; a future refactor could replace `Error::Cli`
/// with (say) `Error::TomlParse` and the loose test would still
/// pass.
#[test]
fn staged_overlay_rejects_workspace_member_manifest() {
    let tmp = tempfile::tempdir().expect("tempdir for workspace-member rejection test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Workspace-member shape: `[package]` + `[package].workspace = "../"`
    // (pointing at an ancestor workspace root that lives in `../`).
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "member"
version = "0.1.0"
workspace = "../"

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

    let result = materialize_overlay(&upstream_manifest);
    let err = result.expect_err(
        "workspace-member manifest (`[package].workspace = \"...\"`) MUST be rejected \
         under R2 — copying the ancestor's inheritance tables down into the overlay \
         is out-of-scope for v0.1.0-beta.6",
    );

    match err {
        lihaaf::Error::Cli {
            clap_exit_code,
            message,
        } => {
            assert_eq!(
                clap_exit_code, 2,
                "exit code must be the clap usage code (2)"
            );
            assert!(
                message.contains("workspace member"),
                "rejection diagnostic must name the failure category; got: {message}"
            );
            assert!(
                message.contains("[package].workspace"),
                "rejection diagnostic must name the offending key; got: {message}"
            );
            // Distinguish from the implicit-member rejection: the
            // explicit case must NOT use the word "implicit".
            assert!(
                !message.contains("implicit"),
                "explicit rejection must not use the implicit-case wording; got: {message}"
            );
        }
        other => panic!("expected Error::Cli for workspace-member rejection, got {other:?}"),
    }
}

/// **R3 invariant: IMPLICIT workspace-member case is REJECTED.**
///
/// In real cargo workspaces, members commonly OMIT
/// `[package].workspace` from their own manifests. Cargo discovers
/// membership by walking UP the filesystem from the member's
/// `Cargo.toml`, finding the nearest ancestor manifest containing
/// `[workspace]`, and reading that ancestor's `members = [...]`
/// array. Example: `cxx`'s root `Cargo.toml` carries
/// `[workspace] members = ["demo", "macro", "gen/build", ...]` and
/// each sub-crate (e.g. `macro/Cargo.toml`) has NO
/// `[package].workspace` line — its membership is implicit via the
/// parent's `members`.
///
/// **What this test pins:** when the upstream manifest has NO local
/// `[workspace]` table but DOES carry any `{ workspace = true }`
/// inheritance reference (here, a single `[dependencies] foo =
/// { workspace = true }`), `materialize_overlay` rejects with an
/// `Error::Cli { clap_exit_code: 2, ... }` whose message names the
/// implicit-member category, the missing local `[workspace]` table,
/// and the offending inheritance shape. Without R3 this case
/// produced a manifest with stranded `{ workspace = true }`
/// references that cargo rejected with the cryptic "workspace
/// inheritance was specified but `[workspace.X]` was not defined"
/// parse error — opaque to users.
///
/// This case doesn't currently exercise in production for the four
/// Round-1 pilots (all are invoked from upstream ROOT per
/// `compat/templates/pilot-stage2.yml`), but it is required for
/// completeness — any future user invoking lihaaf from a workspace
/// sub-crate hits the cryptic cargo error instead of a clean
/// rejection.
#[test]
fn staged_overlay_rejects_implicit_workspace_member_manifest() {
    let tmp = tempfile::tempdir().expect("tempdir for implicit workspace-member rejection test");
    let upstream_dir = tmp.path();
    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Implicit workspace-member shape:
    //  - `[package]` present (it IS a buildable crate)
    //  - NO `[package].workspace` line (membership is implicit)
    //  - NO local `[workspace]` table (the ancestor is the
    //    workspace root)
    //  - ONE `{ workspace = true }` inheritance reference — enough
    //    to trigger the rejection.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "implicit-member"
version = "0.1.0"

[dependencies]
foo = { workspace = true }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    let result = materialize_overlay(&upstream_manifest);
    let err = result.expect_err(
        "implicit workspace-member manifest (no local `[workspace]` but `{ workspace = true }` \
         reference present) MUST be rejected under R3 — injecting `[workspace] = {}` here \
         would strand the inheritance reference at cargo parse time with `\"workspace \
         inheritance was specified but [workspace.X] was not defined\"`",
    );

    match err {
        lihaaf::Error::Cli {
            clap_exit_code,
            message,
        } => {
            assert_eq!(
                clap_exit_code, 2,
                "exit code must match the explicit-rejection contract (clap usage code 2)"
            );
            assert!(
                message.contains("implicit workspace member"),
                "diagnostic must name the implicit-member category; got: {message}"
            );
            assert!(
                message.contains("no local `[workspace]`"),
                "diagnostic must name the structural signal that triggered the rejection; got: {message}"
            );
            assert!(
                message.contains("workspace = true"),
                "diagnostic must point at the inheritance-reference shape; got: {message}"
            );
            assert!(
                message.contains("workspace-ROOT"),
                "diagnostic must direct the user at the workspace root; got: {message}"
            );
        }
        other => {
            panic!("expected Error::Cli for implicit workspace-member rejection, got {other:?}")
        }
    }
}

/// **R4 invariant: ancestor-workspace implicit-member case is REJECTED.**
///
/// The Codex R3 review (PR #37) surfaced a high-severity correctness
/// gap: a manifest can be an IMPLICIT workspace member even without
/// any `{ workspace = true }` inheritance references. Cargo's
/// dependency resolution walks up the filesystem from the manifest
/// and applies state from the first ancestor `Cargo.toml` carrying
/// `[workspace]` — including `[patch.crates-io]`, `[replace]`,
/// `[profile]`, `resolver`, and `[workspace.dependencies]`. The
/// lihaaf overlay declares its own `[workspace]` and terminates
/// cargo's walk-up at the staged manifest, skipping the ancestor
/// entirely. Result: baseline cargo (which sees the ancestor) and
/// the lihaaf overlay (which does not) build against different
/// dependency graphs — producing false-positive or false-negative
/// compat verdicts that mislead users.
///
/// **What this test pins:** when the upstream Cargo.toml has neither
/// a local `[workspace]` table nor any `{ workspace = true }`
/// references, but its parent directory's `Cargo.toml` carries
/// `[workspace] members = ["<dir>"]` plus `[patch.crates-io]`,
/// `materialize_overlay` rejects with `Error::Cli { clap_exit_code:
/// 2, ... }` whose message names the implicit-member-via-ancestor
/// category AND the ancestor manifest path. Without R4 the overlay
/// would silently produce a manifest with a divergent resolved
/// dependency graph — the worst possible failure mode (no error
/// surfaced, just a wrong compat verdict).
///
/// This integration test mirrors the unit test
/// `override_workspace_rejects_manifest_with_ancestor_workspace` at
/// the full pipeline level, so a regression in the R4 logic is
/// caught at both layers.
#[test]
fn staged_overlay_rejects_manifest_with_ancestor_workspace() {
    let tmp = tempfile::tempdir().expect("tempdir for ancestor-workspace rejection test");

    // Parent: workspace root with [patch.crates-io]. This mirrors
    // the Codex repro pattern exactly: an ancestor workspace whose
    // [patch] / [replace] / [profile] state would affect baseline
    // cargo's resolution.
    let parent_manifest = tmp.path().join("Cargo.toml");
    std::fs::write(
        &parent_manifest,
        r#"[workspace]
members = ["sub"]

[patch.crates-io]
serde = { path = "../my-serde-fork" }
"#,
    )
    .expect("writing parent Cargo.toml");

    // Sub-crate: no local [workspace], no inheritance refs. The
    // implicit-member-via-ancestor shape — R4's exact target.
    let sub_dir = tmp.path().join("sub");
    std::fs::create_dir_all(&sub_dir).expect("creating sub/");
    let sub_manifest = sub_dir.join("Cargo.toml");
    std::fs::write(
        &sub_manifest,
        r#"[package]
name = "sub"
version = "0.1.0"

[dependencies]
serde = "1"
"#,
    )
    .expect("writing sub/Cargo.toml");
    std::fs::create_dir_all(sub_dir.join("src")).expect("creating sub/src/");
    std::fs::write(sub_dir.join("src").join("lib.rs"), "pub fn _stub() {}\n")
        .expect("writing sub/src/lib.rs");

    let result = materialize_overlay(&sub_manifest);
    let err = result.expect_err(
        "manifest with ancestor `[workspace]` MUST be rejected under R4 — \
         injecting `[workspace] = {}` would silently skip the ancestor's \
         `[patch]` / `[replace]` / `[profile]` state during cargo resolution, \
         producing a divergent dependency graph between baseline and overlay \
         and therefore false compat verdicts (the worst failure mode)",
    );

    match err {
        lihaaf::Error::Cli {
            clap_exit_code,
            message,
        } => {
            assert_eq!(
                clap_exit_code, 2,
                "exit code must match the rejection contract (clap usage code 2)"
            );
            assert!(
                message.contains("implicit workspace member"),
                "diagnostic must name the implicit-member category; got: {message}"
            );
            assert!(
                message.contains("ancestor manifest"),
                "diagnostic must name the ancestor-detection signal; got: {message}"
            );
            // The diagnostic must include the offending ancestor
            // manifest path so the user can locate the source of the
            // rejection without spelunking.
            let parent_str = parent_manifest.display().to_string();
            assert!(
                message.contains(&parent_str),
                "diagnostic must include the ancestor manifest path `{parent_str}`; got: {message}"
            );
            // Distinguish from the R3 inheritance-refs rejection:
            // this case has no `{ workspace = true }` references, and
            // surfacing that wording would mislead users about which
            // signal triggered the rejection.
            assert!(
                !message.contains("workspace = true"),
                "ancestor-workspace rejection must not mention inheritance refs (this case has none); got: {message}"
            );
        }
        other => {
            panic!("expected Error::Cli for ancestor-workspace rejection, got {other:?}")
        }
    }
}

/// **R3 invariant: workspace-root case (local `[workspace]` + own
/// inheritance refs) is NOT rejected.**
///
/// A manifest with BOTH a local `[workspace]` table AND
/// `{ workspace = true }` references is the standard workspace-root
/// shape: the root crate hosts `[workspace.dependencies]` /
/// `[workspace.package]` / `[workspace.lints]` and also has its OWN
/// `[package]` whose inheritance refs resolve against those local
/// tables. The R3 implicit-member check must NOT fire — the
/// inheritance refs resolve LOCALLY, within the same manifest, which
/// the overlay preserves verbatim.
///
/// **What this test pins:** an upstream Cargo.toml carrying both
/// `[package].version = { workspace = true }` AND a local
/// `[workspace.package] version = "..."` produces a staged overlay
/// WITHOUT error and preserves both the inheritance reference and
/// the local `[workspace.package]` table.
#[test]
fn staged_overlay_allows_root_with_local_workspace_and_inheritance_refs() {
    let input = r#"[package]
name = "root-with-inheritance"
version.workspace = true

[lib]
crate-type = ["dylib", "rlib"]

[workspace]
members = ["nested"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
shared = "1.0"
"#;
    let (_tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream)
        .expect("workspace-root case with own inheritance refs must NOT be rejected");

    let bytes = read_overlay(&plan.sibling_manifest);

    // The `version.workspace = true` reference must survive verbatim.
    assert!(
        bytes.contains("workspace = true"),
        "the local-inheritance reference must pass through unchanged; got:\n{bytes}"
    );
    // The `[workspace.package]` table must survive (preserves R2
    // inheritance-table contract).
    assert!(
        bytes.contains("[workspace.package]"),
        "the local `[workspace.package]` table must pass through; got:\n{bytes}"
    );
    // The `[workspace.dependencies]` table must survive.
    assert!(
        bytes.contains("[workspace.dependencies]"),
        "the local `[workspace.dependencies]` table must pass through; got:\n{bytes}"
    );
    // The membership key must be stripped (R2 selective-rewrite
    // contract).
    assert!(
        !bytes.contains("members = ["),
        "the `members` membership key must be stripped; got:\n{bytes}"
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

/// **Workspace-style cargo-build regression for issue #36.**
///
/// Constructs a real workspace-style synthetic upstream that mirrors the
/// thiserror pilot's shape: the root `Cargo.toml` declares BOTH
/// `[package]` (a buildable top-level crate, the dylib_crate) AND
/// `[workspace] members = ["impl"]` (claiming a member sub-crate). The
/// root package has a path-dep `path = "impl"` pointing at the member.
///
/// This is the exact failure shape v0.1.0-beta.5 produced on cxx +
/// thiserror in the post-publish refresh-pilots run #26000403851:
/// `package <X>/impl/Cargo.toml is a member of the wrong workspace`.
/// The workspace-identity fix (`override_workspace_inheritance`) makes
/// the overlay declare itself as its own workspace root with no
/// members, so cargo's walk-up terminates at the overlay and never
/// tries to attach it to the upstream workspace.
///
/// **What this test bites.** A regression that removes or weakens the
/// `override_workspace_inheritance` step would let the overlay either
/// (a) inherit upstream's `[workspace]` membership (cargo walks up,
/// finds upstream's `[workspace]`, errors on the overlay not being a
/// member), or (b) preserve the upstream's `[workspace] members =
/// [...]` declaration in the overlay (both workspaces then claim impl,
/// error on conflict).
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` for the same OOM
/// reason as the other `cargo_accepts_*` tests — cargo rustc spawns a
/// real subprocess, which costs ~5–10s of wall-clock and a few hundred
/// MB of disk. CI sets the env var; local-only test runs skip.
#[test]
fn cargo_accepts_workspace_style_overlay_for_dylib_build() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_workspace_style_overlay_for_dylib_build: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for workspace-style cargo build test");
    let upstream_dir = tmp.path();

    // Guarantee absolute path — mirrors what CompatArgs::from_cli ensures.
    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli already guarantees \
         compat_root is absolute before overlay)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // 1. Member sub-crate at `<upstream>/impl/` (mirrors thiserror's
    //    `thiserror-impl` and cxx's `cxx-build` arrangement). Both the
    //    root upstream AND this member have their own `Cargo.toml` and
    //    `src/lib.rs`. The root upstream's `[workspace] members = ["impl"]`
    //    claims this member.
    let impl_dir = upstream_dir.join("impl");
    std::fs::create_dir_all(impl_dir.join("src")).expect("creating impl/src/");
    std::fs::write(
        impl_dir.join("Cargo.toml"),
        r#"[package]
name = "ws-demo-impl"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("writing impl/Cargo.toml");
    std::fs::write(
        impl_dir.join("src").join("lib.rs"),
        "// impl stub for workspace-style pilot\npub fn helper() {}\n",
    )
    .expect("writing impl/src/lib.rs");

    // 2. Root upstream manifest: `[package]` (the dylib_crate) +
    //    `[workspace]` (claiming impl as a member) + path-dep on impl.
    //    This is the EXACT shape thiserror's `Cargo.toml` has on master.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "ws-demo"
version = "0.1.0"
edition = "2021"

[workspace]
members = ["impl"]

[dependencies]
ws-demo-impl = { path = "impl" }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "// ws-demo stub\npub fn api() -> u32 { 42 }\n",
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

    // Pre-cargo sanity: the overlay must carry a `[workspace]` table
    // (so cargo stops the walk-up at the overlay) and the upstream's
    // `members` array (in any form — relative or absolutized) must NOT
    // survive. (The dedicated
    // `staged_overlay_overrides_upstream_workspace_inheritance` test
    // pins this contract at the structural level; this assertion is
    // defense-in-depth at the cargo-test layer, catching the case where
    // the override stops running but the absolutization still does.)
    //
    // Under R2 (PR #37 fixup), `[workspace]` may carry inheritance
    // tables (`dependencies`, `package`, `lints`, `metadata`,
    // `resolver`). This test's input has no inheritance tables so
    // the overlay's `[workspace]` will be empty in practice — but the
    // assertion below only checks that `members` is stripped, which
    // is the load-bearing R2 invariant.
    let content = read_overlay(&plan.sibling_manifest);
    assert!(
        content.contains("[workspace]"),
        "staged overlay must declare `[workspace]` (so cargo treats it as its \
         own workspace root); got overlay:\n{content}"
    );
    assert!(
        !content.contains("\nmembers ="),
        "staged overlay must NOT preserve any form of the upstream's \
         `[workspace] members` array (the override strips it in both relative \
         and absolutized form); got overlay:\n{content}"
    );

    // The acid test: cargo rustc against the staged overlay. Without the
    // workspace-identity fix, this produces `package <X>/impl/Cargo.toml
    // is a member of the wrong workspace` and exits 65.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("ws-demo")
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
        "cargo rustc must succeed against the workspace-style staged overlay; \
         got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// **R2 regression: `{ workspace = true }` inheritance reference resolves
/// against the preserved `[workspace.dependencies]` in the staged overlay.**
///
/// This is the exact production failure shape Codex's BLOCK identified
/// on PR #37 R1 (commit 1f6520b). R1 replaced `[workspace]` with `{}`,
/// which broke any overlay manifest that used cargo's workspace
/// inheritance feature. The Codex repro:
///
/// ```toml
/// [package]
/// name = "ws-demo"
/// version = "0.1.0"
///
/// [workspace]
/// members = ["impl"]
///
/// [workspace.dependencies]
/// ws-demo-impl = { path = "impl" }
///
/// [dependencies]
/// ws-demo-impl = { workspace = true }   # ← inherits from above
/// ```
///
/// Under R1: `[workspace.dependencies]` was clobbered to `{}`, so the
/// surviving `{ workspace = true }` reference produced cargo's
/// "workspace inheritance was specified but `[workspace.dependencies]`
/// was not defined" error.
///
/// Under R2 (this commit): `[workspace.dependencies]` is preserved
/// (only `members` / `exclude` / `default-members` are stripped), so
/// the inheritance reference resolves correctly.
///
/// **Why this test is gated.** Runs `cargo rustc` as a subprocess —
/// same OOM concern as the other `cargo_accepts_*` tests. CI sets
/// `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; local runs skip by default.
///
/// **Why this test is load-bearing for the R2 fixup.** Mentally
/// disabling the R2 selective rewrite (reverting to R1's full clobber)
/// would cause `[workspace.dependencies]` to be `{}` in the overlay,
/// and the `{ workspace = true }` reference would fail cargo's parser.
/// This test catches that regression at the cargo-test layer; the
/// structural unit test
/// (`override_workspace_preserves_inheritance_tables`) catches it
/// faster but doesn't prove cargo accepts the result.
#[test]
fn cargo_accepts_workspace_inheritance_reference_in_overlay() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_workspace_inheritance_reference_in_overlay: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp =
        tempfile::tempdir().expect("creating tempdir for workspace-inheritance cargo build test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Member sub-crate at `<upstream>/impl/`.
    let impl_dir = upstream_dir.join("impl");
    std::fs::create_dir_all(impl_dir.join("src")).expect("creating impl/src/");
    std::fs::write(
        impl_dir.join("Cargo.toml"),
        r#"[package]
name = "ws-demo-impl"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("writing impl/Cargo.toml");
    std::fs::write(
        impl_dir.join("src").join("lib.rs"),
        "pub fn helper() -> u32 { 1 }\n",
    )
    .expect("writing impl/src/lib.rs");

    // Root upstream: workspace ROOT carrying both `[package]` and
    // `[workspace.dependencies]`, plus a `{ workspace = true }`
    // reference in `[dependencies]`. The Codex repro shape exactly.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "ws-demo"
version = "0.1.0"
edition = "2021"

[workspace]
members = ["impl"]

[workspace.dependencies]
ws-demo-impl = { path = "impl" }

[dependencies]
ws-demo-impl = { workspace = true }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn api() -> u32 { ws_demo_impl::helper() }\n",
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Pre-cargo sanity: parsed overlay must carry
    // `[workspace.dependencies.ws-demo-impl]` (with absolutized path).
    // This is the R2 invariant the cargo build below tests end-to-end.
    let content = read_overlay(&plan.sibling_manifest);
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must be valid TOML");
    let ws_deps = parsed
        .get("workspace")
        .and_then(|v| v.as_table())
        .and_then(|ws| ws.get("dependencies"))
        .and_then(|v| v.as_table())
        .expect(
            "R2 invariant: `[workspace.dependencies]` MUST survive in the overlay so \
             `{ workspace = true }` references resolve",
        );
    assert!(
        ws_deps.contains_key("ws-demo-impl"),
        "[workspace.dependencies.ws-demo-impl] MUST survive (Codex repro shape); \
         got entries: {:?}",
        ws_deps.keys().collect::<Vec<_>>()
    );

    // The acid test: cargo rustc against the staged overlay. Under R1
    // (the BLOCKed attempt), this produces: "workspace inheritance was
    // specified but `[workspace.dependencies]` was not defined". Under
    // R2 (this commit), it succeeds.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("ws-demo")
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
        "cargo rustc must accept the workspace-inheritance overlay (Codex repro); \
         got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// **`[replace]` path entries are absolutized in the overlay.**
///
/// `[replace]` is cargo's older, soft-deprecated replacement form.  Like
/// `[patch]`, its relative `path` entries would point into the staged
/// manifest dir after overlay materialization — the same failure mode
/// Round-2 FIX class C fixed for `[patch]`. This test pins FIX class IV
/// (Round-3): a future regression removing `absolutize_replace_paths` would
/// produce a relative path in the overlay and fail this assertion.
#[test]
fn replace_paths_are_absolutized() {
    let input = r#"[package]
name = "demo"
version = "0.1.0"

[replace]
"old-dep:0.2.0" = { path = "vendor/old-dep" }
"serde:1.0.0" = { git = "https://example.com/serde", rev = "abc123" }
"#;
    let (tmp, upstream) = write_upstream(input);
    let plan = materialize_overlay(&upstream).expect("overlay must succeed");
    let content = read_overlay(&plan.sibling_manifest);

    let upstream_dir_str = tmp.path().to_string_lossy().replace('\\', "/");

    // path-form entry must be absolutized.
    let expected_path = format!("{upstream_dir_str}/vendor/old-dep");
    assert!(
        content.contains(&format!(r#"path = "{expected_path}""#)),
        "[replace.\"old-dep:0.2.0\"].path must be absolutized; overlay:\n{content}"
    );

    // git-form entry must NOT have a path key added.
    // The relative `path = "vendor/old-dep"` literal must be gone.
    assert!(
        !content.contains(r#"path = "vendor/old-dep""#),
        "relative [replace] path must not survive in overlay; overlay:\n{content}"
    );

    // git/rev fields must pass through verbatim.
    assert!(
        content.contains(r#"git = "https://example.com/serde""#),
        "git-form [replace] git field must be unchanged; overlay:\n{content}"
    );
    assert!(
        content.contains(r#"rev = "abc123""#),
        "git-form [replace] rev field must be unchanged; overlay:\n{content}"
    );
}
