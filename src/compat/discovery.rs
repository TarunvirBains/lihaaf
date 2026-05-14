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
//! `use trybuild::TestCases as Foo;` aliases are NOT syntactically
//! recognized (Q6 locked). Adopters with `use ... as ...` re-exports
//! must register them via `--compat-trybuild-macro`.
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
    /// alias's `::new` form).
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
        // Save and reset both the enclosing-test marker and the
        // per-function bindings so cross-function leakage is
        // impossible.
        let saved_enclosing = self.enclosing_test_fn.take();
        let saved_bindings = std::mem::take(&mut self.local_bindings);

        if is_test_attribute(&node.attrs) {
            self.enclosing_test_fn = Some(node.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, node);

        self.enclosing_test_fn = saved_enclosing;
        self.local_bindings = saved_bindings;
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        // Pattern 3 binding tracker. We only watch for the shape
        // `let <ident> = <TestCases::new()-or-alias>;`. Anything else
        // (typed pattern, destructured tuple, ref binding) is left
        // alone — the binding tracker is strictly opt-in.
        if let (syn::Pat::Ident(pat_ident), Some(init)) = (&node.pat, &node.init)
            && pat_ident.attrs.is_empty()
            && pat_ident.by_ref.is_none()
            && pat_ident.subpat.is_none()
            && self.is_testcases_constructor_expr(&init.expr)
        {
            self.local_bindings.insert(pat_ident.ident.to_string());
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
        // Methods inside `impl Foo { fn bar() { ... } }` get walked
        // via syn's default visitor when this override is absent — and
        // the default visitor does NOT scope `local_bindings`, so a
        // `let t = TestCases::new();` inside an impl method would leak
        // into the enclosing scope's bindings table (or vice-versa).
        // Mirror the save/restore pattern from `visit_item_fn` so each
        // impl method gets its own fresh bindings scope; `#[test]` on
        // impl methods is exceedingly rare but supported uniformly
        // (the same `is_test_attribute` filter applies).
        let saved_enclosing = self.enclosing_test_fn.take();
        let saved_bindings = std::mem::take(&mut self.local_bindings);

        if is_test_attribute(&node.attrs) {
            self.enclosing_test_fn = Some(node.sig.ident.to_string());
        }
        syn::visit::visit_impl_item_fn(self, node);

        self.enclosing_test_fn = saved_enclosing;
        self.local_bindings = saved_bindings;
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
}

impl<'a> DiscoveryVisitor<'a> {
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
            "test" | "::test" | "core::test" | "::core::test"
        )
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
