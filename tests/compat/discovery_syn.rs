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

/// **`use trybuild::TestCases as Foo;` without `--compat-trybuild-macro`
/// emits `discovery_unrecognized`.** `Foo::new().compile_fail(...)`
/// must produce no fixtures and exactly one `discovery_unrecognized`
/// entry naming the file/line — the round-3 fix added `visit_item_use`
/// detection so the visitor records the rename and flags the terminal
/// call on the aliased receiver. Adopters silence the warning by
/// registering the local name via `--compat-trybuild-macro Foo` (see
/// `use_alias_registered_via_flag_does_not_emit_unrecognized`).
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
    // The `Foo::new(); t.compile_fail(...)` chain emits exactly one
    // unrecognized entry — the round-3 `visit_item_use` walker
    // populates `aliased_testcases` from `use trybuild::TestCases as
    // Foo;`, and the terminal-call dispatcher (`try_record_terminal_call`)
    // surfaces the misconfigured alias on the `t.compile_fail(...)`
    // line. The entry must point at the `tests/trybuild.rs` corpus
    // file and the detail must mention the alias scenario so the
    // operator can map back to a `--compat-trybuild-macro` flag.
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry expected for the aliased terminal call; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("alias"),
        "detail must reference the alias issue; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/trybuild.rs"),
        "file must point at the corpus tests/trybuild.rs; got {}",
        entry.file.display()
    );
}

/// **Round-3 BLOCK regression: `use trybuild::TestCases as Foo;` must
/// emit `discovery_unrecognized`.** Earlier the visitor silently
/// dropped the `Foo::new(); t.compile_fail(...)` call chain because
/// `Foo` didn't match the canonical `trybuild::TestCases` path and
/// didn't match any registered `--compat-trybuild-macro` alias.
/// Adopters then lost visibility into the misconfigured-alias case.
///
/// The fix walks `ItemUse` trees to populate a per-file
/// `aliased_testcases` set whenever the rename source is `TestCases`.
/// A subsequent `Foo::new()` (and any `let t = Foo::new(); t.<...>`)
/// terminal call surfaces as exactly one `discovery_unrecognized`
/// entry pointing at the `compile_fail` line, naming the alias issue
/// in `detail`. Zero fixtures because the call was never resolvable.
#[test]
fn use_alias_emits_discovery_unrecognized_for_terminal_call() {
    let crate_root = corpus("use_alias_unrecognized");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "unregistered `use ... as` alias must produce zero fixtures; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry expected for the aliased terminal call; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    // The detail must name the alias scenario so operators can map
    // the entry back to a `--compat-trybuild-macro` registration.
    assert!(
        entry.detail.contains("alias"),
        "detail must reference the alias issue; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/trybuild.rs"),
        "file must point at the corpus tests/trybuild.rs; got {}",
        entry.file.display()
    );
    // The terminal call (`compile_fail`) is on line 15 of the corpus
    // file. Line numbering is 1-indexed and the line points at the
    // method ident, not the receiver.
    assert!(
        entry.line > 0,
        "line must be 1-indexed and non-zero; got {}",
        entry.line
    );
}

/// **Round-5 BLOCK regression: an unregistered `use trybuild::TestCases
/// as Foo;` called as `Foo()` (NO `::new`) also surfaces as
/// `discovery_unrecognized`.** The previous `is_aliased_testcases_constructor`
/// only matched the two-segment `Foo::new` shape, so `Foo()` paired
/// with `Foo().compile_fail(...)` (or via a `let t = Foo();` binding)
/// silently dropped instead of emitting an entry. The registered-alias
/// matcher already accepts both `<alias>::new` AND `<alias>` forms;
/// the unregistered-alias diagnostic surface should mirror it so
/// adopters using either constructor idiom see the same warning.
///
/// Two sub-shapes: the direct chain (`Foo().compile_fail(...)`) and
/// the bound chain (`let t = Foo(); t.compile_fail(...)`). Each must
/// produce exactly one unrecognized entry and zero fixtures.
#[test]
fn use_alias_called_without_new_emits_discovery_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    // Direct chain: `Foo().compile_fail(...)` (no `::new`, no binding).
    let source = "\
use trybuild::TestCases as Foo;\n\
#[test]\n\
fn ui() {\n\
    Foo().compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "unregistered `Foo()` alias must produce zero fixtures; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry expected for the `Foo()` form; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("alias"),
        "detail must mention the alias scenario; got `{}`",
        entry.detail
    );
}

/// **`use ... as Foo;` bound via `let t = Foo();` (no `::new`) is also
/// flagged.** Companion to the direct-call shape above — exercises the
/// `is_aliased_testcases_constructor_expr` path that populates
/// `aliased_bindings`. The terminal call on the binding must emit
/// exactly one `discovery_unrecognized`.
#[test]
fn use_alias_let_bound_without_new_emits_discovery_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("bar.rs"), "fn main() {}\n").unwrap();

    let source = "\
use trybuild::TestCases as Foo;\n\
#[test]\n\
fn ui() {\n\
    let t = Foo();\n\
    t.compile_fail(\"tests/ui/bar.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "unregistered `Foo()` alias + binding must produce zero fixtures; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry expected for the bound `Foo()` form; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("alias"),
        "detail must mention the alias scenario; got `{}`",
        entry.detail
    );
}

/// **Registered aliases via `--compat-trybuild-macro` are NOT flagged
/// as unrecognized even when paired with `use ... as Foo;`.** When the
/// adopter has registered the originating path, the `use` rename
/// silently re-exports a recognized name; we must not double-emit.
///
/// This test pairs with the BLOCK regression above to lock the
/// "register to silence the warning" workflow: the unrecognized
/// emission is gated on the alias NOT being registered, not on the
/// `use` rename's presence.
#[test]
fn use_alias_registered_via_flag_does_not_emit_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests_dir = crate_root.join("tests");
    std::fs::create_dir_all(tests_dir.join("ui")).unwrap();
    std::fs::write(tests_dir.join("ui").join("x.rs"), "fn main() {}\n").unwrap();

    // `use mycrate::ui_tests as Foo;` paired with a `--compat-trybuild-macro`
    // registration of `Foo` keeps the call site recognized — the
    // registered alias is `Foo`, which is exactly the local name the
    // `let t = Foo::new();` binding observes.
    let source = "\
use mycrate::ui_tests as Foo;\n\
#[test]\n\
fn ui() {\n\
    let t = Foo::new();\n\
    t.compile_fail(\"tests/ui/x.rs\");\n\
}\n";
    std::fs::write(tests_dir.join("trybuild.rs"), source).unwrap();

    let aliases = vec!["Foo".to_string()];
    let out = discover(&crate_root, &aliases).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "registered alias must resolve the call; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.unrecognized.is_empty(),
        "registered alias must not produce an unrecognized entry; got {:?}",
        out.unrecognized
    );
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

/// **Round-4 FIX regression: `use trybuild::TestCases;` (no rename)
/// IS recognized at the call site.** The most common trybuild import
/// idiom — a plain `use trybuild::TestCases;` followed by
/// `TestCases::new()` — was previously silently dropped because the
/// `ItemUse` walker only handled `UseTree::Rename` (the `use X as Y`
/// case). The fix extends the walker to also handle `UseTree::Name`
/// when the prefix is exactly `["trybuild"]`; the local name (always
/// `TestCases` in the canonical form) is then recorded into a per-file
/// `imported_testcases` set and `is_testcases_constructor_path`
/// accepts the 2-segment `TestCases::new` form.
///
/// Strict prefix match: a `use somelib::TestCases;` (different
/// upstream crate) is NOT recognized — the visitor cannot prove the
/// re-export points at trybuild's `TestCases`.
#[test]
fn use_testcases_without_rename_is_recognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
use trybuild::TestCases;\n\
#[test]\n\
fn ui() {\n\
    let t = TestCases::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert_eq!(
        out.fixtures.len(),
        1,
        "the canonical `use trybuild::TestCases;` import must produce exactly one fixture; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(out.fixtures[0].kind, CompatFixtureKind::CompileFail);
    assert!(
        out.fixtures[0].relative_path.ends_with("tests/ui/foo.rs"),
        "relative_path must point at the literal arg; got {}",
        out.fixtures[0].relative_path
    );
    assert!(
        out.unrecognized.is_empty(),
        "a recognized no-rename import must not produce any unrecognized entries; got {:?}",
        out.unrecognized
    );
}

/// **Strict prefix: `use somelib::TestCases;` is NOT recognized as
/// trybuild.** The no-rename auto-recognition demands a prefix of
/// exactly `["trybuild"]`; a third-party `TestCases` re-export must
/// not silently impersonate the canonical type. The call site
/// `TestCases::new()` resolves to neither a registered alias nor a
/// canonical import, so the visitor produces no fixtures and emits
/// nothing (the path doesn't match any of the recognized shapes; it's
/// silently dropped, which is the correct behavior for code that
/// targets a non-trybuild library).
#[test]
fn use_testcases_from_non_trybuild_prefix_is_not_recognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
use somelib::TestCases;\n\
#[test]\n\
fn ui() {\n\
    let t = TestCases::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "a non-trybuild `use ::TestCases;` must NOT be recognized; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Round-4 FIX regression: a `let` shadow of a trybuild binding
/// invalidates the binding.** Inside one `#[test]` body, an early
/// `let t = TestCases::new();` records `t` as a trybuild receiver; a
/// subsequent `let t = some_other_function();` REBINDS `t` to a
/// non-receiver. After the shadow, `t.compile_fail(...)` must NOT be
/// treated as a trybuild call — the binding tracker must remove the
/// stale entry on every non-TestCases `let` against the same ident.
///
/// Before the fix the visitor only INSERTED into `local_bindings`,
/// never REMOVED. The shadow above would leave the original `t` entry
/// in place; `t.compile_fail("path")` after the shadow would be
/// silently surfaced as a fixture even though it points at a
/// different runtime value. False positives like this survive every
/// snapshot check until a human notices the discovered fixture has
/// no business being there.
#[test]
fn let_shadow_invalidates_trybuild_binding() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    // Inside one `#[test]` body: the first `t` is a real trybuild
    // receiver; the second `let t = ...;` rebinds `t` to a String,
    // after which `t.compile_fail("path")` is NOT a trybuild call.
    // Note: we deliberately do not call `.compile_fail` on the FIRST
    // binding either — the test isolates the SHADOW semantics by
    // asserting zero fixtures, not "exactly one for the first
    // binding and zero for the shadow". The shadow's terminal call
    // is the only `.compile_fail(...)` in the body.
    let source = "\
fn make_string() -> String { String::from(\"unused\") }\n\
\n\
#[test]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    let _ = &t;\n\
    let t = make_string();\n\
    let _ = t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "the shadowed `t` is NOT a trybuild receiver; expected zero fixtures, got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Round-4 FIX regression: `type Foo = trybuild::TestCases;` emits
/// `discovery_unrecognized`.** Type aliases of `trybuild::TestCases`
/// previously went silent — no `visit_item_type` override existed —
/// so an adopter writing the alias plus `Foo::new()` would see zero
/// fixtures from their tests with no diagnostic. The fix adds a
/// `visit_item_type` override that flags any `type <ident> = <Path>;`
/// where the trailing path segment is `TestCases`. The visitor does
/// NOT auto-recognize the alias (the spec scope is conservative); the
/// emission directs the operator at `--compat-trybuild-macro` or the
/// canonical-form rewrite.
#[test]
fn type_alias_of_testcases_emits_discovery_unrecognized() {
    let crate_root = corpus("type_alias_unrecognized");
    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "type alias is NOT auto-recognized; expected zero fixtures, got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry expected for the type alias; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("type alias"),
        "detail must mention the type alias scenario; got `{}`",
        entry.detail
    );
    assert!(
        entry.detail.contains("Foo"),
        "detail must name the alias ident; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/trybuild.rs"),
        "file must point at the corpus tests/trybuild.rs; got {}",
        entry.file.display()
    );
}

/// **Round-5 BLOCK regression: `use trybuild::TestCases;` inside an
/// inline `mod a { ... }` does NOT leak into sibling `mod b { ... }`.**
/// `imported_testcases` and `aliased_testcases` were previously
/// file-scope BTreeSets — a `use trybuild::TestCases;` inside one
/// module would silently populate the set for the rest of the file,
/// causing a `TestCases::new()` inside a sibling module to be treated
/// as recognized even though `TestCases` is not in scope there. This
/// false positive would surface a phantom fixture from the sibling
/// module's call.
///
/// The fix adds a `visit_item_mod` override that mirrors the
/// save/restore pattern used by `visit_item_fn` for `local_bindings`:
/// `imported_testcases` and `aliased_testcases` are taken into local
/// variables on entry, the inline module is walked with the cleared
/// (empty) sets, and the saved sets are restored on exit. File-level
/// `use` statements (those outside any `mod` block) continue to work
/// because they execute on the file's outer scope before any
/// `visit_item_mod` runs.
#[test]
fn imported_testcases_does_not_leak_across_modules() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tests.join("ui").join("bar.rs"), "fn main() {}\n").unwrap();

    // `mod a` uses `trybuild::TestCases`; its `ui_inside_a` should be
    // recognized. `mod b` does NOT import `TestCases`; its
    // `ui_inside_b` call (which also writes `TestCases::new()`) must
    // NOT be recognized. Previously the file-scope set leaked from
    // `mod a` into `mod b`, falsely recognizing the sibling call.
    let source = "\
mod a {\n\
    use trybuild::TestCases;\n\
    #[test]\n\
    fn ui_inside_a() {\n\
        let t = TestCases::new();\n\
        t.compile_fail(\"tests/ui/foo.rs\");\n\
    }\n\
}\n\
mod b {\n\
    // `TestCases` is NOT in scope here — the leak fix must keep the\n\
    // call below unrecognized.\n\
    #[test]\n\
    fn ui_inside_b() {\n\
        let t = TestCases::new();\n\
        t.compile_fail(\"tests/ui/bar.rs\");\n\
    }\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    // Only `mod a`'s recognized call must surface as a fixture.
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from `mod a`'s `ui_inside_a`; the sibling \
         `mod b` import-leak case must NOT surface a phantom fixture. got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.fixtures[0].relative_path.ends_with("tests/ui/foo.rs"),
        "the recognized fixture must be `mod a`'s foo.rs; got {}",
        out.fixtures[0].relative_path
    );
    assert_eq!(
        out.fixtures[0].call_site.enclosing_test_fn.as_deref(),
        Some("ui_inside_a"),
    );
}

/// **`use trybuild::TestCases as Foo;` inside a `mod a` does NOT leak
/// the alias into `mod b`.** The companion to
/// `imported_testcases_does_not_leak_across_modules` — `aliased_testcases`
/// is also a per-file set that the round-5 fix scopes to the inline
/// module. Before the fix, a `use trybuild::TestCases as Foo;` in `mod a`
/// would populate the file-scope `aliased_testcases`, causing a
/// `Foo::new(); t.compile_fail(...)` chain in `mod b` to emit a
/// spurious `discovery_unrecognized` entry (since `Foo` is not actually
/// in scope in `mod b`).
///
/// The test asserts: zero fixtures (no rename is registered via flag),
/// and zero unrecognized entries for the sibling module's
/// `Foo::new()` chain — `mod b`'s `Foo` is just an unknown identifier
/// from the visitor's POV, not an aliased TestCases receiver.
#[test]
fn aliased_testcases_does_not_leak_across_modules() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    let source = "\
mod a {\n\
    use trybuild::TestCases as Foo;\n\
    #[test]\n\
    fn ui_inside_a() {\n\
        let t = Foo::new();\n\
        t.compile_fail(\"tests/ui/foo.rs\");\n\
    }\n\
}\n\
mod b {\n\
    // `Foo` is NOT in scope here; a Foo::new() chain must NOT be\n\
    // classified as an aliased-TestCases receiver. The visitor\n\
    // would have falsely emitted an unrecognized entry for this\n\
    // sibling call before the per-module scoping fix.\n\
    #[test]\n\
    fn ui_inside_b() {\n\
        let t = Foo::new();\n\
        t.compile_fail(\"tests/ui/bar.rs\");\n\
    }\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");

    // `mod a` produces exactly one unrecognized entry (alias not
    // registered via flag — the round-3 behavior). `mod b`'s sibling
    // call must NOT add a second one — if the alias set leaked, the
    // sibling would also emit `unrecognized`.
    assert!(
        out.fixtures.is_empty(),
        "no aliases are registered; expected zero fixtures. got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry from `mod a`'s aliased call. \
         A second entry from `mod b` would indicate the alias set leaked. got {:?}",
        out.unrecognized
    );
    // The surviving entry must reside inside `mod a` (the call site is
    // in tests/trybuild.rs; line resides in the `mod a` body). The
    // crucial assertion is the COUNT — a leak would double the count.
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("alias"),
        "the entry should reference the alias scenario; got `{}`",
        entry.detail
    );
}

/// **Round-5 BLOCK regression: `#[cfg(...)]`-gated `#[test]` functions
/// emit `discovery_unrecognized` and do NOT contribute fixtures.** A
/// `#[cfg(feature = "foo")] #[test] fn ui() { ... trybuild call ... }`
/// is unevaluable at AST time — the cfg's truth value depends on
/// `--features` at `cargo build` time. The visitor previously descended
/// into the body and surfaced the trybuild call as an active fixture,
/// producing a phantom entry whenever the feature was disabled.
///
/// The fix: any function carrying `#[cfg]` or `#[cfg_attr]` is recorded
/// as `discovery_unrecognized` (detail names the function and mentions
/// `cfg`) and its body is NOT descended. Adjacent un-gated functions
/// remain unaffected — the corpus fixture below has both
/// `#[cfg(feature = "foo")] fn ui` (gated) and `#[test] fn ui_always`
/// (un-gated); the test asserts exactly one fixture (from `ui_always`)
/// and exactly one unrecognized entry (from `ui`).
#[test]
fn cfg_gated_function_emits_discovery_unrecognized() {
    let crate_root = corpus("cfg_gated_fn_unrecognized");
    let out = discover(&crate_root, &[]).expect("discover succeeds");

    // Only `ui_always`'s `bar.rs` fixture must surface — the cfg-gated
    // `ui` function's body must NOT contribute.
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from the un-gated `ui_always`; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.fixtures[0].relative_path.ends_with("tests/ui/bar.rs"),
        "the surviving fixture must be `ui_always`'s `bar.rs`; got {}",
        out.fixtures[0].relative_path
    );
    assert_eq!(
        out.fixtures[0].call_site.enclosing_test_fn.as_deref(),
        Some("ui_always"),
        "the un-gated call must report its enclosing `#[test] fn ui_always`",
    );

    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry from the cfg-gated `ui`; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("cfg"),
        "detail must mention `cfg`; got `{}`",
        entry.detail
    );
    assert!(
        entry.detail.contains("ui"),
        "detail must name the cfg-gated function `ui`; got `{}`",
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

/// **`#[cfg_attr(...)]` on a function also emits `discovery_unrecognized`.**
/// The cfg-gating check is on both `#[cfg(...)]` and `#[cfg_attr(...)]` —
/// the latter can conditionally apply a `#[test]` attribute, so a
/// trybuild call inside the body is just as unevaluable. The detail
/// must distinguish `cfg_attr` from `cfg` so the operator can map back
/// to the source.
#[test]
fn cfg_attr_gated_function_emits_discovery_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();

    // `#[cfg_attr(feature = "X", test)]` conditionally applies `#[test]`.
    // The cfg's truth value is unevaluable, so the whole function must
    // be unrecognized.
    let source = "\
#[cfg_attr(feature = \"foo\", test)]\n\
fn ui() {\n\
    let t = trybuild::TestCases::new();\n\
    t.compile_fail(\"tests/ui/foo.rs\");\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");
    assert!(
        out.fixtures.is_empty(),
        "the cfg_attr-gated body must NOT contribute fixtures; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry for the cfg_attr-gated function; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("cfg_attr"),
        "detail must mention `cfg_attr` specifically; got `{}`",
        entry.detail
    );
    assert!(
        entry.detail.contains("ui"),
        "detail must name the gated function `ui`; got `{}`",
        entry.detail
    );
}

/// **Round-6 BLOCK regression: `#[cfg(...)]`-gated inline modules emit
/// `discovery_unrecognized` and do NOT contribute fixtures.** A
/// `#[cfg(feature = "x")] mod gated { ... trybuild call ... }` is
/// unevaluable at AST time — the cfg's truth value depends on
/// `--features` at `cargo build` time. The visitor previously descended
/// into the body and surfaced the trybuild call as an active fixture,
/// producing a phantom entry whenever the feature was disabled.
///
/// The fix: any inline module carrying `#[cfg]` or `#[cfg_attr]` is
/// recorded as `discovery_unrecognized` (detail names the module and
/// mentions `cfg`) and its body is NOT descended — mirroring the round-5
/// `visit_item_fn` / `visit_impl_item_fn` fix.
///
/// The test pairs a `#[cfg(feature = "x")] mod gated { ... }` with a
/// sibling un-gated module so the assertion validates both halves of
/// the contract: zero fixtures from `gated`, one fixture from the
/// sibling. Adjacent un-gated modules remain unaffected — the fix is
/// per-module.
#[test]
fn cfg_gated_module_emits_discovery_unrecognized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let crate_root = tmp.path().to_path_buf();
    let tests = crate_root.join("tests");
    std::fs::create_dir_all(tests.join("ui")).unwrap();
    std::fs::write(tests.join("ui").join("foo.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tests.join("ui").join("bar.rs"), "fn main() {}\n").unwrap();

    // `gated` is `#[cfg(feature = "x")]`; its `ui` test must NOT
    // contribute a fixture (the feature may be disabled at build time).
    // `always` is un-gated; its `ui_always` test must contribute one
    // fixture as usual.
    let source = "\
#[cfg(feature = \"x\")]\n\
mod gated {\n\
    #[test]\n\
    fn ui() {\n\
        let t = trybuild::TestCases::new();\n\
        t.compile_fail(\"tests/ui/foo.rs\");\n\
    }\n\
}\n\
mod always {\n\
    #[test]\n\
    fn ui_always() {\n\
        let t = trybuild::TestCases::new();\n\
        t.compile_fail(\"tests/ui/bar.rs\");\n\
    }\n\
}\n";
    std::fs::write(tests.join("trybuild.rs"), source).unwrap();

    let out = discover(&crate_root, &[]).expect("discover succeeds");

    // Only the un-gated `always::ui_always` fixture must surface.
    assert_eq!(
        out.fixtures.len(),
        1,
        "exactly one fixture from the un-gated `always` module; got {:?}",
        out.fixtures
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        out.fixtures[0].relative_path.ends_with("tests/ui/bar.rs"),
        "the surviving fixture must be `always::ui_always`'s `bar.rs`; got {}",
        out.fixtures[0].relative_path
    );
    assert_eq!(
        out.fixtures[0].call_site.enclosing_test_fn.as_deref(),
        Some("ui_always"),
        "the un-gated call must report its enclosing `#[test] fn ui_always`",
    );

    assert_eq!(
        out.unrecognized.len(),
        1,
        "exactly one unrecognized entry from the cfg-gated `gated` module; got {:?}",
        out.unrecognized
    );
    let entry = &out.unrecognized[0];
    assert!(
        entry.detail.contains("module"),
        "detail must mention `module`; got `{}`",
        entry.detail
    );
    assert!(
        entry.detail.contains("gated"),
        "detail must name the cfg-gated module `gated`; got `{}`",
        entry.detail
    );
    assert!(
        entry.detail.contains("cfg"),
        "detail must mention `cfg`; got `{}`",
        entry.detail
    );
    assert!(
        entry.file.ends_with("tests/trybuild.rs"),
        "file must point at tests/trybuild.rs; got {}",
        entry.file.display()
    );
    assert!(
        entry.line > 0,
        "line must be 1-indexed and non-zero; got {}",
        entry.line
    );
}

/// **Smoke check on the corpus root.** All scenarios point at
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
        "type_alias_unrecognized",
        "cfg_gated_fn_unrecognized",
    ] {
        let path: &Path = &corpus(scenario);
        assert!(
            path.is_dir(),
            "missing corpus scenario `{scenario}` at {}",
            path.display()
        );
    }
}
