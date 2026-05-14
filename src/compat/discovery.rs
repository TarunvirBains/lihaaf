//! Phase 6 of compat mode — fixture-invocation discovery via syn AST.
//!
//! Implements `docs/compatibility-plan.md` §3.2.1: static AST analysis of
//! every `tests/*.rs` file in the adopter's checkout, surfacing one of
//! three recognized invocation patterns or, when the shape is outside
//! the v0.1 recognized set, a typed `discovery_unrecognized` entry that
//! the §3.3 envelope will carry verbatim.
//!
//! ## Recognized patterns (spec §3.2.1)
//!
//! 1. **Direct `TestCases` calls.** A method-call chain rooted at
//!    `trybuild::TestCases::new()` whose terminal method is `pass` or
//!    `compile_fail`, with a single string-literal argument. The argument
//!    is the fixture path (treated relative to the `tests/*.rs` file's
//!    directory when relative, used verbatim when absolute).
//! 2. **Glob arguments.** When the literal contains `*`, `?`, or `[abc]`
//!    character classes the discovery pass expands it via stdlib
//!    `std::fs` traversal — no `glob` crate dependency. The result is
//!    sorted deterministically by ASCII byte order before being added
//!    to the output. `**` is NOT supported in v0.1 and produces a
//!    `discovery_unrecognized` entry.
//! 3. **`#[test]`-wrapped invocations.** `#[test] fn name() { let t =
//!    trybuild::TestCases::new(); t.compile_fail(<lit>); }`. The
//!    visitor descends into the test function body, tracks single-scope
//!    `let` bindings of `TestCases::new()` (or a custom-macro alias),
//!    and applies pattern 1 to subsequent `.pass()` / `.compile_fail()`
//!    calls on that binding.
//!
//! ## Custom-macro escape hatch
//!
//! `--compat-trybuild-macro <PATH>` (Phase 1 flag, plumbed through
//! [`crate::compat::cli::CompatArgs::compat_trybuild_macro`]) accepts a
//! fully-qualified path that the visitor treats as an alias for
//! `trybuild::TestCases::new()`. Multiple flags are OR'd. The constraint
//! the spec carries forward is that the alias must end in a no-argument
//! constructor (`::new()` shape); the visitor matches against the path
//! string verbatim.
//!
//! `use trybuild::TestCases as Foo;` aliases ARE detected at the
//! `ItemUse` level: the visitor records every rename whose source
//! ident is `TestCases` into a per-file `aliased_testcases` set. When
//! a subsequent `Foo::new()` (or `let t = Foo::new(); t.<method>(...)`)
//! call site uses the alias, the visitor emits a
//! `discovery_unrecognized` entry naming the file/line so the operator
//! sees the misconfiguration. The emission only fires when the local
//! name is NOT registered via `--compat-trybuild-macro`; adopters
//! silence the warning either by registering the local name (e.g.
//! `--compat-trybuild-macro Foo`) or by avoiding the rename and using
//! the canonical `trybuild::TestCases::new()` form at the call site.
//!
//! ## Macro-generated invocations
//!
//! Calls produced by an unexpanded macro (`make_tests!()` that expands
//! to a `TestCases::new().pass(...)` chain at compile time) are NOT
//! recognized — the discovery pass operates on the source AST, not on
//! the post-expansion token tree. Module-level macro invocations
//! (`Item::Macro`, e.g. `make_tests!();` at file scope in a
//! `tests/*.rs`) surface as one `discovery_unrecognized` entry naming
//! the macro path + file + line; discovery does not abort and
//! continues with the rest of the file. Macros at expression positions
//! inside function bodies are NOT flagged — `assert_eq!`, `println!`,
//! and similar are pervasive and would produce noise; adopters with
//! macro-wrapped trybuild constructors at expression position register
//! the wrapper via `--compat-trybuild-macro` instead.
//!
//! ## Determinism
//!
//! For the enumerated patterns two runs from clean state produce the
//! same fixture vector: paths are canonicalized to repo-relative
//! forward-slash strings and sorted by ASCII byte order. Unrecognized
//! entries are sorted by `(file, line)` ASCII. The test suite asserts
//! byte-equal `Debug` output across runs.
//!
//! ## Path-matching rules
//!
//! Match is purely syntactic — there is no type resolution. The visitor
//! recognizes:
//!
//! - `trybuild::TestCases::new()` (canonical form).
//! - `TestCases::new()` at the call site WHEN paired with a
//!   `use trybuild::TestCases;` (no-rename) import in the same file.
//!   The `ItemUse` walker records the local name into a per-file
//!   `imported_testcases` set, then `is_testcases_constructor_path`
//!   accepts the 2-segment `<local>::new` form. Strict prefix match
//!   (`["trybuild"]` only): a `use somelib::TestCases;` is NOT
//!   recognized because the visitor cannot prove the re-export
//!   points at trybuild's `TestCases`.
//! - `<alias>::new()` for any `alias` registered via
//!   `--compat-trybuild-macro`. The alias must be the literal path
//!   string passed on the flag; segment-by-segment equality is required.
//! - The leading `::` form (`::trybuild::TestCases::new()`) is NOT
//!   recognized in v0.1. The spec's canonical form omits the leading
//!   separator and accepting it would broaden the match surface without
//!   improving determinism. There is no `--compat-trybuild-macro`
//!   workaround for this in v0.1: alias-flag values are parsed by
//!   splitting on `::` and dropping empty segments, and the matcher
//!   itself rejects every path whose `leading_colon` is set — so even
//!   a `--compat-trybuild-macro ::trybuild::TestCases` registration
//!   would not match a leading-`::` call site. Adopters writing the
//!   absolute form must rewrite the call site to the canonical form
//!   (or open a v0.2 spec discussion).
//! - `crate::TestCases::new()` (re-exported locally) is NOT recognized;
//!   register via `--compat-trybuild-macro`.
//!
//! ## `#[cfg]` / `#[cfg_attr]` gates
//!
//! A function with ANY `#[cfg(...)]` or `#[cfg_attr(...)]` attribute is
//! conservatively recorded as `discovery_unrecognized` and its body is
//! NOT descended into. The cfg's truth value depends on the adopter's
//! `--features` selection at `cargo build` time; the discovery walk
//! does not resolve cfgs, and silently descending would produce a
//! phantom fixture whenever the feature is disabled. Under-discovery
//! (with operator visibility through the `discovery_unrecognized`
//! emission) is the safe failure mode.
//!
//! ## Known limitations (v0.1)
//!
//! ### Block-scoped shadowing inside a `#[test]` body
//!
//! `local_bindings` and `aliased_bindings` are FLAT `BTreeSet`s
//! per-function — they do NOT model lexical block scope. So:
//!
//! - A nested `{ let t = 1; }` (where `t` was previously a trybuild
//!   receiver in the enclosing scope) permanently REMOVES `t` from
//!   `local_bindings` for the rest of the function. Subsequent
//!   `t.compile_fail(...)` calls on the original outer `t` are no
//!   longer recognized.
//! - Conversely, a binding inserted in a nested block leaks back to
//!   the outer scope after the block ends. The visitor cannot tell
//!   the block boundary from a same-scope statement sequence, so the
//!   inner `let` is treated as a same-scope rebind.
//!
//! **Workaround:** keep trybuild bindings at the function's top-level
//! scope and avoid shadowing the binding ident inside nested blocks.
//! Use distinct identifiers (e.g. `let t = TestCases::new();` at the
//! top and `let unrelated = 1;` inside any nested block) so the
//! visitor's flat set tracks the right shape.
//!
//! ### Pattern-3 binders: only `let`
//!
//! The pattern-3 visitor tracks `let <ident> = <constructor>;`
//! exclusively. Other binders are NOT modeled:
//!
//! - `for t in ... { /* t.compile_fail(...) */ }` (loop binder) —
//!   the iteration variable is not recognized as a trybuild receiver.
//! - `match value { Some(t) => ... }` (pattern binder) — pattern-
//!   destructured idents are not tracked.
//! - `if let Some(t) = ...` (irrefutable-pattern binder) — same as
//!   `match`.
//! - `|t| { t.compile_fail(...) }` (closure parameter) — closure
//!   params are not tracked.
//!
//! Trybuild calls in any of these contexts may produce false negatives
//! (silently dropped) when the binder is the trybuild receiver, or
//! false positives if the ident collides with an outer-scope binding
//! that the flat set is tracking.
//!
//! **Workaround:** keep trybuild calls at the function's top-level
//! scope; do not bind a `TestCases::new()` constructor through a
//! `for` / `match` / `if let` / closure parameter. The canonical
//! shapes (top-level `let t = TestCases::new(); t.compile_fail(...)`
//! and direct-chain `TestCases::new().compile_fail(...)`) are what
//! trybuild-style adopters use in practice and are what v0.1 covers.
//!
//! ### Why this isn't fixed in v0.1
//!
//! The proper fix is a per-function scope-stack data structure
//! (`Vec<BTreeSet<String>>` with push/pop on every `Block`,
//! `ExprBlock`, `ExprClosure`, `ExprMatch` arm, `ExprForLoop`,
//! `ExprIfLet`, etc.). That is a substantial Phase 6 architectural
//! expansion — non-trivial both to implement and to test
//! exhaustively across every binder variant — and the trybuild-style
//! call patterns the v0.1 spec scopes against do not exercise these
//! shapes. The flat set is a deliberately conservative v0.1 contract;
//! the limitation is documented here and revisited in a v0.2 spec
//! discussion if real-world adopter code surfaces the false-negative
//! case.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

use crate::error::Error;
use crate::util;

/// Pass / compile_fail dichotomy mirrored from
/// `crate::discovery::FixtureKind`, kept compat-local so the compat
/// driver's report writer does not couple to the dispatch-side
/// discovery module.
///
/// Marked `pub` (not `pub(crate)`) so the `#[doc(hidden)]` re-export at
/// the crate root can surface the type into the integration-test
/// crate. The compat-mode public-API contract is still that adopters
/// drive lihaaf via `cargo lihaaf --compat`, not via the Rust surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    /// `.pass(...)` — the fixture is expected to compile cleanly.
    Pass,
    /// `.compile_fail(...)` — the fixture is expected to fail with a
    /// snapshot-checked diagnostic.
    CompileFail,
}

impl From<crate::discovery::FixtureKind> for FixtureKind {
    fn from(k: crate::discovery::FixtureKind) -> Self {
        match k {
            crate::discovery::FixtureKind::CompilePass => Self::Pass,
            crate::discovery::FixtureKind::CompileFail => Self::CompileFail,
        }
    }
}

/// One discovered Trybuild fixture call. `pub` for the same reason as
/// [`FixtureKind`] — the `#[doc(hidden)]` re-export at the crate root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFixture {
    /// Absolute path to the `.rs` fixture file.
    pub fixture_path: PathBuf,
    /// Repo-relative, forward-slash form. Used as the §3.3 envelope
    /// key and as the sort key for cross-run determinism.
    pub relative_path: String,
    /// Pass or compile_fail, derived from which `TestCases` method was
    /// invoked at the call site.
    pub kind: FixtureKind,
    /// The test file the trybuild call appeared in.
    pub call_site: CallSite,
}

/// Source citation for a discovered call. `pub` for the same reason as
/// [`FixtureKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Absolute path to the `tests/*.rs` file containing the call.
    pub file: PathBuf,
    /// Line number (1-indexed) where the trybuild method was invoked.
    pub line: usize,
    /// The Rust test function name when the call was inside a
    /// `#[test]`-wrapped function; `None` for top-level invocations.
    pub enclosing_test_fn: Option<String>,
}

/// One AST node that did not match the v0.1 recognized set. `pub` for
/// the same reason as [`FixtureKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryUnrecognized {
    /// File containing the unrecognized node.
    pub file: PathBuf,
    /// Line number (1-indexed) of the node.
    pub line: usize,
    /// Short human-readable description of why the node was
    /// unrecognized (e.g. `non-literal argument to .pass()`,
    /// `parse_failed`, `glob ** not supported in v0.1`).
    pub detail: String,
}

/// Discovery walk result. `pub` for the same reason as
/// [`FixtureKind`].
#[derive(Debug, Clone)]
pub struct DiscoveryOutput {
    /// Fixtures found, sorted by `relative_path` ASCII byte order.
    pub fixtures: Vec<DiscoveredFixture>,
    /// Unrecognized AST shapes, sorted by `(file, line)` ASCII.
    pub unrecognized: Vec<DiscoveryUnrecognized>,
}

/// Walk every `tests/*.rs` file in `crate_root/tests/` (flat — no
/// recursion into `tests/<subdir>/`) and run the AST visitor.
///
/// `custom_macros` is the parsed `--compat-trybuild-macro` arguments: a
/// list of fully-qualified paths the visitor treats as aliases for
/// `trybuild::TestCases::new()`. Order is irrelevant; duplicates are
/// idempotent.
///
/// When `crate_root/tests/` does not exist (a target crate with no
/// integration tests at all), the result is an empty output — neither
/// fixtures nor unrecognized entries. This matches the spec's
/// "discovery does not abort on the empty case" invariant.
///
/// A parse failure on any individual file produces a single
/// `discovery_unrecognized` entry with `detail = "parse_failed: <msg>"`
/// and the walk continues to the next file. The §3.2.1 contract is
/// that discovery is best-effort across files; one malformed file does
/// not stop the rest.
pub fn discover(crate_root: &Path, custom_macros: &[String]) -> Result<DiscoveryOutput, Error> {
    let tests_dir = crate_root.join("tests");
    let mut fixtures: Vec<DiscoveredFixture> = Vec::new();
    let mut unrecognized: Vec<DiscoveryUnrecognized> = Vec::new();

    let test_files = match list_top_level_test_files(&tests_dir) {
        Ok(v) => v,
        Err(e) => match e {
            ListError::DoesNotExist => Vec::new(),
            ListError::Io(err) => return Err(err),
        },
    };

    let alias_set: Vec<&str> = custom_macros.iter().map(String::as_str).collect();

    for test_file in test_files {
        let source = match std::fs::read_to_string(&test_file) {
            Ok(s) => s,
            Err(e) => {
                return Err(Error::io(
                    e,
                    "reading test file for compat discovery",
                    Some(test_file.clone()),
                ));
            }
        };
        let ast = match syn::parse_file(&source) {
            Ok(ast) => ast,
            Err(parse_err) => {
                // §3.2.1: a malformed file produces one unrecognized
                // entry. The error's span line is approximate (some syn
                // parse failures point at the next token); we render
                // line 1 when no span is available.
                let line = parse_err.span().start().line.max(1);
                unrecognized.push(DiscoveryUnrecognized {
                    file: test_file.clone(),
                    line,
                    detail: format!("parse_failed: {parse_err}"),
                });
                continue;
            }
        };

        let mut visitor = DiscoveryVisitor::new(&test_file, &alias_set);
        visitor.visit_file(&ast);

        // Pattern 2 expansion: each visitor hit carries a (kind,
        // literal, call_site) triple. Resolve to one-or-more concrete
        // fixture paths. Glob errors surface as unrecognized entries
        // rather than aborting discovery.
        for hit in visitor.hits {
            match resolve_literal_to_fixtures(crate_root, &test_file, &hit.literal) {
                Ok(paths) => {
                    if paths.is_empty() {
                        // A literal that resolves to zero matches —
                        // either a missing file (literal) or a glob
                        // with no matches. Surface as unrecognized so
                        // the operator sees the divergence; discovery
                        // does not abort.
                        unrecognized.push(DiscoveryUnrecognized {
                            file: hit.call_site.file.clone(),
                            line: hit.call_site.line,
                            detail: format!(
                                "literal `{}` resolved to zero fixture paths",
                                hit.literal
                            ),
                        });
                    }
                    for path in paths {
                        let relative_path = relative_repo_path(crate_root, &path);
                        fixtures.push(DiscoveredFixture {
                            fixture_path: path,
                            relative_path,
                            kind: hit.kind,
                            call_site: hit.call_site.clone(),
                        });
                    }
                }
                Err(detail) => {
                    unrecognized.push(DiscoveryUnrecognized {
                        file: hit.call_site.file.clone(),
                        line: hit.call_site.line,
                        detail,
                    });
                }
            }
        }
        unrecognized.extend(visitor.unrecognized);
    }

    fixtures.sort_by(|a, b| a.relative_path.as_bytes().cmp(b.relative_path.as_bytes()));
    fixtures.dedup_by(|a, b| {
        a.relative_path == b.relative_path && a.kind == b.kind && a.call_site == b.call_site
    });

    unrecognized.sort_by(|a, b| {
        let af = a.file.as_os_str().as_encoded_bytes();
        let bf = b.file.as_os_str().as_encoded_bytes();
        af.cmp(bf)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.detail.as_bytes().cmp(b.detail.as_bytes()))
    });

    Ok(DiscoveryOutput {
        fixtures,
        unrecognized,
    })
}

/// Repo-relative forward-slash form. Falls back to the absolute path
/// stringified verbatim when the fixture lives outside the crate root
/// (the user passed an absolute literal pointing elsewhere — rare).
fn relative_repo_path(crate_root: &Path, absolute: &Path) -> String {
    util::relative_to(absolute, crate_root)
}

/// Outcome of the tests-directory listing helper. `DoesNotExist` is
/// significant in its own right — Phase 6 treats "no `tests/`" as a
/// well-defined empty discovery, not an error.
enum ListError {
    DoesNotExist,
    Io(Error),
}

/// List every `<crate_root>/tests/<name>.rs` file (flat — not
/// recursive). Subdirectories under `tests/` are skipped per spec
/// §3.2.1 ("the discovery pass walks every `tests/*.rs` file"); the
/// flat shape is the trybuild convention.
fn list_top_level_test_files(tests_dir: &Path) -> Result<Vec<PathBuf>, ListError> {
    let entries = match std::fs::read_dir(tests_dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ListError::DoesNotExist);
        }
        Err(e) => {
            return Err(ListError::Io(Error::io(
                e,
                "reading tests/ directory for compat discovery",
                Some(tests_dir.to_path_buf()),
            )));
        }
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            ListError::Io(Error::io(
                e,
                "reading tests/ directory entry for compat discovery",
                Some(tests_dir.to_path_buf()),
            ))
        })?;
        let ft = entry.file_type().map_err(|e| {
            ListError::Io(Error::io(
                e,
                "stat-ing tests/ directory entry for compat discovery",
                Some(entry.path()),
            ))
        })?;
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        files.push(path);
    }
    files.sort_by(|a, b| {
        a.as_os_str()
            .as_encoded_bytes()
            .cmp(b.as_os_str().as_encoded_bytes())
    });
    Ok(files)
}

/// One recognized visitor hit before glob expansion.
struct VisitorHit {
    kind: FixtureKind,
    literal: String,
    call_site: CallSite,
}

/// Single-pass AST visitor. State mutates through `&mut self`; the
/// `syn::visit::Visit<'ast>` trait in syn 2 takes `&mut self` on every
/// override (verified by direct implementation — no `RefCell` needed).
struct DiscoveryVisitor<'a> {
    /// Absolute path of the file being walked. Read-only for the
    /// duration of `visit_file`; stamped onto every recognized hit.
    current_file: &'a Path,
    /// Pre-computed segment vectors for each `--compat-trybuild-macro`
    /// alias, plus a parallel vector with `::new` appended. Splitting
    /// once at visitor construction lets the hot path compare against
    /// borrowed `&str` slices instead of allocating a fresh `String`
    /// per `Expr::Path` node.
    alias_segments: Vec<Vec<String>>,
    alias_with_new_segments: Vec<Vec<String>>,

    /// Name of the enclosing `#[test]` function, or `None` for
    /// top-level / non-`#[test]` calls.
    enclosing_test_fn: Option<String>,
    /// Per-function bindings: each entry records an identifier `t`
    /// from `let t = trybuild::TestCases::new();` (or an alias's
    /// `new()`) seen inside the current `#[test]` body. Cleared on
    /// function exit so cross-function tracking is impossible by
    /// construction.
    local_bindings: BTreeSet<String>,

    /// File-scope: local names introduced by
    /// `use trybuild::TestCases as <name>;` (or `use <registered-alias-path> as <name>;`)
    /// where the local `<name>` is NOT itself registered via
    /// `--compat-trybuild-macro`. The visitor uses this set to flag
    /// `<name>::new()` / `let t = <name>::new(); t.compile_fail(...)`
    /// call shapes as `discovery_unrecognized` so adopters know to
    /// register the alias.
    aliased_testcases: BTreeSet<String>,
    /// Per-function bindings rooted in an `aliased_testcases` entry.
    /// `let t = Foo::new();` populates this when `Foo` is in
    /// `aliased_testcases`. Tracked separately from `local_bindings`
    /// so the terminal-call path can distinguish "recognized alias
    /// binding" from "unregistered alias binding".
    aliased_bindings: BTreeSet<String>,

    /// File-scope: local names introduced by `use trybuild::TestCases;`
    /// (the no-rename form). The local name equals `TestCases` in the
    /// canonical case — `use trybuild::{TestCases};` and
    /// `use trybuild::TestCases;` both produce `"TestCases"` here. The
    /// `<local_name>::new()` call at the use site is then RECOGNIZED
    /// (not flagged as unrecognized) — this is the most common
    /// trybuild import idiom and silently dropping it would be a
    /// pervasive false negative.
    ///
    /// Only the strict `prefix == ["trybuild"]` shape feeds this set —
    /// a hypothetical `use somelib::TestCases;` is intentionally NOT
    /// recognized because the visitor cannot prove the re-export points
    /// at trybuild's `TestCases`.
    imported_testcases: BTreeSet<String>,

    /// Recognized hits, awaiting glob expansion.
    hits: Vec<VisitorHit>,
    /// Unrecognized AST nodes (e.g. `t.pass(format!(...))`).
    unrecognized: Vec<DiscoveryUnrecognized>,
}

impl<'a> DiscoveryVisitor<'a> {
    fn new(current_file: &'a Path, custom_macros: &'a [&'a str]) -> Self {
        let alias_segments: Vec<Vec<String>> = custom_macros
            .iter()
            .map(|alias| {
                alias
                    .split("::")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .collect();
        let alias_with_new_segments: Vec<Vec<String>> = alias_segments
            .iter()
            .map(|segs| {
                let mut v = segs.clone();
                v.push("new".to_string());
                v
            })
            .collect();
        Self {
            current_file,
            alias_segments,
            alias_with_new_segments,
            enclosing_test_fn: None,
            local_bindings: BTreeSet::new(),
            aliased_testcases: BTreeSet::new(),
            aliased_bindings: BTreeSet::new(),
            imported_testcases: BTreeSet::new(),
            hits: Vec::new(),
            unrecognized: Vec::new(),
        }
    }

    /// Match `expr` against the recognized "this is a TestCases receiver"
    /// shape. Returns `true` for:
    ///
    /// - `trybuild::TestCases::new()` (canonical literal form).
    /// - `<alias>::new()` for any alias registered via `custom_macros`.
    /// - An `ExprMethodCall` whose receiver chain ultimately roots at
    ///   one of the above (chained `.pass(...).compile_fail(...)`).
    /// - An `ExprPath` naming a local binding recorded by
    ///   [`Self::local_bindings`] (pattern 3, `#[test]`-wrapped).
    fn receiver_is_testcases(&self, expr: &syn::Expr) -> bool {
        match expr {
            // `trybuild::TestCases::new()` and aliases.
            syn::Expr::Call(call) => {
                let func = &*call.func;
                self.is_testcases_constructor_path(func) && call.args.is_empty()
            }
            // Chained method calls: receiver is itself a `.foo()` call;
            // descend to the leftmost root.
            syn::Expr::MethodCall(inner) => self.receiver_is_testcases(&inner.receiver),
            // Local binding from `let t = TestCases::new();`.
            syn::Expr::Path(path_expr) => {
                if path_expr.attrs.is_empty()
                    && path_expr.qself.is_none()
                    && let Some(ident) = path_expr.path.get_ident()
                {
                    return self.local_bindings.contains(&ident.to_string());
                }
                false
            }
            // Reference (`&t.pass(...)`), parenthesized expression,
            // group — descend through transparent wrappers.
            syn::Expr::Reference(r) => self.receiver_is_testcases(&r.expr),
            syn::Expr::Paren(p) => self.receiver_is_testcases(&p.expr),
            syn::Expr::Group(g) => self.receiver_is_testcases(&g.expr),
            _ => false,
        }
    }

    /// Returns `true` when `expr` is a path call to
    /// `trybuild::TestCases::new` or `<alias>::new` (custom-macro
    /// alias's `::new` form), or to a no-rename-imported
    /// `<local>::new` recorded by [`Self::imported_testcases`].
    fn is_testcases_constructor_path(&self, expr: &syn::Expr) -> bool {
        let syn::Expr::Path(p) = expr else {
            return false;
        };
        // Reject explicit-self (`<T>::new`) and attribute-decorated
        // paths to keep the match surface tight.
        if p.qself.is_some() || !p.attrs.is_empty() {
            return false;
        }
        // Canonical form. Leading `::` is rejected — `path_matches_segments`
        // enforces `leading_colon.is_none()` to mirror the v0.1 spec.
        if path_matches_segments(&p.path, &["trybuild", "TestCases", "new"]) {
            return true;
        }
        // No-rename import form: `use trybuild::TestCases;` makes the
        // local name `TestCases` (or whatever the segment was if the
        // adopter used a group-import) refer to the canonical trybuild
        // type. `<local>::new()` at the call site is a 2-segment path.
        if p.path.leading_colon.is_none() && p.path.segments.len() == 2 {
            let first = p.path.segments[0].ident.to_string();
            if self.imported_testcases.contains(&first) && p.path.segments[1].ident == "new" {
                return true;
            }
        }
        // Alias form: an entry in `custom_macros` is matched against
        // its `::new` suffix. The flag accepts the constructor's
        // owning path (e.g. `mycrate::ui_tests`); the actual call site
        // is `mycrate::ui_tests::new()` or, for adopter convenience,
        // `mycrate::ui_tests()` (no `::new`). Recognize both shapes.
        for (alias_segs, with_new_segs) in self
            .alias_segments
            .iter()
            .zip(self.alias_with_new_segments.iter())
        {
            if path_matches_string_segments(&p.path, with_new_segs)
                || path_matches_string_segments(&p.path, alias_segs)
            {
                return true;
            }
        }
        false
    }

    /// Pull the call-site span line off the method ident.
    fn make_call_site(&self, method: &syn::Ident) -> CallSite {
        CallSite {
            file: self.current_file.to_path_buf(),
            line: method.span().start().line.max(1),
            enclosing_test_fn: self.enclosing_test_fn.clone(),
        }
    }

    /// Extract the single string-literal argument from a method call.
    /// Returns `None` when the call has zero args, more than one arg,
    /// or a non-literal first arg.
    fn extract_string_literal_arg(node: &syn::ExprMethodCall) -> Option<String> {
        if node.args.len() != 1 {
            return None;
        }
        let arg = node.args.first()?;
        match arg {
            syn::Expr::Lit(lit) if lit.attrs.is_empty() => match &lit.lit {
                syn::Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Try to record one of the recognized terminal method calls
    /// (`.pass` / `.compile_fail`). Returns whether the node was
    /// consumed (either as a hit or as an unrecognized entry); the
    /// `visit_*` callers use this to decide whether to keep descending.
    fn try_record_terminal_call(&mut self, node: &syn::ExprMethodCall) -> bool {
        let method_str = node.method.to_string();
        let kind = match method_str.as_str() {
            "pass" => FixtureKind::Pass,
            "compile_fail" => FixtureKind::CompileFail,
            _ => return false,
        };
        if !self.receiver_is_testcases(&node.receiver) {
            // The receiver isn't registered, but it MAY be an
            // unregistered `use trybuild::TestCases as Foo;` alias.
            // Surface that case as `discovery_unrecognized` rather than
            // silently dropping the call — adopters need to know to
            // register the alias via `--compat-trybuild-macro`.
            if self.receiver_is_aliased_testcases(&node.receiver) {
                let call_site = self.make_call_site(&node.method);
                self.unrecognized.push(DiscoveryUnrecognized {
                    file: call_site.file,
                    line: call_site.line,
                    detail: format!(
                        ".{method_str}() called via an unregistered `use ... as` alias — \
                         register the originating constructor path via \
                         --compat-trybuild-macro so discovery can match the call site"
                    ),
                });
                return true;
            }
            return false;
        }
        let call_site = self.make_call_site(&node.method);
        match Self::extract_string_literal_arg(node) {
            Some(literal) => {
                self.hits.push(VisitorHit {
                    kind,
                    literal,
                    call_site,
                });
            }
            None => {
                self.unrecognized.push(DiscoveryUnrecognized {
                    file: call_site.file,
                    line: call_site.line,
                    detail: format!(
                        "non-literal or multi-argument call to .{method_str}() — \
                         only `<TestCases>.{method_str}(\"<path>\")` is recognized in v0.1"
                    ),
                });
            }
        }
        true
    }
}

impl<'ast, 'a> Visit<'ast> for DiscoveryVisitor<'a> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        // Round-5 BLOCK fix: `#[cfg(...)]`-gated functions cannot be
        // evaluated at AST time — the cfg's truth value depends on the
        // adopter's `--features` selection at `cargo build`. A function
        // with ANY `#[cfg]` / `#[cfg_attr]` attribute is therefore
        // recorded as `discovery_unrecognized` and its body is NOT
        // descended into. Under-discovery (with operator visibility)
        // is the safe failure mode: a feature-disabled trybuild call
        // would otherwise produce a phantom fixture in the discovery
        // output that the dispatch run could never match.
        if let Some(cfg_attr) = find_cfg_attribute(&node.attrs) {
            let attr_kind = if cfg_attr.meta.path().is_ident("cfg_attr") {
                "cfg_attr"
            } else {
                "cfg"
            };
            let line = node.sig.ident.span().start().line.max(1);
            let fn_name = node.sig.ident.to_string();
            self.unrecognized.push(DiscoveryUnrecognized {
                file: self.current_file.to_path_buf(),
                line,
                detail: format!(
                    "function `{fn_name}` is cfg-gated (`#[{attr_kind}(...)]`); \
                     trybuild discovery cannot evaluate the cfg without resolution. \
                     Treat as unrecognized."
                ),
            });
            // Do NOT descend into the body — recording a binding or
            // hit from a gated function would silently introduce a
            // phantom fixture when the feature is disabled at build
            // time.
            return;
        }

        // Save and reset both the enclosing-test marker and the
        // per-function bindings so cross-function leakage is
        // impossible.
        //
        // Round-6 BLOCK fix (Gemini): `use` declarations inside a
        // function body are LOCAL to that function in Rust — they do
        // NOT leak to sibling functions in the same file. The visitor
        // must therefore save and RESTORE `imported_testcases` and
        // `aliased_testcases` after the body walk. Unlike `visit_item_mod`
        // (which clears the sets on entry so the inner module starts
        // from an empty file-scope set), functions INHERIT the enclosing
        // file's imports — a file-level `use trybuild::TestCases;` IS
        // visible inside every function in that file. The fix is
        // therefore "snapshot before, restore after": any body-local
        // `use` additions are observed during the walk but rolled back
        // on exit so they cannot leak to siblings.
        //
        // Implementation: snapshot via `.clone()` (the sets are
        // typically empty or small — a handful of imports per file),
        // walk, then overwrite the live set with the snapshot. We
        // cannot use `std::mem::take` because that would empty the live
        // set BEFORE the walk, defeating inheritance from the file
        // scope.
        let saved_enclosing = self.enclosing_test_fn.take();
        let saved_bindings = std::mem::take(&mut self.local_bindings);
        let saved_aliased_bindings = std::mem::take(&mut self.aliased_bindings);
        let saved_imported = self.imported_testcases.clone();
        let saved_aliased = self.aliased_testcases.clone();

        if is_test_attribute(&node.attrs) {
            self.enclosing_test_fn = Some(node.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, node);

        self.enclosing_test_fn = saved_enclosing;
        self.local_bindings = saved_bindings;
        self.aliased_bindings = saved_aliased_bindings;
        self.imported_testcases = saved_imported;
        self.aliased_testcases = saved_aliased;
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        // Pattern 3 binding tracker. We only watch for the shape
        // `let <ident> = <TestCases::new()-or-alias>;`. Anything else
        // (typed pattern, destructured tuple, ref binding) is left
        // alone — the binding tracker is strictly opt-in.
        //
        // Shadow-invalidation: a later `let t = something_else();` in
        // the same scope must REMOVE any previous `t` from
        // `local_bindings` / `aliased_bindings` so a downstream
        // `t.compile_fail(...)` is not treated as a trybuild receiver.
        // Without the removal the visitor would silently surface a
        // fixture from the shadowed identifier — a false positive that
        // would survive every snapshot check until a human noticed.
        if let (syn::Pat::Ident(pat_ident), Some(init)) = (&node.pat, &node.init)
            && pat_ident.attrs.is_empty()
            && pat_ident.by_ref.is_none()
            && pat_ident.subpat.is_none()
        {
            let ident = pat_ident.ident.to_string();
            if self.is_testcases_constructor_expr(&init.expr) {
                self.local_bindings.insert(ident.clone());
                // A `let t = TestCases::new();` shadows a previous
                // `let t = Foo::new();` (where `Foo` is an unregistered
                // alias) just as much as the converse — drop the
                // aliased-binding entry so the receiver classifier does
                // not flag a now-recognized binding as unrecognized.
                self.aliased_bindings.remove(&ident);
            } else if self.is_aliased_testcases_constructor_expr(&init.expr) {
                // Pattern 3 for an unregistered `use ... as Foo;` alias:
                // record the binding so a subsequent
                // `t.compile_fail(...)` call can be flagged as
                // unrecognized rather than silently dropped.
                self.aliased_bindings.insert(ident.clone());
                self.local_bindings.remove(&ident);
            } else {
                // A `let t = <non-trybuild>;` rebinds the name to a
                // non-receiver. Drop both possible entries so the
                // subsequent terminal-call dispatcher does not pick
                // up the stale binding.
                self.local_bindings.remove(&ident);
                self.aliased_bindings.remove(&ident);
            }
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // Pattern 1 (and pattern 3 terminal): direct `TestCases::new`
        // chain or local-binding chain.
        self.try_record_terminal_call(node);
        // Descend into the receiver and args so nested chains (e.g.
        // `t.pass("a").compile_fail("b")`) still surface every
        // terminal.
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        // Round-5 BLOCK fix (mirrored from `visit_item_fn`): a
        // `#[cfg(...)]`-gated impl method cannot be evaluated at AST
        // time. Surface as `discovery_unrecognized` and skip the body.
        if let Some(cfg_attr) = find_cfg_attribute(&node.attrs) {
            let attr_kind = if cfg_attr.meta.path().is_ident("cfg_attr") {
                "cfg_attr"
            } else {
                "cfg"
            };
            let line = node.sig.ident.span().start().line.max(1);
            let fn_name = node.sig.ident.to_string();
            self.unrecognized.push(DiscoveryUnrecognized {
                file: self.current_file.to_path_buf(),
                line,
                detail: format!(
                    "function `{fn_name}` is cfg-gated (`#[{attr_kind}(...)]`); \
                     trybuild discovery cannot evaluate the cfg without resolution. \
                     Treat as unrecognized."
                ),
            });
            return;
        }

        // Methods inside `impl Foo { fn bar() { ... } }` get walked
        // via syn's default visitor when this override is absent — and
        // the default visitor does NOT scope `local_bindings`, so a
        // `let t = TestCases::new();` inside an impl method would leak
        // into the enclosing scope's bindings table (or vice-versa).
        // Mirror the save/restore pattern from `visit_item_fn` so each
        // impl method gets its own fresh bindings scope; `#[test]` on
        // impl methods is exceedingly rare but supported uniformly
        // (the same `is_test_attribute` filter applies).
        //
        // Round-6 BLOCK fix (Gemini, mirrored from `visit_item_fn`):
        // also snapshot/restore `imported_testcases` and `aliased_testcases`
        // because `use` declarations inside an impl method body are
        // local to that method in Rust. Use `.clone()` (snapshot) rather
        // than `mem::take` so the method INHERITS file-scope imports
        // during the walk but body-local additions roll back on exit.
        let saved_enclosing = self.enclosing_test_fn.take();
        let saved_bindings = std::mem::take(&mut self.local_bindings);
        let saved_aliased_bindings = std::mem::take(&mut self.aliased_bindings);
        let saved_imported = self.imported_testcases.clone();
        let saved_aliased = self.aliased_testcases.clone();

        if is_test_attribute(&node.attrs) {
            self.enclosing_test_fn = Some(node.sig.ident.to_string());
        }
        syn::visit::visit_impl_item_fn(self, node);

        self.enclosing_test_fn = saved_enclosing;
        self.local_bindings = saved_bindings;
        self.aliased_bindings = saved_aliased_bindings;
        self.imported_testcases = saved_imported;
        self.aliased_testcases = saved_aliased;
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        // Round-6 BLOCK fix: `#[cfg(...)]` / `#[cfg_attr(...)]` on the
        // module itself gates the ENTIRE inline body — every `use`,
        // every `#[test] fn`, every nested module. Without resolving
        // the cfg (which depends on `--features` at `cargo build` time)
        // we cannot descend safely: a `#[cfg(unix)] mod tests { ... }`
        // on Windows must NOT contribute fixtures, and vice-versa.
        //
        // Mirror the `visit_item_fn` / `visit_impl_item_fn` round-5
        // pattern: emit one `discovery_unrecognized` entry naming the
        // module, then skip the walk (do NOT descend into `node.content`).
        // Under-discovery with operator visibility is the safe failure
        // mode — descending silently would produce a phantom fixture
        // whenever the cfg is disabled at adopter build time.
        if let Some(cfg_attr) = find_cfg_attribute(&node.attrs) {
            let attr_kind = if cfg_attr.meta.path().is_ident("cfg_attr") {
                "cfg_attr"
            } else {
                "cfg"
            };
            let line = node.ident.span().start().line.max(1);
            let mod_name = node.ident.to_string();
            self.unrecognized.push(DiscoveryUnrecognized {
                file: self.current_file.to_path_buf(),
                line,
                detail: format!(
                    "module `{mod_name}` is cfg-gated (`#[{attr_kind}(...)]`); \
                     trybuild discovery cannot evaluate the cfg without resolution. \
                     Treat as unrecognized."
                ),
            });
            return;
        }

        // Round-5 BLOCK fix: `imported_testcases` and `aliased_testcases`
        // are FILE-scope sets — populated by `visit_item_use` from
        // `use` statements at file top-level. But inline modules
        // (`mod a { ... }`) can ALSO carry `use` statements; those uses
        // must NOT leak into:
        //   - the enclosing file's scope (siblings of `mod a` should
        //     not see `mod a`'s imports), or
        //   - sibling modules (`mod b { ... }` after `mod a { ... }`).
        //
        // The save/restore pattern mirrors `visit_item_fn`'s scoping
        // of `local_bindings` / `aliased_bindings`. File-level `use`s
        // observed before any `mod` block are preserved across the
        // walk because the saved sets carry them in.
        //
        // syn's default `visit_item_mod` recurses into `node.content`
        // (the inline body, if any) — we let it do the descent after
        // we've cleared the use-import sets to the empty set so the
        // inner `visit_item_use` calls populate this scope only. After
        // the recursion the saved sets are restored.
        let saved_imported = std::mem::take(&mut self.imported_testcases);
        let saved_aliased = std::mem::take(&mut self.aliased_testcases);

        syn::visit::visit_item_mod(self, node);

        self.imported_testcases = saved_imported;
        self.aliased_testcases = saved_aliased;
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        // syn 2.0: `ItemMacro` covers BOTH `macro_rules! name { ... }`
        // definitions AND module-level invocations like `make_tests!();`.
        // The two are distinguished by `node.ident`: a definition carries
        // `Some(name)` (the `example` in `macro_rules! example { ... }`),
        // an invocation carries `None`. Only invocations are unrecognized
        // for §3.2.1 purposes — a `macro_rules!` definition is a local
        // helper and naming it would generate spurious operator noise.
        if node.ident.is_some() {
            syn::visit::visit_item_macro(self, node);
            return;
        }

        // §3.2.1: macro-generated invocations like `make_tests!();` at
        // module level surface as a single `discovery_unrecognized`
        // entry naming the macro's file + line; the visitor cannot
        // expand macros at AST time and so cannot tell whether the
        // macro wraps a trybuild call or does something unrelated.
        // Adopters with macro-wrapped trybuild invocations register
        // the wrapper's expanded constructor path via
        // `--compat-trybuild-macro`.
        let path_str = path_segments_string(&node.mac.path);
        // The bang token (`!`) is the most precise span for the macro
        // invocation; `node.mac.path` would need the `Spanned` trait
        // imported, and the bang sits right next to the macro name.
        let line = node.mac.bang_token.span.start().line.max(1);
        self.unrecognized.push(DiscoveryUnrecognized {
            file: self.current_file.to_path_buf(),
            line,
            detail: format!(
                "macro invocation `{path_str}!` at module level is not a recognized v0.1 \
                 trybuild shape (discovery operates on the source AST, not on \
                 post-expansion tokens)"
            ),
        });
        syn::visit::visit_item_macro(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        // Round-6 fix (Gemini BLOCK-B extension): a `#[cfg(...)]`-gated
        // type alias is NOT materially present when the cfg is disabled
        // at adopter build time. Surfacing `discovery_unrecognized` for
        // an item that does not exist in the compiled crate is
        // noise — the operator cannot act on it. Skip processing
        // entirely; downstream `<alias>::new()` calls inside the same
        // (gated) context will also be silently dropped by the visitor's
        // unknown-shape policy, and a non-gated call to a gated alias
        // would have failed to compile in the first place.
        if find_cfg_attribute(&node.attrs).is_some() {
            return;
        }

        // `type Foo = trybuild::TestCases;` — a type alias is a v0.1
        // scope-out: type resolution is non-syntactic and the visitor
        // operates on the source AST. If the adopter writes a type
        // alias whose RHS ends in `TestCases`, surface a
        // `discovery_unrecognized` entry so the operator knows the
        // 'Foo::new()' call site will be silently dropped. We do NOT
        // auto-recognize the alias (the spec scope is conservative);
        // adopters silence the warning by registering the originating
        // path via `--compat-trybuild-macro` or by avoiding the alias
        // and writing the canonical `trybuild::TestCases::new()` form
        // at the call site.
        //
        // Match shape: the RHS must be a `Type::Path` whose trailing
        // segment is `TestCases`. The prefix is not constrained — the
        // user explicitly wrote `TestCases` so the emission is
        // actionable regardless of the upstream path.
        if let syn::Type::Path(p) = &*node.ty
            && p.qself.is_none()
            && let Some(last) = p.path.segments.last()
            && last.ident == "TestCases"
        {
            let alias_ident = node.ident.to_string();
            let rhs = path_segments_string(&p.path);
            let line = node.ident.span().start().line.max(1);
            self.unrecognized.push(DiscoveryUnrecognized {
                file: self.current_file.to_path_buf(),
                line,
                detail: format!(
                    "type alias `{alias_ident} = {rhs}` is not recognized; trybuild detection \
                     requires the canonical `trybuild::TestCases::new()` form OR a registered \
                     `--compat-trybuild-macro` alias for the alias's `::new` path"
                ),
            });
        }
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        // Round-6 fix (Gemini BLOCK-B extension): a `#[cfg(...)]`-gated
        // `use` declaration is NOT in scope at adopter build time when
        // the cfg is disabled. Adding the local name to
        // `imported_testcases` / `aliased_testcases` would cause a
        // downstream `TestCases::new()` call (gated under the SAME cfg
        // and disabled together — or gated under a DIFFERENT cfg and
        // potentially enabled in the wrong half) to be incorrectly
        // recognized or incorrectly flagged.
        //
        // The conservative choice: skip processing the `use` entirely.
        // If the call is gated under the same cfg, it shares the use's
        // disabled-at-build-time fate and is correctly ignored. If the
        // call is in a different cfg arm, the visitor cannot know
        // either way — letting the call through as an unknown-shape
        // silent-drop is the safe under-discovery outcome (operator
        // visibility for the unrecognized cfg-gated CALL still happens
        // via `visit_item_fn`'s cfg check). Adopters with cfg-gated
        // imports who want recognition register the canonical path via
        // `--compat-trybuild-macro` (which side-steps the use entirely).
        if find_cfg_attribute(&node.attrs).is_some() {
            return;
        }

        // Two flavors of `use` are interesting here:
        //
        // 1. `use trybuild::TestCases as <name>;` (or any path ending
        //    in `TestCases as <name>;`). The rename's source ident is
        //    `TestCases`; the local name `<name>` is NOT registered
        //    via `--compat-trybuild-macro` (the registration check is
        //    deferred to the call site). Recorded in
        //    `aliased_testcases`; the terminal-call dispatcher emits
        //    `discovery_unrecognized` so the adopter knows to register
        //    the alias.
        // 2. `use trybuild::TestCases;` (no rename). The strict
        //    `prefix == ["trybuild"]` shape feeds `imported_testcases`,
        //    making `TestCases::new()` at the use site a recognized
        //    canonical constructor (treated identically to a literal
        //    `trybuild::TestCases::new()`). This is the most common
        //    trybuild import idiom.
        let mut sink = UseTreeSink {
            renamed: &mut self.aliased_testcases,
            imported: &mut self.imported_testcases,
        };
        collect_use_for_testcases(
            &node.tree,
            /* prefix */ Vec::new(),
            /* leading_colon */ node.leading_colon.is_some(),
            &mut sink,
        );
        syn::visit::visit_item_use(self, node);
    }
}

impl<'a> DiscoveryVisitor<'a> {
    /// Whether `path` is the unregistered-alias constructor shape.
    /// Accepts both forms — `<aliased>::new()` (the canonical
    /// constructor-suffix idiom) AND `<aliased>()` (the
    /// no-`::new` form a registered alias would also accept via
    /// [`Self::is_testcases_constructor_path`]'s `alias_segs` arm).
    /// Round-5 BLOCK fix: previously only the two-segment `::new` form
    /// was matched here, so `use trybuild::TestCases as Foo; Foo()`
    /// silently dropped instead of emitting `discovery_unrecognized`
    /// at the terminal call site. The registered-alias matcher already
    /// recognized both shapes; mirroring that parity here keeps the
    /// unregistered-alias diagnostic surface aligned.
    fn is_aliased_testcases_constructor(&self, expr: &syn::Expr) -> bool {
        let syn::Expr::Path(p) = expr else {
            return false;
        };
        if p.qself.is_some() || !p.attrs.is_empty() || p.path.leading_colon.is_some() {
            return false;
        }
        match p.path.segments.len() {
            // `Foo::new()` — `<alias>::new`.
            2 => {
                let first = p.path.segments[0].ident.to_string();
                let second = p.path.segments[1].ident.to_string();
                second == "new" && self.aliased_testcases.contains(&first)
            }
            // `Foo()` — single-segment alias path; the registered-alias
            // matcher already accepts this via the `alias_segs` arm in
            // `is_testcases_constructor_path`, and the unregistered
            // alias should mirror it so the diagnostic surface stays
            // consistent.
            1 => {
                let first = p.path.segments[0].ident.to_string();
                self.aliased_testcases.contains(&first)
            }
            _ => false,
        }
    }

    /// Whether the receiver chain ultimately roots at an
    /// `aliased_testcases` constructor or an `aliased_bindings`
    /// binding. Mirrors [`Self::receiver_is_testcases`] for the
    /// alias case so the terminal-call dispatcher can distinguish
    /// "recognized" from "unregistered alias" receivers.
    fn receiver_is_aliased_testcases(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Call(call) => {
                self.is_aliased_testcases_constructor(&call.func) && call.args.is_empty()
            }
            syn::Expr::MethodCall(inner) => self.receiver_is_aliased_testcases(&inner.receiver),
            syn::Expr::Path(path_expr) => {
                if path_expr.attrs.is_empty()
                    && path_expr.qself.is_none()
                    && let Some(ident) = path_expr.path.get_ident()
                {
                    return self.aliased_bindings.contains(&ident.to_string());
                }
                false
            }
            syn::Expr::Reference(r) => self.receiver_is_aliased_testcases(&r.expr),
            syn::Expr::Paren(p) => self.receiver_is_aliased_testcases(&p.expr),
            syn::Expr::Group(g) => self.receiver_is_aliased_testcases(&g.expr),
            _ => false,
        }
    }

    /// Whether `expr` is the no-arg constructor expression
    /// `trybuild::TestCases::new()` or an alias's `::new()`. Used by
    /// the pattern-3 `let` tracker.
    fn is_testcases_constructor_expr(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Call(call) => {
                self.is_testcases_constructor_path(&call.func) && call.args.is_empty()
            }
            syn::Expr::Paren(p) => self.is_testcases_constructor_expr(&p.expr),
            syn::Expr::Group(g) => self.is_testcases_constructor_expr(&g.expr),
            _ => false,
        }
    }

    /// Whether `expr` is `<aliased>::new()` (no args) — the alias-case
    /// counterpart to [`Self::is_testcases_constructor_expr`]. Used by
    /// the pattern-3 `let` tracker so `let t = Foo::new();` (where
    /// `Foo` is an unregistered `use trybuild::TestCases as Foo;`
    /// alias) populates [`Self::aliased_bindings`].
    fn is_aliased_testcases_constructor_expr(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Call(call) => {
                self.is_aliased_testcases_constructor(&call.func) && call.args.is_empty()
            }
            syn::Expr::Paren(p) => self.is_aliased_testcases_constructor_expr(&p.expr),
            syn::Expr::Group(g) => self.is_aliased_testcases_constructor_expr(&g.expr),
            _ => false,
        }
    }
}

/// Returns `true` when any attribute in `attrs` is `#[test]`. Recognized
/// shapes: `#[test]` (single segment), `::test`, `core::test`. The
/// strict-syntactic match is the same one libtest uses on the
/// proc-macro path.
fn is_test_attribute(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.meta.path().is_ident("test") {
            return true;
        }
        let segments = path_segments_string(attr.meta.path());
        matches!(
            segments.as_str(),
            "test" | "::test" | "core::test" | "::core::test" | "std::test" | "::std::test"
        )
    })
}

/// Returns the cfg-gating attribute (if any) on `attrs`. Recognized
/// shapes: `#[cfg(...)]` and `#[cfg_attr(...)]` in their unqualified
/// (single-segment) form. Qualified forms (`::core::cfg`, etc.) are not
/// expected in user code — `cfg` is a built-in attribute, not a
/// re-exported item — and would not trigger here, but if a downstream
/// adopter writes one the under-discovery cost is the same as the
/// unguarded body walk (the existing v0.1 behavior).
///
/// Returned reference points into `attrs` so the caller can render the
/// attribute's textual form into the `discovery_unrecognized` detail.
fn find_cfg_attribute(attrs: &[syn::Attribute]) -> Option<&syn::Attribute> {
    attrs.iter().find(|attr| {
        let path = attr.meta.path();
        path.is_ident("cfg") || path.is_ident("cfg_attr")
    })
}

/// Render a `syn::Path` as a `::`-separated string. The leading `::`
/// (if `path.leading_colon` is set) is preserved. Path arguments
/// (`<T>`) are stripped — the discovery pass cares about identity, not
/// generic instantiation.
fn path_segments_string(path: &syn::Path) -> String {
    let mut s = String::new();
    if path.leading_colon.is_some() {
        s.push_str("::");
    }
    for (i, seg) in path.segments.iter().enumerate() {
        if i > 0 {
            s.push_str("::");
        }
        s.push_str(&seg.ident.to_string());
    }
    s
}

/// Returns `true` when `path` has no leading `::` and its segment
/// idents match `expected` element-wise. Borrowing comparison —
/// avoids the `String` allocation that `path_segments_string` would
/// otherwise force on every `Expr::Path` node in the visitor's hot
/// path.
fn path_matches_segments(path: &syn::Path, expected: &[&str]) -> bool {
    if path.leading_colon.is_some() || path.segments.len() != expected.len() {
        return false;
    }
    path.segments
        .iter()
        .zip(expected.iter())
        .all(|(seg, exp)| seg.ident == *exp)
}

/// `String`-segment variant of [`path_matches_segments`] for the
/// pre-computed `--compat-trybuild-macro` alias tables on
/// [`DiscoveryVisitor`].
fn path_matches_string_segments(path: &syn::Path, expected: &[String]) -> bool {
    if path.leading_colon.is_some() || path.segments.len() != expected.len() {
        return false;
    }
    path.segments
        .iter()
        .zip(expected.iter())
        .all(|(seg, exp)| seg.ident == *exp.as_str())
}

/// Mutable sinks for [`collect_use_for_testcases`].
///
/// Keeping the two sets behind one borrow rather than two parallel
/// `&mut` borrows lets the walker stay recursive on a single helper
/// without aliasing the visitor's `&mut self` across separate fields.
struct UseTreeSink<'a> {
    /// Receives renamed locals (`use ... TestCases as <name>;`).
    /// Populated regardless of the prefix shape — the adopter may
    /// have a re-export chain; the spec's alias-detection signal is
    /// the trailing `TestCases` ident.
    renamed: &'a mut BTreeSet<String>,
    /// Receives no-rename canonical imports (`use trybuild::TestCases;`).
    /// ONLY populated when the prefix is exactly `["trybuild"]` so a
    /// `use somelib::TestCases;` is NOT silently treated as canonical.
    imported: &'a mut BTreeSet<String>,
}

/// Walk a `syn::UseTree` and record:
///
/// - `<path-ending-in-TestCases> as <local>` renames into
///   `sink.renamed` (Q6's alias-detection path; later flagged as
///   `discovery_unrecognized` at the terminal-call site).
/// - `trybuild::TestCases` no-rename imports into `sink.imported`
///   (the canonical idiom; the local `<name>::new()` call site is
///   then RECOGNIZED as a fixture).
///
/// `prefix` accumulates the parent `UsePath` segments as the walk
/// descends so `use trybuild::TestCases;` produces `prefix =
/// ["trybuild"]` at the `UseTree::Name { ident: "TestCases" }` leaf.
/// `_leading_colon` is reserved for future absolute-path variants
/// (`use ::trybuild::TestCases;`); v0.1 records either shape
/// indiscriminately because the spec's signal is the trailing
/// `TestCases` segment.
fn collect_use_for_testcases(
    tree: &syn::UseTree,
    prefix: Vec<String>,
    _leading_colon: bool,
    sink: &mut UseTreeSink<'_>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_for_testcases(&path.tree, next, _leading_colon, sink);
        }
        syn::UseTree::Rename(rename) => {
            // `use <prefix>::<rename.ident> as <rename.rename>;`
            // We only record renames whose source ident is `TestCases`.
            // A `use trybuild::TestCases as Foo;` produces `prefix =
            // ["trybuild"]`, `rename.ident = TestCases`, `rename.rename =
            // Foo`. The prefix shape is not constrained here — the
            // adopter may have a re-export chain — but the trailing
            // ident must be `TestCases` for the discovery surface to
            // flag the alias.
            if rename.ident == "TestCases" && !prefix.is_empty() {
                sink.renamed.insert(rename.rename.to_string());
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_for_testcases(item, prefix.clone(), _leading_colon, sink);
            }
        }
        syn::UseTree::Name(name) => {
            // `use <prefix>::<name>;` — the local name equals
            // `<name>`. Recognize the canonical-form import strictly:
            // the trailing ident must be `TestCases` AND the prefix
            // must be exactly `["trybuild"]`. A re-exported
            // `use crate::TestCases;` or a third-party `use somelib::TestCases;`
            // is NOT recognized; the visitor cannot prove the
            // re-export points at trybuild's `TestCases` so we err on
            // the side of false negatives over false positives.
            if name.ident == "TestCases" && prefix.as_slice() == ["trybuild"] {
                sink.imported.insert(name.ident.to_string());
            }
        }
        // `Glob` (`use foo::*;`): nothing to record — `TestCases` is
        // not named explicitly so we cannot identify the local.
        syn::UseTree::Glob(_) => {}
    }
}

// -----------------------------------------------------------------
// Glob expansion (no regex, no `glob` crate)
// -----------------------------------------------------------------

/// Map a string-literal argument to one-or-more concrete fixture
/// paths.
///
/// The literal is treated as relative to the parent directory of
/// `test_file` when relative, used verbatim when absolute. If the
/// literal contains any of `*`, `?`, `[`, it is treated as a glob
/// pattern and expanded via stdlib `std::fs::read_dir` traversal —
/// no `glob` crate is pulled in.
///
/// Returns an `Err(String)` for unrecognized glob shapes (e.g.
/// `**`) so the caller can surface a `discovery_unrecognized` entry
/// with file + line citation.
fn resolve_literal_to_fixtures(
    crate_root: &Path,
    test_file: &Path,
    literal: &str,
) -> Result<Vec<PathBuf>, String> {
    // `**` is not supported in v0.1 — surface as unrecognized.
    if literal.contains("**") {
        return Err(format!(
            "glob `{literal}` uses `**` which is not supported in v0.1"
        ));
    }

    if has_glob_chars(literal) {
        let mut paths = expand_glob(crate_root, test_file, literal)?;
        paths.sort_by(|a, b| {
            a.as_os_str()
                .as_encoded_bytes()
                .cmp(b.as_os_str().as_encoded_bytes())
        });
        Ok(paths)
    } else {
        let absolute = resolve_literal_path(crate_root, test_file, literal);
        if absolute.is_file() {
            Ok(vec![absolute])
        } else {
            Ok(Vec::new())
        }
    }
}

/// Resolve a literal (non-glob) path argument against the conventional
/// trybuild base directory. Trybuild's convention is that fixture
/// paths are relative to the crate root (the directory containing the
/// crate's `Cargo.toml`); on relative literals we resolve there first
/// and fall back to the `tests/*.rs` directory.
fn resolve_literal_path(crate_root: &Path, test_file: &Path, literal: &str) -> PathBuf {
    let literal_path = Path::new(literal);
    if literal_path.is_absolute() {
        return literal_path.to_path_buf();
    }
    let candidate = crate_root.join(literal_path);
    if candidate.exists() {
        return candidate;
    }
    // Fall back to the `tests/*.rs` directory's parent — this matches
    // trybuild's "relative to the test file" expectation for adopters
    // who write `t.compile_fail("ui/foo.rs")` from inside
    // `tests/trybuild.rs`.
    let test_dir = test_file.parent().unwrap_or(crate_root);
    test_dir.join(literal_path)
}

/// Returns `true` when `s` contains any v0.1 glob metacharacter.
fn has_glob_chars(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'['))
}

/// Expand a glob literal into a deterministic list of concrete fixture
/// paths.
///
/// The pattern is split on `/` and each segment is matched separately.
/// Segments without metacharacters extend the path verbatim; segments
/// with metacharacters drive a `read_dir` enumeration of the parent
/// and a per-entry `glob_segment_matches` check.
///
/// Only file entries are returned; the discovery pass does not surface
/// subdirectories. Hidden files (entries whose name begins with `.`)
/// are skipped — trybuild fixtures conventionally live as plain
/// `.rs` files.
fn expand_glob(crate_root: &Path, test_file: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let pattern_path = Path::new(pattern);
    let is_absolute = pattern_path.is_absolute();

    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        return Ok(Vec::new());
    }

    // Anchor selection. For relative globs we walk both the crate-root
    // resolution and the test-file-directory resolution and OR the
    // results. Real-world trybuild adopters pick one or the other
    // convention; we tolerate either to keep discovery robust.
    let mut anchors: Vec<PathBuf> = Vec::new();
    if is_absolute {
        anchors.push(PathBuf::from("/"));
    } else {
        anchors.push(crate_root.to_path_buf());
        if let Some(test_dir) = test_file.parent()
            && test_dir != crate_root
        {
            anchors.push(test_dir.to_path_buf());
        }
    }

    let mut results_seen: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for anchor in anchors {
        let resolved = walk_glob_segments(&anchor, &segments, 0)?;
        for path in resolved {
            results_seen.insert(path.as_os_str().as_encoded_bytes().to_vec(), path);
        }
    }
    Ok(results_seen.into_values().collect())
}

/// Recursive helper for [`expand_glob`]. Walks `segments[idx..]` from
/// `current`. The last segment must resolve to a file; intermediate
/// segments must resolve to directories.
fn walk_glob_segments(
    current: &Path,
    segments: &[&str],
    idx: usize,
) -> Result<Vec<PathBuf>, String> {
    let Some(segment) = segments.get(idx).copied() else {
        if current.is_file() {
            return Ok(vec![current.to_path_buf()]);
        }
        return Ok(Vec::new());
    };
    let is_last = idx + 1 == segments.len();

    if has_glob_chars(segment) {
        let pattern_bytes = segment.as_bytes();
        let entries = match std::fs::read_dir(current) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(format!("glob walk error in `{}`: {e}", current.display()));
            }
        };
        let mut sorted_entries: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|e| format!("glob walk entry error in `{}`: {e}", current.display()))?;
            let name_os = entry.file_name();
            // Skip hidden entries — trybuild fixtures are not dot-files.
            let name_bytes = name_os.as_encoded_bytes();
            if name_bytes.first() == Some(&b'.') {
                continue;
            }
            if !glob_segment_matches(pattern_bytes, name_bytes) {
                continue;
            }
            sorted_entries.push(entry.path());
        }
        sorted_entries.sort_by(|a, b| {
            a.as_os_str()
                .as_encoded_bytes()
                .cmp(b.as_os_str().as_encoded_bytes())
        });

        let mut results: Vec<PathBuf> = Vec::new();
        for path in sorted_entries {
            if is_last {
                if path.is_file() {
                    results.push(path);
                }
            } else if path.is_dir() {
                results.extend(walk_glob_segments(&path, segments, idx + 1)?);
            }
        }
        Ok(results)
    } else {
        // Literal segment — extend the path and recurse.
        let next = current.join(segment);
        if is_last {
            if next.is_file() {
                Ok(vec![next])
            } else {
                Ok(Vec::new())
            }
        } else if next.is_dir() {
            walk_glob_segments(&next, segments, idx + 1)
        } else {
            Ok(Vec::new())
        }
    }
}

/// Match a single path-name byte sequence against a single-segment
/// glob pattern.
///
/// Metacharacters:
///   - `*` — any (possibly empty) sequence of non-`/` bytes.
///   - `?` — exactly one non-`/` byte.
///   - `[abc]` — any single byte from the character class. Ranges
///     are NOT supported in v0.1; `[a-z]` matches the three literal
///     bytes `a`, `-`, `z`. Escapes are NOT supported.
///
/// The matcher is a backtracking walk — the patterns we accept are
/// small (one file-name segment, no nested wildcards) so the
/// worst-case cost is bounded by `pattern.len() * name.len()`.
fn glob_segment_matches(pattern: &[u8], name: &[u8]) -> bool {
    fn rec(p: &[u8], n: &[u8]) -> bool {
        let mut pi = 0;
        let mut ni = 0;
        let mut star_idx: Option<(usize, usize)> = None;
        while ni < n.len() {
            match p.get(pi).copied() {
                Some(b'*') => {
                    star_idx = Some((pi, ni));
                    pi += 1;
                }
                Some(b'?') => {
                    pi += 1;
                    ni += 1;
                }
                Some(b'[') => {
                    // Find the matching `]`.
                    let close = match p.iter().skip(pi + 1).position(|c| *c == b']') {
                        Some(off) => pi + 1 + off,
                        None => return false,
                    };
                    let class = &p[pi + 1..close];
                    if class.contains(&n[ni]) {
                        pi = close + 1;
                        ni += 1;
                    } else if let Some((sp, sn)) = star_idx {
                        pi = sp + 1;
                        ni = sn + 1;
                        star_idx = Some((sp, sn + 1));
                    } else {
                        return false;
                    }
                }
                Some(byte) => {
                    if byte == n[ni] {
                        pi += 1;
                        ni += 1;
                    } else if let Some((sp, sn)) = star_idx {
                        pi = sp + 1;
                        ni = sn + 1;
                        star_idx = Some((sp, sn + 1));
                    } else {
                        return false;
                    }
                }
                None => {
                    if let Some((sp, sn)) = star_idx {
                        pi = sp + 1;
                        ni = sn + 1;
                        star_idx = Some((sp, sn + 1));
                    } else {
                        return false;
                    }
                }
            }
        }
        // Consume any trailing `*` in the pattern.
        while p.get(pi) == Some(&b'*') {
            pi += 1;
        }
        pi == p.len()
    }
    rec(pattern, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_segment_matches_star() {
        assert!(glob_segment_matches(b"*.rs", b"foo.rs"));
        assert!(glob_segment_matches(b"*.rs", b".rs")); // empty match is OK
        assert!(!glob_segment_matches(b"*.rs", b"foo.txt"));
        assert!(glob_segment_matches(b"*", b"anything"));
    }

    #[test]
    fn glob_segment_matches_question_mark() {
        assert!(glob_segment_matches(b"foo?.rs", b"foo1.rs"));
        assert!(glob_segment_matches(b"foo?.rs", b"fooX.rs"));
        assert!(!glob_segment_matches(b"foo?.rs", b"foo.rs"));
        assert!(!glob_segment_matches(b"foo?.rs", b"foo12.rs"));
    }

    #[test]
    fn glob_segment_matches_character_class() {
        assert!(glob_segment_matches(b"fix[abc].rs", b"fixa.rs"));
        assert!(glob_segment_matches(b"fix[abc].rs", b"fixb.rs"));
        assert!(glob_segment_matches(b"fix[abc].rs", b"fixc.rs"));
        assert!(!glob_segment_matches(b"fix[abc].rs", b"fixd.rs"));
    }

    #[test]
    fn glob_segment_matches_literal_dot_in_pattern() {
        // `.` is a literal byte; matches only itself.
        assert!(glob_segment_matches(b"foo.rs", b"foo.rs"));
        assert!(!glob_segment_matches(b"foo.rs", b"fooArs"));
    }

    #[test]
    fn glob_segment_matches_combined_star_class() {
        assert!(glob_segment_matches(b"*[xy].rs", b"foox.rs"));
        assert!(glob_segment_matches(b"*[xy].rs", b"fooy.rs"));
        assert!(!glob_segment_matches(b"*[xy].rs", b"fooz.rs"));
    }

    #[test]
    fn has_glob_chars_negative() {
        assert!(!has_glob_chars("tests/ui/foo.rs"));
        assert!(!has_glob_chars(""));
    }

    #[test]
    fn has_glob_chars_positive() {
        assert!(has_glob_chars("tests/ui/*.rs"));
        assert!(has_glob_chars("tests/ui/f?o.rs"));
        assert!(has_glob_chars("tests/ui/f[abc].rs"));
    }

    #[test]
    fn path_segments_string_no_leading() {
        let p: syn::Path = syn::parse_str("trybuild::TestCases::new").unwrap();
        assert_eq!(path_segments_string(&p), "trybuild::TestCases::new");
    }

    #[test]
    fn path_segments_string_with_leading() {
        let p: syn::Path = syn::parse_str("::trybuild::TestCases::new").unwrap();
        assert_eq!(path_segments_string(&p), "::trybuild::TestCases::new");
    }

    #[test]
    fn is_test_attribute_bare() {
        let attr: syn::Attribute = syn::parse_quote!(#[test]);
        assert!(is_test_attribute(&[attr]));
    }

    #[test]
    fn is_test_attribute_qualified() {
        let attr: syn::Attribute = syn::parse_quote!(#[core::test]);
        assert!(is_test_attribute(&[attr]));
    }

    #[test]
    fn is_test_attribute_rejects_other() {
        let attr: syn::Attribute = syn::parse_quote!(#[cfg(test)]);
        assert!(!is_test_attribute(&[attr]));
    }
}
