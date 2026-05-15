//! Phase 5 of compat mode (issue #10) — dirty-worktree-safe generated
//! output policy.
//!
//! The compat driver materializes a small set of artifacts inside the
//! adopter's target-crate checkout: the sibling overlay
//! (`Cargo.lihaaf.toml`, Phase 2), the §3.3 envelope (Phase 8), and —
//! once Phase 6 fixture-conversion lands — a transient directory of
//! converted fixtures under `target/lihaaf-compat-converted/`. Per
//! `docs/compatibility-plan.md` §3.2.3:
//!
//! > Generated overlays, copied fixture trees, and generated Lihaaf
//! > snapshots must either live under an ignored compat output
//! > directory or be deliberately included in the PR payload. A compat
//! > run must not leave ambiguous untracked files in the fork.
//!
//! This module owns the lifecycle of those generated paths: every
//! phase that produces an artifact calls [`CleanupGuard::track`] before
//! returning the path to the caller, and the driver consumes the guard
//! at the end of [`crate::compat::run`] via [`CleanupGuard::finalize`].
//! Drop is the safety-net for panic / early-return paths.
//!
//! ## Classification
//!
//! Each tracked path is bucketed into one of four states by
//! [`GeneratedPathClass`]:
//!
//! - [`Committed`](GeneratedPathClass::Committed) — `git ls-files
//!   --error-unmatch` returns 0. The path is checked into the target
//!   crate's repository; cleanup is a no-op (the user already
//!   authorized the artifact to live in the worktree).
//! - [`Ignored`](GeneratedPathClass::Ignored) — `git check-ignore --quiet`
//!   returns 0 (or the path lives under `<target_root>/target/`, which
//!   cargo treats as implicitly ignored even before any `.gitignore`
//!   covers it). The user already authorized the artifact via the
//!   `.gitignore` choice; cleanup is a no-op.
//! - [`Cleaned`](GeneratedPathClass::Cleaned) — neither tracked nor
//!   ignored. The driver MUST remove the artifact on every exit path,
//!   including panic and early-return, so the dirty-worktree rule
//!   holds even on failure.
//! - [`Kept`](GeneratedPathClass::Kept) — set on `--keep-output` runs.
//!   Every path that would have been [`Cleaned`](GeneratedPathClass::Cleaned)
//!   is preserved instead; the §3.3 envelope records the residue so
//!   the operator can clean up manually.
//!
//! ## Locked decisions
//!
//! 1. **`Drop` is a safety net, not the primary cleanup path.** The
//!    driver calls [`CleanupGuard::finalize`] at the end of
//!    [`crate::compat::run`]; Drop fires only on panic / early-return
//!    paths. Once `finalize` has run, Drop is a no-op (the consumed
//!    guard has nothing left to clean).
//! 2. **`git check-ignore` is the source of truth for the `Ignored`
//!    classification.** Re-implementing `.gitignore`'s pattern semantics
//!    (glob, negation, `**`, per-directory files) would be a sizable
//!    sub-project; outsourcing to `git` is correct and cheap. The
//!    classifier additionally treats `<target_root>/target/` as
//!    `Ignored` even when no `.gitignore` covers it, because cargo
//!    itself owns that directory.
//! 3. **`git` absence falls back to `Cleaned`.** Without `git` on
//!    `PATH` (or without a `.git/` directory in `target_root`), the
//!    classifier cannot prove a path is ignored or committed; the safe
//!    default is to remove the path on exit.
//! 4. **`--keep-output` overrides `Cleaned`, never `Ignored` /
//!    `Committed`.** A flag designed for local debugging does not
//!    override the user's explicit `.gitignore` or `git add` choice.
//! 5. **SIGINT / SIGTERM are out of scope for Phase 5.** Installing a
//!    signal handler would either pull in a new crate (`ctrlc` /
//!    `signal-hook`) or hand-roll cross-platform FFI; both expand the
//!    dependency surface for marginal gain. Drop covers the panic /
//!    early-return cases; `SIGKILL` and `SIGTERM` remain unrecoverable
//!    by design (documented gap, revisit in Phase 6+).
//! 6. **Panic hook is diagnostic, not cleanup.** Drop runs during stack
//!    unwinding on panic, so the cleanup itself is guaranteed by the
//!    Drop impl alone. The optional [`install_panic_hook`] adds a
//!    diagnostic line that names the partial path before the panic
//!    propagates, chaining to the previously-installed hook so
//!    `libtest`'s panic capture continues to work.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::Error;

/// Classification of a path the compat driver generated.
///
/// One bucket per generated artifact. Populated by
/// [`CleanupGuard::finalize`] from the raw `PendingPath` entries the
/// driver registered during the run.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the
/// crate root can `#[doc(hidden)]` re-export this for the test
/// crate. Not part of any v0.1 stability contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedPathClass {
    /// Path is tracked by the target crate's git index. Compat does
    /// not clean it; the user already authorized the artifact to live
    /// in the worktree.
    Committed,
    /// Path is covered by the target crate's `.gitignore` (or lives
    /// under `<target_root>/target/`). Compat does not clean it; the
    /// user already authorized the artifact via the ignore rule.
    Ignored,
    /// Path is neither tracked nor ignored. Compat removed it on exit
    /// so the worktree stays clean.
    Cleaned,
    /// Path would have been [`Cleaned`](Self::Cleaned) but
    /// `--keep-output` was set. The §3.3 envelope records the residue
    /// so the operator can remove it manually.
    Kept,
}

/// One generated path the driver tracked, after classification has
/// run. The `path` is preserved verbatim from registration so the §3.3
/// envelope can render it directly.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the
/// crate root can `#[doc(hidden)]` re-export this for the test
/// crate. Not part of any v0.1 stability contract.
#[derive(Debug, Clone)]
pub struct GeneratedPath {
    /// Absolute path the driver produced.
    pub path: PathBuf,
    /// Final classification, populated by
    /// [`CleanupGuard::finalize`].
    pub class: GeneratedPathClass,
}

/// One un-classified registration entry. The cleanup classifier walks
/// these in [`CleanupGuard::finalize`] and produces the
/// [`GeneratedPath`] list the envelope writer consumes.
///
/// `target_root` is recorded per-entry rather than once per guard
/// because Phase 8+ may add multi-root scenarios (e.g. an overlay
/// generated under one crate and a sidecar generated under another).
/// In v0.1 every entry shares the same root, but the per-entry shape
/// keeps the structure additive.
#[derive(Debug, Clone)]
struct PendingPath {
    /// Absolute path the driver produced.
    path: PathBuf,
    /// The target crate root the path was produced under. Used as the
    /// `git` working directory for `git check-ignore` / `git ls-files`.
    target_root: PathBuf,
}

/// Tracker for generated paths.
///
/// Wrapped by [`CleanupGuard`] for the public surface; the tracker
/// itself is internal because the lifecycle (Drop, finalize) belongs
/// to the guard.
#[derive(Debug, Default)]
struct CleanupTracker {
    /// Pending registrations. Populated by [`CleanupGuard::track`];
    /// drained by [`CleanupGuard::finalize`] or by the Drop safety net.
    pending: Vec<PendingPath>,
}

/// RAII handle for cleanup of compat-generated paths.
///
/// Construct one near the top of [`crate::compat::run`], call
/// [`CleanupGuard::track`] for every generated path, and call
/// [`CleanupGuard::finalize`] at every well-known exit. If `finalize`
/// is never called (panic, `?`-propagation, early `return`), the Drop
/// impl runs the cleanup as a safety net.
///
/// ## Once-only semantics
///
/// Cleanup runs at most once per guard. Both [`Self::finalize`] and
/// the Drop impl swap a shared [`AtomicBool`] before touching the
/// tracker state, so the second call is a no-op. `finalize` is the
/// explicit, well-known path and reports filesystem errors via
/// `Result`; Drop is the panic / early-return safety net and
/// best-effort-swallows filesystem errors so an unwinding panic is
/// never masked by a secondary cleanup error. The two paths share the
/// private `classify_entry` helper but otherwise differ in mutex
/// acquisition (Drop uses `try_lock`) and error handling.
///
/// ## Thread safety
///
/// The interior `Mutex<CleanupTracker>` makes the guard `Sync` so it
/// can be referenced from `&self` methods on multiple threads (Phase
/// 6+ fixture-conversion may parallelize). The `try_lock` path in
/// Drop avoids a secondary panic if another thread is still inside
/// `track` when an unwinding panic begins on this thread; the
/// original panic is more informative than a "poisoned mutex"
/// secondary panic.
/// `pub` (with the parent module pinned at `pub(crate)`) so the
/// crate root can `#[doc(hidden)]` re-export this for the test
/// crate. Not part of any v0.1 stability contract.
#[derive(Debug)]
pub struct CleanupGuard {
    /// Tracker state. Behind a `Mutex` so `track` can be called from
    /// `&self` (Phase 2 overlay generation is single-threaded today,
    /// but Phase 6 fixture conversion is the natural place to
    /// parallelize and the guard surface should not need to change).
    inner: Mutex<CleanupTracker>,
    /// Mirrors `CompatArgs::inner_cli.keep_output`. When `true`, every
    /// `Cleaned` classification is promoted to `Kept` and the
    /// filesystem is not touched.
    keep_output: bool,
    /// Set once cleanup has run (via `finalize` or Drop). Prevents
    /// double-cleanup and lets Drop skip the lock entirely when the
    /// guard has already been consumed.
    cleaned: AtomicBool,
}

impl CleanupGuard {
    /// Construct a fresh guard. `keep_output` is typically
    /// [`crate::cli::Cli::keep_output`] — pass it through from the
    /// driver so a single source of truth governs both Phase 5 cleanup
    /// and Phase 9 inner-session output-retention.
    ///
    /// `pub` to allow the test crate's `#[doc(hidden)]` re-export to
    /// reach this constructor. Adopters must drive compat mode through
    /// `cargo lihaaf --compat`, not the Rust API.
    pub fn new(keep_output: bool) -> Self {
        Self {
            inner: Mutex::new(CleanupTracker::default()),
            keep_output,
            cleaned: AtomicBool::new(false),
        }
    }

    /// Register a generated path for classification + cleanup.
    ///
    /// `path` may be absolute OR relative to `target_root`; the
    /// API boundary accepts both for caller convenience, but the
    /// tracker stores the path in absolute form internally. This
    /// matters because the classifier (`is_under_cargo_target`)
    /// does a `starts_with` check against `<target_root>/target` —
    /// a relative path would silently miss the prefix and be
    /// misclassified as `Cleaned` (then removed) when it actually
    /// lives under `target/`. `target_root` is the adopter's
    /// `--compat-root` — the working directory for the `git`
    /// invocations in [`CleanupGuard::finalize`].
    ///
    /// Registration is cheap: the entry is appended to a pending
    /// vector; classification + filesystem work is deferred to
    /// `finalize`. This keeps the hot path (overlay generation,
    /// fixture conversion) free of subprocess spawns.
    ///
    /// `pub` to allow the test crate's `#[doc(hidden)]` re-export to
    /// reach this method.
    pub fn track(&self, path: PathBuf, target_root: &Path) {
        // Resolve relative inputs against `target_root` eagerly. The
        // classifier's `starts_with` check and the `git` subprocess
        // both behave correctly on absolute paths; storing the
        // absolute form here means the rest of the cleanup pipeline
        // is invariant w.r.t. how the caller chose to express the
        // path. `PathBuf::join` is a no-op when `path` is already
        // absolute, so absolute inputs pass through unchanged.
        let absolute = if path.is_absolute() {
            path
        } else {
            target_root.join(&path)
        };
        // Acquire-only — if a previous panic poisoned the mutex, we
        // still want to record the new entry. The `into_inner` /
        // `get_mut` path used by Drop handles the poisoning
        // gracefully.
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.pending.push(PendingPath {
            path: absolute,
            target_root: target_root.to_path_buf(),
        });
    }

    /// Classify every registered path, remove what needs removing,
    /// and return the final list for the §3.3 envelope.
    ///
    /// The returned vector is sorted by `path` so the envelope is
    /// deterministic across runs. After `finalize` returns, the guard
    /// has been consumed; the Drop impl is a no-op.
    ///
    /// `pub` to allow the test crate's `#[doc(hidden)]` re-export to
    /// reach this method.
    pub fn finalize(self) -> Result<Vec<GeneratedPath>, Error> {
        self.run_cleanup_once()
    }

    /// Internal: run the cleanup pass at most once.
    ///
    /// Called by both [`Self::finalize`] (the explicit, well-known
    /// path) and [`Drop::drop`] (the safety net). The
    /// [`AtomicBool`] gate makes the second call a no-op so a
    /// finalize-then-Drop sequence does not classify or touch the
    /// filesystem twice.
    ///
    /// **Error policy:** every pending entry is processed even if a
    /// previous entry's removal failed. Failures are accumulated; the
    /// first failure's `Error::Io` is returned with a path-list suffix
    /// when more than one removal failed. This matters because the
    /// atomic-gate marks the guard cleaned at function entry — short-
    /// circuiting on the first error would silently leak every later
    /// entry (Drop sees the gate already tripped and no-ops). The
    /// classifications for every entry, whether removal succeeded or
    /// failed, are returned in the success case; on aggregate error the
    /// returned `Vec` is empty so the caller does not consume a stale
    /// list (the §3.3 envelope writer treats a failed cleanup as a
    /// session-level failure and does not emit residue records for it).
    fn run_cleanup_once(&self) -> Result<Vec<GeneratedPath>, Error> {
        // `swap` returns the previous value. If `true`, cleanup
        // already ran; bail with an empty list (the caller's first
        // call already received the canonical list).
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return Ok(Vec::new());
        }

        let pending = {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut guard.pending)
        };

        let mut classified: Vec<GeneratedPath> = Vec::with_capacity(pending.len());
        let mut first_error: Option<Error> = None;
        let mut additional_failed_paths: Vec<PathBuf> = Vec::new();
        for entry in pending {
            let final_class = classify_entry(&entry, self.keep_output);
            if final_class == GeneratedPathClass::Cleaned
                && let Err(err) = remove_path_best_effort(&entry.path)
            {
                if first_error.is_none() {
                    first_error = Some(err);
                } else {
                    additional_failed_paths.push(entry.path.clone());
                }
            }
            classified.push(GeneratedPath {
                path: entry.path,
                class: final_class,
            });
        }

        if let Some(err) = first_error {
            return Err(aggregate_cleanup_error(err, additional_failed_paths));
        }

        // Determinism: sort by path so two runs from the same input
        // produce byte-identical envelopes (the §3.3 contract).
        classified.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(classified)
    }
}

/// Build the aggregate-failure `Error` returned by [`CleanupGuard::run_cleanup_once`].
///
/// `first_error` is preserved verbatim — the original `Error::Io`'s
/// `source`, `context`, and `path` fields all carry forward so callers
/// can still pattern-match on the underlying `io::ErrorKind`. When more
/// than one entry failed, the additional paths are appended to the
/// context line so the operator sees the full failure surface in one
/// message rather than chasing a series of suppressed errors.
fn aggregate_cleanup_error(first_error: Error, additional: Vec<PathBuf>) -> Error {
    if additional.is_empty() {
        return first_error;
    }
    match first_error {
        Error::Io {
            source,
            context,
            path,
        } => {
            let suffix = additional
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Error::Io {
                source,
                context: format!(
                    "{context} (and {} other compat-generated path(s) also failed to remove: {suffix})",
                    additional.len(),
                ),
                path,
            }
        }
        // `remove_path_best_effort` only returns `Error::Io`; any other
        // variant is a bug, but we surface it verbatim rather than
        // panic so the original diagnostic survives.
        other => other,
    }
}

/// Classify `entry` and apply the `--keep-output` promotion. Returns
/// the final class so the caller can decide whether to remove the
/// path (only [`GeneratedPathClass::Cleaned`] requires removal). The
/// removal itself is the caller's responsibility — `finalize` uses `?`
/// propagation; Drop swallows errors mid-unwind.
fn classify_entry(entry: &PendingPath, keep_output: bool) -> GeneratedPathClass {
    let class = classify(&entry.target_root, &entry.path);
    match (class, keep_output) {
        (GeneratedPathClass::Cleaned, true) => GeneratedPathClass::Kept,
        (other, _) => other,
    }
}

impl Drop for CleanupGuard {
    /// Safety-net cleanup for panic / early-return paths.
    ///
    /// Best-effort only: a filesystem error inside Drop is ignored
    /// rather than double-panicking — the original panic is the more
    /// informative signal. Per the plan §5 risk note, this path uses
    /// `try_lock` rather than `lock` so a contended mutex (e.g. a
    /// parallel fixture-conversion worker in Phase 6+) cannot cause
    /// Drop to block or to panic on a poisoned lock.
    fn drop(&mut self) {
        // If `finalize` already ran, the atomic flag short-circuits
        // and this is a no-op without touching the mutex at all.
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }

        // `try_lock` avoids the two Drop-time hazards the plan calls
        // out: blocking indefinitely when another thread holds the
        // lock, and an unwinding-Drop double-panic on a poisoned
        // mutex. If the lock is unavailable, we silently skip the
        // pending entries — they will leak rather than corrupt the
        // already-panicking process.
        //
        // In a poisoned-mutex case we still try to drain the pending
        // entries via `into_inner` (the `match` handles both arms),
        // because a panic that poisoned the mutex on another thread
        // is exactly the case the safety-net cleanup exists for.
        let pending = match self.inner.try_lock() {
            Ok(mut guard) => std::mem::take(&mut guard.pending),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                std::mem::take(&mut poisoned.into_inner().pending)
            }
            Err(std::sync::TryLockError::WouldBlock) => return,
        };

        for entry in pending {
            let final_class = classify_entry(&entry, self.keep_output);
            if final_class == GeneratedPathClass::Cleaned {
                // Best-effort: discard the error so Drop does not
                // double-panic on a filesystem failure mid-unwind.
                let _ = remove_path_best_effort(&entry.path);
            }
        }
    }
}

/// Classify one path against `target_root`'s git state.
///
/// Order of checks:
///
/// 1. Path resolves under `<target_root>/target/` — treated as
///    [`Ignored`](GeneratedPathClass::Ignored) without invoking git.
///    Cargo owns `target/`; even fork checkouts that lack a
///    `.gitignore` rule for it still treat it as transient.
/// 2. `git ls-files --error-unmatch -- <path>` exits 0 — the path is
///    committed.
/// 3. `git check-ignore --quiet -- <path>` exits 0 — the path is
///    ignored.
/// 4. Otherwise — `Cleaned` (the driver removes it on exit).
///
/// `git` invocations use `<target_root>` as the working directory so
/// the right `.gitignore` rules apply (per-directory `.gitignore`
/// files compose in git's pattern resolution).
///
/// Per locked decision §5.3, a missing `git` binary or a non-git
/// directory falls through to `Cleaned` — the safe default when the
/// classifier cannot prove the path is ignored or committed.
fn classify(target_root: &Path, path: &Path) -> GeneratedPathClass {
    if is_under_cargo_target(target_root, path) {
        return GeneratedPathClass::Ignored;
    }
    if git_is_tracked(target_root, path) {
        return GeneratedPathClass::Committed;
    }
    if git_is_ignored(target_root, path) {
        return GeneratedPathClass::Ignored;
    }
    GeneratedPathClass::Cleaned
}

/// Returns `true` when `path` lives under `<target_root>/target/`.
///
/// Implementation: canonicalize neither path (canonicalization fails
/// for paths that have just been removed, which is exactly the state
/// we may be classifying during cleanup). Instead, compare
/// `target_root.join("target")` against `path`'s `starts_with`. False
/// positives are not a concern: lihaaf does not generate artifacts
/// under any sibling directory named `target`.
fn is_under_cargo_target(target_root: &Path, path: &Path) -> bool {
    let target_dir = target_root.join("target");
    path.starts_with(&target_dir)
}

/// `git ls-files --error-unmatch -- <path>` — exit 0 means the path is
/// tracked. Any other exit (1, 128, "command not found", …) means
/// the classifier cannot prove tracked status and should fall through.
fn git_is_tracked(target_root: &Path, path: &Path) -> bool {
    git_quiet_status(target_root, &["ls-files", "--error-unmatch", "--"], path)
}

/// `git check-ignore --quiet -- <path>` — exit 0 means the path is
/// covered by a `.gitignore` rule reachable from `target_root`.
/// Any other exit means "not ignored" or "git unavailable"; both
/// fall through to the `Cleaned` default.
fn git_is_ignored(target_root: &Path, path: &Path) -> bool {
    git_quiet_status(target_root, &["check-ignore", "--quiet", "--"], path)
}

/// Shared spawn shape for the two classifier git calls.
///
/// Spawns `git <args> <path>` with `<target_root>` as the working
/// directory, redirects stdout / stderr to null (we only consume the
/// exit code), and returns `true` when exit is 0. Spawn failure (no
/// `git` on `PATH`, OS error) returns `false` — the caller treats
/// that as "classifier cannot prove the property" and falls through.
fn git_quiet_status(target_root: &Path, args: &[&str], path: &Path) -> bool {
    let output = Command::new("git")
        .args(args)
        .arg(path)
        .current_dir(target_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match output {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// Remove `path` from the filesystem without relying on a prior
/// `symlink_metadata` stat. Best-effort: a non-existent path is treated
/// as already-cleaned (no error) so the cleanup is idempotent across
/// reruns.
///
/// ## Race-free cascade
///
/// The previous implementation did `symlink_metadata(path)` → branch on
/// `file_type` → call one of `remove_dir_all` / `remove_symlink_dispatch`
/// / `remove_file`. That shape is TOCTOU-vulnerable: between the stat
/// and the removal call, the path entry could be swapped to a different
/// entry kind (e.g. a symlink pointing outside the intended scope), and
/// the wrong syscall would fire. On Windows the old dispatch did a
/// SECOND `symlink_metadata` call inside `remove_symlink_dispatch`,
/// widening the race window further.
///
/// The current cascade eliminates the stat-then-dispatch entirely. Each
/// step operates on the path's current state via a single syscall, and
/// each step's error space tells us which step to try next:
///
/// 1. **`remove_file`** — handles regular files AND file-symlinks.
///    - Unix: `unlink(2)` removes the directory entry for both regular
///      files and symlinks (regardless of target type) — never follows
///      the link.
///    - Windows: `DeleteFileW` handles regular files and file-symlinks;
///      it refuses directories and directory-symlinks with
///      `ERROR_ACCESS_DENIED` (surfaced as `PermissionDenied`, and in
///      some cases `IsADirectory`).
///    - On `IsADirectory` / `PermissionDenied` we proceed to step 2.
/// 2. **`remove_dir`** — handles empty directories AND directory-symlinks.
///    - Unix: `rmdir(2)` succeeds on empty directories; fails
///      `ENOTEMPTY` (`DirectoryNotEmpty`) on non-empty ones.
///    - Windows: `RemoveDirectoryW` removes empty directories AND
///      directory-symlinks AND junctions — it removes the LINK, not the
///      target tree. This is exactly the platform-aware symlink
///      handling the old `remove_symlink_dispatch` provided, but
///      without a separate stat.
///    - On `DirectoryNotEmpty` we proceed to step 3.
/// 3. **`remove_dir_all`** — recursive removal of a non-empty directory.
///    - Rust 1.84+ `std::fs::remove_dir_all` is race-safe internally: it
///      refuses to follow symlinks during the recursive walk, so even if
///      the entry is concurrently swapped between step 2 and step 3, we
///      will not delete the target of a freshly-planted symlink. MSRV
///      is 1.95 (see `Cargo.toml`), so the race-safe behavior is
///      guaranteed.
///
/// Each step independently checks `NotFound` — a concurrent unlink
/// between steps is treated as already-cleaned (idempotent).
///
/// **Why not pre-stat to pick the cheap path?** Picking the cheap path
/// based on a stat is exactly what introduces the race. The cascade
/// runs at most three syscalls in the worst case (file with wrong
/// permissions falling all the way through), and the common cases (a
/// regular file or empty directory) terminate in one or two syscalls
/// without any stat at all. The previous code paid one stat plus one
/// removal in the common case; the cascade pays one or two removals.
/// The cost is comparable, and the safety is strictly better.
fn remove_path_best_effort(path: &Path) -> Result<(), Error> {
    use std::io::ErrorKind;

    // Step 1: try to unlink as a file or file-symlink.
    match std::fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        // IsADirectory (Unix EISDIR; some platforms surface this
        // directly). PermissionDenied is the Windows shape on a
        // directory or directory-symlink (DeleteFileW returns
        // ERROR_ACCESS_DENIED). Either signal means "this entry is
        // not a file" — fall through to step 2.
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::IsADirectory | ErrorKind::PermissionDenied
            ) => {}
        Err(e) => {
            return Err(Error::io(
                e,
                "removing compat-generated file/symlink",
                Some(path.to_path_buf()),
            ));
        }
    }

    // Step 2: try to remove as an empty directory or a directory-symlink.
    // RemoveDirectoryW on Windows handles directory-symlinks and
    // junctions by removing the LINK entry (the target tree is
    // preserved). On Unix, rmdir(2) succeeds on empty real
    // directories and falls through to step 3 on non-empty ones.
    match std::fs::remove_dir(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        // DirectoryNotEmpty (Unix ENOTEMPTY) means we have a non-empty
        // real directory — proceed to recursive removal.
        Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {}
        Err(e) => {
            return Err(Error::io(
                e,
                "removing compat-generated empty dir / dir-symlink",
                Some(path.to_path_buf()),
            ));
        }
    }

    // Step 3: recursive removal of a non-empty directory.
    // std::fs::remove_dir_all (Rust 1.84+) refuses to follow symlinks
    // during the recursive walk; MSRV is 1.95 so this race-safe
    // behavior is guaranteed.
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(
            e,
            "recursively removing compat-generated directory",
            Some(path.to_path_buf()),
        )),
    }
}

/// Install a panic hook that names compat-generated paths in the
/// panic diagnostic. Optional sibling of the Drop guard: Drop runs
/// the actual cleanup, but the panic hook gives the operator a
/// pointer to the partial path so they can investigate or clean up
/// manually if the unwind itself is interrupted.
///
/// The previous hook is preserved and chained: `libtest`'s panic
/// capture (used by `cargo test`) continues to work, and any panic
/// hook installed by the binary's `main` runs after this one.
///
/// **Single-install semantics.** The hook is installed at most once
/// per process (gated by an internal [`AtomicBool`]). The compat
/// driver calls this once at the start of [`crate::compat::run`];
/// repeated calls (from re-entrant compat runs or from tests that
/// share a binary) are no-ops.
///
/// **Diagnostic, not cleanup.** This hook does NOT perform cleanup —
/// the Drop guard does. The hook exists to surface partial-path
/// information that would otherwise be lost in the panic noise. See
/// locked decision §5.6 in this module's header for the full
/// rationale.
///
/// `pub` to allow the test crate's `#[doc(hidden)]` re-export to
/// reach this function.
pub fn install_panic_hook() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The hook fires on every panic in the process, not just
        // compat-mode panics. We deliberately keep the diagnostic
        // minimal — adding context that depends on global state
        // (e.g. "the current compat run was tracking <paths>") would
        // require shared mutable state with its own poisoning
        // failure modes. The Drop guard owns the cleanup; this hook
        // just makes sure the panic surface stays informative.
        eprintln!(
            "lihaaf compat: panic during compat run — Drop guard will attempt cleanup of \
             registered paths; see envelope for residue list"
        );
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: classify a path against a directory that has no `git`
    /// metadata. The classifier should fall through to `Cleaned` per
    /// locked decision §5.3.
    #[test]
    fn classify_without_git_falls_through_to_cleaned() {
        let tmp = tempfile::tempdir().expect("tempdir for classify-no-git test");
        let path = tmp.path().join("artifact.txt");
        std::fs::write(&path, b"contents").expect("write artifact");

        let class = classify(tmp.path(), &path);
        assert_eq!(
            class,
            GeneratedPathClass::Cleaned,
            "non-git tempdir must classify as Cleaned"
        );
    }

    /// `is_under_cargo_target` correctly buckets `<root>/target/...`
    /// paths even when the directory does not exist yet (the classifier
    /// runs during cleanup, when the path may already be removed).
    #[test]
    fn cargo_target_is_classified_without_filesystem_lookup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("target").join("lihaaf-compat-converted");
        // Deliberately do NOT create the directory; the classifier
        // must work on logical-path bookkeeping alone.
        let class = classify(tmp.path(), &target);
        assert_eq!(class, GeneratedPathClass::Ignored);
    }

    /// `is_under_cargo_target` returns `false` for sibling directories
    /// that happen to contain the word "target" but are not the
    /// `<root>/target/` cargo directory.
    #[test]
    fn sibling_target_directory_is_not_under_cargo_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sibling = tmp.path().join("targets").join("file.txt");
        assert!(!is_under_cargo_target(tmp.path(), &sibling));
    }

    /// `remove_path_best_effort` on a non-existent path is a no-op.
    /// This is the rerun-idempotence invariant: cleanup may be called
    /// on a path that a previous attempt already removed.
    #[test]
    fn remove_nonexistent_path_is_ok() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("never-existed.txt");
        remove_path_best_effort(&path).expect("removing non-existent path must succeed");
    }

    /// **`remove_path_best_effort` removes a symlink-to-directory
    /// without following.** On Unix, step 1 of the cascade
    /// (`std::fs::remove_file` → `unlink(2)`) unlinks the symlink
    /// regardless of target kind, so this case terminates at step 1
    /// without touching the target tree. On Windows the same case
    /// falls through to step 2 (`std::fs::remove_dir` →
    /// `RemoveDirectoryW`), which removes the directory-symlink LINK
    /// without recursing into the target. Either way the target tree
    /// is preserved.
    #[cfg(unix)]
    #[test]
    fn remove_symlink_to_directory_unix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("real_dir");
        std::fs::create_dir_all(&target).expect("create target dir");
        std::fs::write(target.join("inside.txt"), b"keep me").expect("write into target");

        let link = tmp.path().join("link_to_dir");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink-to-dir");

        // Sanity: the link exists and resolves to a directory.
        assert!(link.exists(), "symlink must exist before removal");
        assert!(
            link.symlink_metadata().unwrap().file_type().is_symlink(),
            "link_to_dir must be a symlink, not a real dir"
        );

        remove_path_best_effort(&link).expect("removing the symlink must succeed");

        // The link is gone.
        assert!(
            !link.exists() && link.symlink_metadata().is_err(),
            "symlink must be removed"
        );
        // The target tree is untouched (we removed the link, not its
        // contents).
        assert!(target.exists(), "target directory must NOT be removed");
        assert!(
            target.join("inside.txt").exists(),
            "target's contents must NOT be removed"
        );
    }

    /// `remove_path_best_effort` deletes a directory tree
    /// recursively. This matches the `target/lihaaf-compat-converted/`
    /// shape Phase 6 will produce.
    #[test]
    fn remove_directory_tree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let nested = tmp.path().join("dir").join("nested");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(nested.join("file.txt"), b"data").expect("write nested file");

        let dir = tmp.path().join("dir");
        remove_path_best_effort(&dir).expect("recursive removal");
        assert!(!dir.exists(), "directory tree must be gone after cleanup");
    }
}
