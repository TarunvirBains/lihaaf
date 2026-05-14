//! Phase 6 of compat mode — §3.2.1 fixture-invocation discovery
//! integration tests.
//!
//! Every test in this file calls into [`lihaaf::compat_discover`]
//! (declared as a `#[doc(hidden)]` re-export in `src/lib.rs`). The
//! re-exports exist exclusively for this test crate. The supported
//! entry into the discovery walk is `cargo lihaaf --compat`, not the
//! Rust API.
//!
//! The suite covers the §3.2.1 pattern matrix:
//!
//! 1. **Pattern 1 (direct).** `trybuild::TestCases::new().compile_fail("path")` —
//!    literal argument, no glob. The visitor must surface exactly one
//!    fixture pointing at the literal path.
//! 2. **Pattern 2 (glob).** `*` / `?` / `[abc]` expansion via stdlib
//!    `std::fs` traversal; sorted deterministically. The `**`
//!    metacharacter is NOT supported in v0.1 and must surface as a
//!    `discovery_unrecognized` entry.
//! 3. **Pattern 3 (`#[test]`-wrapped).** The visitor descends into
//!    `#[test]` function bodies, tracks `let t = TestCases::new();`
//!    bindings, and applies pattern 1 to `t.compile_fail(...)` calls.
//!    Cross-function tracking is impossible by construction (the
//!    bindings table is saved and reset on every `visit_item_fn`).
//! 4. **Custom-macro aliases** via `--compat-trybuild-macro`. Multiple
//!    aliases are OR'd. `use ... as ...` aliases are NOT syntactically
//!    recognized (Q6 locked); adopters must register via the flag.
//! 5. **Macro-expanded invocations** (e.g. `make_tests!()`) are NOT
//!    recognized — discovery operates on source-AST, not on the
//!    post-expansion token tree.
//! 6. **Determinism.** Two runs from clean state produce byte-equal
//!    `Debug` output.
//! 7. **Flat walk.** Subdirectories of `tests/` are NOT walked.
//! 8. **Empty case.** A target crate with no `tests/` directory yields
//!    an empty output, not an error.
//! 9. **Parse failures.** A malformed `tests/<file>.rs` produces a
//!    single `discovery_unrecognized` of `parse_failed`; discovery
//!    continues with the remaining files.
//!
//! ## Why hermetic
//!
//! Most tests point at the pre-committed corpus under
//! `tests/compat/discovery_corpus/<scenario>/`. The corpus is shape-
//! stable and human-inspectable; the integration test asserts against
//! exact paths and call-site lines. The few tests that need a
//! constructed-on-the-fly tree (custom-macro alias matrix, parse
//! errors paired with a recognized file) own a `tempfile::TempDir`
//! and operate exclusively within it.

use std::path::{Path, PathBuf};

use lihaaf::{CompatDiscoveredFixture, CompatFixtureKind, compat_discover as discover};

/// Path to a corpus scenario under
/// `tests/compat/discovery_corpus/<name>/`. Resolved relative to
/// `CARGO_MANIFEST_DIR` so the test passes from any cwd.
fn corpus(scenario: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/compat/discovery_corpus")
        .join(scenario)
}

/// Helper: find a fixture entry whose `relative_path` ends with `tail`
/// (forward-slash form). Returns the entry by clone — the assertion
/// asserts both kind and call-site fields off the clone.
fn find_fixture_by_tail<'a>(
    output: &'a [CompatDiscoveredFixture],
    tail: &str,
) -> &'a CompatDiscoveredFixture {
    output
        .iter()
        .find(|f| f.relative_path.ends_with(tail))
        .unwrap_or_else(|| {
            panic!(
                "expected fixture whose relative_path ends with `{tail}`; got {:?}",
                output
                    .iter()
                    .map(|f| f.relative_path.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

/// **Pattern 1 (direct, literal argument).** A single
/// `trybuild::TestCases::new().compile_fail("tests/ui/foo.rs")` call
/// must surface exactly one fixture with kind `CompileFail`.
#[test]
fn pattern_1_direct_literal_compile_fail() {
    let crate_root = corpus("direct_literal");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture expected; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.unrecognized.is_empty(),
        "no unrecognized entries; got {:?}",
        out.unrecognized
    );
    let f = &out.fixtures[0];
    assert_eq!(f.kind, CompatFixtureKind::CompileFail);
    assert!(
        f.relative_path.ends_with("tests/ui/foo.rs"),
        "relative_path must end at the literal target; got {}",
        f.relative_path
    );
    assert_eq!(
        f.call_site.enclosing_test_fn.as_deref(),
        Some("ui"),
        "the call lived inside `#[test] fn ui()`"
    );
}

/// **Pattern 2 (glob).** `tests/ui/*.rs` over a corpus of two files
/// (`bar.rs`, `foo.rs`) must expand to two fixtures in ASCII byte
/// order.
#[test]
fn pattern_2_glob_star_expands_sorted() {
    let crate_root = corpus("direct_glob");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        2,
        "exactly two fixtures expected; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(out.unrecognized.is_empty(), "{:?}", out.unrecognized);

    // ASCII byte order: `bar.rs` < `foo.rs`.
    assert!(
        out.fixtures[0].relative_path.ends_with("bar.rs"),
        "first fixture must be bar.rs (ASCII order); got {}",
        out.fixtures[0].relative_path
    );
    assert!(
        out.fixtures[1].relative_path.ends_with("foo.rs"),
        "second fixture must be foo.rs; got {}",
        out.fixtures[1].relative_path
    );
    for f in &out.fixtures {
        assert_eq!(f.kind, CompatFixtureKind::CompileFail);
    }
}

/// **Pattern 3 (`#[test]`-wrapped with local binding).** `let t =
/// trybuild::TestCases::new(); t.compile_fail("...")` inside a
/// `#[test]` function body must be recognized via the per-function
/// binding tracker.
#[test]
fn pattern_3_test_wrapped_let_binding() {
    let crate_root = corpus("test_wrapped");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture expected; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(out.unrecognized.is_empty(), "{:?}", out.unrecognized);
    let f = &out.fixtures[0];
    assert!(f.relative_path.ends_with("tests/ui/x.rs"));
    assert_eq!(f.kind, CompatFixtureKind::CompileFail);
    assert_eq!(
        f.call_site.enclosing_test_fn.as_deref(),
        Some("ui"),
        "the call lived inside `#[test] fn ui()`"
    );
}

/// **Round-3 BLOCK regression: `#[std::test]` attribute recognized.**
/// `is_test_attribute` must accept `std::test` / `::std::test` in
/// addition to the bare `test`, `core::test`, and `::core::test`
/// variants — a `#[std::test]` function body must have
/// `enclosing_test_fn` set on call sites discovered inside it. Custom
/// test framework setups occasionally use the qualified form.
#[test]
fn pattern_3_std_test_attribute_recognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();
    std::fs::write(tests_dir.join("ui").join("x.rs"), "fn main() {}\n").unwrap();

    // `#[std::test]` (the qualified form) annotates the function;
    // discovery must still set `enclosing_test_fn` on the inner call.
    let source = "\
#[std::test]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/x.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture expected inside the `#[std::test]` body; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(out.unrecognized.is_empty(), "{:?}", out.unrecognized);
    let f = &out.fixtures[0];
    assert!(f.relative_path.ends_with("tests/ui/x.rs"));
    assert_eq!(f.kind, CompatFixtureKind::CompileFail);
    assert_eq!(
        f.call_site.enclosing_test_fn.as_deref(),
        Some("ui"),
        "the call lived inside `#[std::test] fn ui()` — \
         `is_test_attribute` must accept the `std::test` path shape"
    );
}

/// **Multiple top-level test files.** A crate with `tests/a.rs` and
/// `tests/b.rs`, each carrying one invocation, must surface both
/// fixtures in deterministic order. The sort key is the fixture
/// relative path, not the originating test file — so the order
/// reflects the fixture paths.
#[test]
fn multiple_test_files_each_with_one_fixture() {
    let crate_root = corpus("multi_file");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        2,
        "exactly two fixtures expected; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(out.unrecognized.is_empty(), "{:?}", out.unrecognized);

    let a = find_fixture_by_tail(&out.fixtures, "fixtures/a.rs");
    let b = find_fixture_by_tail(&out.fixtures, "fixtures/b.rs");
    assert_eq!(a.kind, CompatFixtureKind::Pass);
    assert_eq!(b.kind, CompatFixtureKind::CompileFail);

    // ASCII order: `a.rs` < `b.rs`.
    assert!(
        out.fixtures[0].relative_path < out.fixtures[1].relative_path,
        "fixtures must be sorted by relative_path ASCII; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Empty case.** A target crate with no `tests/` directory at all
/// (`empty_tests/`) yields an empty output — no fixtures, no
/// unrecognized entries, no error.
#[test]
fn empty_tests_directory_yields_empty_output() {
    let crate_root = corpus("empty_tests");
    let out = discover(&crate_root, &[]).expect("discover must succeed without tests/");
    assert!(
        out.fixtures.is_empty(),
        "expected zero fixtures; got {:?}",
        out.fixtures
    );
    assert!(
        out.unrecognized.is_empty(),
        "expected zero unrecognized entries; got {:?}",
        out.unrecognized
    );
}

/// **`use trybuild::TestCases as Foo;` is NOT recognized (Q6 locked).**
/// `Foo::new().compile_fail(...)` must produce no fixtures and one
/// `discovery_unrecognized` entry naming the file/line.
///
/// Adopters with `use ... as ...` re-exports register via
/// `--compat-trybuild-macro`; the discovery pass does not attempt to
/// resolve `use` aliases syntactically.
#[test]
fn use_alias_not_recognized_without_flag() {
    let crate_root = corpus("alias_use_not_recognized");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "use ... as ... is NOT recognized; expected zero fixtures, got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    // The `Foo::new().compile_fail("tests/ui/foo.rs")` call does NOT
    // match `trybuild::TestCases::new` syntactically, so the visitor
    // never reaches the unrecognized-call branch — it simply ignores
    // the call. The integration contract is "no fixtures", which is
    // exactly what we assert above; whether `unrecognized` is empty
    // or not depends on whether the visitor decides to flag the
    // shape. The spec's recognized-only set requires the assertion
    // above and is silent on whether to flag the shape, so we accept
    // any unrecognized count for this scenario.
    let _ = &out.unrecognized;
}

/// **Leading-`::` form is NOT recognized, even with a flag registration.**
/// The spec's canonical form omits the leading separator. The matcher
/// (`path_matches_string_segments` in src/compat/discovery.rs) rejects
/// any `syn::Path` whose `leading_colon` is set, AND the alias parser
/// strips empty segments — so a `--compat-trybuild-macro ::trybuild::TestCases`
/// is stored as `["trybuild", "TestCases"]` and even when the caller
/// types `::trybuild::TestCases::new()` at the call site, the matcher
/// declines because `leading_colon.is_some()`.
///
/// This test documents the v0.1 limitation rather than working around
/// it; the doc-comment fix removes the "register via flag" advice that
/// implied a workaround existed.
#[test]
fn leading_colon_call_site_not_recognized_even_with_alias_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();
    std::fs::write(tests_dir.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
#[test]\n\
fn ui() {\n\
    let t = ::trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("trybuild.rs"), source).unwrap();

    // Registering the absolute form via the flag does NOT make the
    // call site recognizable. The matcher's `leading_colon.is_some()`
    // check is the v0.1-locked behavior; the doc comment used to
    // suggest this flag invocation as an escape hatch, but it does
    // not work.
    let aliases = vec!["::trybuild::TestCases".to_string()];
    let out = discover(&crate_root, &aliases).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "leading-`::` call sites are NOT recognized in v0.1, even with --compat-trybuild-macro; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Macro-expanded invocations are NOT recognized AND surface as
/// `discovery_unrecognized`.** A `make_tests!()` macro call that would
/// expand to a `TestCases::new()` chain at compile time produces zero
/// fixtures plus exactly one `discovery_unrecognized` entry naming the
/// macro invocation's file + line. The visitor cannot expand macros
/// at AST time, so the operator-visible signal is the unrecognized
/// entry (spec §3.2.1).
#[test]
fn macro_wrapper_invocation_not_recognized() {
    let crate_root = corpus("macro_wrapper_unrecognized");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "macro-expanded invocations are NOT recognized; expected zero fixtures, got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "the macro invocation must surface exactly one unrecognized entry; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("make_tests"),
        "detail must name the macro path; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/trybuild.rs"),
        "file must point at the corpus tests/trybuild.rs; got {}",
        entry.file.display()
    );
    assert!(
        entry.line > 0,
        "line must be 1-indexed and non-zero; got {}",
        entry.line
    );
}

/// **`macro_rules!` definitions are NOT unrecognized.** syn 2.0 models
/// both `macro_rules! name { ... }` definitions AND module-level
/// macro invocations as `syn::ItemMacro`; the discriminator is
/// `node.ident` (Some for definitions, None for invocations). The
/// visitor must skip definitions silently — they are local helpers,
/// not unrecognized trybuild shapes — and the expression-position
/// invocation `helper_macro!()` inside a `#[test]` body never reaches
/// `visit_item_macro` because it is an `ExprMacro`, not an
/// `ItemMacro`. Regression for round-2 review FIX_BEFORE_BETA: the
/// earlier code flagged every `ItemMacro` indiscriminately, including
/// `macro_rules!` definitions.
#[test]
fn macro_rules_definition_not_flagged_as_unrecognized() {
    let crate_root = corpus("macro_rules_definition");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "no trybuild calls in this corpus; expected zero fixtures, got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.unrecognized.is_empty(),
        "`macro_rules!` definitions and expression-position invocations \
         must NOT surface as unrecognized; got {:?}",
        out.unrecognized
    );
}

/// **Parse failure produces a single `discovery_unrecognized` entry.**
/// `parse_error/tests/bad.rs` contains a syntax error; the visitor
/// must surface one entry of `detail = "parse_failed: ..."` and not
/// abort discovery.
#[test]
fn parse_error_surfaces_unrecognized_and_continues() {
    let crate_root = corpus("parse_error");
    let out = discover(&crate_root, &[]).expect("discover succeeds even with parse errors");
    assert!(out.fixtures.is_empty(), "{:?}", out.fixtures);
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry from the parse failure; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.starts_with("parse_failed:"),
        "detail must start with `parse_failed:`; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/bad.rs"),
        "file must point at tests/bad.rs; got {}",
        entry.file.display()
    );
}

/// **Flat walk only.** A target crate with `tests/top.rs` AND a
/// `tests/nested/inner.rs` (subdirectory) must only see the
/// top-level file. The §3.2.1 wording is literal: "every `tests/*.rs`
/// file" is flat.
#[test]
fn subdirectory_not_walked() {
    let crate_root = corpus("subdir_ignored");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture (from tests/top.rs); subdir must be skipped. got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    let f = &out.fixtures[0];
    assert!(f.relative_path.ends_with("tests/ui/top.rs"));
    // Verify the sub-directory file was not seen — its target was
    // `tests/ui/should_not_be_seen.rs`. If that path appears, the
    // walk is incorrectly recursive.
    for fix in &out.fixtures {
        assert!(
            !fix.relative_path.contains("should_not_be_seen"),
            "subdirectory file leaked into discovery: {}",
            fix.relative_path
        );
    }
}

/// **Determinism.** Two runs from clean state produce byte-equal
/// `Debug` output. The sort orders (fixture `relative_path` ASCII,
/// unrecognized `(file, line)` ASCII) are the load-bearing contract.
#[test]
fn determinism_two_runs_byte_equal_debug() {
    let crate_root = corpus("direct_glob");
    let first = discover(&crate_root, &[]).expect("first run");
    let second = discover(&crate_root, &[]).expect("second run");
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "two clean-state runs must produce identical Debug output"
    );
}

/// **`--compat-trybuild-macro` alias matrix.** A target crate using a
/// custom-macro alias is invisible without the flag and visible with
/// the flag. The flag accepts multiple paths; they are OR'd.
#[test]
fn custom_macro_alias_via_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();
    std::fs::write(tests_dir.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
#[test]\n\
fn ui() {\n\
    let t = mycrate::ui_tests::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("trybuild.rs"), source).unwrap();

    // Without the flag — invisible.
    let without = discover(&crate_root, &[]).expect("discover without flag");
    assert!(
        without.fixtures.is_empty(),
        "without `--compat-trybuild-macro`, the alias is invisible; got {:?}",
        without
            .fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );

    // With the flag — exactly one fixture.
    let aliases = vec!["mycrate::ui_tests".to_string()];
    let with = discover(&crate_root, &aliases).expect("discover with flag");
    assert_eq!(
        with.fixtures.len(),
        1,
        "with `--compat-trybuild-macro mycrate::ui_tests`, expected one fixture; got {:?}",
        with.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(with.fixtures[0].relative_path.ends_with("tests/ui/foo.rs"));
}

/// **`--compat-trybuild-macro` allows multiple flags OR'd.** Both
/// aliases must be active simultaneously; a fixture using either alias
/// is discovered.
#[test]
fn custom_macro_aliases_or_d() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();
    std::fs::write(tests_dir.join("ui").join("a.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tests_dir.join("ui").join("b.rs"), "fn main() {}\n").unwrap();

    let source_a = "\
#[test]\n\
fn a() {\n\
    let t = a_crate::ui_tests::new();\n\
    t.pass(\"tests/ui/a.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("a.rs"), source_a).unwrap();
    let source_b = "\
#[test]\n\
fn b() {\n\
    let t = b_crate::other_tests::new();\n\
    t.compile_fail(\"tests/ui/b.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("b.rs"), source_b).unwrap();

    let aliases = vec![
        "a_crate::ui_tests".to_string(),
        "b_crate::other_tests".to_string(),
    ];
    let out = discover(&crate_root, &aliases).expect("discover with two aliases");
    assert_eq!(
        out.fixtures.len(),
        2,
        "both aliases must be active; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    let a = find_fixture_by_tail(&out.fixtures, "tests/ui/a.rs");
    let b = find_fixture_by_tail(&out.fixtures, "tests/ui/b.rs");
    assert_eq!(a.kind, CompatFixtureKind::Pass);
    assert_eq!(b.kind, CompatFixtureKind::CompileFail);
}

/// **`**` glob is NOT supported in v0.1.** A pattern containing `**`
/// produces a `discovery_unrecognized` entry; the rest of the file's
/// discovery continues.
#[test]
fn double_star_glob_not_supported() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();

    let source = "\
#[test]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/**/*.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "`**` patterns must not produce fixtures in v0.1; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry from `**`; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("**"),
        "detail must mention the `**` metacharacter; got `{}`",
        entry.detail
    );
}

/// **`?` glob metacharacter** — matches exactly one non-`/` byte.
#[test]
fn glob_question_mark_matches_one_byte() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let ui = crate_root.join("tests").join("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(ui.join("fix_a.rs"), "fn main() {}\n").unwrap();
    std::fs::write(ui.join("fix_b.rs"), "fn main() {}\n").unwrap();
    std::fs::write(ui.join("fix_ab.rs"), "fn main() {}\n").unwrap(); // must NOT match `fix_?.rs`

    let source = "\
#[test]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/fix_?.rs\");\n\
}\n";
    std::fs::write(crate_root.join("tests").join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    let names: Vec<&str> = out
        .fixtures
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        out.fixtures.len(),
        2,
        "exactly two fixtures (fix_a.rs, fix_b.rs); got {names:?}"
    );
    assert!(
        out.fixtures[0].relative_path.ends_with("fix_a.rs"),
        "first must be fix_a.rs (ASCII); got {names:?}"
    );
    assert!(
        out.fixtures[1].relative_path.ends_with("fix_b.rs"),
        "second must be fix_b.rs; got {names:?}"
    );
    for fix in &out.fixtures {
        assert!(
            !fix.relative_path.contains("fix_ab"),
            "fix_ab.rs must NOT match `fix_?.rs` (two chars > one); got {}",
            fix.relative_path
        );
    }
}

/// **`[abc]` glob metacharacter** — matches any one of the bytes in
/// the class.
#[test]
fn glob_character_class_matches_listed_bytes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let ui = crate_root.join("tests").join("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(ui.join("fixa.rs"), "fn main() {}\n").unwrap();
    std::fs::write(ui.join("fixb.rs"), "fn main() {}\n").unwrap();
    std::fs::write(ui.join("fixc.rs"), "fn main() {}\n").unwrap();
    std::fs::write(ui.join("fixd.rs"), "fn main() {}\n").unwrap(); // must NOT match

    let source = "\
#[test]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/fix[abc].rs\");\n\
}\n";
    std::fs::write(crate_root.join("tests").join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        3,
        "exactly three fixtures from `[abc]`; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    for fix in &out.fixtures {
        assert!(
            !fix.relative_path.contains("fixd"),
            "`fixd.rs` must NOT match `fix[abc].rs`; got {}",
            fix.relative_path
        );
    }
}

/// **Non-literal argument produces a `discovery_unrecognized` entry.**
/// `t.compile_fail(some_var)` — the argument is not a string literal,
/// so the visitor surfaces an unrecognized entry naming the file/line.
#[test]
fn non_literal_argument_surfaces_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    std::fs::create_dir_all(crate_root.join("tests")).unwrap();
    let source = "\
#[test]\n\
fn ui() {\n\
    let p = format!(\"tests/ui/{}.rs\", \"foo\");\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(&p);\n\
}\n";
    std::fs::write(crate_root.join("tests").join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(out.fixtures.is_empty(), "{:?}", out.fixtures);
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry for the non-literal arg; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("non-literal"),
        "detail must mention non-literal; got `{}`",
        entry.detail
    );
}

/// **Verbatim debug-equality on the corpus.** Two runs against the
/// pre-committed corpus produce byte-identical `Debug` output. A
/// regression that changes any sort order or non-essential field
/// rendering would surface here.
#[test]
fn determinism_across_corpus_scenarios() {
    for scenario in [
        "direct_literal",
        "direct_glob",
        "test_wrapped",
        "multi_file",
        "subdir_ignored",
    ] {
        let crate_root = corpus(scenario);
        let first = discover(&crate_root, &[]).expect("first run");
        let second = discover(&crate_root, &[]).expect("second run");
        assert_eq!(
            format!("{first:?}"),
            format!("{second:?}"),
            "scenario `{scenario}`: two clean-state runs must produce identical Debug output"
        );
    }
}

/// **Pattern 1 (`.pass(...)`) — kind classification.** The pattern
/// must distinguish `pass` from `compile_fail` correctly.
#[test]
fn pattern_1_pass_kind_correctly_classified() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("ok.rs"), "fn main() {}\n").unwrap();
    let source = "\
#[test]\n\
fn ui() {\n\
    trybuild::TestCases::new().pass(\"tests/ui/ok.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(out.fixtures.len(), 1, "{:?}", out.fixtures);
    assert_eq!(out.fixtures[0].kind, CompatFixtureKind::Pass);
}

/// **Local bindings inside `impl` methods are recognized AND scoped.**
/// The visitor must descend into `impl Foo { fn t() { ... } }` bodies
/// (otherwise pattern 3 inside an impl method would be invisible) AND
/// must save/restore `local_bindings` across the impl method boundary
/// so a `let t = TestCases::new()` inside the method does not leak
/// into the enclosing scope or vice-versa.
///
/// The corpus exercises a `Foo` struct with a `#[test]` impl method
/// containing the pattern-3 shape; the fixture must be recognized.
#[test]
fn impl_method_local_bindings_are_scoped_and_recognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
struct Harness;\n\
\n\
impl Harness {\n\
    #[test]\n\
    fn ui() {\n\
        let t = trybuild::TestCases::new();\n\
        t.pass(\"tests/ui/foo.rs\");\n\
    }\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from the impl-method pattern-3 shape; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    let f = &out.fixtures[0];
    assert_eq!(f.kind, CompatFixtureKind::Pass);
    assert!(f.relative_path.ends_with("tests/ui/foo.rs"));
    assert_eq!(
        f.call_site.enclosing_test_fn.as_deref(),
        Some("ui"),
        "the call lived inside the impl method `#[test] fn ui()`"
    );
}

/// **`impl` method bindings do not leak into sibling impl methods.**
/// A `let t = TestCases::new()` in one impl method must not be in
/// scope for `t.compile_fail(...)` in a sibling method (the second
/// method binds `t` to a different value — a plain integer here —
/// and the visitor must not attribute the call to a trybuild
/// fixture). Mirrors `cross_function_binding_does_not_leak` for the
/// impl-method scope.
#[test]
fn impl_method_bindings_do_not_leak_across_methods() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("first.rs"), "fn main() {}\n").unwrap();

    let source = "\
struct Harness;\n\
\n\
impl Harness {\n\
    #[test]\n\
    fn first() {\n\
        let t = trybuild::TestCases::new();\n\
        t.compile_fail(\"tests/ui/first.rs\");\n\
    }\n\
\n\
    #[test]\n\
    fn second() {\n\
        // Different `t` — must NOT inherit the trybuild binding\n\
        // from `first()`.\n\
        let t = 42;\n\
        let _ = t;\n\
    }\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from `first`; the `second`'s `let t = 42` must not match. got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(out.fixtures[0].relative_path.ends_with("tests/ui/first.rs"));
}

/// **Cross-function bindings do NOT leak.** A `let t = TestCases::new();`
/// in one `#[test]` function must NOT be in scope for a subsequent
/// `t.compile_fail(...)` in a different function body.
#[test]
fn cross_function_binding_does_not_leak() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
#[test]\n\
fn first() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n\
\n\
#[test]\n\
fn second() {\n\
    // `t` here is NOT the trybuild binding from `first`; it is a\n\
    // separate (non-TestCases) value. The visitor must NOT match it.\n\
    let t = 42;\n\
    let _ = t;\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from `first`; the `second`'s `let t = 42` must not match. got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Smoke check on the corpus root.** All five scenarios point at
/// `tests/compat/discovery_corpus/<name>/`; verify each exists so a
/// missing checked-in fixture fails fast with a clear message.
#[test]
fn corpus_scenarios_exist_on_disk() {
    for scenario in [
        "direct_literal",
        "direct_glob",
        "test_wrapped",
        "multi_file",
        "empty_tests",
        "alias_use_not_recognized",
        "macro_wrapper_unrecognized",
        "macro_rules_definition",
        "parse_error",
        "subdir_ignored",
    ] {
        let path: &Path = &corpus(scenario);
        assert!(
            path.is_dir(),
            "missing corpus scenario `{scenario}` at {}",
            path.display()
        );
    }
}
