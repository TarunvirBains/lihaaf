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

use lihaaf::CompatWorkspaceMemberContext as WorkspaceMemberContext;
use lihaaf::compat_overlay_materialize as materialize_overlay;
use lihaaf::compat_overlay_materialize_with_metadata_and_workspace_member_context as materialize_overlay_with_metadata_and_ctx;

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
    // upfront with a directed diagnostic.
    //
    // Issue #53 augments the diagnostic to recommend `--package
    // <pkg>` as the actionable workspace-member entry shape, so the
    // assertions below check for the augmented text shape ("workspace
    // member" + "--package").
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
            // Issue #53 — diagnostic now recommends `--package` so
            // the adopter has an actionable workspace-member entry
            // shape. The legacy "member crate" wording is replaced
            // with "workspace member" + "--package".
            assert!(
                message.contains("--package"),
                "diagnostic must recommend `--package` per #53; got: {message}"
            );
            assert!(
                message.contains("workspace member") || message.contains("Cargo.toml"),
                "diagnostic must direct the adopter; got: {message}"
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
            // Issue #53 — diagnostic now recommends `--package`
            // (see workspace_root_manifest_is_rejected_with_directed_diagnostic
            // above for the rationale).
            assert!(
                message.contains("--package"),
                "diagnostic must recommend `--package` per #53; got: {message}"
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
        // Issue #53 — workspace-member entry via `--package`. The
        // fixture exercises the workspace-root carry-down +
        // `apply_workspace_member_inheritance` + Option H Rule 1
        // INJECT on a member-shaped input (`{ workspace = true }`
        // inheritance refs, no `[workspace]` of its own). The
        // corpus-iteration loop runs a special path for this name
        // (synthesizes a workspace root + ctx alongside the member
        // input) — see the per-fixture conditional below.
        "workspace_member_with_package",
    ];
    let mut checked = 0usize;
    for name in &names {
        let input_path = corpus_dir.join(format!("{name}.input.toml"));
        let expected_path = corpus_dir.join(format!("{name}.expected.toml"));
        let input = std::fs::read_to_string(&input_path)
            .unwrap_or_else(|e| panic!("reading corpus input {input_path:?}: {e}"));
        let expected_template = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("reading corpus expected {expected_path:?}: {e}"));

        // Issue #53 — the workspace-member fixture requires a
        // synthesized workspace-root context. The `--package`
        // resolver's natural shape is `<ws-root>/Cargo.toml` carrying
        // `[workspace.*]` carry-down tables + `<ws-root>/<member>/Cargo.toml`
        // as the input. We materialize with
        // `materialize_overlay_with_metadata_and_workspace_member_context`,
        // passing a populated `WorkspaceMemberContext` so the
        // carry-down + Option H apply. The tempdir owner is bound
        // OUTSIDE the if-branch so it lives until the assertion below
        // (mem::forget would leak; an early drop would invalidate
        // file references inside the overlay during assert_eq).
        let mut _wm_tmp: Option<tempfile::TempDir> = None;
        let (actual, upstream_dir) = if *name == "workspace_member_with_package" {
            let tmp = tempfile::tempdir().expect("tempdir for workspace_member fixture");
            std::fs::create_dir(tmp.path().join("mem")).expect("create member subdir");
            let ws_root_text = r#"[workspace]
members = ["mem"]

[workspace.package]
edition = "2021"

[workspace.dependencies]
serde = "1.0"

[workspace.lints.rust]
unsafe_code = "forbid"
"#;
            let ws_root_manifest = tmp.path().join("Cargo.toml");
            std::fs::write(&ws_root_manifest, ws_root_text).expect("write workspace-root manifest");
            let member_manifest = tmp.path().join("mem").join("Cargo.toml");
            std::fs::write(&member_manifest, &input).expect("write member manifest");
            let ws_root_value: toml::Value =
                toml::from_str(ws_root_text).expect("workspace-root TOML parses");
            let ctx = WorkspaceMemberContext {
                workspace_root_manifest: ws_root_manifest.clone(),
                workspace_root_value: ws_root_value,
            };
            let plan =
                materialize_overlay_with_metadata_and_ctx(&member_manifest, None, Some(&ctx))
                    .expect("workspace-member overlay must succeed");
            let actual = read_overlay(&plan.sibling_manifest);
            // The overlay's `[lib].path` and Rule-1-INJECTed
            // self-patch path are anchored against the MEMBER root
            // (per §3.1.bis routing table — overlay materialization
            // anchors on `member_root`), so the `__UPSTREAM_DIR__`
            // placeholder substitutes for the member dir.
            let member_dir = member_manifest
                .parent()
                .expect("member manifest has parent")
                .to_string_lossy()
                .replace('\\', "/");
            _wm_tmp = Some(tmp);
            (actual, member_dir)
        } else {
            let (_tmp, upstream) = write_upstream(&input);
            let plan = materialize_overlay(&upstream).expect("overlay must succeed");
            let actual = read_overlay(&plan.sibling_manifest);
            let upstream_dir = upstream
                .parent()
                .expect("upstream manifest has a parent dir")
                .to_string_lossy()
                .replace('\\', "/");
            // For the non-workspace-member fixtures, `_tmp` was bound
            // inside `write_upstream` and went out of scope already
            // (the original test code accepts that — the overlay
            // bytes were captured above by `read_overlay`).
            (actual, upstream_dir)
        };

        // Substitute the `__UPSTREAM_DIR__` placeholder in the expected
        // template with the real tempdir (forward-slash form, matching
        // the overlay code's `to_forward_slash` call). The placeholder
        // is a fixed-string substitution — no regex, per spec §6.1.
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
        checked, 9,
        "corpus must include all 9 representative fixtures \
         (6 baseline + 2 issue #40/#47 Option H + 1 issue #53 \
         workspace-member-with-package)"
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

    // cxx-demo path = "." → absolute staged overlay dir (Rule 2 REMAP, Option H §4.2).
    // The self-patch entry is remapped to the staged dir so cargo resolves the
    // overlay manifest, not the upstream root directly.
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
    // The patch entry references the same crate (self-patch); per Option H
    // Rule 2 REMAP it is absolutized to the staged overlay dir, not the
    // upstream root, so cargo resolves the overlay manifest directly.
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

// =====================================================================
// Issue #40 + #47 §5.2 cargo-build-gated integration tests.
//
// These tests prove the Option H 4-rule self-patch policy and the
// staged-mirror writer at the cargo-build integration layer, end to
// end. Each gated test spawns a real `cargo rustc` (or `cargo build`)
// against the materialized overlay and asserts cargo accepts the
// resulting topology / package-root access pattern.
//
// **Gating contract.** All §5.2.5–§5.2.7 / §5.2.9–§5.2.11 tests gate
// on `LIHAAF_RUN_CARGO_BUILD_TESTS=1` per [[lihaaf-no-local-binary-builds]]
// — cargo spawns subprocess + downloads + on-disk artifacts that OOM
// RAM-constrained WSL2 / dev hosts. CI exports the env var so the
// authoritative verification happens there; local-only test runs
// silently skip. Section §5.2.8 is intentionally UNGATED — it is a
// negative test that exercises the materializer's REJECT branch and
// makes NO cargo invocation, so the OOM gate does not apply (a plan
// surface deviation: §5.2.8 was labelled "Cargo-build-gated test" in
// the §5.2 list header, but its mechanics make no cargo call; the
// gate would silently skip the integration-layer Rule 4 REJECT
// coverage on local runs, defeating the discipline contract).
// §5.2.12 (the absolute-path pin) is also UNGATED — it is a
// unit-style integration assertion that runs `materialize_overlay`
// and inspects the emitted byte shape, no cargo spawn.
//
// Plan reference: `docs/plans/issue-40-47-overlay-vs-registry.md`
// §5.2 (R4 Option H 4-rule mapping + R5/R6 staged-mirror closure).
// =====================================================================

/// §5.2.5 — Rule 1 INJECT cargo-graph proof (anyhow-shape).
///
/// Constructs the minimal anyhow-shape repro: a clean single-crate
/// upstream with no pre-existing `[patch.crates-io.<self>]` table.
/// Rule 1 INJECT fires; the overlay carries
/// `[patch.crates-io.anyhow-like] = { path = "<staged-overlay-dir>" }`.
/// Cargo's resolver accepts the injected patch as benign because no
/// workspace member references the crate by registry name — the patch
/// is a no-op for the standalone case (plan §6.4).
///
/// **What this test bites.** A regression that fails to absolutize
/// the injected `path` value, or emits a relative `.` form, would
/// break cargo's `[patch.crates-io.<X>].path` resolution (cargo
/// anchors the path relative to the staged manifest dir; a `.` path
/// would resolve to the staged overlay dir itself, accidentally
/// correct today but coincidence-dependent per plan §6.5). An
/// absolute path is unambiguous.
///
/// **Pre-fix:** without Option H Rule 1, no patch is injected → the
/// test would PASS trivially (no patch table, no resolution to
/// verify). Post-fix: the patch is injected AND cargo accepts the
/// resulting graph.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` per
/// [[lihaaf-no-local-binary-builds]]; plan §5.2.5.
#[test]
fn cargo_accepts_inject_when_clean_upstream_anyhow_shape() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_inject_when_clean_upstream_anyhow_shape: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for Rule 1 anyhow-shape cargo test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "anyhow-like"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
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

    // Pre-cargo sanity: Rule 1 INJECT must have added the self-patch
    // entry pointing at the staged-overlay dir (absolute, forward
    // slash).
    let content = read_overlay(&plan.sibling_manifest);
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must be valid TOML");
    let inject_path = parsed
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("anyhow-like"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("Rule 1 INJECT must add [patch.crates-io.anyhow-like].path");
    assert!(
        Path::new(inject_path).is_absolute(),
        "Rule 1 emission must be absolute; got `{inject_path}`"
    );
    assert!(
        inject_path.ends_with("/target/lihaaf-overlay"),
        "Rule 1 emission must target the staged-overlay-dir; got `{inject_path}`"
    );

    // Acid test: cargo rustc against the staged overlay. The patch
    // is benign for the standalone anyhow-shape (no workspace member
    // references `anyhow-like` by registry name), so cargo's
    // resolver collapses the unused patch and proceeds to compile
    // the root crate.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("anyhow-like")
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
        "cargo rustc must succeed against the Rule 1 INJECT overlay; \
         got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.6 — Rule 2 REMAP cargo-graph proof + staged-mirror (cxx-shape).
///
/// Constructs the cxx-shape repro: upstream carries
/// `[patch.crates-io.foo] = { path = "." }` (resolves to upstream
/// root). The Option H Rule 2 detection fires (joined path
/// lexical-normalizes to upstream root) and REMAPs the entry to
/// `<staged-overlay-dir>`. The shape also has a workspace member
/// `test-suite` with a registry-name dep `foo = "1.0"`, plus
/// `links = "foo-native"` on the root — without the REMAP, cargo
/// would report `package <foo> links to <foo-native> ... conflicts
/// with a previous package which links to <foo-native> as well`
/// because the resolved graph would contain two source-ids for
/// `foo` (overlay's `[package]` + upstream's `path = "."`).
///
/// The shape ALSO includes a real file-reading `build.rs` that opens
/// `src/cxx_stub.cc` via `CARGO_MANIFEST_DIR`. Pre-mirror, this file
/// is not accessible from the staged overlay dir; post-mirror, the
/// staged-overlay's `src/` is a symlink (or copy) to upstream's
/// `src/`, so the build script can read it. The test validates BOTH
/// the policy correctness AND the mirror correctness in a single
/// fixture.
///
/// **What this test bites.** Two simultaneous regressions:
///   - Rule 2 mis-detection (e.g. comparing un-normalized paths) →
///     Rule 4 REJECT fires by mistake, materialize returns Err.
///   - Mirror writer fails to populate `<staged>/src/` → build.rs
///     `read_to_string` fails with `No such file or directory`.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; plan §5.2.6.
#[test]
fn cargo_accepts_remap_when_upstream_self_patch_cxx_shape() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_remap_when_upstream_self_patch_cxx_shape: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for Rule 2 cxx-shape cargo test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");

    // Upstream package-root layout faithfully mirrors the cxx upstream
    // Cargo.toml (github.com/dtolnay/cxx, verified Codex rollout 019e3cc3):
    //   - root manifest with `[package]` + `links` + `build.rs`
    //   - `[workspace] members = ["test-suite"]` — workspace declaration ONLY;
    //     cxx does NOT carry a root `[dependencies]` edge to the member
    //   - workspace member `test-suite` with `foo = "1.0"` registry dep
    //   - `[patch.crates-io.foo] = { path = "." }` (the cxx self-patch)
    //   - build.rs reads `src/cxx_stub.cc` via CARGO_MANIFEST_DIR
    //   - include/stub.h is referenced via `Path::exists()`
    //
    // The absence of `[dependencies] test-suite = { path = "test-suite" }` in
    // the root is load-bearing: adding it creates a `foo → test-suite → foo`
    // active-dep cycle that cargo rejects even when both `foo` references
    // resolve to the same source-id (Codex diagnosis, rollout 019e3cc3).
    // The real cxx avoids this because it never declares test-suite as a
    // root build dependency.
    //
    // cargo rustc below runs `-p foo` against the staged overlay; the member
    // is in the workspace but is NOT a build dep of the root, mirroring real
    // cxx's build topology.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "foo"
version = "1.0.0"
edition = "2021"
links = "foo-native"
build = "build.rs"

[lib]
path = "src/lib.rs"

[workspace]
members = ["test-suite"]

[patch.crates-io]
foo = { path = "." }
"#,
    )
    .expect("writing upstream Cargo.toml");

    // build.rs that exercises CARGO_MANIFEST_DIR-relative file reads
    // (mirrors cxx build.rs:143-148 + :154-159 read pattern).
    std::fs::write(
        upstream_dir.join("build.rs"),
        r#"fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_path = std::path::Path::new(&manifest_dir).join("src").join("cxx_stub.cc");
    let _content = std::fs::read_to_string(&src_path)
        .expect("build.rs: failed to read src/cxx_stub.cc via CARGO_MANIFEST_DIR");
    let include_path = std::path::Path::new(&manifest_dir).join("include").join("stub.h");
    assert!(include_path.exists(),
        "build.rs: include/stub.h not found via CARGO_MANIFEST_DIR: {:?}", include_path);
    println!("cargo:rerun-if-changed=src/cxx_stub.cc");
    println!("cargo:rerun-if-changed=include/stub.h");
}
"#,
    )
    .expect("writing build.rs");

    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("cxx_stub.cc"),
        "// stub C++ file for build-script test\n",
    )
    .expect("writing src/cxx_stub.cc");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    std::fs::create_dir_all(upstream_dir.join("include")).expect("creating include/");
    std::fs::write(
        upstream_dir.join("include").join("stub.h"),
        "// stub header for build-script test\n",
    )
    .expect("writing include/stub.h");

    // Workspace member crate referencing `foo` by registry name.
    let member_dir = upstream_dir.join("test-suite");
    std::fs::create_dir_all(member_dir.join("src")).expect("creating test-suite/src/");
    std::fs::write(
        member_dir.join("Cargo.toml"),
        r#"[package]
name = "test-suite"
version = "0.0.0"
edition = "2021"

[dependencies]
foo = "1.0"
"#,
    )
    .expect("writing test-suite/Cargo.toml");
    std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn _stub() {}\n")
        .expect("writing test-suite/src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Pre-cargo sanity: Rule 2 REMAP must have rewritten the patch
    // entry to the staged-overlay-dir (NOT to upstream root, which
    // would be the BLOCK-1 self-loop bug).
    let content = read_overlay(&plan.sibling_manifest);
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must be valid TOML");
    let remap_path = parsed
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("foo"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("Rule 2 REMAP must keep [patch.crates-io.foo].path");
    let staged_overlay_dir_str = plan
        .sibling_manifest
        .parent()
        .expect("staged manifest has a parent")
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(
        remap_path, staged_overlay_dir_str,
        "Rule 2 REMAP must rewrite [patch.crates-io.foo].path to the \
         staged-overlay-dir, NOT the upstream root (BLOCK-1 self-loop \
         avoidance pin); got `{remap_path}`"
    );

    // Mirror verification: <staged>/src/cxx_stub.cc and
    // <staged>/include/stub.h must be accessible from the staged
    // overlay dir (via symlink or copy). The build.rs read_to_string
    // call exercises this end-to-end below; this is the eager
    // sanity check.
    let staged_overlay_dir = plan
        .sibling_manifest
        .parent()
        .expect("staged manifest has a parent");
    let mirrored_cxx_stub = staged_overlay_dir.join("src").join("cxx_stub.cc");
    let mirrored_stub_h = staged_overlay_dir.join("include").join("stub.h");
    assert!(
        mirrored_cxx_stub.exists(),
        "mirror must populate <staged>/src/cxx_stub.cc; got missing at {:?}",
        mirrored_cxx_stub
    );
    assert!(
        mirrored_stub_h.exists(),
        "mirror must populate <staged>/include/stub.h; got missing at {:?}",
        mirrored_stub_h
    );

    // The acid test: cargo rustc against the staged overlay.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("foo")
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
        "cargo rustc must succeed against the Rule 2 REMAP cxx-shape overlay \
         (validates BOTH policy correctness + staged-mirror correctness); \
         got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.7 — Rule 3 CONTINUE-ABSOLUTIZE cargo-graph proof.
///
/// Upstream has BOTH a self-patch (`foo` → Rule 2 REMAP) AND a
/// non-self sibling patch (`foo-helper = { path = "helper" }` → Rule
/// 3 CONTINUE-ABSOLUTIZE no-op for `apply_self_patch_policy`; the
/// existing `absolutize_patch_paths` pass handles the entry as
/// before). This mirrors cxx's upstream which carries
/// `[patch.crates-io]` entries for both `cxx` (self-patch) and
/// `cxx-build` (sibling-crate patch with `path = "gen/build"`).
///
/// **What this test bites.** A regression that broadens Rule 2 (or
/// Rule 4) to match non-self keys would either (a) REMAP the
/// sibling-crate patch to the staged-overlay-dir (wrong source-id
/// → cargo resolution diverges) or (b) REJECT the sibling patch
/// (cargo never runs).
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; plan §5.2.7.
#[test]
fn cargo_accepts_continue_absolutize_when_non_root_patch() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_continue_absolutize_when_non_root_patch: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for Rule 3 cargo test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");

    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "foo"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[patch.crates-io]
foo = { path = "." }
foo-helper = { path = "helper" }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    // Sibling crate `foo-helper` at `<upstream>/helper/`.
    let helper_dir = upstream_dir.join("helper");
    std::fs::create_dir_all(helper_dir.join("src")).expect("creating helper/src/");
    std::fs::write(
        helper_dir.join("Cargo.toml"),
        r#"[package]
name = "foo-helper"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("writing helper/Cargo.toml");
    std::fs::write(helper_dir.join("src").join("lib.rs"), "pub fn _stub() {}\n")
        .expect("writing helper/src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Pre-cargo sanity: the `foo` self-patch is REMAPPED (Rule 2);
    // the `foo-helper` sibling patch survives untouched by Rule 3
    // and is absolutized to `<upstream>/helper` by the existing
    // `absolutize_patch_paths` pass.
    let content = read_overlay(&plan.sibling_manifest);
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must be valid TOML");
    let foo_path = parsed
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("foo"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("[patch.crates-io.foo].path must exist (Rule 2 REMAP)");
    let foo_helper_path = parsed
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("foo-helper"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("[patch.crates-io.foo-helper].path must exist (Rule 3 CONTINUE-ABSOLUTIZE)");

    let upstream_dir_str = upstream_dir.to_string_lossy().replace('\\', "/");
    assert!(
        foo_path.ends_with("/target/lihaaf-overlay"),
        "Rule 2 REMAP: [patch.crates-io.foo].path must target the \
         staged-overlay-dir; got `{foo_path}`"
    );
    assert_eq!(
        foo_helper_path,
        format!("{upstream_dir_str}/helper"),
        "Rule 3 CONTINUE-ABSOLUTIZE: [patch.crates-io.foo-helper].path \
         must be absolutized to `<upstream>/helper` (unchanged by \
         apply_self_patch_policy; absolutize_patch_paths handles it); \
         got `{foo_helper_path}`"
    );

    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("foo")
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
        "cargo rustc must succeed against the Rule 3 overlay; \
         got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.8 — Rule 4 REJECT integration test.
///
/// Negative test: upstream carries a vendored-fork-style self-patch
/// (`path = "../my-fork"`). The path resolves to a sibling dir, NOT
/// upstream root → Rule 4 REJECT fires. The materializer returns
/// `Error::CompatPatchOverrideConflict` referencing the v0.2/v1.1
/// escape hatch (`--compat-allow-patch-override`).
///
/// **Plan-mechanics deviation.** Plan §5.2.8 was labelled
/// "Cargo-build-gated test" in the §5.2 list header, but the test
/// itself makes NO cargo invocation — it stops at the materializer's
/// REJECT branch. Gating this test would silently skip the
/// integration-layer Rule 4 REJECT coverage on local dev runs,
/// defeating the discipline contract. The §5.1.7–§5.1.9 unit tests
/// cover the same Rule 4 cases at the in-crate layer; this test is
/// the cross-crate-boundary mirror that exercises the same path via
/// the `lihaaf::Error::CompatPatchOverrideConflict` re-export.
///
/// **What this test bites.** A regression that swaps Rule 4's
/// `Err(_)` for `Ok(_)` (e.g. silently overwriting the upstream's
/// vendored-fork entry) would let cargo see a misrouted source-id
/// at build time, producing a confusing downstream failure rather
/// than a clear-message-up-front REJECT.
///
/// Plan §5.2.8.
#[test]
fn materialize_rejects_when_upstream_patch_targets_external_source_rule4() {
    let tmp = tempfile::tempdir().expect("creating tempdir for Rule 4 REJECT integration test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "foo"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[patch.crates-io]
foo = { path = "../my-fork" }
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    // The `../my-fork` sibling dir is not required to actually exist
    // for Rule 4 detection to fire — Rule 4's discriminant is the
    // lexical-normalized-path inequality vs upstream root, NOT
    // filesystem existence. We don't create it to keep the test
    // hermetic.

    let err = materialize_overlay(&upstream_manifest)
        .expect_err("Rule 4 REJECT must surface as Err(_) at the integration layer");

    match err {
        lihaaf::Error::CompatPatchOverrideConflict {
            crate_name,
            upstream_entry: _,
            expected_resolution,
        } => {
            assert_eq!(
                crate_name, "foo",
                "Rule 4 REJECT must name the self-keyed crate (`foo`); \
                 got `{crate_name}`"
            );
            assert!(
                expected_resolution.contains("compat-allow-patch-override"),
                "Rule 4 REJECT message must reference the v0.2/v1.1 escape \
                 hatch (`--compat-allow-patch-override`); got: {expected_resolution}"
            );
        }
        other => panic!(
            "Rule 4 must return Error::CompatPatchOverrideConflict at the \
             integration layer; got {other:?}"
        ),
    }
}

/// §5.2.9 — SEC-8 Rule 1 INJECT cargo-graph proof: workspace member registry
/// dep remapped via injected self-patch (R8 rescope).
///
/// **R8 rescope (Codex rollout 019e3cc3, 2026-05-18):** The prior fixture
/// carried `[dependencies] test-suite = { path = "test-suite" }` in the
/// root, creating a `bar → test-suite → bar` active-dep cycle that cargo
/// rejects even when both `bar` references resolve to the same source-id.
/// The `root → member → root` topology proof is empirically impossible:
/// cargo rejects it unconditionally at the dep-cycle check.
///
/// This test is rescoped to prove what cargo ACTUALLY accepts: a workspace
/// whose root does NOT depend on the member, but whose member carries
/// `bar = "1.0"` (registry-name dep). Rule 1 INJECT fires (no upstream
/// `[patch.crates-io.bar]` entry) and adds the patch. The member's
/// `bar = "1.0"` reference is redirected via the injected patch to the
/// staged-overlay path source-id, which is the same source-id as the root
/// `bar`'s `[package]` → cargo resolves without ambiguity.
///
/// **What this test bites.** A regression that fails to absolutize the
/// injected `[patch.crates-io.bar].path` (or emits a form that cargo
/// doesn't recognize as a path source) would leave `bar = "1.0"` resolving
/// to crates.io while the root resolves to the staged-overlay path → cargo's
/// resolver sees two source-ids for `bar` and reports the ambiguity error
/// (`specification \`bar\` is ambiguous`).
///
/// **What this test does NOT prove.** The `root → member → root` active-dep
/// cycle (root declares test-suite as a `[dependencies]` entry that also
/// dep-on-root-by-registry-name) is rejected by cargo regardless of patch
/// state. That topology is unfaithful to real upstreams (cxx does not carry
/// a root dep on its test-suite workspace member). Tests §5.2.6 and this
/// §5.2.9 both mirror faithful upstream shapes.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; plan §5.2.9.
#[test]
fn cargo_accepts_workspace_member_registry_dep_via_self_patch() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_accepts_workspace_member_registry_dep_via_self_patch: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp =
        tempfile::tempdir().expect("creating tempdir for SEC-8 Rule 1 INJECT cargo-graph test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");
    // Root manifest: workspace declaration ONLY — no root dep on member.
    // cxx-faithful shape: root is in the workspace with its test-suite member,
    // but root does NOT carry `[dependencies] test-suite = { path = "test-suite" }`.
    // Adding that edge creates a `bar → test-suite → bar` active-dep cycle
    // that cargo rejects; omitting it is the correct faithful shape.
    //
    // cargo rustc below runs `-p bar` against the staged overlay; the member
    // is in the workspace but not a build dep of the root.
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "bar"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[workspace]
members = ["test-suite"]
"#,
    )
    .expect("writing upstream Cargo.toml");
    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        "pub fn _stub() {}\n",
    )
    .expect("writing src/lib.rs");

    // Workspace member referencing `bar` by registry name — the dep Rule 1
    // INJECT must redirect to the staged-overlay path.
    let member_dir = upstream_dir.join("test-suite");
    std::fs::create_dir_all(member_dir.join("src")).expect("creating test-suite/src/");
    std::fs::write(
        member_dir.join("Cargo.toml"),
        r#"[package]
name = "test-suite"
version = "0.0.0"
edition = "2021"

[dependencies]
bar = "1.0"
"#,
    )
    .expect("writing test-suite/Cargo.toml");
    std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn _stub() {}\n")
        .expect("writing test-suite/src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Pre-cargo sanity: Rule 1 INJECT must have added
    // `[patch.crates-io.bar] = { path = "<staged-overlay-dir>" }`.
    let content = read_overlay(&plan.sibling_manifest);
    let parsed: toml::Value = toml::from_str(&content).expect("overlay must be valid TOML");
    let bar_path = parsed
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("bar"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("Rule 1 INJECT must add [patch.crates-io.bar].path");
    assert!(
        bar_path.ends_with("/target/lihaaf-overlay"),
        "Rule 1 INJECT path must target the staged-overlay-dir; got `{bar_path}`"
    );

    // The acid test: cargo rustc -p bar against the staged overlay. Pre-fix
    // (without Rule 1 INJECT), cargo's resolver sees `bar` referenced BOTH as
    // the workspace root (path source) AND via `test-suite`'s `bar = "1.0"`
    // registry-name dep → `specification \`bar\` is ambiguous`. Post-fix,
    // the injected patch redirects the registry-name reference to the
    // staged-overlay path → both references collapse to the same source-id →
    // resolution clean. Root doesn't dep on member so no active-dep cycle.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("rustc")
        .arg("-p")
        .arg("bar")
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
        "cargo rustc must accept the workspace-member-registry-dep-via-self-patch \
         topology (SEC-8 Rule 1 INJECT closure); got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.10 — M.5 staged-mirror fixes anyhow-shape silent-false probe.
///
/// Constructs the anyhow build-script probe pattern: `build.rs`
/// checks `Path::new("src").join("nightly.rs")` via
/// `Path::exists()` and emits a custom cfg based on the result.
/// Pre-mirror, the probe file is not accessible from
/// `<staged-overlay>/`, so `probe.exists()` returns false → cargo
/// sets `probe_file_missing` cfg → `src/lib.rs` triggers
/// `compile_error!`, turning the silent-false probe failure into a
/// HARD compile error. Post-mirror, `<staged>/src/nightly.rs` is a
/// symlink to upstream → `probe.exists()` returns true →
/// `probe_file_found` cfg → cargo build succeeds.
///
/// **Why the `compile_error!` is load-bearing.** The anyhow / serde_json
/// silent-false failure mode is the most insidious class: no error
/// message, but the wrong cfg is emitted, which silently changes
/// downstream macro expansions. A test that just asserts cargo build
/// success would PASS pre-fix too (cargo doesn't error on silent-false).
/// Gating downstream code on the cfg via `compile_error!` makes the
/// silent-false a loud, gate-tripping compile failure.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; plan §5.2.10.
#[test]
fn cargo_build_anyhow_shape_probe_file_resolves_via_mirror() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_build_anyhow_shape_probe_file_resolves_via_mirror: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for M.5 anyhow-probe test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "anyhow-like"
version = "1.0.0"
edition = "2021"
build = "build.rs"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("writing upstream Cargo.toml");

    // build.rs probe pattern (mirrors anyhow build.rs:255-257 + :323-367).
    std::fs::write(
        upstream_dir.join("build.rs"),
        r#"fn main() {
    let probe = std::path::Path::new("src").join("nightly.rs");
    if probe.exists() {
        println!("cargo:rustc-cfg=probe_file_found");
    } else {
        println!("cargo:rustc-cfg=probe_file_missing");
    }
    // Tell cargo we look at this cfg (suppresses unexpected_cfgs warning
    // on newer cargos so the test surfaces only the load-bearing failure).
    println!("cargo:rustc-check-cfg=cfg(probe_file_found)");
    println!("cargo:rustc-check-cfg=cfg(probe_file_missing)");
    println!("cargo:rerun-if-changed=src/nightly.rs");
}
"#,
    )
    .expect("writing build.rs");

    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("nightly.rs"),
        "// nightly probe stub\n",
    )
    .expect("writing src/nightly.rs");

    // src/lib.rs uses `cfg`-gated `compile_error!` to make
    // `probe_file_missing` a hard build failure. The `cfg(probe_file_found)`
    // branch is a no-op so the success path compiles cleanly.
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        r#"#[cfg(probe_file_found)]
pub fn probe_found() {}
#[cfg(probe_file_missing)]
compile_error!("probe_file_missing: staged-mirror did not provide src/nightly.rs");
"#,
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    // Mirror sanity: `<staged>/src/nightly.rs` must be accessible
    // (via symlink or copy). Pre-fix without the mirror, this file
    // is missing and the build.rs probe fires the silent-false →
    // compile_error! path.
    let staged_overlay_dir = plan
        .sibling_manifest
        .parent()
        .expect("staged manifest has a parent");
    let mirrored_probe = staged_overlay_dir.join("src").join("nightly.rs");
    assert!(
        mirrored_probe.exists(),
        "mirror must populate <staged>/src/nightly.rs; got missing at {:?}",
        mirrored_probe
    );

    // Acid test: cargo build against the staged overlay. The
    // `compile_error!` in src/lib.rs fires if `probe_file_missing`
    // → cargo build fails with the message. Post-mirror, the probe
    // file is accessible → `probe_file_found` → cargo build clean.
    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&plan.sibling_manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("spawning cargo build; CI must have cargo on PATH");

    assert!(
        output.status.success(),
        "cargo build must succeed for the M.5 anyhow-shape probe \
         (staged-mirror provides src/nightly.rs); got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.11 — M.6 staged-mirror fixes thiserror-shape silent-false probe.
///
/// Same pattern as §5.2.10 but for the `build/probe.rs` path form
/// (distinct top-level mirror entry: `build/` instead of `src/`).
/// This pins the contract that the mirror writer handles top-level
/// `build/` as a real directory to be mirrored, not as a cargo-
/// reserved name. If the implementation special-cases `src/` but
/// forgets `build/`, this test surfaces the gap.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`; plan §5.2.11.
#[test]
fn cargo_build_thiserror_shape_probe_file_resolves_via_mirror() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_build_thiserror_shape_probe_file_resolves_via_mirror: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for M.6 thiserror-probe test");
    let upstream_dir = tmp.path();

    assert!(
        upstream_dir.is_absolute(),
        "tempdir must be absolute (CompatArgs::from_cli guarantees this at the CLI layer)"
    );

    let upstream_manifest = upstream_dir.join("Cargo.toml");
    std::fs::write(
        &upstream_manifest,
        r#"[package]
name = "thiserror-like"
version = "1.0.0"
edition = "2021"
build = "build.rs"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("writing upstream Cargo.toml");

    // build.rs probe pattern (mirrors thiserror build.rs:261-263 + :328-371).
    std::fs::write(
        upstream_dir.join("build.rs"),
        r#"fn main() {
    let probe = std::path::Path::new("build").join("probe.rs");
    if probe.exists() {
        println!("cargo:rustc-cfg=probe_file_found");
    } else {
        println!("cargo:rustc-cfg=probe_file_missing");
    }
    println!("cargo:rustc-check-cfg=cfg(probe_file_found)");
    println!("cargo:rustc-check-cfg=cfg(probe_file_missing)");
    println!("cargo:rerun-if-changed=build/probe.rs");
}
"#,
    )
    .expect("writing build.rs");

    // `build/` as an UPSTREAM directory (NOT cargo's reserved
    // `build = "build.rs"` script — that's a file).
    std::fs::create_dir_all(upstream_dir.join("build")).expect("creating build/");
    std::fs::write(
        upstream_dir.join("build").join("probe.rs"),
        "// thiserror build probe stub\n",
    )
    .expect("writing build/probe.rs");

    std::fs::create_dir_all(upstream_dir.join("src")).expect("creating src/");
    std::fs::write(
        upstream_dir.join("src").join("lib.rs"),
        r#"#[cfg(probe_file_found)]
pub fn probe_found() {}
#[cfg(probe_file_missing)]
compile_error!("probe_file_missing: staged-mirror did not provide build/probe.rs");
"#,
    )
    .expect("writing src/lib.rs");

    let plan = materialize_overlay(&upstream_manifest).expect("overlay must succeed");

    let staged_overlay_dir = plan
        .sibling_manifest
        .parent()
        .expect("staged manifest has a parent");
    let mirrored_probe = staged_overlay_dir.join("build").join("probe.rs");
    assert!(
        mirrored_probe.exists(),
        "mirror must populate <staged>/build/probe.rs (the M.6 path \
         form, distinct from the M.5 `src/` form); got missing at {:?}",
        mirrored_probe
    );

    let target_dir = upstream_dir.join("target").join("lihaaf-build");
    let output = std::process::Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&plan.sibling_manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("spawning cargo build; CI must have cargo on PATH");

    assert!(
        output.status.success(),
        "cargo build must succeed for the M.6 thiserror-shape probe \
         (staged-mirror provides build/probe.rs); got exit {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// §5.2.12 — absolute-path pin for the injected/remapped self-patch.
///
/// Unit-style integration test (UNGATED) that asserts the
/// materialized overlay's `[patch.crates-io.<self>].path` value is:
///
///   1. Absolute (starts with `/` on Unix, drive letter prefix on
///      Windows).
///   2. Tail-matches `/target/lihaaf-overlay` (the staged-overlay
///      directory shape).
///
/// Applies to BOTH Rule 1 (INJECT) and Rule 2 (REMAP) emission
/// paths — both rules emit the same absolute byte shape per plan
/// §6.5 (the unified emission contract).
///
/// **Why this is a separate test from §5.1.3 (the unit-test inode
/// pin).** §5.1.3 is in `src/compat/overlay.rs::tests` and exercises
/// the policy from inside the crate. §5.2.12 is at the
/// integration-crate boundary, going through the
/// `lihaaf::compat_overlay_materialize` re-export. A future
/// regression that downgraded the re-export's surface (e.g.
/// returning a stripped path representation across the crate
/// boundary) would slip past §5.1.3.
///
/// Plan §5.2.12 (the absolute-path pin).
#[test]
fn patch_crates_io_self_injection_absolute_path_only() {
    // Rule 1 INJECT case: clean upstream, no pre-existing patch.
    let rule1_input = r#"[package]
name = "demo"
version = "0.1.0"
"#;
    let (tmp1, upstream1) = write_upstream(rule1_input);
    let plan1 = materialize_overlay(&upstream1).expect("Rule 1 INJECT overlay must succeed");
    let content1 = read_overlay(&plan1.sibling_manifest);
    let parsed1: toml::Value = toml::from_str(&content1).expect("Rule 1 overlay must be TOML");
    let rule1_path = parsed1
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("demo"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("Rule 1 INJECT must produce [patch.crates-io.demo].path");
    assert!(
        Path::new(rule1_path).is_absolute(),
        "Rule 1 INJECT path MUST be absolute (plan §6.5: avoids cargo's \
         coincidental anchoring of `.`; future-proofs against \
         absolutize_patch_paths policy changes); got `{rule1_path}`"
    );
    assert!(
        rule1_path.ends_with("/target/lihaaf-overlay"),
        "Rule 1 INJECT path MUST tail-match `/target/lihaaf-overlay` \
         (plan §6.5 emission contract); got `{rule1_path}`"
    );
    drop(tmp1);

    // Rule 2 REMAP case: upstream carries `path = "."` self-patch.
    let rule2_input = r#"[package]
name = "demo"
version = "0.1.0"

[patch.crates-io]
demo = { path = "." }
"#;
    let (tmp2, upstream2) = write_upstream(rule2_input);
    let plan2 = materialize_overlay(&upstream2).expect("Rule 2 REMAP overlay must succeed");
    let content2 = read_overlay(&plan2.sibling_manifest);
    let parsed2: toml::Value = toml::from_str(&content2).expect("Rule 2 overlay must be TOML");
    let rule2_path = parsed2
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|c| c.get("demo"))
        .and_then(|e| e.get("path"))
        .and_then(|v| v.as_str())
        .expect("Rule 2 REMAP must produce [patch.crates-io.demo].path");
    assert!(
        Path::new(rule2_path).is_absolute(),
        "Rule 2 REMAP path MUST be absolute (plan §6.5 emission contract \
         is unified across Rule 1 / Rule 2); got `{rule2_path}`"
    );
    assert!(
        rule2_path.ends_with("/target/lihaaf-overlay"),
        "Rule 2 REMAP path MUST tail-match `/target/lihaaf-overlay`; \
         got `{rule2_path}`"
    );

    // Cross-rule shape parity pin: Rule 1 and Rule 2 emit the same
    // tail. If a future regression diverged the emission byte shape
    // (e.g. trailing slash on one form but not the other), the
    // corpus-determinism test (`byte_identical_across_two_lihaaf_binaries_on_corpus`)
    // would catch it on the canonical fixtures, but this assertion
    // pins the contract in source even if both fixtures drift in
    // parallel.
    let rule1_tail = rule1_path
        .rsplit('/')
        .next()
        .expect("rule1 path has at least one component");
    let rule2_tail = rule2_path
        .rsplit('/')
        .next()
        .expect("rule2 path has at least one component");
    assert_eq!(
        rule1_tail, rule2_tail,
        "Rule 1 / Rule 2 emission must share the same final component \
         (unified emission contract); got Rule 1 `{rule1_tail}` vs \
         Rule 2 `{rule2_tail}`"
    );
    drop(tmp2);
}

/// **Issue #53 §7.3 integration test: axum-macros workspace-member shape.**
///
/// Synthesizes a workspace that mirrors `tokio-rs/axum`'s shape (virtual
/// workspace with `members = ["pkg-a", "pkg-*"]`, `[workspace.package]
/// edition = "2021"`, `[workspace.dependencies] serde = "1.0"`) and a
/// member `pkg-macros` with `{ workspace = true }` inheritance refs
/// (`rust-version`, `[lints] workspace = true`, `[dependencies] serde =
/// { workspace = true }`). Resolves via `resolve_workspace_member_manifest`,
/// then materializes via `materialize_overlay_with_metadata_and_workspace_member_context`
/// with a populated `WorkspaceMemberContext`. Asserts:
///
/// 1. The resolver returns the member manifest at
///    `<ws>/pkg-macros/Cargo.toml`.
/// 2. The overlay is staged at
///    `<ws>/pkg-macros/target/lihaaf-overlay/Cargo.toml` per §3.1.bis
///    (the staging dir is `<member_root>/target/lihaaf-overlay/`, NOT
///    `<workspace_root>/target/lihaaf-overlay/`).
/// 3. The overlay's `[workspace.dependencies.serde]` carries the
///    workspace-root value (carry-down from §5.3).
/// 4. The overlay's `[workspace.package.edition]` carries the
///    workspace-root value.
/// 5. The overlay's `[workspace]` table does NOT carry `members`,
///    `exclude`, or `default-members` (stripped per
///    `override_workspace_inheritance` Branch 4).
/// 6. The materialization does NOT hit any REJECT branch — the
///    workspace-member context suppresses Branches 2 + 3 of
///    `override_workspace_inheritance`.
///
/// Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` per
/// [[lihaaf-no-local-binary-builds]] — although this test does not
/// spawn `cargo`, it stays under the same gate as the other compat-
/// pipeline integration tests so adopters running `cargo test --lib`
/// on RAM-limited boxes can opt in once for the whole compat-pipeline
/// surface.
#[test]
fn cargo_lihaaf_resolves_axum_macros_shape_workspace_member() {
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping cargo_lihaaf_resolves_axum_macros_shape_workspace_member: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this automatically)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("creating tempdir for axum-macros shape test");
    let ws_root = tmp.path();
    let ws_root_manifest = ws_root.join("Cargo.toml");

    // Synthesize the workspace root (axum-macros shape: virtual
    // workspace with `[workspace.package]` carry-down +
    // `[workspace.dependencies]` carry-down + glob members).
    let ws_root_toml = r#"[workspace]
members = ["pkg-a", "pkg-*"]

[workspace.package]
edition = "2021"
rust-version = "1.65"

[workspace.dependencies]
serde = "1.0"
"#;
    std::fs::write(&ws_root_manifest, ws_root_toml).expect("write workspace-root manifest");

    // Synthesize pkg-a (non-target member; the glob `pkg-*` will also
    // match pkg-macros, but pkg-a needs to exist as a non-target to
    // verify the resolver picks pkg-macros by name not by directory
    // enumeration order).
    let pkg_a_dir = ws_root.join("pkg-a");
    std::fs::create_dir_all(pkg_a_dir.join("src")).expect("create pkg-a/src");
    std::fs::write(
        pkg_a_dir.join("Cargo.toml"),
        r#"[package]
name = "pkg-a"
version = "0.1.0"
rust-version = { workspace = true }

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("write pkg-a Cargo.toml");
    std::fs::write(
        pkg_a_dir.join("src").join("lib.rs"),
        "pub fn _stub_a() {}\n",
    )
    .expect("write pkg-a src/lib.rs");

    // Synthesize pkg-macros (the target member).
    let pkg_macros_dir = ws_root.join("pkg-macros");
    std::fs::create_dir_all(pkg_macros_dir.join("src")).expect("create pkg-macros/src");
    std::fs::write(
        pkg_macros_dir.join("Cargo.toml"),
        r#"[package]
name = "pkg-macros"
version = "0.1.0"
rust-version = { workspace = true }

[lib]
proc-macro = true
path = "src/lib.rs"

[dependencies]
serde = { workspace = true }
"#,
    )
    .expect("write pkg-macros Cargo.toml");
    std::fs::write(
        pkg_macros_dir.join("src").join("lib.rs"),
        "// minimal proc-macro lib stub for #53 integration test\n",
    )
    .expect("write pkg-macros src/lib.rs");

    // 1. Resolver step.
    let (member_manifest, ws_root_value) =
        lihaaf::compat_resolve_workspace_member_manifest(&ws_root_manifest, "pkg-macros")
            .expect("resolver must find pkg-macros via glob match");
    assert_eq!(
        member_manifest,
        pkg_macros_dir.join("Cargo.toml"),
        "resolver must return the pkg-macros manifest path"
    );

    // 2. Materializer step.
    let ctx = WorkspaceMemberContext {
        workspace_root_manifest: ws_root_manifest.clone(),
        workspace_root_value: ws_root_value,
    };
    let plan = materialize_overlay_with_metadata_and_ctx(&member_manifest, None, Some(&ctx))
        .expect("materialization must succeed under workspace-member context");

    // Overlay is staged at <member>/target/lihaaf-overlay/Cargo.toml.
    let expected_overlay_path = pkg_macros_dir
        .join("target")
        .join("lihaaf-overlay")
        .join("Cargo.toml");
    assert_eq!(
        plan.sibling_manifest, expected_overlay_path,
        "overlay must be staged under <member_root>/target/lihaaf-overlay/ per §3.1.bis"
    );

    // 3-5. Read the overlay and assert the carry-down + strip
    // contracts.
    let overlay_text = read_overlay(&plan.sibling_manifest);
    let overlay_value: toml::Value =
        toml::from_str(&overlay_text).expect("overlay TOML must parse");
    let overlay_table = overlay_value.as_table().expect("overlay is a table");

    let workspace_table = overlay_table
        .get("workspace")
        .and_then(|v| v.as_table())
        .expect("overlay must have [workspace]");
    // [workspace.dependencies.serde] carried from workspace root.
    let ws_deps = workspace_table
        .get("dependencies")
        .and_then(|v| v.as_table())
        .expect("[workspace.dependencies] must be carried");
    assert!(
        ws_deps.contains_key("serde"),
        "[workspace.dependencies.serde] must be carried from workspace root"
    );
    // [workspace.package.edition] carried.
    let ws_pkg = workspace_table
        .get("package")
        .and_then(|v| v.as_table())
        .expect("[workspace.package] must be carried");
    assert_eq!(
        ws_pkg.get("edition").and_then(|v| v.as_str()),
        Some("2021"),
        "[workspace.package.edition] must be carried verbatim"
    );

    // Membership keys must be stripped.
    assert!(
        !workspace_table.contains_key("members"),
        "[workspace.members] must be stripped from overlay"
    );
    assert!(
        !workspace_table.contains_key("exclude"),
        "[workspace.exclude] must be stripped from overlay"
    );
    assert!(
        !workspace_table.contains_key("default-members"),
        "[workspace.default-members] must be stripped from overlay"
    );

    // 6. Option H Rule 1 INJECT: the workspace root did not declare a
    // self-patch for pkg-macros, so an injected self-patch points at
    // the staged-overlay dir.
    let patch_self = overlay_table
        .get("patch")
        .and_then(|v| v.get("crates-io"))
        .and_then(|v| v.get("pkg-macros"))
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .expect("[patch.crates-io.pkg-macros].path must be INJECTed (Option H Rule 1)");
    assert!(
        patch_self.ends_with("lihaaf-overlay"),
        "Rule 1 INJECT must target the staged-overlay dir; got `{patch_self}`"
    );

    drop(tmp);
}
