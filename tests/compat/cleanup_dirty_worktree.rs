//! Phase 5 of compat mode (issue #10) — dirty-worktree-safe generated
//! output policy integration tests.
//!
//! Every test in this file reaches `lihaaf::CompatCleanupGuard` and
//! friends through the `#[doc(hidden)]` re-exports declared in
//! `src/lib.rs`. The re-exports exist exclusively for this test crate
//! (and for the `cargo-lihaaf` binary, once Phase 9 wires the guard
//! into `compat::run`). The supported entry to compat mode is `cargo
//! lihaaf --compat`, not the Rust API.
//!
//! ## The contract under test
//!
//! `docs/compatibility-plan.md` §3.2.3 — "Dirty-worktree rule":
//!
//! > Generated overlays, copied fixture trees, and generated Lihaaf
//! > snapshots must either live under an ignored compat output
//! > directory or be deliberately included in the PR payload. A
//! > compat run must not leave ambiguous untracked files in the fork.
//! > The report must list every generated path and classify it as
//! > `committed`, `ignored`, or `cleaned`.
//!
//! And the "Cleanup" subsection:
//!
//! > Cleanup runs on **every** exit path, including the hard-fail
//! > exit-67 case in §3.4 (freshness drift), the
//! > `discovery_unrecognized` error path in §3.2.1, and SIGINT/SIGTERM
//! > during stage 3 — the driver registers an exit hook before
//! > materializing the overlay. The single exception is `--keep-
//! > output`, which preserves all generated paths for local
//! > debugging.
//!
//! ## Test taxonomy
//!
//! Each test owns a `tempfile::TempDir` so the suite is hermetic. The
//! tests that exercise the `Committed` / `Ignored` classifications
//! `git init` inside the tempdir; the tests that exercise the fallback
//! to `Cleaned` deliberately skip git initialization to verify locked
//! decision §5.3 (git absence → `Cleaned`).
//!
//! ## Why these tests bite
//!
//! - `cleanup_removes_cleaned_paths` would fail if `run_cleanup_once`
//!   short-circuited or if the classifier defaulted to `Ignored`
//!   without proof. Either failure mode regresses the dirty-worktree
//!   rule.
//! - `keep_output_converts_cleaned_to_kept` would fail if the
//!   `--keep-output` flag mistakenly cleaned anyway, violating §8.2 of
//!   the v0.1 spec.
//! - `drop_runs_cleanup_on_panic` is the safety-net acid test: if Drop
//!   ever stopped cleaning, a panicked compat run would leak residue.

use std::path::{Path, PathBuf};
use std::process::Command;

use lihaaf::{CompatCleanupGuard as CleanupGuard, CompatGeneratedPathClass as GeneratedPathClass};

/// Helper: initialize a minimal git repo inside `dir` so the
/// classifier's `git ls-files` and `git check-ignore` calls see a
/// real working tree. Uses minimal config (no signing, no global
/// settings consulted) so the test is hermetic.
///
/// Panics if any subprocess fails — these tests cannot proceed
/// without git, and a missing git binary is a test-environment defect
/// rather than something the suite should silently skip.
fn git_init(dir: &Path) {
    run_git(dir, &["init", "--quiet", "--initial-branch=main"]);
    // Per-repo config so `git commit` works without consulting the
    // user's global config (which may be absent in CI).
    run_git(
        dir,
        &["config", "user.email", "lihaaf-test@example.invalid"],
    );
    run_git(dir, &["config", "user.name", "lihaaf-test"]);
    run_git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Stage and commit `path` inside the git repo rooted at `dir`. Used
/// to drive the `Committed` classification path.
fn git_add_and_commit(dir: &Path, path: &Path, msg: &str) {
    let rel = path
        .strip_prefix(dir)
        .expect("commit path must be under dir");
    run_git(dir, &["add", "--", &rel.to_string_lossy()]);
    run_git(dir, &["commit", "--quiet", "-m", msg]);
}

/// Run `git <args>` inside `dir`, panicking on non-zero exit. Stdout
/// and stderr are captured so a failure prints both streams in the
/// test report.
fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git binary must be on PATH for the cleanup test suite");
    if !out.status.success() {
        panic!(
            "git {args:?} failed in {}:\nstdout: {}\nstderr: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Drop a file at `path` with the given byte contents, creating
/// parent directories if needed.
fn write_artifact(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("creating parent dir for test artifact");
    }
    std::fs::write(path, bytes).expect("writing test artifact");
}

/// Look up the classification of a single tracked path in a guard's
/// finalized result. Panics if the path is not present or the result
/// has unexpected shape (multiple entries for the same path).
fn class_of(results: &[lihaaf::CompatGeneratedPath], path: &Path) -> GeneratedPathClass {
    let matches: Vec<&lihaaf::CompatGeneratedPath> =
        results.iter().filter(|r| r.path == path).collect();
    match matches.as_slice() {
        [single] => single.class,
        [] => panic!(
            "path {} not present in finalize result; got {:?}",
            path.display(),
            results.iter().map(|r| &r.path).collect::<Vec<_>>()
        ),
        many => panic!(
            "path {} present {} times in finalize result; expected once",
            path.display(),
            many.len()
        ),
    }
}

/// **Committed path is not cleaned.**
///
/// Register a file that has been `git add`ed and committed; after
/// finalize, the file must still exist on disk and be classified
/// `Committed`.
///
/// Regression bite: if the classifier ever defaulted to `Cleaned`
/// without checking `git ls-files`, this test would fail because the
/// file would be removed despite being tracked.
#[test]
fn committed_path_is_not_cleaned() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");
    git_add_and_commit(root, &path, "compat: committed overlay");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    guard.track(path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert!(
        path.exists(),
        "committed path must survive cleanup; got {:?}",
        results
    );
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Committed);
}

/// **Path under `<root>/target/` is classified `Ignored` and not
/// cleaned.**
///
/// `target/` is cargo-owned; the classifier short-circuits to
/// `Ignored` without invoking git so the rule holds even when the
/// fork has no `.gitignore` rule covering `target/`.
///
/// Regression bite: if the classifier ever required an explicit
/// `.gitignore` rule for `target/`, this test would fail on a
/// freshly-`git init`ed worktree that lacks a `target/` line.
#[test]
fn target_directory_path_classified_as_ignored() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root); // No .gitignore; target/ is implicit.

    let path = root
        .join("target")
        .join("lihaaf-compat-converted")
        .join("fixture.rs");
    write_artifact(&path, b"// converted fixture\n");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    guard.track(path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert!(
        path.exists(),
        "Ignored path must survive cleanup; got {:?}",
        results
    );
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Ignored);
}

/// **Round-3 BLOCK regression: relative paths resolve against
/// `target_root` at `track` time.** The `CleanupGuard::track` API
/// accepts relative-to-`target_root` paths for caller convenience,
/// but the classifier's `is_under_cargo_target` compares against
/// the joined `<target_root>/target` prefix via `starts_with`. A
/// relative path like `target/foo` would silently miss the prefix
/// and fall through to the git-classifier, then to the `Cleaned`
/// default — at which point the file would be REMOVED even though
/// it lives under cargo's owned `target/` directory.
///
/// The fix resolves relative paths against `target_root` eagerly
/// in `track`, storing the absolute form internally. After the
/// fix:
///
/// 1. The result `GeneratedPath.path` is `target_root.join(...)` —
///    the absolute form, byte-equal to what a directly-absolute
///    `track` call would produce.
/// 2. The classification matches the absolute-form invocation
///    (`Ignored` for a `target/`-rooted path).
#[test]
fn relative_track_input_resolves_against_target_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root); // No .gitignore — `target/` is short-circuited.

    // Use a path under `target/` so the `is_under_cargo_target`
    // short-circuit decides the classification; that branch is the
    // one a relative input regressed.
    let relative = PathBuf::from("target/lihaaf-compat-converted/fixture.rs");
    let absolute = root.join(&relative);
    write_artifact(&absolute, b"// converted fixture\n");

    // Reference run: track the absolute form directly. This is the
    // "what should happen" baseline.
    let absolute_guard = CleanupGuard::new(/*keep_output=*/ false);
    absolute_guard.track(absolute.clone(), root);
    let absolute_results = absolute_guard.finalize().expect("finalize must succeed");
    assert_eq!(
        class_of(&absolute_results, &absolute),
        GeneratedPathClass::Ignored,
        "baseline: absolute `target/`-rooted path must classify as Ignored"
    );

    // Restore the file (the reference run did not remove it because
    // it classified as Ignored, so this is a no-op; we keep it
    // defensive against future refactors).
    write_artifact(&absolute, b"// converted fixture\n");

    // Test run: track the RELATIVE form. The fix resolves it against
    // `target_root` at track time, so the result entry's `path` field
    // is byte-equal to the absolute form, AND the classification
    // matches the baseline.
    let relative_guard = CleanupGuard::new(/*keep_output=*/ false);
    relative_guard.track(relative.clone(), root);
    let relative_results = relative_guard.finalize().expect("finalize must succeed");

    // Stored path is the absolute form, not the relative input.
    assert_eq!(
        relative_results.len(),
        1,
        "exactly one tracked entry expected; got {:?}",
        relative_results
    );
    assert_eq!(
        relative_results[0].path,
        absolute,
        "track must store the absolute resolution of a relative input; \
         got `{}` expected `{}`",
        relative_results[0].path.display(),
        absolute.display()
    );
    // Classification matches the absolute-form baseline.
    assert_eq!(
        class_of(&relative_results, &absolute),
        GeneratedPathClass::Ignored,
        "relative-input classification must match absolute-form baseline"
    );
    assert!(
        absolute.exists(),
        "Ignored path must survive cleanup regardless of track-input shape"
    );
}

/// **`.gitignore`d path is classified `Ignored` and not cleaned.**
///
/// Explicitly tests the `git check-ignore` path: the file is not
/// under `target/` (which would short-circuit to `Ignored` without
/// git) but a `.gitignore` rule covers it.
///
/// Regression bite: if the classifier dropped the `git check-ignore`
/// branch, this file would be classified `Cleaned` and removed
/// despite the user authorizing it via `.gitignore`.
#[test]
fn gitignored_path_is_not_cleaned() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let gitignore = root.join(".gitignore");
    std::fs::write(&gitignore, "Cargo.lihaaf.toml\n").expect("write .gitignore");
    git_add_and_commit(root, &gitignore, "compat: ignore overlay");

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    guard.track(path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert!(
        path.exists(),
        "gitignored path must survive cleanup; got {:?}",
        results
    );
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Ignored);
}

/// **Untracked, un-ignored path is classified `Cleaned` and removed.**
///
/// This is the dirty-worktree rule's acid test: an artifact that is
/// neither committed nor covered by `.gitignore` is exactly the
/// ambiguous-untracked-file the spec forbids. The driver MUST remove
/// it on exit.
///
/// Regression bite: if `run_cleanup_once` ever became a no-op (e.g. a
/// future refactor stops calling `remove_path_best_effort`), the file
/// would survive and the worktree would be left dirty.
#[test]
fn untracked_unignored_path_classified_cleaned_and_removed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    guard.track(path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Cleaned);
    assert!(
        !path.exists(),
        "Cleaned path must be removed; survived at {}",
        path.display()
    );
}

/// **`--keep-output` converts `Cleaned` to `Kept` and preserves the
/// path on disk.**
///
/// Mirrors v0.1 spec §8.2 ("preserve all generated paths for local
/// debugging"). The classification still records the residue so the
/// §3.3 envelope can list it.
///
/// Regression bite: if `keep_output` were ignored and the cleanup
/// ran anyway, this test fails on `path.exists()`. If the
/// classification record dropped the `Kept` variant (e.g. forced
/// everything to `Ignored` on keep-output runs), `class_of` returns
/// the wrong variant and fails the equality.
#[test]
fn keep_output_converts_cleaned_to_kept() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    let guard = CleanupGuard::new(/*keep_output=*/ true);
    guard.track(path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Kept);
    assert!(
        path.exists(),
        "Kept path must survive when --keep-output is set; lost at {}",
        path.display()
    );
}

/// **Directory trees are recursively cleaned.**
///
/// Phase 6 will produce `target/lihaaf-compat-converted/` as a
/// directory of converted fixture trees. Removal via
/// `std::fs::remove_dir_all` is the contract the cleanup module
/// promises.
///
/// To force the `Cleaned` classification (not the `Ignored` shortcut
/// that `<root>/target/` triggers), the test places the tree under
/// a different parent.
///
/// Regression bite: if `remove_path_best_effort` ever called
/// `remove_file` unconditionally, the recursive directory would fail
/// to remove with `EISDIR` and the cleanup would error out instead
/// of succeeding.
#[test]
fn directory_tree_is_recursively_cleaned() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let tree_root = root.join("lihaaf-compat-staging");
    write_artifact(&tree_root.join("nested").join("a.rs"), b"// a\n");
    write_artifact(&tree_root.join("nested").join("b.rs"), b"// b\n");
    write_artifact(&tree_root.join("c.rs"), b"// c\n");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    guard.track(tree_root.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    assert_eq!(class_of(&results, &tree_root), GeneratedPathClass::Cleaned);
    assert!(
        !tree_root.exists(),
        "directory tree must be recursively removed; survived at {}",
        tree_root.display()
    );
}

/// **Rerun idempotence: running cleanup twice (via two distinct
/// guards) on the same logical path is well-defined.**
///
/// The second guard tracks a path the first guard already removed;
/// `remove_path_best_effort` treats the non-existent path as
/// already-cleaned (no error), so finalize returns successfully and
/// the path is still classified `Cleaned` (the classifier doesn't
/// require the path to exist to bucket it).
///
/// Regression bite: a `remove_file` that errored on `ENOENT` would
/// propagate `Error::Io` from the second finalize, breaking the
/// rerun-from-clean-state ergonomic the spec requires.
#[test]
fn rerun_idempotent_two_finalize_calls() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    let first = CleanupGuard::new(/*keep_output=*/ false);
    first.track(path.clone(), root);
    let _ = first.finalize().expect("first finalize must succeed");
    assert!(!path.exists(), "first cleanup must have removed the path");

    // Second run from clean state — register the same path again.
    // The artifact is gone, but the classifier still buckets the
    // logical path; the remove call is a no-op (the path doesn't
    // exist).
    let second = CleanupGuard::new(/*keep_output=*/ false);
    second.track(path.clone(), root);
    let results = second
        .finalize()
        .expect("second finalize must succeed on rerun");
    assert_eq!(class_of(&results, &path), GeneratedPathClass::Cleaned);
    assert!(!path.exists(), "second cleanup must remain a no-op");
}

/// **Drop is the panic safety-net.**
///
/// Construct a guard inside `std::panic::catch_unwind`, register a
/// path that should be cleaned, deliberately panic. The Drop impl
/// must fire as the stack unwinds and remove the registered path.
///
/// Regression bite: if Drop ever stopped running cleanup (e.g. a
/// future refactor moved cleanup into a `pub(crate) fn cleanup` that
/// must be called explicitly), this test fails because the residue
/// survives the panic.
#[test]
fn drop_runs_cleanup_on_panic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Move the root out of the tempdir handle so we can keep the
    // tempdir alive after the panic — otherwise the tempdir drops
    // would race the assertion and the test wouldn't observe the
    // cleanup result clearly.
    let root = tmp.path().to_path_buf();
    git_init(&root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    let path_for_closure = path.clone();
    let root_for_closure = root.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let guard = CleanupGuard::new(/*keep_output=*/ false);
        guard.track(path_for_closure.clone(), &root_for_closure);
        panic!("deliberate panic — Drop must still clean");
    }));

    assert!(
        outcome.is_err(),
        "the test must observe an unwinding panic, not a normal return"
    );
    assert!(
        !path.exists(),
        "Drop must have removed the registered path during unwind; survived at {}",
        path.display()
    );
}

/// **Finalize consumes the guard; Drop afterward is a no-op.**
///
/// Once `finalize` has classified and cleaned everything, Drop must
/// not re-run the pipeline (the atomic flag short-circuits). This
/// test re-creates the artifact between `finalize` and the implicit
/// Drop, then asserts the recreated file is still on disk after the
/// Drop runs.
///
/// Regression bite: if Drop ignored the atomic gate and re-ran
/// cleanup, the recreated file would be removed and `path.exists()`
/// would return false.
#[test]
fn finalize_consumes_guard_drop_is_noop() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let path = root.join("Cargo.lihaaf.toml");
    write_artifact(&path, b"# overlay\n");

    // Scope the guard so the implicit Drop fires before the
    // assertion below.
    {
        let guard = CleanupGuard::new(/*keep_output=*/ false);
        guard.track(path.clone(), root);
        let _ = guard.finalize().expect("finalize must succeed");
        // The artifact was registered un-ignored, so finalize
        // removed it.
        assert!(!path.exists(), "finalize must have cleaned the artifact");

        // Re-create the artifact between finalize and the Drop. If
        // Drop re-ran cleanup, this file would be removed; the
        // atomic-gate guarantees Drop is a no-op once finalize has
        // run.
        write_artifact(&path, b"# resurrected after finalize\n");
    }

    assert!(
        path.exists(),
        "Drop after finalize must be a no-op; the resurrected artifact must survive at {}",
        path.display()
    );
}

/// **Multiple paths in one guard sort deterministically.**
///
/// The §3.3 envelope writer (Phase 8) reads `finalize`'s returned
/// list and renders it into JSON. Two runs from identical input must
/// produce byte-identical envelopes, which requires the cleanup
/// result to be sorted by path.
///
/// Regression bite: if `finalize` stopped sorting (e.g. a refactor
/// preserves insertion order), this test fails because the
/// registration order intentionally differs from the sorted order.
#[test]
fn finalize_returns_paths_sorted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);

    let z_path = root.join("z-overlay.toml");
    let a_path = root.join("a-overlay.toml");
    let m_path = root.join("m-overlay.toml");
    write_artifact(&z_path, b"# z\n");
    write_artifact(&a_path, b"# a\n");
    write_artifact(&m_path, b"# m\n");

    let guard = CleanupGuard::new(/*keep_output=*/ false);
    // Intentionally register out of order.
    guard.track(z_path.clone(), root);
    guard.track(a_path.clone(), root);
    guard.track(m_path.clone(), root);

    let results = guard.finalize().expect("finalize must succeed");
    let returned: Vec<&PathBuf> = results.iter().map(|r| &r.path).collect();
    assert_eq!(
        returned,
        vec![&a_path, &m_path, &z_path],
        "finalize must return paths sorted by path; got {:?}",
        returned
    );
}
